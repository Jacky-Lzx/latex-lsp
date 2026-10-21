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
fn static_image_aliases_and_named_calls_resolve_and_rename_the_real_image() {
    for (definitions, call) in [
        (r"\def\Image{\includegraphics}", r"\Image"),
        (
            r"\newcommand{\Image}{\includegraphics}\def\Chain{\Image}",
            r"\Chain",
        ),
        ("", r"\csuse{includegraphics}"),
        ("", r"\csname includegraphics\endcsname"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("{definitions}\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.png,.pdf}}{call}*[width=1cm]{{plot}}");
        let workspace = project(root, &source);
        let main = uri(root, "main.tex");
        assert_eq!(
            targets(&workspace, &main),
            [uri(root, "assets/plot.png")],
            "{source}"
        );
        assert_eq!(
            graphics::images(&workspace, &main),
            [uri(root, "assets/plot.png")].into()
        );
        let changed = changes(
            &workspace,
            &[rename(root, "assets/plot.png", "assets/chart.png")],
        );
        assert_eq!(changed[&main], source.replace("{plot}", "{chart}"));
    }
}

#[test]
fn completion_uses_the_alias_call_scope_for_complete_and_unfinished_names() {
    for call in [r"\Image", r"\csuse{Image}", r"\csname Image\endcsname"] {
        for ending in ["}", ""] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\def\\Image{{\\includegraphics}}\\graphicspath{{{{assets/}}}}{{\\graphicspath{{{{local/}}}}{call}*[width={{1cm}}]{{pl{ending}");
            let workspace = project(root, &source);
            std::fs::create_dir(root.join("local")).unwrap();
            std::fs::write(root.join("local/plot-local.png"), PNG).unwrap();
            let offset = source.rfind("{pl").unwrap() + 3;
            let items = features::completion(
                &workspace,
                &uri(root, "main.tex"),
                Text::new(source.clone()).position(offset),
            );
            assert!(
                items.iter().any(|item| item.label == "plot-local.png"),
                "{source}: {items:?}"
            );
            assert!(
                !items.iter().any(|item| item.label == "plot.png"),
                "{source}: {items:?}"
            );
            let Some(CompletionTextEdit::Edit(edit)) = &items
                .iter()
                .find(|item| item.label == "plot-local.png")
                .unwrap()
                .text_edit
            else {
                panic!()
            };
            assert_eq!(edit.range, Text::new(source).range(offset - 2, offset));
        }
    }
}

#[test]
fn image_aliases_from_inputs_and_local_packages_use_the_compilation_root() {
    for (loader, name) in [
        (r"\input{aliases}", "aliases.tex"),
        (r"\usepackage{aliases}", "aliases.sty"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\documentclass{{article}}{loader}\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.png}}\\input{{child}}");
        std::fs::write(
            root.join(name),
            r"\def\Image{\includegraphics}\def\Chain{\Image}",
        )
        .unwrap();
        std::fs::write(root.join("child.tex"), r"\csname Chain\endcsname{plot}").unwrap();
        let workspace = project(root, &source);
        let child = uri(root, "child.tex");
        assert_eq!(
            targets(&workspace, &child),
            [uri(root, "assets/plot.png")],
            "loader: {loader}; root: {:?}",
            workspace.root_document(&child)
        );
        let changed = changes(
            &workspace,
            &[rename(root, "assets/plot.png", "assets/chart.png")],
        );
        assert_eq!(changed[&child], r"\csname Chain\endcsname{chart}");
    }
}

#[test]
fn quoted_consumed_deferred_or_unproven_image_calls_do_not_choose_files() {
    for body in [
        r"\string\Image{plot}",
        r"\detokenize{\Image{plot}}",
        r"\def\Later{\Image{plot}}",
        r"\def\Ignore#1{}\Ignore{\Image{plot}}",
        r"\let\Snapshot=\includegraphics\Snapshot{plot}",
        r"\def\Wrapper#1{\includegraphics{#1}}\Wrapper{plot}",
        r"\csname \dynamic\endcsname{plot}",
        r"\def\Image#1{other}\Image{plot}",
        r"\def\includegraphics#1{other}\Image{plot}",
        r"\def\csuse#1{other}\csuse{Image}{plot}",
        r"\Image**{plot}",
        r"\Image[a][b][c]{plot}",
        r"\Image[unfinished{plot}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source =
            format!("\\def\\Image{{\\includegraphics}}\\graphicspath{{{{assets/}}}}{body}");
        let workspace = project(directory.path(), &source);
        let main = uri(directory.path(), "main.tex");
        assert!(targets(&workspace, &main).is_empty(), "{source}");
        assert!(graphics::images(&workspace, &main).is_empty(), "{source}");
        assert!(
            changes(
                &workspace,
                &[rename(
                    directory.path(),
                    "assets/plot.pdf",
                    "assets/chart.pdf"
                )]
            )
            .is_empty(),
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex, graphicx and etoolbox"]
fn real_image_aliases_keep_recorder_inputs_after_image_and_root_moves() {
    for call in [r"\Image", r"\csuse{Image}", r"\csname Image\endcsname"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\documentclass{{article}}\\usepackage{{graphicx,etoolbox}}\\def\\Image{{\\includegraphics}}\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.png}}\\begin{{document}}{call}*[width=1cm]{{plot}}\\end{{document}}");
        let mut workspace = project(root, &source);
        std::fs::write(root.join("assets/plot.png"), PNG).unwrap();
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        for moved in [false, true] {
            let main_name = if moved { "book/paper.tex" } else { "main.tex" };
            let image = uri(
                root,
                if moved {
                    "assets/chart.png"
                } else {
                    "assets/plot.png"
                },
            );
            assert_eq!(
                targets(&workspace, &uri(root, main_name)).as_slice(),
                std::slice::from_ref(&image)
            );
            let outcome =
                latex_lsp::build::run(&root.join(main_name), &config, &Default::default());
            assert_eq!(outcome.status, "success", "{call}: {}", outcome.output);
            let inputs = outcome.inputs.unwrap();
            assert!(inputs.contains(&image), "{call}: {inputs:?}");
            assert!(!inputs.contains(&uri(root, "assets/plot.pdf")));
            if !moved {
                let files = [
                    rename(root, "assets/plot.png", "assets/chart.png"),
                    rename(root, "main.tex", "book/paper.tex"),
                ];
                for (document, source) in changes(&workspace, &files) {
                    std::fs::write(document.to_file_path().unwrap(), source).unwrap();
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
    }
}

#[test]
#[ignore = "requires pdflatex, graphicx and etoolbox"]
fn real_missing_alias_images_have_precise_diagnostics_and_repairable_names() {
    for call in [r"\Image", r"\csuse{Image}", r"\csname Image\endcsname"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\documentclass{{article}}\n\\usepackage{{graphicx,etoolbox}}\n\\def\\Image{{\\includegraphics}}\n\\graphicspath{{{{assets/}}}}\n\\begin{{document}}\n{call}[width=1cm]{{plto}}\n\\end{{document}}\n");
        let mut workspace = project(root, &source);
        workspace.code_action_literals = true;
        std::fs::write(root.join("assets/plot.png"), PNG).unwrap();
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let outcome = latex_lsp::build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(outcome.status, "error", "{call}: {}", outcome.output);
        let main = uri(root, "main.tex");
        let diagnostic = outcome.diagnostics[&main]
            .iter()
            .find(|item| item.code == Some(NumberOrString::String("missing-image".into())))
            .unwrap_or_else(|| panic!("{call}: {:?}\n{}", outcome.diagnostics, outcome.output))
            .clone();
        let text = Text::new(source.clone());
        let start = source.find("plto").unwrap();
        assert_eq!(diagnostic.range, text.range(start, start + 4));
        workspace
            .build_diagnostics
            .insert(main.clone(), outcome.diagnostics);
        let fixes = latex_lsp::editing::code_actions(
            &workspace,
            &main,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                ..Default::default()
            },
        );
        let edit = fixes
            .iter()
            .find_map(|item| match item {
                CodeActionOrCommand::CodeAction(action) => action
                    .edit
                    .as_ref()?
                    .changes
                    .as_ref()?
                    .get(&main)?
                    .iter()
                    .find(|edit| edit.new_text == "plot.png"),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{call}: {fixes:?}"));
        let repaired = text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        std::fs::write(root.join("main.tex"), &repaired.source).unwrap();
        let outcome = latex_lsp::build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(outcome.status, "success", "{call}: {}", outcome.output);
        assert!(outcome
            .inputs
            .unwrap()
            .contains(&uri(root, "assets/plot.png")));
    }
}
