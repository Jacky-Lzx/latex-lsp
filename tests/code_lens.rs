use latex_lsp::{
    language::code_lens,
    project::workspace::{Document, Workspace},
};
use lsp_types::{Position, Range, Url};

fn uri(root: &std::path::Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

#[test]
fn build_lenses_use_the_main_document_and_utf16_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (name, source) in [
        ("main.tex", "😀 \\documentclass{article}\\input{chapter}"),
        ("chapter.tex", "Chapter."),
        ("other.ltx", "\\documentclass{report}"),
        ("fragment.tex", "Unrelated fragment."),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    let lens = code_lens::lenses(&workspace, &main);
    assert_eq!(lens.len(), 1);
    assert_eq!(
        lens[0].range,
        Range::new(Position::new(0, 3), Position::new(0, 17))
    );
    let command = lens[0].command.as_ref().unwrap();
    assert_eq!(command.title, "Build main.tex");
    assert_eq!(command.command, "latex.build");
    assert_eq!(command.arguments, Some(vec![serde_json::json!(main)]));
    let chapter = code_lens::lenses(&workspace, &uri(root, "chapter.tex"));
    assert_eq!(chapter.len(), 1);
    let command = chapter[0].command.as_ref().unwrap();
    assert_eq!(command.title, "Build main.tex");
    assert_eq!(
        command.arguments,
        Some(vec![serde_json::json!(uri(root, "chapter.tex"))])
    );
    assert_eq!(
        chapter[0].range,
        Range::new(Position::new(0, 0), Position::new(0, 0))
    );
    assert!(code_lens::lenses(&workspace, &uri(root, "fragment.tex")).is_empty());
    assert_eq!(
        code_lens::lenses(&workspace, &uri(root, "other.ltx"))[0]
            .command
            .as_ref()
            .unwrap()
            .title,
        "Build other.ltx"
    );
}

#[test]
fn shared_sources_offer_separate_roots_and_respect_explicit_root_selection() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for folder in ["a", "b", "shared"] {
        std::fs::create_dir(root.join(folder)).unwrap();
    }
    for name in ["a/main.tex", "b/main.tex"] {
        std::fs::write(
            root.join(name),
            "\\documentclass{article}\\input{../shared/chapter}",
        )
        .unwrap();
    }
    std::fs::write(root.join("shared/chapter.tex"), "Shared chapter.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let source = uri(root, "shared/chapter.tex");
    let result = code_lens::lenses(&workspace, &source);
    assert_eq!(result.len(), 2);
    assert_eq!(
        result
            .iter()
            .map(|lens| lens.command.as_ref().unwrap().title.as_str())
            .collect::<Vec<_>>(),
        ["Build a/main.tex", "Build b/main.tex"]
    );
    for (lens, folder) in result.iter().zip(["a", "b"]) {
        assert_eq!(
            lens.command.as_ref().unwrap().arguments,
            Some(vec![serde_json::json!(uri(
                root,
                &format!("{folder}/main.tex")
            ))])
        );
    }
    workspace.config.build.root_file = Some("b/main.tex".into());
    let selected = code_lens::lenses(&workspace, &source);
    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected[0].command.as_ref().unwrap().title,
        result[1].command.as_ref().unwrap().title
    );
    assert_eq!(
        selected[0].command.as_ref().unwrap().arguments,
        Some(vec![serde_json::json!(source)])
    );
    workspace.config.build.root_file = None;
    workspace.documents.insert(
        source.clone(),
        Document::new(
            &source,
            "% !TeX root = ../a/main.tex\nShared.".into(),
            Some(2),
        ),
    );
    let selected = code_lens::lenses(&workspace, &source);
    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected[0].command.as_ref().unwrap().title,
        result[0].command.as_ref().unwrap().title
    );
    assert_eq!(
        selected[0].command.as_ref().unwrap().arguments,
        Some(vec![serde_json::json!(source)])
    );
    workspace.documents.insert(
        source.clone(),
        Document::new(
            &source,
            "% !TeX root = missing.tex\nShared.".into(),
            Some(3),
        ),
    );
    assert!(code_lens::lenses(&workspace, &source).is_empty());
    workspace.config.build.root_file = Some("missing.tex".into());
    assert!(code_lens::lenses(&workspace, &source).is_empty());
}

