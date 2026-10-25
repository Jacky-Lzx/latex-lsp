use latex_lsp::{language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;
fn edits(source: &str, changes: &[TextEdit]) -> String {
    let mut changes = changes.to_vec();
    changes.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
        .changed(
            changes
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
fn glossary_cross_file_namespace_completion_hover_and_versioned_rename() {
    let dir = tempfile::tempdir().unwrap();
    let declarations = "\\newacronym{cpu}{CPU}{Central processing unit}\n\\newglossaryentry{computer}{name=computer,description={A machine}}";
    std::fs::write(dir.path().join("terms.tex"), declarations).unwrap();
    let source = "\\documentclass{article}\n\\loadglsentries{terms}\n😀 \\label{cpu} \\gls{cpu} \\ref{cpu} \\gls{computer}";
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.open(uri.clone(), source.into(), 7);
    let at = Text::new(source.into()).position(source.find("gls{cpu}").unwrap() + 6);
    let defs = features::definitions(&ws, &uri, at);
    assert_eq!(defs.len(), 1);
    assert!(defs[0].uri.path().ends_with("terms.tex"));
    let hover = format!("{:?}", features::hover(&ws, &uri, at));
    assert!(hover.contains("Central processing unit"), "{hover}");
    assert_eq!(features::references(&ws, &uri, at, true).len(), 2);
    assert!(!ws
        .diagnostics(&uri)
        .iter()
        .any(|d| d.message.contains("Undefined glossary")));
    assert!(features::completion(&ws, &uri, at)
        .iter()
        .any(|i| i.label == "cpu"));
    let changes = features::rename(&ws, &uri, at, "processor")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits(source, &changes[&uri]),
        source.replace("gls{cpu}", "gls{processor}")
    );
    assert_eq!(
        edits(declarations, &changes[&defs[0].uri]),
        declarations.replace("{cpu}", "{processor}")
    );
    assert!(features::rename(&ws, &uri, at, "computer").is_err());
    ws.open(uri.clone(), source.replace("gls{cpu}", "gls{missing}"), 8);
    assert!(ws
        .diagnostics(&uri)
        .iter()
        .any(|d| d.message == "Undefined glossary entry `missing`"));
}
#[test]
fn index_names_and_terms_preserve_printed_forms_and_encapsulators() {
    let uri = Url::parse("file:///extra-symbols/index.tex").unwrap();
    let source = r"\makeindex[name=people,title=People]\index[people]{Ada Lovelace@Lovelace|textbf}\index[people]{Ada Lovelace}\index{math!algebra}\printindex[people]\printindex[missing]";
    let mut ws = Workspace::default();
    ws.open(uri.clone(), source.into(), 1);
    let text = Text::new(source.into());
    let at = text.position(source.find("Ada").unwrap() + 2);
    assert_eq!(features::references(&ws, &uri, at, false).len(), 2);
    let change = features::rename(&ws, &uri, at, "Ada Byron")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits(source, &change[&uri]),
        source.replace("Ada Lovelace", "Ada Byron")
    );
    let at = text.position(source.find("index[people]").unwrap() + 8);
    assert_eq!(features::definitions(&ws, &uri, at).len(), 1);
    let change = features::rename(&ws, &uri, at, "names")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits(source, &change[&uri]),
        source.replace("people", "names")
    );
    let diagnostics = ws.diagnostics(&uri);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.message.starts_with("Undefined index"))
            .count(),
        1
    );
    assert!(features::symbols(&ws.documents[&uri])
        .iter()
        .any(|s| s.name == "math!algebra"));
}
#[test]
fn glossary_static_boundaries_and_duplicate_diagnostics() {
    let uri = Url::parse("file:///extra-symbols/ignored.tex").unwrap();
    let source = "\\newglossaryentry{real}{name=R,description=R}\n\\newglossaryentry{real}{name=X,description=X}\n% \\gls{comment}\n\\verb|\\gls{verbatim}|\n\\newcommand{\\deferred}{\\gls{deferred}}\n\\gls{\\dynamic}\n\\gls{missing}";
    let mut ws = Workspace::default();
    ws.open(uri.clone(), source.into(), 1);
    let diagnostics = ws.diagnostics(&uri);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.message.starts_with("Duplicate glossary"))
            .count(),
        2
    );
    let undefined: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Undefined glossary"))
        .collect();
    assert_eq!(undefined.len(), 1, "{diagnostics:?}");
    assert!(undefined[0].message.contains("missing"));
    let at = Text::new(source.into()).position(source.find("real").unwrap() + 2);
    assert!(features::rename(&ws, &uri, at, "renamed").is_err());
    ws.open(
        uri.clone(),
        "\\newglossaryentry{real}{name=R,description=R}\\gls{r".into(),
        2,
    );
    let at = Position::new(0, ws.documents[&uri].text.source.len() as u32);
    assert!(features::completion(&ws, &uri, at)
        .iter()
        .any(|i| i.label == "real"));
}
#[test]
fn shared_glossary_updates_consumers_and_rejects_ambiguous_roots() {
    let dir = tempfile::tempdir().unwrap();
    let terms = "\\newglossaryentry{shared}{name=S,description=Shared}";
    std::fs::write(dir.path().join("terms.tex"), terms).unwrap();
    for name in ["a", "b"] {
        std::fs::write(
            dir.path().join(format!("{name}.tex")),
            "\\documentclass{article}\\loadglsentries{terms}\\gls{shared}",
        )
        .unwrap();
    }
    let a = Url::from_file_path(dir.path().join("a.tex")).unwrap();
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    let declaration = Url::from_file_path(dir.path().join("terms.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.add_root(dir.path().into());
    ws.open(declaration.clone(), terms.into(), 3);
    let at = Text::new(terms.into()).position(terms.find("shared").unwrap() + 2);
    assert_eq!(features::references(&ws, &declaration, at, false).len(), 2);
    ws.versioned_edits = true;
    let edit = features::rename(&ws, &declaration, at, "renamed")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
        panic!()
    };
    assert_eq!(changes.len(), 3);
    assert_eq!(
        changes
            .iter()
            .find(|e| e.text_document.uri == declaration)
            .unwrap()
            .text_document
            .version,
        Some(3)
    );
    let other = format!(
        "{}\\newglossaryentry{{shared}}{{name=X,description=Other}}",
        ws.documents[&b].text.source
    );
    ws.open(b, other, 4);
    assert!(features::rename(
        &ws,
        &a,
        ws.documents[&a]
            .text
            .position(ws.documents[&a].text.source.find("gls{shared}").unwrap() + 6),
        "renamed"
    )
    .is_err());
}
