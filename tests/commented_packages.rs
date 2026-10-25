use latex_lsp::{features, file_operations, navigation, text::Text, workspace::Workspace};
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
fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

#[test]
fn commented_package_lists_load_external_sources_and_rename_only_real_filename_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("project")).unwrap();
    std::fs::create_dir(root.join("shared")).unwrap();
    for (name, command) in [
        ("one", "firstword"),
        ("two", "secondword"),
        ("phantom", "unusedword"),
    ] {
        std::fs::write(
            root.join(format!("shared/{name}.sty")),
            format!("\\newcommand{{\\{command}}}{{Text}}\n"),
        )
        .unwrap();
    }
    for command in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
        let options = if command == "RequirePackageWithOptions" {
            ""
        } else {
            "[draft]"
        };
        let source=format!("\\documentclass{{article}}\r\n\\{command}{options}{{% emoji😀, ../shared/phantom, }} ignored\r\n ../shared/o ne% one tail\r\n ,% separator, }} ignored\r\n ../shared/two% two tail\r\n}}\r\n\\begin{{document}}\\firstword \\secondword\\end{{document}}\r\n\\verb|\\{command}{{../shared/one}}|\r\n");
        std::fs::write(root.join("project/main.tex"), &source).unwrap();
        let main = uri(root, "project/main.tex");
        let mut workspace = Workspace::default();
        workspace.add_root(root.join("project"));
        workspace.open(main.clone(), source.clone(), 7);
        let files = workspace.project_files(&main);
        assert!(files.contains(&uri(root, "shared/one.sty")));
        assert!(files.contains(&uri(root, "shared/two.sty")));
        assert!(!files.contains(&uri(root, "shared/phantom.sty")));
        assert!(!workspace
            .documents
            .contains_key(&uri(root, "shared/phantom.sty")));
        let text = Text::new(source.clone());
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 2, "{links:?}");
        for (name, literal, call) in [
            ("one", "../shared/o ne", "firstword"),
            ("two", "../shared/two", "secondword"),
        ] {
            let target = uri(root, &format!("shared/{name}.sty"));
            let link = links
                .iter()
                .find(|link| link.target.as_ref() == Some(&target))
                .unwrap();
            assert_eq!(
                &source
                    [text.offset(link.range.start).unwrap()..text.offset(link.range.end).unwrap()],
                literal
            );
            let found = features::definitions(
                &workspace,
                &main,
                text.position(source.rfind(call).unwrap() + 2),
            );
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].uri, target);
        }
        assert!(file_operations::will_rename(
            &workspace,
            &[rename(root, "shared/phantom.sty", "shared/unused.sty")]
        )
        .unwrap()
        .is_none());
        let edits = file_operations::will_rename(
            &workspace,
            &[rename(root, "shared/one.sty", "shared/renamed.sty")],
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[&main].len(), 1);
        assert_eq!(
            apply(&source, &edits[&main]),
            source.replace("\n ../shared/o ne%", "\n ../shared/renamed%")
        );
        let edits = file_operations::will_rename(&workspace, &[rename(root, "shared", "packages")])
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits[&main].len(), 2);
        assert_eq!(
            apply(&source, &edits[&main]),
            source
                .replace("\n ../shared/o ne%", "\n ../packages/one%")
                .replace("\n ../shared/two%", "\n ../packages/two%")
        );
    }
}

#[test]
fn package_list_completion_preserves_comments_and_obeys_actual_optional_argument_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("local.sty"), "Local.").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for command in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
        for options in ["", "[draft]", "[draft [literal]"] {
            if command == "RequirePackageWithOptions" && !options.is_empty() {
                continue;
            }
            for (arguments, expected) in [
                (
                    "{% before, }\n loc¦BAD% tail\n,other}",
                    "{% before, }\n local% tail\n,other}",
                ),
                (
                    "{other,% split, }\r\nloc¦BAD% tail\r\n}",
                    "{other,% split, }\r\nlocal% tail\r\n}",
                ),
                ("{% before\n¦% after\n}", "{% before\nlocal% after\n}"),
                ("{% before\nlo c¦", "{% before\nlocal"),
            ] {
                let marked = format!("\\{command}{options}{arguments}");
                let source = marked.replace('¦', "");
                workspace.open(main.clone(), source.clone(), 7);
                let items = features::completion(
                    &workspace,
                    &main,
                    Text::new(source.clone()).position(marked.find('¦').unwrap()),
                );
                if arguments.contains("loc¦") {
                    let hint = latex_lsp::signature::help(
                        &workspace,
                        &main,
                        Text::new(source.clone()).position(marked.find('¦').unwrap()),
                    )
                    .unwrap();
                    assert_eq!(
                        hint.active_parameter,
                        Some(if command == "RequirePackageWithOptions" {
                            0
                        } else {
                            1
                        }),
                        "{marked}"
                    );
                }
                let item = items
                    .into_iter()
                    .find(|item| item.label == "local.sty")
                    .unwrap_or_else(|| panic!("{marked}"));
                let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                    panic!("missing edit")
                };
                let mut edits = item.additional_text_edits.unwrap_or_default();
                edits.push(edit);
                assert_eq!(
                    apply(&source, &edits),
                    format!("\\{command}{options}{expected}"),
                    "{marked}"
                );
            }
        }
    }
}

