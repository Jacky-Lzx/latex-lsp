use latex_lsp::{
    editing, features,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///bibliography/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    uri
}

fn position(source: &str, needle: &str) -> Position {
    Text::new(source.into()).position(source.find(needle).unwrap() + needle.len())
}

fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .map(|edit| TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
}

#[test]
fn bibliography_relations_navigate_highlight_and_rename_without_changing_literal_text() {
    let mut workspace = Workspace::default();
    let parents = "@book{parent, title={Parent}}\n@xdata{common, year={2026}}\n";
    let parent = insert(&mut workspace, "parents.bib", parents);
    let source = "@incollection{child,\n  crossref = {parent},\n  XREF = \"parent\",\n  xdata = {common, parent},\n  related = \"other, parent\",\n  entryset = {parent, child},\n  title = {parent, crossref={parent}},\n  note = \"crossref = {parent}\"\n}\n% crossref={parent}\n@comment{crossref={parent}}\n@string{crossref = \"parent\"}\n@preamble{\"crossref={parent}\"}\n";
    let child = insert(&mut workspace, "children.bib", source);
    let main = insert(&mut workspace, "main.tex", "\\cite{parent}\\nocite{*}");
    let cursor = position(source, "crossref = {par");
    let definitions = features::definitions(&workspace, &child, cursor);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].uri, parent);
    let references = features::references(&workspace, &child, cursor, false);
    assert_eq!(references.len(), 6);
    assert!(features::hover(&workspace, &child, cursor).is_some());
    assert_eq!(editing::highlights(&workspace, &child, cursor).len(), 5);
    let edits = features::rename(&workspace, &child, cursor, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits[&child].len(), 5);
    assert_eq!(
        apply(parents, edits[&parent].clone()),
        parents.replacen("{parent,", "{renamed,", 1)
    );
    assert_eq!(
        apply(
            &workspace.documents[&main].text.source,
            edits[&main].clone()
        ),
        "\\cite{renamed}\\nocite{*}"
    );
    let updated = apply(source, edits[&child].clone());
    assert!(updated.contains("crossref = {renamed}"));
    assert!(updated.contains("xdata = {common, renamed}"));
    assert!(updated.contains("title = {parent, crossref={parent}}"));
    assert!(updated.contains("note = \"crossref = {parent}\""));
    assert!(updated.contains("@comment{crossref={parent}}"));
    assert!(features::rename(&workspace, &child, cursor, "*").is_err());
}

#[test]
fn bibliography_completion_replaces_only_the_selected_key_and_supports_incomplete_values() {
    for (source, needle, expected) in [
        (
            "@article{child, crossref={pareBAD}}",
            "crossref={pare",
            "@article{child, crossref={parent}}",
        ),
        (
            "@article{child, related={other, pareBAD, last}}",
            "related={other, pare",
            "@article{child, related={other, parent, last}}",
        ),
        (
            "@article{child, xdata=\"other, pareBAD, last\"}",
            "xdata=\"other, pare",
            "@article{child, xdata=\"other, parent, last\"}",
        ),
        (
            "@article{child, crossref={pare",
            "crossref={pare",
            "@article{child, crossref={parent",
        ),
        (
            "@article{child, crossref=\"pare",
            "crossref=\"pare",
            "@article{child, crossref=\"parent",
        ),
        (
            "@article{child, crossref={",
            "crossref={",
            "@article{child, crossref={parent",
        ),
    ] {
        let mut workspace = Workspace::default();
        insert(
            &mut workspace,
            "parents.bib",
            "@book{parent, title={Parent}}",
        );
        let uri = insert(&mut workspace, "children.bib", source);
        let completions = features::completion(&workspace, &uri, position(source, needle));
        let item = completions
            .iter()
            .find(|item| item.label == "parent")
            .unwrap_or_else(|| panic!("missing completion: {source}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit");
        };
        assert_eq!(apply(source, vec![edit.clone()]), expected);
    }
}

#[test]
fn bibliography_string_expressions_escapes_comments_and_other_fields_are_not_key_references() {
    let mut workspace = Workspace::default();
    let source = "@article(child,\n title={crossref={parent}},\n crossref=prefix # {parent},\n xref=parent,\n xdata={parent\\macro},\n related={{parent}},\n note={xdata={parent}},\n % } crossref={parent}\n entryset={parent, other}\n)\n";
    let uri = insert(&mut workspace, "refs.bib", source);
    assert_eq!(
        workspace.documents[&uri]
            .syntax
            .citation_references
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["parent", "other"]
    );
    assert!(features::completion(&workspace, &uri, position(source, "xref=par")).is_empty());
    assert!(
        features::definitions(&workspace, &uri, position(source, "title={crossref={par"))
            .is_empty()
    );
    assert_eq!(workspace.documents[&uri].syntax.citations.len(), 1);
}

#[test]
#[ignore = "requires latexmk, BibTeX and a TeX installation"]
fn real_bibtex_crossrefs_still_compile_after_parent_entry_rename() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("refs.bib");
    let bib_uri = Url::from_file_path(&bib).unwrap();
    let source = "@inproceedings{child, author={A. Author}, title={Child work}, crossref={parent}}\n@proceedings{parent, title={Collected works}, booktitle={Collected works}, editor={E. Editor}, year={2026}}\n";
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\cite{child}\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n").unwrap();
    std::fs::write(&bib, source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let edits = features::rename(
        &workspace,
        &bib_uri,
        position(source, "crossref={par"),
        "renamedParent",
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    std::fs::write(&bib, apply(source, edits[&bib_uri].clone())).unwrap();
    let outcome = latex_lsp::build::run(
        &main,
        &latex_lsp::config::BuildConfig::default(),
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(
        !outcome
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.message.contains("crossref")),
        "{:?}",
        outcome.diagnostics
    );
    assert!(
        std::fs::read_to_string(directory.path().join("build/latex-lsp/main.bbl"))
            .unwrap()
            .contains("Collected works")
    );
}
