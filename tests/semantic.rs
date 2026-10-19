use latex_lsp::{semantic, workspace::Document};
use tower_lsp::lsp_types::*;

fn document(name: &str, source: &str) -> Document {
    Document::new(
        &Url::parse(&format!("file:///semantic/{name}")).unwrap(),
        source.into(),
        Some(1),
    )
}

fn decoded(doc: &Document, range: Option<Range>) -> Vec<(Range, String, String, u32)> {
    let legend = semantic::options().legend;
    let mut at = Position::new(0, 0);
    let mut previous_end = at;
    semantic::tokens(doc, range)
        .unwrap()
        .data
        .into_iter()
        .map(|token| {
            at = Position::new(
                at.line + token.delta_line,
                if token.delta_line == 0 {
                    at.character + token.delta_start
                } else {
                    token.delta_start
                },
            );
            assert!(at >= previous_end, "overlapping or unsorted token");
            assert!(token.length > 0);
            let end = Position::new(at.line, at.character + token.length);
            let value = doc.text.source
                [doc.text.offset(at).unwrap()..doc.text.offset(end).unwrap()]
                .to_owned();
            assert!(!value.contains(['\r', '\n']));
            previous_end = end;
            (
                Range::new(at, end),
                value,
                legend.token_types[token.token_type as usize]
                    .as_str()
                    .into(),
                token.token_modifiers_bitset,
            )
        })
        .collect()
}

#[test]
fn latex_names_and_declarations_have_distinct_semantics_without_coloring_prose() {
    let source = "\\newcommand{\\greet}[1]{Hello #1}\\newlength{\\width}\\width\n\\newenvironment{panel}{}{}\\begin{panel}\\greet{World}\\end{panel}\n\\newcounter{steps}\\setcounter{steps}{1}\\newbool{draft}\\ifbool{draft}{Yes}{No}\n\\label{sec:中}\\ref{sec:中}\\cref{sec:中, sec:two}\\citep[see]{alpha, beta}\\ref{ spaced }\\ref{\\dynamic}\n";
    let doc = document("main.tex", source);
    let tokens = decoded(&doc, None);
    for (value, kind, modifier) in [
        ("\\greet", "macro", 1),
        ("\\greet", "macro", 0),
        ("\\width", "variable", 1),
        ("\\width", "variable", 0),
        ("panel", "type", 1),
        ("panel", "type", 0),
        ("steps", "variable", 1),
        ("steps", "variable", 0),
        ("draft", "variable", 1),
        ("draft", "variable", 0),
        ("sec:中", "enumMember", 1),
        ("sec:中", "enumMember", 0),
        ("sec:two", "enumMember", 0),
        ("alpha", "enumMember", 0),
        ("beta", "enumMember", 0),
        ("\\dynamic", "macro", 0),
    ] {
        assert!(
            tokens
                .iter()
                .any(|(_, text, ty, bits)| text == value && ty == kind && *bits == modifier),
            "missing {value} / {kind} / {modifier}: {tokens:?}"
        );
    }
    assert!(!tokens
        .iter()
        .any(|(_, value, _, _)| value.contains("World") || value.contains("spaced")));
}

#[test]
fn utf16_comments_urls_and_verbatim_regions_do_not_leak_commands_or_include_line_endings() {
    let doc = document(
        "main.tex",
        concat!(
            "😀 \\section{中} % 注释 😀 \\fake{label}\r\n",
            "\\href{%20/\\fake}{\\textbf{Visible}}\r\n",
            "\\verb%\\fake{raw}% \\lstinline[language=TeX]|\\fake{raw}|\r\n",
            "\\begin{verbatim}\r\n😀 \\fake%raw\r\n\\end{verbatim}\r\n",
            "\\begin{comment}\r\n\\fake{comment}\r\n\\end{comment}\r\n",
            "\\\\verb% actual comment\r\n",
        ),
    );
    let tokens = decoded(&doc, None);
    assert!(tokens
        .iter()
        .any(|(range, value, ty, _)| range.start == Position::new(0, 3)
            && value == "\\section"
            && ty == "macro"));
    assert!(tokens
        .iter()
        .any(|(_, value, ty, _)| value == "% 注释 😀 \\fake{label}" && ty == "comment"));
    assert!(tokens
        .iter()
        .any(|(_, value, ty, _)| value == "%20/\\fake" && ty == "string"));
    assert!(tokens
        .iter()
        .any(|(_, value, ty, _)| value == "\\verb" && ty == "macro"));
    assert!(tokens
        .iter()
        .any(|(_, value, ty, _)| value == "\\lstinline" && ty == "macro"));
    assert!(tokens
        .iter()
        .any(|(_, value, ty, _)| value == "😀 \\fake%raw" && ty == "string"));
    assert!(tokens
        .iter()
        .any(|(_, value, ty, _)| value == "\\fake{comment}" && ty == "comment"));
    assert!(!tokens
        .iter()
        .any(|(_, value, ty, _)| value.contains("fake") && ty == "macro"));
    assert!(!tokens
        .iter()
        .any(|(range, _, kind, _)| range.start.line == 9 && kind == "macro"));
}

