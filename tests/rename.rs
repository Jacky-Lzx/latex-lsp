use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, text: &str, version: Option<i32>) -> Url {
    let uri = Url::parse(&format!("file:///rename-test/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, text.into(), version));
    uri
}

fn edits_result(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let changes = edits
        .into_iter()
        .map(|edit| TextDocumentContentChangeEvent {
            range: Some(edit.range),
            range_length: None,
            text: edit.new_text,
        })
        .collect();
    Text::new(source.into()).changed(changes).unwrap().source
}

#[test]
fn renames_labels_in_multiple_files_but_not_comments_or_verbatim() {
    let mut workspace = Workspace::default();
    let declaration = insert(&mut workspace, "chapter.tex", "\\label{sec:old}", None);
    let source =
        "😀 \\ref{sec:old} \\cref{other, sec:old}\n% \\ref{sec:old}\n\\verb|\\ref{sec:old}|";
    let main = insert(&mut workspace, "main.tex", source, Some(7));
    let edit = features::rename(&workspace, &main, Position::new(0, 10), "sec:中文")
        .unwrap()
        .unwrap();
    let changes = edit.changes.unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(
        edits_result(
            &workspace.documents[&declaration].text.source,
            &changes[&declaration]
        ),
        r"\label{sec:中文}"
    );
    assert_eq!(
        edits_result(source, &changes[&main]),
        "😀 \\ref{sec:中文} \\cref{other, sec:中文}\n% \\ref{sec:old}\n\\verb|\\ref{sec:old}|"
    );
}

#[test]
fn citation_rename_updates_bibtex_keys_and_only_matching_citations() {
    let mut workspace = Workspace::default();
    let bib = insert(
        &mut workspace,
        "refs.bib",
        "@book{old, title={old}}\n@article{other, title={Old}}",
        None,
    );
    let main = insert(
        &mut workspace,
        "main.tex",
        r"\cite[see]{old, other}\citep{old}",
        Some(1),
    );
    let edit = features::rename(&workspace, &bib, Position::new(0, 8), "new2026")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits_result(&workspace.documents[&bib].text.source, &edit[&bib]),
        "@book{new2026, title={old}}\n@article{other, title={Old}}"
    );
    assert_eq!(
        edits_result(&workspace.documents[&main].text.source, &edit[&main]),
        r"\cite[see]{new2026, other}\citep{new2026}"
    );
}

#[test]
fn macro_rename_preserves_backslashes_and_carries_document_versions() {
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    let source = r"\newcommand{\foo}[1]{#1} \foo{中文}";
    let uri = insert(&mut workspace, "main.tex", source, Some(12));
    let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
        features::prepare_rename(&workspace, &uri, Position::new(0, 14)).unwrap()
    else {
        panic!("expected rename target");
    };
    assert_eq!(placeholder, "foo");
    assert_eq!(range.start.character, 13);
    let edit = features::rename(&workspace, &uri, Position::new(0, 14), "bar")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edit");
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.version, Some(12));
    let edits = documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect::<Vec<_>>();
    assert_eq!(edits.len(), 2);
    assert_eq!(
        edits_result(source, &edits),
        r"\newcommand{\bar}[1]{#1} \bar{中文}"
    );
}

#[test]
fn modern_and_unbraced_macro_declarations_support_navigation_completion_and_rename() {
    for declaration in [
        r"\newcommand\hello[1]{Hello #1}",
        r"\renewcommand* \hello [1] {Hello #1}",
        "\\providecommand % declaration\n\\hello[1]{Hello #1}",
        r"\DeclareMathOperator*{\hello}{hello}",
        r"\DeclareRobustCommand\hello[1]{#1}",
        r"\NewDocumentCommand{\hello}{m}{#1}",
        r"\RenewDocumentCommand\hello{m}{#1}",
        r"\ProvideDocumentCommand{\hello}{O{x}}{#1}",
        r"\DeclareDocumentCommand{\hello}{m}{#1}",
        r"\def\hello#1{Hello #1}",
        r"\gdef\hello#1{Hello #1}",
        r"\edef\hello{Hello}",
        r"\xdef\hello{Hello}",
        r"\let\hello=\textbf",
        r"\long\def\hello#1{Hello #1}",
    ] {
        let mut workspace = Workspace::default();
        let macros = insert(&mut workspace, "macros.tex", declaration, Some(1));
        let main = insert(&mut workspace, "main.tex", "\\hello{World}\n\\hel", Some(3));
        let definitions = &workspace.documents[&macros].syntax.definitions;
        assert_eq!(definitions.len(), 1, "{declaration}");
        assert_eq!(definitions[0].name, "hello");
        assert!(
            features::diagnostics(&workspace.documents[&macros]).is_empty(),
            "{declaration}"
        );
        assert_eq!(
            features::definitions(&workspace, &main, Position::new(0, 3))[0].uri,
            macros
        );
        assert_eq!(
            features::references(&workspace, &main, Position::new(0, 3), false).len(),
            1,
            "{declaration}"
        );
        assert_eq!(
            features::references(&workspace, &main, Position::new(0, 3), true).len(),
            2,
            "{declaration}"
        );
        assert!(features::completion(&workspace, &main, Position::new(1, 4))
            .iter()
            .any(|item| item.label == "hello"));
        let declaration_position = workspace.documents[&macros]
            .text
            .position(definitions[0].span.start + 2);
        let edits = features::rename(&workspace, &macros, declaration_position, "greeting")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert!(
            edits_result(declaration, &edits[&macros]).contains("\\greeting"),
            "{declaration}"
        );
        assert_eq!(
            edits_result(&workspace.documents[&main].text.source, &edits[&main]),
            "\\greeting{World}\n\\hel"
        );
    }
}

#[test]
fn rejects_conflicts_ambiguous_definitions_invalid_names_and_builtins() {
    let mut workspace = Workspace::default();
    let uri = insert(
        &mut workspace,
        "main.tex",
        r"\label{old}\label{taken}\ref{old}\newcommand{\foo}{x}\foo",
        Some(1),
    );
    assert!(features::rename(&workspace, &uri, Position::new(0, 8), "taken").is_err());
    for name in ["", "bad name", "a,b", "a}x", "\\new"] {
        assert!(features::rename(&workspace, &uri, Position::new(0, 8), name).is_err());
    }
    assert!(
        features::prepare_rename(&workspace, &uri, Position::new(0, 3))
            .unwrap()
            .is_none()
    );
    let macro_position = workspace.documents[&uri]
        .text
        .position(workspace.documents[&uri].text.source.rfind("foo").unwrap() + 1);
    assert!(features::rename(&workspace, &uri, macro_position, "section").is_err());
    insert(&mut workspace, "another.tex", r"\label{old}", None);
    assert!(features::prepare_rename(&workspace, &uri, Position::new(0, 8)).is_err());
}
