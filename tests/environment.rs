use latex_lsp::{build, features, text::Text, workspace::Workspace};
use lsp_types::*;
use std::{path::Path, sync::atomic::AtomicBool};

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
fn environment_copies_keep_targets_sources_and_generated_commands_independent() {
    for command in [
        "NewEnvironmentCopy",
        "RenewEnvironmentCopy",
        "DeclareEnvironmentCopy",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!("\\newenvironment{{originalpanel}}{{}}{{}}\\{command}{{panelcopy}}% keep\r\n{{originalpanel}}\n");
        let body = "\\begin{panelcopy}A\\end{panelcopy}\\panelcopy B\\endpanelcopy\\csname panelcopy\\endcsname C\\csname endpanelcopy\\endcsname\\begin{originalpanel}D\\end{originalpanel}% keep \\panelcopy \\originalpanel\n\\verb|\\panelcopy \\originalpanel|\n";
        let main_source = format!("\\documentclass{{article}}\\input{{definitions}}\\begin{{document}}{body}\\end{{document}}\n");
        let workspace = project(
            root,
            &[
                ("main.tex", &main_source),
                ("definitions.tex", &definitions),
            ],
        );
        let main = uri(root, "main.tex");
        let declaration_uri = uri(root, "definitions.tex");
        for (old, new, references) in [
            ("panelcopy", "copyrenamed", 7),
            ("originalpanel", "sourcerenamed", 4),
        ] {
            let doc = &workspace.documents[&main];
            let position = doc
                .text
                .position(main_source.find(&format!("{{{old}}}")).unwrap() + 2);
            assert_eq!(features::definitions(&workspace, &main, position).len(), 1);
            assert_eq!(
                features::references(&workspace, &main, position, true).len(),
                references
            );
            let changes = features::rename(&workspace, &main, position, new)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 2);
            assert_eq!(
                edited(
                    &workspace.documents[&declaration_uri].text,
                    changes[&declaration_uri].clone()
                ),
                definitions.replace(old, new)
            );
            let (code, ignored) = main_source.split_once("% keep").unwrap();
            assert_eq!(
                edited(&doc.text, changes[&main].clone()),
                format!("{}% keep{ignored}", code.replace(old, new))
            );
        }
        let doc = &workspace.documents[&declaration_uri];
        let start = definitions.rfind("originalpanel").unwrap();
        let position = doc.text.position(start + 6);
        assert_eq!(
            features::definitions(&workspace, &declaration_uri, position).len(),
            1
        );
        assert_eq!(
            features::references(&workspace, &declaration_uri, position, true).len(),
            4
        );
        let items = features::completion(&workspace, &declaration_uri, position);
        let item = items
            .iter()
            .find(|item| item.label == "originalpanel")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing source completion")
        };
        assert_eq!(
            edit.range,
            doc.text.range(start, start + "originalpanel".len())
        );
        assert_eq!(edit.new_text, "originalpanel");
    }
}

#[test]
fn shared_environment_copies_can_rename_the_copy_but_reject_project_specific_source_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\\newenvironment{sourcepanel}{}{}\\input{shared}\\begin{document}\\begin{panelcopy}Text.\\end{panelcopy}\\end{document}";
    let shared_source = "\\NewEnvironmentCopy{panelcopy}{sourcepanel}";
    let workspace = project(
        root,
        &[
            ("a.tex", source),
            ("b.tex", source),
            ("shared.tex", shared_source),
        ],
    );
    let first = uri(root, "a.tex");
    let second = uri(root, "b.tex");
    let shared = uri(root, "shared.tex");
    let at_source = workspace.documents[&first]
        .text
        .position(source.find("sourcepanel").unwrap() + 2);
    assert!(features::rename(&workspace, &first, at_source, "renamedsource").is_err());
    let at_shared_source = workspace.documents[&shared]
        .text
        .position(shared_source.find("sourcepanel").unwrap() + 2);
    assert!(features::rename(&workspace, &shared, at_shared_source, "renamedsource").is_err());
    let at_copy = workspace.documents[&first]
        .text
        .position(source.find("panelcopy").unwrap() + 2);
    let changes = features::rename(&workspace, &first, at_copy, "renamedcopy")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 3);
    for main in [&first, &second] {
        assert_eq!(
            edited(&workspace.documents[main].text, changes[main].clone()),
            source.replace("panelcopy", "renamedcopy")
        );
    }
    assert_eq!(
        edited(&workspace.documents[&shared].text, changes[&shared].clone()),
        "\\NewEnvironmentCopy{renamedcopy}{sourcepanel}"
    );
}

