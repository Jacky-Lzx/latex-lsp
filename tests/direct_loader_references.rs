use latex_lsp::{
    editing::file_operations,
    language::{features, navigation},
    project::workspace::Workspace,
};
use lsp_types::*;
use std::path::Path;

const COMMANDS: [(&str, &str); 8] = [
    ("usepackage", "sty"),
    ("RequirePackage", "sty"),
    ("RequirePackageWithOptions", "sty"),
    ("documentclass", "cls"),
    ("LoadClass", "cls"),
    ("LoadClassWithOptions", "cls"),
    ("PassOptionsToPackage", "sty"),
    ("PassOptionsToClass", "cls"),
];
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn project(root: &Path, body: &str, extension: &str) -> (Workspace, Url, Url) {
    std::fs::write(
        root.join("main.tex"),
        format!(r"\documentclass{{article}}\begin{{document}}{body}\end{{document}}"),
    )
    .unwrap();
    std::fs::write(root.join(format!("refs.{extension}")), "Data.").unwrap();
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    for document in workspace.documents.values_mut() {
        document.version = Some(7);
    }
    (
        workspace,
        uri(root, "main.tex"),
        uri(root, &format!("refs.{extension}")),
    )
}
fn call(name: &str) -> String {
    format!(
        "\\{name}{}{{refs}}",
        if name.starts_with("PassOptions") {
            "{unused}"
        } else {
            ""
        }
    )
}
fn renaming(workspace: &Workspace, target: &Url) -> Option<WorkspaceEdit> {
    let path = target.to_file_path().unwrap();
    file_operations::will_rename(
        workspace,
        &[FileRename {
            old_uri: target.to_string(),
            new_uri: Url::from_file_path(path.with_file_name(format!(
                "renamed.{}",
                path.extension().unwrap().to_str().unwrap()
            )))
            .unwrap()
            .into(),
        }],
    )
    .unwrap()
}
fn candidates(workspace: &Workspace, main: &Url, at: usize) -> Vec<CompletionItem> {
    let doc = &workspace.documents[main];
    features::completion(workspace, main, doc.text.position(at))
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
fn displayed_loader_paths_have_no_navigation_completion_or_file_rename_edits() {
    for (name, extension) in COMMANDS {
        let command = call(name);
        for body in [
            format!("\\meaning{command}"), format!("\\string{command}"),
            format!("\\detokenize{{{command}}}"),
            format!("\\csuse{{detokenize}}{{{command}}}"),
            format!("\\csname detokenize\\endcsname{{{command}}}"),
            format!("\\newcommand{{\\lspShowTokensUnique}}{{\\detokenize}}\\lspShowTokensUnique{{{command}}}"),
            format!("\\newcommand{{\\discard}}[1]{{}}\\discard{{{command}}}"),
        ] {
            let dir=tempfile::tempdir().unwrap();let (workspace,main,target)=project(dir.path(),&body,extension);
            assert!(!navigation::document_links(&workspace,&main).iter().any(|link|link.target.as_ref()==Some(&target)),"{body}");
            assert!(renaming(&workspace,&target).is_none(),"{body}");
            let doc=&workspace.documents[&main];let offset=doc.text.source.rfind("refs").unwrap()+2;
            assert!(features::definitions(&workspace,&main,doc.text.position(offset)).is_empty(),"{body}");
            assert!(candidates(&workspace,&main,offset).is_empty(),"{body}");
        }
    }
}
#[test]
fn actual_loader_after_display_keeps_one_versioned_edit_and_its_path_completion() {
    for (name, extension) in COMMANDS {
        let command = call(name);
        for conversion in [
            r"\detokenize",
            r"\csuse{detokenize}",
            r"\csname detokenize\endcsname",
        ] {
            let body = format!("{conversion}% before 😀\r\n{{{command}}}% after\r\n{command}");
            let dir = tempfile::tempdir().unwrap();
            let (workspace, main, target) = project(dir.path(), &body, extension);
            let Some(DocumentChanges::Edits(documents)) =
                renaming(&workspace, &target).unwrap().document_changes
            else {
                panic!()
            };
            assert_eq!(documents.len(), 1, "{body}");
            let document = &documents[0];
            assert_eq!(document.text_document.uri, main);
            assert_eq!(document.text_document.version, Some(7));
            assert_eq!(document.edits.len(), 1, "{body}");
            let OneOf::Left(edit) = &document.edits[0] else {
                panic!()
            };
            let doc = &workspace.documents[&main];
            let at = doc.text.source.rfind("refs").unwrap();
            assert_eq!(edit.range, doc.text.range(at, at + 4), "{body}");
            assert_eq!(edit.new_text, "renamed");
            let links = navigation::document_links(&workspace, &main);
            let matching: Vec<_> = links
                .iter()
                .filter(|link| link.target.as_ref() == Some(&target))
                .collect();
            assert_eq!(matching.len(), 1, "{body}: {links:?}");
            assert_eq!(matching[0].range, edit.range);
            assert!(
                candidates(&workspace, &main, at + 2)
                    .iter()
                    .any(|item| item.label == format!("refs.{extension}")),
                "{body}"
            );
        }
    }
}
#[test]
fn direct_and_deferred_builtin_loaders_keep_paths_but_overridden_consumers_do_not() {
    for (name, extension) in COMMANDS {
        let command = call(name);
        for body in [
            command.clone(),
            format!("\\newcommand{{\\future}}{{{command}}}\\future"),
            format!("\\textbf{{{command}}}"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (workspace, main, target) = project(dir.path(), &body, extension);
            assert!(renaming(&workspace, &target).is_some(), "{body}");
            assert!(
                navigation::document_links(&workspace, &main)
                    .iter()
                    .any(|link| link.target.as_ref() == Some(&target)),
                "{body}"
            );
        }
        let arity = if name.starts_with("PassOptions") {
            2
        } else {
            1
        };
        let body = format!("\\renewcommand{{\\{name}}}[{arity}]{{}}{command}");
        let dir = tempfile::tempdir().unwrap();
        let (workspace, main, target) = project(dir.path(), &body, extension);
        assert!(renaming(&workspace, &target).is_none(), "{body}");
        assert!(
            !navigation::document_links(&workspace, &main)
                .iter()
                .any(|link| link.target.as_ref() == Some(&target)),
            "{body}"
        );
    }
}
