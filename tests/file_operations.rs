use latex_lsp::{
    file_operations,
    text::Text,
    workspace::{Document, Workspace},
};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

fn edits(edit: WorkspaceEdit) -> BTreeMap<Url, Vec<TextEdit>> {
    if let Some(changes) = edit.changes {
        return changes.into_iter().collect();
    }
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("missing edits");
    };
    documents
        .into_iter()
        .map(|document| {
            let edits = document
                .edits
                .into_iter()
                .map(|edit| match edit {
                    OneOf::Left(edit) => edit,
                    _ => panic!(),
                })
                .collect();
            (document.text_document.uri, edits)
        })
        .collect()
}

fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
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

fn project(root: &Path) -> Workspace {
    std::fs::create_dir(root.join("chapters")).unwrap();
    for (name, source) in [
        ("main.tex", "\\documentclass{local}\n\\usepackage{pkg}\n\\input{chapters/old}\n\\bibliography{refs, other}\n\\includegraphics{fig}\n% \\input{chapters/old}\n\\verb|\\input{chapters/old}|\n"),
        ("chapters/old.tex", "% !TeX root = \"../main.tex\"\n\\input{../shared.v1}\n\\includegraphics{fig}\n"),
        ("shared.v1.tex", "Shared"), ("local.cls", "\\LoadClass{article}"),
        ("pkg.sty", "\\newcommand{\\hello}{Hello}"),
        ("refs.bib", "@book{key,title={Example}}"), ("other.bib", ""), ("fig.png", "png"),
    ] { std::fs::write(root.join(name), source).unwrap(); }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

#[test]
fn batched_file_renames_update_tex_bib_packages_images_and_magic_root_comments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    workspace.versioned_edits = true;
    let main = uri(root, "main.tex");
    let source = workspace.documents[&main].text.source.clone();
    workspace
        .documents
        .insert(main.clone(), Document::new(&main, source.clone(), Some(7)));
    let files = [
        rename(root, "main.tex", "paper.tex"),
        rename(root, "chapters/old.tex", "chapters/新章节.tex"),
        rename(root, "refs.bib", "library.bib"),
        rename(root, "pkg.sty", "package.sty"),
        rename(root, "fig.png", "diagram.png"),
    ];
    let edit = file_operations::will_rename(&workspace, &files)
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(documents)) = &edit.document_changes else {
        panic!();
    };
    assert_eq!(
        documents
            .iter()
            .find(|doc| doc.text_document.uri == main)
            .unwrap()
            .text_document
            .version,
        Some(7)
    );
    let edits = edits(edit);
    let changed_main = apply(&source, edits[&main].clone());
    assert!(changed_main.contains("\\input{chapters/新章节}"));
    assert!(changed_main.contains("\\usepackage{package}"));
    assert!(changed_main.contains("\\bibliography{library, other}"));
    assert!(changed_main.contains("\\includegraphics{diagram}"));
    assert!(changed_main.contains("% \\input{chapters/old}"));
    assert!(changed_main.contains("\\verb|\\input{chapters/old}|"));
    let chapter = uri(root, "chapters/old.tex");
    let changed_chapter = apply(
        &workspace.documents[&chapter].text.source,
        edits[&chapter].clone(),
    );
    assert!(changed_chapter.contains("% !TeX root = \"../paper.tex\""));
    assert!(changed_chapter.contains("\\includegraphics{diagram}"));
    assert!(root.join("main.tex").exists());
    assert!(!root.join("paper.tex").exists());
    // Simulate the client applying edits, then moving files and notifying LSP.
    for (uri, edits) in edits {
        let doc = &workspace.documents[&uri];
        let source = apply(&doc.text.source, edits);
        std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
        if let Some(version) = doc.version {
            workspace
                .documents
                .insert(uri.clone(), Document::new(&uri, source, Some(version + 1)));
        }
    }
    for file in &files {
        std::fs::rename(
            Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
            Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
        )
        .unwrap();
    }
    let affected = file_operations::did_rename(&mut workspace, &files).unwrap();
    let paper = uri(root, "paper.tex");
    assert!(affected.contains(&main) && affected.contains(&paper));
    assert!(!workspace.documents.contains_key(&main));
    assert_eq!(workspace.documents[&paper].version, Some(8));
    let chapter = uri(root, "chapters/新章节.tex");
    assert_eq!(workspace.root_document(&chapter).unwrap(), paper);
    assert!(workspace
        .project_documents(&paper)
        .contains(&uri(root, "library.bib")));
}

