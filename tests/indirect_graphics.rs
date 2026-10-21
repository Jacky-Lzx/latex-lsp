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

#[test]
fn recorder_sources_keep_image_links_completion_and_file_rename_edits() {
    for call in [
        r"\includegraphics",
        r"\Image",
        r"\csuse{Image}",
        r"\csname Image\endcsname",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = r"\documentclass{article}\def\Image{\includegraphics}\def\Chapter{generated}\input{\Chapter}";
        let mut workspace = project(root, source);
        let main = uri(root, "main.tex");
        let generated = uri(root, "generated.tex");
        let body = format!("{call}*[width=1cm]{{plot.png}}");
        std::fs::write(root.join("generated.tex"), &body).unwrap();
        std::fs::write(root.join("plot.png"), PNG).unwrap();
        workspace.reload(&generated);
        workspace
            .recorded_inputs
            .insert(main.clone(), [main.clone(), generated.clone()].into());
        assert_eq!(
            targets(&workspace, &generated),
            [uri(root, "plot.png")],
            "{call}"
        );
        assert_eq!(
            graphics::images(&workspace, &generated),
            [uri(root, "plot.png")].into(),
            "{call}"
        );
        let offset = body.find("plot.png").unwrap() + 2;
        let items = features::completion(
            &workspace,
            &generated,
            Text::new(body.clone()).position(offset),
        );
        assert!(
            items.iter().any(|item| item.label == "plot.png"),
            "{call}: {items:?}"
        );
        let changed = changes(&workspace, &[rename(root, "plot.png", "chart.png")]);
        assert_eq!(
            changed[&generated],
            body.replace("plot.png", "chart.png"),
            "{call}"
        );
        assert!(workspace
            .project_files(&main)
            .contains(&uri(root, "plot.png")));
    }
}

#[test]
fn recorder_definitions_determine_image_meaning_and_argument_boundaries() {
    for (declarations, body, expected) in [
        (
            r"\def\Image{\includegraphics}\def\Chain{\Image}",
            r"\Chain{plot.png}",
            true,
        ),
        (
            r"\def\Image{\includegraphics}",
            r"\csuse{Image}{plot.png}",
            true,
        ),
        (
            r"\def\includegraphics#1{}",
            r"\includegraphics{plot.png}",
            false,
        ),
        (
            r"\def\Hold#1{}",
            r"\Hold{\includegraphics{plot.png}}",
            false,
        ),
        (r"\def\Hold#1{}", r"\Hold\includegraphics{plot.png}", false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!(
            "\\documentclass{{article}}\\def\\Chapter{{generated}}\\input{{\\Chapter}}{body}"
        );
        let mut workspace = project(root, &source);
        let main = uri(root, "main.tex");
        let generated = uri(root, "generated.tex");
        std::fs::write(root.join("generated.tex"), declarations).unwrap();
        std::fs::write(root.join("plot.png"), PNG).unwrap();
        workspace.reload(&generated);
        workspace
            .recorded_inputs
            .insert(main.clone(), [main.clone(), generated].into());
        assert_eq!(
            targets(&workspace, &main).len(),
            usize::from(expected),
            "{declarations}: {body}"
        );
        let offset = source.find("plot.png").unwrap() + 2;
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        );
        assert_eq!(
            items.iter().any(|item| item.label == "plot.png"),
            expected,
            "{declarations}: {items:?}"
        );
        let changed = changes(&workspace, &[rename(root, "plot.png", "chart.png")]);
        if expected {
            assert_eq!(changed[&main], source.replace("plot.png", "chart.png"));
        } else {
            assert!(changed.is_empty(), "{declarations}: {changed:?}");
        }
    }
}

#[test]
fn shared_recorded_sources_keep_root_specific_targets_and_reject_partial_moves() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(
            root.join(name).join("main.tex"),
            r"\documentclass{article}\def\Image{\includegraphics}",
        )
        .unwrap();
        std::fs::write(root.join(name).join("plot.png"), PNG).unwrap();
    }
    let body = r"\Image{plot.png}";
    std::fs::write(root.join("shared.tex"), body).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let shared = uri(root, "shared.tex");
    for name in ["a", "b"] {
        let main = uri(root, &format!("{name}/main.tex"));
        workspace
            .recorded_inputs
            .insert(main.clone(), [main, shared.clone()].into());
    }
    assert!(targets(&workspace, &shared).is_empty());
    assert!(graphics::images(&workspace, &shared).is_empty());
    let error =
        file_operations::will_rename(&workspace, &[rename(root, "a/plot.png", "a/chart.png")])
            .unwrap_err();
    assert!(error.message.contains("different replacements"), "{error}");
}

