use lsp_types::*;
use std::{collections::BTreeMap, path::Path};
use texquill::{
    editing::file_operations,
    language::{features, navigation},
    project::{graphics, workspace::Workspace},
    text::Text,
};

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
fn declared_extension_order_is_separate_from_explicit_graphics_rules() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = concat!(
        "\\graphicspath{{assets/}}\\includegraphics{plot}\n",
        "\\DeclareGraphicsExtensions{ .png , % first\n .pdf }\n",
        "\\includegraphics{plot}\\includegraphics{plot.pdf}\n",
        "\\DeclareGraphicsExtensions{.png}\\includegraphics{plot.pdf}\n",
        "\\DeclareGraphicsExtensions{}\\includegraphics{plot}\\includegraphics{plot.png}\n",
    );
    let workspace = project(root, source);
    let main = uri(root, "main.tex");
    assert_eq!(
        targets(&workspace, &main),
        ["plot.pdf", "plot.png", "plot.pdf", "plot.pdf", "plot.png"]
            .map(|name| uri(root, &format!("assets/{name}")))
    );
    assert_eq!(
        graphics::images(&workspace, &main),
        [uri(root, "assets/plot.pdf"), uri(root, "assets/plot.png")].into()
    );
    let source = source.find("\\includegraphics{plot}").unwrap() + 18;
    assert_eq!(
        features::definitions(
            &workspace,
            &main,
            Text::new(workspace.documents[&main].text.source.clone()).position(source)
        )[0]
        .uri,
        uri(root, "assets/plot.pdf")
    );
}

#[test]
fn settings_follow_groups_environments_inputs_and_package_load_order() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(
        root,
        concat!(
        "\\graphicspath{{assets/}}\\DeclareGraphicsExtensions{.png,.pdf}\n",
        "{\\DeclareGraphicsExtensions{.pdf,.png}\\includegraphics{plot}}\n",
        "\\includegraphics{plot}\n",
        "\\begingroup\\DeclareGraphicsExtensions{.pdf}\\includegraphics{plot}\\endgroup\n",
        "\\begin{center}\\DeclareGraphicsExtensions{.pdf}\\includegraphics{plot}\\end{center}\n",
        "\\includegraphics{plot}\n",
        "\\input{settings}\\includegraphics{plot}\n",
        "\\usepackage{local}\\includegraphics{plot}\n",
        "\\usepackage{local}\\includegraphics{plot}\n",
    ),
    );
    drop(workspace);
    std::fs::write(
        root.join("settings.tex"),
        r"\DeclareGraphicsExtensions{.pdf}",
    )
    .unwrap();
    std::fs::write(root.join("local.sty"), r"\DeclareGraphicsExtensions{.png}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert_eq!(
        targets(&workspace, &uri(root, "main.tex")),
        ["pdf", "png", "pdf", "pdf", "png", "pdf", "png", "png"]
            .map(|extension| uri(root, &format!("assets/plot.{extension}")))
    );
}

#[test]
fn repeated_shared_inputs_with_conflicting_settings_do_not_guess_a_target() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(
        root,
        concat!(
            "\\graphicspath{{assets/}}\\DeclareGraphicsExtensions{.png}\\input{shared}\n",
            "\\DeclareGraphicsExtensions{.pdf}\\input{shared}\n",
        ),
    );
    drop(workspace);
    std::fs::write(root.join("shared.tex"), r"\includegraphics{plot}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert!(targets(&workspace, &uri(root, "shared.tex")).is_empty());
}

#[test]
fn extension_order_guides_renames_and_batch_shadow_checks() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\graphicspath{{assets/}}\DeclareGraphicsExtensions{.png,.pdf}\includegraphics{plot}\includegraphics{plot.pdf}";
    let workspace = project(root, source);
    let main = uri(root, "main.tex");
    let changed = changes(
        &workspace,
        &[rename(root, "assets/plot.png", "assets/chart.png")],
    );
    assert_eq!(changed[&main], source.replace("{plot}", "{chart}"));
    let changed = changes(
        &workspace,
        &[rename(root, "assets/plot.pdf", "assets/chart.pdf")],
    );
    assert_eq!(changed[&main], source.replace("{plot.pdf}", "{chart.pdf}"));
    let changed = changes(
        &workspace,
        &[rename(root, "assets/plot.png", "assets/figure.png")],
    );
    assert_eq!(changed[&main], source.replace("{plot}", "{figure}"));
    // A new PNG shadows a PDF that was selected before the move.
    let mut workspace = workspace;
    workspace.open(main.clone(), source.replace(".png,.pdf", ".pdf,.png"), 1);
    std::fs::write(root.join("incoming.pdf"), "new shadow").unwrap();
    let changed = changes(&workspace, &[rename(root, "incoming.pdf", "plot.pdf")]);
    assert!(changed[&main].contains(r"\includegraphics{assets/plot.pdf}"));
}

