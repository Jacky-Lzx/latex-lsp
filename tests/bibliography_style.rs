use latex_lsp::{
    build, features, file_operations, navigation,
    text::Text,
    workspace::{Document, Workspace},
};
use std::path::Path;
use tower_lsp::lsp_types::*;

const STYLE: &str = r#"ENTRY { title } {} {}
FUNCTION {book} { "\bibitem{" write$ cite$ write$ "}" write$ newline$ title write$ newline$ }
READ
FUNCTION {begin.bib} { "\begin{thebibliography}{9}" write$ newline$ }
EXECUTE {begin.bib}
ITERATE {call.type$}
FUNCTION {end.bib} { "\end{thebibliography}" write$ newline$ }
EXECUTE {end.bib}
"#;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project(root: &Path) -> Workspace {
    for (name,source) in [
        ("main.tex", "\\documentclass{article}\n\\begin{document}\n\\cite{example}\n\\bibliographystyle{local}\n\\bibliography{references}\n\\end{document}\n"),
        ("local.bst",STYLE),
        ("references.bib","@book{example,title={Example}}\n"),
    ] { std::fs::write(root.join(name),source).unwrap(); }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn rename(root: &Path, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, "local.bst").to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

fn apply(workspace: &Workspace, files: &[FileRename]) -> String {
    let edit = file_operations::will_rename(workspace, files)
        .unwrap()
        .unwrap();
    let (uri, mut edits) = edit.changes.unwrap().into_iter().next().unwrap();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[&uri]
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
        .unwrap()
        .source
}

#[test]
fn local_bibtex_styles_are_indexed_linked_completed_and_renamed_as_build_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let main = uri(root, "main.tex");
    let style = uri(root, "local.bst");
    assert!(workspace.project_files(&main).contains(&style));
    assert!(workspace.documents.contains_key(&style));
    let links = navigation::document_links(&workspace, &main);
    assert!(links
        .iter()
        .any(|link| link.target.as_ref() == Some(&style)));
    let files = [rename(root, "book-style.bst")];
    assert!(apply(&workspace, &files).contains("\\bibliographystyle{book-style}"));
    assert!(
        file_operations::will_rename(&workspace, &[rename(root, "style.sty")])
            .unwrap_err()
            .message
            .contains(".bst")
    );
    let source = "\\bibliographystyle{lo";
    workspace
        .documents
        .insert(main.clone(), Document::new(&main, source.into(), Some(1)));
    let items = features::completion(
        &workspace,
        &main,
        Text::new(source.into()).position(source.len()),
    );
    let item = items.iter().find(|item| item.label == "local.bst").unwrap();
    assert_eq!(item.detail.as_deref(), root.join("local.bst").to_str());
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(edit.new_text, "local");
}

#[test]
fn bibtex_style_syntax_is_not_misreported_as_latex_and_bibtex_errors_keep_style_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let style = uri(root, "local.bst");
    let source = "% BibTeX uses its own syntax.\nFUNCTION {bad} { unknownFunction }\n}\n";
    std::fs::write(root.join("local.bst"), source).unwrap();
    workspace.reload(&style);
    assert!(workspace.diagnostics(&style).is_empty());
    workspace.open(style.clone(), "\\sec".into(), 1);
    assert!(features::completion(&workspace, &style, Position::new(0, 4)).is_empty());
    let diagnostics = latex_lsp::build_log::parse_bibliography(
        &root.join("main.tex"),
        "unknownFunction is an unknown function---line 2 of file local.bst\n",
    );
    assert_eq!(diagnostics[&style][0].range.start.line, 1);
    assert_eq!(
        diagnostics[&style][0].severity,
        Some(DiagnosticSeverity::ERROR)
    );
}

#[test]
#[ignore = "requires latexmk and BibTeX"]
fn local_style_renames_still_compile_and_real_bibtex_errors_point_to_the_style_file() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let first = build::run(
        &root.join("main.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(first.status, "success", "{}", first.output);
    assert!(
        !first
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.message.contains("undefined")),
        "{:?}",
        first.diagnostics
    );
    let files = [rename(root, "book-style.bst")];
    std::fs::write(root.join("main.tex"), apply(&workspace, &files)).unwrap();
    std::fs::rename(root.join("local.bst"), root.join("book-style.bst")).unwrap();
    file_operations::did_rename(&mut workspace, &files).unwrap();
    let renamed = build::run(
        &root.join("main.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(renamed.status, "success", "{}", renamed.output);
    assert!(renamed.pdf.unwrap().is_file());
    std::fs::write(
        root.join("book-style.bst"),
        "ENTRY {title} {} {}\nFUNCTION {broken} { unknownFunction }\nREAD\n",
    )
    .unwrap();
    let broken = build::run(
        &root.join("main.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(broken.status, "error", "{}", broken.output);
    let style = uri(root, "book-style.bst");
    assert!(
        broken
            .diagnostics
            .get(&style)
            .is_some_and(|diagnostics| diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("unknown function")
                    && diagnostic.range.start.line == 1)),
        "{:?}",
        broken.diagnostics
    );
}
