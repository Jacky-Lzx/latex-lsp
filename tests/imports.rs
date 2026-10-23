use latex_lsp::{features, navigation, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;
#[test]
fn nested_imports_use_scoped_directories_for_dependencies_navigation_and_completion() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("parts/sub")).unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let a = Url::from_file_path(dir.path().join("parts/a.tex")).unwrap();
    let child = Url::from_file_path(dir.path().join("parts/sub/b.tex")).unwrap();
    std::fs::write(
        dir.path().join("main.tex"),
        "\\documentclass{article}\\import{parts/}{a}",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("parts/a.tex"),
        "\\input{local}\\subimport{sub/}{b}",
    )
    .unwrap();
    std::fs::write(dir.path().join("parts/local.tex"), "\\label{imported}").unwrap();
    std::fs::write(dir.path().join("local.tex"), "\\label{wrong-root}").unwrap();
    std::fs::write(dir.path().join("parts/sub/b.tex"), "\\label{nested}").unwrap();
    let mut ws = Workspace::default();
    ws.open(
        main.clone(),
        std::fs::read_to_string(main.to_file_path().unwrap()).unwrap(),
        1,
    );
    let project = ws.project_documents(&main);
    assert!(project.contains(&a));
    assert!(project.contains(&child));
    assert!(project.contains(&Url::from_file_path(dir.path().join("parts/local.tex")).unwrap()));
    assert!(!project.contains(&Url::from_file_path(dir.path().join("local.tex")).unwrap()));
    let links = navigation::document_links(&ws, &a);
    assert!(links
        .iter()
        .any(|link| link.target.as_ref() == Some(&child)));
    assert!(links.iter().any(|link| link
        .target
        .as_ref()
        .is_some_and(|url| url.path().ends_with("parts/local.tex"))));
    let source = "😀 \\documentclass{article}\\import{parts/}{a}";
    ws.open(main.clone(), source.into(), 2);
    let at = Text::new(source.into()).position(source.len() - 1);
    let items = features::completion(&ws, &main, at);
    assert!(items.iter().any(|item| item.label == "a.tex"));
}
#[test]
fn quoted_or_deferred_imports_do_not_load_project_symbols() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("child.tex"), "\\label{bad}").unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let source="\\documentclass{article}\\newcommand{\\deferred}{\\import{./}{child}}\\detokenize{\\import{./}{child}}";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    let mut ws = Workspace::default();
    ws.open(uri.clone(), source.into(), 1);
    assert_eq!(ws.project_documents(&uri).len(), 1);
}
