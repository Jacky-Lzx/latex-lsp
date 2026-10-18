use latex_lsp::{build, features, text::Text, workspace::Workspace};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};
use tower_lsp::lsp_types::*;

fn setup(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn renamed(workspace: &Workspace, document: &Url, position: Position) -> BTreeMap<Url, String> {
    features::rename(workspace, document, position, "release")
        .unwrap()
        .unwrap()
        .changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let changed = workspace.documents[&uri]
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
                .unwrap();
            (uri, changed.source)
        })
        .collect()
}

#[test]
fn conditionals_navigate_rename_and_complete_predicates_and_both_setters() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newif\\ifdraft\n\\draftfalse\n\\newcommand{\\draft}{Independent.}\n";
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n😀 \\drafttrue\\ifdraft Yes.\\else No.\\fi\n\\csname draftfalse\\endcsname\\csname ifdraft\\endcsname No.\\else Yes.\\fi\n\\draft\n% \\drafttrue\n\\verb|\\ifdraft|\n\\end{document}\n";
    let workspace = setup(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let declaration = Url::from_file_path(root.join("definitions.tex")).unwrap();
    for (uri, marker, local) in [
        (&declaration, "\\ifdraft", 2),
        (&declaration, "\\draftfalse", 8),
        (&main, "\\drafttrue", 7),
        (&main, "\\ifdraft", 2),
        (&main, "draftfalse\\endcsname", 6),
        (&main, "ifdraft\\endcsname", 1),
    ] {
        let text = &workspace.documents[uri].text;
        let position = text.position(text.source.find(marker).unwrap() + local);
        let targets = features::definitions(&workspace, uri, position);
        assert_eq!(targets.len(), 1, "{marker}: {targets:?}");
        assert_eq!(targets[0].uri, declaration);
        assert!(features::hover(&workspace, uri, position).is_some());
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, uri, position).unwrap()
        else {
            panic!("{marker}")
        };
        assert_eq!(placeholder, "draft");
        assert_eq!(
            &text.source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
            "draft"
        );
        let references = features::references(&workspace, uri, position, true);
        assert_eq!(references.len(), 6, "{marker}: {references:?}");
        assert_eq!(
            features::references(&workspace, uri, position, false).len(),
            5
        );
        let changed = renamed(&workspace, uri, position);
        assert_eq!(
            changed[&declaration],
            definitions
                .replace("\\ifdraft", "\\ifrelease")
                .replace("\\draftfalse", "\\releasefalse")
        );
        assert_eq!(
            changed[&main],
            source
                .replace("drafttrue", "releasetrue")
                .replace("ifdraft", "ifrelease")
                .replace("draftfalse", "releasefalse")
                .replace("% \\releasetrue", "% \\drafttrue")
                .replace("\\verb|\\ifrelease|", "\\verb|\\ifdraft|")
        );
    }
    for (marker, expected) in [("\\drafttrue", "drafttrue"), ("\\ifdraft", "ifdraft")] {
        let position = workspace.documents[&main]
            .text
            .position(source.find(marker).unwrap() + marker.len() - 1);
        let items = features::completion(&workspace, &main, position);
        assert!(items.iter().any(|item| item.label == expected), "{items:?}");
    }
    assert!(features::symbols(&workspace.documents[&declaration])
        .iter()
        .any(|symbol| symbol.name == "draft" && symbol.kind == SymbolKind::BOOLEAN));
}

