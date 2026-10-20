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

fn missing_reported_file(
    path: &Path,
    lines: &[&str],
    index: &mut usize,
    command_name: &str,
) -> Option<String> {
    let line = lines[*index];
    let text = Text::new(std::fs::read_to_string(path).ok()?);
    let syntax = crate::syntax::Syntax::latex(&text);
    if command_name == "verbatiminput"
        && !crate::code_inputs::safe_syntax(&text.source, &syntax, command_name)
    {
        return None;
    }
    let mut candidates = Vec::new();
    let mut matched = None;
    for command in syntax
        .commands
        .iter()
        .filter(|command| command.name == command_name)
        .take(1024)
    {
        let Some(argument) = crate::syntax::file_argument(&text.source, command) else {
            continue;
        };
        let value = if command_name == "include" {
            argument.value.trim()
        } else {
            argument.value.as_str()
        };
        if value.is_empty() || value.contains(['\\', '{', '}', '%', '#', '$', '\r', '\n']) {
            continue;
        }
        let names = if command_name == "include" {
            crate::syntax::file_lookup_names(command_name, value).collect::<Vec<_>>()
        } else {
            vec![Cow::Borrowed(value)]
        };
        for name in names {
            let mut messages = vec![format!("No file {name}.")];
            if command_name == "include" {
                messages.push(format!("No file \"{name}\"."));
            }
            for message in messages {
                if message.len() <= 8192 && message.starts_with(line) {
                    if message == line {
                        matched = Some((name.to_string(), *index));
                    } else {
                        candidates.push((message, name.to_string()));
                    }
                }
            }
        }
    }
    // Join only prefixes of a known literal call. This avoids swallowing
    // adjacent diagnostics or interpreting ordinary optional-file messages.
    if candidates.is_empty() {
        return matched.map(|(name, _)| name);
    }
    let mut joined = line.to_owned();
    for (next_index, next) in lines.iter().enumerate().skip(*index + 1).take(64) {
        if joined.len() + next.len() > 8192 {
            break;
        }
        joined.push_str(next);
        candidates.retain(|(message, _)| message.starts_with(&joined));
        if let Some((_, name)) = candidates.iter().find(|(message, _)| message == &joined) {
            matched = Some((name.clone(), next_index));
            candidates.retain(|(message, _)| message != &joined);
        }
        if candidates.is_empty() {
            break;
        }
    }
    matched.map(|(name, last)| {
        *index = last;
        name
    })
}

pub(crate) fn missing_verbatim_name(message: &str) -> Option<&str> {
    message
        .strip_prefix("Verbatim input file `")?
        .strip_suffix("` not found")
}

fn emit_missing_verbatim(result: &mut BTreeMap<Url, Vec<Diagnostic>>, path: &Path, name: &str) {
    let (Ok(uri), Ok(source)) = (Url::from_file_path(path), std::fs::read_to_string(path)) else {
        return;
    };
    let text = Text::new(source);
    let parsed = crate::syntax::Syntax::latex(&text);
    if !crate::code_inputs::safe_syntax(&text.source, &parsed, "verbatiminput") {
        return;
    }
    let message = format!("Verbatim input file `{name}` not found");
    for command in parsed
        .commands
        .iter()
        .filter(|command| command.name == "verbatiminput")
        .take(1024)
    {
        let Some(argument) = crate::syntax::file_argument(&text.source, command)
            .filter(|argument| argument.value == name)
        else {
            continue;
        };
        let range = text.range(argument.span.start, argument.span.end);
        let items = result.entry(uri.clone()).or_default();
        if !items
            .iter()
            .any(|item| item.range == range && item.message == message)
        {
            items.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("latex-build".into()),
                code: Some(NumberOrString::String("missing-verbatim-input".into())),
                message: message.clone(),
                data: Some(serde_json::json!({"file":name})),
                ..Default::default()
            });
        }
    }
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
                    "Ref" | "autopageref" | "vref" | "Vref" | "subref"
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
        // Range references read both endpoints before reporting missing labels.
        let report_end = if command.name == "hyperref" {
            crate::syntax::group(
                &text.source,
                crate::syntax::skip_space(&text.source, argument.outer_span.end),
                b'{',
                b'}',
            )
            .map_or(argument.outer_span.end.saturating_sub(1), |(text, _)| {
                text.span.end
            })
        } else {
            range_end
                .as_ref()
                .map_or(argument.outer_span.end.saturating_sub(1), |end| {
                    end.outer_span.end.saturating_sub(1)
                })
        };
        if text.position(command.span.start).line > line || text.position(report_end).line < line {
            continue;
        }
        if citation && crate::syntax::is_single_citation(&command.name)
            || !citation
                && (crate::syntax::is_single_reference(&command.name)
                    || crate::syntax::has_separate_label_argument(&command.name))
        {
            let source_key = if command.name == "subref" && !hyper_link {
                key.strip_prefix("sub@")
            } else {
                Some(key)
            };
            if let Some(key) = source_key.filter(|key| crate::syntax::is_static_key(key)) {
                for argument in std::iter::once(argument).chain(range_end.as_ref()) {
                    if argument.value == key {
                        ranges.push(text.range(argument.span.start, argument.span.end));
                    }
                }
            }
            continue;
        }
        for span in crate::syntax::key_list_spans(&text.source, argument.span.clone()) {
            if &text.source[span.clone()] == key {
                ranges.push(text.range(span.start, span.end));
            }
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
    name_frame: Option<&'a str>,
}

