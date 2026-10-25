use latex_lsp::{editing, language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;

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
fn command_completion_keeps_reference_and_alias_key_contexts() {
    for (tail, expected) in [
        (r"\tex", "textbf"),
        (r"\ref{ta", "target"),
        (r"\alias{ta", "target"),
        (r"\csuse{ref}{ta", "target"),
    ] {
        let source = format!("\\label{{target}}\\def\\alias{{\\ref}}\n{tail}");
        let (ws, uri) = setup(&source);
        let offset = source.len();
        let items = features::completion(&ws, &uri, Text::new(source).position(offset));
        assert!(
            items.iter().any(|item| item.label == expected),
            "{tail}: {items:?}"
        );
    }
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

#[test]
fn named_key_completions_preserve_notes_lists_suffixes_and_utf16_ranges() {
    let declarations = "😀 \\label{known}\\bibitem{paper}\r\n";
    for (marked, label, expected) in [
        (r"\csuse{ref}{kn¦OWN}", "known", r"\csuse{ref}{known}"),
        (
            r"\csname ref\endcsname{k¦nown}",
            "known",
            r"\csname ref\endcsname{known}",
        ),
        (
            r"\csuse{cref}{other,k¦nown,other}",
            "known",
            r"\csuse{cref}{other,known,other}",
        ),
        (
            r"\csuse{crefrange}{known}{k¦nown}",
            "known",
            r"\csuse{crefrange}{known}{known}",
        ),
        (
            r"\csuse{vref}[note]{k¦nown}",
            "known",
            r"\csuse{vref}[note]{known}",
        ),
        (
            r"\csuse{hyperref}[k¦nown]{text}",
            "known",
            r"\csuse{hyperref}[known]{text}",
        ),
        (
            r"\csuse{cite}[before][after]{p¦aper}",
            "paper",
            r"\csuse{cite}[before][after]{paper}",
        ),
        (
            r"\csname cites\endcsname[before]{paper}[after]{p¦aper}",
            "paper",
            r"\csname cites\endcsname[before]{paper}[after]{paper}",
        ),
        (r"\csuse{ref}{k¦", "known", r"\csuse{ref}{known"),
        (
            r"\newcommand{\later}{\csuse{ref}{k¦nown}}",
            "known",
            r"\newcommand{\later}{\csuse{ref}{known}}",
        ),
    ] {
        let marked = format!("{declarations}{marked}");
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (ws, uri) = setup(&source);
        let items = features::completion(&ws, &uri, Text::new(source.clone()).position(at));
        let item = items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("{item:?}")
        };
        let mut edits = vec![edit.clone()];
        edits.extend(item.additional_text_edits.clone().unwrap_or_default());
        assert_eq!(apply(&source, edits), format!("{declarations}{expected}"));
    }
}

#[test]
fn named_key_completion_excludes_command_names_notes_and_unconfirmed_meanings() {
    for marked in [
        r"\csuse{r¦ef}{known}",
        r"\csuse{ref}[k¦nown]{known}",
        r"\csuse{cite}[p¦aper]{paper}",
        r"\csuse{cites}(p¦aper){paper}",
        r"\newcommand{\discard}[1]{}\discard{\csuse{ref}{k¦nown}}",
        r"\noexpand\csuse{ref}{k¦nown}",
        r"\renewcommand{\ref}[1]{#1}\csuse{ref}{k¦nown}",
        r"\def\csuse#1#2{}\csuse{ref}{k¦nown}",
        r"\csuse{\dynamic}{k¦nown}",
        r"\detokenize{\csuse{ref}{k¦nown}}",
    ] {
        let marked = format!("\\label{{known}}\\bibitem{{paper}}\r\n{marked}");
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (ws, uri) = setup(&source);
        let items = features::completion(&ws, &uri, Text::new(source).position(at));
        assert!(
            !items
                .iter()
                .any(|item| item.label == "known" || item.label == "paper"),
            "{marked}: {items:?}"
        );
    }
}

#[test]
fn shared_named_completion_contexts_follow_unsaved_consumer_semantics() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "a.tex",
            r"\documentclass{article}\label{known}\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\renewcommand{\ref}[1]{#1}\label{known}\input{shared}",
        ),
        ("shared.tex", r"\csuse{ref}{kn}"),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let uri = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let position = Position::new(0, 14);
    assert!(!features::completion(&ws, &uri, position)
        .iter()
        .any(|item| item.label == "known"));
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    ws.open(
        b,
        r"\documentclass{article}\label{known}\input{shared}".into(),
        1,
    );
    assert!(features::completion(&ws, &uri, position)
        .iter()
        .any(|item| item.label == "known"));
    ws.index_truncated.insert("maxFiles".into());
    assert!(!features::completion(&ws, &uri, position)
        .iter()
        .any(|item| item.label == "known"));
}

