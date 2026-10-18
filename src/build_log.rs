//! Recover file/line diagnostics from TeX's file-line errors and traditional logs.
use crate::text::Text;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, Url};

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
            if [
                ".tex", ".sty", ".cls", ".clo", ".def", ".bib", ".bst", ".ltx",
            ]
            .iter()
            .any(|ext| path.trim_matches('"').ends_with(ext))
            {
                return Some((path, number, message.trim()));
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

fn missing_include_key(message: &str) -> Option<&str> {
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
    let mut ranges = Vec::new();
    for command in &syntax.commands {
        if !(if citation {
            crate::syntax::is_citation(&command.name)
        } else {
            crate::syntax::is_reference(&command.name)
        }) {
            continue;
        }
        let Some(argument) = &command.argument else {
            continue;
        };
        if text.position(command.span.start).line > line
            || text.position(argument.span.end).line < line
        {
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
    let Ok(uri) = Url::from_file_path(path) else {
        return;
    };
    // Font/glyph diagnostics often repeat at shipout. Avoid reading and mapping
    // the same source file once for every repetition of an identical warning.
    if result.get(&uri).is_some_and(|diagnostics| {
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message == message
                && diagnostic.severity == Some(severity)
                && diagnostic.range.start.line == line.saturating_sub(1)
        })
    }) {
        return;
    }
    let source = std::fs::read_to_string(path).unwrap_or_default();
    let text = Text::new(source);
    let last_line = text.text_line_count().saturating_sub(1) as u32;
    let line = line.saturating_sub(1).min(last_line);
    let value = text.source.lines().nth(line as usize).unwrap_or("");
    let start = value
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .map(char::len_utf16)
        .sum::<usize>() as u32;
    let end = value.encode_utf16().count() as u32;
    let mut ranges = undefined_reference_ranges(&text, line, &message);
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
        ranges.push(Range::new(
            Position::new(line, start.min(end)),
            Position::new(line, end),
        ));
    }
    result
        .entry(uri)
        .or_default()
        .extend(ranges.into_iter().map(|range| {
            Diagnostic {
                range,
                severity: Some(severity),
                source: Some("latex-build".into()),
                code: box_code(&message)
                    .or_else(|| missing_include_key(&message).map(|_| "missing-include"))
                    .map(|code| NumberOrString::String(code.into())),
                message: message.clone(),
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
}

fn message_boundary(line: &str) -> bool {
    let line = line.trim_start();
    warning_start(line)
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
        || line.starts_with('(')
            && [".tex", ".sty", ".cls", ".clo", ".def", ".ltx"]
                .iter()
                .any(|extension| line.contains(extension))
}

fn wrapped_message(lines: &[&str], i: &mut usize) -> String {
    let mut message = lines[*i].trim().to_owned();
    while !message.ends_with(['.', '!']) && *i + 1 < lines.len() && message.len() < 4096 {
        let next = lines[*i + 1];
        if next.trim().is_empty() || message_boundary(next) {
            break;
        }
        *i += 1;
        message.push(' ');
        message.push_str(next.trim());
    }
    message
}

fn stack_file(root: &Path, tail: &str) -> Option<(PathBuf, usize)> {
    if let Some(quoted) = tail.strip_prefix('"') {
        let end = quoted.find('"')?;
        let name = &quoted[..end];
        return crate::workspace::is_source(Path::new(name))
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
        for extension in crate::workspace::SOURCE_EXTENSIONS {
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

fn update_file_stack(line: &str, root: &Path, stack: &mut Vec<Option<PathBuf>>) {
    let mut i = 0;
    while i < line.len() {
        match line.as_bytes()[i] {
            b'(' => {
                if let Some((file, length)) = stack_file(root, &line[i + 1..]) {
                    stack.push(Some(file));
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
    // when the continuation completes an actual file, with bounded lookahead.
    if line.len() >= 60 {
        for (opening, _) in line.match_indices('(') {
            if stack_file(root, &line[opening + 1..]).is_some() {
                continue;
            }
            let mut joined = line.to_owned();
            for (next_index, next) in lines.iter().enumerate().skip(*i + 1).take(64) {
                if next.is_empty()
                    || warning_start(next)
                    || next.starts_with("! ")
                    || numbered_error(next).is_some()
                    || joined.len() + next.len() > 8192
                {
                    break;
                }
                joined.push_str(next);
                if stack_file(root, &joined[opening + 1..])
                    .is_some_and(|(_, length)| length > line.len() - opening - 1)
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
    let mut result = BTreeMap::new();
    let mut stack: Vec<Option<PathBuf>> = Vec::new();
    let mut pending: Option<(PathBuf, String)> = None;
    let lines = log.lines().collect::<Vec<_>>();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let current = || {
            stack
                .iter()
                .rev()
                .find_map(Option::as_ref)
                .cloned()
                .unwrap_or_else(|| root.into())
        };
        let mut structural = false;
        if let Some((name, number, message)) = numbered_error(line) {
            let path = location(root, name).unwrap_or_else(|| root.into());
            let severity = if message.contains("Warning:") {
                DiagnosticSeverity::WARNING
            } else {
                DiagnosticSeverity::ERROR
            };
            emit(&mut result, &path, number, message.into(), severity);
            pending = None;
        } else if let Some(message) = line.strip_prefix("! ") {
            if let Some((path, message)) = pending.take() {
                emit(&mut result, &path, 1, message, DiagnosticSeverity::ERROR);
            }
            pending = Some((current(), message.into()));
        } else if let Some(tail) = line.trim_start().strip_prefix("l.") {
            let number = tail
                .split(|ch: char| !ch.is_ascii_digit())
                .next()
                .and_then(|value| value.parse().ok());
            if let (Some(number), Some((path, message))) = (number, pending.take()) {
                emit(
                    &mut result,
                    &path,
                    number,
                    message,
                    DiagnosticSeverity::ERROR,
                );
            }
        } else if warning_start(line) || line.starts_with("Missing character:") {
            let path = current();
            // TeX wraps warnings at its configured print width.
            let message = wrapped_message(&lines, &mut i);
            let number = input_line(&message).unwrap_or(1);
            emit(
                &mut result,
                &path,
                number,
                message,
                DiagnosticSeverity::WARNING,
            );
        } else if line.starts_with("No file ") {
            let path = current();
            if let Some(name) = missing_include_file(&path, &lines, &mut i) {
                emit(
                    &mut result,
                    &path,
                    1,
                    format!("Included file `{name}` not found"),
                    DiagnosticSeverity::WARNING,
                );
            }
        } else if line.starts_with("Overfull ") || line.starts_with("Underfull ") {
            emit(
                &mut result,
                &current(),
                input_line(line).unwrap_or(1),
                line.into(),
                DiagnosticSeverity::WARNING,
            );
        } else {
            structural = true;
        }
        // User messages and echoed TeX source can contain unmatched parentheses.
        // They describe diagnostics, rather than file entry/exit events.
        if structural {
            let line = continued_file_line(root, &lines, &mut i);
            update_file_stack(&line, root, &mut stack);
        }
        i += 1;
    }
    if let Some((path, message)) = pending {
        emit(&mut result, &path, 1, message, DiagnosticSeverity::ERROR);
    }
    for diagnostics in result.values_mut() {
        diagnostics.sort_by_key(|diagnostic| (diagnostic.range.start, diagnostic.message.clone()));
        diagnostics.dedup_by(|a, b| a.range == b.range && a.message == b.message);
    }
    result
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
        if let Some((message, tail)) = line.split_once("---line ") {
            if let Some((number, name)) = tail.split_once(" of file ") {
                if let Ok(number) = number.trim().parse::<u32>() {
                    let location = paths.resolve(name).filter(|path| {
                        matches!(
                            path.extension().and_then(|ext| ext.to_str()),
                            Some("tex" | "bib" | "bst" | "sty" | "cls")
                        )
                    });
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
        let explicit_path = bibliography_path(paths, message);
        let number = bibliography_line(message);
        let key = message
            .rsplit_once(" in ")
            .map(|(_, key)| key.trim())
            .or_else(|| {
                message
                    .split_once("entry '")
                    .and_then(|(_, tail)| tail.split_once('\'').map(|(key, _)| key))
            });
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
        emit(
            &mut result,
            path,
            number.unwrap_or(1),
            diagnostic_message,
            severity,
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

fn bibliography_line(message: &str) -> Option<u32> {
    for (index, _) in message.match_indices("line ") {
        if let Some(number) = message[index + 5..]
            .split(|ch: char| !ch.is_ascii_digit())
            .next()
            .and_then(|value| value.parse().ok())
        {
            return Some(number);
        }
    }
    None
}

fn bibliography_path(
    paths: &crate::recorder::BibliographyPaths<'_>,
    message: &str,
) -> Option<PathBuf> {
    for (index, _) in message.match_indices(".bib") {
        let end = index + 4;
        let prefix = &message[..index];
        let token_start = prefix
            .rfind(|ch: char| ch.is_whitespace() || ch == ':')
            .map_or(0, |index| index + 1);
        let starts = prefix
            .rmatch_indices(['\'', '"'])
            .take(16)
            .map(|(index, _)| index + 1)
            .chain(std::iter::once(token_start));
        if let Some(path) = starts
            .into_iter()
            .find_map(|start| paths.resolve(&message[start..end]))
        {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let source = "😀 \\ref{missing} \\cref{found, missing} \\cite{lost}\n% \\ref{missing}\n\\verb|\\ref{missing}|\n\\ref{\nmissing}\n\\ref{O'Connor}\n";
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
