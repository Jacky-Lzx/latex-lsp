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
fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

#[test]
fn commented_includeonly_paths_link_and_rename_precisely_without_loading_chapters() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("chapters")).unwrap();
    for name in ["real", "other chapter", "phantom"] {
        std::fs::write(
            root.join(format!("chapters/{name}.tex")),
            format!("\\label{{{name}}}\n"),
        )
        .unwrap();
    }
    let source = "\\documentclass{article}\r\n\\includeonly{% emoji😀, chapters/phantom, } ignored\r\n chapters/real% tail\r\n ,% separator, } ignored\r\n chapters/other chapter.tex% second tail\r\n}\r\n\\verb|\\includeonly{chapters/real}|\r\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace.open(main.clone(), source.into(), 7);
    assert_eq!(workspace.project_documents(&main), [main.clone()].into());
    let links = navigation::document_links(&workspace, &main);
    assert_eq!(links.len(), 2, "{links:?}");
    let text = Text::new(source.into());
    for (name, literal) in [
        ("real", "chapters/real"),
        ("other chapter", "chapters/other chapter.tex"),
    ] {
        let link = links
            .iter()
            .find(|link| link.target == Some(uri(root, &format!("chapters/{name}.tex"))))
            .unwrap();
        assert_eq!(
            &source[text.offset(link.range.start).unwrap()..text.offset(link.range.end).unwrap()],
            literal
        );
    }
    assert!(file_operations::will_rename(
        &workspace,
        &[rename(root, "chapters/phantom.tex", "chapters/unused.tex")]
    )
    .unwrap()
    .is_none());
    let edit = file_operations::will_rename(
        &workspace,
        &[rename(root, "chapters/real.tex", "chapters/renamed.tex")],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(edit.len(), 1);
    assert_eq!(edit[&main].len(), 1);
    assert_eq!(
        apply(source, &edit[&main]),
        source.replace("\n chapters/real%", "\n chapters/renamed%")
    );
    let edit = file_operations::will_rename(&workspace, &[rename(root, "chapters", "sections")])
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edit[&main].len(), 2);
    assert_eq!(
        apply(source, &edit[&main]),
        source
            .replace("\n chapters/real%", "\n sections/real%")
            .replace(
                "\n chapters/other chapter.tex%",
                "\n sections/other chapter.tex%"
            )
    );
}

#[test]
fn commented_includeonly_completion_replaces_only_the_current_filename() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), "Chapter.").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (arguments, expected) in [
        (
            "{% before, }\n chap¦BAD% tail\n,other}",
            "{% before, }\n chapter% tail\n,other}",
        ),
        (
            "{other,% before, }\r\nchap¦BAD% tail\r\n}",
            "{other,% before, }\r\nchapter% tail\r\n}",
        ),
        ("{% before\n¦% after\n}", "{% before\nchapter% after\n}"),
        ("{% before\nchap¦", "{% before\nchapter"),
    ] {
        let marked = format!("\\includeonly{arguments}");
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        let position = Text::new(source.clone()).position(marked.find('¦').unwrap());
        let item = features::completion(&workspace, &main, position)
            .into_iter()
            .find(|item| item.label == "chapter.tex")
            .unwrap_or_else(|| panic!("{marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        assert_eq!(
            apply(&source, &edits),
            format!("\\includeonly{expected}"),
            "{marked}"
        );
    }
}

#[test]
fn uncertain_includeonly_names_do_not_produce_partial_links_or_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), "Chapter.").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for arguments in [
        r"*{chap¦ter}",
        r"[Note]{chap¦ter}",
        "{chap¦% joined\nter}",
        r"{chap¦ter\suffix}",
        r"{{chap¦ter}}",
        "{% comment chap¦ter\n}",
    ] {
        let marked = format!("\\includeonly{arguments}");
        let source = marked.replace('¦', "");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        workspace.open(main.clone(), source.clone(), 7);
        assert!(
            navigation::document_links(&workspace, &main).is_empty(),
            "{marked}"
        );
        let position = Text::new(source).position(marked.find('¦').unwrap());
        assert!(
            !features::completion(&workspace, &main, position)
                .iter()
                .any(|item| item.label == "chapter.tex"),
            "{marked}"
        );
        assert!(
            file_operations::will_rename(&workspace, &[rename(root, "chapter.tex", "renamed.tex")])
                .unwrap()
                .is_none(),
            "{marked}"
        );
    }
}
