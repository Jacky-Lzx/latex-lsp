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
    for name in ["assets", "sections"] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    for (name, source) in [
        ("main.tex", "\\documentclass{article}\\usepackage{graphicx}\\graphicspath{{assets/}}\\begin{document}\\input{sections/child}\\end{document}"),
        ("sections/child.tex", "\\includegraphics{plot}\\includegraphics{plot.png}"),
        ("assets/plot.pdf", "pdf"),
        ("assets/plot.png", "png"),
        ("sections/plot.pdf", "inaccessible child image"),
        ("sections/plot.png", "inaccessible child image"),
        ("sections/plum.pdf", "inaccessible child image"),
        ("plot.png", "root png"),
    ] { std::fs::write(root.join(name), source).unwrap(); }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn edits(workspace: &Workspace, files: &[FileRename]) -> BTreeMap<Url, String> {
    let Some(edit) = file_operations::will_rename(workspace, files).unwrap() else {
        return BTreeMap::new();
    };
    edit.changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let text = workspace.documents[&uri]
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
            (uri, text.source)
        })
        .collect()
}

fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

#[test]
fn image_links_follow_extension_order_then_root_and_graphics_directories() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    let child = uri(root, "sections/child.tex");
    let targets = || {
        navigation::document_links(&workspace, &child)
            .into_iter()
            .filter_map(|link| link.target)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        targets(),
        vec![uri(root, "assets/plot.pdf"), uri(root, "plot.png")]
    );
    assert_eq!(
        graphics::images(&workspace, &child),
        [uri(root, "assets/plot.pdf"), uri(root, "plot.png")].into()
    );
    let location = features::definitions(&workspace, &child, Position::new(0, 18));
    assert_eq!(location[0].uri, uri(root, "assets/plot.pdf"));
    std::fs::write(root.join("plot.pdf"), "root pdf").unwrap();
    assert_eq!(targets()[0], uri(root, "plot.pdf"));
    std::fs::remove_file(root.join("plot.pdf")).unwrap();
    std::fs::remove_file(root.join("assets/plot.pdf")).unwrap();
    assert_eq!(targets()[0], uri(root, "plot.png"));
    std::fs::remove_file(root.join("plot.png")).unwrap();
    assert_eq!(
        targets(),
        vec![uri(root, "assets/plot.png"), uri(root, "assets/plot.png")]
    );
    std::fs::remove_file(root.join("assets/plot.png")).unwrap();
    assert!(targets().is_empty());
    // A standalone source still resolves paths relative to itself.
    let mut standalone = Workspace::default();
    standalone.documents.insert(
        child.clone(),
        Document::new(&child, r"\includegraphics{plot}".into(), Some(1)),
    );
    assert_eq!(
        navigation::document_links(&standalone, &child)[0].target,
        Some(uri(root, "sections/plot.pdf"))
    );
}

#[test]
fn image_completions_describe_the_files_that_the_root_compiler_can_read() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let child = uri(root, "sections/child.tex");
    let source = r"\includegraphics{pl";
    workspace.open(child.clone(), source.into(), 1);
    let items = features::completion(
        &workspace,
        &child,
        Text::new(source.into()).position(source.len()),
    );
    assert!(!items.iter().any(|item| item.label == "plum.pdf"));
    for (label, file) in [("plot.pdf", "assets/plot.pdf"), ("plot.png", "plot.png")] {
        let item = items.iter().find(|item| item.label == label).unwrap();
        assert_eq!(item.detail.as_deref(), root.join(file).to_str());
    }
}

#[test]
fn image_renames_update_real_graphics_references_and_preserve_other_images() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    let child = uri(root, "sections/child.tex");
    let changed = edits(
        &workspace,
        &[rename(root, "assets/plot.pdf", "assets/figure.pdf")],
    );
    assert_eq!(
        changed[&child],
        r"\includegraphics{figure}\includegraphics{plot.png}"
    );
    assert!(edits(
        &workspace,
        &[rename(root, "sections/plot.pdf", "sections/local.pdf")]
    )
    .is_empty());
    assert!(edits(
        &workspace,
        &[rename(root, "sections/plum.pdf", "sections/local.pdf")]
    )
    .is_empty());
    let changed = edits(&workspace, &[rename(root, "plot.png", "root.png")]);
    assert_eq!(
        changed[&child],
        r"\includegraphics{plot}\includegraphics{root.png}"
    );
}

