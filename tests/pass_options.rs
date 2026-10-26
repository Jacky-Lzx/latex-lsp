use latex_lsp::{
    compilation::build,
    editing::file_operations,
    language::{features, navigation, signature},
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;
use std::path::Path;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
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
fn forwarded_options_link_the_second_argument_without_loading_a_package_or_class() {
    for (command, extension) in [
        ("PassOptionsToPackage", "sty"),
        ("PassOptionsToClass", "cls"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("settings")).unwrap();
        let source=format!("\\documentclass{{article}}\n😀 \\{command} % keep\r\n{{draft,key={{local}},code=\\custom}} % target\r\n{{settings/local}}\r\n\\begin{{document}}Hello.\\end{{document}}\n");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        std::fs::write(
            root.join(format!("settings/local.{extension}")),
            r"\newcommand{\hidden}{Hidden}",
        )
        .unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = uri(root, "main.tex");
        let target = uri(root, &format!("settings/local.{extension}"));
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(links[0].target, Some(target.clone()));
        let start = source.find("settings/local").unwrap();
        assert_eq!(
            links[0].range,
            Text::new(source.clone()).range(start, start + 14)
        );
        assert!(workspace.dependencies(&main).is_empty());
        assert!(!workspace.project_files(&main).contains(&target));
        let changes = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: target.to_string(),
                new_uri: uri(root, &format!("settings/renamed.{extension}")).to_string(),
            }],
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&main].len(), 1);
        assert_eq!(
            apply(&source, &changes[&main]),
            source.replace("{settings/local}", "{settings/renamed}")
        );
    }
}

#[test]
fn package_and_class_file_renames_update_option_forwarding_and_actual_loaders_together() {
    for (command, loader, extension) in [
        ("PassOptionsToPackage", "usepackage", "sty"),
        ("PassOptionsToClass", "documentclass", "cls"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!(
            "\\{command}{{draft,key={{local}},code=\\custom}}{{local}}\n\\{loader}{{local}}\n"
        );
        std::fs::write(root.join("main.tex"), &source).unwrap();
        std::fs::write(root.join(format!("local.{extension}")), "").unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = uri(root, "main.tex");
        let old = uri(root, &format!("local.{extension}"));
        let changes = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: old.to_string(),
                new_uri: uri(root, &format!("renamed.{extension}")).to_string(),
            }],
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert_eq!(changes[&main].len(), 2);
        assert_eq!(
            apply(&source, &changes[&main]),
            source.replace("}{local}", "}{renamed}").replace(
                &format!("\\{loader}{{local}}"),
                &format!("\\{loader}{{renamed}}")
            )
        );
        assert!(file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: old.to_string(),
                new_uri: uri(root, "renamed.txt").to_string()
            }]
        )
        .unwrap_err()
        .message
        .contains(&format!(".{extension}")));
    }
}

