use latex_lsp::{
    features, file_operations, navigation,
    workspace::{Document, Workspace},
};
use std::{collections::BTreeSet, path::Path};
use tower_lsp::lsp_types::*;

const COMMANDS: [&str; 4] = [
    "bibliography",
    "nobibliography",
    "addbibresource",
    "bibliographystyle",
];
fn extension(name: &str) -> &str {
    if name == "bibliographystyle" {
        "bst"
    } else {
        "bib"
    }
}
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn project(root: &Path, source: &str, name: &str) -> Workspace {
    std::fs::create_dir_all(root.join("old")).unwrap();
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(
        root.join(format!("old/refs.{}", extension(name))),
        "@book{alpha,title={Data}}\n@comment{\\def\\bibliography#1{}}\n",
    )
    .unwrap();
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for doc in workspace.documents.values_mut() {
        doc.version = Some(7);
    }
    workspace
}
fn file_items(workspace: &Workspace, main: &Url) -> Vec<CompletionItem> {
    let doc = &workspace.documents[main];
    features::completion(
        workspace,
        main,
        doc.text.position(doc.text.source.find("refs").unwrap() + 2),
    )
    .into_iter()
    .filter(|item| {
        matches!(
            item.kind,
            Some(CompletionItemKind::FILE | CompletionItemKind::FOLDER)
        )
    })
    .collect()
}
fn applied(workspace: &Workspace, edit: WorkspaceEdit, main: &Url) -> String {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!()
    };
    let document = documents
        .iter()
        .find(|doc| &doc.text_document.uri == main)
        .unwrap();
    assert_eq!(document.text_document.version, Some(7));
    let mut edits: Vec<_> = document
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[main]
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
fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).into(),
        new_uri: uri(root, new).into(),
    }
}