#[test]
fn multi_folder_build_lens_titles_distinguish_identically_named_main_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for folder in ["a", "b"] {
        std::fs::create_dir(root.join(folder)).unwrap();
        std::fs::write(
            root.join(folder).join("main.tex"),
            "\\documentclass{article}\\input{../chapter}",
        )
        .unwrap();
    }
    std::fs::write(root.join("chapter.tex"), "Shared.").unwrap();
    let mut workspace = Workspace::default();
    for folder in ["a", "b"] {
        workspace.add_root(root.join(folder));
    }
    workspace.open(uri(root, "chapter.tex"), "Shared.".into(), 1);
    let result = code_lens::lenses(&workspace, &uri(root, "chapter.tex"));
    assert_eq!(result.len(), 2);
    assert_ne!(
        result[0].command.as_ref().unwrap().title,
        result[1].command.as_ref().unwrap().title
    );
    for (lens, folder) in result.iter().zip(["a", "b"]) {
        assert!(lens
            .command
            .as_ref()
            .unwrap()
            .title
            .ends_with(&format!("{folder}/main.tex")));
    }
}

#[test]
fn shared_build_lenses_are_bounded_and_deterministically_ordered() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), "Shared.").unwrap();
    for index in 0..40 {
        std::fs::write(
            root.join(format!("main-{index:02}.tex")),
            "\\documentclass{article}\\input{chapter}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let result = code_lens::lenses(&workspace, &uri(root, "chapter.tex"));
    assert_eq!(result.len(), 32);
    for (index, lens) in result.iter().enumerate() {
        assert_eq!(
            lens.command.as_ref().unwrap().title,
            format!("Build main-{index:02}.tex")
        );
    }
}

#[test]
fn external_relative_root_settings_keep_the_original_source_command_context() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let folder = root.join("project/nested/deep");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(root.join("main.tex"), "\\documentclass{article}").unwrap();
    std::fs::write(folder.join("chapter.tex"), "Chapter.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(folder.clone());
    workspace.config.build.root_file = Some("../../../main.tex".into());
    let source = uri(&folder, "chapter.tex");
    let main = uri(root, "main.tex");
    assert_eq!(workspace.root_document(&source).unwrap(), main);
    assert_eq!(workspace.root_document(&main).unwrap(), main);
    let result = code_lens::lenses(&workspace, &source);
    assert_eq!(result.len(), 1);
    let command = result[0].command.as_ref().unwrap();
    assert_eq!(command.arguments, Some(vec![serde_json::json!(source)]));
    let argument = Url::parse(command.arguments.as_ref().unwrap()[0].as_str().unwrap()).unwrap();
    assert_eq!(workspace.root_document(&argument).unwrap(), main);
}

#[test]
fn shared_parent_root_directives_are_resolved_deduplicated_and_checked_for_cycles() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("a.tex"),
        "% !TeX root = b.tex\n\\documentclass{article}\\input{chapter}",
    )
    .unwrap();
    std::fs::write(
        root.join("b.tex"),
        "\\documentclass{article}\\input{chapter}",
    )
    .unwrap();
    std::fs::write(root.join("chapter.tex"), "Shared.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let source = uri(root, "chapter.tex");
    let result = code_lens::lenses(&workspace, &source);
    assert_eq!(result.len(), 1);
    let command = result[0].command.as_ref().unwrap();
    assert_eq!(command.title, "Build b.tex");
    let argument = Url::parse(command.arguments.as_ref().unwrap()[0].as_str().unwrap()).unwrap();
    assert_eq!(
        workspace.root_document(&argument).unwrap(),
        uri(root, "b.tex")
    );
    let second = uri(root, "b.tex");
    workspace.documents.insert(
        second.clone(),
        Document::new(
            &second,
            "% !TeX root = a.tex\n\\documentclass{article}\\input{chapter}".into(),
            Some(1),
        ),
    );
    assert!(code_lens::lenses(&workspace, &source).is_empty());
}

#[test]
fn build_lenses_exclude_unsupported_documents_and_refresh_after_root_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (name, source) in [
        (
            "main.tex",
            "\\documentclass{article}\\input{chapter}\\bibliography{refs}",
        ),
        ("chapter.tex", "Chapter."),
        ("refs.bib", "@book{one,title={One}}"),
        ("custom.bst", "ENTRY {} {} {}"),
        ("orphan.sty", "\\ProvidesPackage{orphan}"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for name in ["refs.bib", "custom.bst", "orphan.sty", "missing.tex"] {
        assert!(
            code_lens::lenses(&workspace, &uri(root, name)).is_empty(),
            "{name}"
        );
    }
    let untitled = Url::parse("untitled:main.tex").unwrap();
    workspace.documents.insert(
        untitled.clone(),
        Document::new(&untitled, "\\documentclass{article}".into(), Some(1)),
    );
    assert!(code_lens::lenses(&workspace, &untitled).is_empty());
    let main = uri(root, "main.tex");
    assert_eq!(
        code_lens::lenses(&workspace, &uri(root, "chapter.tex")).len(),
        1
    );
    std::fs::remove_file(root.join("main.tex")).unwrap();
    assert!(code_lens::lenses(&workspace, &uri(root, "chapter.tex")).is_empty());
    workspace.documents.remove(&main);
    assert!(code_lens::lenses(&workspace, &uri(root, "chapter.tex")).is_empty());
}
