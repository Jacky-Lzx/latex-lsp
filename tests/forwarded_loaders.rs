use latex_lsp::{build, features, file_operations, navigation, text::Text, workspace::Workspace};
use std::path::Path;
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn apply(workspace: &Workspace, files: &[FileRename]) -> Vec<(Url, String)> {
    file_operations::will_rename(workspace, files)
        .unwrap()
        .unwrap()
        .changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let source = workspace.documents[&uri]
                .text
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
                .source;
            (uri, source)
        })
        .collect()
}

#[test]
fn option_forwarding_loaders_resolve_complete_and_rename_compiler_load_names() {
    for (command, extension) in [
        ("RequirePackageWithOptions", "sty"),
        ("LoadClassWithOptions", "cls"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let literal = if extension == "cls" {
            "lo cal"
        } else {
            "local"
        };
        let name = format!("{literal}.{extension}.{extension}");
        std::fs::write(root.join(&name), "Expected source.").unwrap();
        std::fs::write(root.join(format!("local.{extension}")), "Decoy source.").unwrap();
        if extension == "cls" {
            std::fs::write(root.join("local.cls.cls"), "Compact-name decoy.").unwrap();
        }
        let source = format!("\\{command}{{lo cal.{extension}}}[2020/01/01]");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = uri(root, "main.tex");
        let expected = uri(root, &name);
        assert_eq!(
            navigation::document_links(&workspace, &main)[0].target,
            Some(expected.clone())
        );
        assert!(workspace.project_files(&main).contains(&expected));
        let rename = FileRename {
            old_uri: expected.to_string(),
            new_uri: uri(root, &format!("renamed.{extension}")).to_string(),
        };
        let edits = apply(&workspace, &[rename]);
        assert_eq!(
            edits,
            vec![(main.clone(), format!("\\{command}{{renamed}}[2020/01/01]"))]
        );
        assert!(file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: expected.to_string(),
                new_uri: uri(root, "renamed.txt").to_string(),
            }]
        )
        .unwrap_err()
        .message
        .contains(&format!(".{extension}")));
        let source = format!("\\{command}{{lo");
        workspace.open(main.clone(), source.clone(), 1);
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(source.len()),
        );
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, format!("{literal}.{extension}"));
    }
}

#[test]
fn forwarded_package_lists_keep_siblings_and_version_constraints_during_file_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\RequirePackageWithOptions{first, sec ond }[2020/01/01]";
    std::fs::write(root.join("main.tex"), source).unwrap();
    for name in ["first.sty", "second.sty"] {
        std::fs::write(root.join(name), "Package.").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    assert_eq!(navigation::document_links(&workspace, &main).len(), 2);
    let edits = apply(
        &workspace,
        &[FileRename {
            old_uri: uri(root, "second.sty").to_string(),
            new_uri: uri(root, "renamed.sty").to_string(),
        }],
    );
    assert_eq!(
        edits,
        vec![(
            main,
            r"\RequirePackageWithOptions{first, renamed }[2020/01/01]".into()
        )]
    );
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn forwarding_loaders_keep_options_and_compiler_inputs_after_file_renames() {
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for (name, source) in [
            ("main.tex", "\\documentclass[12pt]{wrapper}\n\\usepackage[draft]{wrapperpkg}\n\\begin{document}Hello.\\end{document}\n"),
            ("wrapper.cls", "\\LoadClassWithOptions{localclass}[2020/01/01]\n\\relax\n"),
            ("localclass.cls", "\\ProvidesClass{localclass}[2026/01/01]\n\\LoadClassWithOptions{article}\n\\ifnum\\@ptsize=2 \\typeout{CLASS OPTIONS PRESERVED}\\else\\errmessage{Class options lost}\\fi\n"),
            ("wrapperpkg.sty", "\\RequirePackageWithOptions{localpkg}[2020/01/01]\n\\relax\n"),
            ("localpkg.sty", "\\ProvidesPackage{localpkg}[2026/01/01]\n\\DeclareOption{draft}{\\typeout{PACKAGE OPTIONS PRESERVED}}\n\\DeclareOption*{\\errmessage{Unexpected package option}}\n\\ProcessOptions\\relax\n"),
        ] { std::fs::write(root.join(name), source).unwrap(); }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = uri(root, "main.tex");
        let config = latex_lsp::config::BuildConfig {
            executable: engine.into(),
            ..Default::default()
        };
        for renamed in [false, true] {
            let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            for marker in ["CLASS OPTIONS PRESERVED", "PACKAGE OPTIONS PRESERVED"] {
                assert!(outcome.output.contains(marker), "{}", outcome.output);
            }
            for (wrapper, base, extension) in [
                ("wrapper.cls", "localclass", "cls"),
                ("wrapperpkg.sty", "localpkg", "sty"),
            ] {
                let filename = format!(
                    "{}.{extension}",
                    if renamed {
                        format!("new{base}")
                    } else {
                        base.into()
                    }
                );
                let target = uri(root, &filename);
                assert!(workspace.project_files(&main).contains(&target));
                assert!(navigation::document_links(&workspace, &uri(root, wrapper))
                    .iter()
                    .any(|link| link.target == Some(target.clone())));
                let recorder =
                    std::fs::read_to_string(outcome.log.as_ref().unwrap().with_extension("fls"))
                        .unwrap();
                assert!(
                    recorder
                        .lines()
                        .any(|line| line.starts_with("INPUT ") && line.ends_with(&filename)),
                    "{recorder}"
                );
            }
            if renamed {
                break;
            }
            let files =
                [("localclass", "cls"), ("localpkg", "sty")].map(|(name, ext)| FileRename {
                    old_uri: uri(root, &format!("{name}.{ext}")).to_string(),
                    new_uri: uri(root, &format!("new{name}.{ext}")).to_string(),
                });
            for (document, source) in apply(&workspace, &files) {
                std::fs::write(document.to_file_path().unwrap(), &source).unwrap();
                workspace.reload(&document);
            }
            for file in &files {
                std::fs::rename(
                    Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
                    Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
                )
                .unwrap();
            }
            file_operations::did_rename(&mut workspace, &files).unwrap();
        }
    }
}

#[test]
fn option_forwarding_loaders_do_not_offer_files_for_invalid_explicit_options_or_stars() {
    for command in ["RequirePackageWithOptions", "LoadClassWithOptions"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for name in ["local.sty", "local.cls"] {
            std::fs::write(root.join(name), "Source.").unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = uri(root, "main.tex");
        for arguments in ["[draft]{loc", "*{loc"] {
            let source = format!("\\{command}{arguments}");
            workspace.open(main.clone(), format!("{source}al}}"), 1);
            assert!(
                navigation::document_links(&workspace, &main).is_empty(),
                "{source}"
            );
            assert!(!workspace
                .project_files(&main)
                .contains(&uri(root, "local.sty")));
            assert!(!workspace
                .project_files(&main)
                .contains(&uri(root, "local.cls")));
            workspace.open(main.clone(), source.clone(), 2);
            let items = features::completion(
                &workspace,
                &main,
                Text::new(source.clone()).position(source.len()),
            );
            assert!(
                !items
                    .iter()
                    .any(|item| item.kind == Some(CompletionItemKind::FILE)),
                "{source}: {items:?}"
            );
        }
    }
}
