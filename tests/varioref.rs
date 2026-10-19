use latex_lsp::{
    build_log, editing, features, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///varioref/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(3)));
    uri
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

#[test]
fn varioref_labels_navigate_and_rename_across_notes_stars_and_files() {
    let mut workspace = Workspace::default();
    let declaration = insert(
        &mut workspace,
        "chapter.tex",
        r"\label{sec:old}\label{ sec:old }",
    );
    let calls = [
        r"\vref{sec:old}",
        r"\vref[sec:old \ref{sec:old}]{sec:old}",
        "\\vref % star\r\n * [same]{sec:old}",
        r"\Vref{sec:old}",
        r"\Vref*[sec:old]{sec:old}",
        r"\vpageref{sec:old}",
        r"\vpageref*[sec:old][prefix]{sec:old}",
        r"\fullref{sec:old}",
    ];
    let valid = format!("😀 {}", calls.join("\n"));
    let ignored = "\n% \\Vref{sec:old}\n\\verb|\\vpageref{sec:old}|";
    let source = format!("{valid}{ignored}");
    let main = insert(&mut workspace, "main.tex", &source);
    let text = &workspace.documents[&main].text;
    for (at, _) in valid.match_indices("{sec:old}") {
        let position = text.position(at + 3);
        let definitions = features::definitions(&workspace, &main, position);
        assert_eq!(definitions.len(), 1, "{at}: {definitions:?}");
        assert_eq!(definitions[0].uri, declaration);
        assert_eq!(
            features::references(&workspace, &main, position, true).len(),
            10,
            "{at}"
        );
        let edit = features::rename(&workspace, &main, position, "sec:new")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            apply(&source, &edit[&main]),
            format!("{}{ignored}", valid.replace("{sec:old}", "{sec:new}"))
        );
        assert_eq!(
            apply(
                &workspace.documents[&declaration].text.source,
                &edit[&declaration]
            ),
            r"\label{sec:new}\label{ sec:old }"
        );
    }
}

#[test]
fn unsupported_reference_stars_and_excess_notes_are_not_rename_targets() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "chapter.tex", r"\label{sec:old}");
    for source in [
        r"\vref[first][second]{sec:old}",
        r"\Vref*[first][second]{sec:old}",
        r"\vpageref[first][second][third]{sec:old}",
        r"\vpageref**{sec:old}",
        r"\fullref*{sec:old}",
        r"\fullref[note]{sec:old}",
        r"\eqref*{sec:old}",
        r"\ref[note]{sec:old}",
        r"\getrefnumber*{sec:old}",
        r"\Vref{ sec:old }",
        r"\vpageref{\chosen}",
    ] {
        let main = insert(&mut workspace, "invalid.tex", source);
        let position = workspace.documents[&main]
            .text
            .position(source.rfind('{').unwrap() + 2);
        assert!(
            features::definitions(&workspace, &main, position).is_empty(),
            "{source}"
        );
        assert!(
            features::prepare_rename(&workspace, &main, position)
                .unwrap()
                .is_none(),
            "{source}"
        );
    }
}

#[test]
fn label_completion_skips_complete_varioref_notes_and_preserves_their_contents() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "chapter.tex", r"\label{sec:target}");
    for (marked, expected) in [
        (
            r"\vref[same \emph{page}]{sec:t¦BAD}",
            r"\vref[same \emph{page}]{sec:target}",
        ),
        (
            "\\Vref % star\n * [same]{sec:t¦BAD}",
            "\\Vref % star\n * [same]{sec:target}",
        ),
        (
            r"\vpageref*[{same [nested]}][other \emph{page}]{ sec:t¦BAD }",
            r"\vpageref*[{same [nested]}][other \emph{page}]{sec:target}",
        ),
        (r"\fullref{sec:t¦BAD}", r"\fullref{sec:target}"),
        (r"\vref[same]{¦}", r"\vref[same]{sec:target}"),
        (
            r"\vpageref[here][there]{sec:t¦",
            r"\vpageref[here][there]{sec:target",
        ),
    ] {
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let items = features::completion(&workspace, &main, Text::new(source.clone()).position(at));
        let item = items
            .iter()
            .find(|item| item.label == "sec:target")
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing label edit")
        };
        let mut edits = item.additional_text_edits.clone().unwrap_or_default();
        edits.push(edit.clone());
        assert_eq!(apply(&source, &edits), expected, "{marked}");
    }
}

