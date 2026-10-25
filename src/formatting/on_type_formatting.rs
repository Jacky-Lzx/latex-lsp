//! Bounded local indentation; never launches a formatter or changes TeX tokens.
use crate::{
    language::signature::CommandSignatures,
    parser::syntax,
    project::workspace::{Document, DocumentKind, Workspace},
};
use std::collections::BTreeSet;
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

struct Context<'a> {
    signatures: CommandSignatures<'a>,
    operands: BTreeSet<usize>,
    declarations: BTreeSet<(usize, usize)>,
    deferred: Vec<std::ops::Range<usize>>,
}

fn structure(doc: &Document, start: usize, end: usize, context: &Context<'_>) -> Option<Structure> {
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
                "{" => 1,
                "}" => -1,
                _ => 0,
            };
            if token.starts_with('\\') && !context.operands.contains(&at) {
                let index = doc
                    .syntax
                    .commands
                    .binary_search_by_key(&at, |cmd| cmd.span.start)
                    .ok();
                let invocation = index.and_then(|index| {
                    context.signatures.named_invocation(
                        source,
                        &doc.syntax,
                        index,
                        &context.operands,
                        &context.declarations,
                    )
                });
                let name = if let Some((name, span)) = invocation {
                    next = span.end;
                    name
                } else {
                    token.strip_prefix('\\').unwrap()
                };
                if next > end {
                    return None;
                }
                let active = !context.deferred.iter().any(|span| span.contains(&at));
                let invokes = |builtin| context.signatures.invokes_builtin(name, builtin);
                if invokes("begingroup") || invokes("bgroup") {
                    delta = i32::from(active);
                } else if invokes("endgroup") || invokes("egroup") {
                    delta = -i32::from(active);
                } else if invokes("begin") || invokes("end") {
                    if let Some(argument) = syntax::environment_name_argument(source, next) {
                        if argument.outer_span.end > end {
                            return None;
                        }
                        next = argument.outer_span.end;
                        if active && argument.value != "document" {
                            delta = if invokes("begin") { 1 } else { -1 };
                        }
                    } else if source.as_bytes().get(syntax::skip_space(source, next)) == Some(&b'{')
                    {
                        // An incomplete or dynamic environment name has no safe boundary.
                        return None;
                    }
                }
            }
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
    workspace: &Workspace,
    uri: &Url,
    position: Position,
    ch: &str,
    options: &FormattingOptions,
) -> Result<Vec<TextEdit>> {
    crate::formatting::validate_options(options)?;
    let Some(doc) = workspace.documents.get(uri) else {
        return Ok(vec![]);
    };
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
    let scope = workspace.symbol_scope(uri);
    let documents: Vec<_> = workspace
        .documents
        .iter()
        .filter(|(uri, doc)| scope.contains(*uri) && doc.kind == DocumentKind::Latex)
        .map(|(_, doc)| doc)
        .collect();
    if !workspace.index_truncated.is_empty()
        || documents
            .iter()
            .map(|doc| doc.text.source.len())
            .sum::<usize>()
            > 2 * 1024 * 1024
        || documents
            .iter()
            .map(|doc| doc.syntax.definitions.len() + doc.syntax.environments.len())
            .sum::<usize>()
            > 1024
        || documents
            .iter()
            .map(|doc| doc.syntax.commands.len())
            .sum::<usize>()
            > 32768
    {
        return Ok(vec![]);
    }
    let signatures = CommandSignatures::new(
        documents
            .iter()
            .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
    );
    let context = Context {
        operands: signatures.source_argument_tokens(&doc.text.source, &doc.syntax),
        declarations: doc
            .syntax
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect(),
        deferred: crate::language::signature::definition_body_spans(&doc.text.source, &doc.syntax),
        signatures,
    };
    let Some(current) = structure(doc, start, end, &context) else {
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
    let Some(previous) = structure(doc, previous_start, previous_end, &context) else {
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
        let mut workspace = Workspace::default();
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(1)));
        let doc = &workspace.documents[&uri];
        let options = options(tab_size, spaces);
        let edits = edits(&workspace, &uri, doc.text.position(cursor), ch, &options).unwrap();
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
        let mut workspace = Workspace::default();
        workspace.documents.insert(
            uri.clone(),
            Document::new(&uri, "  text\n😀".into(), Some(1)),
        );
        let valid_options = options(2, true);
        for position in [Position::new(1, 1), Position::new(2, 0)] {
            assert!(edits(&workspace, &uri, position, "\n", &valid_options).is_err());
        }
        assert!(edits(
            &workspace,
            &uri,
            Position::new(1, 999),
            "\n",
            &valid_options
        )
        .unwrap()
        .is_empty());
        assert!(edits(
            &workspace,
            &uri,
            Position::new(1, 0),
            "\n",
            &options(0, true)
        )
        .is_err());
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

    #[test]
    fn user_macro_arguments_do_not_create_environment_or_group_indentation() {
        for (declaration, call) in [
            ("", r"\AddToHook{unused}[mine]{\begin{itemize}}"),
            ("", r"\AddToHookNext{unused}{\begin{itemize}}"),
            ("", r"\AtEndDocument{\begingroup}"),
            (
                r"\def\forward{\AddToHook}",
                r"\forward{unused}{\begin{itemize}}",
            ),
            ("\\newcommand{\\take}[1]{}", "\\take{\\begin{itemize}}"),
            ("\\newcommand{\\take}[1]{}", "\\take\\begin{itemize}"),
            ("\\def\\take#1STOP{}", "\\take\\begin{itemize}STOP"),
            (
                "\\NewDocumentCommand{\\take}{O{} m}{}",
                "\\take[\\begingroup]{\\begin{itemize}}",
            ),
            (
                "\\newcommand{\\take}[1]{}",
                "\\csuse{take}{\\begin{itemize}}",
            ),
            (
                "\\newcommand{\\take}[1]{}",
                "\\csname take\\endcsname{\\begingroup}",
            ),
        ] {
            let input = format!("{declaration}\n  {call}\n¦");
            check(&input, "\n", 2, true, &input.replace('¦', "  "));
        }
        // Raw braces still guide editing inside an unfinished argument.
        check(
            "\\newcommand{\\take}[1]{}\n\\take{\n¦\\begin{itemize}\n}",
            "\n",
            2,
            true,
            "\\newcommand{\\take}[1]{}\n\\take{\n  \\begin{itemize}\n}",
        );
    }

    #[test]
    fn noexpand_indent_uses_single_token_spaces_aliases_and_redefinitions() {
        for (declaration, call, indentation) in [
            ("", "\\noexpand\\begin{itemize}", "  "),
            ("", "\\csuse{noexpand}\\begin{itemize}", "  "),
            ("", "\\csuse{noexpand} \\begin{itemize}", "    "),
            ("", "\\csname noexpand\\endcsname \\begin{itemize}", "  "),
            ("", "\\noexpand{\\begin{itemize}}", "    "),
            ("\\def\\hold{\\noexpand}", "\\hold\\begin{itemize}", "  "),
            ("\\def\\noexpand{}", "\\noexpand\\begin{itemize}", "    "),
            ("\\def\\noexpand#1{}", "\\noexpand\\begin{itemize}", "  "),
        ] {
            let input = format!("{declaration}\n  {call}\n¦");
            check(&input, "\n", 2, true, &input.replace('¦', indentation));
        }
    }

    #[test]
    fn named_environment_and_group_calls_share_alias_and_redefinition_rules() {
        for (declaration, call, indentation) in [
            ("", "\\csuse{begin}{itemize}", "    "),
            ("", "\\csname begin\\endcsname{itemize}", "    "),
            ("", "\\csuse{begingroup}", "    "),
            ("\\def\\start{\\begin}", "\\start{itemize}", "    "),
            ("\\def\\start{\\begingroup}", "\\start", "    "),
            ("\\def\\begin#1{}", "\\begin{itemize}", "  "),
            ("\\def\\begingroup{}", "\\begingroup", "  "),
        ] {
            let input = format!("{declaration}\n  {call}\n¦");
            check(&input, "\n", 2, true, &input.replace('¦', indentation));
        }
        check(
            "  text\n  \\csuse{end}{itemize}¦",
            "}",
            2,
            true,
            "  text\n\\csuse{end}{itemize}",
        );
        check(
            "  text\n  \\csname end\\endcsname{itemize}¦",
            "}",
            2,
            true,
            "  text\n\\csname end\\endcsname{itemize}",
        );
        check(
            "\\newcommand{\\take}[1]{}\n  text\n  \\take\\end{itemize}¦",
            "}",
            2,
            true,
            "\\newcommand{\\take}[1]{}\n  text\n  \\take\\end{itemize}",
        );
    }

    #[test]
    fn project_parameter_updates_and_analysis_budgets_control_indentation() {
        let dir = tempfile::tempdir().unwrap();
        let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
        let macros = Url::from_file_path(dir.path().join("macros.tex")).unwrap();
        let source = "\\documentclass{article}\n\\input{macros}\n  \\take\\begin{itemize}\n";
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        std::fs::write(macros.to_file_path().unwrap(), "\\newcommand{\\take}[1]{}").unwrap();
        let mut workspace = Workspace::default();
        workspace.config.distribution.enabled = false;
        workspace.open(uri.clone(), source.into(), 1);
        let cursor = workspace.documents[&uri].text.position(source.len());
        let options = options(2, true);
        let indentation = |workspace: &Workspace| {
            let edits = edits(workspace, &uri, cursor, "\n", &options).unwrap();
            edits
                .into_iter()
                .map(|edit| edit.new_text)
                .collect::<Vec<_>>()
        };
        assert_eq!(indentation(&workspace), ["  "]);
        workspace.open(macros.clone(), "\\newcommand{\\take}{}".into(), 2);
        assert_eq!(indentation(&workspace), ["    "]);
        workspace.open(macros.clone(), "\\newcommand{\\take}[1]{}".into(), 3);
        assert_eq!(indentation(&workspace), ["  "]);
        workspace.index_truncated.insert("maxFiles".into());
        assert!(indentation(&workspace).is_empty());
        workspace.index_truncated.clear();
        workspace.open(macros, "\\def\\take{}\n".repeat(1025), 4);
        assert!(indentation(&workspace).is_empty());
    }
}