#[test]
fn forwarding_keys_share_navigation_rename_and_macro_name_targets() {
    for (definition, call, key) in [
        (r"\def\myref{\ref}", r"\myref{label}", "label"),
        (
            r"\newcommand{\myref}{\ref}\def\next{\myref}",
            r"\next{label}",
            "label",
        ),
        (r"\def\myref{\ref}", r"\csuse{myref}{label}", "label"),
        (
            r"\def\myref{\ref}",
            r"\csname myref\endcsname{label}",
            "label",
        ),
        (
            r"\newcommand{\mycite}{\cite}",
            r"\mycite[before][after]{paper}",
            "paper",
        ),
        (
            r"\def\mycite{\cites}",
            r"\csuse{mycite}{paper}{paper}",
            "paper",
        ),
    ] {
        let source = format!("😀 \\label{{label}}\\bibitem{{paper}}\r\n{definition}\r\n{call}");
        let (ws, uri) = setup(&source);
        let position = Text::new(source.clone()).position(source.rfind(key).unwrap() + 1);
        assert_eq!(
            features::definitions(&ws, &uri, position).len(),
            1,
            "{source}"
        );
        assert!(features::hover(&ws, &uri, position).is_some(), "{source}");
        let count = if call.contains("{paper}{paper}") {
            3
        } else {
            2
        };
        assert_eq!(
            features::references(&ws, &uri, position, true).len(),
            count,
            "{source}"
        );
        assert_eq!(
            editing::highlights(&ws, &uri, position).len(),
            count,
            "{source}"
        );
        assert!(
            features::prepare_rename(&ws, &uri, position)
                .unwrap()
                .is_some(),
            "{source}"
        );
        let edit = features::rename(&ws, &uri, position, "updated")
            .unwrap()
            .unwrap();
        let edits = &edit.changes.as_ref().unwrap()[&uri];
        assert_eq!(edits.len(), count, "{source}");
        let updated = apply(&source, edits.clone());
        assert!(updated.contains(definition), "{updated}");
        assert!(!updated.contains(&format!("{{{key}}}")), "{updated}");
        if call.starts_with(r"\myref") {
            let command = Text::new(source.clone()).position(source.rfind(r"\myref").unwrap() + 3);
            let definitions = features::definitions(&ws, &uri, command);
            assert_eq!(definitions.len(), 1);
            assert_eq!(definitions[0].range.start.line, 1);
        }
    }
}

