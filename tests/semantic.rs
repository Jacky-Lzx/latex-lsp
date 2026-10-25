use latex_lsp::{language::semantic, project::workspace::Document};
use lsp_types::*;

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
fn bibtex_abbreviation_declarations_and_value_operands_are_variables() {
    let doc = document("strings.bib", "% 😀\r\n@string{Journal={Text}}\r\n@string{Full=JOURNAL # { text}}\r\n@preamble{journal}\r\n@book{Journal,title=Full,month=jan,note={Journal}}");
    let tokens = decoded(&doc, None);
    let variables: Vec<_> = tokens
        .iter()
        .filter(|(_, _, kind, _)| kind == "variable")
        .map(|(_, name, _, declaration)| (name.as_str(), *declaration))
        .collect();
    assert_eq!(
        variables,
        [
            ("Journal", 1),
            ("Full", 1),
            ("JOURNAL", 0),
            ("journal", 0),
            ("Full", 0),
            ("jan", 0)
        ]
    );
    let range = tokens
        .iter()
        .find(|(_, name, _, _)| name == "JOURNAL")
        .unwrap()
        .0;
    let partial = decoded(
        &doc,
        Some(Range::new(
            range.start,
            Position::new(range.start.line, range.start.character + 3),
        )),
    );
    assert_eq!(partial[0].1, "JOU");
}

#[test]
fn bibliography_literals_preserve_validated_link_boundaries() {
    let doc = document("refs.bib", "@book{😀, url={\r\n https://example.org/中😀 \r\n}, doi={10.1234/key}, title={https://example.org/title}}\r\n@book{other,url={https://example.org/duplicate},url=unknown,doi={10.1234/} # suffix}");
    let tokens = decoded(&doc, None);
    let strings: Vec<_> = tokens
        .iter()
        .filter(|(_, _, kind, _)| kind == "string")
        .map(|(_, value, _, _)| value.as_str())
        .collect();
    assert_eq!(
        strings,
        [
            "{",
            " ",
            "https://example.org/中😀",
            " ",
            "}",
            "{",
            "10.1234/key",
            "}",
            "{https://example.org/title}",
            "{https://example.org/duplicate}",
            "{10.1234/}"
        ]
    );
    let range = tokens
        .iter()
        .find(|(_, value, _, _)| value == "https://example.org/中😀")
        .unwrap()
        .0;
    assert_eq!(range.start, Position::new(1, 1));
    let partial = decoded(
        &doc,
        Some(Range::new(
            range.start,
            Position::new(1, range.start.character + 5),
        )),
    );
    assert_eq!(partial[0].1, "https");
}

#[test]
fn bibliography_structure_highlights_original_case_and_expression_operands() {
    let doc = document("structure.bib", "@Custom-Type{key, TiTle = {Some {title}}, year=2026, note=jan # \"中😀\" # 42}\r\n@STRING(Full = \"Journal\" # jan)\r\n@PREAMBLE{Full # { text}}\r\n@boo");
    let tokens: Vec<_> = decoded(&doc, None)
        .into_iter()
        .map(|(_, value, kind, bits)| (value, kind, bits))
        .collect();
    let expected = [
        ("Custom-Type", "type", 0),
        ("key", "enumMember", 1),
        ("TiTle", "property", 0),
        ("=", "operator", 0),
        ("{Some {title}}", "string", 0),
        ("year", "property", 0),
        ("=", "operator", 0),
        ("2026", "number", 0),
        ("note", "property", 0),
        ("=", "operator", 0),
        ("jan", "variable", 0),
        ("#", "operator", 0),
        ("\"中😀\"", "string", 0),
        ("#", "operator", 0),
        ("42", "number", 0),
        ("STRING", "type", 0),
        ("Full", "variable", 1),
        ("=", "operator", 0),
        ("\"Journal\"", "string", 0),
        ("#", "operator", 0),
        ("jan", "variable", 0),
        ("PREAMBLE", "type", 0),
        ("Full", "variable", 0),
        ("#", "operator", 0),
        ("{ text}", "string", 0),
        ("boo", "type", 0),
    ];
    assert_eq!(
        tokens,
        expected.map(|(value, kind, bits)| (value.into(), kind.into(), bits))
    );
}

