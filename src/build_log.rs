//! Recover file/line diagnostics from TeX's file-line errors and traditional logs.
use crate::text::Text;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    ops::Range as ByteRange,
    path::{Path, PathBuf},
};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, Url};

const AUXILIARY_EXTENSIONS: &[&str] = &[
    "aux", "bbl", "toc", "lof", "lot", "out", "nav", "snm", "vrb",
];

fn log_extensions() -> impl Iterator<Item = &'static str> {
    crate::workspace::SOURCE_EXTENSIONS
        .iter()
        .copied()
        .chain(AUXILIARY_EXTENSIONS.iter().copied())
}

fn log_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| log_extensions().any(|candidate| candidate == extension))
}

/// TeX's byte-based print width can split a UTF-8 scalar across a line break.
/// Remove a break only when the surrounding bytes form a valid scalar.
pub(crate) fn decode_output(bytes: &[u8]) -> Cow<'_, str> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Cow::Borrowed(text);
    }
    let mut repaired = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let width = match bytes[i] {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        let mut scalar = [0; 4];
        scalar[0] = bytes[i];
        let mut count = 1;
        let mut after = i + 1;
        let mut wrapped = false;
        while count < width && after < bytes.len() {
            if !wrapped && bytes[after] == b'\n' {
                wrapped = true;
                after += 1;
            } else if !wrapped && bytes[after..].starts_with(b"\r\n") {
                wrapped = true;
                after += 2;
            } else if (0x80..=0xbf).contains(&bytes[after]) {
                scalar[count] = bytes[after];
                count += 1;
                after += 1;
            } else {
                break;
            }
        }
        if wrapped && count == width && std::str::from_utf8(&scalar[..width]).is_ok() {
            repaired.extend_from_slice(&scalar[..width]);
            i = after;
        } else {
            repaired.push(bytes[i]);
            i += 1;
        }
    }
    Cow::Owned(String::from_utf8_lossy(&repaired).into_owned())
}

/// Read TeX's final PDF summary, including wrapped paths and page/byte counts.
pub fn page_count(log: &str) -> Option<u32> {
    let lines = log.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate().rev() {
        if !line.starts_with("Output written on ") {
            continue;
        }
        let mut summary = String::new();
        for line in lines.iter().skip(index).take(16) {
            if *line == crate::output::TRUNCATED || summary.len() + line.len() > 8192 {
                break;
            }
            summary.push_str(line);
            let Some((_, tail)) = summary.rsplit_once('(') else {
                continue;
            };
            let Some((pages, tail)) = tail.split_once(" page") else {
                continue;
            };
            let tail = tail.strip_prefix('s').unwrap_or(tail);
            let valid = tail == ")."
                || tail
                    .strip_prefix(", ")
                    .and_then(|bytes| {
                        bytes
                            .strip_suffix(" bytes).")
                            .or_else(|| bytes.strip_suffix(" byte)."))
                    })
                    .is_some_and(|bytes| bytes.parse::<u64>().is_ok());
            if valid {
                if let Ok(pages) = pages.parse::<u32>() {
                    if pages > 0 {
                        return Some(pages);
                    }
                }
            }
        }
    }
    None
}

fn location(root: &Path, name: &str) -> Option<PathBuf> {
    let path = root.parent()?.join(name.trim().trim_matches('"'));
    let path = path.canonicalize().ok()?;
    path.is_file().then_some(path)
}

fn numbered_error(line: &str) -> Option<(&str, u32, &str)> {
    // Test each colon rather than splitting once: Windows drive letters and
    // filenames containing colons must not be mistaken for the line separator.
    for (colon, _) in line.match_indices(':') {
        let tail = &line[colon + 1..];
        let (number, message) = tail.split_once(':')?;
        if let Ok(number) = number.parse::<u32>() {
            let path = line[..colon].trim();
            if log_source(Path::new(path.trim_matches('"'))) {
                return Some((path, number, message.trim_start()));
            }
        }
    }
    None
}

fn input_line(message: &str) -> Option<u32> {
    let tail = message
        .rsplit_once("input line ")
        .or_else(|| message.rsplit_once("at lines "))
        .or_else(|| message.rsplit_once("at line "))?
        .1;
    tail.trim_start()
        .split(|ch: char| !ch.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

pub(crate) fn undefined_reference_key(message: &str) -> Option<(bool, &str)> {
    if !message.contains("undefined") {
        return None;
    }
    let (citation, tail) = if let Some((_, tail)) = message
        .split_once("Reference `")
        .or_else(|| message.split_once("Reference '"))
        .or_else(|| message.split_once("Hyper reference `"))
        .or_else(|| message.split_once("Hyper reference '"))
    {
        (false, tail)
    } else if let Some((_, tail)) = message
        .split_once("Citation `")
        .or_else(|| message.split_once("Citation '"))
    {
        (true, tail)
    } else {
        return None;
    };
    let (key, _) = tail
        .rsplit_once("' on ")
        .or_else(|| tail.rsplit_once("' undefined"))?;
    Some((citation, key))
}

fn destination_warning_name(message: &str) -> Option<&str> {
    if let Some(tail) = message.strip_prefix("pdfTeX warning (dest): name{") {
        let (name, tail) = tail.split_once("} has been referenced")?;
        let tail: String = tail.chars().filter(|ch| !ch.is_whitespace()).collect();
        if tail != "butdoesnotexist,replacedbyafixedone" {
            return None;
        }
        Some(name)
    } else {
        message
            .strip_prefix("warning")?
            .trim_start()
            .strip_prefix("(pdf backend): unreferenced destination with name '")?
            .strip_suffix('\'')
    }
}

fn duplicate_destination_warning_name(message: &str) -> Option<&str> {
    if let Some(tail) =
        message.strip_prefix("pdfTeX warning (ext4): destination with the same identifier (name{")
    {
        let (name, tail) = tail.split_once("}) has been already used")?;
        let tail: String = tail.chars().filter(|ch| !ch.is_whitespace()).collect();
        (tail == ",duplicateignored").then_some(name)
    } else {
        message
            .strip_prefix("warning")?
            .trim_start()
            .strip_prefix("(pdf backend): ignoring duplicate destination with the name '")?
            .strip_suffix('\'')
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DestinationWarningKind {
    Missing,
    Duplicate,
}

impl DestinationWarningKind {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Missing => "undefined-pdf-destination",
            Self::Duplicate => "duplicate-pdf-destination",
        }
    }
}

pub(crate) fn destination_warning(message: &str) -> Option<(DestinationWarningKind, &str)> {
    let (kind, name) = if let Some(name) = destination_warning_name(message) {
        (DestinationWarningKind::Missing, name)
    } else {
        (
            DestinationWarningKind::Duplicate,
            duplicate_destination_warning_name(message)?,
        )
    };
    crate::syntax::is_static_key(name).then_some((kind, name))
}

pub(crate) fn undefined_destination_name(message: &str) -> Option<&str> {
    let name = destination_warning_name(message)?;
    crate::syntax::is_static_key(name).then_some(name)
}

fn box_code(message: &str) -> Option<&'static str> {
    let code = if message.starts_with("Overfull ") {
        "overfull-box"
    } else if message.starts_with("Underfull ") {
        "underfull-box"
    } else {
        return None;
    };
    Some(code)
}

fn box_range(text: &Text, message: &str) -> Option<Range> {
    box_code(message)?;
    let tail = message.rsplit_once("at lines ")?.1;
    let (start, end) = tail.split_once("--")?;
    let start: u32 = start.parse().ok()?;
    let end: u32 = end
        .split(|ch: char| !ch.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    if start == 0 || end < start {
        return None;
    }
    let last = text.text_line_count().saturating_sub(1) as u32;
    let start = (start - 1).min(last);
    let end = (end - 1).min(last);
    let first = text.source.lines().nth(start as usize).unwrap_or("");
    let last = text.source.lines().nth(end as usize).unwrap_or("");
    Some(Range::new(
        Position::new(
            start,
            first
                .chars()
                .take_while(|ch| ch.is_whitespace())
                .map(char::len_utf16)
                .sum::<usize>() as u32,
        ),
        Position::new(end, last.encode_utf16().count() as u32),
    ))
}

fn missing_include_file(path: &Path, lines: &[&str], index: &mut usize) -> Option<String> {
    let line = lines[*index];
    let text = Text::new(std::fs::read_to_string(path).ok()?);
    let syntax = crate::syntax::Syntax::latex(&text);
    let mut candidates = Vec::new();
    for command in syntax
        .commands
        .iter()
        .filter(|command| command.name == "include")
        .take(1024)
    {
        let Some(argument) = &command.argument else {
            continue;
        };
        let value = argument.value.trim();
        if value.is_empty() || value.contains(['\\', '{', '}', '%', '#', '$', '\r', '\n']) {
            continue;
        }
        for name in crate::syntax::file_lookup_names("include", value) {
            for message in [format!("No file {name}."), format!("No file \"{name}\".")] {
                if message.len() <= 8192 && message.starts_with(line) {
                    if message == line {
                        return Some(name.into_owned());
                    }
                    candidates.push((message, name.to_string()));
                }
            }
        }
    }
    // Join only prefixes of a known static include. This avoids swallowing
    // adjacent diagnostics or inventing chapter warnings for optional files.
    if candidates.is_empty() {
        return None;
    }
    let mut joined = line.to_owned();
    for (next_index, next) in lines.iter().enumerate().skip(*index + 1).take(64) {
        if joined.len() + next.len() > 8192 {
            break;
        }
        joined.push_str(next);
        candidates.retain(|(message, _)| message.starts_with(&joined));
        if let Some((_, name)) = candidates.iter().find(|(message, _)| message == &joined) {
            *index = next_index;
            return Some(name.clone());
        }
        if candidates.is_empty() {
            break;
        }
    }
    None
}

pub(crate) fn missing_include_key(message: &str) -> Option<&str> {
    message
        .strip_prefix("Included file `")?
        .strip_suffix("` not found")
}

fn missing_include_ranges(text: &Text, name: &str) -> Vec<Range> {
    crate::syntax::Syntax::latex(text)
        .commands
        .iter()
        .filter(|command| command.name == "include")
        .filter_map(|command| {
            let argument = command.argument.as_ref()?;
            let span = crate::syntax::trimmed_span(&text.source, argument.span.clone());
            crate::syntax::file_lookup_names("include", argument.value.trim())
                .any(|candidate| candidate == name)
                .then(|| text.range(span.start, span.end))
        })
        .collect()
}

pub(crate) fn undefined_reference_ranges(text: &Text, line: u32, message: &str) -> Vec<Range> {
    let Some((citation, key)) = undefined_reference_key(message) else {
        return vec![];
    };
    let syntax = crate::syntax::Syntax::latex(text);
    let hyper_link = message.contains("Hyper reference `") || message.contains("Hyper reference '");
    let mut ranges = Vec::new();
    for command in &syntax.commands {
        if !(if citation {
            crate::syntax::warns_on_undefined_citation(&command.name)
        } else if hyper_link {
            command.name == "hyperref"
                || matches!(
                    command.name.as_str(),
                    "Ref" | "autopageref" | "vref" | "Vref"
                ) && text
                    .source
                    .as_bytes()
                    .get(crate::syntax::skip_space(&text.source, command.span.end))
                    != Some(&b'*')
        } else {
            command.name != "hyperref"
                && (crate::syntax::warns_on_undefined_reference(&command.name)
                    || crate::syntax::is_counter_reference(&command.name))
        }) {
            continue;
        }
        let separate_label = crate::syntax::separate_label_argument(&text.source, command);
        let argument = if crate::syntax::has_separate_label_argument(&command.name) {
            separate_label.as_ref()
        } else {
            command.argument.as_ref()
        };
        let Some(argument) = argument else {
            continue;
        };
        let range_end = crate::syntax::range_reference_end(&text.source, command);
        // hyperref reads the display text before warning, so a multiline link
        // reports the closing text line rather than the bracketed key's line.
        // Varioref reads both endpoints before reporting either missing label.
        let report_end = if command.name == "hyperref" {
            crate::syntax::group(
                &text.source,
                crate::syntax::skip_space(&text.source, argument.outer_span.end),
                b'{',
                b'}',
            )
            .map_or(argument.span.end, |(text, _)| text.span.end)
        } else {
            range_end
                .as_ref()
                .map_or(argument.span.end, |end| end.span.end)
        };
        if text.position(command.span.start).line > line || text.position(report_end).line < line {
            continue;
        }
        if !citation
            && (crate::syntax::is_single_reference(&command.name)
                || crate::syntax::has_separate_label_argument(&command.name))
        {
            if crate::syntax::is_static_key(key) {
                for argument in std::iter::once(argument).chain(range_end.as_ref()) {
                    if argument.value == key {
                        ranges.push(text.range(argument.span.start, argument.span.end));
                    }
                }
            }
            continue;
        }
        let mut start = argument.span.start;
        for value in argument.value.split(',') {
            let span = crate::syntax::trimmed_span(&text.source, start..start + value.len());
            if &text.source[span.clone()] == key {
                ranges.push(text.range(span.start, span.end));
            }
            start += value.len() + 1;
        }
    }
    ranges
}

fn emit(
    result: &mut BTreeMap<Url, Vec<Diagnostic>>,
    path: &Path,
    line: u32,
    message: String,
    severity: DiagnosticSeverity,
) {
    emit_with_context(result, path, line, message, severity, None, true);
}

pub(crate) fn unresolved_location(diagnostic: &Diagnostic) -> bool {
    diagnostic.data.as_ref().is_some_and(|data| {
        ["unresolvedSource", "unresolvedLine"]
            .iter()
            .any(|key| data.get(key).and_then(serde_json::Value::as_bool) == Some(true))
    })
}

struct PackageOptions {
    loaded: String,
    requested: String,
}

struct ErrorContext<'a> {
    source: &'a str,
    package_options: Option<PackageOptions>,
}

impl<'a> ErrorContext<'a> {
    fn source(source: &'a str) -> Self {
        Self {
            source,
            package_options: None,
        }
    }
}

/// Help is printed after the two-line source echo, with byte wraps even inside
/// names and values. Keep an independent, bounded cursor so looking for help
/// cannot consume a later diagnostic or change the main log parser's position.
#[derive(Clone, Copy)]
struct OptionHelp<'a> {
    lines: &'a [&'a str],
    index: usize,
    bytes: usize,
}

impl<'a> OptionHelp<'a> {
    fn read_line(&mut self) -> Option<&'a str> {
        let line = *self.lines.get(self.index)?;
        if self.index >= 64 || self.bytes + line.len() > 8192 || line == crate::output::TRUNCATED {
            return None;
        }
        self.index += 1;
        self.bytes += line.len();
        Some(line)
    }

    fn expect(&mut self, mut value: &str) -> Option<()> {
        while !value.is_empty() {
            let line = self.read_line()?;
            if line.is_empty() {
                return None;
            }
            value = value.strip_prefix(line)?;
        }
        Some(())
    }

    fn options(&mut self, following: &str) -> Option<String> {
        let mut value = self.read_line()?.strip_prefix("  [")?.to_owned();
        loop {
            if value.len() > 4096 {
                return None;
            }
            if let Some(content) = value.strip_suffix(']') {
                let mut after = *self;
                if after.expect(following).is_some() {
                    *self = after;
                    return Some(content.into());
                }
            }
            let line = self.read_line()?;
            if line.trim().is_empty()
                || message_boundary(line)
                || line.trim_start().starts_with('(')
                || ["The package ", "There has now ", "Adding the global "]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
            {
                return None;
            }
            value.push_str(line);
        }
    }
}

fn package_option_details(lines: &[&str], start: usize, message: &str) -> Option<PackageOptions> {
    let name = package_option_clash_name(message)?;
    let mut lines = lines.get(start..)?;
    // TeX's second source-echo line is indented. Only this immediate line may
    // precede the help header; arbitrary intervening output is not evidence.
    if lines
        .first()
        .is_some_and(|line| line.is_empty() || line.starts_with(' '))
    {
        lines = &lines[1..];
    }
    let mut help = OptionHelp {
        lines,
        index: 0,
        bytes: 0,
    };
    help.expect(&format!(
        "The package {name} has already been loaded with options:"
    ))?;
    let loaded = help.options("There has now been an attempt to load it with options")?;
    let requested = help.options("Adding the global options:")?;
    Some(PackageOptions { loaded, requested })
}

