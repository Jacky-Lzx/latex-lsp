//! Source comments document a unique user declaration, without markup execution.
use crate::{
    syntax,
    workspace::{Document, Workspace},
};
use std::collections::BTreeSet;
use tower_lsp::lsp_types::Url;

const MAX_LINES: usize = 32;
const MAX_BYTES: usize = 8192;

fn directive(text: &str) -> bool {
    text.split_whitespace().next().is_some_and(|word| {
        word.eq_ignore_ascii_case("!tex")
            || word.eq_ignore_ascii_case("!bib")
            || word.eq_ignore_ascii_case("arara:")
            || word.eq_ignore_ascii_case("vim:")
    })
}

pub(crate) fn for_command(ws: &Workspace, scope: &BTreeSet<Url>, name: &str) -> Option<String> {
    let mut definitions = scope
        .iter()
        .filter_map(|uri| ws.documents.get(uri))
        .flat_map(|doc| {
            doc.syntax
                .definitions
                .iter()
                .filter(move |entry| entry.name == name)
                .map(move |entry| (doc, entry))
        });
    let (doc, entry) = definitions.next()?;
    if definitions.next().is_some() {
        return None;
    }
    comments(doc, entry)
}

fn comments(doc: &Document, entry: &syntax::Entry) -> Option<String> {
    let source = &doc.text.source;
    let declaration = crate::signature::command_declaration_start(&doc.syntax, entry)?;
    let line_start = source[..declaration].rfind('\n').map_or(0, |at| at + 1);
    let mut prefix = &source[line_start..declaration];
    loop {
        prefix = prefix.trim_start_matches([' ', '\t']);
        if prefix.is_empty() {
            break;
        }
        let token = syntax::token(prefix, 0)?;
        if !matches!(token, "\\long" | "\\outer" | "\\global" | "\\protected") {
            return None;
        }
        prefix = &prefix[token.len()..];
    }
    let mut end = line_start;
    let mut lines = Vec::new();
    let mut bytes = 0;
    while end > 0 {
        let previous_end = end - 1;
        let start = source[..previous_end].rfind('\n').map_or(0, |at| at + 1);
        let line = source[start..previous_end]
            .strip_suffix('\r')
            .unwrap_or(&source[start..previous_end]);
        let contents = line.trim_start_matches([' ', '\t']);
        if !contents.starts_with('%') {
            break;
        }
        let at = start + line.len() - contents.len();
        if doc
            .syntax
            .comments
            .binary_search_by_key(&at, |span| span.start)
            .is_err()
        {
            break;
        }
        let contents = contents.trim_start_matches('%');
        let contents = contents.strip_prefix(' ').unwrap_or(contents);
        if directive(contents.trim()) {
            break;
        }
        bytes += line.len();
        if lines.len() >= MAX_LINES || bytes > MAX_BYTES {
            return None;
        }
        lines.push(contents);
        end = start;
    }
    lines.reverse();
    let start = lines.iter().position(|line| !line.trim().is_empty())?;
    let end = lines.iter().rposition(|line| !line.trim().is_empty())? + 1;
    Some(lines[start..end].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn notes(source: &str) -> Option<String> {
        let uri = Url::parse("untitled:docs.tex").unwrap();
        let doc = Document::new(&uri, source.into(), Some(1));
        comments(
            &doc,
            doc.syntax
                .definitions
                .iter()
                .find(|entry| entry.name == "example")
                .unwrap(),
        )
    }
    #[test]
    fn adjacent_real_comments_keep_paragraphs_unicode_and_literal_markup() {
        assert_eq!(notes("%% A 中文😀 command.\r\n%\r\n% **literal** <script>text</script> [link](https://example.test)\r\n\\newcommand{\\example}[1]{#1}"), Some("A 中文😀 command.\n\n**literal** <script>text</script> [link](https://example.test)".into()));
        assert_eq!(
            notes("%\n% Note\n%%\n\\def\\example{}"),
            Some("Note".into())
        );
    }
    #[test]
    fn blank_lines_code_inline_comments_and_magic_directives_end_the_block() {
        for source in [
            "% Old note\n\n\\newcommand{\\example}{}",
            "Code % not documentation\n\\newcommand{\\example}{}",
            "% Old note\nCode\n\\newcommand{\\example}{}",
            "% !TeX program = xelatex\n\\newcommand{\\example}{}",
            "% arara: pdflatex\n\\newcommand{\\example}{}",
            "% Note\nText \\newcommand{\\example}{}",
        ] {
            assert_eq!(notes(source), None, "{source}");
        }
        assert_eq!(
            notes("% !TeX root = main.tex\n% Actual documentation\n\\newcommand{\\example}{}"),
            Some("Actual documentation".into())
        );
    }
    #[test]
    fn declaration_comments_work_with_multiline_headers_modifiers_and_named_forms() {
        for declaration in [
            "\\newcommand\n  {\\example}[1]{#1}",
            "\\NewDocumentCommand{\\example}{m}{#1}",
            "\\long\\def\\example#1{#1}",
            "\\global \\protected \\def\\example{}",
            "\\csdef{example}{}",
            "\\let\\example\\relax",
        ] {
            assert_eq!(
                notes(&format!("% Documentation\n{declaration}")),
                Some("Documentation".into()),
                "{declaration}"
            );
        }
    }
    #[test]
    fn preceding_raw_material_and_over_budget_blocks_do_not_become_documentation() {
        assert_eq!(
            notes("\\begin{verbatim}\n% Raw note\n\\end{verbatim}\n\\newcommand{\\example}{}"),
            None
        );
        assert_eq!(
            notes(&format!(
                "{}\\newcommand{{\\example}}{{}}",
                "% Notes\n".repeat(MAX_LINES + 1)
            )),
            None
        );
        assert_eq!(
            notes(&format!(
                "% {}\n\\newcommand{{\\example}}{{}}",
                "文".repeat(MAX_BYTES)
            )),
            None
        );
    }
    #[test]
    fn project_resolution_requires_a_unique_user_definition() {
        let mut ws = Workspace::default();
        let a = Url::parse("untitled:a.tex").unwrap();
        let b = Url::parse("untitled:b.tex").unwrap();
        ws.open(a.clone(), "% First\n\\newcommand{\\example}{}".into(), 1);
        ws.open(b.clone(), "% Second\n\\newcommand{\\example}{}".into(), 1);
        assert_eq!(
            for_command(&ws, &[a.clone()].into(), "example"),
            Some("First".into())
        );
        assert_eq!(for_command(&ws, &[a, b].into(), "example"), None);
        assert_eq!(for_command(&ws, &BTreeSet::new(), "textbf"), None);
    }
}
