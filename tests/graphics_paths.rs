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
fn graphics_paths_follow_declaration_order_and_local_scope() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\graphicspath{{first/}}\includegraphics{plot}\graphicspath{{second/}}\includegraphics{plot}{\graphicspath{{local/}}\includegraphics{plot}}\includegraphics{plot}\graphicspath{}\includegraphics{plot}";
    for path in ["first", "second", "local"] {
        std::fs::create_dir(root.join(path)).unwrap();
        std::fs::write(root.join(path).join("plot.pdf"), path).unwrap();
    }
    let workspace = project(root, source);
    assert_eq!(
        targets(&workspace, &uri(root, "main.tex")),
        ["first", "second", "local", "second"].map(|path| uri(root, &format!("{path}/plot.pdf")))
    );
    let changed = changes(
        &workspace,
        &[rename(root, "second/plot.pdf", "second/chart.pdf")],
    );
    let expected = source
        .replacen(
            r"\graphicspath{{second/}}\includegraphics{plot}",
            r"\graphicspath{{second/}}\includegraphics{chart}",
            1,
        )
        .replacen(
            r"}\includegraphics{plot}\graphicspath{}",
            r"}\includegraphics{chart}\graphicspath{}",
            1,
        );
    assert_eq!(changed[&uri(root, "main.tex")], expected);
}

fn directory_project(root: &Path, source: &str) -> Workspace {
    for name in ["first", "second", "local"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
        for extension in ["pdf", "png"] {
            std::fs::write(root.join(format!("{name}/plot.{extension}")), "image").unwrap();
        }
    }
    project(root, source)
}

#[test]
fn global_paths_aliases_and_extension_assignments_preserve_their_own_scope() {
    for (setting, selected) in [
        (r"{\global\graphicspath{{second/}}}", "second/plot.pdf"),
        (
            r"{\globaldefs=1 \graphicspath{{second/}}\globaldefs=0 }",
            "second/plot.pdf",
        ),
        (
            r"{\globaldefs=-1 \global\graphicspath{{second/}}}",
            "first/plot.pdf",
        ),
        (
            r"{{\global\graphicspath{{second/}}}\graphicspath{{local/}}}",
            "second/plot.pdf",
        ),
        (
            r"\def\Paths{\graphicspath}\def\Chain{\Paths}{\global\Chain{{second/}}}",
            "second/plot.pdf",
        ),
        (
            r"{\global\csuse{graphicspath}{{second/}}}",
            "second/plot.pdf",
        ),
        (
            r"{\global\csname graphicspath\endcsname{{second/}}}",
            "second/plot.pdf",
        ),
        (
            r"{\global\DeclareGraphicsExtensions{.png}\graphicspath{{second/}}}",
            "first/plot.png",
        ),
        (
            r"{\global\graphicspath{{second/}}\DeclareGraphicsExtensions{.png}}",
            "second/plot.pdf",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\graphicspath{{{{first/}}}}{setting}\\includegraphics{{plot}}");
        let workspace = directory_project(root, &source);
        assert_eq!(
            targets(&workspace, &uri(root, "main.tex")),
            [uri(root, selected)],
            "{setting}"
        );
    }
}

#[test]
fn paths_follow_input_order_and_packages_only_once() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\def\Paths{\graphicspath}\graphicspath{{first/}}\input{settings}\includegraphics{plot}\usepackage{localpaths}\includegraphics{plot}\input{settings}\includegraphics{plot}\usepackage{localpaths}\includegraphics{plot}";
    drop(directory_project(root, source));
    std::fs::write(root.join("settings.tex"), r"\Paths{{second/}}").unwrap();
    std::fs::write(root.join("localpaths.sty"), r"\graphicspath{{local/}}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert_eq!(
        targets(&workspace, &uri(root, "main.tex")),
        ["second", "local", "second", "second"].map(|name| uri(root, &format!("{name}/plot.pdf")))
    );
}