/// A direct TeX error prints the consumed source prefix on its l.N line.
/// Expansion frames precede that line. Allow a single recently read token for
/// undefined commands only when it also ends the consumed source prefix. Math
/// mode adds this frame even for direct calls. Other expansion frames still
/// require a whole-line diagnostic rather than blaming a valid macro call.
fn error_context<'a>(
    lines: &[&'a str],
    start: usize,
    number: u32,
    message: &str,
) -> Option<ErrorContext<'a>> {
    let mut context = lines
        .iter()
        .enumerate()
        .skip(start)
        .take(16)
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, &line)| (index, line));
    let (mut index, mut line) = context.next()?;
    let recent = (message == "Undefined control sequence.")
        .then(|| line.trim_start().strip_prefix("<recently read>"))
        .flatten()
        .map(str::trim);
    if let Some(token) = recent {
        if crate::syntax::control_word(token, 0)? != (0..token.len()) {
            return None;
        }
        (index, line) = context.next()?;
    }
    let graphics = missing_graphic_name(message).is_some() && missing_file_name(message).is_none();
    let documentation = line.trim() == "See the LaTeX manual or LaTeX Companion for explanation."
        || graphics
            && [
                "See the pdftex.def package documentation for explanation.",
                "See the luatex.def package documentation for explanation.",
            ]
            .contains(&line.trim());
    if recent.is_none() && documentation {
        if context.next()?.1.trim() != "Type  H <return>  for immediate help."
            || context.next()?.1.trim() != "..."
        {
            return None;
        }
        (index, line) = context.next()?;
    }
    // XeTeX reports this one internal read frame for a direct graphicx call.
    // Other expansion frames still prevent precise source attribution.
    if graphics
        && message.starts_with("Unable to load picture or PDF file ")
        && line.trim() == "<to be read again>"
    {
        if context.next()?.1.trim() != "}" {
            return None;
        }
        (index, line) = context.next()?;
    }
    let tail = line.trim_start().strip_prefix("l.")?;
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    if tail[..digits].parse::<u32>().ok()? != number {
        return None;
    }
    let prefix = tail[digits..].strip_prefix(' ')?;
    recent
        .is_none_or(|token| prefix.trim_end().ends_with(token))
        .then(|| ErrorContext {
            source: prefix,
            package_options: package_option_details(lines, index + 1, message),
        })
}

pub(crate) fn missing_file_name(message: &str) -> Option<&str> {
    let quoted = message
        .strip_prefix("LaTeX Error: File ")
        .and_then(|tail| tail.strip_suffix(" not found."))
        .or_else(|| {
            message
                .strip_prefix("I can't find file ")?
                .strip_suffix('.')
        })?;
    let name = quoted
        .strip_prefix('`')
        .or_else(|| quoted.strip_prefix('\''))?
        .strip_suffix('\'')?;
    let name = name
        .strip_prefix('"')
        .and_then(|name| name.strip_suffix('"'))
        .unwrap_or(name);
    (!name.is_empty()).then_some(name)
}

pub(crate) fn missing_graphic_name(message: &str) -> Option<&str> {
    if let Some(name) = missing_file_name(message) {
        return Some(name);
    }
    let quoted = ["pdftex.def", "luatex.def"].into_iter().find_map(|driver| {
        message
            .strip_prefix(&format!("Package {driver} Error: File "))?
            .strip_suffix(" not found: using draft setting.")
    });
    let name = if let Some(quoted) = quoted {
        quoted.strip_prefix('`')?.strip_suffix('\'')?
    } else {
        message
            .strip_prefix("Unable to load picture or PDF file '")?
            .strip_suffix("'.")?
    };
    let name = name
        .strip_prefix('"')
        .and_then(|name| name.strip_suffix('"'))
        .unwrap_or(name);
    (!name.is_empty()).then_some(name)
}

fn following_error_location<'a>(lines: &[&'a str], start: usize) -> Option<(u32, Option<&'a str>)> {
    let mut direct = true;
    for line in lines.iter().skip(start).take(16) {
        let line = line.trim_start();
        if let Some(tail) = line.strip_prefix("l.") {
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            return Some((
                tail[..digits].parse().ok()?,
                direct.then(|| tail[digits..].strip_prefix(' ')).flatten(),
            ));
        }
        if message_boundary(line) || line.starts_with('(') {
            break;
        }
        if !line.trim().is_empty() && line.trim() != "<read *>" {
            direct = false;
        }
    }
    None
}

fn uninterrupted_file_prompt(lines: &[&str], start: usize, end: usize) -> bool {
    end.saturating_sub(start) <= 32
        && lines.get(start..end).is_some_and(|lines| {
            lines
                .iter()
                .all(|line| !message_boundary(line) && !line.trim_start().starts_with('('))
        })
}

fn missing_path_range(
    text: &Text,
    line: u32,
    context: &str,
    name: &str,
    command_name: &str,
) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    let syntax = crate::syntax::Syntax::latex(text);
    let mut matches = syntax.commands.iter().filter_map(|command| {
        if command.name != command_name {
            return None;
        }
        let argument = command.argument.as_ref()?;
        let span = crate::syntax::trimmed_span(&text.source, argument.span.clone());
        let value = &text.source[span.clone()];
        if !crate::syntax::is_literal_file_path(value)
            || !crate::syntax::file_lookup_names(command_name, value)
                .any(|candidate| candidate == name)
            || text.position(argument.outer_span.end).line != line
            || text.source[argument.outer_span.clone()].starts_with('"')
                && !text.source[argument.outer_span.clone()].ends_with('"')
        {
            return None;
        }
        let consumed = &text.source[start..argument.outer_span.end];
        let matched = shortened.map_or(consumed == prefix, |suffix| {
            suffix.ends_with(&text.source[argument.outer_span.clone()])
                && consumed.ends_with(suffix)
        });
        matched.then(|| text.range(span.start, span.end))
    });
    let range = matches.next()?;
    matches.next().is_none().then_some(range)
}

fn package_option_clash_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("LaTeX Error: Option clash for package ")?
        .strip_suffix('.')?;
    (crate::syntax::is_literal_file_path(name) && !name.contains([' ', '\t', '\r', '\n']))
        .then_some(name)
}

fn loader_failure_range(
    text: &Text,
    line: u32,
    context: &str,
    name: &str,
) -> Option<(Range, &'static str)> {
    if !name.ends_with(".sty") && !name.ends_with(".cls") {
        return None;
    }
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    let syntax = crate::syntax::Syntax::latex(text);
    let mut matches = Vec::new();
    for command in &syntax.commands {
        let command_name = match command.name.as_str() {
            "usepackage" => "usepackage",
            "RequirePackage" => "RequirePackage",
            "RequirePackageWithOptions" => "RequirePackageWithOptions",
            "documentclass" => "documentclass",
            "LoadClass" => "LoadClass",
            "LoadClassWithOptions" => "LoadClassWithOptions",
            _ => continue,
        };
        let Some(argument) = &command.argument else {
            continue;
        };
        let next = crate::syntax::skip_space(&text.source, argument.outer_span.end);
        // LaTeX reads the optional minimum version, or one lookahead token,
        // before the loader reports a failure. That token may be on
        // the next source line, while the path being diagnosed is above it.
        let tail = if text.source.as_bytes().get(next) == Some(&b'[') {
            crate::syntax::group(&text.source, next, b'[', b']').map(|(_, end)| end)
        } else if text.source.as_bytes().get(next) == Some(&b'\\') {
            crate::syntax::control_word(&text.source, next)
                .map(|word| word.end)
                .or_else(|| {
                    text.source
                        .get(next + 1..)?
                        .chars()
                        .next()
                        .map(|token| next + 1 + token.len_utf8())
                })
        } else {
            text.source
                .get(next..)?
                .chars()
                .next()
                .map(|token| next + token.len_utf8())
        };
        if text.source.as_bytes().get(next) == Some(&b'[') && tail.is_none() {
            continue;
        }
        let confirmed = [Some(argument.outer_span.end), tail]
            .into_iter()
            .flatten()
            .any(|end| {
                if text.position(end).line != line {
                    return false;
                }
                let consumed = &text.source[start..end];
                let token = if end == argument.outer_span.end {
                    &text.source[argument.outer_span.clone()]
                } else {
                    &text.source[next..end]
                };
                shortened.map_or(consumed == prefix, |suffix| {
                    suffix.ends_with(token)
                        && consumed.ends_with(suffix)
                        && (text.position(argument.outer_span.start).line != line
                            || suffix.contains(&text.source[argument.outer_span.clone()]))
                        && text.source[start..]
                            .split('\n')
                            .next()
                            .unwrap_or_default()
                            .match_indices(suffix)
                            .take(2)
                            .count()
                            == 1
                })
            });
        if !confirmed {
            continue;
        }
        let values = if crate::syntax::file_list(command_name) {
            argument.value.split(',').collect::<Vec<_>>()
        } else {
            vec![argument.value.as_str()]
        };
        // A dynamic sibling can generate the reported filename; do not infer
        // which item caused the error without a completely static list.
        if values.iter().any(|value| {
            !crate::syntax::is_literal_file_path(&crate::syntax::loader_name(value.trim()))
        }) {
            continue;
        }
        let mut offset = argument.span.start;
        for value in values {
            let span = crate::syntax::trimmed_span(&text.source, offset..offset + value.len());
            if crate::syntax::file_lookup_names(command_name, value.trim())
                .any(|candidate| candidate == name)
            {
                matches.push((text.range(span.start, span.end), command_name));
            }
            offset += value.len() + 1;
        }
    }
    (matches.len() == 1).then(|| matches[0])
}

pub(crate) fn undefined_environment_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("LaTeX Error: Environment ")?
        .strip_suffix(" undefined.")?;
    crate::syntax::is_static_key(name).then_some(name)
}

pub(crate) fn undefined_counter_name(message: &str) -> Option<&str> {
    let tail = message.strip_prefix("LaTeX Error: No counter ")?;
    let name = tail
        .strip_prefix('\'')
        .or_else(|| tail.strip_prefix('`'))?
        .strip_suffix("' defined.")?;
    crate::syntax::is_static_key(name).then_some(name)
}

fn undefined_counter_range(text: &Text, line: u32, context: &str, name: &str) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    let syntax = crate::syntax::Syntax::latex(text);
    let mut matches = syntax
        .commands
        .iter()
        .flat_map(|command| crate::counters::invocations(&text.source, command))
        .filter_map(|(span, tail)| {
            if text.source[span.clone()] != *name || text.position(tail.end).line != line {
                return None;
            }
            let consumed = &text.source[start..tail.end];
            let matched = shortened.map_or(consumed == prefix, |suffix| {
                suffix.ends_with(&text.source[tail]) && consumed.ends_with(suffix)
            });
            matched.then(|| text.range(span.start, span.end))
        });
    let range = matches.next()?;
    matches.next().is_none().then_some(range)
}

struct EnvironmentError<'a> {
    command: &'static str,
    name: &'a str,
    code: &'static str,
    expected: Option<&'a str>,
}

fn environment_error(message: &str) -> Option<EnvironmentError<'_>> {
    if let Some(name) = undefined_environment_name(message) {
        return Some(EnvironmentError {
            command: "begin",
            name,
            code: "undefined-environment",
            expected: None,
        });
    }
    let (name, expected) = mismatched_environment_names(message)?;
    Some(EnvironmentError {
        command: "end",
        name,
        code: "mismatched-environment-end",
        expected: Some(expected),
    })
}

pub(crate) fn mismatched_environment_names(message: &str) -> Option<(&str, &str)> {
    let tail = message.strip_prefix("LaTeX Error: \\begin{")?;
    let (expected, tail) = tail.split_once('}')?;
    let tail = if let Some(tail) = tail.strip_prefix(" on input line ") {
        let length = tail.bytes().take_while(u8::is_ascii_digit).count();
        if length == 0 {
            return None;
        }
        &tail[length..]
    } else {
        tail
    };
    let name = tail.strip_prefix(" ended by \\end{")?.strip_suffix("}.")?;
    (crate::syntax::is_static_key(name) && crate::syntax::is_static_key(expected))
        .then_some((name, expected))
}

fn environment_name_range(
    text: &Text,
    line: u32,
    context: &str,
    error: &EnvironmentError<'_>,
) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    let syntax = crate::syntax::Syntax::latex(text);
    let mut matches = syntax.commands.iter().filter_map(|command| {
        if command.name != error.command {
            return None;
        }
        let argument = command.argument.as_ref()?;
        let span = argument.span.clone();
        if text.source[span.clone()] != *error.name
            || text.position(argument.outer_span.end).line != line
        {
            return None;
        }
        let consumed = &text.source[start..argument.outer_span.end];
        let matched = shortened.map_or(consumed == prefix, |suffix| {
            suffix.ends_with(&text.source[argument.outer_span.clone()])
                && consumed.ends_with(suffix)
        });
        matched.then(|| text.range(span.start, span.end))
    });
    let range = matches.next()?;
    matches.next().is_none().then_some(range)
}

fn shortened_context(prefix: &str) -> Option<&str> {
    // TeX may cut the leading scalar in half when shortening error context.
    // Discard only its undecodable leading bytes; matching still requires a
    // complete command/argument and exactly one source occurrence.
    prefix
        .strip_prefix("...")
        .map(|suffix| suffix.trim_start_matches('\u{fffd}'))
}

fn undefined_command_range(text: &Text, line: u32, context: &str) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    let syntax = crate::syntax::Syntax::latex(text);
    let mut matches = syntax.commands.iter().filter(|command| {
        if text.position(command.span.start).line != line {
            return false;
        }
        let consumed = &text.source[start..command.span.end];
        shortened.map_or(consumed == prefix, |suffix| {
            suffix.ends_with(&text.source[command.span.clone()]) && consumed.ends_with(suffix)
        })
    });
    let command = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(text.range(command.span.start, command.span.end))
}