impl<'a> ErrorContext<'a> {
    fn source(source: &'a str) -> Self {
        Self {
            source,
            package_options: None,
            name_frame: None,
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
    let name_frame = (message == "Undefined control sequence." && recent.is_none())
        .then(|| line.trim_start().strip_prefix("<argument> "))
        .flatten()
        .map(str::trim_end);
    if let Some(frame) = name_frame {
        let token = frame
            .strip_prefix("FV@SVG@")
            .or_else(|| frame.strip_prefix("FV@SV@"))
            .or_else(|| frame.strip_prefix("r@"))
            .or_else(|| frame.strip_prefix("sub@"))
            .unwrap_or(frame);
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
    // FancyVerb always expands this internal error macro, even for a direct
    // call. Accept only the frames printed by pdfTeX and XeTeX; an additional
    // user expansion frame must still prevent attribution to a literal path.
    if missing_fancyverb_name(message).is_some() && line.starts_with("\\FV@Error ") {
        match line.trim_end() {
            "\\FV@Error ... {FancyVerb Error:" => {
                if context.next()?.1 != "\\space \\space #1" || context.next()?.1 != "}" {
                    return None;
                }
            }
            "\\FV@Error ...ncyVerb Error:^^J\\space \\space #1^^J}" => {}
            _ => return None,
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
            name_frame,
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

pub(crate) fn missing_listing_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("Package Listings Error: File `")?
        .strip_suffix("' not found.")?;
    (!name.is_empty() && name.len() <= 8192).then_some(name)
}

pub(crate) fn missing_fancyverb_name(message: &str) -> Option<&str> {
    let name = message
        .strip_prefix("FancyVerb Error:")?
        .trim_start()
        .strip_prefix("No verbatim file ")?
        .strip_suffix('.')?;
    let name = name
        .strip_prefix('"')
        .and_then(|name| name.strip_suffix('"'))
        .unwrap_or(name);
    crate::syntax::is_literal_file_path(name).then_some(name)
}

fn missing_read_file(message: &str) -> bool {
    missing_file_name(message).is_some() || missing_listing_name(message).is_some()
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
    if shortened.is_some_and(|suffix| !unique_context_on_line(text, start, suffix)) {
        return None;
    }
    let syntax = crate::syntax::Syntax::latex(text);
    let fancyverb = crate::fancyvrb::is_input(command_name);
    if crate::syntax::code_input_command(command_name)
        && !crate::code_inputs::safe_syntax(&text.source, &syntax, command_name)
    {
        return None;
    }
    let mut matches = syntax.commands.iter().filter_map(|command| {
        if !(if fancyverb {
            crate::fancyvrb::is_input(&command.name)
        } else {
            command.name == command_name
        }) {
            return None;
        }
        let argument = crate::syntax::file_argument(&text.source, command)?;
        if crate::syntax::code_input_command(command_name)
            && !crate::code_inputs::safe_call(&text.source, command)
        {
            return None;
        }
        let span = crate::syntax::trimmed_span(&text.source, argument.span.clone());
        let value = &text.source[span.clone()];
        let matches_report = if command_name == "lstinputlisting" {
            crate::listings::missing_report(value) == name
        } else if fancyverb {
            value == name
        } else {
            crate::syntax::file_lookup_names(command_name, value).any(|candidate| candidate == name)
        };
        if !crate::syntax::is_literal_file_path(value)
            || !matches_report
            || text.position(argument.outer_span.end).line != line
            || text.source[argument.outer_span.clone()].starts_with('"')
                && !text.source[argument.outer_span.clone()].ends_with('"')
        {
            return None;
        }
        let consumed = &text.source[start..argument.outer_span.end];
        let matched = shortened.map_or(consumed == prefix, |suffix| {
            (fancyverb && !suffix.is_empty()
                || suffix.ends_with(&text.source[argument.outer_span.clone()]))
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
        let Some(argument) = crate::syntax::file_argument(&text.source, command) else {
            continue;
        };
        let next = crate::syntax::skip_space(&text.source, argument.outer_span.end);
        // LaTeX reads the optional minimum version, or one lookahead token,
        // before the loader reports a failure. That token may be on
        // the next source line, while the path being diagnosed is above it.
        let tail = if text.source.as_bytes().get(next) == Some(&b'[') {
            crate::syntax::optional_note_end(&text.source, next)
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
        // A dynamic sibling can generate the reported filename. Require the
        // complete static list before attributing an error to one item.
        let spans = if crate::syntax::file_list(command_name) {
            let Some(spans) = crate::syntax::complete_file_list_spans(&text.source, command) else {
                continue;
            };
            spans
        } else {
            let span = crate::syntax::trimmed_span(&text.source, argument.span.clone());
            if !crate::syntax::is_literal_file_path(&text.source[span.clone()]) {
                continue;
            }
            vec![span]
        };
        for span in spans {
            if crate::syntax::file_lookup_names(command_name, &text.source[span.clone()])
                .any(|candidate| candidate == name)
            {
                matches.push((text.range(span.start, span.end), command_name));
            }
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
    if shortened.is_some_and(|suffix| !unique_context_on_line(text, start, suffix)) {
        return None;
    }
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

fn unique_context_on_line(text: &Text, start: usize, context: &str) -> bool {
    // Unknown callers and opaque text can duplicate a supported invocation's
    // suffix. Check the whole source line before filtering candidate calls.
    let Some(line) = text
        .source
        .get(start..)
        .and_then(|tail| tail.lines().next())
    else {
        return false;
    };
    line.find(context)
        .is_some_and(|first| line.rfind(context) == Some(first))
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

fn reference_macro_error_invocation(
    source: &str,
    command: &crate::syntax::Command,
    frame: &str,
) -> Option<(ByteRange<usize>, ByteRange<usize>)> {
    let frame = match command.name.as_str() {
        "vpageref"
        | "getrefnumber"
        | "getpagerefnumber"
        | "refused"
        | "IfRefUndefinedExpandable"
        | "setcounterref"
        | "addtocounterref"
        | "setcounterpageref"
        | "addtocounterpageref" => frame.strip_prefix("r@")?,
        "subref" => frame.strip_prefix("sub@")?,
        _ => frame,
    };
    let (start, open) = match command.name.as_str() {
        "ref"
        | "pageref"
        | "eqref"
        | "Ref"
        | "autoref"
        | "autopageref"
        | "nameref"
        | "vref"
        | "Vref"
        | "vpageref"
        | "getrefnumber"
        | "getpagerefnumber"
        | "refused"
        | "subref"
        | "getrefbykeydefault"
        | "IfRefUndefinedExpandable"
        | "IfRefUndefinedBabel" => (
            crate::syntax::reference_argument_start(source, &command.name, command.span.end)?,
            b'{',
        ),
        "cite" | "nocite" | "citet" | "citep" | "citeauthor" | "citeyear" | "citenum"
        | "bibentry" | "citeyearpar" | "citealt" | "citealp" | "citefullauthor" | "Citet"
        | "Citep" | "Citealt" | "Citealp" | "Citeauthor" | "citetalias" | "citepalias" => (
            crate::syntax::citation_argument_start(source, &command.name, command.span.end)?,
            b'{',
        ),
        "hyperref" => {
            crate::syntax::separate_label_argument_start(source, command)?;
            (crate::syntax::skip_space(source, command.span.end), b'[')
        }
        "setcounterref" | "addtocounterref" | "setcounterpageref" | "addtocounterpageref" => {
            let (counter, end) = crate::syntax::group(
                source,
                crate::syntax::skip_space(source, command.span.end),
                b'{',
                b'}',
            )?;
            if !crate::syntax::is_static_key(&counter.value) {
                return None;
            }
            (crate::syntax::skip_space(source, end), b'{')
        }
        _ => return None,
    };
    let close = if open == b'[' { b']' } else { b'}' };
    let (mut argument, _) = crate::syntax::group(source, start, open, close)?;
    let mut tail = argument.outer_span.clone();
    if command.name == "getrefbykeydefault" {
        // refcount consumes the selector before expanding the key, but has
        // not read the default yet. Dynamic selectors cannot identify a peer.
        let (selector, end) = crate::syntax::group(
            source,
            crate::syntax::skip_space(source, tail.end),
            b'{',
            b'}',
        )?;
        if !selector.value.is_empty() && !crate::syntax::is_static_key(&selector.value) {
            return None;
        }
        tail.end = end;
    }
    if command.name == "hyperref" && argument.value.starts_with('{') {
        let (inner, end) = crate::syntax::group(source, argument.span.start, b'{', b'}')?;
        if end != argument.span.end {
            return None;
        }
        argument = inner;
    }
    if argument.value != frame
        || crate::syntax::control_word(&argument.value, 0)? != (0..argument.value.len())
    {
        return None;
    }
    Some((argument.span, tail))
}

fn undefined_name_command_range(
    text: &Text,
    line: u32,
    context: &str,
    frame: &str,
) -> Option<Range> {
    let prefix = context.trim_end();
    let shortened = shortened_context(prefix);
    let start = text.offset(Position::new(line, 0))?;
    if shortened.is_some_and(|suffix| !unique_context_on_line(text, start, suffix)) {
        return None;
    }
    let syntax = crate::syntax::Syntax::latex(text);
    let mut matches = syntax.commands.iter().filter_map(|command| {
        let (span, tail) = crate::saved_text::name_error_invocation(&text.source, command, frame)
            .or_else(|| crate::named_commands::name_error_invocation(&text.source, command, frame))
            .or_else(|| reference_macro_error_invocation(&text.source, command, frame))?;
        if text.position(tail.end).line != line
            || !(syntax
                .name_command_references
                .iter()
                .any(|entry| entry.span == span)
                || syntax.commands.iter().any(|command| command.span == span))
        {
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

fn emit_with_context(
    result: &mut BTreeMap<Url, Vec<Diagnostic>>,
    path: &Path,
    line: u32,
    mut message: String,
    severity: DiagnosticSeverity,
    error: Option<ErrorContext<'_>>,
    source_confirmed: bool,
) {
    let context = error
        .as_ref()
        .filter(|context| context.name_frame.is_none())
        .map(|context| context.source);
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
    .then(|| {
        let error = error.as_ref()?;
        if let Some(frame) = error.name_frame {
            undefined_name_command_range(&text, line, error.source, frame)
        } else {
            undefined_command_range(&text, line, error.source)
        }
    })
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
    let missing_listing = missing_listing_name(&message);
    let listing_range = missing_listing
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| missing_path_range(&text, line, context?, name, "lstinputlisting"));
    ranges.extend(listing_range);
    let fancyverb_range = missing_fancyverb_name(&message)
        .filter(|_| reported_line.checked_sub(1) == Some(line))
        .and_then(|name| missing_path_range(&text, line, context?, name, "VerbatimInput"));
    ranges.extend(fancyverb_range);
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
                    .or_else(|| listing_range.map(|_| "missing-listing"))
                    .or_else(|| fancyverb_range.map(|_| "missing-fancyverb-input"))
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
                    .or_else(|| listing_range.map(|range| {
                        let start = text.offset(range.start).unwrap();
                        let end = text.offset(range.end).unwrap();
                        serde_json::json!({"file":&text.source[start..end]})
                    }))
                    .or_else(|| fancyverb_range.map(|range| {
                        let start = text.offset(range.start).unwrap();
                        let end = text.offset(range.end).unwrap();
                        serde_json::json!({"file":&text.source[start..end]})
                    }))
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

fn pdftex_error(message: &str) -> bool {
    message
        .strip_prefix("pdfTeX error")
        .is_some_and(|tail| tail.starts_with(':') || tail.starts_with(" ("))
}

fn error_message(line: &str) -> Option<&str> {
    line.strip_prefix("! ").or_else(|| {
        let message = line.strip_prefix('!')?;
        pdftex_error(message).then_some(message)
    })
}

fn fatal_summary(message: &str) -> bool {
    message.trim() == "==> Fatal error occurred, no output PDF file produced!"
}

fn incomplete_engine_file(message: &str) -> bool {
    pdftex_error(message)
        && message
            .rsplit_once("(file ")
            .is_some_and(|(_, file)| !file.contains("):"))
}

fn driver_message(line: &str) -> Option<(&'static str, DiagnosticSeverity, &str)> {
    let line = line.trim_start();
    let (driver, tail) = ["xdvipdfmx", "dvipdfmx"].into_iter().find_map(|driver| {
        line.strip_prefix(driver)
            .and_then(|tail| tail.strip_prefix(':'))
            .map(|tail| (driver, tail))
    })?;
    let (severity, body) = if let Some(body) = tail.strip_prefix("fatal:") {
        (DiagnosticSeverity::ERROR, body)
    } else {
        (DiagnosticSeverity::WARNING, tail.strip_prefix("warning:")?)
    };
    let body = body.trim();
    (!body.is_empty()).then_some((driver, severity, body))
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
        || error_message(line).is_some()
        || driver_message(line).is_some()
        || line.starts_with("==> Fatal error occurred")
        || line.starts_with("Transcript written on ")
        || line.starts_with("Output written on ")
        || line == "No pages of output."
        || line == "No output PDF file written."
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
    if initial == "FancyVerb Error:"
        && lines
            .get(*i + 1)
            .is_some_and(|line| line.starts_with("  No verbatim file "))
    {
        // The package deliberately puts its final period on a separate line.
        // A period at a print-width boundary can instead belong to a filename.
        // Do not consume an incomplete block or an adjacent diagnostic.
        let mut message = initial.to_owned();
        for (index, next) in lines.iter().enumerate().skip(*i + 1).take(64) {
            if *next == "." && message.contains("  No verbatim file ") {
                message.push('.');
                *i = index;
                return message;
            }
            if next.trim().is_empty()
                || message_boundary(next)
                || next.starts_with('\\')
                || message.len() + next.len() > 8192
            {
                break;
            }
            message.push_str(next);
        }
        return initial.to_owned();
    }
    let mut message = initial.to_owned();
    for _ in 0..64 {
        // A wrap can land on a period inside a driver or filename. It is only
        // the end of a recognized file error once its closing text is present.
        let incomplete_file = [
            "LaTeX Error: File ",
            "I can't find file ",
            "Package pdftex.def Error: File ",
            "Package luatex.def Error: File ",
            "Package Listings Error: File ",
            "Unable to load picture or PDF file ",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix) || prefix.starts_with(&message))
            && missing_graphic_name(&message).is_none()
            && missing_listing_name(&message).is_none()
            || incomplete_engine_file(&message);
        if message.trim_end().ends_with(['.', '!']) && !incomplete_file || message.len() >= 4096 {
            break;
        }
        let Some(next) = lines.get(*i + 1) else { break };
        let engine_file_close =
            incomplete_engine_file(&message) && next.trim_start().starts_with("):");
        if next.trim().is_empty()
            || (message_boundary(next) && !engine_file_close)
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
                    || error_message(next).is_some()
                    || driver_message(next).is_some()
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

/// PDF driver errors can appear only on stderr, after TeX closes its log. Keep
/// intervening blank barriers so unrelated driver detail blocks stay separate.
pub(crate) fn parse_driver_output(root: &Path, output: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    let mut log = String::new();
    for line in output.lines() {
        if driver_message(line).is_some() || line == crate::output::TRUNCATED {
            log.push_str(line);
        }
        log.push('\n');
    }
    parse(root, &log)
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
        if let Some((driver, severity, body)) = driver_message(line) {
            if let Some((path, cause, _)) = pending.take() {
                emit_with_context(
                    &mut result,
                    &path.path,
                    1,
                    cause,
                    DiagnosticSeverity::ERROR,
                    None,
                    path.confirmed,
                );
            }
            let level = if severity == DiagnosticSeverity::ERROR {
                "fatal"
            } else {
                "warning"
            };
            let mut message = format!("{driver}:{level}: {body}");
            if severity == DiagnosticSeverity::WARNING {
                for _ in 0..64 {
                    let Some((next_driver, next_severity, next_body)) =
                        lines.get(i + 1).and_then(|line| driver_message(line))
                    else {
                        break;
                    };
                    if next_driver != driver
                        || next_severity != severity
                        || !next_body.starts_with(">>")
                        || message.len() + next_body.len() + 1 > 4096
                    {
                        break;
                    }
                    message.push('\n');
                    message.push_str(next_body);
                    i += 1;
                }
            }
            // The PDF driver has no TeX source-line context. Its messages must
            // not borrow an open input file or a later l.N from the TeX log.
            emit_with_context(&mut result, root, 1, message, severity, None, false);
            i += 1;
            continue;
        }
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
                        && missing_read_file(&cause)
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
        } else if let Some(message) = error_message(line) {
            let message = wrapped_error_message(root, &lines, &separator_lines, &mut i, message);
            if let Some((path, cause, context_start)) = pending.take() {
                let (cause_line, context) = if path.confirmed
                    && path == current()
                    && message == "Emergency stop."
                    && missing_read_file(&cause)
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
            if path.confirmed {
                // Both commands can print the same text. Match independently
                // and prefer the complete longest message across families.
                let mut include_end = i;
                let include =
                    missing_reported_file(&path.path, &lines, &mut include_end, "include");
                let mut verbatim_end = i;
                let verbatim =
                    missing_reported_file(&path.path, &lines, &mut verbatim_end, "verbatiminput");
                i = include_end.max(verbatim_end);
                if let Some(name) = include.filter(|_| include_end == i) {
                    emit(
                        &mut result,
                        &path.path,
                        1,
                        format!("Included file `{name}` not found"),
                        DiagnosticSeverity::WARNING,
                    );
                }
                if let Some(name) = verbatim.filter(|_| verbatim_end == i) {
                    emit_missing_verbatim(&mut result, &path.path, &name);
                }
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
    // LuaTeX also marks its generic fatal trailer as an error. Keep it only
    // when the log provides no specific cause, otherwise it can sort before
    // the actual failure and become the build's misleading summary.
    let specific_error = result.values().flatten().any(|diagnostic| {
        diagnostic.severity == Some(DiagnosticSeverity::ERROR)
            && !diagnostic.message.trim().is_empty()
            && !fatal_summary(&diagnostic.message)
    });
    if specific_error {
        for diagnostics in result.values_mut() {
            diagnostics.retain(|diagnostic| !fatal_summary(&diagnostic.message));
        }
        result.retain(|_, diagnostics| !diagnostics.is_empty());
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
    let mut result = parse_bibliography_in(root, &paths, log);
    if !crate::output::truncated(log) {
        if let Ok(uri) = Url::from_file_path(root) {
            let sources = BTreeSet::from([uri]);
            locate_missing_bibliography_styles(&sources, &mut result);
            locate_missing_bibliography_databases(&sources, &mut result);
        }
    }
    result
}

#[derive(Clone, Copy)]
enum MissingBibliographyKind {
    Style,
    Database,
}

impl MissingBibliographyKind {
    fn code(self) -> &'static str {
        match self {
            Self::Style => "missing-bibliography-style",
            Self::Database => "missing-bibliography-database",
        }
    }

    fn commands(self) -> &'static [&'static str] {
        match self {
            Self::Style => &["bibliographystyle"],
            Self::Database => &["bibliography", "nobibliography"],
        }
    }

    fn matches(self, diagnostic: &Diagnostic) -> bool {
        diagnostic.code == Some(NumberOrString::String(self.code().into()))
            && (diagnostic.severity == Some(DiagnosticSeverity::ERROR)
                || matches!(self, Self::Database)
                    && diagnostic.severity == Some(DiagnosticSeverity::WARNING)
                    && diagnostic
                        .message
                        .starts_with("Latexmk: Bibliography database not found: "))
    }

    fn name(self, message: &str) -> Option<&str> {
        let (prefix, suffix) = match self {
            Self::Style => ("BibTeX: I couldn't open style file ", ".bst"),
            Self::Database => ("BibTeX: I couldn't open database file ", ".bib"),
        };
        let name = message.strip_prefix(prefix).or_else(|| {
            matches!(self, Self::Database)
                .then(|| message.strip_prefix("Latexmk: Bibliography database not found: "))
                .flatten()
        })?;
        let stem = name.strip_suffix(suffix)?;
        (!stem.is_empty()
            && crate::syntax::is_literal_file_path(name)
            && !name.chars().any(char::is_whitespace))
        .then_some(name)
    }
}

pub(crate) fn missing_bibliography_style_name(message: &str) -> Option<&str> {
    MissingBibliographyKind::Style.name(message)
}

pub(crate) fn missing_bibliography_database_name(message: &str) -> Option<&str> {
    MissingBibliographyKind::Database.name(message)
}

pub(crate) fn locate_missing_bibliography_styles(
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
) {
    locate_missing_bibliography_files(MissingBibliographyKind::Style, sources, diagnostics);
}

pub(crate) fn locate_missing_bibliography_databases(
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
) {
    locate_missing_bibliography_files(MissingBibliographyKind::Database, sources, diagnostics);
}

/// Auxiliary line numbers identify generated data, not the originating TeX
/// declaration. Only a unique static declaration in confirmed sources can
/// provide a source range; unread namesakes must not participate.
fn locate_missing_bibliography_files(
    kind: MissingBibliographyKind,
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
) {
    use std::io::Read;

    let requested: BTreeSet<_> = diagnostics
        .values()
        .flatten()
        .filter(|item| kind.matches(item))
        .filter_map(|item| kind.name(&item.message).map(str::to_owned))
        .collect();
    if requested.is_empty() || requested.len() > 64 {
        return;
    }
    let mut occurrences: BTreeMap<String, Vec<(Url, Range, String)>> = BTreeMap::new();
    let mut total = 0usize;
    let mut count = 0usize;
    for uri in sources {
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("bib" | "bst")
        ) {
            continue;
        }
        count += 1;
        if count > 1024
            || std::fs::metadata(&path).map_or(true, |metadata| metadata.len() > 2 * 1024 * 1024)
        {
            return;
        }
        let Ok(file) = std::fs::File::open(path) else {
            return;
        };
        let mut source = String::new();
        if file
            .take(2 * 1024 * 1024 + 1)
            .read_to_string(&mut source)
            .is_err()
        {
            return;
        }
        total += source.len();
        if source.len() > 2 * 1024 * 1024 || total > 16 * 1024 * 1024 {
            return;
        }
        let text = Text::new(source);
        let syntax = crate::syntax::Syntax::latex(&text);
        if syntax
            .definitions
            .iter()
            .any(|entry| kind.commands().contains(&entry.name.as_str()))
        {
            return;
        }
        for command in syntax
            .commands
            .iter()
            .filter(|command| kind.commands().contains(&command.name.as_str()))
        {
            // An expanded or spliced name might be the actual failed file.
            // Do not choose a static namesake by guessing execution order.
            let spans = match kind {
                MissingBibliographyKind::Style => {
                    let Some(argument) = crate::syntax::file_argument(&text.source, command) else {
                        return;
                    };
                    vec![argument.span.clone()]
                }
                MissingBibliographyKind::Database => {
                    let Some(spans) =
                        crate::syntax::complete_file_list_spans(&text.source, command)
                    else {
                        return;
                    };
                    spans
                }
            };
            for span in spans {
                let value = &text.source[span.clone()];
                let name = match kind {
                    // BibTeX prints an extra suffix for explicit style suffixes,
                    // whereas database names are reported with just one .bib.
                    MissingBibliographyKind::Style => format!("{value}.bst"),
                    MissingBibliographyKind::Database => {
                        let Some(name) =
                            crate::syntax::file_lookup_names(&command.name, value).next()
                        else {
                            return;
                        };
                        name.into_owned()
                    }
                };
                if requested.contains(&name) {
                    let entries = occurrences.entry(name).or_default();
                    if entries.len() < 2 {
                        entries.push((
                            uri.clone(),
                            text.range(span.start, span.end),
                            command.name.clone(),
                        ));
                    }
                }
            }
        }
    }
    let mut relocated = Vec::new();
    for entries in diagnostics.values_mut() {
        entries.retain(|item| {
            if !kind.matches(item) {
                return true;
            }
            let Some(name) = kind.name(&item.message) else {
                return true;
            };
            if item
                .data
                .as_ref()
                .and_then(|data| data.get("file"))
                .and_then(|name| name.as_str())
                != Some(name)
            {
                return true;
            }
            let Some(candidates) = occurrences.get(name).filter(|entries| entries.len() == 1)
            else {
                return true;
            };
            let (uri, range, command) = &candidates[0];
            let mut item = item.clone();
            item.range = *range;
            item.data = Some(match kind {
                MissingBibliographyKind::Style => serde_json::json!({"file":name}),
                MissingBibliographyKind::Database => {
                    serde_json::json!({"file":name,"bibliographyCommand":command})
                }
            });
            relocated.push((uri.clone(), item));
            false
        });
    }
    diagnostics.retain(|_, entries| !entries.is_empty());
    for (uri, item) in relocated {
        diagnostics.entry(uri).or_default().push(item);
    }
}

/// latexmk can skip BibTeX when a database is absent, even with a successful
/// exit and a prebuilt .bbl. Read only its final log-inspection block, so TeX
/// messages and earlier passes cannot revive an already repaired failure.
pub(crate) fn parse_latexmk_missing_databases(
    root: &Path,
    log_path: &Path,
    output: &str,
) -> BTreeMap<Url, Vec<Diagnostic>> {
    let mut result = BTreeMap::new();
    if crate::output::truncated(output) {
        return result;
    }
    let Ok(expected) = log_path.canonicalize() else {
        return result;
    };
    let parent = root.parent().unwrap_or_else(|| Path::new("."));
    let lines: Vec<_> = output.lines().collect();
    let Some(start) = lines.iter().rposition(|line| {
        line.strip_prefix("Latexmk: Getting log file '")
            .and_then(|name| name.strip_suffix('\''))
            .is_some_and(|name| {
                parent
                    .join(name)
                    .canonicalize()
                    .is_ok_and(|path| path == expected)
            })
    }) else {
        return result;
    };
    let tail = &lines[start + 1..];
    let end = tail
        .iter()
        .position(|line| {
            line.starts_with("Latexmk: applying rule ")
                || line.starts_with("Running '")
                || line.starts_with("Latexmk: Getting log file ")
        })
        .unwrap_or(tail.len());
    let driver = &tail[..end];
    let Some(header) = driver
        .iter()
        .rposition(|line| *line == "Latexmk: Found bibliography file(s):")
    else {
        return result;
    };
    let mut index = header + 1;
    let mut found = BTreeSet::new();
    while let Some(name) = driver.get(index).and_then(|line| line.strip_prefix("  ")) {
        if found.len() == 4096 || name.len() > 8192 {
            return result;
        }
        found.insert(name);
        index += 1;
    }
    if found.is_empty() || driver.get(index) != Some(&"Bib file(s) not found in search path:") {
        return result;
    }
    index += 1;
    let mut missing = BTreeSet::new();
    while let Some(name) = driver.get(index).and_then(|line| line.strip_prefix("  ")) {
        let message = format!("Latexmk: Bibliography database not found: {name}");
        if missing.len() == 64
            || name.len() > 8192
            || !found.contains(name)
            || missing_bibliography_database_name(&message).is_none()
        {
            return result;
        }
        missing.insert(name);
        index += 1;
    }
    // A process can stop midway through an otherwise literal filename. A
    // following driver event confirms that the entire list was printed.
    if !tail
        .iter()
        .skip(index)
        .take(16)
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| line.starts_with("Latexmk:") || line.starts_with("Running '"))
    {
        return result;
    }
    for name in missing {
        let message = format!("Latexmk: Bibliography database not found: {name}");
        emit(
            &mut result,
            root,
            1,
            message.clone(),
            DiagnosticSeverity::WARNING,
        );
        if let Ok(uri) = Url::from_file_path(root) {
            if let Some(entries) = result.get_mut(&uri) {
                for item in entries.iter_mut().filter(|item| item.message == message) {
                    item.code = Some(NumberOrString::String(
                        "missing-bibliography-database".into(),
                    ));
                    item.data = Some(serde_json::json!({"file":name,"unresolvedSource":true}));
                }
            }
        }
    }
    result
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
                    let message = format!("BibTeX: {}", message.trim());
                    let missing_file = (Path::new(name.trim().trim_matches('"'))
                        .extension()
                        .and_then(|extension| extension.to_str())
                        == Some("aux"))
                    .then(|| {
                        [
                            MissingBibliographyKind::Style,
                            MissingBibliographyKind::Database,
                        ]
                        .into_iter()
                        .find_map(|kind| kind.name(&message).map(|name| (kind, name)))
                    })
                    .flatten();
                    emit(
                        &mut result,
                        &path,
                        number,
                        message.clone(),
                        DiagnosticSeverity::ERROR,
                    );
                    if let (Some((kind, file)), Ok(uri)) = (missing_file, Url::from_file_path(root))
                    {
                        if let Some(entries) = result.get_mut(&uri) {
                            for entry in entries.iter_mut().filter(|entry| {
                                entry.message == message
                                    && entry.severity == Some(DiagnosticSeverity::ERROR)
                            }) {
                                entry.code = Some(NumberOrString::String(kind.code().into()));
                                entry.data =
                                    Some(serde_json::json!({"file":file,"unresolvedSource":true}));
                            }
                        }
                    }
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
    bibliography_fields_with(message, filename, move |start| {
        bibliography_quote_end(message, start)
    })
}

fn bibliography_fields_with<'a>(
    message: &'a str,
    filename: Option<&'a ByteRange<usize>>,
    quote_end: impl Fn(usize) -> Option<usize> + 'a,
) -> impl Iterator<Item = (usize, char)> + 'a {
    let mut quoted_until = 0;
    message.char_indices().filter(move |&(index, ch)| {
        if index < quoted_until || filename.is_some_and(|span| span.contains(&index)) {
            return false;
        }
        let word_start = !message[..index].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if word_start && matches!(ch, '\'' | '"') {
            quoted_until = quote_end(index).map_or(message.len(), |end| end + 1);
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
    let mut unconfirmed = None;
    let explicit = bibliography_source_fields(paths, message).find_map(|(start, _)| {
        if message[..start].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_') {
            return None;
        }
        markers.iter().find_map(|marker| {
            let tail = message[start..].strip_prefix(*marker)?.trim_start();
            let generic = matches!(*marker, "in " | "In ");
            let file = tail
                .strip_prefix("file ")
                .or_else(|| tail.strip_prefix("File "));
            let tail = if generic {
                file.unwrap_or(tail).trim_start()
            } else {
                tail
            };
            let start = message.len() - tail.len();
            let location = if tail.starts_with(['\'', '"']) {
                let QuotedBibliographyPath {
                    path,
                    span,
                    recorded,
                } = bibliography_quoted_path(paths, message, start);
                // A generic quoted word is prose, unless it names a
                // bibliography or belongs to a complete logged source.
                if generic
                    && file.is_none()
                    && path.is_none()
                    && !recorded
                    && bibliography_quote_end(message, start)
                        .is_some_and(|end| !message[start + 1..end].contains(".bib"))
                {
                    return None;
                }
                BibliographyLocation::Named { path, span }
            } else {
                bibliography_explicit_unquoted_path(paths, message, start)?
            };
            // Prose such as `in parser` or `in 'parser'` must not preempt
            // the real file field. Retain an unresolved generic field only
            // if no stronger source follows it.
            if generic
                && file.is_none()
                && matches!(location, BibliographyLocation::Named { path: None, .. })
            {
                unconfirmed.get_or_insert(location);
                None
            } else {
                Some(location)
            }
        })
    });
    if let Some(location) = explicit.or(unconfirmed) {
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
            let QuotedBibliographyPath {
                path,
                span,
                recorded,
            } = bibliography_quoted_path(paths, message, index);
            if path.is_some() || recorded {
                return BibliographyLocation::Named { path, span };
            }
            let body = &message[index + 1..span.end];
            let first_quote = bibliography_quote_end(message, index);
            // A plain quoted word cannot consume another quoted database
            // merely because whitespace separates their closing quotes.
            let filename = first_quote.is_none_or(|end| message[index + 1..end].contains(".bib"))
                && body
                    .match_indices(ch)
                    .any(|(end, _)| body[..end].ends_with(".bib"));
            // Preserve uncertain filenames as a whole, but keep independent
            // quoted message strings from hiding a bare path between them.
            quoted_until = if filename {
                span.end
            } else {
                first_quote.map_or(message.len(), |end| end + 1)
            };
            continue;
        }
        if !ch.is_whitespace() && bibliography_token_start(message, index) {
            if let Some(location) = bibliography_recorded_unquoted_path(paths, message, index) {
                return location;
            }
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

fn bibliography_token_start(message: &str, start: usize) -> bool {
    start == 0
        || message[..start]
            .ends_with(|ch: char| ch.is_whitespace() || matches!(ch, ':' | '(' | '[' | '{'))
}

/// Known bare filenames can themselves contain quotes or apparent fields.
/// Skip their complete spans before looking for explicit source markers.
fn bibliography_source_fields<'a>(
    paths: &'a crate::recorder::BibliographyPaths<'_>,
    message: &'a str,
) -> impl Iterator<Item = (usize, char)> + 'a {
    let mut skip_until = 0;
    message.char_indices().filter(move |&(start, ch)| {
        if start < skip_until {
            return false;
        }
        let word_start = !message[..start].ends_with(|ch: char| ch.is_alphanumeric() || ch == '_');
        if word_start && matches!(ch, '\'' | '"') {
            skip_until = bibliography_recorded_quote_end(paths, message, start)
                .map(|(end, _)| end)
                .or_else(|| bibliography_quote_end(message, start))
                .map_or(message.len(), |end| end + 1);
            return false;
        }
        if !ch.is_whitespace() && bibliography_token_start(message, start) {
            if let Some(BibliographyLocation::Named { span, .. }) =
                bibliography_recorded_unquoted_path(paths, message, start)
            {
                skip_until = span.end;
                return false;
            }
        }
        true
    })
}

fn bibliography_unquoted_terminated(tail: &str) -> bool {
    tail.is_empty()
        || tail.starts_with(|ch: char| {
            ch.is_whitespace() || matches!(ch, ',' | ':' | ';' | ')' | ']' | '}')
        })
        || tail
            .strip_prefix('.')
            .is_some_and(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
}

/// Strip balanced message wrappers, preserving unmatched literal openers.
fn bibliography_unquoted_name(body: &str, end: usize) -> (&str, usize) {
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
    if wrapped {
        (&body[leading..end], body.len() - after.len())
    } else {
        (&body[..end], end)
    }
}

fn bibliography_recorded_unquoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> Option<BibliographyLocation> {
    let body = &message[start..];
    let token_end = body.find(char::is_whitespace).unwrap_or(body.len());
    // A recorded short name must not truncate a complete longer bare token.
    let minimum_end = body[..token_end]
        .rmatch_indices(".bib")
        .map(|(index, _)| index + 4)
        .find(|&end| bibliography_unquoted_terminated(&body[end..]))
        .unwrap_or(0);
    body.rmatch_indices(".bib").find_map(|(index, _)| {
        let end = index + 4;
        if end < minimum_end || !bibliography_unquoted_terminated(&body[end..]) {
            return None;
        }
        let (name, span_end) = bibliography_unquoted_name(body, end);
        // A literal opening bracket in a logged name takes precedence over
        // treating it as a wrapper supplied by the diagnostic message.
        let (name, span_end, source) = paths
            .recorded_bibliography_source(&body[..end])
            .map(|source| (&body[..end], end, source))
            .or_else(|| {
                paths
                    .recorded_bibliography_source(name)
                    .map(|source| (name, span_end, source))
            })?;
        let path = if source.selected {
            source.path
        } else {
            source.path.or_else(|| paths.resolve(name))
        };
        Some(BibliographyLocation::Named {
            path,
            span: start..start + span_end,
        })
    })
}

fn bibliography_unquoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> Option<BibliographyLocation> {
    if let Some(location) = bibliography_recorded_unquoted_path(paths, message, start) {
        return Some(location);
    }
    let body = &message[start..];
    let token_end = body.find(char::is_whitespace).unwrap_or(body.len());
    // A whole filename takes precedence over internal .bib fragments or
    // punctuation. A missing explicit source still restricts entry lookup.
    let mut extensions = body[..token_end].rmatch_indices(".bib").peekable();
    extensions.peek()?;
    let end = extensions.find_map(|(index, _)| {
        let end = index + 4;
        bibliography_unquoted_terminated(&body[end..]).then_some(end)
    });
    let (path, end) = match end {
        Some(end) => {
            let (name, span_end) = bibliography_unquoted_name(body, end);
            (paths.resolve(name), span_end)
        }
        None => (None, token_end),
    };
    Some(BibliographyLocation::Named {
        path,
        span: start..start + end,
    })
}

/// An explicit file field restricts lookup even without an input record.
/// Its complete name can contain spaces; quoted error text is not a boundary.
fn bibliography_explicit_unquoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> Option<BibliographyLocation> {
    let recorded = bibliography_recorded_unquoted_path(paths, message, start);
    let body = &message[start..];
    let last_extension = bibliography_fields(body, None)
        .filter_map(|(index, _)| body[index..].starts_with(".bib").then_some(index + 4))
        .last();
    if let Some(BibliographyLocation::Named { path, span }) = recorded {
        // Keep the selected input's identity, including disappearance. A
        // complete longer explicit name can still override a logged prefix.
        if last_extension.is_none_or(|end| start + end <= span.end) {
            return Some(BibliographyLocation::Named { path, span });
        }
    }
    let Some(end) = last_extension else {
        return bibliography_unquoted_path(paths, message, start);
    };
    let (path, end) = if bibliography_unquoted_terminated(&body[end..]) {
        let (name, span_end) = bibliography_unquoted_name(body, end);
        (paths.resolve(name), span_end)
    } else {
        // A longer incomplete/unsupported name cannot borrow its existing
        // .bib prefix, nor a quoted filename elsewhere in the message.
        let end = body[end..]
            .find(char::is_whitespace)
            .map_or(body.len(), |offset| end + offset);
        (None, end)
    };
    Some(BibliographyLocation::Named {
        path,
        span: start..start + end,
    })
}

fn bibliography_quote_terminated(tail: &str) -> bool {
    let after = tail.trim_start_matches([',', ':', ';', ')', ']', '}', '.']);
    tail.is_empty()
        || (after.len() != tail.len()
            && (after.is_empty() || after.starts_with(char::is_whitespace)))
}

/// Logged names disambiguate literal quotes even when punctuation and spaces
/// follow them. Match the longest complete source across equivalent paths;
/// shorter existing files cannot replace a missing recorded source.
fn bibliography_recorded_quote_end(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> Option<(usize, crate::recorder::RecordedBibliographySource)> {
    let quote = message.as_bytes()[start] as char;
    let body = &message[start + 1..];
    body.rmatch_indices(quote).find_map(|(end, _)| {
        let name = &body[..end];
        if !name.ends_with(".bib") {
            return None;
        }
        let tail = &body[end + 1..];
        if !tail.starts_with(char::is_whitespace) && !bibliography_quote_terminated(tail) {
            return None;
        }
        paths
            .recorded_bibliography_source(name)
            .map(|source| (start + 1 + end, source))
    })
}

struct QuotedBibliographyPath {
    path: Option<PathBuf>,
    span: ByteRange<usize>,
    recorded: bool,
}

fn bibliography_quoted_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
    start: usize,
) -> QuotedBibliographyPath {
    let parsed = bibliography_name_quote_end(message, start);
    let recorded = bibliography_recorded_quote_end(paths, message, start).filter(|(end, _)| {
        // A recorded prefix cannot truncate an otherwise complete longer
        // filename, including a source missing from this tool's input list.
        !parsed.is_some_and(|parsed| parsed > *end && message[start + 1..parsed].ends_with(".bib"))
    });
    let is_recorded = recorded.is_some();
    let (end, path) = if let Some((end, source)) = recorded {
        // A complete explicit file can be outside the input list. A source
        // actually selected by this context must never switch after deletion.
        let path = if source.selected {
            source.path
        } else {
            source
                .path
                .or_else(|| paths.resolve(&message[start + 1..end]))
        };
        (Some(end), path)
    } else {
        (
            parsed,
            parsed.and_then(|end| {
                let name = &message[start + 1..end];
                name.ends_with(".bib")
                    .then(|| paths.resolve(name))
                    .flatten()
            }),
        )
    };
    QuotedBibliographyPath {
        path,
        span: start..end.map_or(message.len(), |end| end + 1),
        recorded: is_recorded,
    }
}

fn bibliography_name_quote_end(message: &str, start: usize) -> Option<usize> {
    let quote = message.as_bytes()[start] as char;
    let body = &message[start + 1..];
    let mut candidate = None;
    let mut ambiguous = false;
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
            end > 0 && bibliography_quote_terminated(tail[end..].trim_start())
        });
        let strong = bibliography_quote_terminated(tail) || line;
        if strong {
            return Some(start + 1 + end);
        }
        if tail.starts_with(char::is_whitespace) {
            ambiguous |= candidate.is_some();
            candidate = Some(start + 1 + end);
        }
    }
    if ambiguous {
        None
    } else {
        candidate
    }
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
    fn biber_explicit_paths_preserve_unreported_files_and_do_not_replace_changed_inputs() {
        for name in ["plain.bib", "refs.bib': copy.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let selected = root.join("selected");
            std::fs::create_dir(&selected).unwrap();
            let main = root.join("main.tex");
            let blg = root.join("main.blg");
            let explicit = root.join(name);
            let input = selected.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(&explicit, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            std::fs::write(&input, "@book{entry,title={Read input}}\n").unwrap();
            for source in [explicit.to_str().unwrap().to_owned(), format!("../{name}")] {
                let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - syntax error in file '{source}', line 3\nWARN - Missing field for entry 'entry' (file '{source}')\n");
                let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                    .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
                let diagnostics = parse_bibliography_in(&main, &paths, &log);
                let items = diagnostics
                    .get(&Url::from_file_path(&explicit).unwrap())
                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(items.len(), 2);
                assert!(
                    items
                        .iter()
                        .all(|item| item.range.start.line == 2 && !unresolved_location(item)),
                    "{log}: {items:?}"
                );
                assert_eq!(
                    crate::recorder::bibliography_inputs_in(&paths, &log),
                    [Url::from_file_path(&input).unwrap()].into()
                );
            }
            let log = format!("INFO - Found BibTeX data source '{name}'\nWARN - Missing field for entry 'entry' (file '{name}')\n");
            let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
            assert_eq!(
                paths.recorded_bibliography_source(name).unwrap().path,
                Some(input.clone())
            );
            std::fs::remove_file(&input).unwrap();
            let diagnostics = parse_bibliography_in(&main, &paths, &log);
            assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_recorded_sources_share_absolute_relative_and_dot_spellings() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("inputs")).unwrap();
        std::fs::create_dir(root.join("build")).unwrap();
        let main = root.join("main.tex");
        let blg = root.join("build/main.blg");
        let name = "inputs/中文 refs.bib': in file 'wrong.bib' line 700 copy.bib";
        let bib = root.join(name);
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
        for decoy in ["inputs/中文 refs.bib", "wrong.bib", "copy.bib"] {
            std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
        }
        let spellings = [
            name.to_owned(),
            format!("./{name}"),
            bib.to_str().unwrap().to_owned(),
            root.join(format!("./{name}")).to_str().unwrap().to_owned(),
        ];
        for missing in [false, true] {
            if missing {
                std::fs::remove_file(&bib).unwrap();
            }
            for input in &spellings {
                for source in &spellings {
                    for quote in ['\'', '"'] {
                        for message in [
                            format!("syntax error in file {quote}{source}{quote}, line 3"),
                            format!("syntax error {quote}{source}{quote}, line 3"),
                            format!("Missing field {quote}{source}{quote} for entry 'entry'"),
                        ] {
                            let log = format!(
                                "INFO - Found BibTeX data source '{input}'\n{} - {message}\n",
                                if message.starts_with("Missing") {
                                    "WARN"
                                } else {
                                    "ERROR"
                                }
                            );
                            let diagnostics = parse_bibliography_log(&main, &blg, &log);
                            let target = if missing { &main } else { &bib };
                            let items = diagnostics
                                .get(&Url::from_file_path(target).unwrap())
                                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                            assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                            assert_eq!(items.len(), 1);
                            if missing {
                                assert_eq!(items[0].range.start.line, 0);
                                assert_eq!(
                                    items[0].data.as_ref().unwrap()["unresolvedSource"],
                                    true
                                );
                            } else {
                                assert_eq!(items[0].range.start.line, 2, "{log}: {items:?}");
                                assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                                if message.starts_with("Missing") {
                                    assert_eq!(
                                        items[0].range,
                                        Range::new(Position::new(2, 6), Position::new(2, 11))
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_recorded_sources_respect_search_paths_and_selected_working_directories() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let output = root.join("build");
        std::fs::create_dir(&output).unwrap();
        let main = root.join("main.tex");
        let blg = output.join("main.blg");
        let name = "refs.bib': line 700 copy.bib";
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        for base in [root, &output] {
            std::fs::write(base.join(name), "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            std::fs::write(base.join("refs.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        }
        for auxiliary in ["main.aux", "build/main.aux"] {
            for selected in [root, &output] {
                for absolute_input in [false, true] {
                    let input = if absolute_input {
                        selected.join(name).to_str().unwrap().to_owned()
                    } else {
                        name.to_owned()
                    };
                    let source = if absolute_input {
                        name.to_owned()
                    } else {
                        selected.join(name).to_str().unwrap().to_owned()
                    };
                    let log = format!("The top-level auxiliary file: {auxiliary}\nINFO - Found BibTeX data source '{input}'\nWARN - Missing field for entry 'entry' (file '{source}')\n");
                    let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                        .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
                    let diagnostics = parse_bibliography_in(&main, &paths, &log);
                    let target = Url::from_file_path(selected.join(name)).unwrap();
                    let items = diagnostics
                        .get(&target)
                        .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                    assert_eq!(diagnostics.len(), 1);
                    assert_eq!(
                        items[0].range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
            }
        }
        // An absolute input record disambiguates an unreported full-name
        // lookalike in cwd, even if cwd precedes the log directory.
        let actual = output.join(name);
        let log = format!("INFO - Found BibTeX data source '{}'\nWARN - Missing field for entry 'entry' (file '{name}')\n", actual.display());
        for missing in [false, true] {
            if missing {
                std::fs::remove_file(&actual).unwrap();
            }
            for search in [String::new(), format!("BIBINPUTS='{}'\n", root.display())] {
                let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                    .with_search_output(&search);
                let diagnostics = parse_bibliography_in(&main, &paths, &log);
                let target = if missing { &main } else { &actual };
                let items = &diagnostics[&Url::from_file_path(target).unwrap()];
                assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
                assert_eq!(items.len(), 1);
                if missing {
                    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
                } else {
                    assert_eq!(
                        items[0].range,
                        Range::new(Position::new(2, 6), Position::new(2, 11))
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn repeated_bibliography_records_do_not_hide_a_later_complete_source_name() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let name = "refs.bib': copy.bib";
        let bib = root.join(name);
        std::fs::write(&main, "\\cite{entry}\n").unwrap();
        std::fs::write(root.join("refs.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Actual}}\n").unwrap();
        let mut log = "INFO - Found BibTeX data source 'refs.bib'\n".repeat(4096);
        log.push_str(&format!("INFO - Found BibTeX data source '{name}'\nWARN - Missing field for entry 'entry' in file '{name}'\n"));
        let diagnostics = parse_bibliography(&main, &log);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let items = diagnostics
            .get(&Url::from_file_path(&bib).unwrap())
            .unwrap_or_else(|| panic!("{diagnostics:?}"));
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].range,
            Range::new(Position::new(1, 6), Position::new(1, 11))
        );
    }

    #[test]
    #[cfg(unix)]
    fn biber_logged_quoted_names_disambiguate_punctuation_spaces_and_metadata() {
        for suffix in [
            ": copy.bib",
            "; copy.bib",
            ") copy.bib",
            "] copy.bib",
            "} copy.bib",
            ", copy.bib",
            ": line 700 copy.bib",
            ", in file 'wrong.bib' copy.bib",
            ", entry 'wrong' copy.bib",
        ] {
            for quote in ['\'', '"'] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let name = format!("中文 refs.bib{quote}{suffix}");
                let bib = root.join(&name);
                std::fs::write(&main, "\\cite{entry}\n").unwrap();
                for decoy in ["中文 refs.bib", "copy.bib", "wrong.bib"] {
                    std::fs::write(
                        root.join(decoy),
                        "@book{entry,title={Wrong}}\n@book{wrong}\n",
                    )
                    .unwrap();
                }
                std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
                for missing in [false, true] {
                    if missing {
                        std::fs::remove_file(&bib).unwrap();
                    }
                    for outer in ['\'', '"'] {
                        for message in [
                            format!("syntax error in file {outer}{name}{outer}, line 3"),
                            format!("syntax error on line 3 in file {outer}{name}{outer}"),
                            format!("syntax error {outer}{name}{outer}: line 3"),
                            format!("Missing field for entry 'entry' (file {outer}{name}{outer})"),
                            format!("Missing field {outer}{name}{outer} for entry 'entry'"),
                        ] {
                            // Both names are reported; their order and existence
                            // cannot make a literal long name select the short one.
                            for names in [
                                format!("INFO - Found BibTeX data source '中文 refs.bib'\nINFO - Found BibTeX data source '{name}'\n"),
                                format!("INFO - Found BibTeX data source '{name}'\nINFO - Found BibTeX data source '中文 refs.bib'\n"),
                            ] {
                                let log = format!("{names}{} - {message}\n", if message.starts_with("Missing") { "WARN" } else { "ERROR" });
                                let diagnostics = parse_bibliography(&main, &log);
                                let target = if missing { &main } else { &bib };
                                let items = diagnostics.get(&Url::from_file_path(target).unwrap()).unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                                assert_eq!(items.len(), 1);
                                if missing {
                                    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true, "{log}: {items:?}");
                                    assert_eq!(items[0].range.start.line, 0);
                                } else {
                                    assert_eq!(items[0].range.start.line, 2, "{log}: {items:?}");
                                    assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                                    if message.starts_with("Missing") {
                                        assert_eq!(items[0].range, Range::new(Position::new(2, 6), Position::new(2, 11)), "{log}: {items:?}");
                                    }
                                }
                            }
                        }
                    }
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
    #[cfg(unix)]
    fn biber_logged_unquoted_sources_keep_complete_names_outside_message_fields() {
        for name in [
            "refs.bib line 700 copy.bib",
            "中文 refs.bib in file 'wrong.bib' entry 'wrong' copy.bib",
            "refs.bib': copy.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "\\cite{entry}\n").unwrap();
            std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            for decoy in ["refs.bib", "wrong.bib", "copy.bib"] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for input in [name.to_owned(), bib.to_str().unwrap().to_owned()] {
                    for source in [
                        name.to_owned(),
                        format!("./{name}"),
                        bib.to_str().unwrap().to_owned(),
                    ] {
                        for (open, close) in [("", ""), ("(", ")"), ("[({", "})]")] {
                            for message in [
                                format!("Unexpected token \"in 'wrong.bib', line 900\" in file {open}{source}{close}, line 3"),
                                format!("syntax error {open}{source}{close}, line 3"),
                                format!("Missing field {open}{source}{close} for entry 'entry'"),
                                format!("Missing field for entry 'entry' (file {open}{source}{close})"),
                            ] {
                                let log = format!("INFO - Found BibTeX data source '{input}'\nERROR - {message}\n");
                                let diagnostics = parse_bibliography(&main, &log);
                                let target = if missing { &main } else { &bib };
                                let items = diagnostics
                                    .get(&Url::from_file_path(target).unwrap())
                                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                                assert_eq!(items.len(), 1);
                                assert_eq!(items[0].message, format!("Biber: {message}"));
                                if missing {
                                    assert_eq!(items[0].range.start, Position::new(0, 0));
                                    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
                                } else {
                                    assert_eq!(items[0].range.start.line, 2, "{log}: {items:?}");
                                    assert!(!unresolved_location(&items[0]), "{log}: {items:?}");
                                    if message.starts_with("Missing") {
                                        assert_eq!(items[0].range, Range::new(Position::new(2, 6), Position::new(2, 11)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_recorded_unquoted_prefixes_do_not_shorten_complete_unreported_tokens() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, "Text\n").unwrap();
        std::fs::write(root.join("refs.bib"), "@book{entry,title={Wrong}}\n").unwrap();
        for name in ["refs.bib,copy.bib", "refs.bib:copy.bib"] {
            let bib = root.join(name);
            std::fs::write(&bib, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for message in [
                    format!("syntax error in file {name}, line 3"),
                    format!("Missing field for entry 'entry' (file {name})"),
                ] {
                    let log =
                        format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                    let diagnostics = parse_bibliography(&main, &log);
                    let target = if missing { &main } else { &bib };
                    let items = diagnostics
                        .get(&Url::from_file_path(target).unwrap())
                        .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                    assert_eq!(diagnostics.len(), 1);
                    assert_eq!(items[0].range.start.line, if missing { 0 } else { 2 });
                    assert_eq!(unresolved_location(&items[0]), missing, "{log}: {items:?}");
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_unquoted_sources_preserve_explicit_files_and_missing_selected_inputs() {
        for name in ["plain.bib", "refs.bib line 700 copy.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let selected = root.join("selected");
            std::fs::create_dir(&selected).unwrap();
            let main = root.join("main.tex");
            let explicit = root.join(name);
            let input = selected.join(name);
            let blg = root.join("main.blg");
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&explicit, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            std::fs::write(&input, "@book{entry,title={Selected}}\n").unwrap();
            let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - syntax error in file {}, line 3\nWARN - Missing field for entry 'entry' (file {})\n", explicit.display(), explicit.display());
            let paths = crate::recorder::BibliographyPaths::new(&main, &blg, &log)
                .with_search_output(&format!("BIBINPUTS='{}'\n", selected.display()));
            let diagnostics = parse_bibliography_in(&main, &paths, &log);
            assert_eq!(diagnostics.len(), 1);
            let items = &diagnostics[&Url::from_file_path(&explicit).unwrap()];
            assert_eq!(items.len(), 2);
            assert!(
                items
                    .iter()
                    .all(|item| item.range.start.line == 2 && !unresolved_location(item)),
                "{log}: {items:?}"
            );
            assert_eq!(
                paths.recorded_bibliography_source(name).unwrap().path,
                Some(input.clone())
            );
            std::fs::remove_file(&input).unwrap();
            let diagnostics = parse_bibliography_in(
                &main,
                &paths,
                &format!("WARN - Missing field for entry 'entry' (file {name})\n"),
            );
            assert_eq!(diagnostics.len(), 1);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items[0].range.start, Position::new(0, 0));
            assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_unreported_explicit_sources_keep_spaces_and_do_not_borrow_message_files() {
        for name in [
            "missing refs.bib",
            "refs.bib line 700 copy.bib",
            "refs.bib, copy.bib",
            "中文 refs.bib': copy.bib",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            for decoy in ["refs.bib", "wrong.bib", "copy.bib"] {
                std::fs::write(root.join(decoy), "@book{entry,title={Wrong}}\n").unwrap();
            }
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for marker in ["file", "File", "in", "In", "in file", "in \tFile"] {
                    for (open, close) in [("", ""), ("[({", "})]")] {
                        for message in [
                            format!(
                                "Unexpected token 'wrong.bib' {marker} {open}{name}{close}, line 3"
                            ),
                            format!(
                                "Missing field for entry 'entry' ({marker} {open}{name}{close})"
                            ),
                        ] {
                            let log = format!(
                                "INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n"
                            );
                            let diagnostics = parse_bibliography(&main, &log);
                            let target = if missing { &main } else { &bib };
                            let items = diagnostics
                                .get(&Url::from_file_path(target).unwrap())
                                .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                            assert_eq!(diagnostics.len(), 1);
                            assert_eq!(items.len(), 1);
                            assert_eq!(items[0].message, format!("Biber: {message}"));
                            assert_eq!(
                                items[0].range.start.line,
                                if missing { 0 } else { 2 },
                                "{log}: {items:?}"
                            );
                            assert_eq!(unresolved_location(&items[0]), missing, "{log}: {items:?}");
                            if !missing && message.starts_with("Missing") {
                                assert_eq!(
                                    items[0].range,
                                    Range::new(Position::new(2, 6), Position::new(2, 11))
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn biber_explicit_multiword_non_bibliography_names_do_not_borrow_short_sources() {
        for name in ["missing refs.bib.backup", "missing refs.bib2"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            std::fs::write(&main, "Text\n").unwrap();
            for file in [name, "refs.bib", "wrong.bib"] {
                std::fs::write(root.join(file), "@book{entry,title={Wrong}}\n").unwrap();
            }
            for message in [
                format!("Unexpected token 'wrong.bib' in file {name}, line 3"),
                format!("Missing field for entry 'entry' (file {name})"),
            ] {
                let log =
                    format!("INFO - Found BibTeX data source 'refs.bib'\nERROR - {message}\n");
                let diagnostics = parse_bibliography(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}: {diagnostics:?}");
                let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
                assert_eq!(items[0].range.start, Position::new(0, 0));
                assert_eq!(items[0].data.as_ref().unwrap()["unresolvedSource"], true);
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn biber_quoted_generic_prose_does_not_hide_the_actual_source_field() {
        for name in ["refs.bib", "中文 refs.bib': copy.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join(name);
            std::fs::write(&main, "Text\n").unwrap();
            std::fs::write(&bib, "% header\n\n@book{entry,title={Actual}}\n").unwrap();
            std::fs::write(root.join("wrong.bib"), "@book{entry,title={Wrong}}\n").unwrap();
            for missing in [false, true] {
                if missing {
                    std::fs::remove_file(&bib).unwrap();
                }
                for prose in ['\'', '"'] {
                    for quote in ['\'', '"'] {
                        for marker in ["file", "in file", "in"] {
                            for message in [
                                format!("Error in {prose}parser{prose}; {marker} {quote}{name}{quote}, line 3"),
                                format!("Missing field in {prose}parser{prose}; for entry 'entry' ({marker} {quote}{name}{quote})"),
                                format!("Error in {prose}parser{prose}; {quote}{name}{quote}, line 3"),
                                format!("Missing field in {prose}parser{prose}; {quote}{name}{quote} for entry 'entry'"),
                                format!("Error in {prose}parser{prose}; {name}, line 3"),
                                format!("Missing field in {prose}parser{prose}; {name} for entry 'entry'"),
                                format!("Error in {prose}parser{prose} {quote}{name}{quote}, line 3"),
                                format!("Missing field in {prose}parser{prose} {quote}{name}{quote} for entry 'entry'"),
                            ] {
                                let log = format!("INFO - Found BibTeX data source '{name}'\nERROR - {message}\n");
                                let diagnostics = parse_bibliography(&main, &log);
                                let target = if missing { &main } else { &bib };
                                let items = diagnostics.get(&Url::from_file_path(target).unwrap())
                                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"));
                                assert_eq!(diagnostics.len(), 1);
                                assert_eq!(items[0].message, format!("Biber: {message}"));
                                assert_eq!(items[0].range.start.line, if missing { 0 } else { 2 }, "{log}: {items:?}");
                                assert_eq!(unresolved_location(&items[0]), missing, "{log}: {items:?}");
                                if !missing && message.starts_with("Missing") {
                                    assert_eq!(items[0].range, Range::new(Position::new(2, 6), Position::new(2, 11)));
                                }
                            }
                        }
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

#[cfg(test)]
mod bibliography_style_locations {
    use super::*;

    fn missing(root: &Path) -> BTreeMap<Url, Vec<Diagnostic>> {
        parse_bibliography(
            root,
            "I couldn't open style file local,styel.bst\n---line 2 of file main.aux\n",
        )
    }

    #[test]
    fn style_locations_use_only_confirmed_sources_and_preserve_ambiguous_failures() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let settings = root.join("settings.tex");
        let unread = root.join("unread.tex");
        std::fs::write(&main, "\\input{settings}\n").unwrap();
        let source = "% emoji😀\r\n\\bibliographystyle{% before\r\n local,styel% after\r\n}\r\n";
        std::fs::write(&settings, source).unwrap();
        std::fs::write(&unread, source).unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let settings_uri = Url::from_file_path(&settings).unwrap();
        let unread_uri = Url::from_file_path(&unread).unwrap();
        let sources = BTreeSet::from([main_uri.clone(), settings_uri.clone()]);
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert!(!diagnostics.contains_key(&main_uri));
        let item = &diagnostics[&settings_uri][0];
        let start = source.find(" local,styel").unwrap() + 1;
        assert_eq!(
            item.range,
            Text::new(source.into()).range(start, start + "local,styel".len())
        );
        assert_eq!(
            item.data,
            Some(serde_json::json!({"file":"local,styel.bst"}))
        );

        let mut sources = sources;
        sources.insert(unread_uri);
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert!(!diagnostics.contains_key(&settings_uri));
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
        std::fs::write(&unread, r"\bibliographystyle{\chosen}").unwrap();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
    }

    #[test]
    fn unavailable_oversized_or_truncated_style_sources_do_not_create_false_unique_locations() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let settings = root.join("settings.tex");
        let other = root.join("other.tex");
        std::fs::write(&main, "Main.\n").unwrap();
        std::fs::write(&settings, r"\bibliographystyle{local,styel}").unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let sources = [main.clone(), settings, other.clone()]
            .into_iter()
            .map(|path| Url::from_file_path(path).unwrap())
            .collect();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
        std::fs::write(other, "%".repeat(2 * 1024 * 1024 + 1)).unwrap();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_styles(&sources, &mut diagnostics);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
        std::fs::write(&main, r"\bibliographystyle{local,styel}").unwrap();
        let log = format!(
            "I couldn't open style file local,styel.bst\n---line 2 of file main.aux\n{}\n",
            crate::output::TRUNCATED
        );
        let diagnostics = parse_bibliography(&main, &log);
        assert_eq!(
            diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
            true
        );
    }
}

#[cfg(test)]
mod bibliography_database_locations {
    use super::*;

    fn missing(root: &Path) -> BTreeMap<Url, Vec<Diagnostic>> {
        parse_bibliography(
            root,
            "I couldn't open database file reffs.bib\n---line 3 of file main.aux\n",
        )
    }

    #[test]
    fn databases_use_confirmed_sources_including_local_packages_and_reject_ambiguity() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(&main, r"\usepackage{settings}").unwrap();
        let source = "\\bibliography{first,% before\r\n re ffs.bib% after\r\n,second}";
        let settings = root.join("settings.sty");
        let unread = root.join("unread.tex");
        std::fs::write(&settings, source).unwrap();
        std::fs::write(&unread, source).unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let settings_uri = Url::from_file_path(&settings).unwrap();
        let unread_uri = Url::from_file_path(&unread).unwrap();
        let mut sources = BTreeSet::from([main_uri.clone(), settings_uri.clone()]);
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_databases(&sources, &mut diagnostics);
        assert!(!diagnostics.contains_key(&main_uri));
        let item = &diagnostics[&settings_uri][0];
        let start = source.find("re ffs").unwrap();
        assert_eq!(
            item.range,
            Text::new(source.into()).range(start, start + "re ffs.bib".len())
        );
        assert_eq!(
            item.data,
            Some(serde_json::json!({"file":"reffs.bib","bibliographyCommand":"bibliography"}))
        );
        sources.insert(unread_uri);
        for source in [
            source,
            r"\nobibliography{\chosen}",
            r"\renewcommand{\bibliography}[1]{}",
        ] {
            std::fs::write(&unread, source).unwrap();
            let mut diagnostics = missing(&main);
            locate_missing_bibliography_databases(&sources, &mut diagnostics);
            assert_eq!(
                diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
                true
            );
        }
    }

    #[test]
    fn incomplete_database_sources_keep_unresolved_errors() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let settings = root.join("settings.tex");
        let other = root.join("other.tex");
        std::fs::write(&main, "Main.\n").unwrap();
        std::fs::write(&settings, r"\bibliography{reffs}").unwrap();
        let main_uri = Url::from_file_path(&main).unwrap();
        let sources = [&main, &settings, &other]
            .into_iter()
            .map(|path| Url::from_file_path(path).unwrap())
            .collect();
        let assert_unresolved = |diagnostics: &BTreeMap<Url, Vec<Diagnostic>>| {
            assert_eq!(
                diagnostics[&main_uri][0].data.as_ref().unwrap()["unresolvedSource"],
                true
            )
        };
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_databases(&sources, &mut diagnostics);
        assert_unresolved(&diagnostics);
        std::fs::write(&other, "%".repeat(2 * 1024 * 1024 + 1)).unwrap();
        let mut diagnostics = missing(&main);
        locate_missing_bibliography_databases(&sources, &mut diagnostics);
        assert_unresolved(&diagnostics);
        std::fs::write(&main, r"\bibliography{reffs}").unwrap();
        let log = format!(
            "I couldn't open database file reffs.bib\n---line 3 of file main.aux\n{}\n",
            crate::output::TRUNCATED
        );
        assert_unresolved(&parse_bibliography(&main, &log));
    }

    #[test]
    fn style_and_database_uncertainty_is_independent() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let log="I couldn't open database file reffs.bib\n---line 3 of file main.aux\nI couldn't open style file styel.bst\n---line 2 of file main.aux\n";
        for (source, precise, unresolved) in [
            (
                r"\bibliographystyle{styel}\bibliography{\chosen}",
                "missing-bibliography-style",
                "missing-bibliography-database",
            ),
            (
                r"\bibliographystyle{\chosen}\bibliography{reffs}",
                "missing-bibliography-database",
                "missing-bibliography-style",
            ),
        ] {
            std::fs::write(&main, source).unwrap();
            let diagnostics = parse_bibliography(&main, log);
            let find = |code: &str| {
                diagnostics
                    .values()
                    .flatten()
                    .find(|item| item.code == Some(NumberOrString::String(code.into())))
                    .unwrap()
            };
            assert!(!unresolved_location(find(precise)));
            assert!(unresolved_location(find(unresolved)));
        }
    }
}

#[cfg(test)]
mod latexmk_database_warnings {
    use super::*;

    fn section(found: &[&str], missing: &[&str]) -> String {
        let mut output = "Latexmk: Found bibliography file(s):\n".to_string();
        for name in found {
            output.push_str(&format!("  {name}\n"));
        }
        if !missing.is_empty() {
            output.push_str("Bib file(s) not found in search path:\n");
            for name in missing {
                output.push_str(&format!("  {name}\n"));
            }
        }
        output.push_str("Latexmk: All targets are up-to-date\n");
        output
    }
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        let log = directory.path().join("out (local)");
        std::fs::create_dir(&log).unwrap();
        let log = log.join("main's.log");
        std::fs::write(
            &root,
            "% 😀 before\r\n\\bibliography{first, re ffs.bib,second}\r\n",
        )
        .unwrap();
        std::fs::write(&log, "Current TeX log.\n").unwrap();
        (directory, root, log)
    }
    fn inspect(log: &str, tail: &str) -> String {
        format!("Transcript written on {log}.\nLatexmk: Getting log file '{log}'\nLatexmk: Examining '{log}'\n{tail}")
    }

    #[test]
    fn final_driver_lists_create_bounded_warnings_and_support_exact_source_lookup() {
        let (_directory, root, log) = setup();
        let tail = section(
            &["first.bib", "reffs.bib", "second.bib"],
            &["reffs.bib", "reffs.bib"],
        );
        let with_blank_line = tail.replace("\nLatexmk: All targets", "\n\nLatexmk: All targets");
        assert!(!parse_latexmk_missing_databases(
            &root,
            &log,
            &inspect("out (local)/main's.log", &with_blank_line)
        )
        .is_empty());
        for printed in [
            "out (local)/main's.log".to_string(),
            log.display().to_string(),
        ] {
            let mut diagnostics =
                parse_latexmk_missing_databases(&root, &log, &inspect(&printed, &tail));
            let uri = Url::from_file_path(&root).unwrap();
            assert_eq!(diagnostics[&uri].len(), 1);
            let item = &diagnostics[&uri][0];
            assert_eq!(
                item.message,
                "Latexmk: Bibliography database not found: reffs.bib"
            );
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert!(unresolved_location(item));
            locate_missing_bibliography_databases(&BTreeSet::from([uri.clone()]), &mut diagnostics);
            let item = &diagnostics[&uri][0];
            let source = std::fs::read_to_string(&root).unwrap();
            let start = source.find("re ffs.bib").unwrap();
            assert_eq!(
                item.range,
                Text::new(source).range(start, start + "re ffs.bib".len())
            );
            assert_eq!(
                item.data,
                Some(serde_json::json!({"file":"reffs.bib","bibliographyCommand":"bibliography"}))
            );
        }
    }

    #[test]
    fn earlier_passes_user_messages_other_jobs_and_child_tool_output_do_not_supply_warnings() {
        let (_directory, root, log) = setup();
        let missing = section(&["reffs.bib"], &["reffs.bib"]);
        let recovered = section(&["./refs.bib"], &[]);
        let marker = inspect("out (local)/main's.log", "");
        for output in [
            missing.clone(),
            format!("{marker}Latexmk: Found bibliography file(s):\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib"),
            format!("{marker}Latexmk: Found bibliography file(s):\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib\n"),
            format!("{marker}Latexmk: Found bibliography file(s):\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib\nUnexpected tool output\n"),
            format!("{missing}{marker}Latexmk: All targets are up-to-date\n"),
            format!("{marker}{missing}{marker}{recovered}"),
            format!("{marker}{missing}{recovered}"),
            inspect("other.log",&missing),
            format!("{marker}Latexmk: Getting log file 'other.log'\n{missing}"),
            format!("{marker}Latexmk: applying rule 'bibtex main'...\n{missing}"),
            format!("{marker}Running 'unrelated compiler'\n{missing}"),
            format!("{marker}Latexmk: Bibliography file(s) from .bcf file:\n  reffs.bib\nBib file(s) not found in search path:\n  reffs.bib\n"),
            format!("{marker}Bib file(s) not found in search path:\n  reffs.bib\n"),
        ] {
            assert!(parse_latexmk_missing_databases(&root,&log,&output).is_empty(),"{output}");
        }
    }

    #[test]
    fn incomplete_invalid_or_unconfirmed_driver_names_do_not_create_partial_warnings() {
        let (_directory, root, log) = setup();
        for (found, missing) in [
            (vec!["reffs.bib"], vec!["other.bib"]),
            (
                vec!["reffs.bib", "wrong.tex"],
                vec!["reffs.bib", "wrong.tex"],
            ),
            (
                vec!["reffs.bib", "spaced name.bib"],
                vec!["reffs.bib", "spaced name.bib"],
            ),
            (
                vec!["reffs.bib", r"\chosen.bib"],
                vec!["reffs.bib", r"\chosen.bib"],
            ),
            (vec![".bib"], vec![".bib"]),
            (vec![], vec!["reffs.bib"]),
        ] {
            let output = inspect("out (local)/main's.log", &section(&found, &missing));
            assert!(
                parse_latexmk_missing_databases(&root, &log, &output).is_empty(),
                "{output}"
            );
        }
        let output = inspect(
            "out (local)/main's.log",
            &section(&["reffs.bib"], &["reffs.bib"]),
        );
        let truncated = format!("{output}{}\n", crate::output::TRUNCATED);
        assert!(parse_latexmk_missing_databases(&root, &log, &truncated).is_empty());
        std::fs::remove_file(&log).unwrap();
        assert!(parse_latexmk_missing_databases(&root, &log, &output).is_empty());
    }

    #[test]
    fn driver_database_lists_reject_excessive_counts_and_names() {
        let (_directory, root, log) = setup();
        for (count, limit) in [(65, 64), (4097, 0)] {
            let names: Vec<_> = (0..count).map(|n| format!("refs{n}.bib")).collect();
            let found: Vec<_> = names.iter().map(String::as_str).collect();
            let missing = if limit == 0 { &found[..1] } else { &found[..] };
            let output = inspect("out (local)/main's.log", &section(&found, missing));
            assert!(parse_latexmk_missing_databases(&root, &log, &output).is_empty());
        }
        let name = format!("{}.bib", "r".repeat(8193));
        let output = inspect("out (local)/main's.log", &section(&[&name], &[&name]));
        assert!(parse_latexmk_missing_databases(&root, &log, &output).is_empty());
    }
}