#[test]
fn file_urls_preserve_percent_encoding_when_assets_are_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let original = uri(root, "original file.pdf");
    std::fs::write(original.to_file_path().unwrap(), "pdf").unwrap();
    let new = uri(root, "renamed file#1.pdf");
    let main = uri(root, "main.tex");
    let source = format!("\\url{{{original}}}");
    workspace
        .documents
        .insert(main.clone(), Document::new(&main, source.clone(), Some(1)));
    let files = [FileRename {
        old_uri: original.to_string(),
        new_uri: new.to_string(),
    }];
    let edits = edits(
        file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap(),
    );
    let source = apply(&source, edits[&main].clone());
    assert_eq!(source, format!("\\url{{{new}}}"));
    let document = Document::new(&main, source, Some(2));
    assert!(latex_lsp::features::diagnostics(&document).is_empty());
}

#[test]
fn directory_moves_rebase_references_inside_moved_documents() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    let files = [rename(root, "chapters", "parts/content")];
    let edits = edits(
        file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap(),
    );
    let main = uri(root, "main.tex");
    let chapter = uri(root, "chapters/old.tex");
    let changed_main = apply(
        &workspace.documents[&main].text.source,
        edits[&main].clone(),
    );
    assert!(changed_main.contains("\\input{parts/content/old}"));
    let changed = apply(
        &workspace.documents[&chapter].text.source,
        edits[&chapter].clone(),
    );
    assert!(changed.contains("../../main.tex"));
    assert!(changed.contains("\\input{../../shared.v1}"));
    // The image was relative to the project root, which remains unchanged.
    assert!(changed.contains("\\includegraphics{fig}"));
}

#[test]
fn moving_an_entire_project_preserves_references_and_updates_workspace_roots() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("old");
    let new = directory.path().join("new");
    std::fs::create_dir(&root).unwrap();
    let mut workspace = project(&root);
    let files = [FileRename {
        old_uri: Url::from_file_path(&root).unwrap().to_string(),
        new_uri: Url::from_file_path(&new).unwrap().to_string(),
    }];
    assert!(file_operations::will_rename(&workspace, &files)
        .unwrap()
        .is_none());
    std::fs::rename(&root, &new).unwrap();
    file_operations::did_rename(&mut workspace, &files).unwrap();
    assert!(workspace.roots.contains(&new));
    assert!(!workspace.roots.contains(&root));
    assert_eq!(workspace.documents.len(), 7);
    let main = uri(&new, "main.tex");
    assert_eq!(
        workspace
            .root_document(&uri(&new, "chapters/old.tex"))
            .unwrap(),
        main
    );
}

#[test]
#[ignore = "requires latexmk, BibTeX and a TeX installation"]
fn renamed_projects_still_compile_after_root_and_dependency_moves() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("chapters")).unwrap();
    for (name,source) in [
        ("main.tex", "\\documentclass{article}\n\\usepackage{mypackage}\n\\begin{document}\n\\input{chapters/chapter}\n\\cite{example}\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}\n"),
        ("chapters/chapter.tex", "% !TeX root = ../main.tex\n\\input{shared}\n\\hello\n"),
        ("shared.tex", "Shared text."),
        ("mypackage.sty", "\\newcommand{\\hello}{Hello}"),
        ("references.bib", "@book{example, author={A}, title={Example}, year={2025}, publisher={P}}"),
    ] { std::fs::write(root.join(name),source).unwrap(); }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let files = [
        rename(root, "main.tex", "book/paper.tex"),
        rename(root, "chapters", "parts/content"),
        rename(root, "mypackage.sty", "renamedpackage.sty"),
        rename(root, "references.bib", "library.bib"),
    ];
    let edits = edits(
        file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap(),
    );
    for (uri, edits) in edits {
        let source = apply(&workspace.documents[&uri].text.source, edits);
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    std::fs::create_dir(root.join("book")).unwrap();
    std::fs::create_dir(root.join("parts")).unwrap();
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
        workspace
            .root_document(&uri(root, "parts/content/chapter.tex"))
            .unwrap(),
        main
    );
    let outcome = latex_lsp::build::run(
        &main.to_file_path().unwrap(),
        &Default::default(),
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert_eq!(
        outcome.status, "success",
        "{}\n{}\n{:?}",
        outcome.message, outcome.output, outcome.diagnostics
    );
    assert!(outcome.pdf.unwrap().is_file());
}

#[test]
fn invalid_batches_and_unrepresentable_references_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    assert!(
        file_operations::will_rename(&workspace, &[rename(root, "pkg.sty", "bad#name.sty")])
            .is_err()
    );
    assert!(file_operations::will_rename(
        &workspace,
        &[
            rename(root, "refs.bib", "new.bib"),
            rename(root, "other.bib", "new.bib")
        ]
    )
    .is_err());
    let remote = FileRename {
        old_uri: "https://example.com/main.tex".into(),
        new_uri: "file:///new.tex".into(),
    };
    assert!(file_operations::will_rename(&workspace, &[remote]).is_err());
    assert!(file_operations::will_rename(&workspace, &[])
        .unwrap()
        .is_none());
}