#[test]
fn dynamic_and_overridden_settings_do_not_create_speculative_links_or_moves() {
    for setting in [
        r"\DeclareGraphicsExtensions{\chosen}",
        r"\DeclareGraphicsExtensions[option]{.png}",
        r"\DeclareGraphicsExtensions*{.png}",
        r"\newcommand{\choose}{\DeclareGraphicsExtensions{.png}}\choose",
        r"\def\DeclareGraphicsExtensions#1{}\DeclareGraphicsExtensions{.png}",
        r"\DeclareGraphicsRule{.png}{png}{.png}{}",
        r"\DeclareGraphicsExtensions{.png,,.pdf}",
        r"\iftrue\DeclareGraphicsExtensions{.png}\fi",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\graphicspath{{{{assets/}}}}{setting}\\includegraphics{{plot}}");
        let workspace = project(root, &source);
        assert!(
            targets(&workspace, &uri(root, "main.tex")).is_empty(),
            "{setting}"
        );
        assert!(
            changes(
                &workspace,
                &[rename(root, "assets/plot.pdf", "assets/chart.pdf")]
            )
            .is_empty(),
            "{setting}"
        );
    }
}

#[test]
fn comments_verbatim_and_escaped_braces_preserve_the_active_setting() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(
        root,
        concat!(
            "\\graphicspath{{assets/}}\\DeclareGraphicsExtensions{.png}\n",
            "% \\DeclareGraphicsExtensions{.pdf}\n",
            "\\verb|\\DeclareGraphicsExtensions{.pdf}|\n",
            "\\detokenize{\\DeclareGraphicsExtensions{.pdf}}\n",
            "\\begin{verbatim}\n\\DeclareGraphicsExtensions{.pdf}\n\\end{verbatim}\n",
            "\\{\\includegraphics{plot}\\}\n",
        ),
    );
    assert_eq!(
        targets(&workspace, &uri(root, "main.tex")),
        [uri(root, "assets/plot.png")]
    );
}

#[test]
fn shared_roots_keep_their_own_settings_and_reject_conflicting_path_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(
        root,
        r"\documentclass{article}\graphicspath{{assets/}}\DeclareGraphicsExtensions{.png}\input{shared}",
    );
    drop(workspace);
    std::fs::write(root.join("other.tex"), r"\documentclass{article}\graphicspath{{assets/}}\DeclareGraphicsExtensions{.pdf}\input{shared}").unwrap();
    std::fs::write(root.join("shared.tex"), r"\includegraphics{plot}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let shared = uri(root, "shared.tex");
    assert!(targets(&workspace, &shared).is_empty());
    let error = file_operations::will_rename(
        &workspace,
        &[rename(root, "assets/plot.png", "assets/chart.png")],
    )
    .unwrap_err();
    assert!(error.message.contains("different replacements"), "{error}");
}

