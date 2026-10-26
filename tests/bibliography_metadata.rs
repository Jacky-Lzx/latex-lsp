use latex_lsp::{
    language::features,
    project::workspace::{Document, Workspace},
};
use lsp_types::*;
use std::path::Path;

fn project(root: &Path, bibliography: &str) -> (Workspace, Url, Url) {
    std::fs::write(root.join("main.tex"), r"\documentclass{article}\begin{document}中文😀 \cite{alpha}\bibliography{refs}\end{document}").unwrap();
    std::fs::write(root.join("refs.bib"), bibliography).unwrap();
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for doc in workspace.documents.values_mut() {
        doc.version = Some(7);
    }
    (
        workspace,
        Url::from_file_path(root.join("main.tex")).unwrap(),
        Url::from_file_path(root.join("refs.bib")).unwrap(),
    )
}
fn at(workspace: &Workspace, main: &Url) -> Position {
    let doc = &workspace.documents[main];
    doc.text
        .position(doc.text.source.find("alpha").unwrap() + 2)
}
fn detail(source: &str) -> String {
    let uri = Url::parse("file:///metadata/refs.bib").unwrap();
    Document::new(&uri, source.into(), Some(7)).syntax.citations[0]
        .detail
        .clone()
}

#[test]
fn citation_metadata_reaches_hover_completion_and_symbols_without_changing_key_edits() {
    let directory = tempfile::tempdir().unwrap();
    let (workspace, main, database) = project(directory.path(), "@article{alpha, title={An alpha study 😀}, author=\"Ada Lovelace and Grace Hopper\", year=2026}\r\n");
    let position = at(&workspace, &main);
    let hover = features::hover(&workspace, &main, position).unwrap();
    let HoverContents::Markup(contents) = hover.contents else {
        panic!()
    };
    assert_eq!(contents.kind, MarkupKind::Markdown);
    assert_eq!(contents.value, "```bibtex\n@article{alpha, title={An alpha study 😀}, author=\"Ada Lovelace and Grace Hopper\", year=2026}\n```");
    let completions = features::completion(&workspace, &main, position);
    let completion = completions
        .iter()
        .find(|item| item.label == "alpha")
        .unwrap();
    let symbols = features::symbols(&workspace.documents[&database]);
    assert!(completion.detail.is_none());
    assert_eq!(
        completion.documentation,
        Some(Documentation::MarkupContent(contents))
    );
    let text = symbols[0].detail.as_ref().unwrap();
    for expected in [
        "BibTeX article",
        "Title: An alpha study 😀",
        "Author: Ada Lovelace and Grace Hopper",
        "Year: 2026",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    let definition = &features::definitions(&workspace, &main, position)[0];
    assert_eq!(definition.uri, database);
    let bib = &workspace.documents[&database].text;
    assert_eq!(
        &bib.source[bib.offset(definition.range.start).unwrap()
            ..bib.offset(definition.range.end).unwrap()],
        "alpha"
    );
    let edit = features::rename(&workspace, &main, position, "renamedkey")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(edits)) = edit.document_changes else {
        panic!()
    };
    assert_eq!(edits.len(), 2);
    for edit in edits {
        assert_eq!(edit.text_document.version, Some(7));
        let original = &workspace.documents[&edit.text_document.uri].text;
        for change in edit.edits {
            let OneOf::Left(change) = change else {
                panic!()
            };
            assert_eq!(change.new_text, "renamedkey");
            assert_eq!(
                &original.source[original.offset(change.range.start).unwrap()
                    ..original.offset(change.range.end).unwrap()],
                "alpha"
            );
            if edit.text_document.uri == database {
                assert_eq!(change.range.start, definition.range.start);
            }
        }
    }
}

#[test]
fn literal_metadata_handles_quotes_groups_and_numbers_without_expanding_strings() {
    for (source, expected, absent) in [
        (
            "@book(alpha, TITLE=\"A {Nested} Title\", AUTHOR={Author Name}, YEAR={2025})",
            vec![
                "Title: A {Nested} Title",
                "Author: Author Name",
                "Year: 2025",
            ],
            vec![],
        ),
        (
            r"@article{alpha, title={Hash # literal}, author={\LaTeX Author}, year=2026}",
            vec![
                "Title: Hash # literal",
                r"Author: \LaTeX Author",
                "Year: 2026",
            ],
            vec![],
        ),
        (
            r"@article{alpha, title={First} # {Second}, author=authors, year=yearmacro}",
            vec![],
            vec!["Title:", "Author:", "Year:"],
        ),
        (
            r"@article{alpha, crossref={parent}, author={Known Author}}",
            vec!["Author: Known Author"],
            vec!["Title:", "Year:"],
        ),
    ] {
        let actual = detail(source);
        for expected in expected {
            assert!(actual.contains(expected), "{source}: {actual}");
        }
        for absent in absent {
            assert!(!actual.contains(absent), "{source}: {actual}");
        }
    }
}

#[test]
fn duplicate_empty_and_unterminated_fields_do_not_invent_metadata() {
    for source in [
        "@article{alpha, title={One}, TITLE={Two}, year=2026, YEAR=2025}",
        "@article{alpha, title={}, author=\" \"}",
        "@article{alpha, title={Unterminated",
        "@article{alpha, title=\"Unterminated",
        "@article{alpha, title={Incomplete {nested}",
        "@article{alpha, title=\"Incomplete \\\"",
        "@article{alpha, title={One}, title=macro, title={Three}}",
    ] {
        assert_eq!(detail(source), "BibTeX article", "{source}");
    }
}

#[test]
fn metadata_previews_bound_unicode_and_follow_the_current_project_and_buffer() {
    let title = "😀".repeat(205);
    assert_eq!(
        detail(&format!("@article{{alpha, title={{{title}}}}}")),
        format!("BibTeX article · Title: {}…", "😀".repeat(200))
    );
    assert!(
        detail("@article{alpha, title={A\r\n   title\t with  spaces}}")
            .ends_with("Title: A title with spaces")
    );
    let directory = tempfile::tempdir().unwrap();
    let (mut workspace, main, database) =
        project(directory.path(), "@article{alpha,title={Selected}}\n");
    let other = Url::from_file_path(directory.path().join("other.bib")).unwrap();
    workspace.documents.insert(
        other.clone(),
        Document::new(
            &other,
            "@article{alpha,title={Other Unique}}".into(),
            Some(2),
        ),
    );
    for title in ["Selected", "Updated 😀"] {
        workspace.documents.insert(
            database.clone(),
            Document::new(
                &database,
                format!("@article{{alpha,title={{{title}}}}}"),
                Some(8),
            ),
        );
        let hover = features::hover(&workspace, &main, at(&workspace, &main)).unwrap();
        let HoverContents::Markup(contents) = hover.contents else {
            panic!()
        };
        assert!(contents.value.contains(title), "{}", contents.value);
        assert!(
            !contents.value.contains("Other Unique"),
            "{}",
            contents.value
        );
    }
}
