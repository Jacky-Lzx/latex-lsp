use latex_lsp::{
    config::{Engine, Formatter},
    workspace::Workspace,
};
use tower_lsp::lsp_types::*;

#[test]
fn folder_settings_inherit_and_root_overrides_do_not_leak() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    std::fs::create_dir(a.path().join("chapters")).unwrap();
    std::fs::write(
        a.path().join("book.tex"),
        "\\documentclass{article}\\input{chapters/one}",
    )
    .unwrap();
    std::fs::write(a.path().join("slides.tex"), "\\documentclass{beamer}").unwrap();
    std::fs::write(a.path().join("chapters/one.tex"), "\\ref{missing}").unwrap();
    std::fs::write(a.path().join(".latex-lsp.json"), r#"{"formatting":{"formatter":"latexindent"},"projects":{"book.tex":{"formatting":{"formatter":"tex-fmt"}},"slides.tex":{"build":{"engine":"xelatex"}}}}"#).unwrap();
    std::fs::write(
        a.path().join("chapters/.latex-lsp.json"),
        r#"{"formatting":{"timeoutSeconds":3},"diagnostics":{"undefinedReferences":false}}"#,
    )
    .unwrap();
    let mut ws = Workspace::default();
    ws.add_root(a.path().into());
    ws.add_root(b.path().into());
    let book = Url::from_file_path(a.path().join("book.tex")).unwrap();
    let slides = Url::from_file_path(a.path().join("slides.tex")).unwrap();
    let child = Url::from_file_path(a.path().join("chapters/one.tex")).unwrap();
    let other = Url::from_file_path(b.path().join("main.tex")).unwrap();
    assert_eq!(ws.settings(&book).formatting.formatter, Formatter::TexFmt);
    assert_eq!(
        ws.settings(&slides).formatting.formatter,
        Formatter::Latexindent
    );
    assert_eq!(ws.build_config(&slides).engine, Some(Engine::XeLatex));
    assert_eq!(ws.build_config(&book).engine, None);
    assert_eq!(ws.settings(&child).formatting.formatter, Formatter::TexFmt);
    assert_eq!(ws.settings(&child).formatting.timeout_seconds, 3);
    assert!(ws.diagnostics(&child).is_empty());
    assert_eq!(ws.settings(&other).formatting.formatter, Formatter::TexFmt);
}

#[test]
fn project_root_file_is_relative_to_configuration_and_invalid_settings_are_visible() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    std::fs::write(dir.path().join("main.tex"), "\\documentclass{article}").unwrap();
    std::fs::write(dir.path().join("nested/chapter.tex"), "text").unwrap();
    std::fs::write(
        dir.path().join("nested/.latex-lsp.json"),
        r#"{"build":{"rootFile":"../main.tex"}}"#,
    )
    .unwrap();
    let mut ws = Workspace::default();
    ws.add_root(dir.path().into());
    let uri = Url::from_file_path(dir.path().join("nested/chapter.tex")).unwrap();
    assert_eq!(
        ws.root_document(&uri).unwrap(),
        Url::from_file_path(dir.path().join("main.tex")).unwrap()
    );
    std::fs::write(
        dir.path().join("nested/.latex-lsp.json"),
        r#"{"formatting":{"unknown":true}}"#,
    )
    .unwrap();
    assert!(ws
        .diagnostics(&uri)
        .iter()
        .any(|diag| diag.code == Some(NumberOrString::String("project-configuration".into()))));
}

#[test]
fn independent_unsaved_main_documents_do_not_share_symbols() {
    let mut ws = Workspace::default();
    let a = Url::parse("file:///unwritten-a/main.tex").unwrap();
    let b = Url::parse("file:///unwritten-b/main.tex").unwrap();
    ws.open(
        a.clone(),
        "\\documentclass{article}\\label{private}".into(),
        1,
    );
    ws.open(
        b.clone(),
        "\\documentclass{article}\\ref{private}".into(),
        1,
    );
    assert!(ws
        .diagnostics(&b)
        .iter()
        .any(|diag| diag.message == "Undefined reference `private`"));
}