#[test]
fn varioref_signatures_track_the_optional_notes_and_required_label() {
    for (marked, label, active) in [
        (r"\vref[sa¦me]{key}", r"\vref[same-page]{key}", 0),
        (r"\vref*{ke¦y}", r"\vref[same-page]{key}", 1),
        (r"\Vref*[same]{ke¦y}", r"\Vref[same-page]{key}", 1),
        (
            r"\vpageref[sa¦me][prefix]{key}",
            r"\vpageref[same-page][prefix]{key}",
            0,
        ),
        (
            r"\vpageref*[same][pre¦fix]{key}",
            r"\vpageref[same-page][prefix]{key}",
            1,
        ),
        (r"\vpageref{ke¦y}", r"\vpageref[same-page][prefix]{key}", 2),
        (r"\fullref{ke¦y}", r"\fullref{key}", 0),
    ] {
        let mut workspace = Workspace::default();
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let help = signature::help(&workspace, &main, Text::new(source).position(at))
            .unwrap_or_else(|| panic!("missing signature: {marked}"));
        assert_eq!(help.signatures[0].label, label, "{marked}");
        assert_eq!(help.active_parameter, Some(active), "{marked}");
    }
    for name in ["vref", "Vref", "vpageref", "fullref"] {
        let mut workspace = Workspace {
            snippets: true,
            ..Workspace::default()
        };
        let source = format!("\\{name}");
        let main = insert(&mut workspace, "main.tex", &source);
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(source.len()),
        );
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing command edit")
        };
        assert_eq!(edit.new_text, format!("{name}{{${{1:key}}}}"));
    }
}

#[test]
fn varioref_reference_warnings_have_precise_utf16_ranges_and_label_spelling_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for (call, prefix) in [
        (r"\vref[same]{sec:itnro}", "Reference"),
        (r"\Vref[same]{sec:itnro}", "Hyper reference"),
        (r"\vref[same]{sec:itnro}", "Hyper reference"),
        (r"\Vref*[same]{sec:itnro}", "Reference"),
        (r"\vpageref*[same][prefix]{sec:itnro}", "Reference"),
        (r"\fullref{sec:itnro}", "Reference"),
    ] {
        let source = format!("\\label{{sec:intro}}\n😀 {call}\n");
        std::fs::write(&main, &source).unwrap();
        let log = format!(
            "{}:2: LaTeX Warning: {prefix} `sec:itnro' on page 1 undefined on input line 2.\n",
            main.display()
        );
        let diagnostics = build_log::parse(&main, &log);
        let diagnostic = &diagnostics[&uri][0];
        let text = Text::new(source.clone());
        let at = source.rfind("sec:itnro").unwrap();
        assert_eq!(
            diagnostic.range,
            text.range(at, at + 9),
            "{call}: {diagnostic:?}"
        );
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Workspace::default()
        };
        workspace.open(uri.clone(), source, 3);
        workspace
            .build_diagnostics
            .insert(uri.clone(), diagnostics.clone());
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic.clone()],
                only: None,
                trigger_kind: None,
            },
        );
        assert_eq!(actions.len(), 1, "{call}: {actions:?}");
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!("missing quick fix")
        };
        let edit = &action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri][0];
        assert_eq!(edit.range, diagnostic.range);
        assert_eq!(edit.new_text, "sec:intro");
    }
}

#[test]
fn spelling_repairs_match_the_selected_warning_before_deduplicating_the_same_edit() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "\\label{sec:intro}\n\\vref[same]{sec:itnro}\n";
    std::fs::write(&main, source).unwrap();
    let log = ["Hyper reference", "Reference"]
        .map(|prefix| {
            format!(
                "{}:2: LaTeX Warning: {prefix} `sec:itnro' on page 1 undefined on input line 2.\n",
                main.display()
            )
        })
        .concat();
    let mut diagnostics = build_log::parse(&main, &log);
    assert_eq!(diagnostics[&uri].len(), 2);
    for reverse in [false, true] {
        if reverse {
            diagnostics.get_mut(&uri).unwrap().reverse();
        }
        let mut workspace = Workspace {
            code_action_literals: true,
            apply_edits: true,
            ..Default::default()
        };
        workspace.open(uri.clone(), source.into(), 3);
        workspace
            .build_diagnostics
            .insert(uri.clone(), diagnostics.clone());
        for diagnostic in &diagnostics[&uri] {
            let context = CodeActionContext {
                diagnostics: vec![diagnostic.clone()],
                only: None,
                trigger_kind: None,
            };
            let actions = editing::code_actions(&workspace, &uri, diagnostic.range, &context);
            assert_eq!(actions.len(), 1, "{reverse}: {diagnostic:?}");
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!("missing literal action")
            };
            assert_eq!(
                action.diagnostics.as_ref().unwrap(),
                &vec![diagnostic.clone()]
            );
            workspace.code_action_literals = false;
            let legacy = editing::code_actions(&workspace, &uri, diagnostic.range, &context);
            let CodeActionOrCommand::Command(command) = &legacy[0] else {
                panic!("missing legacy action")
            };
            let request =
                serde_json::from_value(command.arguments.as_ref().unwrap()[0].clone()).unwrap();
            assert!(editing::resolve_fix(&workspace, request).is_some());
            workspace.code_action_literals = true;
        }
        // With no specific warning selected, show a single replacement choice.
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostics[&uri][0].range,
            &CodeActionContext {
                diagnostics: vec![],
                only: None,
                trigger_kind: None,
            },
        );
        assert_eq!(actions.len(), 1);
    }
}
