//! Bounded local indentation; never launches a formatter or changes TeX tokens.
use crate::{syntax, workspace::Document};
use tower_lsp::{jsonrpc::Result, lsp_types::*};

const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_INDENT: usize = 4096;

#[derive(Default)]
struct Structure {
    delta: i32,
    leading_closes: i32,
    prefix_end: usize,
}

fn opaque(doc: &Document) -> impl Iterator<Item = &std::ops::Range<usize>> {
    doc.syntax
        .ignored
        .iter()
        .chain(&doc.syntax.literal_conversions)
        .chain(&doc.syntax.literal_tests)
        .chain(&doc.syntax.ambiguous_string_tests)
}

fn structure(
    doc: &Document,
    start: usize,
    end: usize,
    deferred: &[std::ops::Range<usize>],
) -> Option<Structure> {
    if end - start > MAX_LINE_BYTES {
        return None;
    }
    let source = &doc.text.source;
    let mut ignored: Vec<_> = opaque(doc)
        .filter(|span| span.start < end && span.end > start)
        .collect();
    ignored.sort_unstable_by_key(|span| span.start);
    let mut opaque = 0;
    let mut at = start;
    let mut leading = true;
    let mut result = Structure::default();
    while at < end {
        while ignored.get(opaque).is_some_and(|span| span.end <= at) {
            opaque += 1;
        }
        if let Some(span) = ignored.get(opaque).filter(|span| span.contains(&at)) {
            at = span.end.min(end);
            leading = false;
            continue;
        }
        let token = syntax::token(source, at)?;
        let mut next = at + token.len();
        let mut delta = 0;
        if !doc.syntax.control_operands.contains(&at)
            && !doc.syntax.punctuation_operands.contains(&at)
        {
            delta = match token {
                "\\noexpand" => {
                    let operand = syntax::skip_space(source, next);
                    if source.as_bytes().get(operand) == Some(&b'\\') {
                        next = operand + syntax::token(source, operand)?.len();
                        if next > end {
                            return None;
                        }
                    }
                    0
                }
                "{" => 1,
                "}" => -1,
                "\\begingroup" | "\\bgroup" => {
                    i32::from(!deferred.iter().any(|span| span.contains(&at)))
                }
                "\\endgroup" | "\\egroup" => {
                    -i32::from(!deferred.iter().any(|span| span.contains(&at)))
                }
                "\\begin" | "\\end" => {
                    let command = doc
                        .syntax
                        .commands
                        .binary_search_by_key(&at, |command| command.span.start)
                        .ok()
                        .map(|index| &doc.syntax.commands[index]);
                    if let Some(argument) = command.and_then(|command| command.argument.as_ref()) {
                        if argument.outer_span.end > end {
                            return None;
                        }
                        if source.as_bytes().get(argument.outer_span.start) != Some(&b'{')
                            || !syntax::is_static_key(&argument.value)
                        {
                            return None;
                        }
                        next = argument.outer_span.end;
                        if argument.value == "document"
                            || deferred.iter().any(|span| span.contains(&at))
                        {
                            0
                        } else if token == "\\begin" {
                            1
                        } else {
                            -1
                        }
                    } else {
                        0
                    }
                }
                _ => 0,
            };
        }
        result.delta += delta;
        if leading && delta == -1 {
            result.leading_closes += 1;
            result.prefix_end = next;
        } else if !matches!(token, " " | "\t") {
            leading = false;
        }
        at = next;
    }
    Some(result)
}

fn line(doc: &Document, row: u32) -> Option<(usize, usize, usize)> {
    let start = doc.text.offset(Position::new(row, 0))?;
    let contents = doc.text.source[start..].split('\n').next()?;
    let contents = contents.strip_suffix('\r').unwrap_or(contents);
    let indent = contents
        .bytes()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    Some((start, start + contents.len(), start + indent))
}

