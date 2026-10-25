use latex_lsp::{
    editing::file_operations,
    language::{features, navigation},
    project::workspace::{Document, Workspace},
};
use lsp_types::*;
use std::{collections::BTreeSet, path::Path};

const COMMANDS: [&str; 6] = [
    "input",
    "include",
    "subfile",
    "InputIfFileExists",
    "IfFileExists",
    "includeonly",
];
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn setup(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
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
fn call(name: &str, argument: &str) -> String {
    format!(
        "\\{name}{argument}{}",
        if matches!(name, "IfFileExists" | "InputIfFileExists") {
            "{Present}{Missing}"
        } else {
            ""
        }
    )
}
fn applied(workspace: &Workspace, edit: WorkspaceEdit, target: &Url) -> String {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edits")
    };
    let document = documents
        .iter()
        .find(|doc| &doc.text_document.uri == target)
        .unwrap();
    assert_eq!(
        document.text_document.version,
        workspace.documents[target].version
    );
    let mut edits: Vec<_> = document
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[target]
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
fn file_items(workspace: &Workspace, main: &Url, source: &str) -> Vec<CompletionItem> {
    let at = workspace.documents[main]
        .text
        .position(source.find("chapter").unwrap() + 3);
    features::completion(workspace, main, at)
        .into_iter()
        .filter(|item| {
            matches!(
                item.kind,
                Some(CompletionItemKind::FILE | CompletionItemKind::FOLDER)
            )
        })
        .collect()
}

#[test]
fn built_in_source_paths_remain_editable_in_direct_calls_and_future_macro_bodies() {
    for name in COMMANDS {
        for argument in ["{old/chapter}", "% before 😀\r\n{old/chapter.tex}"] {
            let command = call(name, argument);
            for body in [
                command.clone(),
                format!("\\newcommand{{\\future}}{{{command}}}\\future"),
                format!("\\def\\future{{{command}}}\\future"),
                format!("\\csdef{{future}}{{{command}}}\\csuse{{future}}"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!(r"\documentclass{{article}}{body}");
                let workspace = setup(
                    root,
                    &[("main.tex", &source), ("old/chapter.tex", "Chapter")],
                );
                let main = uri(root, "main.tex");
                let target = uri(root, "old/chapter.tex");
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
                    file_items(&workspace, &main, &source)
                        .iter()
                        .any(|item| item.label == "old/chapter.tex"),
                    "{name}/{body}"
                );
                for (old, new, expected) in [
                    (
                        "old/chapter.tex",
                        "old/renamed.tex",
                        source.replace("old/chapter", "old/renamed"),
                    ),
                    ("old", "moved", source.replace("old/", "moved/")),
                ] {
                    let edit = file_operations::will_rename(&workspace, &[rename(root, old, new)])
                        .unwrap()
                        .unwrap();
                    assert_eq!(applied(&workspace, edit, &main), expected, "{name}/{body}");
                }
            }
        }
    }
}

#[test]
fn redefined_and_consumed_source_calls_are_data_without_automatic_path_edits() {
    for name in COMMANDS {
        let command = call(name, "{old/chapter}");
        for (definitions, body) in [
            (format!("\\def\\{name}#1{{}}"), command.clone()),
            (String::new(), format!("\\hold{{{command}}}")),
            (String::new(), format!("\\hold{command}")),
            (String::new(), format!("\\take {command}STOP")),
            (String::new(), format!("\\string{command}")),
            (
                String::new(),
                format!("\\newcommand{{\\future}}{{\\hold{{{command}}}}}\\future"),
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!(
                r"\documentclass{{article}}\newcommand{{\hold}}[1]{{}}\def\take#1STOP{{}}{definitions}{body}"
            );
            let workspace = setup(
                root,
                &[("main.tex", &source), ("old/chapter.tex", "Chapter")],
            );
            let main = uri(root, "main.tex");
            let links = navigation::document_links(&workspace, &main);
            assert!(links.is_empty(), "{name}/{body}: {links:?}");
            let at = workspace.documents[&main]
                .text
                .position(source.find("chapter").unwrap() + 3);
            assert!(
                features::definitions(&workspace, &main, at).is_empty(),
                "{name}/{body}"
            );
            assert!(
                file_items(&workspace, &main, &source).is_empty(),
                "{name}/{body}"
            );
            for (old, new) in [("old/chapter.tex", "old/renamed.tex"), ("old", "moved")] {
                let edit =
                    file_operations::will_rename(&workspace, &[rename(root, old, new)]).unwrap();
                assert!(edit.is_none(), "{name}/{body}: {edit:?}");
            }
        }
    }
}

#[test]
fn recorded_signature_providers_can_change_direct_file_reference_meaning() {
    for name in COMMANDS {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!(r"\documentclass{{article}}{}", call(name, "{chapter}"));
        let mut workspace = setup(root, &[("main.tex", &source), ("chapter.tex", "Chapter")]);
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
        assert!(file_items(&workspace, &main, &source).is_empty());
        assert!(file_operations::will_rename(
            &workspace,
            &[rename(root, "chapter.tex", "renamed.tex")]
        )
        .unwrap()
        .is_none());
    }
}

#[test]
fn shared_source_edits_require_file_meaning_to_agree_across_build_roots() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = setup(
        root,
        &[
            ("first.tex", r"\documentclass{article}\input{shared}"),
            (
                "second.tex",
                r"\documentclass{article}\renewcommand{\include}[1]{}\input{shared}",
            ),
            ("shared.tex", r"\include{chapter}"),
            ("chapter.tex", "Chapter"),
        ],
    );
    let error =
        file_operations::will_rename(&workspace, &[rename(root, "chapter.tex", "renamed.tex")])
            .unwrap_err();
    assert_eq!(error.code, latex_lsp::rpc::ErrorCode::InvalidParams);
}
