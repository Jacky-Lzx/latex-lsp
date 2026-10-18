use latex_lsp::{
    features, file_operations, graphics, navigation,
    text::Text,
    workspace::{Document, Workspace},
};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project(root: &Path) -> Workspace {
    for directory in ["assets.v1/nested", "图 片", "sections", "other", "book"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    for (name, source) in [
        ("main.tex", "\\documentclass{article}\n\\usepackage{local}\n\\graphicspath{{./assets.v1/}{图 片/}}\n\\begin{document}\n\\input{sections/child}\n\\end{document}\n"),
        ("local.sty", "\\usepackage{graphicx}\n\\graphicspath{{other/}}"),
        ("sections/child.tex", "\\includegraphics{plot}\n\\includegraphics{中文 图}\n\\includegraphics{nested/second}\n\\includegraphics{package}\n\\input{hidden}"),
        ("unrelated.tex", "\\documentclass{article}\\graphicspath{{secret/}}"),
        ("assets.v1/plot.pdf", "pdf"),
        ("assets.v1/nested/second.png", "png"),
        ("assets.v1/hidden.tex", ""),
        ("图 片/中文 图.pdf", "pdf"),
        ("other/package.pdf", "pdf"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

fn changed(workspace: &Workspace, files: &[FileRename]) -> BTreeMap<Url, String> {
    let Some(edit) = file_operations::will_rename(workspace, files).unwrap() else {
        return BTreeMap::new();
    };
    edit.changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let text = workspace.documents[&uri].text.clone();
            let text = text
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
            (uri, text)
        })
        .collect()
}

#[test]
fn project_graphics_paths_resolve_nested_unicode_and_package_images() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let child = uri(root, "sections/child.tex");
    let links = navigation::document_links(&workspace, &child);
    let targets: Vec<_> = links.into_iter().map(|link| link.target.unwrap()).collect();
    assert_eq!(
        targets,
        vec![
            uri(root, "assets.v1/plot.pdf"),
            uri(root, "图 片/中文 图.pdf"),
            uri(root, "assets.v1/nested/second.png"),
            uri(root, "other/package.pdf")
        ]
    );
    std::fs::write(root.join("plot.pdf"), "root").unwrap();
    assert_eq!(
        navigation::document_links(&workspace, &child)[0].target,
        Some(uri(root, "plot.pdf"))
    );
    std::fs::write(root.join("sections/plot.pdf"), "local").unwrap();
    assert_eq!(
        navigation::document_links(&workspace, &child)[0].target,
        Some(uri(root, "sections/plot.pdf"))
    );
    let main = uri(root, "main.tex");
    workspace.documents.insert(main.clone(), Document::new(&main, "\\documentclass{article}\\graphicspath{{\\unknown/}{missing/}{./}{assets.v1}}% \\graphicspath{{other/}}\n\\verb|\\graphicspath{{图 片/}}|".into(), Some(1)));
    let declarations = graphics::declarations(&workspace, &main);
    assert_eq!(declarations.len(), 2);
    assert_eq!(declarations[1].path, root.canonicalize().unwrap());
    assert_eq!(
        graphics::directories(&workspace, &main),
        vec![root.canonicalize().unwrap()]
    );
    let unrelated = uri(root, "unrelated.tex");
    assert!(graphics::directories(&workspace, &unrelated).is_empty());
}

#[test]
fn image_completion_searches_declared_paths_and_preserves_directory_prefixes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let child = uri(root, "sections/child.tex");
    for (source, label) in [
        ("\\includegraphics{pl", "plot.pdf"),
        ("\\includegraphics{nested/se", "nested/second.png"),
        ("\\includegraphics{中", "中文 图.pdf"),
    ] {
        workspace
            .documents
            .insert(child.clone(), Document::new(&child, source.into(), Some(1)));
        let items = features::completion(
            &workspace,
            &child,
            Text::new(source.into()).position(source.len()),
        );
        let item = items.iter().find(|item| item.label == label).unwrap();
        assert_eq!(
            item.detail.as_ref().unwrap(),
            &root
                .join(if label == "plot.pdf" {
                    "assets.v1/plot.pdf"
                } else if label.starts_with("nested") {
                    "assets.v1/nested/second.png"
                } else {
                    "图 片/中文 图.pdf"
                })
                .display()
                .to_string()
        );
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!();
        };
        assert_eq!(edit.new_text, label);
    }
    let source = "\\input{hid";
    workspace
        .documents
        .insert(child.clone(), Document::new(&child, source.into(), Some(2)));
    assert!(features::completion(
        &workspace,
        &child,
        Text::new(source.into()).position(source.len())
    )
    .is_empty());
}

