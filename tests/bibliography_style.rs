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

#[test]
fn bibtex_databases_use_the_bib_suffix_even_when_literal_lookalikes_exist() {
    for name in ["references", "references.ltx", "references.bib"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        project(root);
        let expected = if name.ends_with(".bib") {
            name.to_owned()
        } else {
            std::fs::write(root.join(name), "Wrong database.").unwrap();
            format!("{name}.bib")
        };
        std::fs::write(root.join(&expected), "@book{example,title={Expected}}\n").unwrap();
        std::fs::write(
            root.join("main.tex"),
            format!(r"\documentclass{{article}}\bibliography{{{name}}}"),
        )
        .unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = uri(root, "main.tex");
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1, "{name}: {links:?}");
        assert_eq!(links[0].target, Some(uri(root, &expected)), "{name}");
        assert!(workspace
            .project_files(&main)
            .contains(&uri(root, &expected)));
        if expected != name {
            assert!(!workspace.project_files(&main).contains(&uri(root, name)));
            std::fs::remove_file(root.join(&expected)).unwrap();
            assert!(navigation::document_links(&workspace, &main).is_empty());
        }
    }
}

#[test]
fn bibtex_style_paths_require_the_bst_suffix_and_completions_resolve_to_that_style() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    project(root);
    for name in ["local", "local.inc"] {
        std::fs::write(root.join(name), "Wrong literal style.").unwrap();
        std::fs::write(root.join(format!("{name}.bst")), STYLE).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    workspace.open(
        main.clone(),
        r"\bibliographystyle{local}\bibliographystyle{local.inc}".into(),
        1,
    );
    assert_eq!(
        navigation::document_links(&workspace, &main)
            .into_iter()
            .map(|link| link.target.unwrap())
            .collect::<Vec<_>>(),
        vec![uri(root, "local.bst"), uri(root, "local.inc.bst")]
    );
    workspace.open(main.clone(), r"\bibliographystyle{lo".into(), 2);
    let items = features::completion(&workspace, &main, Position::new(0, 20));
    let item = items
        .iter()
        .find(|item| item.label == "local.inc.bst")
        .unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(edit.new_text, "local.inc");
    std::fs::remove_file(root.join("local.inc.bst")).unwrap();
    workspace.open(main.clone(), r"\bibliographystyle{local.inc}".into(), 3);
    assert!(navigation::document_links(&workspace, &main).is_empty());
}

#[test]
fn bibliography_file_renames_require_bib_but_biblatex_resources_keep_explicit_extensions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let main = uri(root, "main.tex");
    let files = [FileRename {
        old_uri: uri(root, "references.bib").to_string(),
        new_uri: uri(root, "references.ltx").to_string(),
    }];
    let error = file_operations::will_rename(&workspace, &files).unwrap_err();
    assert!(error.message.contains(".bib"), "{error}");
    workspace.open(main, r"\addbibresource{references.bib}".into(), 1);
    assert_eq!(
        apply(&workspace, &files),
        r"\addbibresource{references.ltx}"
    );
}

#[test]
fn renaming_bibtex_lookalikes_preserves_the_compiler_selected_database() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    std::fs::write(root.join("references.ltx"), "Wrong source.").unwrap();
    std::fs::write(
        root.join("references.ltx.bib"),
        "@book{example,title={Expected}}\n",
    )
    .unwrap();
    let main = uri(root, "main.tex");
    workspace.open(main, r"\bibliography{references.ltx}".into(), 1);
    let file = |old: &str, new: &str| FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    };
    assert!(
        file_operations::will_rename(&workspace, &[file("references.ltx", "lookalike.ltx")])
            .unwrap()
            .is_none()
    );
    assert_eq!(
        apply(&workspace, &[file("references.ltx.bib", "renamed.bib")]),
        r"\bibliography{renamed}"
    );
}

#[test]
#[ignore = "requires latexmk and BibTeX"]
fn bibtex_compiler_databases_and_styles_match_links_before_and_after_database_rename() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    project(root);
    std::fs::write(root.join("local.inc.bst"), STYLE).unwrap();
    std::fs::write(root.join("local.inc"), "Wrong style.").unwrap();
    std::fs::write(
        root.join("references.inc.bib"),
        "@book{example,title={Expected}}\n",
    )
    .unwrap();
    std::fs::write(root.join("references.inc"), "Wrong database.").unwrap();
    let source = r"\documentclass{article}\begin{document}\cite{example}\bibliographystyle{local.inc}\bibliography{references.inc}\end{document}";
    std::fs::write(root.join("main.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    for renamed in [false, true] {
        let outcome = build::run(
            &root.join("main.tex"),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        let database = if renamed {
            "renamed.bib"
        } else {
            "references.inc.bib"
        };
        let links = navigation::document_links(&workspace, &main);
        assert!(links
            .iter()
            .any(|link| link.target == Some(uri(root, database))));
        assert!(links
            .iter()
            .any(|link| link.target == Some(uri(root, "local.inc.bst"))));
        assert!(outcome
            .bibliography_inputs
            .unwrap()
            .contains(&uri(root, database)));
        assert!(
            std::fs::read_to_string(outcome.log.as_ref().unwrap().with_extension("blg"))
                .unwrap()
                .contains("local.inc.bst")
        );
        if renamed {
            break;
        }
        let files = [FileRename {
            old_uri: uri(root, database).to_string(),
            new_uri: uri(root, "renamed.bib").to_string(),
        }];
        std::fs::write(root.join("main.tex"), apply(&workspace, &files)).unwrap();
        std::fs::rename(root.join(database), root.join("renamed.bib")).unwrap();
        file_operations::did_rename(&mut workspace, &files).unwrap();
    }
}
