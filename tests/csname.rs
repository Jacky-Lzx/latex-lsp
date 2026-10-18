use latex_lsp::{build, editing, features, workspace::Workspace};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn renamed(workspace: &Workspace, document: &Url, position: Position) -> BTreeMap<Url, String> {
    features::rename(workspace, document, position, "greeting")
        .unwrap()
        .unwrap()
        .changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let source = workspace.documents[&uri]
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
                .source;
            (uri, source)
        })
        .collect()
}

#[test]
fn literal_macro_names_support_navigation_prepared_rename_references_and_highlights() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newcommand{\\hello}[1]{Hello #1}\n";
    let source = "\\documentclass{article}\r\n\\input{definitions}\r\n\\begin{document}\r\n😀 \\csname hello\\endcsname{World}\r\n\\hello{Direct}\\csname  hello  \\endcsname{Other}\r\n% \\csname hello\\endcsname\r\n\\verb|\\csname hello\\endcsname|\r\n\\end{document}\r\n";
    let workspace = project(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = uri(root, "main.tex");
    let text = &workspace.documents[&main].text;
    for start in [
        source.find("hello").unwrap(),
        source.find("\\hello").unwrap() + 2,
        source.find("  hello").unwrap() + 2,
    ] {
        let position = text.position(start + 2);
        let locations = features::definitions(&workspace, &main, position);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].uri, uri(root, "definitions.tex"));
        assert!(features::hover(&workspace, &main, position).is_some());
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, &main, position).unwrap()
        else {
            panic!("missing prepared rename")
        };
        assert_eq!(placeholder, "hello");
        assert_eq!(
            &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
            "hello"
        );
        assert_eq!(
            features::references(&workspace, &main, position, false).len(),
            3
        );
        assert_eq!(
            features::references(&workspace, &main, position, true).len(),
            4
        );
        let highlights = editing::highlights(&workspace, &main, position);
        assert_eq!(highlights.len(), 3);
        assert!(highlights
            .iter()
            .all(|entry| entry.kind == Some(DocumentHighlightKind::READ)));
        let changed = renamed(&workspace, &main, position);
        assert_eq!(
            changed[&main],
            source
                .replace("😀 \\csname hello", "😀 \\csname greeting")
                .replace(
                    "\\hello{Direct}\\csname  hello",
                    "\\greeting{Direct}\\csname  greeting"
                )
        );
        assert_eq!(
            changed[&uri(root, "definitions.tex")],
            definitions.replace("hello", "greeting")
        );
    }
}

#[test]
fn computed_commented_and_unterminated_names_are_not_guessed_or_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\newcommand{\\hello}{}\n\\csname hello world\\endcsname\n\\csname hello%comment\n\\endcsname\n\\csname \\prefix hello\\endcsname\n\\csname hello";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let text = &workspace.documents[&main].text;
    let position = text.position(source.find("hello").unwrap());
    let changed = renamed(&workspace, &main, position);
    assert_eq!(changed[&main], source.replacen("\\hello", "\\greeting", 1));
    for (offset, _) in source.match_indices("hello").skip(1) {
        assert!(
            features::prepare_rename(&workspace, &main, text.position(offset + 2))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn shared_literal_calls_to_different_macro_definitions_are_ambiguous() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root, &[
        ("a.tex", "\\documentclass{article}\n\\newcommand{\\hello}{A}\n\\begin{document}\\input{shared}\\end{document}"),
        ("b.tex", "\\documentclass{article}\n\\newcommand{\\hello}{B}\n\\begin{document}\\input{shared}\\end{document}"),
        ("shared.tex", "\\csname hello\\endcsname"),
    ]);
    for file in ["a.tex", "shared.tex"] {
        let document = uri(root, file);
        let text = &workspace.documents[&document].text;
        let position = text.position(text.source.find("hello").unwrap() + 2);
        assert!(features::prepare_rename(&workspace, &document, position).is_err());
        assert!(features::rename(&workspace, &document, position, "greeting").is_err());
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn renamed_literal_macro_calls_keep_control_word_syntax_and_compile() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\csname hello\\endcsname{World}\n\\hello{Direct}\n\\end{document}\n";
    let workspace = project(
        root,
        &[
            ("main.tex", source),
            ("definitions.tex", "\\newcommand{\\hello}[1]{Hello #1}\n"),
        ],
    );
    let main = uri(root, "main.tex");
    let position = workspace.documents[&main]
        .text
        .position(source.find("hello").unwrap() + 2);
    for (uri, source) in renamed(&workspace, &main, position) {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    assert_eq!(outcome.pages, Some(1));
}