#[test]
fn builtin_resource_paths_keep_navigation_completion_and_versioned_moves_in_future_macro_code() {
    for name in COMMANDS {
        for argument in [
            "{old/refs}".to_string(),
            format!("% before 😀\r\n{{old/refs.{}}}", extension(name)),
        ] {
            let command = format!("\\{name}{argument}");
            for body in [
                command.clone(),
                format!("\\newcommand{{\\future}}{{{command}}}\\future"),
                format!("\\def\\future{{{command}}}\\future"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!(r"\documentclass{{article}}{body}");
                let workspace = project(root, &source, name);
                let main = uri(root, "main.tex");
                let target = uri(root, &format!("old/refs.{}", extension(name)));
                let links = navigation::document_links(&workspace, &main);
                let link = links
                    .iter()
                    .find(|link| link.target.as_ref() == Some(&target))
                    .unwrap_or_else(|| panic!("{name}/{body}: {links:?}"));
                assert_eq!(
                    features::definitions(&workspace, &main, link.range.start)[0].uri,
                    target
                );
                assert!(
                    file_items(&workspace, &main)
                        .iter()
                        .any(|item| item.label == format!("old/refs.{}", extension(name))),
                    "{name}/{body}"
                );
                for (old, new, expected) in [
                    (
                        format!("old/refs.{}", extension(name)),
                        format!("old/renamed.{}", extension(name)),
                        source.replace("old/refs", "old/renamed"),
                    ),
                    (
                        "old".into(),
                        "moved".into(),
                        source.replace("old/", "moved/"),
                    ),
                ] {
                    let edit =
                        file_operations::will_rename(&workspace, &[rename(root, &old, &new)])
                            .unwrap()
                            .unwrap();
                    assert_eq!(applied(&workspace, edit, &main), expected, "{name}/{body}");
                }
                let suffix_change = file_operations::will_rename(
                    &workspace,
                    &[rename(
                        root,
                        &format!("old/refs.{}", extension(name)),
                        "old/renamed.txt",
                    )],
                );
                if name == "addbibresource" {
                    // Explicit resource names keep their suffix, unlike the
                    // automatic suffix rules of traditional BibTeX commands.
                    let edit = suffix_change.unwrap().unwrap();
                    let expected = source
                        .replace("old/refs.bib", "old/renamed.txt")
                        .replace("old/refs", "old/renamed.txt");
                    assert_eq!(applied(&workspace, edit, &main), expected);
                } else {
                    assert!(suffix_change.is_err(), "{name}/{body}");
                }
            }
        }
    }
}

#[test]
fn overridden_and_consumed_resource_calls_keep_filename_text_out_of_automatic_file_edits() {
    for name in COMMANDS {
        for argument in [
            "{old/refs}".to_string(),
            format!("% before 😀\r\n{{old/refs.{}}}", extension(name)),
        ] {
            let command = format!("\\{name}{argument}");
            for body in [
                format!("\\def\\{name}#1{{}}{command}"),
                format!("\\hold{{{command}}}"),
                format!("\\hold{command}"),
                format!("\\take {command}STOP"),
                format!("\\string{command}"),
                format!("\\meaning{command}"),
                format!("\\newcommand{{\\future}}{{\\hold{{{command}}}}}\\future"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!(
                    r"\documentclass{{article}}\newcommand{{\hold}}[1]{{}}\def\take#1STOP{{}}{body}"
                );
                let workspace = project(root, &source, name);
                let main = uri(root, "main.tex");
                let links = navigation::document_links(&workspace, &main);
                assert!(links.is_empty(), "{name}/{body}: {links:?}");
                let at = workspace.documents[&main]
                    .text
                    .position(source.find("refs").unwrap() + 2);
                assert!(
                    features::definitions(&workspace, &main, at).is_empty(),
                    "{name}/{body}"
                );
                assert!(file_items(&workspace, &main).is_empty(), "{name}/{body}");
                for (old, new) in [
                    (
                        format!("old/refs.{}", extension(name)),
                        format!("old/renamed.{}", extension(name)),
                    ),
                    ("old".into(), "moved".into()),
                ] {
                    assert!(
                        file_operations::will_rename(&workspace, &[rename(root, &old, &new)])
                            .unwrap()
                            .is_none(),
                        "{name}/{body}"
                    );
                }
            }
        }
    }
}

#[test]
fn recorded_source_providers_can_change_resource_file_reference_meaning() {
    for name in COMMANDS {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!(r"\documentclass{{article}}\{name}{{old/refs}}");
        let mut workspace = project(root, &source, name);
        let main = uri(root, "main.tex");
        assert_eq!(navigation::document_links(&workspace, &main).len(), 1);
        let provider = uri(root, "provider.tex");
        let text = format!("\\def\\{name}#1{{}}");
        std::fs::write(root.join("provider.tex"), &text).unwrap();
        workspace
            .documents
            .insert(provider.clone(), Document::new(&provider, text, Some(3)));
        workspace
            .recorded_inputs
            .insert(main.clone(), BTreeSet::from([provider]));
        assert!(navigation::document_links(&workspace, &main).is_empty());
        assert!(file_items(&workspace, &main).is_empty());
        assert!(file_operations::will_rename(
            &workspace,
            &[rename(
                root,
                &format!("old/refs.{}", extension(name)),
                &format!("old/renamed.{}", extension(name))
            )]
        )
        .unwrap()
        .is_none());
    }
}

#[test]
fn shared_resource_paths_require_the_same_file_meaning_in_every_project() {
    for name in COMMANDS {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut workspace = project(root, r"\documentclass{article}\input{shared}", name);
        for (file, source) in [
            (
                "second.tex",
                format!(r"\documentclass{{article}}\def\{name}#1{{}}\input{{shared}}"),
            ),
            ("shared.tex", format!(r"\{name}{{old/refs}}")),
        ] {
            std::fs::write(root.join(file), &source).unwrap();
            let target = uri(root, file);
            workspace
                .documents
                .insert(target.clone(), Document::new(&target, source, Some(7)));
        }
        let error = file_operations::will_rename(
            &workspace,
            &[rename(
                root,
                &format!("old/refs.{}", extension(name)),
                &format!("old/renamed.{}", extension(name)),
            )],
        )
        .unwrap_err();
        assert_eq!(
            error.code,
            tower_lsp::jsonrpc::ErrorCode::InvalidParams,
            "{name}"
        );
    }
}
