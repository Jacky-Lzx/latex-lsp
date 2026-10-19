use latex_lsp::{
    features, file_operations, navigation, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use std::path::Path;
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
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

#[test]
fn hidden_bibliographies_load_external_databases_and_resolve_inline_entry_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("project")).unwrap();
    std::fs::create_dir(root.join("shared")).unwrap();
    std::fs::write(root.join("shared/refs.bib"), "@book{alpha,title={Title}}").unwrap();
    std::fs::write(root.join("shared/extra.bib"), "@book{beta,title={Other}}").unwrap();
    let source = "\\documentclass{article}\n\\nobibliography{../shared/re fs, ../shared/extra.bib}\n\\bibentry{alpha}\\bibentry{beta}\n";
    std::fs::write(root.join("project/main.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.join("project"));
    let main = uri(root, "project/main.tex");
    workspace.open(main.clone(), source.into(), 7);
    let refs = uri(root, "shared/refs.bib");
    let extra = uri(root, "shared/extra.bib");
    let files = workspace.project_files(&main);
    for target in [&refs, &extra] {
        assert!(files.contains(target), "{files:?}");
        assert!(workspace.documents.contains_key(target));
        assert!(navigation::document_links(&workspace, &main)
            .iter()
            .any(|link| link.target.as_ref() == Some(target)));
    }
    for (key, target) in [("alpha", &refs), ("beta", &extra)] {
        let offset = source.rfind(key).unwrap() + 2;
        let definitions =
            features::definitions(&workspace, &main, Text::new(source.into()).position(offset));
        assert_eq!(definitions.len(), 1);
        assert_eq!(&definitions[0].uri, target);
    }
}

#[test]
fn hidden_bibliography_file_renames_preserve_lists_extensions_and_other_document_text() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("db")).unwrap();
    for name in ["refs", "extra"] {
        std::fs::write(
            root.join(format!("db/{name}.bib")),
            format!("@book{{{name},title={{Title}}}}"),
        )
        .unwrap();
    }
    let source = "\\documentclass{article}\n\\nobibliography{db/refs, db/extra.bib}\n% \\nobibliography{db/refs}\n\\verb|\\nobibliography{db/refs}|\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    let files = [
        FileRename {
            old_uri: uri(root, "db/refs.bib").to_string(),
            new_uri: uri(root, "db/renamed.bib").to_string(),
        },
        FileRename {
            old_uri: uri(root, "db/extra.bib").to_string(),
            new_uri: uri(root, "db/additional.bib").to_string(),
        },
    ];
    let changes = file_operations::will_rename(&workspace, &files)
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[&main].len(), 2);
    assert_eq!(
        apply(source, &changes[&main]),
        source.replacen(
            "{db/refs, db/extra.bib}",
            "{db/renamed, db/additional.bib}",
            1
        )
    );
    let invalid = FileRename {
        new_uri: uri(root, "db/refs.tex").to_string(),
        ..files[0].clone()
    };
    assert!(file_operations::will_rename(&workspace, &[invalid])
        .unwrap_err()
        .message
        .contains(".bib"));
}

#[test]
fn hidden_bibliography_completion_handles_lists_incomplete_arguments_and_crlf_spaces() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("db")).unwrap();
    std::fs::write(root.join("db/refs.bib"), "@book{alpha,title={Title}}").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (marked, expected) in [
        (
            r"\nobibliography{db/re¦BAD, other}",
            r"\nobibliography{db/refs, other}",
        ),
        (
            r"\nobibliography{other, db/re¦BAD.bib}",
            r"\nobibliography{other, db/refs.bib}",
        ),
        (
            "\\nobibliography{other,\r\n db/re¦BAD}",
            "\\nobibliography{other,\r\n db/refs}",
        ),
        (r"\nobibliography{db/re¦", r"\nobibliography{db/refs"),
    ] {
        let source = marked.replace('¦', "");
        workspace
            .documents
            .insert(main.clone(), Document::new(&main, source.clone(), Some(7)));
        let at = Text::new(source.clone()).position(marked.find('¦').unwrap());
        let item = features::completion(&workspace, &main, at)
            .into_iter()
            .find(|item| item.label == "db/refs.bib")
            .unwrap_or_else(|| panic!("{marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit");
        };
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        assert_eq!(apply(&source, &edits), expected, "{marked}");
        let help = signature::help(&workspace, &main, at).unwrap();
        assert_eq!(help.signatures[0].label, "\\nobibliography{databases}");
        assert_eq!(help.active_parameter, Some(0));
    }
}

#[test]
fn no_argument_star_and_unsupported_notes_do_not_link_complete_or_rename_following_groups() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("refs.bib"), "@book{alpha,title={Title}}").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let files = [FileRename {
        old_uri: uri(root, "refs.bib").to_string(),
        new_uri: uri(root, "renamed.bib").to_string(),
    }];
    for marked in [
        r"\nobibliography*{re¦fs}",
        r"\nobibliography *{re¦fs}",
        "\\nobibliography% comment\n*{re¦fs}",
        r"\nobibliography[Note]{re¦fs}",
        r"\nobibliography{refs} Body {re¦fs}",
    ] {
        let source = marked.replace('¦', "");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        workspace.open(main.clone(), source.clone(), 7);
        let at = Text::new(source).position(marked.find('¦').unwrap());
        assert!(
            !features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.label == "refs.bib"),
            "{marked}"
        );
        assert!(signature::help(&workspace, &main, at).is_none(), "{marked}");
        if marked.contains("Body") {
            assert_eq!(navigation::document_links(&workspace, &main).len(), 1);
            let changes = file_operations::will_rename(&workspace, &files)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes[&main].len(), 1);
        } else {
            assert!(
                navigation::document_links(&workspace, &main).is_empty(),
                "{marked}"
            );
            assert!(
                !workspace
                    .dependencies(&main)
                    .contains(&uri(root, "refs.bib")),
                "{marked}"
            );
            assert!(
                file_operations::will_rename(&workspace, &files)
                    .unwrap()
                    .is_none(),
                "{marked}"
            );
        }
    }
}