#[test]
fn copied_source_arguments_preserve_semantic_spaces_dynamic_names_and_label_namespaces() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\\newenvironment{originalpanel}{}{}\\label{originalpanel}\\NewEnvironmentCopy{panelcopy}{ originalpanel }\\DeclareEnvironmentCopy{othercopy}{\\prefix originalpanel}\\begin{originalpanel}Text.\\end{originalpanel}\\ref{originalpanel}";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let doc = &workspace.documents[&main];
    let position = doc.text.position(source.find("originalpanel").unwrap() + 2);
    let changes = features::rename(&workspace, &main, position, "changedpanel")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    let changed = edited(&doc.text, changes[&main].clone());
    assert_eq!(
        changed,
        source
            .replace(
                "newenvironment{originalpanel}",
                "newenvironment{changedpanel}"
            )
            .replace("begin{originalpanel}", "begin{changedpanel}")
            .replace("end{originalpanel}", "end{changedpanel}")
    );
    for marker in [" originalpanel ", "\\prefix originalpanel"] {
        let at = doc
            .text
            .position(source.find(marker).unwrap() + marker.len() - 2);
        assert!(features::definitions(&workspace, &main, at).is_empty());
    }
}

#[test]
fn copied_source_completion_accepts_incomplete_and_multiline_names_without_editing_the_target() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(
        root,
        &[
            ("main.tex", "\\documentclass{article}\\input{definitions}"),
            ("definitions.tex", "\\newenvironment{originalpanel}{}{}"),
        ],
    );
    let main = uri(root, "main.tex");
    for marked in [
        "\\documentclass{article}\\input{definitions}\\NewEnvironmentCopy{copy}{orig¦",
        "\\documentclass{article}\\input{definitions}\\DeclareEnvironmentCopy{copy} % keep\r\n{\r\n orig¦inalpanel\r\n}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 2);
        let doc = &workspace.documents[&main];
        let items = features::completion(&workspace, &main, doc.text.position(offset));
        let item = items.iter().find(|item| item.label == "originalpanel").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else { panic!("missing source completion") };
        assert_eq!(edit.range.start.line, edit.range.end.line);
        let mut edits = vec![edit.clone()];
        edits.extend(item.additional_text_edits.clone().unwrap_or_default());
        let changed = edited(&doc.text, edits);
        assert!(changed.contains("Copy{copy}"));
        assert!(changed.ends_with("{originalpanel}") || changed.ends_with("{originalpanel"), "{changed}");
    }
}

#[test]
fn copied_builtin_sources_have_hover_and_references_without_a_local_declaration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\NewEnvironmentCopy{customquote}{quote}\\begin{quote}Text.\\end{quote}";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let position = workspace.documents[&main]
        .text
        .position(source.find("{quote}").unwrap() + 2);
    assert!(features::hover(&workspace, &main, position).is_some());
    assert_eq!(
        features::references(&workspace, &main, position, false).len(),
        3
    );
    assert!(features::prepare_rename(&workspace, &main, position)
        .unwrap()
        .is_none());
}