fn emit_with_context(
    result: &mut BTreeMap<Url, Vec<Diagnostic>>,
    path: &Path,
    line: u32,
    mut message: String,
    severity: DiagnosticSeverity,
    error: Option<ErrorContext<'_>>,
    source_confirmed: bool,
) {
    let context = error.as_ref().map(|context| context.source);
    let package_options = error
        .as_ref()
        .and_then(|context| context.package_options.as_ref());
    let Ok(uri) = Url::from_file_path(path) else {
        return;
    };
    // Font/glyph diagnostics often repeat at shipout. Avoid reading and mapping
    // the same source file once for every repetition of an identical warning.
    if line != 0
        && context.is_none()
        && result.get(&uri).is_some_and(|diagnostics| {
            diagnostics.iter().any(|diagnostic| {
                diagnostic.message == message
                    && diagnostic.severity == Some(severity)
                    && diagnostic.range.start.line == line.saturating_sub(1)
                    && unresolved_location(diagnostic) != source_confirmed
            })
        })
    {
        return;
    }
    let source = std::fs::read_to_string(path).unwrap_or_default();
    let text = Text::new(source);
    let last_line = text.text_line_count().saturating_sub(1) as u32;
    let reported_line = line;
    let invalid_line = line.checked_sub(1).is_none_or(|line| line > last_line);
    let rewritten = line.saturating_sub(1) > last_line
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| AUXILIARY_EXTENSIONS.contains(&extension));
    if rewritten {
        message.push_str(&format!(
            " (reported line {reported_line} is no longer present in the generated file)"
        ));
    }
    let line = line.saturating_sub(1).min(last_line);
    let value = text.source.lines().nth(line as usize).unwrap_or("");
    let start = value
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .map(char::len_utf16)
        .sum::<usize>() as u32;
    let end = value.encode_utf16().count() as u32;
    let line_range = Range::new(
        Position::new(line, start.min(end)),
        Position::new(line, end),
    );
    if !source_confirmed || invalid_line {
        // References and destinations can be located without an expansion
        // context. They must not borrow matching names from a fallback file
        // or a clamped line whose reported location could not be verified.
        let mut data = serde_json::json!({});
        if !source_confirmed {
            data["unresolvedSource"] = true.into();
        }
        if invalid_line {
            data["unresolvedLine"] = true.into();
            data["reportedLine"] = reported_line.into();
        }
        result.entry(uri).or_default().push(Diagnostic {
            range: line_range,
            severity: Some(severity),
            source: Some("latex-build".into()),
            message,
            data: Some(data),
            ..Default::default()
        });
        return;
    }
    let mut ranges = undefined_reference_ranges(&text, line, &message);
    let destination = destination_warning(&message);
    let destination_ranges: Vec<_> = destination
        .into_iter()
        .flat_map(|(kind, name)| {
            let syntax = crate::syntax::Syntax::latex(&text);
            let entries = match kind {
                DestinationWarningKind::Missing => syntax.anchor_references,
                DestinationWarningKind::Duplicate => syntax.anchors,
            };
            entries
                .into_iter()
                .filter(move |entry| entry.name == name)
                .map(|entry| text.range(entry.span.start, entry.span.end))
        })
        .collect();
    let precise_destination = !destination_ranges.is_empty();
    ranges.extend(destination_ranges);
    let command_range = (message == "Undefined control sequence."
        && reported_line.checked_sub(1) == Some(line))
    .then(|| context.and_then(|context| undefined_command_range(&text, line, context)))
    .flatten();
    ranges.extend(command_range);
    let environment = environment_error(&message);
    let environment_range = environment
        .as_ref()
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|error| environment_name_range(&text, line, context?, error));
    ranges.extend(environment_range);
    let counter_name = undefined_counter_name(&message);
    let counter_range = counter_name
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| undefined_counter_range(&text, line, context?, name));
    ranges.extend(counter_range);
    let missing_file = missing_file_name(&message);
    let input_range = missing_file
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| missing_path_range(&text, line, context?, name, "input"));
    ranges.extend(input_range);
    let loader_range =
        missing_file.and_then(|name| loader_failure_range(&text, line, context?, name));
    ranges.extend(loader_range.map(|(range, _)| range));
    let package_clash = package_option_clash_name(&message);
    let clash_range = package_clash
        .and_then(|name| loader_failure_range(&text, line, context?, &format!("{name}.sty")))
        // WithOptions does not supply a new explicit options list. Do not
        // attribute a clash to a forwarding call based on its name alone.
        .filter(|(_, command)| matches!(*command, "usepackage" | "RequirePackage"));
    ranges.extend(clash_range.map(|(range, _)| range));
    let missing_graphic = missing_graphic_name(&message);
    let graphic_range = missing_graphic
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| missing_path_range(&text, line, context?, name, "includegraphics"));
    ranges.extend(graphic_range);
    let command_name = command_range.and_then(|range| {
        let start = text.offset(range.start)?;
        let end = text.offset(range.end)?;
        text.source[start..end].strip_prefix('\\')
    });
    if let Some(name) = missing_include_key(&message) {
        ranges.extend(missing_include_ranges(&text, name));
        // Optional package inputs also print "No file". Report chapter
        // omissions only when the source identifies the corresponding include.
        if ranges.is_empty() {
            return;
        }
    }
    if let Some(range) = box_range(&text, &message) {
        ranges.push(range);
    }
    if ranges.is_empty() {
        ranges.push(line_range);
    }
    let displayed_message = match (clash_range, package_options) {
        (Some(_), Some(options)) => format!(
            "{message}\nAlready loaded options: [{}]\nRequested options: [{}]",
            options.loaded, options.requested
        ),
        _ => message.clone(),
    };
    result
        .entry(uri)
        .or_default()
        .extend(ranges.into_iter().map(|range| {
            Diagnostic {
                range,
                severity: Some(severity),
                source: Some("latex-build".into()),
                code: box_code(&message)
                    .or_else(|| destination.filter(|_| precise_destination).map(|(kind, _)| kind.code()))
                    .or_else(|| missing_include_key(&message).map(|_| "missing-include"))
                    .or_else(|| input_range.map(|_| "missing-input"))
                    .or_else(|| loader_range.map(|(_, command)| if crate::syntax::loader_extension(command) == Some("sty") { "missing-package" } else { "missing-class" }))
                    .or_else(|| clash_range.map(|_| "package-option-clash"))
                    .or_else(|| graphic_range.map(|_| "missing-image"))
                    .or_else(|| command_range.map(|_| "undefined-control-sequence"))
                    .or_else(|| environment_range.and(environment.as_ref()).map(|error| error.code))
                    .or_else(|| counter_range.map(|_| "undefined-counter"))
                    .map(|code| NumberOrString::String(code.into())),
                message: displayed_message.clone(),
                data: precise_destination.then(|| serde_json::json!({"destination":destination.map(|(_, name)| name)}))
                    .or_else(|| command_name.map(|name| serde_json::json!({"command":name})))
                    .or_else(|| {
                        environment_range
                            .and(environment.as_ref())
                            .map(|error| match error.expected {
                                Some(expected) => serde_json::json!({"environment":error.name,"expectedEnvironment":expected}),
                                None => serde_json::json!({"environment":error.name}),
                            })
                    })
                    .or_else(|| counter_range.and(counter_name).map(|name| serde_json::json!({"counter":name})))
                    .or_else(|| input_range.and(missing_file).map(|name| serde_json::json!({"file":name})))
                    .or_else(|| loader_range.map(|(_, command)| serde_json::json!({"file":missing_file,"loaderCommand":command})))
                    .or_else(|| clash_range.map(|(_, command)| {
                        let mut data = serde_json::json!({"package":package_clash,"loaderCommand":command});
                        if let Some(options) = package_options {
                            data["loadedOptions"] = options.loaded.clone().into();
                            data["requestedOptions"] = options.requested.clone().into();
                        }
                        data
                    }))
                    .or_else(|| graphic_range.and(missing_graphic).map(|name| serde_json::json!({"file":name}))),
                ..Default::default()
            }
        }));
}

fn warning_start(line: &str) -> bool {
    (line.starts_with("LaTeX ") || line.starts_with("Package ") || line.starts_with("Class "))
        && line.contains(" Warning:")
        || [
            "pdfTeX warning",
            "LuaTeX warning",
            "LuaHBTeX warning",
            "XeTeX warning",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        || line
            .strip_prefix("warning")
            .is_some_and(|tail| tail.trim_start().starts_with("(pdf backend):"))
}

fn message_boundary(line: &str) -> bool {
    let line = line.trim_start();
    line == crate::output::TRUNCATED
        || warning_start(line)
        || line.starts_with("! ")
        || line.starts_with("Missing character:")
        || line.starts_with("Overfull ")
        || line.starts_with("Underfull ")
        || line.starts_with("No file ")
        || numbered_error(line).is_some()
        || line
            .strip_prefix("l.")
            .is_some_and(|tail| tail.starts_with(|ch: char| ch.is_ascii_digit()))
        || line.starts_with(')')
}

fn wrapped_message(
    root: &Path,
    lines: &[&str],
    separator_lines: &[usize],
    i: &mut usize,
) -> String {
    let initial = lines[*i].trim_start();
    let lua_destination = initial.strip_prefix("warning").is_some_and(|tail| {
        tail.trim_start()
            .starts_with("(pdf backend): unreferenced destination with name '")
            || tail
                .trim_start()
                .starts_with("(pdf backend): ignoring duplicate destination with the name '")
    });
    if initial.starts_with("pdfTeX warning (dest): name{")
        || initial.starts_with("pdfTeX warning (ext4): destination with the same identifier (name{")
        || lua_destination
    {
        // The engines break these messages at a byte count, even within a
        // destination name or a word. Join the original bytes; adding spaces
        // or trimming a continuation can change the destination's identity.
        let mut message = initial.to_owned();
        for _ in 0..64 {
            let complete = destination_warning_name(&message).is_some()
                || duplicate_destination_warning_name(&message).is_some();
            // LuaTeX prints names without escaping apostrophes. An apostrophe
            // exactly at the print width can belong to a continued name.
            if (complete && (!lua_destination || lines[*i].len() < 79)) || message.len() >= 4096 {
                break;
            }
            let Some(next) = lines.get(*i + 1) else { break };
            let closes_destination = initial
                .starts_with("pdfTeX warning (ext4): destination with the same identifier (name{")
                && message.ends_with('}')
                && next.starts_with(") has been already used");
            if next.trim().is_empty()
                || (message_boundary(next) && !closes_destination)
                || next.trim_start().starts_with(['<', '('])
                || [
                    "Output written on ",
                    "Transcript written on ",
                    "Here is how much",
                ]
                .iter()
                .any(|prefix| next.trim_start().starts_with(prefix))
            {
                break;
            }
            let mut next_index = *i + 1;
            if numbered_error(&continued_error_line(
                root,
                lines,
                separator_lines,
                &mut next_index,
            ))
            .is_some()
            {
                break;
            }
            message.push_str(next);
            *i += 1;
        }
        return message;
    }
    let mut message = lines[*i].trim().to_owned();
    while !message.ends_with(['.', '!']) && *i + 1 < lines.len() && message.len() < 4096 {
        let next = lines[*i + 1];
        if next.trim().is_empty() || message_boundary(next) {
            break;
        }
        let mut next_index = *i + 1;
        if numbered_error(&continued_error_line(
            root,
            lines,
            separator_lines,
            &mut next_index,
        ))
        .is_some()
        {
            break;
        }
        if next.trim_start().starts_with('(') {
            let mut next_index = *i + 1;
            let announcement = continued_file_line(root, lines, &mut next_index);
            if announcement
                .trim_start()
                .strip_prefix('(')
                .is_some_and(|tail| {
                    stack_file(root, tail).is_some() || unresolved_file_opening(tail)
                })
            {
                break;
            }
        }
        *i += 1;
        message.push(' ');
        message.push_str(next.trim());
    }
    message
}

fn continued_error_line<'a>(
    root: &Path,
    lines: &[&'a str],
    separator_lines: &[usize],
    i: &mut usize,
) -> Cow<'a, str> {
    let line = lines[*i];
    if line.len() < 60 || numbered_error(line).is_some() || message_boundary(line) {
        return Cow::Borrowed(line);
    }
    // Most long lines are ordinary compiler output. Look ahead only when a
    // possible file-line suffix exists nearby, avoiding repeated scans of
    // large typeout logs that contain no diagnostics.
    let next = separator_lines.partition_point(|index| *index <= *i);
    if separator_lines
        .get(next)
        .is_none_or(|index| index - *i > 64)
    {
        return Cow::Borrowed(line);
    }
    let mut joined = line.to_owned();
    for (index, next) in lines.iter().enumerate().skip(*i + 1).take(64) {
        // A suffix of a wrapped filename can itself look like a file-line
        // error. Only treat it as an independent diagnostic if it resolves.
        let numbered = numbered_error(next);
        if next.trim().is_empty()
            || numbered.is_some_and(|(name, _, _)| location(root, name).is_some())
            || numbered.is_none() && message_boundary(next)
            || joined.len() + next.len() > 8192
        {
            break;
        }
        joined.push_str(next);
        if numbered_error(&joined).is_some_and(|(name, _, _)| location(root, name).is_some()) {
            *i = index;
            return Cow::Owned(joined);
        }
    }
    Cow::Borrowed(line)
}

fn wrapped_error_message(
    root: &Path,
    lines: &[&str],
    separator_lines: &[usize],
    i: &mut usize,
    initial: &str,
) -> String {
    let mut message = initial.to_owned();
    for _ in 0..64 {
        // A wrap can land on a period inside a driver or filename. It is only
        // the end of a recognized file error once its closing text is present.
        let incomplete_file = [
            "LaTeX Error: File ",
            "I can't find file ",
            "Package pdftex.def Error: File ",
            "Package luatex.def Error: File ",
            "Unable to load picture or PDF file ",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix) || prefix.starts_with(&message))
            && missing_graphic_name(&message).is_none();
        if message.trim_end().ends_with(['.', '!']) && !incomplete_file || message.len() >= 4096 {
            break;
        }
        let Some(next) = lines.get(*i + 1) else { break };
        if next.trim().is_empty()
            || message_boundary(next)
            || [
                "<inserted text>",
                "<recently read>",
                "<to be read again>",
                "<argument>",
                "<*>",
                "See the LaTeX manual",
                "Type  H <return>",
            ]
            .iter()
            .any(|prefix| next.trim_start().starts_with(prefix))
        {
            break;
        }
        let mut next_index = *i + 1;
        if numbered_error(&continued_error_line(
            root,
            lines,
            separator_lines,
            &mut next_index,
        ))
        .is_some()
        {
            break;
        }
        if next.trim_start().starts_with('(') {
            let announcement = continued_file_line(root, lines, &mut next_index);
            if announcement
                .trim_start()
                .strip_prefix('(')
                .is_some_and(|tail| {
                    stack_file(root, tail).is_some() || unresolved_file_opening(tail)
                })
            {
                break;
            }
        }
        // TeX wraps at a byte count, including inside words and filenames.
        // Preserve the spaces it actually printed when removing the break.
        message.push_str(next);
        *i += 1;
    }
    message.trim().to_owned()
}

fn stack_file(root: &Path, tail: &str) -> Option<(PathBuf, usize)> {
    if let Some(quoted) = tail.strip_prefix('"') {
        let end = quoted.find('"')?;
        let name = &quoted[..end];
        return log_source(Path::new(name))
            .then(|| location(root, name))
            .flatten()
            .map(|path| (path, end + 2));
    }
    // TeX also emits unquoted names containing spaces or parentheses. Require
    // an existing source file rather than splitting the name at whitespace.
    let mut result = None;
    for (dot, _) in tail
        .match_indices('.')
        .take_while(|(index, _)| *index < 4096)
    {
        for extension in log_extensions() {
            let end = dot + 1 + extension.len();
            if tail.get(dot + 1..end) != Some(extension)
                || tail[end..]
                    .starts_with(|ch: char| !ch.is_whitespace() && !matches!(ch, '(' | ')' | '['))
            {
                continue;
            }
            if let Some(path) = location(root, &tail[..end]) {
                result = Some((path, end));
            }
        }
    }
    result
}

#[derive(Clone, PartialEq, Eq)]
struct LogSource {
    path: PathBuf,
    confirmed: bool,
}

/// A recognizable source name still blocks its parent when that file has
/// disappeared before analysis. Generic parentheses do not change sources.
fn unresolved_stack_file(tail: &str) -> Option<usize> {
    if let Some(quoted) = tail.strip_prefix('"') {
        let end = quoted.bytes().take(4096).position(|byte| byte == b'"')?;
        return log_source(Path::new(&quoted[..end])).then_some(end + 2);
    }
    let mut result = None;
    for (dot, _) in tail
        .as_bytes()
        .iter()
        .take(4096)
        .enumerate()
        .filter(|(_, byte)| **byte == b'.')
    {
        for extension in log_extensions() {
            let end = dot + 1 + extension.len();
            if tail.get(dot + 1..end) != Some(extension)
                || tail[end..]
                    .starts_with(|ch: char| !ch.is_whitespace() && !matches!(ch, '(' | ')' | '['))
            {
                continue;
            }
            // Do not swallow an ordinary parenthesis followed by a separate
            // file entry on the same line. Balanced parentheses in a name,
            // and arbitrary parentheses inside quoted names, remain valid.
            let mut depth = 0usize;
            let balanced = tail[..end].bytes().all(|byte| match byte {
                b'(' => {
                    depth += 1;
                    true
                }
                b')' => {
                    if depth == 0 {
                        false
                    } else {
                        depth -= 1;
                        true
                    }
                }
                _ => true,
            });
            if balanced && depth == 0 {
                result = Some(end);
            }
        }
    }
    result
}

// A closed parenthesis mentioning a missing .tex file can be warning prose.
// It opens no persistent context; retain it in the surrounding message.
fn unresolved_file_opening(tail: &str) -> bool {
    let Some(end) = unresolved_stack_file(tail) else {
        return false;
    };
    let tail = &tail[end..];
    let mut depth = 1usize;
    for (index, byte) in tail.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return tail[index + 1..].contains('(');
                }
            }
            _ => {}
        }
    }
    true
}

