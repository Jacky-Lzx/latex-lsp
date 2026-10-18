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
            if [".tex", ".sty", ".cls", ".clo", ".def", ".bib", ".ltx"]
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

/// BibTeX and Biber write a separate log, often after TeX has already produced
/// a PDF. Preserve those errors instead of hiding them behind latexmk's exit code.
pub fn parse_bibliography(root: &Path, log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    let mut result = BTreeMap::new();
    let databases: Vec<_> = log
        .lines()
        .filter_map(|line| {
            let name = if line.starts_with("Database file #") {
                line.split_once(": ")?.1.trim()
            } else {
                let tail = line.split_once("Found BibTeX data source '")?.1;
                tail.split_once('\'')?.0
            };
            location(root, name)
        })
        .collect();
    let mut previous = "";
    for line in log.lines() {
        if let Some((message, tail)) = line.split_once("---line ") {
            if let Some((number, name)) = tail.split_once(" of file ") {
                if let Ok(number) = number.trim().parse::<u32>() {
                    let location = location(root, name).filter(|path| {
                        matches!(
                            path.extension().and_then(|ext| ext.to_str()),
                            Some("tex" | "bib" | "sty" | "cls")
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
        let explicit_path = bibliography_path(root, message);
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

fn bibliography_path(root: &Path, message: &str) -> Option<PathBuf> {
    for (index, _) in message.match_indices(".bib") {
        let end = index + 4;
        let prefix = &message[..index];
        let start = prefix
            .rfind(['\'', '"'])
            .map(|index| index + 1)
            .unwrap_or_else(|| {
                prefix
                    .rfind(|ch: char| ch.is_whitespace() || ch == ':')
                    .map_or(0, |index| index + 1)
            });
        if let Some(path) = location(root, &message[start..end]) {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
