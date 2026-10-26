use latex_lsp::{
    compilation::build_log,
    language::{features, semantic},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

#[path = "support/completion.rs"]
mod completion;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///bibtex-entry-types/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    uri
}
fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));
    Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .map(|e| TextDocumentContentChangeEvent {
                    range: Some(e.range),
                    range_length: None,
                    text: e.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
}

#[test]
fn custom_type_identifiers_preserve_metadata_and_versioned_cross_file_renames() {
    for kind in [
        "custom1",
        "custom_type",
        "custom-type",
        "custom:type",
        "_custom",
        "-custom",
        ":custom",
        "étype",
        "typeé",
        "a!",
        "a$",
        "a&",
        "a*",
        "a+",
        "a.",
        "a/",
        "a;",
        "a<",
        "a>",
        "a?",
        "a@",
        "a[",
        "a\\",
        "a]",
        "a^",
        "a`",
        "a|",
        "a~",
    ] {
        for (open, close) in [('{', '}'), ('(', ')')] {
            let old = "book(old)";
            let mut workspace = Workspace {
                versioned_edits: true,
                ..Default::default()
            };
            let source = format!("% 😀\r\n@ \t{kind}{open}{old}\r\n , title={{Literal {old}}}, author={{A. Author}}, year=2026{close}\r\n@article{{child, crossref={{{old}}}}}");
            let bib = insert(&mut workspace, "refs.bib", &source);
            let main_source = format!("😀 \\cite{{{old}}}");
            let main = insert(&mut workspace, "main.tex", &main_source);
            let doc = &workspace.documents[&bib];
            assert_eq!(
                doc.syntax
                    .citations
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>(),
                [old, "child"],
                "{kind}"
            );
            let entry = &doc.syntax.citations[0];
            for expected in [
                format!("BibTeX {kind}"),
                "Title: Literal book(old)".into(),
                "Author: A. Author".into(),
                "Year: 2026".into(),
            ] {
                assert!(entry.detail.contains(&expected), "{}", entry.detail);
            }
            let position =
                Text::new(main_source.clone()).position(main_source.find(old).unwrap() + 1);
            let definitions = features::definitions(&workspace, &main, position);
            assert_eq!(definitions.len(), 1);
            assert_eq!(definitions[0].uri, bib);
            assert_eq!(
                definitions[0].range,
                doc.text.range(entry.span.start, entry.span.end)
            );
            assert!(features::hover(&workspace, &main, position).is_some());
            let completion = features::completion(&workspace, &main, position)
                .into_iter()
                .find(|i| i.label == old)
                .unwrap();
            assert!(completion::preview(&completion).contains("title={Literal book(old)}"));
            assert_eq!(
                features::references(&workspace, &main, position, true).len(),
                3
            );
            assert_eq!(features::symbols(doc).len(), 2);
            let legend = semantic::options().legend;
            let mut pos = Position::default();
            let mut keys = Vec::new();
            for token in semantic::tokens(doc, None).unwrap().data {
                pos = Position::new(
                    pos.line + token.delta_line,
                    if token.delta_line == 0 {
                        pos.character + token.delta_start
                    } else {
                        token.delta_start
                    },
                );
                if legend.token_types[token.token_type as usize].as_str() == "enumMember" {
                    keys.push(
                        &source[doc.text.offset(pos).unwrap()
                            ..doc
                                .text
                                .offset(Position::new(pos.line, pos.character + token.length))
                                .unwrap()],
                    );
                }
            }
            assert_eq!(keys, [old, "child", old]);
            let Some(DocumentChanges::Edits(changes)) =
                features::rename(&workspace, &main, position, "book(next)")
                    .unwrap()
                    .unwrap()
                    .document_changes
            else {
                panic!()
            };
            assert_eq!(changes.len(), 2);
            for change in changes {
                assert_eq!(change.text_document.version, Some(7));
                let uri = change.text_document.uri;
                let original = &workspace.documents[&uri].text;
                let edits = change
                    .edits
                    .into_iter()
                    .map(|e| {
                        let OneOf::Left(e) = e else { panic!() };
                        e
                    })
                    .collect::<Vec<_>>();
                for e in &edits {
                    assert_eq!(
                        &original.source[original.offset(e.range.start).unwrap()
                            ..original.offset(e.range.end).unwrap()],
                        old
                    );
                }
                let expected = if uri == main {
                    "😀 \\cite{book(next)}".into()
                } else {
                    assert_eq!(uri, bib);
                    format!("% 😀\r\n@ \t{kind}{open}book(next)\r\n , title={{Literal {old}}}, author={{A. Author}}, year=2026{close}\r\n@article{{child, crossref={{book(next)}}}}")
                };
                assert_eq!(apply(&original.source, &edits), expected);
            }
        }
    }
}

#[test]
fn type_identifiers_keep_reserved_entries_and_invalid_numeric_types_opaque() {
    for gap in ["", " ", "\r\n\t"] {
        let source = format!("@{gap}comment{{@custom1{{fake, title={{Hidden}}}}}}\n@{gap}string{{macro=\"@custom_type(fake, title={{Hidden}})\"}}\n@{gap}preamble{{\"@custom-type{{fake, title={{Hidden}}}}\"}}\n@{gap}123{{invalid, title={{@book{{fake,}}}}}}\n@{gap}1custom{{invalid, title={{@book{{fake,}}}}}}\n@{gap}{{invalid, title={{@book{{fake,}}}}}}\n@{gap}comment1{{one, title={{@custom1{{fake,}}}}}}\n@{gap}string_type(two, title={{Kept}})\n@{gap}preamble:extra{{three, title={{Kept}}}}\n");
        let mut workspace = Workspace::default();
        let bib = insert(&mut workspace, "refs.bib", &source);
        assert_eq!(
            workspace.documents[&bib]
                .syntax
                .citations
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", "three"],
            "{source}"
        );
    }
}

#[test]
fn standard_unknown_type_warnings_use_exact_types_without_retargeting_examples_or_ambiguity() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("refs.bib");
    let other = directory.path().join("other.bib");
    let main_uri = Url::from_file_path(&main).unwrap();
    let bib_uri = Url::from_file_path(&bib).unwrap();
    std::fs::write(&main, "Main document\n").unwrap();
    for key in [
        "ordinary",
        "book(old)",
        "book(old",
        "book)old",
        "refs.bib",
        "(中文)",
    ] {
        let source = format!("% 😀\r\n@ custom_type(\r\n{key}\r\n , title={{Example}}, author={{A. Author}}, year=2026)\n");
        std::fs::write(&bib, &source).unwrap();
        let log = format!("Database file #1: refs.bib\nWarning--entry type for \"{key}\" isn't style-file defined\n--line 3 of file refs.bib\n");
        let diagnostics = build_log::parse_bibliography(&main, &log);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let items = &diagnostics[&bib_uri];
        assert_eq!(items.len(), 1);
        let start = source.find("custom_type").unwrap();
        assert_eq!(
            items[0].range,
            Text::new(source.clone()).range(start, start + "custom_type".len())
        );
        assert_eq!(items[0].severity, Some(DiagnosticSeverity::WARNING));
    }
    std::fs::write(&bib, "@custom1{ordinary, title={Example}}").unwrap();
    for message in [
        "Example entry type for \"ordinary\" isn't style-file defined",
        "entry type for \"ordinary\" wasn't style-file defined",
        "entry type for \"ordinary\" isn't style-file defined (example)",
        "entry type for \"\" isn't style-file defined",
    ] {
        let diagnostics = build_log::parse_bibliography(
            &main,
            &format!("Database file #1: refs.bib\nWarning--{message}\n"),
        );
        assert!(
            !diagnostics.contains_key(&bib_uri),
            "{message}: {diagnostics:?}"
        );
        assert_eq!(diagnostics[&main_uri].len(), 1);
    }
    std::fs::write(&other, "@custom_type{ordinary, title={Other}}").unwrap();
    let diagnostics = build_log::parse_bibliography(&main, "Database file #1: refs.bib\nDatabase file #2: other.bib\nWarning--entry type for \"ordinary\" isn't style-file defined\n");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[&main_uri].len(), 1);
}
