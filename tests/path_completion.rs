use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn apply_completion(workspace: &mut Workspace, uri: &Url, marked: &str, label: &str) -> String {
    let offset = marked.find('|').unwrap();
    let source = marked.replace('|', "");
    workspace
        .documents
        .insert(uri.clone(), Document::new(uri, source.clone(), Some(1)));
    let text = Text::new(source);
    let items = features::completion(workspace, uri, text.position(offset));
    let item = items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("missing {label}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!();
    };
    text.changed(vec![TextDocumentContentChangeEvent {
        range: Some(edit.range),
        range_length: None,
        text: edit.new_text.clone(),
    }])
    .unwrap()
    .source
}

#[test]
fn input_completions_keep_non_tex_extensions_and_links_do_not_infer_them() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in [
        "main.tex",
        "commands.def",
        "defaults.clo",
        "chapter.ltx",
        "local.sty",
    ] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    for (marked, label, expected) in [
        (
            r"\input{comm|ands}",
            "commands.def",
            r"\input{commands.def}",
        ),
        (
            r"\input{defa|ults}",
            "defaults.clo",
            r"\input{defaults.clo}",
        ),
        (r"\input{chap|ter}", "chapter.ltx", r"\input{chapter.ltx}"),
        (r"\input{lo|cal}", "local.sty", r"\input{local.sty}"),
    ] {
        assert_eq!(
            apply_completion(&mut workspace, &uri, marked, label),
            expected
        );
    }
    let source =
        "\\input{commands}\n\\input{commands.def}\n\\input{chapter}\n\\input{chapter.ltx}\n";
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    let links = latex_lsp::navigation::document_links(&workspace, &uri);
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].range.start.line, 1);
    assert_eq!(links[1].range.start.line, 3);
}

#[test]
fn file_completions_replace_partial_names_preserve_lists_and_reuse_existing_slashes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("sections")).unwrap();
    for name in [
        "main.tex",
        "chapter.tex",
        "my file.tex",
        "references.bib",
        "local.sty",
        "custom.cls",
        "image.png",
        "sections/中文.tex",
    ] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    for (marked, label, expected) in [
        (r"\input{cha|pter}", "chapter.tex", r"\input{chapter}"),
        (
            r"\input{cha|pter.tex}",
            "chapter.tex",
            r"\input{chapter.tex}",
        ),
        (r"\input{chapter.|}", "chapter.tex", r"\input{chapter.tex}"),
        (
            r"\input{se|ctions/chapter}",
            "sections/",
            r"\input{sections/chapter}",
        ),
        (
            r"😀 \input{sections/中|旧}",
            "sections/中文.tex",
            r"😀 \input{sections/中文}",
        ),
        (r"\input{my f|ile}", "my file.tex", r"\input{my file}"),
        (
            r"\bibliography{other, re|ferences}",
            "references.bib",
            r"\bibliography{other, references}",
        ),
        (
            r"\addbibresource{re|f}",
            "references.bib",
            r"\addbibresource{references.bib}",
        ),
        (
            r"\includegraphics[width=2cm]{im|age}",
            "image.png",
            r"\includegraphics[width=2cm]{image.png}",
        ),
        (
            r"\usepackage{other, lo|cal}",
            "local.sty",
            r"\usepackage{other, local}",
        ),
        (
            r"\documentclass{cu|stom}",
            "custom.cls",
            r"\documentclass{custom}",
        ),
    ] {
        assert_eq!(
            apply_completion(&mut workspace, &uri, marked, label),
            expected
        );
    }
}

#[test]
fn completions_find_project_root_files_parent_paths_and_prefer_build_root_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir_all(root.join("sections")).unwrap();
    for (name, source) in [
        (
            "main.tex",
            "\\documentclass{article}\\input{sections/child}",
        ),
        ("sections/child.tex", ""),
        ("image.pdf", "pdf"),
        ("local.tex", "root"),
        ("sections/local.tex", "near"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    std::fs::write(directory.path().join("outside.tex"), "outside").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.clone());
    let child = Url::from_file_path(root.join("sections/child.tex")).unwrap();
    assert_eq!(
        apply_completion(
            &mut workspace,
            &child,
            r"\includegraphics{im|}",
            "image.pdf"
        ),
        r"\includegraphics{image.pdf}"
    );
    let marked = r"\input{lo|}";
    apply_completion(&mut workspace, &child, marked, "local.tex");
    let items = features::completion(&workspace, &child, Position::new(0, 9));
    let locals: Vec<_> = items
        .iter()
        .filter(|item| item.label == "local.tex")
        .collect();
    assert_eq!(locals.len(), 1);
    assert_eq!(locals[0].detail.as_deref(), root.join("local.tex").to_str());
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    assert_eq!(
        apply_completion(&mut workspace, &main, r"\input{../ou|}", "../outside.tex"),
        r"\input{../outside}"
    );
}

#[test]
fn file_completions_filter_types_hidden_files_and_ignored_regions_and_bound_results() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["main.tex", "chapter.tex", "wrong.png", ".hidden.tex"] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    let source = r"\input{";
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    let items = features::completion(
        &workspace,
        &uri,
        Text::new(source.into()).position(source.len()),
    );
    assert!(items.iter().any(|item| item.label == "chapter.tex"));
    assert!(!items
        .iter()
        .any(|item| item.label == "wrong.png" || item.label == ".hidden.tex"));
    assert_eq!(
        apply_completion(&mut workspace, &uri, r"\input{.h|}", ".hidden.tex"),
        r"\input{.hidden}"
    );
    for source in [r"% \input{ch", "\\begin{verbatim}\n\\input{ch"] {
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
        assert!(features::completion(
            &workspace,
            &uri,
            Text::new(source.into()).position(source.len())
        )
        .is_empty());
    }
    std::fs::create_dir(root.join("many")).unwrap();
    for n in 0..270 {
        std::fs::write(root.join(format!("many/file{n}.tex")), "").unwrap();
    }
    let source = r"\input{many/";
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    assert_eq!(
        features::completion(
            &workspace,
            &uri,
            Text::new(source.into()).position(source.len())
        )
        .len(),
        256
    );
}
