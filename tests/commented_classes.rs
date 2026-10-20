use latex_lsp::{
    features, file_operations, navigation, signature, text::Text, workspace::Workspace,
};
use std::path::Path;
use tower_lsp::lsp_types::*;
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
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
fn commented_single_class_names_load_spaced_comma_filenames_and_edit_only_the_real_path() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("project")).unwrap();
    std::fs::create_dir(root.join("shared")).unwrap();
    for (name, word) in [
        ("my class,name", "Correct"),
        ("myclass,name", "Decoy"),
        ("phantom", "Unused"),
    ] {
        std::fs::write(
            root.join(format!("shared/{name}.cls")),
            format!("\\LoadClass{{article}}\\newcommand{{\\classword}}{{{word}}}\\endinput\n"),
        )
        .unwrap();
    }
    for command in ["documentclass", "LoadClass", "LoadClassWithOptions"] {
        let options = if command == "LoadClassWithOptions" {
            ""
        } else {
            "[draft [literal]"
        };
        let call=format!("\\{command}{options}{{% emoji😀, ../shared/phantom, }} ignored\r\n ../shared/my class,name% tail\r\n}}\r\n");
        let body=format!("\\begin{{document}}\\classword\r\n\\verb|\\{command}{{../shared/my class,name}}|\r\n\\end{{document}}\r\n");
        let main_source = if command == "documentclass" {
            format!("{call}{body}")
        } else {
            format!("\\documentclass{{bridge}}\r\n{body}")
        };
        std::fs::write(root.join("project/main.tex"), &main_source).unwrap();
        let (loader, source) = if command == "documentclass" {
            (uri(root, "project/main.tex"), main_source.clone())
        } else {
            let source = format!("{call}\\endinput\r\n");
            std::fs::write(root.join("project/bridge.cls"), &source).unwrap();
            (uri(root, "project/bridge.cls"), source)
        };
        let main = uri(root, "project/main.tex");
        let mut workspace = Workspace::default();
        workspace.add_root(root.join("project"));
        workspace.open(main.clone(), main_source.clone(), 7);
        if loader != main {
            workspace.open(loader.clone(), source.clone(), 7);
        }
        let target = uri(root, "shared/my class,name.cls");
        let files = workspace.project_files(&main);
        assert!(files.contains(&target));
        assert!(!files.contains(&uri(root, "shared/myclass,name.cls")));
        assert!(!files.contains(&uri(root, "shared/phantom.cls")));
        let links = navigation::document_links(&workspace, &loader);
        assert_eq!(links.len(), 1, "{command}: {links:?}");
        assert_eq!(links[0].target, Some(target.clone()));
        let text = Text::new(source.clone());
        assert_eq!(
            &source[text.offset(links[0].range.start).unwrap()
                ..text.offset(links[0].range.end).unwrap()],
            "../shared/my class,name"
        );
        let definitions = features::definitions(
            &workspace,
            &main,
            Text::new(main_source.clone()).position(main_source.find("\\classword").unwrap() + 3),
        );
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].uri, target);
        assert!(file_operations::will_rename(
            &workspace,
            &[rename(root, "shared/phantom.cls", "shared/unused.cls")]
        )
        .unwrap()
        .is_none());
        assert!(file_operations::will_rename(
            &workspace,
            &[rename(root, "shared/my class,name.cls", "shared/other.sty")]
        )
        .is_err());
        let changes = file_operations::will_rename(
            &workspace,
            &[rename(
                root,
                "shared/my class,name.cls",
                "shared/new class,name.cls",
            )],
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&loader].len(), 1);
        assert_eq!(
            apply(&source, &changes[&loader]),
            source.replace(
                "\n ../shared/my class,name%",
                "\n ../shared/new class,name%"
            )
        );
        let changes =
            file_operations::will_rename(&workspace, &[rename(root, "shared", "classes")])
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&loader].len(), 1);
        assert_eq!(
            apply(&source, &changes[&loader]),
            source.replace(
                "\n ../shared/my class,name%",
                "\n ../classes/my class,name%"
            )
        );
    }
}
#[test]
fn class_path_completion_keeps_spaces_and_commas_and_reads_first_unbraced_option_end() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("my class,name.cls"), "Class.").unwrap();
    std::fs::write(root.join("myclass,name.cls"), "Decoy.").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for command in ["documentclass", "LoadClass", "LoadClassWithOptions"] {
        for options in ["", "[draft]", "[draft [literal]"] {
            if command == "LoadClassWithOptions" && !options.is_empty() {
                continue;
            }
            for (arguments, expected) in [
                (
                    "{% before, }\r\nmy class,na¦BAD% tail\r\n}",
                    "{% before, }\r\nmy class,name% tail\r\n}",
                ),
                ("{my class,na¦BAD}", "{my class,name}"),
                ("{% before\nmy class,na¦", "{% before\nmy class,name"),
                ("{% before\n¦% tail\n}", "{% before\nmy class,name% tail\n}"),
            ] {
                let marked = format!("\\{command}{options}{arguments}");
                let source = marked.replace('¦', "");
                workspace.open(main.clone(), source.clone(), 7);
                let position = Text::new(source.clone()).position(marked.find('¦').unwrap());
                let item = features::completion(&workspace, &main, position)
                    .into_iter()
                    .find(|item| item.label == "my class,name.cls")
                    .unwrap_or_else(|| panic!("{marked}"));
                if arguments.contains("na¦") {
                    let help = signature::help(&workspace, &main, position).unwrap();
                    assert_eq!(
                        help.active_parameter,
                        Some(if command == "LoadClassWithOptions" {
                            0
                        } else {
                            1
                        }),
                        "{marked}"
                    );
                }
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
fn uncertain_single_class_names_do_not_link_or_edit_partial_or_following_body_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("my class,name.cls"), "Class.").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for command in ["documentclass", "LoadClass", "LoadClassWithOptions"] {
        for arguments in [
            r"*{my class,na¦me}",
            r"[one][two]{my class,na¦me}",
            "{my class,na¦% joined\nme}",
            r"{my class,na¦me\suffix}",
            r"{{my class,na¦me}}",
            "{% comment my class,na¦me\n}",
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
                .any(|item| item.label == "my class,name.cls"),
                "{marked}"
            );
            assert!(
                file_operations::will_rename(
                    &workspace,
                    &[rename(root, "my class,name.cls", "new class,name.cls")]
                )
                .unwrap()
                .is_none(),
                "{marked}"
            );
        }
        if command == "LoadClassWithOptions" {
            let source = r"\LoadClassWithOptions[draft]{my class,name}";
            workspace.open(main.clone(), source.into(), 7);
            assert!(navigation::document_links(&workspace, &main).is_empty());
        }
    }
}
