use latex_lsp::{
    features, file_operations, navigation,
    workspace::{Document, Workspace},
};
use std::path::Path;
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
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
fn call(form: &str, loader: &str) -> String {
    match form {
        "direct" => format!("\\{loader}"),
        "named" => format!("\\csuse{{{loader}}}"),
        "literal" => format!("\\csname {loader}\\endcsname"),
        "alias" => r"\loadpkg".into(),
        "named-alias" => r"\csuse{loadpkg}".into(),
        "literal-alias" => r"\csname loadpkg\endcsname".into(),
        _ => panic!(),
    }
}
fn apply(workspace: &Workspace, edit: &WorkspaceEdit, target: &Url) -> String {
    let Some(DocumentChanges::Edits(documents)) = &edit.document_changes else {
        panic!("expected versioned edits")
    };
    let change = documents
        .iter()
        .find(|doc| &doc.text_document.uri == target)
        .unwrap();
    assert_eq!(
        change.text_document.version,
        workspace.documents[target].version
    );
    let mut edits: Vec<_> = change
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
fn items(workspace: &mut Workspace, target: &Url, marked: &str) -> Vec<CompletionItem> {
    let at = marked.find('|').unwrap();
    workspace.documents.insert(
        target.clone(),
        Document::new(target, marked.replace('|', ""), Some(7)),
    );
    features::completion(
        workspace,
        target,
        workspace.documents[target].text.position(at),
    )
}
fn complete(workspace: &mut Workspace, target: &Url, marked: &str, label: &str) -> String {
    let candidates = items(workspace, target, marked);
    let item = candidates
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{marked}: missing {label}: {candidates:?}"));
    let Some(CompletionTextEdit::Edit(main)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(main.range.start.line, main.range.end.line);
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(main.clone());
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    for pair in edits.windows(2) {
        assert!(pair[1].range.end <= pair[0].range.start);
    }
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
fn file_items(items: &[CompletionItem]) -> Vec<&CompletionItem> {
    items
        .iter()
        .filter(|item| {
            matches!(
                item.kind,
                Some(CompletionItemKind::FILE | CompletionItemKind::FOLDER)
            )
        })
        .collect()
}

#[test]
fn grouped_include_paths_keep_direct_and_indirect_navigation_completion_and_renames() {
    for form in [
        "direct",
        "named",
        "literal",
        "alias",
        "named-alias",
        "literal-alias",
    ] {
        for argument in [
            "{old/fixture}",
            "% before 😀\r\n{% file\r\nold/fixture.tex% tail\r\n}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!(
                "\\documentclass{{article}}\\def\\loadpkg{{\\include}}{}{argument}\\leaked",
                call(form, "include")
            );
            let mut workspace = project(
                root,
                &[
                    ("main.tex", &source),
                    ("old/fixture.tex", r"\newcommand{\leaked}{Leaked}"),
                ],
            );
            let main = uri(root, "main.tex");
            let target = uri(root, "old/fixture.tex");
            assert!(
                workspace.dependencies(&main).contains(&target),
                "{form}/{argument}"
            );
            let links = navigation::document_links(&workspace, &main);
            let link = links
                .iter()
                .find(|link| link.target.as_ref() == Some(&target))
                .unwrap();
            assert_eq!(
                features::definitions(&workspace, &main, link.range.start)[0].uri,
                target
            );
            let at = source.rfind(r"\leaked").unwrap() + 2;
            assert_eq!(
                features::definitions(
                    &workspace,
                    &main,
                    workspace.documents[&main].text.position(at)
                )[0]
                .uri,
                target
            );
            let at = source.find("fixture").unwrap() + 3;
            let marked = format!("{}|{}", &source[..at], &source[at..]);
            assert_eq!(
                complete(&mut workspace, &main, &marked, "old/fixture.tex"),
                source,
                "{form}/{argument}"
            );
            for (old, new, expected) in [
                (
                    "old/fixture.tex",
                    "old/renamed.tex",
                    source.replace("old/fixture", "old/renamed"),
                ),
                ("old", "moved", source.replace("old/", "moved/")),
            ] {
                let edit = file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: uri(root, old).to_string(),
                        new_uri: uri(root, new).to_string(),
                    }],
                )
                .unwrap()
                .unwrap();
                assert_eq!(
                    apply(&workspace, &edit, &main),
                    expected,
                    "{form}/{argument}"
                );
            }
        }
    }
}

#[test]
fn unsupported_direct_include_modifiers_do_not_import_or_navigate_body_groups() {
    for modifier in ["*", "[note]", "*[note]", "[first][second]"] {
        for argument in [
            "{chapter}",
            "% before 😀\r\n{% file\r\nchapter.tex% tail\r\n}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\documentclass{{article}}\\include{modifier}{argument}\\leaked");
            let mut workspace = project(
                root,
                &[
                    ("main.tex", &source),
                    ("chapter.tex", r"\newcommand{\leaked}{Leaked}"),
                ],
            );
            let main = uri(root, "main.tex");
            let target = uri(root, "chapter.tex");
            assert!(
                !workspace.dependencies(&main).contains(&target),
                "{modifier}/{argument}"
            );
            assert!(
                navigation::document_links(&workspace, &main).is_empty(),
                "{modifier}/{argument}"
            );
            for at in [
                source.find("chapter").unwrap() + 2,
                source.rfind(r"\leaked").unwrap() + 2,
            ] {
                assert!(
                    features::definitions(
                        &workspace,
                        &main,
                        workspace.documents[&main].text.position(at)
                    )
                    .is_empty(),
                    "{modifier}/{argument}"
                );
            }
            let at = source.find("chapter").unwrap() + 3;
            let marked = format!("{}|{}", &source[..at], &source[at..]);
            assert!(
                file_items(&items(&mut workspace, &main, &marked)).is_empty(),
                "{modifier}/{argument}"
            );
        }
    }
}

#[test]
fn unsupported_include_body_text_stays_unchanged_when_files_or_directories_move() {
    for modifier in ["*", "[note]", "*[note]", "[first][second]"] {
        for suffix in ["", ".tex"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\documentclass{{article}}\\begin{{document}}\\include{modifier}% before 😀\r\n{{% body\r\nold/chapter{suffix}% tail\r\n}}\\end{{document}}");
            let workspace = project(
                root,
                &[("main.tex", &source), ("old/chapter.tex", "Chapter")],
            );
            for (old, new) in [("old/chapter.tex", "old/renamed.tex"), ("old", "moved")] {
                let edit = file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: uri(root, old).to_string(),
                        new_uri: uri(root, new).to_string(),
                    }],
                )
                .unwrap();
                assert!(edit.is_none(), "{modifier}/{suffix}: {edit:?}");
            }
        }
    }
}