#[test]
#[ignore = "requires pdflatex and graphicx"]
fn real_declared_extensions_keep_the_selected_images_after_batched_moves() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let config = texquill::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let drawing = tempfile::tempdir().unwrap();
    std::fs::write(
        drawing.path().join("drawing.tex"),
        r"\documentclass{article}\begin{document}Figure\end{document}",
    )
    .unwrap();
    let outcome = texquill::compilation::build::run(
        &drawing.path().join("drawing.tex"),
        &config,
        &Default::default(),
    );
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    let pdf = outcome.pdf.unwrap();
    let workspace = project(
        root,
        concat!(
            "\\documentclass{article}\\usepackage{graphicx}\\graphicspath{{assets/}}\n",
            "\\DeclareGraphicsExtensions{.png,.pdf}\\begin{document}\n",
            "\\includegraphics{plot}\\includegraphics{plot.pdf}\n",
            "{\\DeclareGraphicsExtensions{.pdf}\\includegraphics{plot}}\n",
            "\\input{child}\\end{document}\n",
        ),
    );
    drop(workspace);
    std::fs::write(root.join("child.tex"), r"\includegraphics{plot}").unwrap();
    for name in ["plot.pdf", "chart.pdf"] {
        std::fs::copy(&pdf, root.join("assets").join(name)).unwrap();
    }
    std::fs::write(root.join("assets/plot.png"), PNG).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for moved in [false, true] {
        let main = uri(root, if moved { "book/main.tex" } else { "main.tex" });
        let selected = uri(
            root,
            if moved {
                "assets/chart.png"
            } else {
                "assets/plot.png"
            },
        );
        assert_eq!(
            targets(&workspace, &main),
            [
                selected.clone(),
                uri(root, "assets/plot.pdf"),
                uri(root, "assets/plot.pdf")
            ]
        );
        assert_eq!(
            targets(&workspace, &uri(root, "child.tex")).as_slice(),
            std::slice::from_ref(&selected)
        );
        let outcome = texquill::compilation::build::run(
            &main.to_file_path().unwrap(),
            &config,
            &Default::default(),
        );
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        let inputs = outcome.inputs.unwrap();
        assert!(inputs.contains(&selected));
        assert!(inputs.contains(&uri(root, "assets/plot.pdf")));
        assert!(!inputs.contains(&uri(root, "assets/chart.pdf")));
        if !moved {
            let files = [
                rename(root, "assets/plot.png", "assets/chart.png"),
                rename(root, "main.tex", "book/main.tex"),
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

#[test]
fn endinput_and_closing_braces_at_file_boundaries_preserve_caller_settings() {
    for child_source in [
        "\\DeclareGraphicsExtensions{.pdf}\\endinput\n}\n",
        "\\DeclareGraphicsExtensions{.pdf}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(
            root,
            r"\graphicspath{{assets/}}\DeclareGraphicsExtensions{.png}{\input{settings}\includegraphics{plot}}\includegraphics{plot}",
        );
        drop(workspace);
        std::fs::write(root.join("settings.tex"), child_source).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        assert_eq!(
            targets(&workspace, &uri(root, "main.tex")),
            [uri(root, "assets/plot.pdf"), uri(root, "assets/plot.png")]
        );
    }
}

#[test]
fn excessive_group_depth_and_recursive_inputs_stop_static_inference() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = format!(
        "\\DeclareGraphicsExtensions{{.png}}{}\\includegraphics{{assets/plot}}{}",
        "{".repeat(300),
        "}".repeat(300)
    );
    let workspace = project(root, &source);
    assert!(targets(&workspace, &uri(root, "main.tex")).is_empty());
    std::fs::write(
        root.join("main.tex"),
        r"\DeclareGraphicsExtensions{.png}\input{loop}\includegraphics{assets/plot}",
    )
    .unwrap();
    std::fs::write(root.join("loop.tex"), r"\input{main}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert!(targets(&workspace, &uri(root, "main.tex")).is_empty());
}

#[test]
fn recorder_only_settings_without_a_proven_execution_order_do_not_assume_defaults() {
    for (definition, setting) in [
        ("", r"\DeclareGraphicsExtensions{.png}"),
        ("", r"\csuse{DeclareGraphicsExtensions}{.png}"),
        ("", r"\csname DeclareGraphicsExtensions\endcsname{.png}"),
        (r"\def\Ext{\DeclareGraphicsExtensions}", r"\Ext{.png}"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut workspace = project(
            root,
            &format!("{definition}\\documentclass{{article}}\\graphicspath{{{{assets/}}}}\\includegraphics{{plot}}"),
        );
        let settings = uri(root, "generated.sty");
        std::fs::write(settings.to_file_path().unwrap(), setting).unwrap();
        workspace.reload(&settings);
        let main = uri(root, "main.tex");
        workspace
            .project
            .recorded_inputs
            .insert(main.clone(), [main.clone(), settings.clone()].into());
        assert!(workspace.project_documents(&main).contains(&settings));
        assert!(targets(&workspace, &main).is_empty(), "{setting}");
    }
}

#[test]
fn global_extension_assignments_follow_tex_scope_rules() {
    for (setting, expected) in [
        (r"{\global\DeclareGraphicsExtensions{.png}}", "png"),
        (
            r"{\globaldefs=1 \DeclareGraphicsExtensions{.png}\globaldefs=0 }",
            "png",
        ),
        (
            r"{\globaldefs=-1 \global\DeclareGraphicsExtensions{.png}}",
            "pdf",
        ),
        (
            r"{{\global\DeclareGraphicsExtensions{.png}}\DeclareGraphicsExtensions{.pdf}}",
            "png",
        ),
        (
            r"{\globaldefs=1 \DeclareGraphicsExtensions{.png}\globaldefs=0 \DeclareGraphicsExtensions{.pdf}}",
            "png",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source=format!("\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.pdf}}{setting}\\includegraphics{{plot}}");
        let workspace = project(root, &source);
        let main = uri(root, "main.tex");
        assert_eq!(
            targets(&workspace, &main),
            [uri(root, &format!("assets/plot.{expected}"))],
            "{setting}"
        );
        let changed = changes(
            &workspace,
            &[rename(
                root,
                &format!("assets/plot.{expected}"),
                &format!("assets/chart.{expected}"),
            )],
        );
        assert_eq!(
            changed[&main],
            source.replace("{plot}", "{chart}"),
            "{setting}"
        );
    }
}

#[test]
fn literal_globaldefs_values_and_static_prefix_aliases_keep_their_operands() {
    for value in ["1", "+ - - 1", "'1", "\"1", "`A", "`\\A", "0% join\n 1"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source=format!("\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.pdf}}{{\\globaldefs={value} \\DeclareGraphicsExtensions{{.png}}\\globaldefs=0 }}\\includegraphics{{plot}}");
        let workspace = project(root, &source);
        assert_eq!(
            targets(&workspace, &uri(root, "main.tex")),
            [uri(root, "assets/plot.png")],
            "{value}"
        );
    }
    for (definitions, prefix, parameter) in [
        (r"\def\G{\global}\def\D{\globaldefs}", r"\G", r"\D"),
        ("", r"\csuse{global}", r"\csuse{globaldefs}"),
        (
            "",
            r"\csname global\endcsname",
            r"\csname globaldefs\endcsname",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source=format!("{definitions}\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.pdf}}{{{prefix}\\DeclareGraphicsExtensions{{.png}}}}\\includegraphics{{plot}}{{{parameter}=-1 {prefix}\\DeclareGraphicsExtensions{{.pdf}}}}\\includegraphics{{plot}}");
        let workspace = project(root, &source);
        assert_eq!(
            targets(&workspace, &uri(root, "main.tex")),
            [uri(root, "assets/plot.png"), uri(root, "assets/plot.png")],
            "{source}"
        );
    }
}

#[test]
fn global_settings_cross_input_boundaries_and_unrelated_global_assignments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(
        root,
        r"\graphicspath{{assets/}}\DeclareGraphicsExtensions{.pdf}{\global\input{settings}}\includegraphics{plot}\global\def\unrelated{Text}\includegraphics{plot}",
    );
    drop(workspace);
    std::fs::write(
        root.join("settings.tex"),
        r"\DeclareGraphicsExtensions{.png}",
    )
    .unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert_eq!(
        targets(&workspace, &uri(root, "main.tex")),
        [uri(root, "assets/plot.png"), uri(root, "assets/plot.png")]
    );
}

#[test]
fn uncertain_globaldefs_assignments_and_deferred_scope_changes_do_not_guess() {
    for assignment in [
        r"\globaldefs=\chosen",
        r"\globaldefs=0\chosen",
        r"\globaldefs=2147483648 ",
        r"\globaldefs='9 ",
        r"\iftrue\globaldefs=1 \fi",
        r"\def\change{\globaldefs=1 }\change",
        r"\newcommand{\change}{\globaldefs=1 }\change",
        r"\def\csuse#1{}\csuse{global}",
        r"\def\csname#1\endcsname{}\csname global\endcsname",
        r"\def\endcsname{}\csname global\endcsname",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source=format!("\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.pdf}}{{{assignment}\\DeclareGraphicsExtensions{{.png}}}}\\includegraphics{{plot}}");
        let workspace = project(root, &source);
        assert!(
            targets(&workspace, &uri(root, "main.tex")).is_empty(),
            "{assignment}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex, graphicx and etoolbox"]
fn real_global_settings_match_tex_and_preserve_images_after_renaming() {
    let drawing = tempfile::tempdir().unwrap();
    let config = texquill::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    std::fs::write(
        drawing.path().join("drawing.tex"),
        r"\documentclass{article}\begin{document}Figure\end{document}",
    )
    .unwrap();
    let graphic = texquill::compilation::build::run(
        &drawing.path().join("drawing.tex"),
        &config,
        &Default::default(),
    );
    assert_eq!(graphic.status, "success", "{}", graphic.output);
    let pdf = graphic.pdf.unwrap();
    let mut cases = vec![
        (
            r"{\global\DeclareGraphicsExtensions{.png}}".to_owned(),
            "png",
        ),
        (
            r"{\globaldefs=-1 \global\DeclareGraphicsExtensions{.png}}".to_owned(),
            "pdf",
        ),
        (
            r"{{\global\DeclareGraphicsExtensions{.png}}\DeclareGraphicsExtensions{.pdf}}"
                .to_owned(),
            "png",
        ),
        (
            r"\def\G{\global}{\G\DeclareGraphicsExtensions{.png}}".to_owned(),
            "png",
        ),
        (
            r"{\csname global\endcsname\DeclareGraphicsExtensions{.png}}".to_owned(),
            "png",
        ),
    ];
    cases.extend([
        (
            r"\def\Ext{\DeclareGraphicsExtensions}{\global\Ext{.png}}".to_owned(),
            "png",
        ),
        (
            r"\newcommand{\Ext}{\DeclareGraphicsExtensions}{\Ext{.png}}".to_owned(),
            "pdf",
        ),
        (
            r"{\global\csname DeclareGraphicsExtensions\endcsname{.png}}".to_owned(),
            "png",
        ),
        (
            r"{\globaldefs=1 \csname DeclareGraphicsExtensions\endcsname{.png}\globaldefs=0 }"
                .to_owned(),
            "png",
        ),
        (
            r"{\globaldefs=-1 \global\csname DeclareGraphicsExtensions\endcsname{.png}}".to_owned(),
            "pdf",
        ),
        (
            r"{\global\csuse{DeclareGraphicsExtensions}{.png}}".to_owned(),
            "png",
        ),
        (
            r"\def\Ext{\DeclareGraphicsExtensions}\def\Chain{\Ext}{\global\csuse{Chain}{.png}}"
                .to_owned(),
            "png",
        ),
        (
            r"\def\Ext{\DeclareGraphicsExtensions}{\global\csname Ext\endcsname{.png}}".to_owned(),
            "png",
        ),
    ]);
    for value in ["1", "+ - - 1", "'1", "\"1", "`A", "`\\A", "0% join\n 1"] {
        cases.push((
            format!("{{\\globaldefs={value} \\DeclareGraphicsExtensions{{.png}}\\globaldefs=0 }}"),
            "png",
        ));
    }
    for (setting, extension) in cases {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source=format!("\\documentclass{{article}}\\usepackage{{graphicx,etoolbox}}\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.pdf}}\\begin{{document}}\n{setting}\\includegraphics{{plot}}\\end{{document}}\n");
        let mut workspace = project(root, &source);
        std::fs::copy(&pdf, root.join("assets/plot.pdf")).unwrap();
        std::fs::write(root.join("assets/plot.png"), PNG).unwrap();
        let main = uri(root, "main.tex");
        for moved in [false, true] {
            let name = if moved { "chart" } else { "plot" };
            let selected = uri(root, &format!("assets/{name}.{extension}"));
            assert_eq!(
                targets(&workspace, &main).as_slice(),
                std::slice::from_ref(&selected),
                "{setting}"
            );
            let outcome = texquill::compilation::build::run(
                &root.join("main.tex"),
                &config,
                &Default::default(),
            );
            assert_eq!(outcome.status, "success", "{setting}: {}", outcome.output);
            let inputs = outcome.inputs.unwrap();
            assert!(inputs.contains(&selected), "{setting}: {inputs:?}");
            let other = if extension == "png" { "pdf" } else { "png" };
            assert!(
                !inputs.contains(&uri(root, &format!("assets/plot.{other}"))),
                "{setting}: {inputs:?}"
            );
            if !moved {
                let files = [rename(
                    root,
                    &format!("assets/plot.{extension}"),
                    &format!("assets/chart.{extension}"),
                )];
                for (document, source) in changes(&workspace, &files) {
                    std::fs::write(document.to_file_path().unwrap(), source).unwrap();
                }
                std::fs::rename(
                    root.join(format!("assets/plot.{extension}")),
                    root.join(format!("assets/chart.{extension}")),
                )
                .unwrap();
                file_operations::did_rename(&mut workspace, &files).unwrap();
            }
        }
    }
}

#[test]
fn indirect_extension_declarations_follow_local_and_global_scope() {
    for (definition, call) in [
        (r"\def\Ext{\DeclareGraphicsExtensions}", r"\Ext"),
        (r"\newcommand{\Ext}{\DeclareGraphicsExtensions}", r"\Ext"),
        (
            r"\def\Ext{\DeclareGraphicsExtensions}\def\Chain{\Ext}",
            r"\Chain",
        ),
        ("", r"\csuse{DeclareGraphicsExtensions}"),
        ("", r"\csname DeclareGraphicsExtensions\endcsname"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("{definition}\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.pdf}}{{{call}{{.png}}\\includegraphics{{plot}}}}\\includegraphics{{plot}}{{\\global{call}{{.png}}}}\\includegraphics{{plot}}{{\\globaldefs=-1 \\global{call}{{.pdf}}}}\\includegraphics{{plot}}");
        let workspace = project(root, &source);
        let main = uri(root, "main.tex");
        assert_eq!(
            targets(&workspace, &main),
            ["png", "pdf", "png", "png"].map(|ext| uri(root, &format!("assets/plot.{ext}"))),
            "{source}"
        );
        let moved = changes(
            &workspace,
            &[rename(root, "assets/plot.png", "assets/chart.png")],
        );
        let expected = source.replacen("{plot}", "{chart}", 1);
        let split = expected.find("\\global").unwrap();
        let expected = format!(
            "{}{}",
            &expected[..split],
            expected[split..].replace("{plot}", "{chart}")
        );
        assert_eq!(moved[&main], expected, "{source}");
    }
}

#[test]
fn indirect_extension_settings_cross_static_inputs_and_package_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\graphicspath{{assets/}}\DeclareGraphicsExtensions{.pdf}\usepackage{local}\input{settings}\includegraphics{plot}";
    drop(project(root, source));
    std::fs::write(
        root.join("local.sty"),
        r"\ProvidesPackage{local}\def\Ext{\DeclareGraphicsExtensions}\def\Chain{\Ext}",
    )
    .unwrap();
    std::fs::write(root.join("settings.tex"), r"{\global\csuse{Chain}{.png}}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    assert_eq!(targets(&workspace, &main), [uri(root, "assets/plot.png")]);
    assert_eq!(
        graphics::images(&workspace, &main),
        [uri(root, "assets/plot.png")].into()
    );
    let moved = changes(
        &workspace,
        &[rename(root, "assets/plot.png", "assets/chart.png")],
    );
    assert_eq!(moved[&main], source.replace("{plot}", "{chart}"));
}

#[test]
fn unresolved_indirect_extension_calls_do_not_select_or_move_images() {
    for setting in [
        r"\def\Ext#1{\DeclareGraphicsExtensions{#1}}\Ext{.png}",
        r"\def\Ext{\DeclareGraphicsExtensions}\def\wrapper{\Ext{.png}}\wrapper",
        r"\csuse{DeclareGraphicsExtensions}[bad]{.png}",
        r"\csname DeclareGraphicsExtensions\endcsname*{.png}",
        r"\csuse{DeclareGraphicsRule}{.png}{png}{.png}{}",
        r"\def\csuse#1{}\csuse{DeclareGraphicsExtensions}{.png}",
        r"\let\Ext\DeclareGraphicsExtensions\Ext{.png}",
        r"\csuse{\chosen}{.png}",
        r"\csname DeclareGraphics\chosen\endcsname{.png}",
        r"\def\Ext{\DeclareGraphicsExtensions}\def\consume#1{}\consume\Ext{.png}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\graphicspath{{{{assets/}}}}\\DeclareGraphicsExtensions{{.pdf}}{setting}\\includegraphics{{plot}}");
        let workspace = project(root, &source);
        assert!(
            targets(&workspace, &uri(root, "main.tex")).is_empty(),
            "{setting}"
        );
        assert!(
            changes(
                &workspace,
                &[rename(root, "assets/plot.pdf", "assets/chart.pdf")]
            )
            .is_empty(),
            "{setting}"
        );
    }
}

#[test]
fn displayed_indirect_extension_tokens_do_not_change_settings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(
        root,
        r"\def\Ext{\DeclareGraphicsExtensions}\graphicspath{{assets/}}\DeclareGraphicsExtensions{.pdf}\string\Ext\detokenize{\csuse{\chosen}{.png}\csname DeclareGraphics\chosen\endcsname{.png}}\includegraphics{plot}",
    );
    assert_eq!(
        targets(&workspace, &uri(root, "main.tex")),
        [uri(root, "assets/plot.pdf")]
    );
}
