use latex_lsp::{editing, features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:named-references.tex").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), source.into(), 5);
    (ws, uri)
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

#[test]
fn named_label_and_citation_keys_navigate_highlight_and_rename_with_deferred_bodies() {
    let source = "😀 \\label{label} \\bibitem{paper}\r\n\\newcommand{\\later}{\\csuse{ref}{label}\\csname cite\\endcsname{paper}}\r\n\\ref{label} \\cite{paper} \\csuse{crefrange}{label}{label} \\csuse{cites}{paper}{paper}";
    for key in ["label", "paper"] {
        let (mut ws, uri) = setup(source);
        let text = Text::new(source.into());
        let at = source.rfind(key).unwrap();
        let position = text.position(at + 1);
        let definitions = features::definitions(&ws, &uri, position);
        assert_eq!(definitions.len(), 1, "{key}: {definitions:?}");
        assert_eq!(definitions[0].range.start.line, 0);
        assert!(features::hover(&ws, &uri, position).is_some());
        let references = features::references(&ws, &uri, position, true);
        assert_eq!(references.len(), 5, "{key}: {references:?}");
        assert_eq!(editing::highlights(&ws, &uri, position).len(), 5);
        let prepared = features::prepare_rename(&ws, &uri, position)
            .unwrap()
            .unwrap();
        assert!(
            matches!(prepared, PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } if placeholder == key)
        );
        let replacement = format!("updated{key}");
        let edit = features::rename(&ws, &uri, position, &replacement)
            .unwrap()
            .unwrap();
        let edits = &edit.changes.as_ref().unwrap()[&uri];
        assert_eq!(edits.len(), 5, "{key}: {edits:?}");
        let updated = apply(source, edits.clone());
        assert_eq!(
            updated.matches(&format!("{{{key}}}")).count(),
            0,
            "{updated}"
        );
        assert_eq!(
            updated.matches(&format!("{{{replacement}}}")).count(),
            5,
            "{updated}"
        );
        ws.open(uri.clone(), updated, 6);
        assert!(!ws
            .diagnostics(&uri)
            .iter()
            .any(|diagnostic| diagnostic.message.starts_with("Undefined")
                && diagnostic.message.contains(&replacement)));
    }
}

#[test]
fn named_reference_data_and_operand_uses_block_partial_key_renames() {
    for context in [
        r"\newcommand{\discard}[1]{}\discard{\csuse{ref}{key}}",
        r"\noexpand\csuse{ref}{key}",
        r"\unknown{\csname ref\endcsname{key}}",
    ] {
        let source = format!("\\label{{key}}{context}");
        let (ws, uri) = setup(&source);
        let declaration = Text::new(source.clone()).position(source.find("key").unwrap() + 1);
        assert!(
            features::rename(&ws, &uri, declaration, "updated").is_err(),
            "{source}"
        );
        let call = Text::new(source.clone()).position(source.rfind("key").unwrap() + 1);
        assert!(
            features::definitions(&ws, &uri, call).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn proven_literal_named_reference_examples_keep_their_original_text() {
    let source = r"\label{key}\detokenize{\csuse{ref}{key}}";
    let (ws, uri) = setup(source);
    let edit = features::rename(&ws, &uri, Position::new(0, 8), "updated")
        .unwrap()
        .unwrap();
    let edits = &edit.changes.as_ref().unwrap()[&uri];
    assert_eq!(edits.len(), 1);
    assert!(apply(source, edits.clone()).ends_with(r"\detokenize{\csuse{ref}{key}}"));
    let at = Text::new(source.into()).position(source.rfind("key").unwrap() + 1);
    assert!(features::definitions(&ws, &uri, at).is_empty());
}

#[test]
fn redefined_named_interfaces_and_targets_keep_their_ordinary_data() {
    for definition in [r"\def\csuse#1#2{}", r"\renewcommand{\ref}[1]{#1}"] {
        let source = format!("{definition}\\label{{key}}\\csuse{{ref}}{{key}}");
        let (ws, uri) = setup(&source);
        let declaration = Text::new(source.clone()).position(source.find("key").unwrap() + 1);
        let edit = features::rename(&ws, &uri, declaration, "updated")
            .unwrap()
            .unwrap();
        let edits = &edit.changes.as_ref().unwrap()[&uri];
        assert_eq!(edits.len(), 1);
        assert!(apply(&source, edits.clone()).ends_with(r"\csuse{ref}{key}"));
    }
}

#[test]
fn shared_named_keys_require_consistent_meanings_in_every_consuming_project() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "a.tex",
            r"\documentclass{article}\input{common}\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\renewcommand{\ref}[1]{#1}\input{common}\input{shared}",
        ),
        ("common.tex", r"\label{key}"),
        ("shared.tex", r"\csuse{ref}{key}"),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let common = Url::from_file_path(dir.path().join("common.tex")).unwrap();
    let shared = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let position = Position::new(0, 8);
    assert!(features::rename(&ws, &common, position, "updated").is_err());
    assert!(features::definitions(&ws, &shared, Position::new(0, 13)).is_empty());
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    ws.open(
        b,
        r"\documentclass{article}\input{common}\input{shared}".into(),
        1,
    );
    let edit = features::rename(&ws, &common, position, "updated")
        .unwrap()
        .unwrap();
    let changes = edit.changes.unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[&common].len(), 1);
    assert_eq!(changes[&shared].len(), 1);
    ws.index_truncated.insert("maxFiles".into());
    assert!(features::rename(&ws, &common, position, "updated").is_err());
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn real_named_key_renames_preserve_references_emitted_from_macro_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut source = "\\documentclass{article}\n\\usepackage{etoolbox}\n\\newcommand{\\later}{\\csuse{ref}{key} \\csuse{cite}{paper}}\n\\begin{document}\n\\section{Title}\\label{key}\n\\later \\csname ref\\endcsname{key} \\csname cite\\endcsname{paper}\n\\begin{thebibliography}{9}\\bibitem{paper} Entry\\end{thebibliography}\n\\end{document}".to_owned();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    for name in ["key", "paper"] {
        ws.open(uri.clone(), source.clone(), 1);
        let at = source.rfind(name).unwrap();
        let position = Text::new(source.clone()).position(at + 1);
        let edit = features::rename(&ws, &uri, position, &format!("updated{name}"))
            .unwrap()
            .unwrap();
        source = apply(&source, edit.changes.unwrap().remove(&uri).unwrap());
    }
    std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
    for _ in 0..2 {
        let output = std::process::Command::new("pdflatex")
            .args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
}

#[test]
fn named_key_renames_keep_independent_project_definitions_and_semantics_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let source = r"\documentclass{article}\label{key}\csuse{ref}{key}";
    for name in ["a.tex", "b.tex"] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let a = Url::from_file_path(dir.path().join("a.tex")).unwrap();
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    ws.open(b, format!("{source}\\renewcommand{{\\ref}}[1]{{#1}}"), 1);
    let position = Text::new(source.into()).position(source.rfind("key").unwrap() + 1);
    let edit = features::rename(&ws, &a, position, "updated")
        .unwrap()
        .unwrap();
    let changes = edit.changes.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[&a].len(), 2);
}

#[test]
fn excessive_named_key_lists_reject_renames_instead_of_returning_partial_edits() {
    let source = format!(
        "\\label{{key}}\\csuse{{cref}}{{{}}}",
        vec!["key"; 20_001].join(",")
    );
    let (ws, uri) = setup(&source);
    let at = source.find("key").unwrap();
    let position = Text::new(source).position(at + 1);
    assert!(features::rename(&ws, &uri, position, "updated").is_err());
}