#[test]
fn compound_destinations_keep_comments_separators_and_display_text_separate() {
    let doc = document("main.tex", "\\hyperdef[label]{section}% keep\r\n{intro}{Display}\\hyperref{}{section}% link\r\n{intro}{Display}\\pdfbookmark[01]{Title}{book}\\hyperlink{book.01}{Link}");
    let tokens = decoded(&doc, None);
    for (value, modifier) in [
        ("label", 1),
        ("section", 1),
        ("intro", 1),
        ("section", 0),
        ("intro", 0),
        ("book", 1),
        ("book.01", 0),
    ] {
        assert!(
            tokens.iter().any(|(_, text, ty, bits)| text == value
                && ty == "enumMember"
                && *bits == modifier),
            "{value} / {modifier}: {tokens:?}"
        );
    }
    assert!(tokens
        .iter()
        .filter(|(_, _, ty, _)| ty == "enumMember")
        .all(|(_, value, _, _)| !value.contains(['{', '}', '%'])));
    assert!(!tokens
        .iter()
        .any(|(_, value, _, _)| value.contains("Display")
            || value.contains("Title")
            || value.contains("Link")));
}

#[test]
fn range_queries_clip_tokens_and_keep_absolute_document_positions() {
    let doc = document(
        "main.tex",
        "Prose\r\n% 中😀 text\r\n\\label{key}\\ref{key}\r\n",
    );
    let range = Range::new(Position::new(1, 2), Position::new(1, 5));
    let tokens = decoded(&doc, Some(range));
    assert_eq!(tokens, [(range, "中😀".into(), "comment".into(), 0)]);
    assert!(decoded(
        &doc,
        Some(Range::new(Position::new(2, 3), Position::new(2, 3)))
    )
    .is_empty());
    for invalid in [
        Range::new(Position::new(1, 4), Position::new(1, 5)), // surrogate midpoint
        Range::new(Position::new(2, 4), Position::new(2, 1)),
        Range::new(Position::new(99, 0), Position::new(99, 0)),
    ] {
        assert!(semantic::tokens(&doc, Some(invalid)).is_err());
    }
    let full = decoded(&doc, None);
    let selected = Range::new(Position::new(2, 0), Position::new(3, 0));
    assert_eq!(
        decoded(&doc, Some(selected)),
        full.into_iter()
            .filter(|(range, _, _, _)| range.start.line == 2)
            .collect::<Vec<_>>()
    );
}

#[test]
fn bibliography_keys_use_the_existing_index_and_style_files_stay_unclassified() {
    let doc = document("refs.bib", "% 😀 \\fake\r\n@comment{Ignore @book{fake, title={X}}}\r\n@book{parent, title={Some @article{fake}}, year={2025}}\r\n@article{child, crossref={parent}}\r\n");
    let tokens = decoded(&doc, None);
    assert_eq!(
        tokens
            .iter()
            .filter(|(_, _, kind, _)| kind == "enumMember")
            .map(|(_, value, _, bits)| (value.as_str(), *bits))
            .collect::<Vec<_>>(),
        [("parent", 1), ("child", 1), ("parent", 0)]
    );
    assert_eq!(
        tokens
            .iter()
            .filter(|(_, _, kind, _)| kind == "comment")
            .count(),
        2
    );
    assert!(decoded(
        &document("plain.bst", "ENTRY { title } {} {}\n% comment\n"),
        None
    )
    .is_empty());
}

#[test]
fn viewport_ranges_accept_eof_and_oversized_columns_without_relaxing_edit_positions() {
    for source in ["\\ref{key}", "\\ref{key}\n", "\\ref{key}\r\n"] {
        let doc = document("no-final-newline.tex", source);
        let full = semantic::tokens(&doc, None).unwrap();
        let eof = Position::new(doc.text.text_line_count() as u32, 0);
        let viewport = semantic::tokens(&doc, Some(Range::new(Position::new(0, 0), eof))).unwrap();
        assert_eq!(full.data, viewport.data);
        assert_eq!(
            full.data,
            semantic::tokens(
                &doc,
                Some(Range::new(Position::new(0, 0), Position::new(0, u32::MAX)))
            )
            .unwrap()
            .data
        );
        assert!(doc.text.offset(eof).is_none());
        assert!(doc.text.offset(Position::new(0, u32::MAX)).is_none());
    }
}

#[test]
fn large_single_line_documents_encode_in_order_without_overlaps() {
    let mut source = "😀 ".to_owned();
    source.push_str(&"\\ref{key}".repeat(10_000));
    let doc = document("large.tex", &source);
    let tokens = semantic::tokens(&doc, None).unwrap().data;
    assert_eq!(tokens.len(), 20_000);
    assert_eq!(tokens[0].delta_start, 3);
    assert!(tokens.iter().all(|token| token.delta_line == 0));
    assert_eq!(
        tokens.iter().map(|token| token.delta_start).sum::<u32>(),
        89_999
    );
}