#[test]
fn bibliography_comments_do_not_leak_types_fields_or_operands() {
    let source = concat!(
        "% top 😀\r\n@book % type gap\r\n{ % header\r\nkey\"quoted,\r\n",
        "% fields @fake{bad, title=9}\r\nTiTle % assignment\r\n= % operand\r\n",
        "\"100% {quoted \"nested\"} \\fake\" % tail\r\n# % concat\r\n2026, note={100% \\fake}}\r\n",
        "@comment{ @book{hidden,title=2026} % opaque\r\n title=jan }\r\n",
        "@string{month % name gap\r\n= % string gap\r\njan}\r\n",
        "@preamble{% preamble\r\nmonth}\r\n",
    );
    let doc = document("comments.bib", source);
    let tokens = decoded(&doc, None);
    let comments: Vec<_> = tokens
        .iter()
        .filter(|(_, _, kind, _)| kind == "comment")
        .map(|(_, value, _, _)| value.as_str())
        .collect();
    assert_eq!(
        comments,
        [
            "% top 😀",
            "% type gap",
            "% header",
            "% fields @fake{bad, title=9}",
            "% assignment",
            "% operand",
            "% tail",
            "% concat",
            "@comment{ @book{hidden,title=2026} % opaque",
            " title=jan }",
            "% name gap",
            "% string gap",
            "% preamble"
        ]
    );
    for (value, kind) in [
        ("key\"quoted", "enumMember"),
        ("\"100% {quoted \"nested\"} \\fake\"", "string"),
        ("{100% \\fake}", "string"),
        ("2026", "number"),
    ] {
        assert!(
            tokens
                .iter()
                .any(|(_, text, ty, _)| text == value && ty == kind),
            "missing {value}: {tokens:?}"
        );
    }
    assert!(!tokens.iter().any(|(_, _, kind, _)| kind == "macro"));
}

#[test]
fn bibliography_relationship_keys_override_literal_backgrounds() {
    let doc = document("relationships.bib", "@book{child,crossref={parent},related=\"other, key\"}\r\n@book{parent,title={line one\r\n中😀 line two}}");
    let tokens = decoded(&doc, None);
    for value in ["parent", "other", "key"] {
        assert!(tokens
            .iter()
            .any(|(_, text, kind, bits)| text == value && kind == "enumMember" && *bits == 0));
    }
    let selected = Range::new(Position::new(2, 0), Position::new(2, 3));
    assert_eq!(
        decoded(&doc, Some(selected)),
        [(selected, "中😀".into(), "string".into(), 0)]
    );
}

#[test]
fn bibliography_unfinished_literals_and_fields_remain_highlighted() {
    for literal in ["{unfinished % 😀 ", "\"unfinished {nested} % 😀 "] {
        let doc = document(
            "unfinished.bib",
            &format!("@book{{key, title = jan # {literal}"),
        );
        let tokens = decoded(&doc, None);
        assert!(tokens
            .iter()
            .any(|(_, value, kind, _)| value == literal && kind == "string"));
        assert!(tokens
            .iter()
            .any(|(_, value, kind, _)| value == "jan" && kind == "variable"));
        assert!(!tokens.iter().any(|(_, _, kind, _)| kind == "comment"));
    }
    let doc = document("unfinished.bib", "@book{key, unknown_field\r\n@123");
    let tokens = decoded(&doc, None);
    assert!(tokens
        .iter()
        .any(|(_, value, kind, _)| value == "unknown_field" && kind == "property"));
    assert!(!tokens
        .iter()
        .any(|(_, value, kind, _)| value == "123" && kind == "type"));
}

#[test]
fn bibliography_large_index_keeps_tokens_ordered_and_comments_isolated() {
    let source: String = (0..1000)
        .map(|index| format!("@book{{key{index},title={{中😀 % text}},year=2026 # jan}} % end\r\n"))
        .collect();
    let doc = document("large.bib", &source);
    let tokens = decoded(&doc, None);
    assert_eq!(tokens.len(), 11000);
    assert_eq!(
        tokens
            .iter()
            .filter(|(_, _, kind, _)| kind == "comment")
            .count(),
        1000
    );
}

#[test]
fn bibliography_comment_construct_overrides_comments_between_type_and_body() {
    let doc = document(
        "opaque.bib",
        "@comment % header\r\n{ title={% literal}, year=42 }\r\n@book{key,title={visible}}",
    );
    let tokens = decoded(&doc, None);
    assert_eq!(
        tokens
            .iter()
            .filter(|(_, _, kind, _)| kind == "comment")
            .count(),
        2
    );
    assert!(!tokens.iter().any(|(_, _, kind, _)| kind == "number"));
    assert!(tokens
        .iter()
        .any(|(_, value, kind, _)| value == "{visible}" && kind == "string"));
}

#[test]
fn environment_copy_targets_are_declarations_and_sources_are_type_references() {
    for command in [
        "NewEnvironmentCopy",
        "RenewEnvironmentCopy",
        "DeclareEnvironmentCopy",
    ] {
        let source = format!("😀 \\{command}{{copied}}{{original}}\\begin{{copied}}Text.\\end{{copied}}\n\\{command}{{other}}{{ original }}");
        let doc = document("main.tex", &source);
        let tokens = decoded(&doc, None);
        for (value, bits) in [("copied", 1), ("copied", 0), ("original", 0)] {
            assert!(
                tokens.iter().any(|(_, text, kind, flags)| text == value
                    && kind == "type"
                    && *flags == bits),
                "{tokens:?}"
            );
        }
        let unsupported = source.rfind("{ original }").unwrap() + 1;
        assert!(!tokens.iter().any(|(range, _, kind, _)| {
            kind == "type" && doc.text.offset(range.start).unwrap() >= unsupported
        }));
    }
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