#[test]
fn forwarding_keys_publish_precise_missing_diagnostics_and_keep_query_commands_quiet() {
    for (definition, call, expected) in [
        (
            r"\def\myref{\ref}",
            r"\myref{missing}",
            vec![(false, "missing")],
        ),
        (
            r"\def\myref{\cref}",
            r"\myref{missing,other}",
            vec![(false, "missing"), (false, "other")],
        ),
        (
            r"\def\myref{\crefrange}",
            r"\myref{missing}{other}",
            vec![(false, "missing"), (false, "other")],
        ),
        (
            r"\def\myref{\hyperref}",
            r"\myref[missing]{Title}",
            vec![(false, "missing")],
        ),
        (
            r"\def\mycite{\cite}",
            r"\csuse{mycite}[note]{missing,other}",
            vec![(true, "missing"), (true, "other")],
        ),
        (
            r"\def\mycite{\cites}",
            r"\mycite[note]{missing}[note]{other}",
            vec![(true, "missing"), (true, "other")],
        ),
        (r"\def\myref{\getrefnumber}", r"\myref{missing}", vec![]),
    ] {
        let source = format!("{definition}\r\n😀 {call}");
        let (ws, uri) = setup(&source);
        let diagnostics: Vec<_> = ws
            .diagnostics(&uri)
            .into_iter()
            .filter(|d| d.message.starts_with("Undefined"))
            .collect();
        assert_eq!(
            diagnostics.len(),
            expected.len(),
            "{source}: {diagnostics:?}"
        );
        for (citation, key) in expected {
            let at = source.rfind(key).unwrap();
            let code = if citation {
                "undefined-citation"
            } else {
                "undefined-reference"
            };
            assert!(
                diagnostics.iter().any(|d| d.range
                    == Text::new(source.clone()).range(at, at + key.len())
                    && d.code == Some(NumberOrString::String(code.into()))),
                "{source}: {diagnostics:?}"
            );
        }
    }
}

#[test]
fn forwarding_key_completion_uses_leaf_grammar_and_replaces_the_complete_key() {
    for (definition, marked, label, expected) in [
        (
            r"\def\myref{\ref}",
            r"\myref{kn¦BAD}",
            "known",
            r"\myref{known}",
        ),
        (
            r"\def\myref{\crefrange}",
            r"\csuse{myref}{known}{kn¦BAD}",
            "known",
            r"\csuse{myref}{known}{known}",
        ),
        (
            r"\def\mycite{\cite}",
            r"\csname mycite\endcsname[note]{pa¦BAD}",
            "paper",
            r"\csname mycite\endcsname[note]{paper}",
        ),
        (
            r"\def\mycite{\cites}",
            r"\mycite{paper}[note]{pa¦",
            "paper",
            r"\mycite{paper}[note]{paper",
        ),
    ] {
        let prefix = format!("😀 \\label{{known}}\\bibitem{{paper}}\r\n{definition}\r\n");
        let at = prefix.len() + marked.find('¦').unwrap();
        let source = format!("{prefix}{}", marked.replace('¦', ""));
        let (ws, uri) = setup(&source);
        let items = features::completion(&ws, &uri, Text::new(source.clone()).position(at));
        let item = items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("{source}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("{item:?}")
        };
        let mut edits = vec![edit.clone()];
        edits.extend(item.additional_text_edits.clone().unwrap_or_default());
        assert_eq!(apply(&source, edits), format!("{prefix}{expected}"));
    }
}

#[test]
fn forwarding_key_semantics_reject_non_aliases_and_non_executable_data() {
    for (definition, call) in [
        (r"\def\myref#1{\ref{#1}}", r"\myref{missing}"),
        (r"\def\myref{\ref\relax}", r"\myref{missing}"),
        (r"\edef\myref{\ref}", r"\myref{missing}"),
        (r"\let\myref=\ref", r"\myref{missing}"),
        (r"\def\myref{\ref}\def\ref#1{}", r"\myref{missing}"),
        (r"\def\myref{\ref}\def\myref{}", r"\myref{missing}"),
        (r"\def\myref{\next}\def\next{\myref}", r"\myref{missing}"),
        (r"\def\myref{\ref}", r"\noexpand\myref{missing}"),
        (r"\def\myref{\ref}", r"\meaning\myref{missing}"),
        (r"\def\myref{\ref}", r"\detokenize{\myref{missing}}"),
        (r"\def\myref{\ref}\def\take#1{}", r"\take{\myref{missing}}"),
    ] {
        let source = format!("\\label{{known}}{definition}\r\n{call}");
        let (ws, uri) = setup(&source);
        assert!(
            !ws.diagnostics(&uri)
                .iter()
                .any(|d| d.message.starts_with("Undefined")),
            "{source}"
        );
        let at = source.rfind("missing").unwrap();
        let position = Text::new(source.clone()).position(at + 2);
        assert!(
            features::definitions(&ws, &uri, position).is_empty(),
            "{source}"
        );
        assert!(
            !features::completion(&ws, &uri, position)
                .iter()
                .any(|item| item.label == "known"),
            "{source}"
        );
    }
}