fn update_file_stack(
    line: &str,
    root: &Path,
    stack: &mut Vec<Option<LogSource>>,
    sources: &mut BTreeSet<Url>,
) {
    let mut i = 0;
    while i < line.len() {
        match line.as_bytes()[i] {
            b'(' => {
                let tail = &line[i + 1..];
                let unresolved = unresolved_stack_file(tail);
                let resolved = stack_file(root, tail)
                    .filter(|(_, length)| unresolved.is_none_or(|longer| longer <= *length));
                if let Some((file, length)) = resolved {
                    if sources.len() < 4096 && crate::workspace::is_source(&file) {
                        if let Ok(uri) = Url::from_file_path(&file) {
                            sources.insert(uri);
                        }
                    }
                    stack.push(Some(LogSource {
                        path: file,
                        confirmed: true,
                    }));
                    i += length;
                } else if let Some(length) = unresolved {
                    stack.push(Some(LogSource {
                        path: root.into(),
                        confirmed: false,
                    }));
                    i += length;
                } else {
                    stack.push(None);
                }
            }
            b')' => {
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
}

fn continued_file_line<'a>(root: &Path, lines: &[&'a str], i: &mut usize) -> Cow<'a, str> {
    let line = lines[*i];
    // TeX wraps file announcements at max_print_line (normally 79). Join only
    // when the continuation completes a file, with bounded lookahead. Explicit
    // unresolved paths retain their source barrier after byte wrapping too.
    if line.len() >= 60 {
        for (opening, _) in line.match_indices('(') {
            let tail = &line[opening + 1..];
            let complete =
                stack_file(root, tail).is_some() || unresolved_stack_file(tail).is_some();
            // A closing quote proves the filename ended. An unquoted name at
            // the print boundary can still continue, even when its prefix is
            // also an existing file. Prefer an actual longer continuation.
            if complete && tail.starts_with('"') {
                continue;
            }
            let explicit_path =
                tail.starts_with(['"', '/']) || tail.starts_with("./") || tail.starts_with("../");
            let mut joined = line.to_owned();
            let mut plain_continuation = true;
            for (next_index, next) in lines.iter().enumerate().skip(*i + 1).take(64) {
                if next.is_empty()
                    || *next == crate::output::TRUNCATED
                    || warning_start(next)
                    || next.starts_with("! ")
                    || numbered_error(next).is_some()
                    || joined.len() + next.len() > 8192
                {
                    break;
                }
                joined.push_str(next);
                plain_continuation &= !next.chars().any(|ch| ch.is_whitespace() || ch == ':');
                let tail = &joined[opening + 1..];
                if stack_file(root, tail)
                    .is_some_and(|(_, length)| length > line.len() - opening - 1)
                    || explicit_path
                        && (!complete || plain_continuation)
                        && unresolved_stack_file(tail)
                            .is_some_and(|length| length > line.len() - opening - 1)
                {
                    *i = next_index;
                    return Cow::Owned(joined);
                }
            }
        }
    }
    Cow::Borrowed(line)
}

pub fn parse(root: &Path, log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    parse_with_sources(root, log).0
}

/// The recorder also lists existence probes. File-entry events in the final
/// log provide narrower evidence for diagnostics with no source line.
pub(crate) fn parse_with_sources(
    root: &Path,
    log: &str,
) -> (BTreeMap<Url, Vec<Diagnostic>>, BTreeSet<Url>) {
    let mut result = BTreeMap::new();
    let mut sources = BTreeSet::new();
    let root_path = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if let Ok(uri) = Url::from_file_path(root_path) {
        sources.insert(uri);
    }
    let mut stack: Vec<Option<LogSource>> = Vec::new();
    let mut fallback_confirmed = true;
    let mut pending: Option<(LogSource, String, usize)> = None;
    let lines = log.lines().collect::<Vec<_>>();
    let separator_lines: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let bytes = line.as_bytes();
            line.match_indices(':')
                .any(|(offset, _)| {
                    offset == 0
                        || bytes[offset - 1].is_ascii_digit()
                        || bytes.get(offset + 1).is_some_and(u8::is_ascii_digit)
                })
                .then_some(index)
        })
        .collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line == crate::output::TRUNCATED {
            if let Some((path, message, _)) = pending.take() {
                emit_with_context(
                    &mut result,
                    &path.path,
                    1,
                    message,
                    DiagnosticSeverity::ERROR,
                    None,
                    path.confirmed,
                );
            }
            stack.clear();
            fallback_confirmed = false;
            i += 1;
            continue;
        }
        let current = || {
            stack
                .iter()
                .rev()
                .find_map(Option::as_ref)
                .cloned()
                .unwrap_or_else(|| LogSource {
                    path: root.into(),
                    confirmed: fallback_confirmed,
                })
        };
        let mut structural = false;
        let diagnostic_start = i;
        let error_line = continued_error_line(root, &lines, &separator_lines, &mut i);
        if let Some((name, number, message)) = numbered_error(&error_line) {
            let reported_path = location(root, name);
            let confirmed_path = reported_path.is_some();
            let path = reported_path.unwrap_or_else(|| root.into());
            let message = wrapped_error_message(root, &lines, &separator_lines, &mut i, message);
            if let Some((cause_path, cause, context_start)) = pending.take() {
                // File lookup prompts end with a separate Emergency stop. The
                // numbered error supplies the missing location of its cause.
                if cause_path.path != path
                    || cause_path.confirmed != confirmed_path
                    || cause != message
                {
                    let associated = confirmed_path
                        && cause_path.confirmed
                        && cause_path.path == path
                        && message == "Emergency stop."
                        && missing_file_name(&cause).is_some()
                        && uninterrupted_file_prompt(&lines, context_start, diagnostic_start);
                    let cause_line = if associated { number } else { 1 };
                    let context = associated
                        .then(|| following_error_location(&lines, i + 1))
                        .flatten()
                        .filter(|(line, _)| *line == number)
                        .and_then(|(_, context)| context);
                    emit_with_context(
                        &mut result,
                        &cause_path.path,
                        cause_line,
                        cause,
                        DiagnosticSeverity::ERROR,
                        context.map(ErrorContext::source),
                        cause_path.confirmed,
                    );
                }
            }
            let severity = if message.contains("Warning:")
                || warning_start(&message)
                || message.starts_with("Missing character:")
                || box_code(&message).is_some()
                || missing_include_key(&message).is_some()
            {
                DiagnosticSeverity::WARNING
            } else {
                DiagnosticSeverity::ERROR
            };
            // An unresolved reported source falls back to the root for display.
            // Its context must not identify coincidentally matching root text.
            let context = confirmed_path
                .then(|| error_context(&lines, i + 1, number, &message))
                .flatten();
            emit_with_context(
                &mut result,
                &path,
                number,
                message,
                severity,
                context,
                confirmed_path,
            );
        } else if let Some(message) = line.strip_prefix("! ") {
            let message = wrapped_error_message(root, &lines, &separator_lines, &mut i, message);
            if let Some((path, cause, context_start)) = pending.take() {
                let (cause_line, context) = if path.confirmed
                    && path == current()
                    && message == "Emergency stop."
                    && missing_file_name(&cause).is_some()
                    && uninterrupted_file_prompt(&lines, context_start, diagnostic_start)
                {
                    following_error_location(&lines, i + 1).unwrap_or((1, None))
                } else {
                    (1, None)
                };
                emit_with_context(
                    &mut result,
                    &path.path,
                    cause_line,
                    cause,
                    DiagnosticSeverity::ERROR,
                    context.map(ErrorContext::source),
                    path.confirmed,
                );
            }
            pending = Some((current(), message, i + 1));
        } else if let Some(tail) = line.trim_start().strip_prefix("l.") {
            let number = tail
                .split(|ch: char| !ch.is_ascii_digit())
                .next()
                .and_then(|value| value.parse().ok());
            if let (Some(number), Some((path, message, context_start))) = (number, pending.take()) {
                let context = path
                    .confirmed
                    .then(|| error_context(&lines, context_start, number, &message))
                    .flatten();
                emit_with_context(
                    &mut result,
                    &path.path,
                    number,
                    message,
                    DiagnosticSeverity::ERROR,
                    context,
                    path.confirmed,
                );
            }
        } else if warning_start(line) || line.starts_with("Missing character:") {
            let path = current();
            // TeX wraps warnings at its configured print width.
            let message = wrapped_message(root, &lines, &separator_lines, &mut i);
            let number = input_line(&message).unwrap_or(1);
            emit_with_context(
                &mut result,
                &path.path,
                number,
                message,
                DiagnosticSeverity::WARNING,
                None,
                path.confirmed,
            );
        } else if line.starts_with("No file ") {
            let path = current();
            if let Some(name) = path
                .confirmed
                .then(|| missing_include_file(&path.path, &lines, &mut i))
                .flatten()
            {
                emit(
                    &mut result,
                    &path.path,
                    1,
                    format!("Included file `{name}` not found"),
                    DiagnosticSeverity::WARNING,
                );
            }
        } else if line.starts_with("Overfull ") || line.starts_with("Underfull ") {
            let path = current();
            emit_with_context(
                &mut result,
                &path.path,
                input_line(line).unwrap_or(1),
                line.into(),
                DiagnosticSeverity::WARNING,
                None,
                path.confirmed,
            );
        } else {
            structural = true;
        }
        // User messages and echoed TeX source can contain unmatched parentheses.
        // They describe diagnostics, rather than file entry/exit events.
        if structural {
            let line = continued_file_line(root, &lines, &mut i);
            update_file_stack(&line, root, &mut stack, &mut sources);
        }
        i += 1;
    }
    if let Some((path, message, _)) = pending {
        emit_with_context(
            &mut result,
            &path.path,
            1,
            message,
            DiagnosticSeverity::ERROR,
            None,
            path.confirmed,
        );
    }
    for diagnostics in result.values_mut() {
        diagnostics.sort_by_key(|diagnostic| (diagnostic.range.start, diagnostic.message.clone()));
        diagnostics.dedup_by(|a, b| {
            a.range == b.range && a.message == b.message && a.code == b.code && a.data == b.data
        });
    }
    (result, sources)
}

/// BibTeX and Biber write a separate log, often after TeX has already produced
/// a PDF. Preserve those errors instead of hiding them behind latexmk's exit code.
pub fn parse_bibliography(root: &Path, log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    parse_bibliography_log(root, &root.with_extension("blg"), log)
}

pub fn parse_bibliography_log(
    root: &Path,
    log_path: &Path,
    log: &str,
) -> BTreeMap<Url, Vec<Diagnostic>> {
    let paths = crate::recorder::BibliographyPaths::new(root, log_path, log);
    parse_bibliography_in(root, &paths, log)
}

pub(crate) fn parse_bibliography_in(
    root: &Path,
    paths: &crate::recorder::BibliographyPaths<'_>,
    log: &str,
) -> BTreeMap<Url, Vec<Diagnostic>> {
    let mut result = BTreeMap::new();
    let databases: Vec<_> = log
        .lines()
        .filter_map(crate::recorder::bibliography_source)
        .filter(|name| name.trim_matches('"').ends_with(".bib"))
        .filter_map(|name| paths.resolve(name))
        .collect();
    let mut previous = "";
    for line in log.lines() {
        if line == crate::output::TRUNCATED {
            previous = "Error context omitted from a large bibliography log";
            continue;
        }
        if let Some((message, tail)) = line.split_once("---line ") {
            if let Some((number, name)) = tail.split_once(" of file ") {
                if let Ok(number) = number.trim().parse::<u32>() {
                    // The logged input name identifies its role. A source
                    // alias can resolve to any suffix; an auxiliary alias
                    // must not become a source because its target is .bib.
                    let location = matches!(
                        Path::new(name.trim().trim_matches('"'))
                            .extension()
                            .and_then(|ext| ext.to_str()),
                        Some("tex" | "bib" | "bst" | "sty" | "cls")
                    )
                    .then(|| paths.resolve(name))
                    .flatten();
                    let (path, number) = location
                        .map(|path| (path, number))
                        .unwrap_or_else(|| (root.into(), 1));
                    let message = if message.trim().is_empty() {
                        previous
                    } else {
                        message
                    };
                    emit(
                        &mut result,
                        &path,
                        number,
                        format!("BibTeX: {}", message.trim()),
                        DiagnosticSeverity::ERROR,
                    );
                    continue;
                }
            }
        }
        if !line.trim().is_empty() {
            previous = line.trim();
        }
        if let Some((message, _)) = line.split_once("---while reading file ") {
            emit(
                &mut result,
                root,
                1,
                format!("BibTeX: {}", message.trim()),
                DiagnosticSeverity::ERROR,
            );
            continue;
        }
        let (message, severity, tool) = if let Some(message) = line.strip_prefix("Warning--") {
            (message, DiagnosticSeverity::WARNING, "BibTeX")
        } else if let Some((_, message)) = line.split_once("ERROR - ") {
            (message, DiagnosticSeverity::ERROR, "Biber")
        } else if let Some((_, message)) = line.split_once("WARN - ") {
            (message, DiagnosticSeverity::WARNING, "Biber")
        } else {
            continue;
        };
        let location = bibliography_path(paths, message, tool == "Biber");
        let filename = match &location {
            BibliographyLocation::Named { span, .. } => Some(span),
            BibliographyLocation::Inferred => None,
        };
        let named_entry = bibliography_entry(message, filename);
        let number = if tool == "Biber" {
            bibliography_line(message, filename)
        } else {
            // BibTeX reports source lines in its separate ---line format.
            // Warnings ending in `in key` preserve the entire literal key.
            None
        };
        let (explicit_path, databases, source_confirmed) = match location {
            BibliographyLocation::Inferred => (None, databases.as_slice(), true),
            // A named file restricts entry lookup even when it is missing.
            // Another database cannot supply its source range.
            BibliographyLocation::Named { path, .. } => {
                let confirmed = path.is_some();
                (path, &[][..], confirmed)
            }
        };
        let trailing_entry = message.rsplit_once(" in ").map(|(_, key)| key.trim());
        let key = if tool == "Biber" {
            match named_entry {
                BibliographyEntry::Absent => trailing_entry,
                BibliographyEntry::Key(span) => Some(&message[span]),
                BibliographyEntry::Unconfirmed(_) => None,
            }
        } else {
            // BibTeX appends the literal key, including any quotes in it.
            trailing_entry.or(match named_entry {
                BibliographyEntry::Key(span) => Some(&message[span]),
                _ => None,
            })
        };
        let mut entry = None;
        if let Some(key) = key {
            let candidates: Vec<_> = explicit_path
                .iter()
                .chain(databases.iter())
                .filter_map(|path| {
                    let text = Text::new(std::fs::read_to_string(path).ok()?);
                    let syntax = crate::syntax::Syntax::bibtex(&text);
                    let declaration = syntax.citations.iter().find(|entry| entry.name == key)?;
                    Some((
                        path.clone(),
                        text.range(declaration.span.start, declaration.span.end),
                    ))
                })
                .collect();
            if let Some(first) = candidates
                .first()
                .filter(|first| candidates.iter().all(|candidate| candidate == *first))
            {
                entry = Some(first.clone());
            }
        }
        let path = explicit_path
            .as_deref()
            .or_else(|| entry.as_ref().map(|(path, _)| path.as_path()))
            .unwrap_or(root);
        let diagnostic_message = format!("{tool}: {}", message.trim());
        emit_with_context(
            &mut result,
            path,
            if source_confirmed {
                number.unwrap_or(1)
            } else {
                1
            },
            diagnostic_message,
            severity,
            None,
            source_confirmed,
        );
        if number.is_none() {
            if let Some((_, range)) = &entry {
                if let Ok(uri) = Url::from_file_path(path) {
                    result.get_mut(&uri).unwrap().last_mut().unwrap().range = *range;
                }
            }
        }
    }
    for diagnostics in result.values_mut() {
        diagnostics.sort_by_key(|diagnostic| (diagnostic.range.start, diagnostic.message.clone()));
        diagnostics.dedup_by(|a, b| a.range == b.range && a.message == b.message);
    }
    result
}

/// Closing quotes must be separated from the rest of a literal name.
/// Apostrophes joined to more key text, including other quotes, are literal.
fn bibliography_quote_end(message: &str, start: usize) -> Option<usize> {
    let quote = message.as_bytes()[start] as char;
    message[start + 1..]
        .match_indices(quote)
        .map(|(index, _)| start + 1 + index)
        .find(|&index| {
            let tail = &message[index + 1..];
            tail.is_empty()
                || tail.starts_with(char::is_whitespace)
                || tail.starts_with([',', ')', '}'])
                || {
                    let after = tail.trim_start_matches(['.', ':', ';', ']', '!', '?']);
                    after.len() != tail.len()
                        && (after.is_empty() || after.starts_with(char::is_whitespace))
                }
        })
}

/// Metadata is only meaningful outside the complete source field and quotes.
fn bibliography_fields<'a>(
    message: &'a str,
    filename: Option<&'a ByteRange<usize>>,
) -> impl Iterator<Item = (usize, char)> + 'a {
    let mut quoted_until = 0;
    message.char_indices().filter(move |&(index, ch)| {
        if index < quoted_until || filename.is_some_and(|span| span.contains(&index)) {
            return false;
        }
        let word_start = !message[..index].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if word_start && matches!(ch, '\'' | '"') {
            quoted_until =
                bibliography_quote_end(message, index).map_or(message.len(), |end| end + 1);
            return false;
        }
        true
    })
}

enum BibliographyEntry {
    Absent,
    Key(ByteRange<usize>),
    Unconfirmed(ByteRange<usize>),
}

