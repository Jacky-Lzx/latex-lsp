use latex_lsp::{
    features, file_operations, graphics, navigation, text::Text, workspace::Workspace,
};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

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
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for (call, argument) in [
            (r"\includegraphics", r#"" plot  image.png ""#),
            (r"\csuse{Image}", r#""plot  image".png"#),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let compiler = root.join(engine);
            std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: compiler.to_string_lossy().into(),
                ..Default::default()
            };
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
                let outcome = latex_lsp::build::run(&path, &config, &Default::default());
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
            let outcome = latex_lsp::build::run(&path, &config, &Default::default());
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
                code_action_literals: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace
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
            let outcome = latex_lsp::build::run(&path, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            assert!(outcome
                .inputs
                .unwrap()
                .contains(&uri(root, "assets/chart image.png")));
        }
    }
}
