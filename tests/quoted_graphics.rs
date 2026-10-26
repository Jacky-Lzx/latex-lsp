use latex_lsp::{
    editing::file_operations,
    language::{features, navigation},
    project::{graphics, workspace::Workspace},
    text::Text,
};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0,
    0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 96, 96, 96, 0, 0, 0, 4, 0,
    1, 246, 23, 56, 85, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project(root: &Path, source: &str) -> Workspace {
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("main.tex"), source).unwrap();
    for name in [
        "plot.pdf",
        "plot.png",
        "plot.pdf.png",
        "figure.pdf",
        "figure.png",
    ] {
        std::fs::write(root.join("assets").join(name), name).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn targets(workspace: &Workspace, document: &Url) -> Vec<Url> {
    navigation::document_links(workspace, document)
        .into_iter()
        .filter_map(|link| link.target)
        .filter(|target| graphics::is_image(&target.to_file_path().unwrap()))
        .collect()
}

fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

fn changes(workspace: &Workspace, files: &[FileRename]) -> BTreeMap<Url, String> {
    file_operations::will_rename(workspace, files)
        .unwrap()
        .map_or_else(BTreeMap::new, |edit| {
            edit.changes
                .unwrap()
                .into_iter()
                .map(|(uri, mut edits)| {
                    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
                    let changed = workspace.documents[&uri]
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
                        .unwrap();
                    (uri, changed.source)
                })
                .collect()
        })
}

#[test]
fn quoted_graphics_paths_resolve_and_keep_quoted_renames() {
    for argument in [
        r#""plot image.png""#,
        r#""plot image".png"#,
        r#""plot" image.png"#,
        r#" "plot image.png" "#,
    ] {
        for call in [
            r"\includegraphics",
            r"\Image",
            r"\csuse{Image}",
            r"\csname Image\endcsname",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source =
                format!("\\def\\Image{{\\includegraphics}}{call}*[width=1cm]{{{argument}}}");
            let workspace = project(root, &source);
            std::fs::write(root.join("plot image.png"), PNG).unwrap();
            let main = uri(root, "main.tex");
            assert_eq!(
                targets(&workspace, &main),
                [uri(root, "plot image.png")],
                "{source}"
            );
            assert_eq!(
                graphics::images(&workspace, &main),
                [uri(root, "plot image.png")].into()
            );
            let changed = changes(
                &workspace,
                &[rename(root, "plot image.png", "chart image.png")],
            );
            assert_eq!(
                changed[&main],
                source.replace(argument.trim(), r#""chart image.png""#),
                "{source}"
            );
        }
    }
}

fn apply_completion(workspace: &Workspace, main: &Url, marked: &str, label: &str) -> String {
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let items = features::completion(workspace, main, Text::new(source.clone()).position(offset));
    let item = items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{marked}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("{item:?}")
    };
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source)
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
fn quoted_completion_preserves_balanced_quotes_suffixes_options_and_directories() {
    for (argument, label, expected) in [
        (
            r#""pl¦ot image.png""#,
            "plot image.png",
            r#""plot image.png""#,
        ),
        (
            r#""pl¦"ot image.png"#,
            "plot image.png",
            r#""plot image.png""#,
        ),
        (r#" "plot ¦""#, "plot image.png", r#" "plot image.png""#),
        (
            "\"pl¦ot\n image.png\"",
            "plot image.png",
            r#""plot image.png""#,
        ),
        (
            "\"plot\r\n  ¦image.png\"",
            "plot image.png",
            r#""plot image.png""#,
        ),
        (r#""pl¦"#, "plot image.png", r#""plot image.png""#),
        (r#""ass¦ets/plot.png""#, "assets/", r#""assets/plot.png""#),
        (r#""ass¦"ets/plot.png"#, "assets/", r#""assets"/plot.png"#),
        (
            r#""assets"/pl¦ot.png"#,
            "assets/plot.png",
            r#""assets/plot.png""#,
        ),
    ] {
        for call in [r"\includegraphics", r"\Image", r"\csname Image\endcsname"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let marked =
                format!("\\def\\Image{{\\includegraphics}}{call}*[width=1cm]{{{argument}}}");
            let workspace = project(root, &marked.replace('¦', ""));
            std::fs::write(root.join("plot image.png"), PNG).unwrap();
            let actual = apply_completion(&workspace, &uri(root, "main.tex"), &marked, label);
            assert_eq!(actual, marked.replace(argument, expected), "{marked}");
        }
    }
}

#[test]
fn quoted_whitespace_matches_tex_and_ignores_unbalanced_or_dynamic_names() {
    for argument in [
        r#""plot  image.png""#,
        "\"plot\n  image.png\"",
        r#"" plot image.png ""#,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\includegraphics{{{argument}}}");
        let workspace = project(root, &source);
        std::fs::write(root.join("plot image.png"), PNG).unwrap();
        std::fs::write(root.join("plot  image.png"), "wrong file").unwrap();
        assert_eq!(
            targets(&workspace, &uri(root, "main.tex")),
            [uri(root, "plot image.png")],
            "{argument}"
        );
    }
    for argument in [
        r#""plot image.png"#,
        r#""plot\macro image.png""#,
        "\"plot\n\nimage.png\"",
        "\"plot% comment\n image.png\"",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(root, &format!("\\includegraphics{{{argument}}}"));
        std::fs::write(root.join("plot image.png"), PNG).unwrap();
        assert!(
            targets(&workspace, &uri(root, "main.tex")).is_empty(),
            "{argument}"
        );
    }
}

#[test]
fn quoted_search_directories_resolve_and_keep_quotes_through_directory_and_root_moves() {
    for entry in [
        r#""figure files/""#,
        r#""figure files"/"#,
        r#""figure" files/"#,
        r#""figure  files/""#,
        r#""./figure files/""#,
        "\"figure\n files/\"",
    ] {
        for call in [r"\graphicspath", r"\Paths", r"\csuse{Paths}"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\def\\Paths{{\\graphicspath}}{call}{{{{{entry}}}}}\\includegraphics{{\"plot image\"}}");
            let workspace = project(root, &source);
            for folder in ["figure files", "figure  files", "\"figure files\""] {
                std::fs::create_dir(root.join(folder)).unwrap();
                std::fs::write(root.join(folder).join("plot image.png"), PNG).unwrap();
            }
            let main = uri(root, "main.tex");
            assert_eq!(
                targets(&workspace, &main),
                [uri(root, "figure files/plot image.png")],
                "{source}"
            );
            assert_eq!(
                graphics::images(&workspace, &main),
                [uri(root, "figure files/plot image.png")].into()
            );
            assert!(changes(
                &workspace,
                &[rename(root, "assets/plot.png", "assets/chart.png")]
            )
            .is_empty());
            let prefix = if entry.contains("./") { "./" } else { "" };
            let changed = changes(&workspace, &[rename(root, "figure files", "new figures")]);
            assert_eq!(
                changed[&main],
                source.replace(entry, &format!("\"{prefix}new figures/\"")),
                "{source}"
            );
            let changed = changes(
                &workspace,
                &[
                    rename(root, "figure files", "new figures"),
                    rename(root, "main.tex", "book/paper.tex"),
                ],
            );
            assert_eq!(
                changed[&main],
                source.replace(entry, r#""../new figures/""#),
                "{source}"
            );
        }
    }
}

#[test]
fn quoted_search_directories_preserve_absolute_paths_and_reject_unrepresentable_moves() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("figure files")).unwrap();
    std::fs::write(root.join("figure files/plot.png"), PNG).unwrap();
    let entry = format!("\"{}/\"", root.join("figure files").display());
    let source = format!("\\graphicspath{{{{{entry}}}}}\\includegraphics{{plot}}");
    let workspace = project(root, &source);
    let main = uri(root, "main.tex");
    let changed = changes(&workspace, &[rename(root, "figure files", "new figures")]);
    assert_eq!(
        changed[&main],
        source.replace(
            &entry,
            &format!("\"{}/\"", root.join("new figures").display())
        )
    );
    assert!(file_operations::will_rename(
        &workspace,
        &[rename(root, "figure files", "two  spaces")]
    )
    .is_err());
}

#[test]
fn quoted_search_directory_completions_use_the_active_normalized_directory() {
    for entry in [
        r#""figure files/""#,
        r#""figure files"/"#,
        r#""figure  files/""#,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("figure files")).unwrap();
        std::fs::write(root.join("figure files/plot image.png"), PNG).unwrap();
        let marked = format!("\\graphicspath{{{{assets/}}}}\\def\\Paths{{\\graphicspath}}\\csname Paths\\endcsname{{{{{entry}}}}}\\includegraphics*[width=1cm]{{\"pl¦ot image\"}}");
        let workspace = project(root, &marked.replace('¦', ""));
        let changed = apply_completion(
            &workspace,
            &uri(root, "main.tex"),
            &marked,
            "plot image.png",
        );
        assert_eq!(
            changed,
            marked.replace(r#""pl¦ot image""#, r#""plot image.png""#)
        );
    }
}

#[test]
fn malformed_or_trailing_space_search_directories_do_not_guess_targets_or_edits() {
    for entry in [
        r#""figure files/"#,
        r#""figure files/" "#,
        r#""figure files/ ""#,
        "figure files/ ",
        r#""\chosen/""#,
        "\"figure\n\nfiles/\"",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("figure files")).unwrap();
        std::fs::write(root.join("figure files/plot.png"), PNG).unwrap();
        let source = format!("\\graphicspath{{{{{entry}}}}}\\includegraphics{{plot}}");
        let workspace = project(root, &source);
        let main = uri(root, "main.tex");
        assert!(targets(&workspace, &main).is_empty(), "{source}");
        assert!(
            changes(&workspace, &[rename(root, "figure files", "new figures")]).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn quoted_search_directories_require_a_confirmed_pdftex_engine_including_magic_programs() {
    for (program, custom_arguments, magic, expected) in [
        ("pdflatex", false, "", true),
        ("xelatex", false, "", false),
        ("lualatex", false, "", false),
        ("custom-compiler", false, "", false),
        ("pdflatex", true, "", false),
        ("latexmk", false, "% !TeX program = xelatex\n", false),
        ("latexmk", false, "% !TeX program = lualatex\n", false),
        ("latexmk", false, "% !TeX program = pdflatex\n", true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("figure files")).unwrap();
        std::fs::write(root.join("figure files/plot.png"), PNG).unwrap();
        let source =
            format!("{magic}\\graphicspath{{{{\"figure files/\"}}}}\\includegraphics{{plot}}");
        let mut workspace = project(root, &source);
        workspace.config.build.executable = program.into();
        if custom_arguments {
            workspace.config.build.arguments = Some(vec![]);
        }
        let main = uri(root, "main.tex");
        assert_eq!(
            !targets(&workspace, &main).is_empty(),
            expected,
            "{program}/{magic}/{custom_arguments}"
        );
        assert_eq!(
            !changes(&workspace, &[rename(root, "figure files", "new figures")]).is_empty(),
            expected,
            "{program}/{magic}/{custom_arguments}"
        );
    }
}

#[test]
fn other_engines_do_not_link_the_literal_quote_directory_used_by_existence_checks() {
    for program in ["xelatex", "lualatex", "custom-compiler"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for folder in ["figure files", "\"figure files\""] {
            std::fs::create_dir(root.join(folder)).unwrap();
            std::fs::write(root.join(folder).join("plot.png"), PNG).unwrap();
        }
        let mut workspace = project(
            root,
            r#"\graphicspath{{"figure files"/}}\includegraphics{plot}"#,
        );
        workspace.config.build.executable = program.into();
        assert!(
            targets(&workspace, &uri(root, "main.tex")).is_empty(),
            "{program}"
        );
        for folder in ["figure files", "\"figure files\""] {
            assert!(
                changes(&workspace, &[rename(root, folder, "new figures")]).is_empty(),
                "{program}: {folder}"
            );
        }
    }
}

#[test]
fn quoted_completion_skips_paragraphs_and_dynamic_filename_prefixes() {
    for argument in [
        "\n\n\"pl¦",
        "\"pl\n\n  ¦",
        r#""pl\macro¦"#,
        "\"pl%comment\n¦",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let marked = format!("\\includegraphics{{{argument}}}");
        let source = marked.replace('¦', "");
        let workspace = project(root, &source);
        std::fs::write(root.join("plot image.png"), PNG).unwrap();
        let items = features::completion(
            &workspace,
            &uri(root, "main.tex"),
            Text::new(source).position(marked.find('¦').unwrap()),
        );
        assert!(
            items
                .iter()
                .all(|item| item.kind != Some(CompletionItemKind::FILE)
                    && item.kind != Some(CompletionItemKind::FOLDER)),
            "{marked}: {items:?}"
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex, lualatex, graphicx and etoolbox"]
fn real_quoted_images_compile_after_moves_and_missing_filename_repairs() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for (call, argument) in [
            (r"\includegraphics", r#"" plot  image.png ""#),
            (r"\csuse{Image}", r#""plot  image".png"#),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let config = engine_config(root, engine);
            let source = format!("\\documentclass{{article}}\n\\usepackage{{graphicx,etoolbox}}\n\\def\\Image{{\\includegraphics}}\n\\begin{{document}}\n\\typeout{{😀}} {call}*[width=1cm]{{{argument}}}\n\\end{{document}}\n");
            let mut workspace = project(root, &source);
            std::fs::write(root.join("plot image.png"), PNG).unwrap();
            // TeX folds ordinary repeated spaces even inside filename quotes.
            std::fs::write(root.join("plot  image.png"), "not the selected image").unwrap();
            let main = uri(root, "main.tex");
            let path = root.join("main.tex");
            for moved in [false, true] {
                let image = uri(
                    root,
                    if moved {
                        "assets/chart image.png"
                    } else {
                        "plot image.png"
                    },
                );
                assert_eq!(
                    targets(&workspace, &main).as_slice(),
                    std::slice::from_ref(&image)
                );
                let outcome =
                    latex_lsp::compilation::build::run(&path, &config, &Default::default());
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{argument}: {}",
                    outcome.output
                );
                assert!(outcome.inputs.unwrap().contains(&image));
                if !moved {
                    let files = [rename(root, "plot image.png", "assets/chart image.png")];
                    for (document, changed) in changes(&workspace, &files) {
                        std::fs::write(document.to_file_path().unwrap(), changed).unwrap();
                    }
                    std::fs::rename(
                        root.join("plot image.png"),
                        root.join("assets/chart image.png"),
                    )
                    .unwrap();
                    file_operations::did_rename(&mut workspace, &files).unwrap();
                }
            }
            // Restore segmented quoting to exercise the driver's unquoted error.
            let missing = if argument.ends_with(".png") {
                r#""assets/chrat image".png"#
            } else {
                r#""assets/chrat image.png""#
            };
            let source = std::fs::read_to_string(&path)
                .unwrap()
                .replace(r#""assets/chart image.png""#, missing);
            assert!(source.contains(missing), "{source}");
            std::fs::write(&path, &source).unwrap();
            let outcome = latex_lsp::compilation::build::run(&path, &config, &Default::default());
            assert_eq!(outcome.status, "error", "{engine}: {}", outcome.output);
            let diagnostic = outcome.diagnostics[&main]
                .iter()
                .find(|item| item.code == Some(NumberOrString::String("missing-image".into())))
                .unwrap_or_else(|| {
                    panic!(
                        "{engine}/{missing}: {:?}\n{}",
                        outcome.diagnostics, outcome.output
                    )
                })
                .clone();
            let text = Text::new(source);
            let start = text.source.find(missing).unwrap();
            assert_eq!(diagnostic.range, text.range(start, start + missing.len()));
            let mut workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace
                .project
                .build_diagnostics
                .insert(main.clone(), outcome.diagnostics);
            let actions = latex_lsp::editing::code_actions(
                &workspace,
                &main,
                diagnostic.range,
                &CodeActionContext {
                    diagnostics: vec![diagnostic],
                    ..Default::default()
                },
            );
            assert_eq!(actions.len(), 1, "{engine}/{missing}: {actions:?}");
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!()
            };
            let edits = action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&main].clone();
            assert_eq!(edits[0].new_text, r#""assets/chart image.png""#);
            let repaired = text
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
                .unwrap();
            std::fs::write(&path, repaired.source).unwrap();
            let outcome = latex_lsp::compilation::build::run(&path, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            assert!(outcome
                .inputs
                .unwrap()
                .contains(&uri(root, "assets/chart image.png")));
        }
    }
}

#[cfg(unix)]
fn engine_config(root: &Path, engine: &str) -> latex_lsp::config::BuildConfig {
    use std::os::unix::fs::PermissionsExt;
    let compiler = root.join(engine);
    std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
    latex_lsp::config::BuildConfig {
        executable: compiler.to_string_lossy().into(),
        ..Default::default()
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex, lualatex, graphicx and etoolbox"]
fn real_quoted_search_directories_keep_images_and_repairs_after_directory_and_root_moves() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for (index, (call, entry)) in [
            (r"\graphicspath", r#""figure  files/""#),
            (r"\Paths", r#""figure files"/"#),
            (r"\csuse{Paths}", "\"figure\n files/\""),
        ]
        .into_iter()
        .enumerate()
        {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let mut source = format!("\\documentclass{{article}}\n\\usepackage{{graphicx,etoolbox}}\n\\def\\Paths{{\\graphicspath}}\n{call}{{{{{entry}}}}}\n\\DeclareGraphicsExtensions{{.png}}\n\\begin{{document}}\n\\includegraphics[width=1cm]{{\"plot image\"}}\n\\end{{document}}\n");
            let mut workspace = project(root, &source);
            std::fs::create_dir(root.join("figure files")).unwrap();
            std::fs::write(root.join("figure files/plot image.png"), PNG).unwrap();
            for wrong in ["figure  files", "\"figure files\""] {
                // Xe/LuaTeX may test the literally quoted directory before
                // their image driver removes quotes. Keep it absent when
                // verifying that a normal project cannot use these entries.
                if engine != "pdflatex" && wrong.contains('"') {
                    continue;
                }
                std::fs::create_dir(root.join(wrong)).unwrap();
                std::fs::write(root.join(wrong).join("plot image.png"), "wrong image").unwrap();
            }
            let config = engine_config(root, engine);
            workspace.config.build = config.clone();
            let main = uri(root, "main.tex");
            if engine != "pdflatex" {
                assert!(targets(&workspace, &main).is_empty());
                assert!(
                    changes(&workspace, &[rename(root, "figure files", "new figures")]).is_empty()
                );
                let outcome = latex_lsp::compilation::build::run(
                    &root.join("main.tex"),
                    &config,
                    &Default::default(),
                );
                assert_eq!(
                    outcome.status, "error",
                    "{engine}/{entry}: {}",
                    outcome.output
                );
                source = source.replace(entry, "figure files/");
                std::fs::write(root.join("main.tex"), &source).unwrap();
                workspace.reload(&main);
            }
            for moved in [false, true] {
                let main = uri(root, if moved { "book/paper.tex" } else { "main.tex" });
                let image = uri(
                    root,
                    if moved {
                        "new figures/plot image.png"
                    } else {
                        "figure files/plot image.png"
                    },
                );
                assert_eq!(
                    targets(&workspace, &main).as_slice(),
                    std::slice::from_ref(&image)
                );
                let outcome = latex_lsp::compilation::build::run(
                    &main.to_file_path().unwrap(),
                    &config,
                    &Default::default(),
                );
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{entry}/{moved}: {}",
                    outcome.output
                );
                assert!(outcome.inputs.unwrap().contains(&image));
                if !moved {
                    let files = [
                        rename(root, "figure files", "new figures"),
                        rename(root, "main.tex", "book/paper.tex"),
                    ];
                    let changed = changes(&workspace, &files);
                    let expected = if engine == "pdflatex" {
                        source.replace(entry, r#""../new figures/""#)
                    } else {
                        source.replace("{figure files/}", "{../new figures/}")
                    };
                    assert_eq!(changed[&main], expected);
                    for (document, changed) in changed {
                        std::fs::write(document.to_file_path().unwrap(), changed).unwrap();
                    }
                    std::fs::create_dir(root.join("book")).unwrap();
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
            if index != 0 {
                continue;
            }
            let main = uri(root, "book/paper.tex");
            let path = main.to_file_path().unwrap();
            let source = std::fs::read_to_string(&path)
                .unwrap()
                .replace(r#""plot image""#, r#""plto image""#);
            std::fs::write(&path, &source).unwrap();
            let outcome = latex_lsp::compilation::build::run(&path, &config, &Default::default());
            assert_eq!(outcome.status, "error", "{engine}: {}", outcome.output);
            let diagnostic = outcome.diagnostics[&main]
                .iter()
                .find(|item| item.code == Some(NumberOrString::String("missing-image".into())))
                .unwrap_or_else(|| {
                    panic!("{engine}: {:?}\n{}", outcome.diagnostics, outcome.output)
                })
                .clone();
            let text = Text::new(source);
            let start = text.source.find(r#""plto image""#).unwrap();
            assert_eq!(
                diagnostic.range,
                text.range(start, start + r#""plto image""#.len())
            );
            let mut workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.config.build = config.clone();
            workspace.add_root(root.into());
            workspace
                .project
                .build_diagnostics
                .insert(main.clone(), outcome.diagnostics);
            let actions = latex_lsp::editing::code_actions(
                &workspace,
                &main,
                diagnostic.range,
                &CodeActionContext {
                    diagnostics: vec![diagnostic],
                    ..Default::default()
                },
            );
            assert_eq!(actions.len(), 1, "{engine}: {actions:?}");
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!()
            };
            let edit = action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&main][0].clone();
            assert_eq!(edit.new_text, r#""plot image.png""#);
            let repaired = text
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap();
            std::fs::write(&path, repaired.source).unwrap();
            let outcome = latex_lsp::compilation::build::run(&path, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            assert!(outcome
                .inputs
                .unwrap()
                .contains(&uri(root, "new figures/plot image.png")));
        }
    }
}
