use latex_lsp::{build, features, text::Text, workspace::Workspace};
use std::{path::Path, sync::atomic::AtomicBool};
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn edited(text: &Text, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    text.changed(
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
fn verbatim_environment_detection_requires_the_actual_name_including_spaces() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\begin{ verbatim }\n\\newcommand{\\visible}{Text}\n\\end{ verbatim }\n\\begin{verbatim}\n\\newcommand{\\hidden}{Text}\n\\end{verbatim}\n";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let doc = &workspace.documents[&main];
    assert!(doc
        .syntax
        .definitions
        .iter()
        .any(|entry| entry.name == "visible"));
    assert!(!doc
        .syntax
        .definitions
        .iter()
        .any(|entry| entry.name == "hidden"));
    assert!(
        doc.syntax.diagnostics.is_empty(),
        "{:?}",
        doc.syntax.diagnostics
    );
}

#[test]
fn generated_environment_commands_resolve_and_rename_from_every_spelling() {
    for declaration in ["newenvironment", "NewDocumentEnvironment", "NewEnviron"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions =
            format!("\\{declaration}{{panel}}{{}}{{}}\n\\renewcommand{{\\endpanel}}{{Done}}\n");
        let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n😀 \\panel Body.\\endpanel\n\\begin{panel}Body.\\end{panel}\n\\csname panel\\endcsname Body.\\csname endpanel\\endcsname\n\\thepanel \\value{panel}\n% \\panel \\csname endpanel\\endcsname\n\\verb|\\panel|\n\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", &definitions)],
        );
        let main = uri(root, "main.tex");
        let text = &workspace.documents[&main].text;
        for (marker, local) in [
            ("\\panel Body", 2),
            ("\\endpanel", 2),
            ("\\begin{panel}", 9),
            ("\\csname panel", 10),
            ("\\csname endpanel", 10),
        ] {
            let position = text.position(source.find(marker).unwrap() + local);
            let locations = features::definitions(&workspace, &main, position);
            assert_eq!(locations.len(), 1, "{declaration}: {marker}");
            assert_eq!(locations[0].uri, uri(root, "definitions.tex"));
            assert!(features::hover(&workspace, &main, position).is_some());
            let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
                features::prepare_rename(&workspace, &main, position).unwrap()
            else {
                panic!("missing prepared rename: {marker}");
            };
            assert_eq!(placeholder, "panel");
            assert_eq!(
                &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
                "panel"
            );
            assert_eq!(
                features::references(&workspace, &main, position, true).len(),
                8
            );
            let changes = features::rename(&workspace, &main, position, "remarkbox")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(
                edited(text, changes[&main].clone()),
                source
                    .replace(
                        "😀 \\panel Body.\\endpanel",
                        "😀 \\remarkbox Body.\\endremarkbox"
                    )
                    .replace(
                        "\\begin{panel}Body.\\end{panel}",
                        "\\begin{remarkbox}Body.\\end{remarkbox}"
                    )
                    .replace(
                        "\\csname panel\\endcsname Body.\\csname endpanel\\endcsname",
                        "\\csname remarkbox\\endcsname Body.\\csname endremarkbox\\endcsname"
                    )
            );
            let definitions_uri = uri(root, "definitions.tex");
            assert_eq!(
                edited(
                    &workspace.documents[&definitions_uri].text,
                    changes[&definitions_uri].clone()
                ),
                definitions.replace("panel", "remarkbox")
            );
            assert!(features::rename(&workspace, &main, position, "remark-box").is_err());
        }
    }
}

#[test]
fn literal_environment_names_allow_nonletters_without_direct_control_words() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\newenvironment{panel}{}{}\n\\begin{panel}Text.\\end{panel}\n\\csname panel\\endcsname Text.\\csname endpanel\\endcsname";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let text = &workspace.documents[&main].text;
    let changes = features::rename(
        &workspace,
        &main,
        text.position(source.find("panel").unwrap()),
        "panel-two",
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(
        edited(text, changes[&main].clone()),
        source.replace("panel", "panel-two")
    );
}

#[test]
fn shared_direct_commands_cannot_rename_another_projects_macro() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root, &[
        ("a.tex", "\\documentclass{article}\n\\newenvironment{panel}{}{}\n\\begin{document}\\input{shared}\\end{document}"),
        ("b.tex", "\\documentclass{article}\n\\newcommand{\\panel}{B}\n\\newcommand{\\endpanel}{}\n\\begin{document}\\input{shared}\\end{document}"),
        ("shared.tex", "\\panel Text.\\endpanel"),
    ]);
    let a = uri(root, "a.tex");
    let text = &workspace.documents[&a].text;
    let position = text.position(text.source.find("panel").unwrap());
    assert!(features::rename(&workspace, &a, position, "remarkbox").is_err());
    assert!(features::prepare_rename(&workspace, &a, position).is_err());
}

#[test]
#[ignore = "requires pdflatex"]
fn renamed_direct_and_csname_environment_calls_compile_with_preserved_arguments() {
    for declaration in [
        "\\newenvironment{panel}[2][Default]{\\begin{quote}#1: #2.}{\\end{quote}}",
        "\\NewDocumentEnvironment{panel}{O{Default}m}{\\begin{quote}#1: #2.}{\\end{quote}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main_source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\panel[Direct]{Title}Body.\\endpanel\n\\begin{panel}{Grouped}Body.\\end{panel}\n\\csname panel\\endcsname[Literal]{Title}Body.\\csname endpanel\\endcsname\n\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", main_source), ("definitions.tex", declaration)],
        );
        let main = uri(root, "main.tex");
        let text = &workspace.documents[&main].text;
        let changes = features::rename(
            &workspace,
            &main,
            text.position(main_source.find("\\endpanel").unwrap() + 2),
            "remarkbox",
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        for (document_uri, edits) in changes {
            std::fs::write(
                document_uri.to_file_path().unwrap(),
                edited(&workspace.documents[&document_uri].text, edits),
            )
            .unwrap();
        }
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let outcome = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status, "success",
            "{}\n{}",
            outcome.message, outcome.output
        );
        assert_eq!(outcome.pages, Some(1));
    }
}
