use latex_lsp::{file_operations, navigation, workspace::Workspace};
use lsp_types::*;
use std::path::Path;

const FILES: [(&str, &str); 9] = [
    ("input", "tex"),
    ("include", "tex"),
    ("InputIfFileExists", "tex"),
    ("IfFileExists", "tex"),
    ("includeonly", "tex"),
    ("bibliography", "bib"),
    ("nobibliography", "bib"),
    ("addbibresource", "bib"),
    ("bibliographystyle", "bst"),
];
fn uri(root: &Path, file: &str) -> Url {
    Url::from_file_path(root.join(file)).unwrap()
}
fn project(root: &Path, body: &str, extension: &str) -> (Workspace, Url, Url) {
    std::fs::write(
        root.join("main.tex"),
        format!(r"\documentclass{{article}}\begin{{document}}{body}\end{{document}}"),
    )
    .unwrap();
    std::fs::write(root.join(format!("refs.{extension}")), "Data.").unwrap();
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for doc in workspace.documents.values_mut() {
        doc.version = Some(7);
    }
    (
        workspace,
        uri(root, "main.tex"),
        uri(root, &format!("refs.{extension}")),
    )
}
fn rename(workspace: &Workspace, target: &Url) -> Option<WorkspaceEdit> {
    let old_path = target.to_file_path().unwrap();
    let new_path = old_path.with_file_name(format!(
        "renamed.{}",
        old_path.extension().unwrap().to_str().unwrap()
    ));
    file_operations::will_rename(
        workspace,
        &[FileRename {
            old_uri: target.to_string(),
            new_uri: Url::from_file_path(new_path).unwrap().to_string(),
        }],
    )
    .unwrap()
}
fn call(name: &str) -> String {
    let value = if name == "addbibresource" {
        "refs.bib"
    } else {
        "refs"
    };
    format!(
        "\\{name}{{{value}}}{}",
        if matches!(name, "InputIfFileExists" | "IfFileExists") {
            "{}{}"
        } else {
            ""
        }
    )
}

#[test]
fn real_file_commands_after_detokenize_keep_links_and_only_the_actual_path_edit() {
    for (name, extension) in FILES {
        let command = call(name);
        for (definition, conversion) in [
            ("", r"\detokenize"),
            ("", r"\csname detokenize\endcsname"),
            ("", r"\csuse{detokenize}"),
            (r"\newcommand{\showtokens}{\detokenize}", r"\showtokens"),
        ] {
            for gap in ["", "% before 😀\r\n"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let body =
                    format!("{definition}{conversion}{gap}{{{command}}}% after\r\n{command}");
                let (workspace, main, target) = project(root, &body, extension);
                let edit = rename(&workspace, &target).unwrap_or_else(|| panic!("{name}/{body}"));
                let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
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
                let value = if name == "addbibresource" {
                    "refs.bib"
                } else {
                    "refs"
                };
                let replacement = if name == "addbibresource" {
                    "renamed.bib"
                } else {
                    "renamed"
                };
                let expected = doc.text.source.rfind(value).unwrap();
                assert_eq!(
                    edit.range,
                    doc.text.range(expected, expected + value.len()),
                    "{body}"
                );
                assert_eq!(edit.new_text, replacement);
                let links = navigation::document_links(&workspace, &main);
                let matching: Vec<_> = links
                    .iter()
                    .filter(|link| link.target.as_ref() == Some(&target))
                    .collect();
                assert_eq!(matching.len(), 1, "{body}: {links:?}");
                assert_eq!(matching[0].range, edit.range);
            }
        }
    }
}

#[test]
fn unknown_consumers_and_redefined_detokenize_keep_opaque_file_operands() {
    for body in [
        r"\unknown{Shown}\input{refs}",
        r"\newcommand{\detokenize}[2]{#1#2}\detokenize{Shown}\input{refs}",
        r"\newcommand{\detokenize}[2]{#1#2}\newcommand{\showtokens}{\detokenize}\showtokens{Shown}\input{refs}",
        r"\newcommand{\take}[2]{#1#2}\take{Shown}\input{refs}",
        r"\detokenize*\input{refs}",
        r"\newcommand{\install}{\newcommand{\detokenize}[2]{#1#2}}\install\detokenize{Shown}\input{refs}",
        r"\newcommand{\install}{\newcommand{\detokenize}[2]{#1#2}}\install\newcommand{\showtokens}{\detokenize}\showtokens{Shown}\input{refs}",
        r"\makeatletter\@input{provider.tex}\makeatother\relax ProviderDone \detokenize{Shown}\input{refs}",
        r"\csuse{@input}{provider.tex} ProviderDone \csuse{detokenize}{Shown}\input{refs}",
        r"\csname @input\endcsname{provider.tex} ProviderDone \csname detokenize\endcsname{Shown}\input{refs}",
        r"\csuse{\provider}{provider.tex} ProviderDone \detokenize{Shown}\input{refs}",
        r"\csname \provider\endcsname{provider.tex} ProviderDone \detokenize{Shown}\input{refs}",
        r"\input{\provider} ProviderDone \detokenize{Shown}\input{refs}",
        r"\newcommand{\install}{\input{provider}}\install ProviderDone \detokenize{Shown}\input{refs}",
        r"\newcommand{\install}{\input{provider}}\install\newcommand{\showtokens}{\detokenize}\showtokens{Shown}\input{refs}",
        r"\newcommand{\forward}[1]{#1}\forward{\input{provider}} ProviderDone \detokenize{Shown}\input{refs}",
        r"\scantokens{\input{provider}} ProviderDone \detokenize{Shown}\input{refs}",
        r"\csuse{input}{\provider} ProviderDone \detokenize{Shown}\input{refs}",
        r"\csname input\endcsname{\provider} ProviderDone \detokenize{Shown}\input{refs}",
        r"\newcommand{\showtokens}{\detokenize}\newcommand{\install}{\renewcommand{\showtokens}[2]{#1#2}}\install\showtokens{Shown}\input{refs}",
        r"\newcommand{\install}{\renewcommand{\csuse}[3]{#2}}\install\csuse{detokenize}{Shown}\input{refs}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (workspace, main, target) = project(directory.path(), body, "tex");
        assert!(rename(&workspace, &target).is_none(), "{body}");
        assert!(
            !navigation::document_links(&workspace, &main)
                .iter()
                .any(|link| link.target.as_ref() == Some(&target)),
            "{body}"
        );
    }
}

#[test]
fn completed_formatter_and_deferred_detokenize_calls_leave_following_paths_editable() {
    for body in [
        r"\textbf{Shown}\input{refs}",
        r"\newcommand{\showtokens}[1]{#1}\showtokens{Shown}\input{refs}",
        r"\newcommand{\future}{\detokenize{Shown}}\future\input{refs}",
        r"\newcommand{\showtokens}{\detokenize}\newcommand{\future}{\showtokens{Shown}}\future\input{refs}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (workspace, _, target) = project(directory.path(), body, "tex");
        assert!(rename(&workspace, &target).is_some(), "{body}");
    }
}