#[test]
fn recorded_image_fixes_recheck_alias_definitions_before_editing() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root, r"\documentclass{article}\def\Image{\includegraphics}");
    let main = uri(root, "main.tex");
    let generated = uri(root, "generated.tex");
    let body = r"\Image[width=1cm]{plto.png}";
    std::fs::write(root.join("generated.tex"), body).unwrap();
    std::fs::write(root.join("plot.png"), PNG).unwrap();
    workspace.reload(&generated);
    workspace
        .recorded_inputs
        .insert(main.clone(), [main.clone(), generated.clone()].into());
    workspace.code_action_literals = true;
    let offset = body.find("plto.png").unwrap();
    let diagnostic = Diagnostic {
        range: Text::new(body.into()).range(offset, offset + 8),
        source: Some("latex-build".into()),
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String("missing-image".into())),
        message: "LaTeX Error: File `plto.png' not found.".into(),
        data: Some(serde_json::json!({"file":"plto.png"})),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        main.clone(),
        [(generated.clone(), vec![diagnostic.clone()])].into(),
    );
    let fixes = |workspace: &Workspace| {
        latex_lsp::editing::code_actions(
            workspace,
            &generated,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic.clone()],
                ..Default::default()
            },
        )
    };
    assert_eq!(fixes(&workspace).len(), 1);
    workspace.open(main, r"\documentclass{article}\def\Image#1{}".into(), 2);
    assert!(fixes(&workspace).is_empty());
}

#[test]
#[ignore = "requires pdflatex, graphicx and etoolbox"]
fn real_dynamic_source_images_keep_navigation_and_build_inputs_after_moves() {
    for call in [r"\Image", r"\csuse{Image}", r"\csname Image\endcsname"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = r"\documentclass{article}\usepackage{graphicx,etoolbox}\def\Image{\includegraphics}\def\Chapter{generated}\begin{document}\input{\Chapter}\end{document}";
        let mut workspace = project(root, source);
        let main = uri(root, "main.tex");
        let generated = uri(root, "generated.tex");
        std::fs::write(
            root.join("generated.tex"),
            format!("{call}*[width=1cm]{{plot.png}}"),
        )
        .unwrap();
        std::fs::write(root.join("plot.png"), PNG).unwrap();
        workspace.reload(&generated);
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        for moved in [false, true] {
            let image = uri(
                root,
                if moved {
                    "assets/chart.png"
                } else {
                    "plot.png"
                },
            );
            let outcome =
                latex_lsp::build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(outcome.status, "success", "{call}: {}", outcome.output);
            let inputs = outcome.inputs.unwrap();
            assert!(inputs.contains(&generated));
            assert!(inputs.contains(&image));
            workspace
                .recorded_inputs
                .insert(main.clone(), workspace.project_inputs(&main, &inputs));
            assert_eq!(
                targets(&workspace, &generated).as_slice(),
                std::slice::from_ref(&image)
            );
            assert!(workspace.project_files(&main).contains(&image));
            if !moved {
                let files = [rename(root, "plot.png", "assets/chart.png")];
                for (document, source) in changes(&workspace, &files) {
                    std::fs::write(document.to_file_path().unwrap(), source).unwrap();
                }
                std::fs::rename(root.join("plot.png"), root.join("assets/chart.png")).unwrap();
                file_operations::did_rename(&mut workspace, &files).unwrap();
                workspace.reload(&generated);
            }
        }
    }
}

#[test]
fn displayed_recorder_files_do_not_define_image_aliases_until_input_as_source() {
    for executed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let loader = if executed { r"\input{generated}" } else { "" };
        let source = format!("\\documentclass{{article}}\\lstinputlisting{{generated.tex}}{loader}\\Image{{plot.png}}");
        std::fs::write(root.join("generated.tex"), r"\def\Image{\includegraphics}").unwrap();
        let mut workspace = project(root, &source);
        std::fs::write(root.join("plot.png"), PNG).unwrap();
        let main = uri(root, "main.tex");
        workspace.recorded_inputs.insert(
            main.clone(),
            [main.clone(), uri(root, "generated.tex")].into(),
        );
        assert_eq!(targets(&workspace, &main).len(), usize::from(executed));
    }
}

#[test]
fn incomplete_static_discovery_does_not_reintroduce_image_guesses() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("part.tex"), r"\def\input#1{}\end{x}").unwrap();
    let source = r"\documentclass{article}\def\readpart{\input}\begin{document}\begin{x}\readpart{part}\includegraphics{plot.png}\end{document}";
    let workspace = project(root, source);
    std::fs::write(root.join("plot.png"), PNG).unwrap();
    let main = uri(root, "main.tex");
    assert!(targets(&workspace, &main).is_empty());
    assert!(graphics::images(&workspace, &main).is_empty());
    let offset = source.find("plot.png").unwrap() + 2;
    let items = features::completion(&workspace, &main, Text::new(source.into()).position(offset));
    assert!(!items.iter().any(|item| item.label == "plot.png"));
    assert!(changes(&workspace, &[rename(root, "plot.png", "chart.png")]).is_empty());
}