fn bibliography_entry(message: &str, filename: Option<&ByteRange<usize>>) -> BibliographyEntry {
    for (index, _) in bibliography_fields(message, filename) {
        let word_start = !message[..index].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if !word_start
            || !(message[index..].starts_with("entry ") || message[index..].starts_with("Entry "))
        {
            continue;
        }
        let start = index + 6;
        if !message[start..].starts_with(['\'', '"']) {
            continue;
        }
        return match bibliography_quote_end(message, start) {
            Some(end) => BibliographyEntry::Key(start + 1..end),
            None => BibliographyEntry::Unconfirmed(start + 1..message.len()),
        };
    }
    BibliographyEntry::Absent
}

fn bibliography_line(message: &str, filename: Option<&ByteRange<usize>>) -> Option<u32> {
    for (index, _) in bibliography_fields(message, filename) {
        let word_start = !message[..index].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if !word_start || !message[index..].starts_with("line ") {
            continue;
        }
        let tail = &message[index + 5..];
        let end = tail
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(tail.len());
        let after = &tail[end..];
        if after.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_')
            || after
                .strip_prefix('.')
                .is_some_and(|tail| tail.starts_with(|ch: char| ch.is_ascii_digit()))
        {
            continue;
        }
        if let Ok(number) = tail[..end].parse() {
            return Some(number);
        }
    }
    None
}

enum BibliographyLocation {
    Inferred,
    Named {
        path: Option<PathBuf>,
        /// Complete source field in the diagnostic, before canonicalization.
        span: ByteRange<usize>,
    },
}

fn bibliography_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    biber: bool,
) -> BibliographyLocation {
    // BibTeX's `in ...` warning suffix is the literal entry key. It can
    // contain quotes, spaces or .bib and can even name an existing file.
    if !biber && message.contains(" in ") {
        return BibliographyLocation::Inferred;
    }
    // In BibTeX warnings, `in 'entry'` names a literal bibliography key.
    let markers: &[&str] = if biber {
        &["file ", "File ", "in ", "In "]
    } else {
        &["file ", "File "]
    };
    let explicit = bibliography_fields(message, None).find_map(|(start, _)| {
        if message[..start].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
            return None;
        }
        markers.iter().find_map(|marker| {
            let tail = message[start..].strip_prefix(*marker)?.trim_start();
            let start = message.len() - tail.len();
            if tail.starts_with(['\'', '"']) {
                let (path, span) = bibliography_quoted_path(paths, message, start);
                Some(BibliographyLocation::Named { path, span })
            } else {
                bibliography_unquoted_path(paths, message, start)
            }
        })
    });
    if let Some(location) = explicit {
        return location;
    }
    // A reported entry key is literal metadata even when it names an
    // existing .bib file. Only text outside that field can name a source.
    let entry_span = match bibliography_entry(message, None) {
        BibliographyEntry::Key(span) | BibliographyEntry::Unconfirmed(span) => {
            Some(span.start - 1..(span.end + 1).min(message.len()))
        }
        BibliographyEntry::Absent => None,
    };
    let mut quoted_until = 0;
    for (index, ch) in message.char_indices() {
        if index < quoted_until
            || entry_span
                .as_ref()
                .is_some_and(|span| span.contains(&index))
        {
            continue;
        }
        let word_start = !message[..index].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if word_start && matches!(ch, '\'' | '"') {
            let (path, span) = bibliography_quoted_path(paths, message, index);
            if path.is_some() {
                return BibliographyLocation::Named { path, span };
            }
            let body = &message[index + 1..span.end];
            let filename = body
                .match_indices(ch)
                .any(|(end, _)| body[..end].ends_with(".bib"));
            // Preserve uncertain filenames as a whole, but keep independent
            // quoted message strings from hiding a bare path between them.
            quoted_until = if filename {
                span.end
            } else {
                bibliography_quote_end(message, index).map_or(message.len(), |end| end + 1)
            };
            continue;
        }
        if !message[index..].starts_with(".bib") {
            continue;
        }
        let prefix = &message[..index];
        let token_start = prefix
            .char_indices()
            .rev()
            .find(|&(_, ch)| ch.is_whitespace() || matches!(ch, ':' | '(' | '[' | '{'))
            .map_or(0, |(index, ch)| index + ch.len_utf8());
        if let Some(location @ BibliographyLocation::Named { path: Some(_), .. }) =
            bibliography_unquoted_path(paths, message, token_start)
        {
            return location;
        }
    }
    BibliographyLocation::Inferred
}

fn bibliography_unquoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> Option<BibliographyLocation> {
    let body = &message[start..];
    let token_end = body.find(char::is_whitespace).unwrap_or(body.len());
    // A whole filename takes precedence over internal .bib fragments or
    // punctuation. A missing explicit source still restricts entry lookup.
    let mut extensions = body[..token_end].rmatch_indices(".bib").peekable();
    extensions.peek()?;
    let end = extensions.find_map(|(index, _)| {
        let end = index + 4;
        let tail = &body[end..];
        let terminated = tail.is_empty()
            || tail.starts_with(|ch: char| {
                ch.is_whitespace() || matches!(ch, ',' | ':' | ';' | ')' | ']' | '}')
            })
            || tail
                .strip_prefix('.')
                .is_some_and(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace));
        terminated.then_some(end)
    });
    let (path, end) = match end {
        Some(end) => {
            let leading = body[..end]
                .find(|ch| !matches!(ch, '(' | '[' | '{'))
                .unwrap_or(end);
            let mut after = &body[end..];
            let wrapped = body[..leading].chars().rev().all(|open| {
                let close = match open {
                    '(' => ')',
                    '[' => ']',
                    _ => '}',
                };
                if let Some(tail) = after.strip_prefix(close) {
                    after = tail;
                    true
                } else {
                    false
                }
            });
            // Strip only balanced message wrappers; an unmatched opener
            // can be a literal filename character and must not be borrowed.
            if wrapped {
                (paths.resolve(&body[leading..end]), body.len() - after.len())
            } else {
                (paths.resolve(&body[..end]), end)
            }
        }
        None => (None, token_end),
    };
    Some(BibliographyLocation::Named {
        path,
        span: start..start + end,
    })
}