#[test]
fn image_links_keep_dotted_filename_fallbacks_and_ignore_extensionless_probes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    for (name, content) in [
        ("plot", "extensionless probe"),
        ("chart.v1", "unknown graphics rule probe"),
        ("assets/chart.v1.pdf", "pdf"),
        ("assets/named.pdf.pdf", "pdf"),
    ] {
        std::fs::write(root.join(name), content).unwrap();
    }
    let child = uri(root, "sections/child.tex");
    workspace.open(
        child.clone(),
        r"\includegraphics{plot}\includegraphics{chart.v1}\includegraphics{named.pdf}".into(),
        1,
    );
    let links = navigation::document_links(&workspace, &child);
    assert_eq!(
        links
            .into_iter()
            .map(|link| link.target.unwrap())
            .collect::<Vec<_>>(),
        vec![
            uri(root, "assets/plot.pdf"),
            uri(root, "assets/chart.v1.pdf"),
            uri(root, "assets/named.pdf.pdf")
        ]
    );
}

#[test]
fn moving_sources_keeps_image_paths_relative_to_the_compilation_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    let main = uri(root, "main.tex");
    let child = uri(root, "sections/child.tex");
    let moved_child = edits(
        &workspace,
        &[rename(root, "sections/child.tex", "chapters/child.tex")],
    );
    assert!(!moved_child.contains_key(&child));
    assert!(moved_child[&main].contains(r"\input{chapters/child}"));
    let moved_main = edits(&workspace, &[rename(root, "main.tex", "book/main.tex")]);
    assert!(moved_main[&main].contains(r"\graphicspath{{../assets/}}"));
    assert!(moved_main[&main].contains(r"\input{../sections/child}"));
    assert_eq!(
        moved_main[&child],
        r"\includegraphics{plot}\includegraphics{../plot.png}"
    );
}

#[test]
#[ignore = "requires pdflatex and graphicx"]
fn root_graphics_images_remain_selected_after_image_and_main_file_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let child = uri(root, "sections/child.tex");
    std::fs::write(root.join("sections/child.tex"), r"\includegraphics{plot}").unwrap();
    workspace.reload(&child);
    let drawing = tempfile::tempdir().unwrap();
    std::fs::write(
        drawing.path().join("drawing.tex"),
        r"\documentclass{article}\begin{document}Figure\end{document}",
    )
    .unwrap();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let image = latex_lsp::build::run(
        &drawing.path().join("drawing.tex"),
        &config,
        &Default::default(),
    );
    assert_eq!(image.status, "success", "{}", image.output);
    std::fs::copy(image.pdf.unwrap(), root.join("assets/plot.pdf")).unwrap();
    let before = latex_lsp::build::run(&root.join("main.tex"), &config, &Default::default());
    assert_eq!(before.status, "success", "{}", before.output);
    let inputs = before.inputs.unwrap();
    assert!(inputs.contains(&uri(root, "assets/plot.pdf")));
    assert!(!inputs.contains(&uri(root, "sections/plot.pdf")));
    assert!(!inputs.contains(&uri(root, "plot.png")));
    let files = [
        rename(root, "assets/plot.pdf", "assets/figure.pdf"),
        rename(root, "main.tex", "book/main.tex"),
    ];
    let changed = edits(&workspace, &files);
    assert_eq!(changed[&child], r"\includegraphics{figure}");
    for (uri, source) in changed {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
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
    assert_eq!(
        navigation::document_links(&workspace, &child)[0].target,
        Some(uri(root, "assets/figure.pdf"))
    );
    let after = latex_lsp::build::run(&root.join("book/main.tex"), &config, &Default::default());
    assert_eq!(after.status, "success", "{}", after.output);
    assert!(after
        .inputs
        .unwrap()
        .contains(&uri(root, "assets/figure.pdf")));
}