#[test]
fn forwarding_completions_target_only_the_filename_and_preserve_options_and_suffix_rules() {
    for (command, extension) in [
        ("PassOptionsToPackage", "sty"),
        ("PassOptionsToClass", "cls"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for name in [
            "main.tex".into(),
            format!("local.{extension}"),
            format!("local.{extension}.{extension}"),
            format!("unusable space.{extension}"),
        ] {
            std::fs::write(root.join(name), "").unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = uri(root, "main.tex");
        for (marked, label, expected) in [
            (
                format!("\\{command}{{draft,key={{local}}}}{{lo¦cal}}"),
                format!("local.{extension}"),
                format!("\\{command}{{draft,key={{local}}}}{{local}}"),
            ),
            (
                format!("😀 \\{command} % keep\r\n{{draft}} % target\r\n{{lo¦cal.{extension}}}"),
                format!("local.{extension}.{extension}"),
                format!("😀 \\{command} % keep\r\n{{draft}} % target\r\n{{local.{extension}}}"),
            ),
            (
                format!("\\{command}{{draft}}{{lo¦"),
                format!("local.{extension}"),
                format!("\\{command}{{draft}}{{local"),
            ),
        ] {
            let offset = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            workspace.open(main.clone(), source.clone(), 1);
            let position = Text::new(source.clone()).position(offset);
            let items = features::completion(&workspace, &main, position);
            let item = items
                .iter()
                .find(|item| item.label == label)
                .unwrap_or_else(|| panic!("{marked}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!()
            };
            assert_eq!(edit.range.start.line, edit.range.end.line);
            assert!(edit.range.start <= position && position <= edit.range.end);
            assert_eq!(apply(&source, std::slice::from_ref(edit)), expected);
            assert!(!items.iter().any(|item| item.label.starts_with("unusable")));
        }
        let marked = format!("\\{command}{{lo¦cal}}{{local}}");
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 2);
        assert!(
            features::completion(&workspace, &main, Text::new(source).position(offset))
                .iter()
                .all(|item| item.kind != Some(CompletionItemKind::FILE))
        );
    }
}

#[test]
fn uncertain_option_targets_do_not_trim_spaces_expand_macros_or_treat_package_names_as_lists() {
    for command in ["PassOptionsToPackage", "PassOptionsToClass"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for name in ["local.sty", "local.cls", "other.sty", "other.cls"] {
            std::fs::write(root.join(name), "").unwrap();
        }
        for target in [
            "local ",
            " local",
            "lo cal",
            "local\n",
            r"\dynamic",
            "local,other",
        ] {
            let source = format!("\\{command}{{draft}}{{{target}}}");
            std::fs::write(root.join("main.tex"), &source).unwrap();
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let main = uri(root, "main.tex");
            assert!(
                navigation::document_links(&workspace, &main).is_empty(),
                "{source}"
            );
            workspace.open(main.clone(), source.clone(), 1);
            let position = Text::new(source.clone()).position(source.rfind('}').unwrap());
            assert!(
                features::completion(&workspace, &main, position)
                    .iter()
                    .all(|item| item.kind != Some(CompletionItemKind::FILE)),
                "{source}"
            );
        }
        let source=format!("% \\{command}{{draft}}{{local}}\n\\verb|\\{command}{{draft}}{{local}}|\n\\{command}*{{draft}}{{local}}\n\\{command}[bad]{{draft}}{{local}}\n");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        assert!(navigation::document_links(&workspace, &uri(root, "main.tex")).is_empty());
    }
}

#[test]
fn forwarding_commands_offer_two_required_arguments_and_explain_loading_order() {
    let main = Url::parse("file:///pass-options/main.tex").unwrap();
    for command in ["PassOptionsToPackage", "PassOptionsToClass"] {
        let mut workspace = Workspace {
            client: latex_lsp::project::state::ClientCapabilities {
                snippets: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let source = r"\PassOptionsTo";
        workspace.open(main.clone(), source.into(), 1);
        let items = features::completion(&workspace, &main, Position::new(0, source.len() as u32));
        let item = items.iter().find(|item| item.label == command).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(
            edit.new_text,
            format!("{command}{{${{1:options}}}}{{${{2:name}}}}")
        );
        for (marked, active) in [
            (format!("\\{command}{{dra¦ft}}{{local}}"), 0),
            (format!("\\{command}{{draft}}{{lo¦cal}}"), 1),
        ] {
            let at = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            workspace.open(main.clone(), source.clone(), 2);
            let help = signature::help(&workspace, &main, Text::new(source).position(at)).unwrap();
            assert_eq!(
                help.signatures[0].label,
                format!("\\{command}{{options}}{{name}}")
            );
            assert_eq!(help.active_parameter, Some(active));
        }
    }
}

#[test]
fn shared_option_forwarding_rejects_inconsistent_file_renames_across_projects() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("shared.tex"),
        r"\PassOptionsToPackage{draft}{local}",
    )
    .unwrap();
    for project in ["a", "b"] {
        std::fs::create_dir(root.join(project)).unwrap();
        std::fs::write(root.join(project).join("local.sty"), "").unwrap();
        std::fs::write(root.join(project).join("main.tex"), "\\documentclass{article}\n\\input{../shared}\n\\usepackage{local}\n\\begin{document}Hello.\\end{document}\n").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert!(navigation::document_links(&workspace, &uri(root, "shared.tex")).is_empty());
    let error = file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: uri(root, "a/local.sty").to_string(),
            new_uri: uri(root, "a/renamed.sty").to_string(),
        }],
    )
    .unwrap_err();
    assert!(error.message.contains("Shared file reference"), "{error}");
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_package_and_class_renames_preserve_forwarded_options_in_main_and_support_sources() {
    for engine in ["pdflatex", "xelatex"] {
        for (kind, extension, loader, support) in [
            ("Package", "sty", "usepackage", false),
            ("Package", "sty", "RequirePackage", true),
            ("Class", "cls", "documentclass", false),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let package=format!("\\DeclareOption{{draft}}{{\\typeout{{FORWARD-RENAME:APPLIED}}}}\n\\ProcessOptions\\relax\n{}",if kind=="Class" {"\\LoadClass{article}\n"} else {""});
            std::fs::write(root.join(format!("local.{extension}")), package).unwrap();
            let loads=format!("\\PassOptionsTo{kind} % options\r\n{{draft}} % target\r\n{{local}}\r\n\\{loader}{{local}}\n");
            std::fs::write(root.join("wrapper.sty"), &loads).unwrap();
            let source = if support {
                "\\documentclass{article}\n\\usepackage{wrapper}\n\\begin{document}Hello.\\end{document}\n".into()
            } else {
                format!(
                    "{}{loads}\\begin{{document}}Hello.\\end{{document}}\n",
                    if kind == "Package" {
                        "\\documentclass{article}\n"
                    } else {
                        ""
                    }
                )
            };
            std::fs::write(root.join("main.tex"), &source).unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            for renamed in [false, true] {
                let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
                assert_eq!(outcome.status, "success", "{}", outcome.output);
                assert!(
                    outcome.output.contains("FORWARD-RENAME:APPLIED"),
                    "{}",
                    outcome.output
                );
                assert!(outcome.inputs.unwrap().contains(&uri(
                    root,
                    &format!("{}.{extension}", if renamed { "renamed" } else { "local" })
                )));
                if !renamed {
                    let mut workspace = Workspace::default();
                    workspace.add_root(root.into());
                    let changes = file_operations::will_rename(
                        &workspace,
                        &[FileRename {
                            old_uri: uri(root, &format!("local.{extension}")).to_string(),
                            new_uri: uri(root, &format!("renamed.{extension}")).to_string(),
                        }],
                    )
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                    let owner = uri(root, if support { "wrapper.sty" } else { "main.tex" });
                    assert_eq!(changes[&owner].len(), 2, "{changes:?}");
                    for (uri, edits) in changes {
                        std::fs::write(
                            uri.to_file_path().unwrap(),
                            apply(&workspace.documents[&uri].text.source, &edits),
                        )
                        .unwrap();
                    }
                    std::fs::rename(
                        root.join(format!("local.{extension}")),
                        root.join(format!("renamed.{extension}")),
                    )
                    .unwrap();
                }
            }
        }
    }
}
