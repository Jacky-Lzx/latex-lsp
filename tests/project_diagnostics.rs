use latex_lsp::workspace::{Document, Workspace};
use std::path::Path;
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn cross_file(workspace: &Workspace, uri: &Url) -> Vec<Diagnostic> {
    workspace
        .diagnostics(uri)
        .into_iter()
        .filter(|diagnostic| {
            diagnostic.code == Some(NumberOrString::String("duplicate-definition".into()))
        })
        .collect()
}

fn write(root: &Path, name: &str, source: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

#[test]
fn duplicate_labels_and_bibliography_keys_report_related_definitions_in_the_same_project() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (name, source) in [
        ("a/main.tex", "\\documentclass{article}\n\\input{chapter}\n\\bibliography{first,second}\n\\label{键}\n\\label{namespace}\n"),
        ("a/chapter.tex", "😀 \\label{键}\n% \\label{namespace}\n\\verb|\\label{namespace}|"),
        ("a/first.bib", "@book{键,title={First}}\n@book{namespace,title={Separate namespace}}"),
        ("a/second.bib", "@book{键,title={Second}}"),
        ("b/main.tex", "\\documentclass{article}\\label{键}\\bibliography{references}"),
        ("b/references.bib", "@book{键,title={Independent paper}}"),
        ("unused.tex", "\\label{namespace}"),
    ] {
        write(root, name, source);
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "a/main.tex");
    let child = uri(root, "a/chapter.tex");
    let diagnostics = cross_file(&workspace, &main);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(
        diagnostics[0].range,
        Range::new(Position::new(3, 7), Position::new(3, 8))
    );
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::WARNING));
    let related = diagnostics[0].related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].location.uri, child);
    assert_eq!(
        related[0].location.range,
        Range::new(Position::new(0, 10), Position::new(0, 11))
    );
    let diagnostics = cross_file(&workspace, &uri(root, "a/first.bib"));
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message.contains("bibliography key"));
    assert_eq!(
        diagnostics[0].related_information.as_ref().unwrap()[0]
            .location
            .uri,
        uri(root, "a/second.bib")
    );
    assert!(cross_file(&workspace, &uri(root, "b/main.tex")).is_empty());
    assert!(cross_file(&workspace, &uri(root, "b/references.bib")).is_empty());
    assert!(cross_file(&workspace, &uri(root, "unused.tex")).is_empty());

    // The dependency graph and declaration index use current editor buffers.
    workspace.open(child.clone(), "\\label{unique}".into(), 1);
    assert!(cross_file(&workspace, &main).is_empty());
    assert!(cross_file(&workspace, &child).is_empty());
    workspace.open(
        main.clone(),
        "\\documentclass{article}\\label{键}".into(),
        1,
    );
    assert!(cross_file(&workspace, &uri(root, "a/first.bib")).is_empty());
}

#[test]
fn shared_definitions_merge_conflicts_without_linking_independent_consumers() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        write(
            root,
            &format!("{name}/main.tex"),
            "\\documentclass{article}\\input{../shared}\\label{shared}",
        );
    }
    write(root, "shared.tex", "\\label{shared}");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let shared = uri(root, "shared.tex");
    let diagnostics = cross_file(&workspace, &shared);
    assert_eq!(diagnostics.len(), 1);
    let related = diagnostics[0].related_information.as_ref().unwrap();
    assert_eq!(related.len(), 2);
    assert_eq!(related[0].location.uri, uri(root, "a/main.tex"));
    assert_eq!(related[1].location.uri, uri(root, "b/main.tex"));
    let diagnostics = cross_file(&workspace, &uri(root, "a/main.tex"));
    let related = diagnostics[0].related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].location.uri, shared);
}

#[test]
fn same_file_duplicates_keep_existing_diagnostics_and_related_locations_are_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "main.tex",
        "\\documentclass{article}\\label{local}\\label{local}",
    );
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    assert!(cross_file(&workspace, &main).is_empty());
    assert_eq!(workspace.diagnostics(&main).len(), 1);
    let mut source = "\\documentclass{article}\\label{key}".to_owned();
    for number in 0..32 {
        let name = format!("part{number}.tex");
        write(root, &name, "\\label{key}");
        source.push_str(&format!("\\input{{part{number}}}"));
        workspace.reload(&uri(root, &name));
    }
    workspace
        .documents
        .insert(main.clone(), Document::new(&main, source, Some(1)));
    let diagnostics = cross_file(&workspace, &main);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].related_information.as_ref().unwrap().len(),
        16
    );
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn duplicate_label_diagnostics_agree_with_real_compilation_and_clear_after_repair() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(root, "main.tex", "\\documentclass{article}\n\\begin{document}\n\\label{shared}\\input{chapter}\n\\ref{shared}\n\\end{document}\n");
    write(root, "chapter.tex", "\\label{shared}Chapter.\n");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    let child = uri(root, "chapter.tex");
    assert_eq!(cross_file(&workspace, &main).len(), 1);
    assert_eq!(cross_file(&workspace, &child).len(), 1);
    let first = latex_lsp::build::run(
        &root.join("main.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(first.status, "success", "{}", first.output);
    assert!(
        first
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.message.contains("multiply defined")),
        "{:?}",
        first.diagnostics
    );
    write(root, "chapter.tex", "\\label{unique}Chapter.\n");
    workspace.reload(&child);
    assert!(cross_file(&workspace, &main).is_empty());
    assert!(cross_file(&workspace, &child).is_empty());
    let repaired = latex_lsp::build::run(
        &root.join("main.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(repaired.status, "success", "{}", repaired.output);
    assert!(
        !repaired
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.message.contains("multiply defined")),
        "{:?}",
        repaired.diagnostics
    );
}
