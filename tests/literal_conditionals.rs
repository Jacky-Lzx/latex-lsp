use latex_lsp::{
    compilation::build, language::features, project::workspace::Workspace, text::Text,
};
use lsp_types::*;
use std::{path::Path, sync::atomic::AtomicBool};

fn setup(root: &Path, source: &str) -> (Workspace, Url) {
    let path = root.join("main.tex");
    std::fs::write(&path, source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    (workspace, Url::from_file_path(path).unwrap())
}

fn renamed(workspace: &Workspace, uri: &Url, position: Position, new_name: &str) -> String {
    let edit = features::rename(workspace, uri, position, new_name)
        .unwrap()
        .unwrap();
    let mut edits = edit.changes.unwrap()[uri].clone();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[uri]
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

#[test]
fn expanded_literal_conditionals_index_unicode_names_and_rename_every_derived_spelling() {
    for name in ["draft", "draft-mode", "模式"] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("\\documentclass{{article}}\n\\expandafter% declaration\n\\newif% target\n\\csname if{name}\\endcsname\n\\begin{{document}}😀 \\csname {name}true\\endcsname\\csname if{name}\\endcsname Yes.\\else No.\\fi\n\\csname {name}false\\endcsname\n\\end{{document}}\n");
        let (workspace, uri) = setup(directory.path(), &source);
        assert_eq!(
            workspace.documents[&uri].syntax.conditionals.len(),
            1,
            "{name}"
        );
        assert!(workspace.documents[&uri]
            .syntax
            .definitions
            .iter()
            .all(|entry| entry.name != "csname"));
        for (marker, local) in [
            (format!("if{name}\\endcsname"), 1),
            (format!("{name}true\\endcsname"), name.len() + 2),
        ] {
            let text = &workspace.documents[&uri].text;
            let position = text.position(source.find(&marker).unwrap() + local);
            assert_eq!(features::definitions(&workspace, &uri, position).len(), 1);
            assert_eq!(
                features::references(&workspace, &uri, position, true).len(),
                4
            );
            assert_eq!(
                features::references(&workspace, &uri, position, false).len(),
                3
            );
            let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
                features::prepare_rename(&workspace, &uri, position).unwrap()
            else {
                panic!("{name}")
            };
            assert_eq!(placeholder, name);
            assert_eq!(
                &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
                name
            );
            assert_eq!(
                renamed(&workspace, &uri, position, "发布-模式"),
                source.replace(name, "发布-模式")
            );
            assert!(features::rename(&workspace, &uri, position, "true").is_err());
        }
    }
}

#[test]
fn literal_conditional_completions_skip_nonletters_and_computed_or_unexpanded_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let source = "\\expandafter\\newif\\csname ifdraft\\endcsname\n\\expandafter\\newif\\csname ifdraft-mode\\endcsname\n\\expandafter\\newif\\csname if模式\\endcsname\n\\newif\\csname ifunexpanded\\endcsname\n\\expandafter\\newif\\csname ifcomputed\\suffix\\endcsname\n\\expandafter\\newif\\csname unrelated\\endcsname\n\\drafttrue\n";
    let (workspace, uri) = setup(directory.path(), source);
    assert_eq!(workspace.documents[&uri].syntax.conditionals.len(), 3);
    let position = Text::new(source.into()).position(source.find("\\drafttrue").unwrap() + 1);
    let labels = features::completion(&workspace, &uri, position)
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
    for name in ["ifdraft", "drafttrue", "draftfalse"] {
        assert!(
            labels.iter().any(|label| label == name),
            "{name}: {labels:?}"
        );
    }
    for name in [
        "ifdraft-mode",
        "draft-modetrue",
        "if模式",
        "模式true",
        "ifunexpanded",
        "ifcomputed",
        "ifrelated",
    ] {
        assert!(
            !labels.iter().any(|label| label == name),
            "{name}: {labels:?}"
        );
    }
}

#[test]
fn literal_conditional_names_used_directly_require_valid_control_word_replacements() {
    let directory = tempfile::tempdir().unwrap();
    let source = "\\expandafter\\newif\\csname ifdraft\\endcsname\n\\drafttrue\\ifdraft Yes.\\fi\n";
    let (workspace, uri) = setup(directory.path(), source);
    let position = Text::new(source.into()).position(source.find("ifdraft").unwrap() + 2);
    assert!(features::rename(&workspace, &uri, position, "发布-模式").is_err());
    assert_eq!(
        renamed(&workspace, &uri, position, "release"),
        source.replace("draft", "release")
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn real_literal_conditional_renames_preserve_values_with_unicode_and_hyphen_names() {
    for name in ["draft", "draft-mode", "模式"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\documentclass{{article}}\n\\expandafter\\newif\\csname if{name}\\endcsname\n\\begin{{document}}\n\\csname {name}true\\endcsname\\csname if{name}\\endcsname\\typeout{{LITERAL-SWITCH:true}}\\fi\n\\csname {name}false\\endcsname\\csname if{name}\\endcsname Wrong.\\else\\typeout{{LITERAL-SWITCH:false}}\\fi Text.\n\\end{{document}}\n");
        let (workspace, uri) = setup(root, &source);
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let before = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(before.status, "success", "{}", before.output);
        let position =
            Text::new(source.clone()).position(source.find(&format!("if{name}")).unwrap() + 2);
        let changed = renamed(&workspace, &uri, position, "发布-模式");
        assert_eq!(changed, source.replace(name, "发布-模式"));
        std::fs::write(root.join("main.tex"), changed).unwrap();
        let after = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(after.status, "success", "{}", after.output);
        assert_eq!(after.pages, Some(1));
        for state in ["LITERAL-SWITCH:true", "LITERAL-SWITCH:false"] {
            assert!(before.output.contains(state));
            assert!(after.output.contains(state));
        }
    }
}
