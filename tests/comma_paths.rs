use lsp_types::*;
use std::path::Path;
use texquill::{
    editing::file_operations,
    language::{features, navigation},
    project::workspace::Workspace,
    text::Text,
};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
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

fn complete(workspace: &mut Workspace, main: &Url, marked: &str, label: &str) -> String {
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    workspace.open(main.clone(), source.clone(), 3);
    let items = features::completion(workspace, main, Text::new(source.clone()).position(offset));
    let item = items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{marked}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    let start = Text::new(source.clone()).offset(edit.range.start).unwrap();
    assert!(item
        .filter_text
        .as_ref()
        .unwrap()
        .starts_with(&source[start..offset]));
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    apply(&source, edits)
}

#[test]
fn single_file_completion_keeps_commas_in_filenames_and_directories() {
    for (command, extension, suffix) in [
        ("includegraphics", "pdf", ".pdf"),
        ("include", "tex", ""),
        ("subfile", "tex", ""),
        ("addbibresource", "bib", ".bib"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("main.tex"), "").unwrap();
        std::fs::create_dir(root.join("dir,one")).unwrap();
        for name in [
            format!("plot,one.{extension}"),
            format!("plot, one.{extension}"),
            format!("dir,one/plot,one.{extension}"),
        ] {
            std::fs::write(root.join(name), "").unwrap();
        }
        let main = uri(root, "main.tex");
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        for (typed, label, replacement) in [
            (
                "plot,o¦ne",
                format!("plot,one.{extension}"),
                format!("plot,one{suffix}"),
            ),
            (
                "plot, o¦ld",
                format!("plot, one.{extension}"),
                format!("plot, one{suffix}"),
            ),
            (
                "dir,o¦ld/plot,one",
                "dir,one/".into(),
                "dir,one/plot,one".into(),
            ),
            (
                "dir,one/plot,o¦ld",
                format!("dir,one/plot,one.{extension}"),
                format!("dir,one/plot,one{suffix}"),
            ),
        ] {
            let marked = format!("😀 \\{command}{{{typed}}}{{body, untouched}}");
            assert_eq!(
                complete(&mut workspace, &main, &marked, &label),
                format!("😀 \\{command}{{{replacement}}}{{body, untouched}}")
            );
        }
        // An explicit suffix remains explicit when replacing inside its name.
        let marked = format!("\\{command}{{plot,o¦ld.{extension}}}");
        assert_eq!(
            complete(
                &mut workspace,
                &main,
                &marked,
                &format!("plot,one.{extension}")
            ),
            format!("\\{command}{{plot,one.{extension}}}")
        );
    }
}

#[test]
fn graphics_optional_calls_and_nested_roots_complete_then_link_and_rename_the_same_file() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir_all(root.join("chapters")).unwrap();
    std::fs::create_dir_all(root.join("images,one")).unwrap();
    std::fs::create_dir_all(root.join("chapters/images,one")).unwrap();
    let main_source = "\\documentclass{article}\\usepackage{graphicx}\\input{chapters/child}\n";
    std::fs::write(root.join("main.tex"), main_source).unwrap();
    std::fs::write(root.join("images,one/图,one.pdf"), "expected").unwrap();
    std::fs::write(root.join("chapters/images,one/图,one.pdf"), "decoy").unwrap();
    let child = uri(root, "chapters/child.tex");
    for invocation in [
        "includegraphics",
        "includegraphics*",
        "includegraphics[width=1cm]",
        "includegraphics*[width=1cm]",
    ] {
        let marked = format!("% 😀\r\n\\{invocation} % keep\r\n{{images,one/图,o¦ld.pdf}}\r\n");
        std::fs::write(child.to_file_path().unwrap(), marked.replace('¦', "")).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let completed = complete(&mut workspace, &child, &marked, "images,one/图,one.pdf");
        assert_eq!(completed, marked.replace("图,o¦ld", "图,one"));
        workspace.open(child.clone(), completed.clone(), 4);
        let links = navigation::document_links(&workspace, &child);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, Some(uri(root, "images,one/图,one.pdf")));
        workspace.client.versioned_edits = true;
        let changes = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(root, "images,one/图,one.pdf").into(),
                new_uri: uri(root, "images,one/图,two.pdf").into(),
            }],
        )
        .unwrap()
        .unwrap();
        let Some(DocumentChanges::Edits(documents)) = changes.document_changes else {
            panic!("expected versioned edit")
        };
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].text_document.uri, child);
        assert_eq!(documents[0].text_document.version, Some(4));
        let edits = documents[0]
            .edits
            .iter()
            .map(|edit| match edit {
                OneOf::Left(edit) => edit.clone(),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            apply(&completed, edits),
            completed.replace("图,one", "图,two")
        );
    }
}

#[test]
fn list_file_completion_still_replaces_only_the_current_entry() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in [
        "main.tex", "one.tex", "two.tex", "one.bib", "two.bib", "one.sty", "two.sty",
    ] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (command, extension) in [
        ("includeonly", "tex"),
        ("bibliography", "bib"),
        ("usepackage", "sty"),
    ] {
        let marked = format!("\\{command}{{one, t¦ypo, untouched}}");
        assert_eq!(
            complete(&mut workspace, &main, &marked, &format!("two.{extension}")),
            format!("\\{command}{{one, two, untouched}}")
        );
    }
}
