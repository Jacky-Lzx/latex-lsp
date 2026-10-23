use latex_lsp::{editing, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

#[test]
fn static_references_update_without_building_and_remain_project_scoped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.tex"),
        "\\documentclass{article}\n\\input{chapter}\n\\bibliography{refs}",
    )
    .unwrap();
    std::fs::write(dir.path().join("chapter.tex"), "\\label{known}").unwrap();
    std::fs::write(
        dir.path().join("refs.bib"),
        "@book{paper,title={Book},ids={alias}}",
    )
    .unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    let source = "\\documentclass{article}\n\\input{chapter}\n\\bibliography{refs}\n😀 \\cref{known,missing} \\crefrange{known}{other}\n\\cite{paper,alias,absent} \\nocite{*}\n% \\ref{comment}\n\\verb|\\ref{literal}|\n\\ref{\\dynamic}\n";
    ws.open(uri.clone(), source.into(), 1);
    let diagnostics = ws.diagnostics(&uri);
    let undefined: Vec<_> = diagnostics.iter().filter(|diag| matches!(&diag.code,Some(NumberOrString::String(code)) if code.starts_with("undefined-"))).collect();
    assert_eq!(undefined.len(), 3, "{diagnostics:?}");
    let text = Text::new(source.into());
    for key in ["missing", "other", "absent"] {
        assert!(undefined.iter().any(|diag| diag.range
            == text.range(
                source.find(key).unwrap(),
                source.find(key).unwrap() + key.len()
            )));
    }
    let other = Url::from_file_path(dir.path().join("independent.tex")).unwrap();
    std::fs::write(
        dir.path().join("independent.tex"),
        "\\documentclass{article}",
    )
    .unwrap();
    ws.open(other, "\\documentclass{article}\\label{missing}".into(), 1);
    assert!(ws
        .diagnostics(&uri)
        .iter()
        .any(|diag| diag.message == "Undefined reference `missing`"));
    ws.open(
        uri.clone(),
        format!("{source}\\label{{missing}}\\label{{other}}\\bibitem{{absent}}"),
        2,
    );
    assert!(!ws
        .diagnostics(&uri)
        .iter()
        .any(|diag| diag.message.starts_with("Undefined")));
}

#[test]
fn static_diagnostics_can_be_disabled_and_offer_existing_spelling_fixes() {
    let mut ws = Workspace::default();
    let uri = Url::parse("file:///unsaved/main.tex").unwrap();
    ws.open(
        uri.clone(),
        "\\label{introduction}\\ref{introdution}".into(),
        1,
    );
    ws.code_action_literals = true;
    let diagnostics = ws.diagnostics(&uri);
    let diagnostic = diagnostics
        .iter()
        .find(|diag| diag.message.starts_with("Undefined"))
        .unwrap();
    let actions = editing::code_actions(
        &ws,
        &uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            ..Default::default()
        },
    );
    assert!(!actions.is_empty(), "{actions:?}");
    ws.config.diagnostics.undefined_references = false;
    assert!(ws.diagnostics(&uri).is_empty());
}
