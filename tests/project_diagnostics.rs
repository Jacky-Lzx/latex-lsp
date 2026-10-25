use latex_lsp::project::workspace::{Document, Workspace};
use lsp_types::*;
use std::path::Path;

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
fn pdfbookmark_duplicate_checks_use_the_effective_destination_and_refresh_when_its_level_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(root, "a/main.tex", "\\documentclass{article}\\hypertarget{intro.0}{Target}\\label{intro.0}\\input{bookmarks}\n");
    write(root, "a/bookmarks.tex", "\\pdfbookmark{Title}{intro}\n");
    write(
        root,
        "b/main.tex",
        "\\documentclass{article}\\pdfbookmark{Other}{intro}\n",
    );
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "a/main.tex");
    let child = uri(root, "a/bookmarks.tex");
    let items = cross_file(&workspace, &main);
    assert_eq!(items.len(), 1, "{items:?}");
    assert!(items[0].message.contains("PDF destination `intro.0`"));
    assert_eq!(
        items[0].related_information.as_ref().unwrap()[0]
            .location
            .uri,
        child
    );
    assert_eq!(cross_file(&workspace, &child).len(), 1);
    assert!(cross_file(&workspace, &uri(root, "b/main.tex")).is_empty());
    workspace.open(child.clone(), "\\pdfbookmark[1]{Title}{intro}\n".into(), 1);
    assert!(cross_file(&workspace, &main).is_empty());
    assert!(cross_file(&workspace, &child).is_empty());
}

#[test]
fn distinct_spaced_label_names_do_not_report_normalized_duplicate_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "main.tex",
        "\\documentclass{article}\\label{key}\\input{chapter}",
    );
    write(root, "chapter.tex", r"\label{ key }");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    let child = uri(root, "chapter.tex");
    assert!(workspace.diagnostics(&main).is_empty());
    assert!(workspace.diagnostics(&child).is_empty());
    workspace.documents.insert(
        child.clone(),
        Document::new(&child, r"\label{key}".into(), Some(1)),
    );
    assert_eq!(cross_file(&workspace, &main).len(), 1);
    assert_eq!(cross_file(&workspace, &child).len(), 1);
}

#[test]
fn hyperdef_labels_share_duplicate_checks_and_refresh_with_ordinary_label_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "a/main.tex",
        r"\documentclass{article}\label{shared}\input{target}",
    );
    write(
        root,
        "a/target.tex",
        r"\hyperdef[{shared}]{category}{name}{Text}",
    );
    write(
        root,
        "b/main.tex",
        r"\documentclass{article}\hyperdef[shared]{category}{name}{Other paper}",
    );
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "a/main.tex");
    let target = uri(root, "a/target.tex");
    assert_eq!(cross_file(&workspace, &main).len(), 1);
    let duplicates = cross_file(&workspace, &target);
    assert_eq!(duplicates.len(), 1);
    assert_eq!(
        duplicates[0].range,
        Range::new(Position::new(0, 11), Position::new(0, 17))
    );
    assert!(cross_file(&workspace, &uri(root, "b/main.tex")).is_empty());
    workspace.open(
        target.clone(),
        r"\hyperdef[{unique}]{category}{name}{Text}".into(),
        1,
    );
    assert!(cross_file(&workspace, &main).is_empty());
    assert!(cross_file(&workspace, &target).is_empty());
}

#[test]
fn pdf_destination_duplicates_include_compound_definitions_but_keep_labels_and_projects_distinct() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "a/main.tex",
        r"\documentclass{article}\hypertarget{section.first}{Text}\label{section.first}\input{targets}",
    );
    write(root, "a/targets.tex", r"\hyperdef{section}{first}{Text}");
    write(
        root,
        "b/main.tex",
        r"\documentclass{article}\hypertarget{section.first}{Other paper}",
    );
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "a/main.tex");
    let child = uri(root, "a/targets.tex");
    let diagnostics = cross_file(&workspace, &main);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("PDF destination"));
    assert_eq!(
        diagnostics[0].related_information.as_ref().unwrap()[0]
            .location
            .uri,
        child
    );
    assert_eq!(cross_file(&workspace, &child).len(), 1);
    assert!(cross_file(&workspace, &uri(root, "b/main.tex")).is_empty());
    workspace.open(child.clone(), r"\hyperdef{section}{second}{Text}".into(), 1);
    assert!(cross_file(&workspace, &main).is_empty());
    assert!(cross_file(&workspace, &child).is_empty());
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
    let first = latex_lsp::compilation::build::run(
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
    let repaired = latex_lsp::compilation::build::run(
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

#[test]
fn local_duplicate_diagnostics_link_first_declarations_with_distinct_namespaces() {
    let uri = Url::parse("untitled:duplicate-names.tex").unwrap();
    let source = "😀 \\label{键} \\bibitem{键} \\hypertarget{键}{First}\r\n\\label{键} \\bibitem{键} \\hypertarget{键}{Second}";
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 1);
    let diagnostics = workspace.diagnostics(&uri);
    let duplicates: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message == "Duplicate key `键`")
        .collect();
    assert_eq!(duplicates.len(), 3, "{diagnostics:?}");
    let text = latex_lsp::text::Text::new(source.into());
    for (code, command) in [
        ("duplicate-label", "label"),
        ("duplicate-bibliography-key", "bibitem"),
        ("duplicate-pdf-destination", "hypertarget"),
    ] {
        let diagnostic = duplicates
            .iter()
            .find(|diagnostic| diagnostic.code == Some(NumberOrString::String(code.into())))
            .unwrap();
        let related = diagnostic.related_information.as_ref().unwrap();
        assert_eq!(related.len(), 1);
        let at = source.find(&format!("\\{command}{{键}}")).unwrap() + command.len() + 2;
        assert_eq!(
            related[0].location,
            Location::new(uri.clone(), text.range(at, at + "键".len()))
        );
        assert_eq!(diagnostic.range.start.line, 1);
    }
}

#[test]
fn local_bibliography_duplicates_refresh_first_positions_without_indexing_value_examples() {
    let uri = Url::parse("untitled:duplicates.bib").unwrap();
    let source = "@comment{ @book{键,title={Example}} }\r\n@book{键,title={😀 @article{键, ignored}}}\r\n@article{键,title={Second}}\r\n@misc{键,title={Third}}";
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 1);
    let diagnostics = workspace.diagnostics(&uri);
    let duplicates: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == Some(NumberOrString::String("duplicate-bibliography-key".into()))
        })
        .collect();
    assert_eq!(duplicates.len(), 2, "{diagnostics:?}");
    assert!(duplicates.iter().all(
        |diagnostic| diagnostic.related_information.as_ref().unwrap()[0]
            .location
            .uri
            == uri
    ));
    assert!(duplicates.iter().all(
        |diagnostic| diagnostic.related_information.as_ref().unwrap()[0]
            .location
            .range
            .start
            .line
            == 1
    ));
    let updated = source.replacen("\r\n@book{键,", "\r\n@book{unique,", 1);
    workspace.open(uri.clone(), updated, 2);
    let diagnostics = workspace.diagnostics(&uri);
    let duplicates: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == Some(NumberOrString::String("duplicate-bibliography-key".into()))
        })
        .collect();
    assert_eq!(duplicates.len(), 1, "{diagnostics:?}");
    assert_eq!(
        duplicates[0].related_information.as_ref().unwrap()[0]
            .location
            .range
            .start
            .line,
        2
    );
    workspace.open(uri.clone(), "@book{unique,title={Only entry}}".into(), 3);
    assert!(workspace.diagnostics(&uri).is_empty());
}