#[test]
fn conditional_renames_reject_derived_command_collisions_builtins_and_invalid_control_words() {
    for extra in [
        r"\newif\ifrelease",
        r"\newcommand{\releasetrue}{}",
        r"\newlength{\releasefalse}",
        r"\newenvironment{ifrelease}{}{}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("\\newif\\ifdraft\n{extra}\n");
        let workspace = setup(root, &[("main.tex", &source)]);
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let position = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
        for name in ["release", "true", "false", "num", "release-two"] {
            assert!(
                features::rename(&workspace, &uri, position, name).is_err(),
                "{extra}: {name}"
            );
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\newif\\ifdraft\n\\newcommand{\\oldmacro}{}\\newenvironment{oldenv}{}{}\\newcounter{oldcounter}\n";
    let workspace = setup(root, &[("main.tex", source)]);
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    for old in ["oldmacro", "oldenv"] {
        let position = Text::new(source.into()).position(source.find(old).unwrap() + 1);
        assert!(
            features::rename(&workspace, &uri, position, "drafttrue").is_err(),
            "{old}"
        );
    }
}

#[test]
fn shared_conditional_occurrences_cannot_rename_another_projects_switch() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = setup(root, &[
        ("a.tex", "\\documentclass{article}\\newif\\ifdraft\\begin{document}\\input{shared}\\end{document}"),
        ("b.tex", "\\documentclass{article}\\newif\\ifdraft\\begin{document}\\input{shared}\\end{document}"),
        ("shared.tex", "\\drafttrue\\ifdraft Shared.\\fi"),
    ]);
    let uri = Url::from_file_path(root.join("a.tex")).unwrap();
    let text = &workspace.documents[&uri].text;
    let position = text.position(text.source.find("draft").unwrap());
    assert!(features::rename(&workspace, &uri, position, "release").is_err());
}

#[test]
fn conditional_completions_follow_projects_and_skip_computed_or_ignored_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\n\\newif% declaration\n\\ifdraft\n% \\newif\\ifcomment\n\\verb|\\newif\\ifverbatim|\n\\newif\\other\n\\newif\\csname ifcomputed\\endcsname\n\\begin{document}\n\\drafttrue\\ifdraft Text.\\fi\n\\end{document}\n";
    let workspace = setup(
        root,
        &[
            ("a.tex", source),
            (
                "b.tex",
                "\\documentclass{article}\\newif\\ifdifferent\\begin{document}Text.\\end{document}",
            ),
        ],
    );
    let uri = Url::from_file_path(root.join("a.tex")).unwrap();
    assert_eq!(workspace.documents[&uri].syntax.conditionals.len(), 1);
    let text = &workspace.documents[&uri].text;
    let position = text.position(source.find("\\drafttrue").unwrap() + 1);
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
        "ifdifferent",
        "differenttrue",
        "ifcomment",
        "ifverbatim",
        "ifcomputed",
    ] {
        assert!(!labels.iter().any(|label| label == name), "{name}");
    }
    let changed = renamed(&workspace, &uri, position);
    assert_eq!(changed.len(), 1);
    assert!(changed[&uri].contains("\\newif% declaration\n\\ifrelease"));
}

#[test]
#[ignore = "requires pdflatex"]
fn real_conditional_renames_preserve_true_false_and_local_group_behavior() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newif\\ifdraft\n\\draftfalse\n";
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\ifdraft Wrong.\\else\\typeout{SWITCH:initial-false}\\fi\n\\drafttrue\\ifdraft\\typeout{SWITCH:true}\\fi\n{\\draftfalse\\ifdraft Wrong.\\else\\typeout{SWITCH:local-false}\\fi}\n\\ifdraft\\typeout{SWITCH:restored-true}\\fi\n\\csname draftfalse\\endcsname\\csname ifdraft\\endcsname Wrong.\\else\\typeout{SWITCH:csname-false}\\fi\nText.\\end{document}\n";
    let workspace = setup(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = root.join("main.tex");
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let first = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(first.status, "success", "{}", first.output);
    let uri = Url::from_file_path(root.join("definitions.tex")).unwrap();
    let position = Text::new(definitions.into()).position(definitions.find("draft").unwrap());
    for (uri, changed) in renamed(&workspace, &uri, position) {
        std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
    }
    let second = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(second.status, "success", "{}", second.output);
    assert_eq!(second.pages, Some(1));
    let states = |output: &str| {
        output
            .lines()
            .filter(|line| line.starts_with("SWITCH:"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(states(&first.output), states(&second.output));
    assert_eq!(states(&second.output).len(), 5);
}