#[test]
fn uncertain_package_lists_and_unsupported_flags_do_not_link_or_edit_following_body_names() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("local.sty"), "Local.").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for command in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
        for arguments in [
            r"*{loc¦al}",
            r" * {loc¦al}",
            r"[one][two]{loc¦al}",
            "{lo¦% joined\ncal}",
            r"{loc¦al\suffix}",
            r"{{loc¦al}}",
            "{% comment loc¦al\n}",
        ] {
            let marked = format!("\\{command}{arguments}");
            let source = marked.replace('¦', "");
            std::fs::write(root.join("main.tex"), &source).unwrap();
            workspace.open(main.clone(), source.clone(), 7);
            assert!(
                navigation::document_links(&workspace, &main).is_empty(),
                "{marked}"
            );
            assert!(
                !features::completion(
                    &workspace,
                    &main,
                    Text::new(source).position(marked.find('¦').unwrap())
                )
                .iter()
                .any(|item| item.label == "local.sty"),
                "{marked}"
            );
            assert!(
                file_operations::will_rename(
                    &workspace,
                    &[rename(root, "local.sty", "renamed.sty")]
                )
                .unwrap()
                .is_none(),
                "{marked}"
            );
        }
        if command == "RequirePackageWithOptions" {
            let marked = "\\RequirePackageWithOptions[draft]{loc¦al}";
            let source = marked.replace('¦', "");
            workspace.open(main.clone(), source.clone(), 7);
            assert!(navigation::document_links(&workspace, &main).is_empty());
            assert!(!features::completion(
                &workspace,
                &main,
                Text::new(source).position(marked.find('¦').unwrap())
            )
            .iter()
            .any(|item| item.label == "local.sty"));
        }
    }
}

#[test]
fn package_errors_and_repairs_do_not_guess_from_partly_dynamic_commented_lists() {
    for command in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
        for unknown in [r"\dynamic", r"{local}", "lo% joined\ncal"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source =
                format!("\\{command}{{% head, phantom, }}\n locla,{unknown}% tail\n}}\n\\relax\n");
            let main = root.join("main.tex");
            std::fs::write(&main, &source).unwrap();
            std::fs::write(root.join("local.sty"), "Local.\n").unwrap();
            let log = format!("(./main.tex\n! LaTeX Error: File `locla.sty' not found.\n! Emergency stop.\n<read *>\nl.{} \\relax\n", source.lines().count());
            let main_uri = uri(root, "main.tex");
            let parsed = latex_lsp::build_log::parse(&main, &log);
            let mut diagnostic = parsed[&main_uri]
                .iter()
                .find(|diagnostic| diagnostic.message.contains("not found"))
                .unwrap()
                .clone();
            assert!(diagnostic.code.is_none(), "{source}: {diagnostic:?}");
            let text = Text::new(source.clone());
            let start = source.find("locla").unwrap();
            diagnostic.range = text.range(start, start + 5);
            diagnostic.code = Some(NumberOrString::String("missing-package".into()));
            diagnostic.data = Some(serde_json::json!({"file":"locla.sty","loaderCommand":command}));
            let mut workspace = Workspace {
                code_action_literals: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(main_uri.clone(), source.clone(), 7);
            workspace.build_diagnostics.insert(
                main_uri.clone(),
                [(main_uri.clone(), vec![diagnostic.clone()])].into(),
            );
            let actions = latex_lsp::editing::code_actions(
                &workspace,
                &main_uri,
                diagnostic.range,
                &CodeActionContext {
                    diagnostics: vec![diagnostic],
                    ..Default::default()
                },
            );
            assert!(actions.is_empty(), "{source}: {actions:?}");
        }
    }
}
