use latex_lsp::{features, text::Text, workspace::Workspace};
use lsp_types::*;
#[test]
fn external_labels_complete_navigate_and_hover_without_polluting_source_symbols() {
    let dir = tempfile::tempdir().unwrap();
    let source = "\\documentclass{article}\n\\externaldocument[ext-]{other}\n\\ref{ext-section}\n";
    std::fs::write(dir.path().join("main.tex"), source).unwrap();
    std::fs::write(
        dir.path().join("other.tex"),
        "\\label{section}\\newcommand{\\private}{hidden}",
    )
    .unwrap();
    std::fs::write(dir.path().join("other.aux"),"\\relax\n\\newlabel{section}{{2.3}{7}{External title}{section.2}{}}\n\\@input{child.aux}\n\\@writefile{toc}{\\newlabel{fake}{{1}{1}}}\n").unwrap();
    std::fs::write(
        dir.path().join("child.aux"),
        "\\newlabel{child}{{3}{9}{Child title}{}}\n\\@input{other.aux}\n",
    )
    .unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.open(uri.clone(), source.into(), 1);
    let text = Text::new(source.into());
    let position = text.position(source.find("ext-section").unwrap() + 5);
    assert!(!ws
        .diagnostics(&uri)
        .iter()
        .any(|diagnostic| diagnostic.message.starts_with("Undefined")));
    let definitions = features::definitions(&ws, &uri, position);
    assert_eq!(definitions.len(), 1);
    assert!(definitions[0].uri.path().ends_with("other.tex"));
    assert_eq!(definitions[0].range.start.character, 7);
    let hover = format!("{:?}", features::hover(&ws, &uri, position));
    assert!(hover.contains("2.3"));
    assert!(hover.contains("External title"));
    let items = features::completion(&ws, &uri, position);
    assert!(items.iter().any(|item| item.label == "ext-section"));
    assert!(!ws
        .project_documents(&uri)
        .iter()
        .any(|uri| uri.path().ends_with("other.tex")));
    assert!(features::prepare_rename(&ws, &uri, position)
        .unwrap()
        .is_none());
    let changed = source.replace("ext-section", "ext-child");
    ws.open(uri.clone(), changed, 2);
    assert!(!ws
        .diagnostics(&uri)
        .iter()
        .any(|diag| diag.message.starts_with("Undefined")));
    // Auxiliary cache follows file changes; stale local aux labels do not define source labels.
    std::fs::write(
        dir.path().join("other.aux"),
        "\\newlabel{section}{{4}{8}{Changed}{}}\n",
    )
    .unwrap();
    ws.open(uri.clone(), source.into(), 3);
    assert!(format!("{:?}", features::hover(&ws, &uri, position)).contains("Changed"));
}
#[test]
fn local_aux_values_respect_build_output_job_name_and_do_not_hide_deleted_labels() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("out")).unwrap();
    let source = "\\documentclass{article}\\ref{removed}";
    std::fs::write(dir.path().join("main.tex"), source).unwrap();
    std::fs::write(
        dir.path().join("out/custom.aux"),
        "\\newlabel{removed}{{1}{2}{Old title}{}}\n",
    )
    .unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.build.output_directory = Some("out".into());
    ws.config.build.job_name = Some("custom".into());
    ws.open(uri.clone(), source.into(), 1);
    let at = Text::new(source.into()).position(source.find("removed").unwrap() + 2);
    assert!(format!("{:?}", features::hover(&ws, &uri, at)).contains("Old title"));
    assert!(ws
        .diagnostics(&uri)
        .iter()
        .any(|diag| diag.message == "Undefined reference `removed`"));
}