#[test]
fn image_renames_keep_short_names_and_rebase_directory_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    let main = uri(root, "main.tex");
    let child = uri(root, "sections/child.tex");
    let files = [rename(root, "assets.v1/plot.pdf", "assets.v1/chart.pdf")];
    let edits = changed(&workspace, &files);
    assert!(edits[&child].contains("\\includegraphics{chart}"));
    assert!(!edits.contains_key(&main));
    let edits = changed(&workspace, &[rename(root, "assets.v1", "new.images")]);
    assert!(edits[&main].contains("\\graphicspath{{./new.images/}{图 片/}}"));
    assert!(!edits.contains_key(&child));
    let edits = changed(&workspace, &[rename(root, "main.tex", "book/paper.tex")]);
    assert!(edits[&main].contains("\\graphicspath{{../assets.v1/}{../图 片/}}"));
    assert!(edits[&uri(root, "local.sty")].contains("\\graphicspath{{../other/}}"));
    assert!(!edits.contains_key(&child));
    let edits = changed(
        &workspace,
        &[rename(root, "assets.v1/plot.pdf", "book/chart.pdf")],
    );
    assert!(edits[&child].contains("\\includegraphics{book/chart}"));
    assert!(changed(&workspace, &[rename(root, "unrelated.tex", "other.tex")]).is_empty());
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn graphics_directory_and_root_moves_still_compile() {
    let image = tempfile::tempdir().unwrap();
    std::fs::write(
        image.path().join("image.tex"),
        "\\documentclass{article}\\begin{document}Image\\end{document}",
    )
    .unwrap();
    let image_build = latex_lsp::build::run(
        &image.path().join("image.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(image_build.status, "success", "{}", image_build.output);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    // Use one graphicspath declaration; TeX's later declarations replace earlier ones.
    std::fs::write(root.join("local.sty"), "\\usepackage{graphicx}").unwrap();
    std::fs::write(
        root.join("sections/child.tex"),
        "\\includegraphics[width=1cm]{plot}",
    )
    .unwrap();
    std::fs::copy(image_build.pdf.unwrap(), root.join("assets.v1/plot.pdf")).unwrap();
    workspace.reload(&uri(root, "local.sty"));
    workspace.reload(&uri(root, "sections/child.tex"));
    let files = [
        rename(root, "assets.v1", "new.images"),
        rename(root, "main.tex", "book/paper.tex"),
    ];
    for (uri, source) in changed(&workspace, &files) {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    for file in &files {
        std::fs::rename(
            Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
            Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
        )
        .unwrap();
    }
    file_operations::did_rename(&mut workspace, &files).unwrap();
    let main = uri(root, "book/paper.tex");
    assert_eq!(
        navigation::document_links(&workspace, &uri(root, "sections/child.tex"))[0].target,
        Some(uri(root, "new.images/plot.pdf"))
    );
    let outcome = latex_lsp::build::run(
        &main.to_file_path().unwrap(),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(outcome.pdf.unwrap().is_file());
    assert!(outcome
        .inputs
        .unwrap()
        .contains(&uri(root, "new.images/plot.pdf")));
}

fn shared_project(root: &Path) -> Workspace {
    std::fs::write(
        root.join("common.sty"),
        "\\usepackage{graphicx}\n\\graphicspath{{assets/}{future/}}\n",
    )
    .unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir_all(root.join(format!("{name}/assets"))).unwrap();
        std::fs::create_dir(root.join(format!("{name}/book"))).unwrap();
        std::fs::write(root.join(format!("{name}/assets/plot.pdf")), "pdf").unwrap();
        std::fs::write(root.join(format!("{name}/main.tex")),"\\documentclass{article}\n\\usepackage{../common}\n\\begin{document}\n\\includegraphics[width=1cm]{plot}\n\\end{document}\n").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

#[test]
fn shared_package_graphics_paths_use_each_calling_roots_working_directory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = shared_project(root);
    for name in ["a", "b"] {
        let main = uri(root, &format!("{name}/main.tex"));
        let image = uri(root, &format!("{name}/assets/plot.pdf"));
        assert_eq!(
            graphics::directories(&workspace, &main),
            vec![root.join(format!("{name}/assets"))]
        );
        assert!(navigation::document_links(&workspace, &main)
            .iter()
            .any(|link| link.target.as_ref() == Some(&image)));
        let doc = &workspace.documents[&main];
        let position = doc
            .text
            .position(doc.text.source.find("{plot}").unwrap() + 3);
        let items = features::completion(&workspace, &main, position);
        let image = items.iter().find(|item| item.label == "plot.pdf").unwrap();
        assert!(
            image
                .detail
                .as_ref()
                .unwrap()
                .ends_with(&format!("{name}/assets/plot.pdf")),
            "{image:?}"
        );
    }
}

#[test]
fn shared_graphics_declarations_require_consistent_edits_in_every_project() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = shared_project(root);
    for files in [
        vec![rename(root, "a/assets", "a/pictures")],
        vec![rename(root, "a/main.tex", "a/book/main.tex")],
    ] {
        let error = file_operations::will_rename(&workspace, &files).unwrap_err();
        assert!(error.message.contains("different replacements"), "{error}");
    }
    let files = [
        rename(root, "a/assets", "a/pictures"),
        rename(root, "b/assets", "b/pictures"),
    ];
    let edits = file_operations::will_rename(&workspace, &files)
        .unwrap()
        .unwrap();
    let common = uri(root, "common.sty");
    assert_eq!(edits.changes.unwrap()[&common].len(), 1);
    let edits = changed(&workspace, &files);
    assert_eq!(edits.len(), 1);
    assert!(edits[&common].contains("\\graphicspath{{pictures/}{future/}}"));
    let files = [
        rename(root, "a/main.tex", "a/book/main.tex"),
        rename(root, "b/main.tex", "b/book/main.tex"),
    ];
    let edits = changed(&workspace, &files);
    assert!(edits[&common].contains("\\graphicspath{{../assets/}{../future/}}"));
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn shared_graphics_paths_still_compile_after_both_roots_and_image_directories_move() {
    let image = tempfile::tempdir().unwrap();
    std::fs::write(
        image.path().join("image.tex"),
        "\\documentclass{article}\\begin{document}Image\\end{document}",
    )
    .unwrap();
    let image_build = latex_lsp::build::run(
        &image.path().join("image.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(image_build.status, "success", "{}", image_build.output);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = shared_project(root);
    for name in ["a", "b"] {
        std::fs::copy(
            image_build.pdf.as_ref().unwrap(),
            root.join(format!("{name}/assets/plot.pdf")),
        )
        .unwrap();
    }
    let files = [
        rename(root, "a/assets", "a/pictures"),
        rename(root, "b/assets", "b/pictures"),
        rename(root, "a/main.tex", "a/book/main.tex"),
        rename(root, "b/main.tex", "b/book/main.tex"),
    ];
    for (uri, source) in changed(&workspace, &files) {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    for file in &files {
        std::fs::rename(
            Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
            Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
        )
        .unwrap();
    }
    file_operations::did_rename(&mut workspace, &files).unwrap();
    for name in ["a", "b"] {
        let main = uri(root, &format!("{name}/book/main.tex"));
        assert!(navigation::document_links(&workspace, &main)
            .iter()
            .any(|link| link.target == Some(uri(root, &format!("{name}/pictures/plot.pdf")))));
        let outcome = latex_lsp::build::run(
            &main.to_file_path().unwrap(),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(
            outcome.status, "success",
            "{}\n{}",
            outcome.message, outcome.output
        );
        assert!(outcome.pdf.unwrap().is_file());
    }
}