#[test]
fn forwarding_key_operands_block_partial_renames_but_literal_examples_are_preserved() {
    for call in [
        r"\noexpand\myref{key}",
        r"\def\take#1{}\take{\myref{key}}",
        r"\unknown{\myref{key}}",
    ] {
        let source = format!("\\label{{key}}\\def\\myref{{\\ref}}{call}");
        let (ws, uri) = setup(&source);
        assert!(
            features::rename(&ws, &uri, Position::new(0, 8), "updated").is_err(),
            "{source}"
        );
    }
    let source = r"\label{key}\def\myref{\ref}\detokenize{\myref{key}}";
    let (ws, uri) = setup(source);
    let edit = features::rename(&ws, &uri, Position::new(0, 8), "updated")
        .unwrap()
        .unwrap();
    assert_eq!(edit.changes.as_ref().unwrap()[&uri].len(), 1);
}

#[test]
fn shared_forwarding_keys_require_every_consumer_and_track_unsaved_alias_replacements() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "a.tex",
            r"\documentclass{article}\def\myref{\ref}\input{common}\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\def\myref#1{#1}\input{common}\input{shared}",
        ),
        ("common.tex", r"\label{known}"),
        ("shared.tex", r"\myref{known}"),
        (
            "independent.tex",
            r"\documentclass{article}\def\myref#1{}\label{known}",
        ),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let common = Url::from_file_path(dir.path().join("common.tex")).unwrap();
    let shared = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let key = Position::new(0, 9);
    assert!(features::definitions(&ws, &shared, key).is_empty());
    assert!(features::rename(&ws, &common, Position::new(0, 8), "updated").is_err());
    assert!(!features::completion(&ws, &shared, key)
        .iter()
        .any(|item| item.label == "known"));
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    ws.open(
        b,
        r"\documentclass{article}\def\myref{\ref}\input{common}\input{shared}".into(),
        2,
    );
    assert_eq!(features::definitions(&ws, &shared, key).len(), 1);
    assert!(features::completion(&ws, &shared, key)
        .iter()
        .any(|item| item.label == "known"));
    let changes = features::rename(&ws, &common, Position::new(0, 8), "updated")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[&shared].len(), 1);
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn real_forwarding_key_renames_preserve_resolved_references_and_citations() {
    let dir = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut source = "\\documentclass{article}\n\\usepackage{etoolbox}\n\\def\\myref{\\ref}\\newcommand{\\mycite}{\\cite}\\def\\nextref{\\myref}\n\\newcommand{\\later}{\\myref{key} \\mycite{paper}}\n\\begin{document}\n\\section{Title}\\label{key}\n\\later \\nextref{key} \\csname myref\\endcsname{key} \\csuse{mycite}{paper}\n\\begin{thebibliography}{9}\\bibitem{paper} Entry\\end{thebibliography}\n\\end{document}".to_owned();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    for key in ["key", "paper"] {
        ws.open(uri.clone(), source.clone(), 1);
        let position = Text::new(source.clone()).position(source.rfind(key).unwrap() + 1);
        let edit = features::rename(&ws, &uri, position, &format!("updated{key}"))
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