#[test]
#[ignore = "requires pdflatex with environment copy support"]
fn real_environment_copy_renames_preserve_both_commands_and_defaults_after_source_redefinition() {
    for copy in ["NewEnvironmentCopy", "DeclareEnvironmentCopy"] {
        for declaration in [
            "\\newenvironment{originalpanel}[1][blue]{\\typeout{ENV-COPY:#1}\\begin{quote}}{\\end{quote}}",
            "\\NewDocumentEnvironment{originalpanel}{O{blue}}{\\typeout{ENV-COPY:#1}\\begin{quote}}{\\end{quote}}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let definitions = format!("{declaration}\\{copy}{{panelcopy}}{{originalpanel}}% \\panelcopy\n\\renewenvironment{{originalpanel}}{{Changed.}}{{}}\n");
            let body = "\\begin{panelcopy}Default.\\end{panelcopy}\\begin{panelcopy}[red]Explicit.\\end{panelcopy}\\panelcopy Literal.\\endpanelcopy\\csname panelcopy\\endcsname Named.\\csname endpanelcopy\\endcsname\n% \\panelcopy\n\\verb|\\panelcopy|\n";
            let workspace = project(root, &[("main.tex", "\\documentclass{article}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n"), ("definitions.tex", &definitions), ("content.tex", body)]);
            let main = root.join("main.tex");
            let config = latex_lsp::config::BuildConfig { executable: "pdflatex".into(), ..Default::default() };
            let before = build::run(&main, &config, &Default::default());
            assert_eq!(before.status, "success", "{copy}: {}", before.output);
            let child = uri(root, "content.tex");
            let position = workspace.documents[&child].text.position(body.find("panelcopy").unwrap() + 2);
            let changes = features::rename(&workspace, &child, position, "renamedpanel").unwrap().unwrap().changes.unwrap();
            assert_eq!(changes.len(), 2);
            for (uri, edits) in changes {
                std::fs::write(uri.to_file_path().unwrap(), edited(&workspace.documents[&uri].text, edits)).unwrap();
            }
            assert_eq!(std::fs::read_to_string(root.join("definitions.tex")).unwrap(), definitions.replacen("{panelcopy}", "{renamedpanel}", 1));
            assert!(std::fs::read_to_string(root.join("content.tex")).unwrap().ends_with("% \\panelcopy\n\\verb|\\panelcopy|\n"));
            let after = build::run(&main, &config, &Default::default());
            assert_eq!(after.status, "success", "{copy}: {}", after.output);
            for outcome in [before, after] {
                let observed: Vec<_> = outcome.output.lines().filter(|line| line.starts_with("ENV-COPY:")).collect();
                assert_eq!(observed, ["ENV-COPY:blue", "ENV-COPY:red", "ENV-COPY:blue", "ENV-COPY:blue"]);
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex with environment copy support"]
fn real_copied_theorems_keep_the_original_counter_when_either_environment_is_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    project(root, &[
        ("main.tex", "\\documentclass{article}\\input{definitions}\\begin{document}\\input{content}\\end{document}\n"),
        ("definitions.tex", "\\newtheorem{originalpanel}{Original}\\NewEnvironmentCopy{panelcopy}{originalpanel}\n"),
        ("content.tex", "\\begin{originalpanel}First.\\end{originalpanel}\\begin{panelcopy}Second.\\end{panelcopy}\\panelcopy Third.\\endpanelcopy\\typeout{ENV-COUNT:\\arabic{originalpanel}}\n"),
    ]);
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    for step in [
        None,
        Some(("panelcopy", "renamedcopy")),
        Some(("originalpanel", "renamedoriginal")),
    ] {
        if let Some((old, new)) = step {
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let child = uri(root, "content.tex");
            let doc = &workspace.documents[&child];
            let offset = if old == "originalpanel" {
                doc.text.source.rfind(old).unwrap() + 2
            } else {
                doc.text.source.find(old).unwrap() + 2
            };
            let changes = features::rename(&workspace, &child, doc.text.position(offset), new)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            for (uri, edits) in changes {
                std::fs::write(
                    uri.to_file_path().unwrap(),
                    edited(&workspace.documents[&uri].text, edits),
                )
                .unwrap();
            }
        }
        let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        assert!(
            outcome.output.lines().any(|line| line == "ENV-COUNT:3"),
            "{}",
            outcome.output
        );
    }
    assert_eq!(std::fs::read_to_string(root.join("definitions.tex")).unwrap(), "\\newtheorem{renamedoriginal}{Original}\\NewEnvironmentCopy{renamedcopy}{renamedoriginal}\n");
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