#[test]
fn ambiguous_shared_inputs_and_dynamic_paths_do_not_guess_image_targets() {
    for setting in [
        r"\graphicspath{{\chosen/}}",
        r"\iftrue\graphicspath{{second/}}\fi",
        r"\def\Paths#1{\graphicspath{#1}}\Paths{{second/}}",
        r"\graphicspath{{first/}garbage}",
        r"\graphicspath[bad]{{second/}}",
        r"\csuse{\chosen}{{second/}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\graphicspath{{{{first/}}}}{setting}\\includegraphics{{plot}}");
        let workspace = directory_project(root, &source);
        assert!(
            targets(&workspace, &uri(root, "main.tex")).is_empty(),
            "{setting}"
        );
        assert!(
            changes(
                &workspace,
                &[rename(root, "first/plot.pdf", "first/chart.pdf")]
            )
            .is_empty(),
            "{setting}"
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    drop(directory_project(
        root,
        r"\graphicspath{{first/}}\input{shared}\graphicspath{{second/}}\input{shared}",
    ));
    std::fs::write(root.join("shared.tex"), r"\includegraphics{plot}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert!(targets(&workspace, &uri(root, "shared.tex")).is_empty());
}

#[test]
fn completions_use_the_directories_active_at_each_image_call() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source =
        r"\graphicspath{{first/}}\includegraphics{pl}\graphicspath{{second/}}\includegraphics{pl}";
    let workspace = directory_project(root, source);
    std::fs::write(root.join("first/plot-first.pdf"), "first").unwrap();
    std::fs::write(root.join("second/plot-second.pdf"), "second").unwrap();
    for (offset, selected, excluded) in [
        (
            source.find("{pl}").unwrap() + 3,
            "plot-first.pdf",
            "plot-second.pdf",
        ),
        (
            source.rfind("{pl}").unwrap() + 3,
            "plot-second.pdf",
            "plot-first.pdf",
        ),
    ] {
        let items = features::completion(
            &workspace,
            &uri(root, "main.tex"),
            Text::new(source.into()).position(offset),
        );
        assert!(items.iter().any(|item| item.label == selected), "{items:?}");
        assert!(
            !items.iter().any(|item| item.label == excluded),
            "{items:?}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex, graphicx and etoolbox"]
fn real_path_scope_and_batched_directory_and_root_moves_preserve_images() {
    for (setting, selected) in [
        (r"\graphicspath{{second/}}", "second"),
        (r"{\graphicspath{{second/}}}", "first"),
        (r"{\global\graphicspath{{second/}}}", "second"),
        (
            r"{\globaldefs=1 \graphicspath{{second/}}\globaldefs=0 }",
            "second",
        ),
        (r"{\globaldefs=-1 \global\graphicspath{{second/}}}", "first"),
        (
            r"\def\Paths{\graphicspath}{\global\Paths{{second/}}}",
            "second",
        ),
        (r"{\global\csuse{graphicspath}{{second/}}}", "second"),
        (
            r"{\global\csname graphicspath\endcsname{{second/}}}",
            "second",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\documentclass{{article}}\\usepackage{{graphicx,etoolbox}}\\graphicspath{{{{first/}}}}\\DeclareGraphicsExtensions{{.png}}\\begin{{document}}{setting}\\includegraphics{{plot}}\\end{{document}}");
        let mut workspace = directory_project(root, &source);
        for name in ["first", "second", "local"] {
            std::fs::write(root.join(format!("{name}/plot.png")), PNG).unwrap();
        }
        let config = texquill::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        for moved in [false, true] {
            let (main_name, path) = if moved {
                ("book/paper.tex", format!("moved/{selected}/plot.png"))
            } else {
                ("main.tex", format!("{selected}/plot.png"))
            };
            let main = uri(root, main_name);
            assert_eq!(targets(&workspace, &main), [uri(root, &path)], "{setting}");
            let outcome = texquill::compilation::build::run(
                &root.join(main_name),
                &config,
                &Default::default(),
            );
            assert_eq!(outcome.status, "success", "{setting}: {}", outcome.output);
            let inputs = outcome.inputs.unwrap();
            assert!(inputs.contains(&uri(root, &path)), "{setting}: {inputs:?}");
            let other = if selected == "first" {
                "second"
            } else {
                "first"
            };
            assert!(
                !inputs.contains(&uri(root, &format!("{other}/plot.png"))),
                "{setting}: {inputs:?}"
            );
            if !moved {
                std::fs::create_dir(root.join("book")).unwrap();
                std::fs::create_dir(root.join("moved")).unwrap();
                let files = [
                    rename(root, selected, &format!("moved/{selected}")),
                    rename(root, "main.tex", "book/paper.tex"),
                ];
                for (document, source) in changes(&workspace, &files) {
                    std::fs::write(document.to_file_path().unwrap(), source).unwrap();
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
}

#[test]
fn recorder_only_path_settings_with_unknown_execution_order_do_not_choose_images() {
    for setting in [
        r"\graphicspath{{second/}}",
        r"\csuse{graphicspath}{{second/}}",
        r"\Paths{{second/}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut workspace = directory_project(
            root,
            r"\def\Paths{\graphicspath}\graphicspath{{first/}}\includegraphics{plot}",
        );
        let settings = uri(root, "generated.sty");
        std::fs::write(settings.to_file_path().unwrap(), setting).unwrap();
        workspace.reload(&settings);
        let main = uri(root, "main.tex");
        workspace
            .project
            .recorded_inputs
            .insert(main.clone(), [main.clone(), settings].into());
        assert!(targets(&workspace, &main).is_empty(), "{setting}");
    }
}
