use latex_lsp::{
    features, file_operations, navigation,
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
fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

#[test]
fn commented_database_lists_load_only_actual_external_files_with_precise_links() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("project")).unwrap();
    std::fs::create_dir(root.join("shared")).unwrap();
    for (name, key) in [
        ("refs_v1", "alpha"),
        ("other", "beta"),
        ("phantom", "unused"),
    ] {
        std::fs::write(
            root.join(format!("shared/{name}.bib")),
            format!("@book{{{key},title={{Title}}}}"),
        )
        .unwrap();
    }
    for command in ["bibliography", "nobibliography"] {
        let source = format!("\\documentclass{{article}}\r\n\\{command}{{% emoji😀, ../shared/phantom, }} ignored\r\n ../shared/re fs\\_v1% first tail\r\n ,% separator, }} ignored\r\n ../shared/other% other tail\r\n}}\r\n\\bibentry{{alpha}}\\bibentry{{beta}}\r\n");
        std::fs::write(root.join("project/main.tex"), &source).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.join("project"));
        let main = uri(root, "project/main.tex");
        workspace.open(main.clone(), source.clone(), 7);
        let files = workspace.project_files(&main);
        assert!(files.contains(&uri(root, "shared/refs_v1.bib")));
        assert!(files.contains(&uri(root, "shared/other.bib")));
        assert!(!files.contains(&uri(root, "shared/phantom.bib")));
        assert!(!workspace
            .documents
            .contains_key(&uri(root, "shared/phantom.bib")));
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 2, "{links:?}");
        let text = Text::new(source.clone());
        for (name, literal, key) in [
            ("refs_v1", r"../shared/re fs\_v1", "alpha"),
            ("other", "../shared/other", "beta"),
        ] {
            let target = uri(root, &format!("shared/{name}.bib"));
            let link = links
                .iter()
                .find(|link| link.target.as_ref() == Some(&target))
                .unwrap();
            assert_eq!(
                &source
                    [text.offset(link.range.start).unwrap()..text.offset(link.range.end).unwrap()],
                literal
            );
            let definitions = features::definitions(
                &workspace,
                &main,
                text.position(source.rfind(key).unwrap() + 2),
            );
            assert_eq!(definitions.len(), 1);
            assert_eq!(definitions[0].uri, target);
        }
        let changes = file_operations::will_rename(
            &workspace,
            &[rename(root, "shared/refs_v1.bib", "shared/renamed.bib")],
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert_eq!(
            apply(&source, &changes[&main]),
            source.replace(r"../shared/re fs\_v1", "../shared/renamed")
        );
        let before_escape = source.find(r"re fs\_v1").unwrap() + "re fs".len();
        assert!(
            !features::completion(&workspace, &main, text.position(before_escape))
                .iter()
                .any(|item| item.label == "../shared/refs_v1.bib")
        );
        assert!(file_operations::will_rename(
            &workspace,
            &[rename(root, "shared/phantom.bib", "shared/unused.bib")]
        )
        .unwrap()
        .is_none());
    }
}

#[test]
fn database_file_and_directory_renames_leave_comments_separators_and_raw_text_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("db")).unwrap();
    for name in ["refs", "other", "phantom"] {
        std::fs::write(
            root.join(format!("db/{name}.bib")),
            format!("@book{{{name},title={{Title}}}}"),
        )
        .unwrap();
    }
    for command in ["bibliography", "nobibliography"] {
        let source = format!("\\documentclass{{article}}\r\n\\{command}{{% head, db/phantom, }}\r\n db/refs% refs tail\r\n ,% split, }}\r\n db/other% other tail\r\n}}\r\n\\verb|\\{command}{{db/refs}}|\r\n");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = uri(root, "main.tex");
        let changes = file_operations::will_rename(
            &workspace,
            &[rename(root, "db/other.bib", "db/renamed.bib")],
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&main].len(), 1);
        assert_eq!(
            apply(&source, &changes[&main]),
            source.replace("\n db/other%", "\n db/renamed%")
        );
        let changes = file_operations::will_rename(&workspace, &[rename(root, "db", "papers")])
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&main].len(), 2);
        assert_eq!(
            apply(&source, &changes[&main]),
            source
                .replace("\n db/refs%", "\n papers/refs%")
                .replace("\n db/other%", "\n papers/other%")
        );
        assert!(file_operations::will_rename(
            &workspace,
            &[rename(root, "db/phantom.bib", "db/unused.bib")]
        )
        .unwrap()
        .is_none());
    }
}

#[test]
fn database_completion_preserves_surrounding_comments_and_ignores_their_commas() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("refs.bib"), "@book{alpha,title={Title}}").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for command in ["bibliography", "nobibliography"] {
        for (arguments, expected) in [
            (
                "{% before, }\n re¦BAD% tail\n,other}",
                "{% before, }\n refs% tail\n,other}",
            ),
            (
                "{other,% before, }\r\nre¦BAD% tail\r\n}",
                "{other,% before, }\r\nrefs% tail\r\n}",
            ),
            ("{% before\n¦% after\n}", "{% before\nrefs% after\n}"),
            ("{% before\nre¦", "{% before\nrefs"),
            ("{re ¦BAD}", "{refs}"),
        ] {
            let marked = format!("\\{command}{arguments}");
            let source = marked.replace('¦', "");
            workspace
                .documents
                .insert(main.clone(), Document::new(&main, source.clone(), Some(7)));
            let at = Text::new(source.clone()).position(marked.find('¦').unwrap());
            let item = features::completion(&workspace, &main, at)
                .into_iter()
                .find(|item| item.label == "refs.bib")
                .unwrap_or_else(|| panic!("{marked}"));
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!("missing edit");
            };
            let mut edits = item.additional_text_edits.unwrap_or_default();
            edits.push(edit);
            assert_eq!(
                apply(&source, &edits),
                format!("\\{command}{expected}"),
                "{marked}"
            );
        }
    }
}

#[test]
fn uncertain_database_arguments_do_not_advertise_partial_or_following_filenames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("refs.bib"), "@book{alpha,title={Title}}").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for command in ["bibliography", "nobibliography"] {
        for arguments in [
            r"*{re¦fs}",
            r"[Note]{re¦fs}",
            "{re¦% joined\nfs}",
            r"{re¦fs\suffix}",
            r"{{re¦fs}}",
            "{% comment re¦fs\n}",
        ] {
            let marked = format!("\\{command}{arguments}");
            let source = marked.replace('¦', "");
            std::fs::write(root.join("main.tex"), &source).unwrap();
            workspace.open(main.clone(), source.clone(), 7);
            let at = Text::new(source).position(marked.find('¦').unwrap());
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
                !features::completion(&workspace, &main, at)
                    .iter()
                    .any(|item| item.label == "refs.bib"),
                "{marked}"
            );
            assert!(
                file_operations::will_rename(
                    &workspace,
                    &[rename(root, "refs.bib", "renamed.bib")]
                )
                .unwrap()
                .is_none(),
                "{marked}"
            );
        }
    }
}
