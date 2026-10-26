use latex_lsp::{
    editing::file_operations, language::navigation, project::workspace::Workspace, text::Text,
};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).into(),
        new_uri: uri(root, new).into(),
    }
}
fn project(root: &Path, body: &str) -> Workspace {
    std::fs::write(
        root.join("main.tex"),
        format!("\\documentclass{{article}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n"),
    )
    .unwrap();
    std::fs::write(
        root.join("chapter.tex"),
        "\\typeout{SOURCE-NAME-AUDIT:EXPECTED}Read.\n",
    )
    .unwrap();
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(
        uri(root, "main.tex"),
        std::fs::read_to_string(root.join("main.tex")).unwrap(),
        7,
    );
    workspace
}
fn edits(edit: WorkspaceEdit) -> BTreeMap<Url, Vec<TextEdit>> {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned document edits")
    };
    documents
        .into_iter()
        .map(|doc| {
            assert_eq!(doc.text_document.version, Some(7));
            (
                doc.text_document.uri,
                doc.edits
                    .into_iter()
                    .map(|edit| match edit {
                        OneOf::Left(edit) => edit,
                        _ => panic!("unexpected annotated edit"),
                    })
                    .collect(),
            )
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

const CALLS: &[&str] = &[
    "\\input{chapter}",
    "\\input chapter\n",
    "\\input\"chapter\"\n",
    "\\include{chapter}",
    "\\InputIfFileExists{chapter}{}{\\errmessage{Missing expected file}}",
    "\\IfFileExists{chapter}{\\input{chapter}}{\\errmessage{Missing expected file}}",
];

#[test]
fn source_renames_preserve_leading_filename_spaces_and_explicit_trailing_stem_spaces() {
    for call in CALLS {
        for (new, expected) in [
            (" leading.tex", "./ leading"),
            ("chapter .tex", "chapter .tex"),
            ("single space.tex", "single space"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let workspace = project(root, call);
            let main = uri(root, "main.tex");
            let changes = edits(
                file_operations::will_rename(&workspace, &[rename(root, "chapter.tex", new)])
                    .unwrap()
                    .unwrap(),
            );
            let changed = apply(
                &workspace.documents[&main].text.source,
                changes[&main].clone(),
            );
            assert!(changed.contains(expected), "{call}/{new}: {changed}");
            if new == "chapter .tex" {
                assert!(!changed.contains("{chapter }"), "{changed}");
            }
        }
    }
}

#[test]
fn source_renames_reject_whitespace_that_would_change_the_selected_file() {
    for call in CALLS {
        for new in [
            "two  spaces.tex",
            "tab\tname.tex",
            "trail.tex ",
            "line\nname.tex",
            "control\u{7}name.tex",
            "nul\0name.tex",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let workspace = project(root, call);
            assert!(
                file_operations::will_rename(&workspace, &[rename(root, "chapter.tex", new)])
                    .is_err(),
                "{call}/{new:?}"
            );
        }
    }
}

#[test]
fn moving_roots_preserves_spaced_target_links_for_a_second_rename() {
    for (old, new) in [
        ("chapter.tex", " leading.tex"),
        ("chapter.tex", "chapter .tex"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut workspace = project(
            root,
            "\\input{chapter}\n% \\input{chapter}\n\\verb|\\input{chapter}|",
        );
        std::fs::create_dir(root.join("book")).unwrap();
        let files = [
            rename(root, old, new),
            rename(root, "main.tex", "book/paper.tex"),
        ];
        let main = uri(root, "main.tex");
        let changed = edits(
            file_operations::will_rename(&workspace, &files)
                .unwrap()
                .unwrap(),
        );
        let updated = apply(
            &workspace.documents[&main].text.source,
            changed[&main].clone(),
        );
        assert!(
            updated.contains(&format!("../{new}"))
                || new == " leading.tex" && updated.contains("../ leading"),
            "{updated}"
        );
        assert!(updated.contains("% \\input{chapter}\n\\verb|\\input{chapter}|"));
        workspace.open(main.clone(), updated.clone(), 8);
        std::fs::write(root.join("main.tex"), updated).unwrap();
        for file in &files {
            std::fs::rename(
                Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
                Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
            )
            .unwrap();
        }
        file_operations::did_rename(&mut workspace, &files).unwrap();
        let paper = uri(root, "book/paper.tex");
        let links = navigation::document_links(&workspace, &paper);
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(links[0].target, Some(uri(root, new)));
        // didRename updates the URI while retaining the unsaved buffer version.
        let edit = file_operations::will_rename(&workspace, &[rename(root, new, "second.tex")])
            .unwrap()
            .unwrap();
        let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
            panic!("expected versioned edits")
        };
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].text_document.version, Some(8));
        assert_eq!(documents[0].text_document.uri, paper);
        assert!(
            matches!(&documents[0].edits[0],OneOf::Left(edit) if edit.new_text=="../second.tex" || edit.new_text=="../second")
        );
    }
}

#[test]
fn root_comments_and_file_urls_keep_whitespace_that_is_not_tokenized_as_tex() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root, "\\input{chapter}");
    let main = uri(root, "main.tex");
    let child = uri(root, "chapter.tex");
    let source = format!("% !TeX root = main.tex\n\\url{{{main}#page=1}}\n");
    std::fs::write(root.join("chapter.tex"), &source).unwrap();
    workspace.open(child.clone(), source, 7);
    let changed = edits(
        file_operations::will_rename(&workspace, &[rename(root, "main.tex", "two  spaces.tex")])
            .unwrap()
            .unwrap(),
    );
    let updated = apply(
        &workspace.documents[&child].text.source,
        changed[&child].clone(),
    );
    assert!(
        updated.contains("% !TeX root = two  spaces.tex"),
        "{updated}"
    );
    assert!(updated.contains("two%20%20spaces.tex#page=1"), "{updated}");
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_source_file_renames_preserve_exact_inputs_across_engines_and_input_forms() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for call in CALLS {
            for new in [" leading.tex", "chapter .tex"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let mut workspace = project(root, call);
                std::fs::write(
                    root.join("leading.tex"),
                    "\\typeout{SOURCE-NAME-AUDIT:WRONG}Wrong.\n",
                )
                .unwrap();
                let compiler = root.join(engine);
                std::fs::write(&compiler,format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
                let config = latex_lsp::config::BuildConfig {
                    executable: compiler.to_string_lossy().into(),
                    ..Default::default()
                };
                let before = latex_lsp::compilation::build::run(
                    &root.join("main.tex"),
                    &config,
                    &Default::default(),
                );
                assert_eq!(
                    before.status, "success",
                    "{engine}/{call}: {}",
                    before.output
                );
                assert!(
                    before.output.contains("SOURCE-NAME-AUDIT:EXPECTED"),
                    "{}",
                    before.output
                );
                let main = uri(root, "main.tex");
                let files = [rename(root, "chapter.tex", new)];
                let changed = edits(
                    file_operations::will_rename(&workspace, &files)
                        .unwrap()
                        .unwrap(),
                );
                let updated = apply(
                    &workspace.documents[&main].text.source,
                    changed[&main].clone(),
                );
                workspace.open(main.clone(), updated.clone(), 8);
                std::fs::write(root.join("main.tex"), updated).unwrap();
                std::fs::rename(root.join("chapter.tex"), root.join(new)).unwrap();
                std::fs::write(
                    root.join("chapter.tex"),
                    "\\typeout{SOURCE-NAME-AUDIT:WRONG}Wrong.\n",
                )
                .unwrap();
                file_operations::did_rename(&mut workspace, &files).unwrap();
                let after = latex_lsp::compilation::build::run(
                    &root.join("main.tex"),
                    &config,
                    &Default::default(),
                );
                assert_eq!(
                    after.status, "success",
                    "{engine}/{call}/{new}: {}",
                    after.output
                );
                assert!(
                    after.output.contains("SOURCE-NAME-AUDIT:EXPECTED"),
                    "{engine}/{call}/{new}: {}",
                    after.output
                );
                assert!(
                    !after.output.contains("SOURCE-NAME-AUDIT:WRONG"),
                    "{}",
                    after.output
                );
                assert!(
                    after.inputs.unwrap().contains(&uri(root, new)),
                    "{engine}/{call}/{new}"
                );
                let links = navigation::document_links(&workspace, &main);
                assert!(
                    links.iter().all(|link| link.target == Some(uri(root, new))),
                    "{links:?}"
                );
                assert!(!links.is_empty());
            }
        }
    }
}