pub(crate) fn edits(
    doc: &Document,
    position: Position,
    ch: &str,
    options: &FormattingOptions,
) -> Result<Vec<TextEdit>> {
    crate::formatting::validate_options(options)?;
    let cursor = doc.text.clamped_offset(position).ok_or_else(|| {
        tower_lsp::jsonrpc::Error::invalid_params("Invalid on-type formatting position")
    })?;
    if !matches!(ch, "\n" | "}") || position.line == 0 {
        return Ok(vec![]);
    }
    let (start, end, indent_end) = line(doc, position.line).unwrap();
    if end - start > MAX_LINE_BYTES
        || opaque(doc).any(|span| {
            span.contains(&cursor)
                || (cursor == doc.text.source.len() && span.end == cursor && span.start < cursor)
                || span.contains(&start)
        })
    {
        return Ok(vec![]);
    }
    if ch == "\n" && cursor > indent_end {
        return Ok(vec![]);
    }
    if ch == "}" && !doc.text.source[..cursor].ends_with('}') {
        return Ok(vec![]);
    }
    if doc.text.source.len() > 2 * 1024 * 1024
        || doc.syntax.definitions.len() + doc.syntax.environments.len() > 1024
    {
        return Ok(vec![]);
    }
    let deferred = crate::signature::definition_body_spans(&doc.text.source, &doc.syntax);
    let Some(current) = structure(doc, start, end, &deferred) else {
        return Ok(vec![]);
    };
    if ch == "}" && (current.leading_closes == 0 || current.prefix_end != cursor) {
        return Ok(vec![]);
    }
    let (previous_start, previous_end, previous_indent) = line(doc, position.line - 1).unwrap();
    // Raw block text is not a useful indentation anchor for the next line.
    if doc.syntax.ignored.iter().any(|span| {
        span.contains(&previous_start) && !doc.syntax.comments.iter().any(|comment| comment == span)
    }) {
        return Ok(vec![]);
    }
    let Some(previous) = structure(doc, previous_start, previous_end, &deferred) else {
        return Ok(vec![]);
    };
    let tab = options.tab_size as usize;
    let mut columns = 0usize;
    for byte in doc.text.source[previous_start..previous_indent].bytes() {
        columns += if byte == b'\t' {
            tab - columns % tab
        } else {
            1
        };
        if columns > MAX_INDENT {
            return Ok(vec![]);
        }
    }
    // A leading closer has already reduced its own line's indentation.
    let adjustment = previous.delta + previous.leading_closes - current.leading_closes;
    let target = (columns as i64 + i64::from(adjustment) * tab as i64).max(0) as usize;
    if target > MAX_INDENT {
        return Ok(vec![]);
    }
    let replacement = if options.insert_spaces {
        " ".repeat(target)
    } else {
        format!("{}{}", "\t".repeat(target / tab), " ".repeat(target % tab))
    };
    if doc.text.source[start..indent_end] == replacement {
        return Ok(vec![]);
    }
    Ok(vec![TextEdit {
        range: doc.text.range(start, indent_end),
        new_text: replacement,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(tab_size: u32, insert_spaces: bool) -> FormattingOptions {
        FormattingOptions {
            tab_size,
            insert_spaces,
            ..Default::default()
        }
    }

    fn check(source: &str, ch: &str, tab_size: u32, spaces: bool, expected: &str) {
        let cursor = source.find('¦').unwrap();
        let source = source.replacen('¦', "", 1);
        let uri = Url::parse("file:///tmp/on-type.tex").unwrap();
        let doc = Document::new(&uri, source.clone(), Some(1));
        let options = options(tab_size, spaces);
        let edits = edits(&doc, doc.text.position(cursor), ch, &options).unwrap();
        let mut actual = source;
        for edit in edits.iter().rev() {
            assert_eq!(edit.range.start.line, edit.range.end.line);
            assert_eq!(edit.range.start.character, 0);
            assert!(edit
                .new_text
                .bytes()
                .all(|byte| matches!(byte, b' ' | b'\t')));
            let start = doc.text.offset(edit.range.start).unwrap();
            let end = doc.text.offset(edit.range.end).unwrap();
            assert!(actual[start..end]
                .bytes()
                .all(|byte| matches!(byte, b' ' | b'\t')));
            actual.replace_range(start..end, &edit.new_text);
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn newlines_indent_environments_groups_and_blank_lines() {
        for (input, expected) in [
            ("\\begin{itemize}\n¦", "\\begin{itemize}\n  "),
            ("  \\begin{enumerate}\n¦", "  \\begin{enumerate}\n    "),
            ("\\textbf{\n¦", "\\textbf{\n  "),
            ("  \\begingroup\n¦", "  \\begingroup\n    "),
            ("\\begin{document}\n¦", "\\begin{document}\n"),
            ("    \n¦", "    \n    "),
            ("  \\begin{a}\\end{a}\n¦", "  \\begin{a}\\end{a}\n  "),
        ] {
            check(input, "\n", 2, true, expected);
        }
    }

    #[test]
    fn leading_closers_dedent_once_and_never_change_content() {
        for (input, expected) in [
            ("  text\n  \\end{itemize}¦", "  text\n\\end{itemize}"),
            ("  text\n  }¦", "  text\n}"),
            ("    text\n    }}¦ rest", "    text\n}} rest"),
            (
                "\\begin{itemize}\n¦\\end{itemize}",
                "\\begin{itemize}\n\\end{itemize}",
            ),
            ("  \\end{inner}\n¦", "  \\end{inner}\n  "),
            ("  }\n¦", "  }\n  "),
            ("    text}\n¦", "    text}\n  "),
        ] {
            let ch = if input.contains('¦') && input[..input.find('¦').unwrap()].ends_with('}') {
                "}"
            } else {
                "\n"
            };
            check(input, ch, 2, true, expected);
        }
    }

    #[test]
    fn comments_escaped_braces_and_token_operands_do_not_open_groups() {
        for input in [
            "  text % { \\begin{itemize}\n¦",
            "  \\{ escaped\n¦",
            "  \\string{\n¦",
            "  \\noexpand\\begingroup\n¦",
            "  \\verb|{ \\begin{itemize}|\n¦",
        ] {
            let expected = input.replace('¦', "  ");
            check(input, "\n", 2, true, &expected);
        }
        check(
            "\\begin{itemize} % comment\n¦",
            "\n",
            2,
            true,
            "\\begin{itemize} % comment\n  ",
        );
    }

    #[test]
    fn verbatim_comments_and_midline_triggers_remain_untouched() {
        for input in [
            "\\begin{verbatim}\nraw {\n¦",
            "\\begin{lstlisting}\n  }¦\n\\end{lstlisting}",
            "\\begin{minted}{tex}\n  }¦\n\\end{minted}",
            "  text\n  % }¦",
            "  text\n  \\section{title}¦",
            "  text\n  \\}¦",
            "  text\n  text¦",
        ] {
            let ch = if input[..input.find('¦').unwrap()].ends_with('}') {
                "}"
            } else {
                "\n"
            };
            check(input, ch, 2, true, &input.replace('¦', ""));
        }
    }

    #[test]
    fn utf16_crlf_tabs_and_existing_indentation_are_preserved() {
        check(
            "\t\\begin{itemize} % 😀\r\n ¦",
            "\n",
            4,
            false,
            "\t\\begin{itemize} % 😀\r\n\t\t",
        );
        check(" \ttext\r\n        }¦ 😀", "}", 4, false, " \ttext\r\n} 😀");
        check("  text😀\r\n¦", "\n", 2, true, "  text😀\r\n  ");
        check(
            "\\begin{itemize}\n  ¦",
            "\n",
            2,
            true,
            "\\begin{itemize}\n  ",
        );
    }

    #[test]
    fn invalid_positions_and_options_are_rejected_and_oversized_columns_clamp() {
        let uri = Url::parse("file:///tmp/on-type.tex").unwrap();
        let doc = Document::new(&uri, "  text\n😀".into(), Some(1));
        let valid_options = options(2, true);
        for position in [Position::new(1, 1), Position::new(2, 0)] {
            assert!(edits(&doc, position, "\n", &valid_options).is_err());
        }
        assert!(edits(&doc, Position::new(1, 999), "\n", &valid_options)
            .unwrap()
            .is_empty());
        assert!(edits(&doc, Position::new(1, 0), "\n", &options(0, true)).is_err());
    }

    #[test]
    fn pathological_lines_and_indentation_do_not_produce_large_edits() {
        let input = format!("{}{{\n¦", "x".repeat(MAX_LINE_BYTES));
        check(&input, "\n", 2, true, &input.replace('¦', ""));
        let input = format!("{}{{\n¦", " ".repeat(MAX_INDENT));
        check(&input, "\n", 2, true, &input.replace('¦', ""));
    }

    #[test]
    fn deferred_environment_and_group_commands_do_not_leak_past_macro_definitions() {
        for input in [
            "  \\newcommand{\\openlist}{\\begin{itemize}}\n¦",
            "  \\def\\openlist{\\begingroup}\n¦",
            "  \\newenvironment{example}{\\begin{itemize}}{\\end{itemize}}\n¦",
        ] {
            check(input, "\n", 2, true, &input.replace('¦', "  "));
        }
        check(
            "\\newcommand{\\example}{\n¦text\n}",
            "\n",
            2,
            true,
            "\\newcommand{\\example}{\n  text\n}",
        );
    }
}