fn bibliography_quoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> (Option<PathBuf>, ByteRange<usize>) {
    let quote = message.as_bytes()[start] as char;
    let body = &message[start + 1..];
    let resolve = |name: &str| {
        name.ends_with(".bib")
            .then(|| paths.resolve(name))
            .flatten()
    };
    let mut candidate = None;
    let mut ambiguous = false;
    let terminated = |tail: &str| {
        let after = tail.trim_start_matches([',', ':', ';', ')', ']', '}', '.']);
        tail.is_empty()
            || (after.len() != tail.len()
                && (after.is_empty() || after.starts_with(char::is_whitespace)))
    };
    for (end, _) in body.match_indices(quote) {
        let tail = &body[end + 1..];
        // Message punctuation must itself end at a separator. For example,
        // the quote in refs.bib':copy.bib is still part of the filename.
        // A whitespace-only boundary is tentative unless it introduces a
        // complete line field: `line 700 copy.bib` is still name text.
        let line = tail.trim_start().strip_prefix("line ").is_some_and(|tail| {
            let end = tail
                .find(|ch: char| !ch.is_ascii_digit())
                .unwrap_or(tail.len());
            end > 0 && terminated(tail[end..].trim_start())
        });
        let strong = terminated(tail) || line;
        if strong {
            return (resolve(&body[..end]), start..start + end + 2);
        }
        if tail.starts_with(char::is_whitespace) {
            ambiguous |= candidate.is_some();
            candidate = Some(&body[..end]);
        }
    }
    let end = if ambiguous {
        message.len()
    } else {
        candidate.map_or(message.len(), |name| start + name.len() + 2)
    };
    (
        if ambiguous {
            None
        } else {
            candidate.and_then(resolve)
        },
        start..end,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_sources_track_closed_nested_and_wrapped_file_entries_not_echoed_messages() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let long = format!("章节-{}.tex", "segment".repeat(20));
        for name in [
            "main.tex",
            "one.tex",
            "two.tex",
            "ignored.tex",
            "package.sty",
            "main.aux",
            long.as_str(),
        ] {
            std::fs::write(root.join(name), "Text\n").unwrap();
        }
        let entry = format!("(\"./{long}\")");
        let split = entry
            .char_indices()
            .map(|(at, _)| at)
            .find(|at| *at >= 79)
            .unwrap();
        let log = format!("(./main.tex\n(./one.tex)(./two.tex)\n(./package.sty) (./main.aux)\n{}\n{}\nLaTeX Warning: Do not read (./ignored.tex).\nl.3 (./ignored.tex)\n)\n", &entry[..split], &entry[split..]);
        let (_, sources) = parse_with_sources(&main, &log);
        let expected: BTreeSet<_> = [
            "main.tex",
            "one.tex",
            "two.tex",
            "package.sty",
            long.as_str(),
        ]
        .into_iter()
        .map(|name| Url::from_file_path(root.join(name)).unwrap())
        .collect();
        assert_eq!(sources, expected);
    }

    #[test]
    fn truncated_logs_keep_only_the_root_and_source_entries_actually_observed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        for name in ["main.tex", "observed.tex", "not-seen.tex"] {
            std::fs::write(root.join(name), "Text\n").unwrap();
        }
        let log = format!(
            "(./not-se\n{}\n(./observed.tex)\n",
            crate::output::TRUNCATED
        );
        let (_, sources) = parse_with_sources(&main, &log);
        assert_eq!(
            sources,
            [
                Url::from_file_path(&main).unwrap(),
                Url::from_file_path(root.join("observed.tex")).unwrap()
            ]
            .into()
        );
    }

    #[test]
    fn duplicate_destination_warnings_select_declarations_and_preserve_shipout_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\hypertarget{section.1}{Target}\\label{section.1}\\hyperlink{section.1}{Link}\r\n\\hyperdef[reference-label]{section}% keep\r\n{1}{Target}\r\n";
        std::fs::write(&main, source).unwrap();
        let doc = crate::workspace::Document::new(
            &Url::from_file_path(&main).unwrap(),
            source.into(),
            None,
        );
        for message in [
            "pdfTeX warning (ext4): destination with the same identifier (name{section.1}) has been already used, duplicate ignored",
            "warning  (pdf backend): ignoring duplicate destination with the name 'section.1'",
        ] {
            let log = format!("{message}\n<argument> ...shipout:D \\box_use:N \\l_shipout_box\nl.4 \\end{{document}}\nLaTeX Warning: Keep.\n");
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            let duplicates: Vec<_> = items.iter().filter(|item| item.code == Some(NumberOrString::String("duplicate-pdf-destination".into()))).collect();
            assert_eq!(duplicates.len(), 2, "{items:?}");
            assert_eq!(items.len(), 3, "{items:?}");
            for (item, entry) in duplicates.iter().zip(&doc.syntax.anchors) {
                assert_eq!(item.range, doc.text.range(entry.span.start, entry.span.end));
                assert_eq!(item.message, message);
                assert_eq!(item.data, Some(serde_json::json!({"destination":"section.1"})));
            }
        }
    }

    #[test]
    fn duplicate_destination_messages_recover_byte_wraps_without_normalizing_name_spaces() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("section.{}", "long".repeat(25));
        std::fs::write(&main, format!("\\hypertarget{{{name}}}{{Target}}\n")).unwrap();
        for (prefix, suffix) in [
            (
                "pdfTeX warning (ext4): destination with the same identifier (name{",
                "}) has been already used, duplicate ignored",
            ),
            (
                "warning  (pdf backend): ignoring duplicate destination with the name '",
                "'",
            ),
        ] {
            let message = format!("{prefix}{name}{suffix}");
            for split in prefix.len()..message.len() {
                let log = format!(
                    "{}\n{}\n<argument> shipout\nl.1 text\n",
                    &message[..split],
                    &message[split..]
                );
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 1, "split {split}: {items:?}");
                assert_eq!(items[0].message, message, "split {split}");
                assert_eq!(
                    items[0].data,
                    Some(serde_json::json!({"destination":name})),
                    "split {split}"
                );
            }
            let spaced = format!("{prefix} {name}{suffix}");
            assert!(destination_warning(&spaced).is_none());
        }
        for message in [
            "pdfTeX warning (ext4): destination with the same identifier (name{target}) has been already used",
            "warning  (pdf backend): ignoring duplicate destination with the name '\\dynamic'",
        ] {
            assert!(destination_warning(message).is_none());
        }
    }

    #[test]
    fn destination_warning_byte_wraps_preserve_names_and_stop_at_resource_output() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("section.{}.!{}", "long".repeat(25), "tail".repeat(15));
        let source = format!("\\hyperlink{{{name}}}{{Text}}\n");
        std::fs::write(&main, &source).unwrap();
        for (prefix, suffix) in [
            (
                "pdfTeX warning (dest): name{",
                "} has been referenced but does not exist, replaced by a fixed one",
            ),
            (
                "warning  (pdf backend): unreferenced destination with name '",
                "'",
            ),
        ] {
            let message = format!("{prefix}{name}{suffix}");
            // Include breaks immediately after punctuation and inside the
            // explanatory words, not just at convenient token boundaries.
            for split in prefix.len()..message.len() {
                let log = format!(
                    "{}\n{}\n</fonts/resource.otf>\nLaTeX Warning: Next warning.\n",
                    &message[..split],
                    &message[split..]
                );
                let diagnostics = parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2, "split {split}: {items:?}");
                let item = items
                    .iter()
                    .find(|item| item.message != "LaTeX Warning: Next warning.")
                    .unwrap();
                assert_eq!(item.message, message, "split {split}");
                assert_eq!(
                    item.data,
                    Some(serde_json::json!({"destination":name})),
                    "split {split}"
                );
                assert_eq!(
                    item.range,
                    Text::new(source.clone()).range(11, 11 + name.len())
                );
            }
        }
    }

    #[test]
    fn destination_wraps_preserve_real_whitespace_and_incomplete_message_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "\\hyperlink{section.target}{Text}\n").unwrap();
        for message in [
            "pdfTeX warning (dest): name{section. target} has been referenced but does not exist, replaced by a fixed one",
            "warning  (pdf backend): unreferenced destination with name 'section. target'",
        ] {
            let split = message.find(" target").unwrap();
            let log = format!("{}\n{}\n</fonts/resource.otf>\n", &message[..split], &message[split..]);
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 1, "{items:?}");
            assert_eq!(items[0].message, message);
            assert!(items[0].code.is_none());
            assert!(items[0].data.is_none());
        }
        for boundary in [
            "</fonts/resource.otf>",
            "LaTeX Warning: Next warning.",
            "! Next error.",
            crate::output::TRUNCATED,
            "(./main.tex",
        ] {
            let log = format!("pdfTeX warning (dest): name{{section.\n{boundary}\n");
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            let item = items
                .iter()
                .find(|item| item.message.starts_with("pdfTeX warning"))
                .unwrap();
            assert_eq!(item.message, "pdfTeX warning (dest): name{section.");
            assert!(item.code.is_none());
        }
    }

    #[test]
    fn unicode_destinations_survive_log_breaks_inside_utf8_characters() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("section.{}终点", "章节".repeat(20));
        std::fs::write(&main, format!("\\hyperlink{{{name}}}{{Text}}\n")).unwrap();
        let message = format!("pdfTeX warning (dest): name{{{name}}} has been referenced but does not exist, replaced by a fixed one");
        for split in "pdfTeX warning (dest): name{".len()..message.len() {
            let log = [
                &message.as_bytes()[..split],
                b"\n",
                &message.as_bytes()[split..],
                b"\n</fonts/resource.pfb>\n",
            ]
            .concat();
            let diagnostics = parse(&main, &decode_output(&log));
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 1, "split {split}: {items:?}");
            assert_eq!(
                items[0].data,
                Some(serde_json::json!({"destination":name})),
                "split {split}: {items:?}"
            );
        }
    }

    #[test]
    fn lua_destination_apostrophes_at_print_width_do_not_target_a_different_shorter_name() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let prefix = "warning  (pdf backend): unreferenced destination with name '";
        for width in [79, 80] {
            let shorter = "x".repeat(width - prefix.len() - 1);
            let name = format!("{shorter}'suffix");
            std::fs::write(
                &main,
                format!("\\hyperlink{{{name}}}{{Long}}\\hyperlink{{{shorter}}}{{Short}}\n"),
            )
            .unwrap();
            let message = format!("{prefix}{name}'");
            let log = format!(
                "{}\n{}\n</fonts/resource.otf>\n",
                &message[..width],
                &message[width..]
            );
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 1, "{items:?}");
            assert_eq!(items[0].message, message);
            assert_eq!(items[0].data, Some(serde_json::json!({"destination":name})));
        }
    }

    #[test]
    fn destination_warnings_locate_direct_and_compound_links_without_inventing_lines() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\hyperlink{section.lost}{Text}\\label{section.lost}\r\n\\hyperref{}{section}% keep\r\n{lost}{Link}\r\n\\hyperref{remote}{section}{lost}{Remote}\n\\verb|\\hyperlink{section.lost}{Ignored}|\n";
        std::fs::write(&main, source).unwrap();
        let text = Text::new(source.into());
        let first = source.find("section.lost").unwrap();
        let second = source.find("{section}").unwrap() + 1;
        let last = source.find("{lost}").unwrap() + 5;
        for warning in [
            "pdfTeX warning (dest): name{section.lost} has been referenced but does not exis\nt, replaced by a fixed one",
            "warning  (pdf backend): unreferenced destination with name 'section.lost'",
        ] {
            let log = format!("(./main.tex\n{warning}\n<font/resource.otf>\n! Other error.\nl.1 bad\n)\n");
            let diagnostics = parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 3, "{items:?}");
            let references: Vec<_> = items.iter().filter(|item| item.code == Some(NumberOrString::String("undefined-pdf-destination".into()))).collect();
            assert_eq!(references.len(), 2, "{items:?}");
            assert_eq!(references[0].range, text.range(first, first + 12));
            assert_eq!(references[1].range, text.range(second, last));
            assert!(references.iter().all(|item| item.data == Some(serde_json::json!({"destination":"section.lost"})) && !item.message.contains("font/")));
        }
    }

    #[test]
    fn destination_warning_names_preserve_semantic_spaces_and_reject_uncertain_messages() {
        for name in ["lost", "section.foo", "章节.目标"] {
            for message in [format!("pdfTeX warning (dest): name{{{name}}} has been referenced but does not exist , replaced by a fixed one"), format!("warning  (pdf backend): unreferenced destination with name '{name}'")] {
                assert_eq!(undefined_destination_name(&message), Some(name));
            }
        }
        for message in [
            "pdfTeX warning (dest): name{a b} has been referenced but does not exist, replaced by a fixed one",
            "warning  (pdf backend): unreferenced destination with name ' lost'",
            "warning  (pdf backend): unreferenced destination with name 'lost ' ",
            "pdfTeX warning (dest): name{lost} has been referenced",
            "warning  (pdf backend): unreferenced destination with name '\\dynamic'",
            "pdfTeX warning (dest): name{lost} has been referenced but was found",
        ] {
            assert_eq!(undefined_destination_name(message), None, "{message}");
        }
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "First line\n\\hyperlink{\\dynamic}{Text}\n").unwrap();
        let diagnostics = parse(&main, "warning  (pdf backend): unreferenced destination with name 'unknown'\n<font/resource.otf>\n");
        let items = &diagnostics[&Url::from_file_path(main).unwrap()];
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none());
        assert!(items[0].data.is_none());
        assert!(!items[0].message.contains("font/"));
    }

    #[test]
    fn final_pdf_page_counts_support_engine_variants_and_wrapped_summaries() {
        for (log, expected) in [
            ("Output written on main.pdf (1 page, 123 bytes).\n", Some(1)),
            ("Output written on main.pdf (12 pages).\n", Some(12)),
            ("Output written on /long/path (draft)/\nmain.pdf (2 pa\nges, 3456 bytes).\n", Some(2)),
            ("Output written on main.pdf (3 pages, 123 bytes).\nOutput written on main.pdf (4 pages, 456 bytes).\n", Some(4)),
            ("Writing main.pdf\n", None),
            ("Output written on main.pdf (0 pages, 0 bytes).\n", None),
            ("Output written on main.pdf (999999999999 pages, 9 bytes).\n", None),
            ("Output written on main.pdf (2 pages, invalid bytes).\n", None),
        ] {
            assert_eq!(page_count(log), expected, "{log}");
        }
    }

    #[test]
    fn tex_byte_wrapping_recovers_split_utf8_without_joining_valid_or_malformed_lines() {
        for scalar in ["é", "中", "😀"] {
            for split in 1..scalar.len() {
                for newline in [b"\n".as_slice(), b"\r\n".as_slice()] {
                    let bytes = [
                        b"No file dir/",
                        &scalar.as_bytes()[..split],
                        newline,
                        &scalar.as_bytes()[split..],
                        b".tex.\n",
                    ]
                    .concat();
                    assert_eq!(
                        decode_output(&bytes),
                        format!("No file dir/{scalar}.tex.\n")
                    );
                }
            }
        }
        assert_eq!(decode_output("中文\n😀\n".as_bytes()), "中文\n😀\n");
        for bytes in [
            b"\xff\n\x80".as_slice(),
            b"\xe0\x80\n\x80".as_slice(),
            b"\xe4\nA".as_slice(),
            b"\xe4\n\n\xb8\xad".as_slice(),
        ] {
            assert_eq!(decode_output(bytes), String::from_utf8_lossy(bytes));
        }
    }

    #[test]
    #[cfg(unix)]
    fn bibliography_line_errors_follow_source_aliases_with_unrelated_target_extensions() {
        for extension in ["payload", "png", "tex", "bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let database = root.join(format!("database.{extension}"));
            let style = root.join(format!("style.{extension}"));
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(
                &database,
                "% database\n@book{entry,title={Book}}\n@book{entry,title={Duplicate}}\n",
            )
            .unwrap();
            std::fs::write(&style, "ENTRY {title} {} {}\nFUNCTION {broken} {unknown}\n").unwrap();
            std::os::unix::fs::symlink(&database, root.join("with spaces.bib")).unwrap();
            std::os::unix::fs::symlink(&style, root.join("local.bst")).unwrap();
            let diagnostics = parse_bibliography(&main, "Database file #1: with spaces.bib\nThe style file: local.bst\nRepeated entry---line 3 of file \"with spaces.bib\"\nunknown is an unknown function---line 2 of file local.bst\nWarning--empty author in entry\n");
            let database_uri = Url::from_file_path(&database).unwrap();
            let style_uri = Url::from_file_path(&style).unwrap();
            assert_eq!(diagnostics.len(), 2, "{extension}: {diagnostics:?}");
            let errors = &diagnostics[&database_uri];
            assert_eq!(errors.len(), 2);
            assert!(errors
                .iter()
                .any(|item| item.message == "BibTeX: Repeated entry"
                    && item.range.start.line == 2
                    && item.severity == Some(DiagnosticSeverity::ERROR)));
            assert!(errors
                .iter()
                .any(|item| item.message == "BibTeX: empty author in entry"
                    && item.range.start == Position::new(1, 6)
                    && item.severity == Some(DiagnosticSeverity::WARNING)));
            assert_eq!(diagnostics[&style_uri][0].range.start.line, 1);
            assert_eq!(
                diagnostics[&style_uri][0].message,
                "BibTeX: unknown is an unknown function"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn bibliography_line_errors_do_not_treat_auxiliary_alias_targets_as_sources() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let physical = root.join("physical.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&physical, "% database\n@book{entry,\n title={Book}}\n").unwrap();
        for name in ["main.aux", "raw.csv"] {
            std::os::unix::fs::symlink(&physical, root.join(name)).unwrap();
        }
        std::fs::create_dir(root.join("directory.bst")).unwrap();
        for name in ["main.aux", "raw.csv", "missing.bib", "directory.bst"] {
            let log = format!("Error in auxiliary input---line 3 of file {name}\n");
            let diagnostics = parse_bibliography(&main, &log);
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items[0].range.start.line, 0);
            assert_eq!(items[0].message, "BibTeX: Error in auxiliary input");
        }
    }

    #[test]
    fn bibliography_diagnostics_use_the_tool_working_directory_for_database_and_style_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("build")).unwrap();
        for name in ["refs.bib", "build/refs.bib"] {
            std::fs::write(
                root.join(name),
                "% database\n@book{example,title={Example}}\n",
            )
            .unwrap();
        }
        for name in ["local.bst", "build/local.bst"] {
            std::fs::write(
                root.join(name),
                "ENTRY {title} {} {}\nFUNCTION {bad} {unknown}\n",
            )
            .unwrap();
        }
        for (auxiliary, prefix) in [("main.aux", "build/"), ("build/main.aux", "")] {
            let log = format!("The top-level auxiliary file: {auxiliary}\nDatabase file #1: refs.bib\nThe style file: local.bst\nWarning--empty author in example\nRepeated entry---line 2 of file refs.bib\nunknown is an unknown function---line 2 of file local.bst\n");
            let diagnostics =
                parse_bibliography_log(&root.join("main.tex"), &root.join("build/main.blg"), &log);
            let bib = Url::from_file_path(root.join(format!("{prefix}refs.bib"))).unwrap();
            let style = Url::from_file_path(root.join(format!("{prefix}local.bst"))).unwrap();
            assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
            assert_eq!(diagnostics[&bib].len(), 2);
            assert!(diagnostics[&bib]
                .iter()
                .all(|diagnostic| diagnostic.range.start.line == 1));
            assert_eq!(diagnostics[&style][0].range.start.line, 1);
        }
    }

    #[test]
    fn biber_diagnostic_paths_do_not_resolve_prefixes_of_longer_filenames() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "\\cite{entry}\nText.\n").unwrap();
        std::fs::write(
            root.join("refs.bib"),
            "% decoy\n@book{decoy,title={Wrong}}\n",
        )
        .unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        for name in [
            "refs.bib.backup",
            "refs.bib2",
            "refs.bib-archive",
            "refs.bib_archive",
            "refs.bib中文",
            "refs.bib/nonexistent.bib",
        ] {
            if !name.contains('/') {
                std::fs::write(
                    root.join(name),
                    "% actual input\n@book{entry,title={Book}}\n",
                )
                .unwrap();
            }
            for quoted in [false, true] {
                let name = if quoted {
                    format!("'{name}'")
                } else {
                    name.into()
                };
                let message = format!("syntax error in file {name}, line 2");
                let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
                assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:?}");
                let items = diagnostics
                    .get(&main_uri)
                    .unwrap_or_else(|| panic!("{name}: {diagnostics:?}"));
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].message, format!("Biber: {message}"));
            }
        }
    }

    #[test]
    fn biber_diagnostic_paths_keep_complete_names_before_message_punctuation() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% database\n@book{entry,title={Book}}\n").unwrap();
        for message in [
            "syntax error in file 'refs.bib', line 2",
            "syntax error in file \"refs.bib\", line 2",
            "syntax error in refs.bib, line 2",
            "refs.bib: syntax error at line 2",
            "syntax error in refs.bib; line 2",
            "syntax error (refs.bib) at line 2",
            "syntax error [refs.bib] at line 2",
            "syntax error in refs.bib. See line 2",
            "syntax error on line 2 of refs.bib.",
            "syntax error on line 2 of refs.bib",
        ] {
            let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
            assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items[0].message, format!("Biber: {message}"));
            assert_eq!(items[0].range.start.line, 1);
        }
    }

    #[test]
    fn biber_quoted_paths_keep_spaces_apostrophes_and_internal_bib_suffixes() {
        for name in [
            "refs.bib copy.bib",
            "refs.bib's copy.bib",
            "refs.bib' copy.bib",
            "中文 refs.bib copy's refs.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let target = root.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            for decoy in [
                "refs.bib",
                "copy.bib",
                "s copy.bib",
                "copy's refs.bib",
                "中文 refs.bib",
            ] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            std::fs::write(&target, "% database\n\n@book{entry,title={Correct}}\n").unwrap();
            for quote in ['\'', '"'] {
                let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - Error in {quote}{name}{quote} line 3\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\nWARN - Missing another field for entry 'entry' in file {quote}{name}{quote}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{name} {quote}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&target).unwrap()];
                assert_eq!(items.len(), 3);
                assert!(items.iter().all(|item| item.range.start.line == 2));
                for warning in items
                    .iter()
                    .filter(|item| item.severity == Some(DiagnosticSeverity::WARNING))
                {
                    assert_eq!(
                        warning.range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_quoted_paths_keep_punctuation_and_line_text_after_literal_quotes() {
        for suffix in [
            ":copy.bib",
            ";copy.bib",
            ")copy.bib",
            "]copy.bib",
            "}copy.bib",
            ",copy.bib",
            "),;copy.bib",
            " line 700 copy.bib",
            " line 700.copy.bib",
        ] {
            for quote in ['\'', '"'] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let name = format!("中文 refs.bib{quote}{suffix}");
                let target = root.join(&name);
                std::fs::write(&main, "\\cite{entry}\n").unwrap();
                for decoy in ["中文 refs.bib", "copy.bib"] {
                    std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
                }
                std::fs::write(&target, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
                for message in [
                    format!("syntax error in file {quote}{name}{quote}, line 3"),
                    format!("syntax error on line 3 in file {quote}{name}{quote}"),
                    format!("syntax error {quote}{name}{quote}: line 3"),
                ] {
                    let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - {message}\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    let items = diagnostics
                        .get(&Url::from_file_path(&target).unwrap())
                        .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                    assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                    assert_eq!(items.len(), 2);
                    assert!(
                        items
                            .iter()
                            .all(|item| item.range.start.line == 2 && !unresolved_location(item)),
                        "{log}: {items:?}"
                    );
                    let warning = items
                        .iter()
                        .find(|item| item.severity == Some(DiagnosticSeverity::WARNING))
                        .unwrap();
                    assert_eq!(
                        warning.range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
                std::fs::remove_file(&target).unwrap();
                let log = format!("INFO - Found BibTeX data source '中文 refs.bib'\nERROR - syntax error in file {quote}{name}{quote}, line 3\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items.len(), 2);
                assert!(
                    items.iter().all(|item| item
                        .data
                        .as_ref()
                        .is_some_and(|data| data["unresolvedSource"] == true)),
                    "{log}: {items:?}"
                );
            }
        }
    }

    #[test]
    fn biber_missing_quoted_paths_do_not_borrow_prefix_suffix_or_other_database_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let source = "\\cite{entry}\n";
        std::fs::write(&main, source).unwrap();
        for name in ["refs.bib", "copy.bib", "unrelated.bib"] {
            std::fs::write(root.join(name), "% decoy\n\n\n@book{entry,title={Wrong}}\n").unwrap();
        }
        std::fs::create_dir(root.join("directory.bib")).unwrap();
        for quote in ['\'', '"'] {
            for name in ["refs.bib missing copy.bib", "directory.bib"] {
                for closed in [false, true] {
                    let closing = if closed {
                        quote.to_string()
                    } else {
                        String::new()
                    };
                    let log = format!("INFO - Found BibTeX data source 'unrelated.bib'\nERROR - Error in {quote}{name}{closing} line 3\nWARN - Missing field for entry 'entry' (file {quote}{name}{closing})\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    assert_eq!(diagnostics.len(), 1, "{quote}: {diagnostics:?}");
                    let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                    assert_eq!(items.len(), 2);
                    let text = Text::new(source.into());
                    assert!(items
                        .iter()
                        .all(|item| item.range.start.line == 0
                            && text.offset(item.range.end).is_some()));
                    assert!(items.iter().all(|item| item
                        .data
                        .as_ref()
                        .is_some_and(|data| data["unresolvedSource"] == true)));
                }
            }
        }
    }

    #[test]
    fn biber_explicit_file_warnings_never_reuse_another_databases_entry_range() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let target = root.join("actual.bib");
        let contents = "@book{other,title={Actual}}\n";
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&target, contents).unwrap();
        std::fs::write(
            root.join("unrelated.bib"),
            "% other\n\n\n@book{entry,title={Wrong}}\n",
        )
        .unwrap();
        let log = "INFO - Found BibTeX data source 'unrelated.bib'\nWARN - Missing field for entry 'entry' (file 'actual.bib')\n";
        let diagnostics = parse_bibliography(&main, log);
        assert_eq!(diagnostics.len(), 1);
        let items = &diagnostics[&Url::from_file_path(&target).unwrap()];
        assert_eq!(items[0].range.start, Position::new(0, 0));
        assert!(Text::new(contents.into())
            .offset(items[0].range.end)
            .is_some());
    }

    #[test]
    fn bibtex_entry_keys_are_not_interpreted_as_namesake_source_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{'entry'}\n").unwrap();
        std::fs::write(root.join("entry.bib"), "@book{wrong,title={Decoy}}\n").unwrap();
        for key in ["'entry'", "'entry.bib'", "entry.bib"] {
            std::fs::write(&bib, format!("@book{{{key},title={{Book}}}}\n")).unwrap();
            let log =
                format!("Database file #1: refs.bib\nWarning--empty author and editor in {key}\n");
            let diagnostics = parse_bibliography(&main, &log);
            assert_eq!(diagnostics.len(), 1, "{key}: {diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items.len(), 1);
            assert_eq!(
                items[0].range,
                Range::new(
                    Position::new(0, 6),
                    Position::new(0, 6 + key.encode_utf16().count() as u32)
                )
            );
        }
    }

    #[test]
    fn biber_line_fields_ignore_complete_quoted_source_names() {
        for name in [
            "line 700 refs.bib",
            "outline 700 refs.bib",
            "line 700 author's refs.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n% end\n").unwrap();
            for quote in ['\'', '"'] {
                for message in [
                    format!("syntax error in file {quote}{name}{quote}, line 2"),
                    format!("syntax error on line 2 in file {quote}{name}{quote}"),
                ] {
                    let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - {message}\nWARN - Missing field for entry 'entry' (file {quote}{name}{quote})\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                    let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
                    assert_eq!(items.len(), 2);
                    assert!(
                        items
                            .iter()
                            .all(|item| item.range.start.line == 1 && !unresolved_location(item)),
                        "{message}: {items:?}"
                    );
                    let warning = items
                        .iter()
                        .find(|item| item.severity == Some(DiagnosticSeverity::WARNING))
                        .unwrap();
                    assert_eq!(
                        warning.range,
                        Range::new(Position::new(1, 6), Position::new(1, 11))
                    );
                }
            }
        }
    }

    #[test]
    fn biber_line_fields_require_unquoted_complete_words_and_numbers() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n% end\n").unwrap();
        for prefix in [
            "outline 700; error",
            "pipeline 700; error",
            "_line 700; error",
            "unexpected token 'line 700'",
            "unexpected token \"line 700\"",
            "unexpected token 'author's line 700'",
            "invalid line 700bad; error",
            "invalid line 700.5; error",
        ] {
            let message = format!("{prefix} in file 'refs.bib', line 2");
            let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items[0].range.start.line, 1, "{message}: {items:?}");
            assert!(!unresolved_location(&items[0]), "{message}: {items:?}");
        }
        for number in [0, u32::MAX] {
            let message = format!("syntax error in file 'refs.bib', line {number}");
            let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(items[0].data.as_ref().unwrap()["reportedLine"], number);
            assert!(unresolved_location(&items[0]));
        }
    }

    #[test]
    fn bibtex_warning_text_does_not_override_the_entry_location_with_a_line_number() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n").unwrap();
        let diagnostics = parse_bibliography(
            &main,
            "Database file #1: refs.bib\nWarning--line 700 value in entry\n",
        );
        let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(
            items[0].range,
            Range::new(Position::new(1, 6), Position::new(1, 11))
        );
        assert!(!unresolved_location(&items[0]));
    }

    #[test]
    fn biber_entry_fields_preserve_internal_leading_and_trailing_quotes() {
        for key in [
            "O'Connor2026",
            "'start",
            "end'",
            "two''quotes",
            "name'-part",
            "name'_part",
            "name'.part",
            "name':part",
            "name';part",
            "name']part",
            "name'!part",
            "name'?part",
            "王'文2026",
            "😀'key",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join("refs.bib");
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            // Prefix and suffix declarations must not replace the full key.
            std::fs::write(&bib, format!("@book{{O,title={{Prefix}}}}\n@book{{Connor2026,title={{Suffix}}}}\n@book{{{key},title={{Actual}}}}\n")).unwrap();
            for label in ["entry", "Entry"] {
                for quote in ['\'', '"'] {
                    let message = format!("Missing required field for {label} {quote}{key}{quote}");
                    let log =
                        format!("INFO - Found BibTeX data source 'refs.bib'\nWARN - {message}\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                    let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
                    assert_eq!(
                        items[0].range,
                        Range::new(
                            Position::new(2, 6),
                            Position::new(2, 6 + key.encode_utf16().count() as u32)
                        ),
                        "{message}: {items:?}"
                    );
                    assert_eq!(items[0].message, format!("Biber: {message}"));
                }
            }
        }
    }

    #[test]
    fn biber_entry_keys_do_not_select_namesake_database_files() {
        for key in ["entry.bib", "'entry.bib'", "O'Connor.bib", "dir/entry.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join("refs.bib");
            let decoy = root.join(key);
            std::fs::create_dir_all(decoy.parent().unwrap()).unwrap();
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, format!("% header\n@book{{{key},title={{Actual}}}}\n")).unwrap();
            std::fs::write(
                &decoy,
                format!("% decoy\n\n\n@book{{{key},title={{Wrong}}}}\n"),
            )
            .unwrap();
            for label in ["entry", "Entry"] {
                for quote in ['\'', '"'] {
                    for source in ["", " in refs.bib", " in file 'refs.bib'"] {
                        let log = format!("INFO - Found BibTeX data source 'refs.bib'\nWARN - Missing field for {label} {quote}{key}{quote}{source}\n");
                        let diagnostics = parse_bibliography(&main, &log);
                        assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                        let items = diagnostics
                            .get(&Url::from_file_path(&bib).unwrap())
                            .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                        assert_eq!(
                            items[0].range,
                            Range::new(
                                Position::new(1, 6),
                                Position::new(1, 6 + key.encode_utf16().count() as u32)
                            ),
                            "{log}: {items:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn biber_unquoted_sources_take_precedence_over_quoted_error_filenames() {
        for name in [
            "refs.bib",
            "author's.bib",
            "中文.bib",
            "dir.bib/refs.bib",
            "refs.bib,copy.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::create_dir_all(bib.parent().unwrap()).unwrap();
            std::fs::write(&main, "Text\n").unwrap();
            for decoy in ["wrong.bib", "s.bib", "copy.bib", "refs.bib"] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
            for content in [
                "\"in 'wrong.bib', line 900\"",
                "'file \"wrong.bib\"'",
                "\"wrong.bib\"",
                "'wrong.bib'",
            ] {
                for marker in [
                    "in file",
                    "file",
                    "File",
                    "in",
                    "In",
                    "in file ",
                    "in file \u{3000}",
                ] {
                    for message in [
                        format!(
                            "Error in parser: unexpected token {content} {marker} {name}, line 2"
                        ),
                        format!("{marker} {name}, unexpected token {content}, line 2"),
                        format!("Unexpected token {content} {marker} ({name}), line 2"),
                        format!("{marker} [{name}], unexpected token {content}, line 2"),
                        format!("Unexpected token {content} {marker} {{{name}}}, line 2"),
                        format!("Unexpected token {content} {marker} [({name})], line 2"),
                    ] {
                        let log = format!(
                            "INFO - Found BibTeX data source '{name}'\nERROR - {message}\n"
                        );
                        let diagnostics = parse_bibliography(&main, &log);
                        assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                        let items = diagnostics
                            .get(&Url::from_file_path(&bib).unwrap())
                            .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                        assert_eq!(
                            items[0].range.start,
                            Position::new(1, 0),
                            "{log}: {items:?}"
                        );
                        assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                        assert_eq!(items[0].message, format!("Biber: {message}"));
                    }
                }
            }
        }
    }

    #[test]
    fn biber_unquoted_missing_sources_do_not_borrow_quoted_files_or_entry_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        for name in ["wrong.bib", "refs.bib"] {
            std::fs::write(root.join(name), "% header\n\n@book{entry,title={Wrong}}\n").unwrap();
        }
        std::fs::create_dir(root.join("directory.bib")).unwrap();
        for name in [
            "missing.bib",
            "directory.bib",
            "refs.bib.backup",
            "refs.bib2",
            "refs.bib's-backup",
            "refs.bib/nonexistent.bib",
            "(refs.bib",
            "[refs.bib",
            "{refs.bib",
        ] {
            for message in [
                format!("Unexpected token \"in 'wrong.bib'\" in file {name}, line 2"),
                format!("Missing field for entry 'entry' in file {name}"),
            ] {
                let log =
                    format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = diagnostics
                    .get(&Url::from_file_path(&main).unwrap())
                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                assert_eq!(items[0].range.start, Position::new(0, 0));
                assert_eq!(
                    items[0].data.as_ref().unwrap()["unresolvedSource"],
                    true,
                    "{log}: {items:?}"
                );
            }
        }
    }

    #[test]
    fn biber_unquoted_bare_sources_ignore_nested_filenames_in_quoted_errors() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        for name in ["wrong.bib", "s.bib", "refs.bib", "author's.bib"] {
            std::fs::write(root.join(name), "% header\n@book{entry,title={Actual}}\n").unwrap();
        }
        for name in ["refs.bib", "author's.bib"] {
            for tail in [
                format!("{name}: syntax error at line 2"),
                format!("syntax error ({name}) at line 2"),
                format!("syntax error [{name}] at line 2"),
                format!("syntax error on line 2 of {name}."),
                format!("syntax error \u{3000}{name}, line 2"),
                format!("syntax error \u{a0}{name}, line 2"),
            ] {
                for message in [
                    format!("Unexpected token \"in 'wrong.bib', line 900\"; {tail}"),
                    format!("Unexpected token \"bang\" {tail} \"detail\""),
                    format!("Unexpected token \"in 'wrong.bib', line 900\" {tail} \"detail\""),
                ] {
                    let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
                    assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                    let items = diagnostics
                        .get(&Url::from_file_path(root.join(name)).unwrap())
                        .unwrap_or_else(|| panic!("{message}: {diagnostics:?}"));
                    assert_eq!(
                        items[0].range.start,
                        Position::new(1, 0),
                        "{message}: {items:?}"
                    );
                    assert!(!unresolved_location(&items[0]));
                }
            }
        }
    }

    #[test]
    fn biber_standalone_quoted_sources_preserve_complete_filename_boundaries() {
        for name in [
            "refs.bib copy.bib",
            "refs.bib's copy.bib",
            "refs.bib' copy.bib",
            "中文 refs.bib copy's refs.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            for decoy in [
                "refs.bib",
                "copy.bib",
                "s copy.bib",
                "copy's refs.bib",
                "中文 refs.bib",
            ] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            for quote in ['\'', '"'] {
                let message = format!("syntax error {quote}{name}{quote}, line 3");
                let diagnostics = parse_bibliography(&main, &format!("ERROR - {message}\n"));
                assert_eq!(diagnostics.len(), 1, "{message}: {diagnostics:?}");
                let items = diagnostics
                    .get(&Url::from_file_path(&bib).unwrap())
                    .unwrap_or_else(|| panic!("{message}: {diagnostics:?}"));
                assert_eq!(
                    items[0].range.start,
                    Position::new(2, 0),
                    "{message}: {items:?}"
                );
            }
        }
    }

    #[test]
    fn biber_source_fields_ignore_markers_in_quoted_error_content() {
        for name in ["refs.bib", "author's 中文 refs.bib", "entry 'wrong'.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
            std::fs::write(root.join("wrong.bib"), "@book{entry,title={Wrong}}\n").unwrap();
            for (outer, inner) in [('"', '\''), ('\'', '"')] {
                for marker in ["in", "In", "file", "File"] {
                    let content = format!(
                        "Unexpected token {outer}{marker} {inner}wrong.bib{inner}, line 900{outer}"
                    );
                    for quote in ['\'', '"'] {
                        let source = format!("in file {quote}{name}{quote}");
                        for message in [
                            format!("{content} {source}, line 2"),
                            format!("{source}, {content}, line 2"),
                        ] {
                            let log = format!(
                                "INFO - Found BibTeX data source '{name}'\nERROR - {message}\n"
                            );
                            let diagnostics = parse_bibliography(&main, &log);
                            assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                            let items = diagnostics
                                .get(&Url::from_file_path(&bib).unwrap())
                                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                            assert_eq!(
                                items[0].range.start,
                                Position::new(1, 0),
                                "{log}: {items:?}"
                            );
                            assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                            assert_eq!(items[0].message, format!("Biber: {message}"));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn biber_missing_source_fields_do_not_use_files_named_in_quoted_error_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(root.join("refs.bib"), "@book{entry,title={Other}}\n").unwrap();
        std::fs::write(root.join("wrong.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        for (outer, inner) in [('"', '\''), ('\'', '"')] {
            for marker in ["in", "In", "file", "File"] {
                let message = format!("Unexpected token {outer}{marker} {inner}wrong.bib{inner}, line 900{outer} in file 'missing.bib', line 2");
                let log =
                    format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items[0].range.start, Position::new(0, 0));
                assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
                assert_eq!(items[0].message, format!("Biber: {message}"));
            }
        }
    }

    #[test]
    fn biber_incomplete_entry_keys_do_not_select_namesake_database_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(root.join("entry.bib"), "@book{entry.bib,title={Wrong}}\n").unwrap();
        for quote in ['\'', '"'] {
            let log = format!("WARN - Missing field for entry {quote}entry.bib\n");
            let diagnostics = parse_bibliography(&main, &log);
            assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
            let items = diagnostics
                .get(&Url::from_file_path(&main).unwrap())
                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
            assert_eq!(items[0].range.start, Position::new(0, 0));
        }
    }

    #[test]
    fn biber_entry_fields_skip_names_inside_files_and_find_metadata_after_them() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let name = "entry 'wrong'.bib";
        let bib = root.join(name);
        std::fs::write(&main, "\\cite{O'Connor2026}\n").unwrap();
        std::fs::write(
            &bib,
            "% header\n@book{wrong,title={Wrong}}\n@book{O'Connor2026,title={Actual}}\n",
        )
        .unwrap();
        let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - syntax error in file \"{name}\"\nWARN - Missing field in file \"{name}\" for entry 'O'Connor2026'\n");
        let diagnostics = parse_bibliography(&main, &log);
        let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(items.len(), 2);
        let error = items
            .iter()
            .find(|item| item.severity == Some(DiagnosticSeverity::ERROR))
            .unwrap();
        assert_eq!(error.range.start, Position::new(0, 0));
        let warning = items
            .iter()
            .find(|item| item.severity == Some(DiagnosticSeverity::WARNING))
            .unwrap();
        assert_eq!(
            warning.range,
            Range::new(Position::new(2, 6), Position::new(2, 18))
        );
    }

    #[test]
    fn biber_unterminated_entry_fields_do_not_borrow_trailing_keys() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "\\cite{other}\n").unwrap();
        std::fs::write(
            root.join("refs.bib"),
            "% header\n@book{other,title={Wrong}}\n",
        )
        .unwrap();
        let log = "INFO - Found BibTeX data source 'refs.bib'\nWARN - Missing field for entry 'missing in other\n";
        let diagnostics = parse_bibliography(&main, log);
        assert_eq!(diagnostics.len(), 1);
        let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(items[0].range.start, Position::new(0, 0));
    }

    #[test]
    fn biber_entry_fields_ignore_quoted_content_and_partial_words() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(
            &bib,
            "% header\n@book{wrong,title={Wrong}}\n@book{O'Connor2026,title={Actual}}\n",
        )
        .unwrap();
        for prefix in [
            "unexpected token \"entry 'wrong'\"",
            "unexpected token 'entry \"wrong\"'",
            "subentry 'wrong'",
            "_entry 'wrong'",
            "王entry 'wrong'",
        ] {
            for suffix in ["", " for Entry 'O'Connor2026': missing author"] {
                let log = format!(
                    "INFO - Found BibTeX data source 'refs.bib'\nWARN - {prefix}{suffix}\n"
                );
                let diagnostics = parse_bibliography(&main, &log);
                let expected = if suffix.is_empty() { &main } else { &bib };
                let items = &diagnostics[&Url::from_file_path(expected).unwrap()];
                assert_eq!(items.len(), 1, "{log}: {items:?}");
                if suffix.is_empty() {
                    assert_eq!(items[0].range.start, Position::new(0, 0));
                } else {
                    assert_eq!(
                        items[0].range,
                        Range::new(Position::new(2, 6), Position::new(2, 18))
                    );
                }
            }
        }
    }

    #[test]
    fn biber_entry_quote_boundaries_do_not_expose_quoted_line_fields() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
        for quoted in [
            "'name'-line 700'",
            "'name'.line 700'",
            "'name':line 700'",
            "'name''line 700'",
            "'name';line 700'",
            "'name']line 700'",
        ] {
            let log =
                format!("WARN - Unexpected token {quoted} for entry 'entry' in file 'refs.bib'\n");
            let diagnostics = parse_bibliography(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&bib).unwrap()];
            assert_eq!(
                items[0].range,
                Range::new(Position::new(1, 6), Position::new(1, 11)),
                "{log}: {items:?}"
            );
            assert!(!unresolved_location(&items[0]));
        }
    }

    #[test]
    fn biber_databases_with_apostrophes_keep_entry_warning_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let bib = root.join("author's refs.bib");
        std::fs::write(&bib, "% database\n@book{example,title={Example}}\n").unwrap();
        let diagnostics = parse_bibliography(&root.join("main.tex"), "INFO - Found BibTeX data source 'author's refs.bib'\nWARN - Missing field for entry 'example'\nERROR - Error in 'author's refs.bib' line 2\n");
        let diagnostics = &diagnostics[&Url::from_file_path(bib).unwrap()];
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.range.start.line == 1));
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING))
            .unwrap();
        assert_eq!(diagnostic.range.start.line, 1);
        assert_eq!(diagnostic.range.start.character, 6);
    }

    #[test]
    fn missing_include_messages_locate_paths_without_reporting_normal_missing_auxiliary_files() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let child = directory.path().join("child.tex");
        std::fs::write(&main, "\\input{child}\n").unwrap();
        let source = "😀 \\include{missing chapter}\n% \\include{missing chapter}\n\\verb|\\include{missing chapter}|\n\\include{other}\n\\include{ missing chapter }\n";
        std::fs::write(&child, source).unwrap();
        let log = "(./main.tex\n(./child.tex\nNo file main.aux.\nNo file main.bbl.\nNo file optional.tex.\nNo file \"missing chapter.tex\".\nPackage local Warning: Unfinished message (\nNo file other.tex.\nNo file missing chapter.tex.\n)\nNo file optional.tex.\n)";
        let result = parse(&main, log);
        assert!(!result.contains_key(&Url::from_file_path(&main).unwrap()));
        let diagnostics = &result[&Url::from_file_path(&child).unwrap()];
        let missing: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("missing-include".into()))
            })
            .collect();
        assert_eq!(missing.len(), 3, "{diagnostics:?}");
        assert_eq!(missing[0].range.start.character, 12);
        let text = Text::new(source.into());
        assert_eq!(
            missing
                .iter()
                .map(
                    |diagnostic| &source[text.offset(diagnostic.range.start).unwrap()
                        ..text.offset(diagnostic.range.end).unwrap()]
                )
                .collect::<Vec<_>>(),
            vec!["missing chapter", "other", "missing chapter"]
        );
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "Package local Warning: Unfinished message ("));
        assert!(missing
            .iter()
            .all(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING)));
    }

    #[test]
    fn wrapped_missing_includes_preserve_file_context_and_adjacent_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let child = root.join("child.tex");
        std::fs::write(&main, "\\input{child}\n\\include{parent}\n").unwrap();
        let chapter = format!("chapters/{} (中文 file)", "long-path-".repeat(12));
        let source = format!("😀 \\include{{{chapter}}}\n");
        std::fs::write(&child, &source).unwrap();
        for width in [16, 79, 121] {
            for quoted in [false, true] {
                let missing = if quoted {
                    format!("No file \"{chapter}.tex\".")
                } else {
                    format!("No file {chapter}.tex.")
                };
                let wrapped = missing
                    .chars()
                    .collect::<Vec<_>>()
                    .chunks(width)
                    .map(|chars| chars.iter().collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                let log = format!("(./main.tex\n(./child.tex\nNo file optional(broken).tex.\n{wrapped}\nNo file chapters/long-path-\nPackage local Warning: Adjacent warning on input line 1.\n)\nNo file parent.tex.\n)");
                let result = parse(&main, &log);
                let child_diagnostics = &result[&Url::from_file_path(&child).unwrap()];
                assert_eq!(child_diagnostics.len(), 2, "{result:?}");
                let missing = child_diagnostics
                    .iter()
                    .find(|diagnostic| missing_include_key(&diagnostic.message).is_some())
                    .unwrap();
                assert_eq!(
                    missing.message,
                    format!("Included file `{chapter}.tex` not found")
                );
                assert_eq!(
                    missing.range,
                    Range::new(
                        Position::new(0, 12),
                        Position::new(0, 12 + chapter.encode_utf16().count() as u32)
                    )
                );
                let parent = &result[&Url::from_file_path(&main).unwrap()];
                assert_eq!(parent.len(), 1);
                assert_eq!(parent[0].message, "Included file `parent.tex` not found");
                assert_eq!(parent[0].range.start.line, 1);
            }
        }
    }

    #[test]
    fn box_diagnostics_span_reported_source_lines_and_keep_valid_utf16_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "First\r\n  😀 second\r\n 中文 third\r\nLast\r\n").unwrap();
        let log = "(./main.tex\nOverfull \\hbox (12pt too wide) in paragraph at lines 2--3\nUnderfull \\hbox (badness 10000) in paragraph at lines 3--200\nOverfull \\vbox (2pt too high) detected at line 4\nUnderfull \\hbox (badness 10000) in paragraph at lines 3--1\n)";
        let result = parse(&main, log);
        let diagnostics = &result[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 4);
        let find = |prefix: &str| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic.message.starts_with(prefix))
                .unwrap()
        };
        let overfull = find("Overfull \\hbox");
        assert_eq!(
            overfull.range,
            Range::new(Position::new(1, 2), Position::new(2, 9))
        );
        assert_eq!(
            overfull.code,
            Some(NumberOrString::String("overfull-box".into()))
        );
        let underfull = find("Underfull \\hbox (badness 10000) in paragraph at lines 3--200");
        assert_eq!(
            underfull.range,
            Range::new(Position::new(2, 1), Position::new(4, 0))
        );
        assert_eq!(
            underfull.code,
            Some(NumberOrString::String("underfull-box".into()))
        );
        assert_eq!(
            find("Overfull \\vbox").range,
            Range::new(Position::new(3, 0), Position::new(3, 4))
        );
        assert_eq!(
            find("Underfull \\hbox (badness 10000) in paragraph at lines 3--1").range,
            Range::new(Position::new(2, 1), Position::new(2, 9))
        );
    }

    #[test]
    fn single_quoted_citation_warnings_highlight_keys_and_preserve_apostrophes() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\autocite{first, O'Connor}\n\\ref{missing}\n";
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nPackage biblatex Warning: Citation 'O'Connor' on page 1 undefined on input line 1.\nPackage biblatex Warning: Citation 'first' undefined\n(biblatex) on input line 1.\nLaTeX Warning: Reference 'missing' on page 1 undefined on input line 2.\n)";
        let result = parse(&main, log);
        let diagnostics = &result[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 3);
        let text = Text::new(source.into());
        let values: Vec<_> = diagnostics
            .iter()
            .map(|diagnostic| {
                &source[text.offset(diagnostic.range.start).unwrap()
                    ..text.offset(diagnostic.range.end).unwrap()]
            })
            .collect();
        assert_eq!(values, vec!["first", "O'Connor", "missing"]);
        assert_eq!(diagnostics[0].range.start.character, 13);
    }

    #[test]
    fn bibliography_errors_and_entry_warnings_point_to_the_database() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let bib = directory.path().join("with spaces.bib");
        std::fs::write(&main, "\\cite{example}\n").unwrap();
        std::fs::write(
            &bib,
            "\n@book{example,\n title={Example}\n}\n@book{example, title={Duplicate}}\n",
        )
        .unwrap();
        let log = "Database file #1: with spaces.bib\nRepeated entry---line 5 of file with spaces.bib\nI was expecting a `,' or a `}'---line 3 of file with spaces.bib\nWarning--empty author in example\n";
        let diagnostics = parse_bibliography(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].range.start, Position::new(1, 6));
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(diagnostics[1].range.start.line, 2);
        assert_eq!(diagnostics[2].range.start.line, 4);
        let log = "[0] INFO - Found BibTeX data source 'with spaces.bib'\n[1] ERROR - syntax error in file 'with spaces.bib', line 3\n[2] WARN - Missing required field for entry 'example'\n";
        let diagnostics = parse_bibliography(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].range.start.line, 1);
        assert_eq!(diagnostics[1].range.start.line, 2);
        assert!(diagnostics[1].message.starts_with("Biber:"));
        let diagnostics = parse_bibliography(&main,"I couldn't open database file missing.bib\n---line 4 of file main.aux\nI found no database files---while reading file main.aux\n");
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("couldn't open database file missing.bib")));
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.range.start.line == 0));
    }

    #[test]
    fn file_line_errors_and_multiline_warnings_use_the_right_file() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let chapter = directory.path().join("章节.tex");
        std::fs::write(&main, "\\input{章节}\n\\ref{missing}\n").unwrap();
        std::fs::write(&chapter, "hello\n😀 \\bad\n").unwrap();
        let log = "(./main.tex\n(./章节.tex\n./章节.tex:2: Undefined control sequence.\nl.2 \\bad\n)\nLaTeX Warning: Reference `missing' on page 1 undefined on input\n line 2.\n\nOverfull \\hbox (12pt too wide) in paragraph at lines 2--2\n)";
        let diagnostics = parse(&main, log);
        let chapter = &diagnostics[&Url::from_file_path(chapter).unwrap()];
        assert_eq!(chapter.len(), 1);
        assert_eq!(chapter[0].range.start.line, 1);
        assert_eq!(chapter[0].range.end.character, 7);
        let main = &diagnostics[&Url::from_file_path(main).unwrap()];
        assert_eq!(main.len(), 2);
        assert!(main
            .iter()
            .all(|diagnostic| diagnostic.range.start.line == 1));
    }

    #[test]
    fn font_class_engine_and_glyph_warnings_preserve_adjacent_errors_and_deduplicate_shipout_messages(
    ) {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "First\nSecond\nThird\n").unwrap();
        let log = "(./main.tex\nClass memoir Warning: Option ignored\nLaTeX Font Warning: Font shape unavailable\n(Font) using a substitute on input line 3.\nMissing character: There is no 😀 (U+1F600) in font\n Latin Modern Roman!\nMissing character: There is no 😀 (U+1F600) in font\n Latin Modern Roman!\npdfTeX warning (dest): destination missing!\n! First error.\n! Second error.\nl.2 bad\n)\n";
        let diagnostics = parse(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 6, "{diagnostics:?}");
        let font = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.starts_with("LaTeX Font Warning:"))
            .unwrap();
        assert_eq!(font.range.start.line, 2);
        assert!(font.message.contains("(Font) using a substitute"));
        let glyph = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.message.starts_with("Missing character:"))
            .collect::<Vec<_>>();
        assert_eq!(glyph.len(), 1);
        assert!(glyph[0].message.ends_with("Latin Modern Roman!"));
        assert_eq!(glyph[0].severity, Some(DiagnosticSeverity::WARNING));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "Class memoir Warning: Option ignored"));
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
                .count(),
            2
        );
    }

    #[test]
    fn traditional_errors_and_quoted_paths_are_supported() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let chapter = directory.path().join("with spaces.tex");
        std::fs::write(&main, "main\n").unwrap();
        std::fs::write(&chapter, "hello\n\\bad\n").unwrap();
        let diagnostics = parse(
            &main,
            "(./main.tex\n(\"./with spaces.tex\"\n! Undefined control sequence.\nl.2 \\bad\n)\n)",
        );
        assert_eq!(
            diagnostics[&Url::from_file_path(chapter).unwrap()][0]
                .range
                .start
                .line,
            1
        );
    }

    #[test]
    fn source_paths_and_diagnostic_parentheses_preserve_nested_file_context() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let definition = directory.path().join("配置 (旧).def");
        let options = directory.path().join("layout options.clo");
        for path in [&main, &definition, &options] {
            std::fs::write(path, "One\n😀 Two\nThree\nFour\nFive\n").unwrap();
        }
        let log = "(./main.tex\n(./配置 (旧).def\nPackage local Warning: Closing ) marker on input line 2.\nPackage local Warning: Still local on input line 3.\n! Error with a ( marker.\nl.4 \\bad)\nPackage local Warning: After error on input line 5.\n)\n(./layout options.clo\nLaTeX Warning: Options warning on input line 2.\n)\nLaTeX Warning: Back in main on input line 3.\n)";
        let diagnostics = parse(&main, log);
        let local = &diagnostics[&Url::from_file_path(&definition).unwrap()];
        assert_eq!(local.len(), 4, "{diagnostics:?}");
        assert_eq!(local[0].range.end.character, 6);
        assert!(local
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == 3
                && diagnostic.severity == Some(DiagnosticSeverity::ERROR)));
        assert_eq!(
            diagnostics[&Url::from_file_path(&options).unwrap()][0]
                .range
                .start
                .line,
            1
        );
        assert_eq!(
            diagnostics[&Url::from_file_path(&main).unwrap()][0]
                .range
                .start
                .line,
            2
        );
    }

    #[test]
    fn undefined_references_highlight_keys_in_lists_multiline_calls_and_utf16_text() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\ref{missing} \\cref{found, missing} \\cite{lost}\n% \\ref{missing}\n\\verb|\\ref{missing}|\n\\ref\n{missing}\n\\ref{O'Connor}\n";
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\nPackage natbib Warning: Citation `lost' on page 1 undefined on input line 1.\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 2.\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 3.\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 5.\nLaTeX Warning: Reference `O'Connor' undefined on input line 6.\n)";
        let diagnostics = parse(&main, log);
        let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(diagnostics.len(), 7, "{diagnostics:?}");
        let text = Text::new(source.into());
        let value = |range: Range| {
            &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()]
        };
        let first: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.range.start.line == 0)
            .collect();
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].range.start.character, 8);
        assert_eq!(
            first
                .iter()
                .map(|diagnostic| value(diagnostic.range))
                .collect::<Vec<_>>(),
            vec!["missing", "missing", "lost"]
        );
        assert_eq!(value(diagnostics[3].range), "% \\ref{missing}");
        assert_eq!(value(diagnostics[4].range), "\\verb|\\ref{missing}|");
        assert_eq!(value(diagnostics[5].range), "missing");
        assert_eq!(value(diagnostics[6].range), "O'Connor");
    }

    #[test]
    fn single_reference_ranges_do_not_normalize_whitespace_but_cref_lists_keep_key_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "😀 \\ref{missing} \\ref{ missing } \\cref{found, missing }\n";
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\nLaTeX Warning: Reference ` missing ' on page 1 undefined on input line 1.\n)\n";
        let diagnostics = parse(&main, log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 3, "{errors:?}");
        let text = Text::new(source.into());
        let precise: Vec<_> = errors
            .iter()
            .filter(|error| !error.message.contains("` missing '"))
            .collect();
        assert_eq!(precise.len(), 2);
        for (error, start) in precise.iter().zip([
            source.find("missing").unwrap(),
            source.rfind("missing").unwrap(),
        ]) {
            assert_eq!(error.range, text.range(start, start + 7));
        }
        let fallback = errors
            .iter()
            .find(|error| error.message.contains("` missing '"))
            .unwrap();
        assert_eq!(fallback.range, text.range(0, source.len() - 1));
    }

    #[test]
    fn silent_refcount_queries_do_not_inherit_warnings_from_another_call_on_the_line() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = concat!(
            "\\refused{missing} \\getrefnumber{missing} \\getpagerefnumber{missing} ",
            "\\getrefbykeydefault{missing}{page}{0} ",
            "\\IfRefUndefinedExpandable{missing}{A}{B} \\IfRefUndefinedBabel{missing}{A}{B}\n",
        );
        std::fs::write(&main, source).unwrap();
        let log = "(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\n)\n";
        let diagnostics = parse(&main, log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 1, "{errors:?}");
        let start = source.find("missing").unwrap();
        assert_eq!(
            errors[0].range,
            Text::new(source.into()).range(start, start + 7)
        );
    }

    #[test]
    fn wrapped_file_announcements_rejoin_existing_names_without_swallowing_warnings() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let name = format!("settings {}(old).def", "long-name-".repeat(12));
        let definition = directory.path().join(&name);
        std::fs::write(&main, "Main\n").unwrap();
        std::fs::write(&definition, "First\nSecond\n").unwrap();
        for quoted in [false, true] {
            let opening = if quoted {
                format!("(\"./{name}\"")
            } else {
                format!("(./{name}")
            };
            let wrapped = format!("{}\n{}", &opening[..79], &opening[79..]);
            let log = format!("(./main.tex\n{wrapped}\nLaTeX Warning: Local warning on input line 2.\n)\nLaTeX Warning: Root warning on input line 1.\n)\n");
            let diagnostics = parse(&main, &log);
            assert_eq!(
                diagnostics[&Url::from_file_path(&definition).unwrap()][0]
                    .range
                    .start
                    .line,
                1
            );
            assert_eq!(diagnostics[&Url::from_file_path(&main).unwrap()].len(), 1);
        }
        let log = format!(
            "(./main.tex\n({}\nLaTeX Warning: Do not swallow this on input line 1.\n",
            "not-a-file-".repeat(9)
        );
        let diagnostics = parse(&main, &log);
        assert_eq!(diagnostics[&Url::from_file_path(&main).unwrap()].len(), 1);
    }
}
