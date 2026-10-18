//! Recover file/line diagnostics from TeX's file-line errors and traditional logs.
use crate::text::Text;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range, Url};

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
            if [".tex", ".sty", ".cls", ".bib", ".ltx"]
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
    result.entry(uri).or_default().push(Diagnostic {
        range: Range::new(
            Position::new(line, start.min(end)),
            Position::new(line, end),
        ),
        severity: Some(severity),
        source: Some("latex-build".into()),
        message,
        ..Default::default()
    });
}

fn update_file_stack(line: &str, root: &Path, stack: &mut Vec<Option<PathBuf>>) {
    for (i, ch) in line.char_indices() {
        match ch {
            '(' => {
                let tail = &line[i + 1..];
                let candidate = if let Some(quoted) = tail.strip_prefix('"') {
                    quoted.find('"').map(|end| &quoted[..end])
                } else {
                    let end = tail
                        .find(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')'))
                        .unwrap_or(tail.len());
                    Some(&tail[..end])
                };
                let file = candidate
                    .filter(|name| {
                        [".tex", ".sty", ".cls", ".ltx"]
                            .iter()
                            .any(|ext| name.ends_with(ext))
                    })
                    .and_then(|name| location(root, name));
                stack.push(file);
            }
            ')' => {
                stack.pop();
            }
            _ => {}
        }
    }
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
        } else if line.starts_with("LaTeX Warning:")
            || line.starts_with("Package ") && line.contains(" Warning:")
        {
            let mut message = line.trim().to_owned();
            let path = current();
            // TeX wraps warnings at its configured print width.
            while !message.ends_with('.')
                && i + 1 < lines.len()
                && !lines[i + 1].trim().is_empty()
                && message.len() < 4096
            {
                i += 1;
                message.push(' ');
                message.push_str(lines[i].trim());
            }
            let number = input_line(&message).unwrap_or(1);
            emit(
                &mut result,
                &path,
                number,
                message,
                DiagnosticSeverity::WARNING,
            );
        } else if line.starts_with("Overfull ") || line.starts_with("Underfull ") {
            emit(
                &mut result,
                &current(),
                input_line(line).unwrap_or(1),
                line.into(),
                DiagnosticSeverity::WARNING,
            );
        }
        update_file_stack(line, root, &mut stack);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