#[test]
fn directory_moves_with_leading_spaces_rebase_links_with_and_without_root_moves() {
    for move_root in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("parts")).unwrap();
        std::fs::create_dir(root.join("book")).unwrap();
        std::fs::write(root.join("parts/chapter.tex"), "Chapter").unwrap();
        let mut workspace = project(root, "\\input{parts/chapter}");
        let main = uri(root, "main.tex");
        let mut files = vec![rename(root, "parts", " leading")];
        if move_root {
            files.push(rename(root, "main.tex", "book/paper.tex"));
        }
        let changed = edits(
            file_operations::will_rename(&workspace, &files)
                .unwrap()
                .unwrap(),
        );
        let updated = apply(
            &workspace.documents[&main].text.source,
            changed[&main].clone(),
        );
        let expected = if move_root {
            "../ leading/chapter"
        } else {
            "./ leading/chapter"
        };
        assert!(updated.contains(expected), "{updated}");
        workspace.open(main.clone(), updated.clone(), 8);
        std::fs::write(root.join("main.tex"), updated).unwrap();
        for file in &files {
            std::fs::rename(
                Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
                Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
            )
            .unwrap();
        }
        file_operations::did_rename(&mut workspace, &files).unwrap();
        let main = if move_root {
            uri(root, "book/paper.tex")
        } else {
            main
        };
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(links[0].target, Some(uri(root, " leading/chapter.tex")));
    }
}
