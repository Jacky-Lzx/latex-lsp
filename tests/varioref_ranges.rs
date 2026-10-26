use lsp_types::*;
use texquill::{
    compilation::build_log,
    editing,
    language::{features, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///varioref-ranges/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(4)));
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
fn range_endpoints_navigate_and_rename_independently_across_files() {
    let mut workspace = Workspace::default();
    let declaration = insert(
        &mut workspace,
        "chapter.tex",
        r"\label{sec:first}\label{sec:last}",
    );
    let calls = [
        r"\vrefrange{sec:first}{sec:last}",
        r"\vrefrange[sec:last]{sec:first}{sec:last}",
        r"\vpagerefrange{sec:first}{sec:last}",
        r"\vpagerefrange*[sec:first]{sec:first}{sec:last}",
        r"\vrefrange[see [sec:last]{sec:first}{sec:last}",
        r"\vpagerefrange*[see [sec:first]{sec:first}{sec:last}",
        "\\vpagerefrange % star\r\n * [same]{sec:first}% between\r\n{sec:last}",
    ];
    let active = format!("😀 {}", calls.join("\n"));
    let ignored =
        "\n% \\vrefrange{sec:first}{sec:last}\n\\verb|\\vpagerefrange{sec:first}{sec:last}|\n\\vrefrange[see [nested]]{sec:first}{sec:last}\n\\vpagerefrange*[see [nested]]{sec:first}{sec:last}";
    let source = format!("{active}{ignored}");
    let main = insert(&mut workspace, "main.tex", &source);
    for key in ["sec:first", "sec:last"] {
        for (at, _) in active.match_indices(&format!("{{{key}}}")) {
            let position = workspace.documents[&main].text.position(at + 3);
            let definitions = features::definitions(&workspace, &main, position);
            assert_eq!(definitions.len(), 1, "{at}: {definitions:?}");
            assert_eq!(definitions[0].uri, declaration);
            assert_eq!(
                features::references(&workspace, &main, position, true).len(),
                calls.len() + 1
            );
            let edit = features::rename(&workspace, &main, position, "sec:renamed")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(
                apply(&source, &edit[&main]),
                format!(
                    "{}{ignored}",
                    active.replace(&format!("{{{key}}}"), "{sec:renamed}")
                )
            );
            assert_eq!(
                apply(
                    &workspace.documents[&declaration].text.source,
                    &edit[&declaration]
                ),
                workspace.documents[&declaration]
                    .text
                    .source
                    .replace(key, "sec:renamed")
            );
        }
    }
}

#[test]
fn invalid_range_flags_and_notes_do_not_create_label_targets() {
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "chapter.tex",
        r"\label{sec:first}\label{sec:last}",
    );
    for source in [
        r"\vrefrange*{sec:first}{sec:last}",
        r"\vrefrange[first][second]{sec:first}{sec:last}",
        r"\vpagerefrange**{sec:first}{sec:last}",
        r"\vpagerefrange*[first][second]{sec:first}{sec:last}",
        r"\vrefrange[see [nested]]{sec:first}{sec:last}",
        r"\vpagerefrange*[see [nested]]{sec:first}{sec:last}",
    ] {
        let main = insert(&mut workspace, "invalid.tex", source);
        for key in ["sec:first", "sec:last"] {
            let position = workspace.documents[&main]
                .text
                .position(source.rfind(key).unwrap() + 2);
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
            assert!(
                signature::help(&workspace, &main, position).is_none(),
                "{source}"
            );
            assert!(
                !features::completion(&workspace, &main, position)
                    .iter()
                    .any(|item| item.label == key),
                "{source}"
            );
        }
    }
    for source in [
        r"\vrefrange{sec:first}{ sec:last }",
        r"\vpagerefrange{sec:first}{\chosen}",
    ] {
        let main = insert(&mut workspace, "invalid-key.tex", source);
        let position = workspace.documents[&main]
            .text
            .position(source.rfind('{').unwrap() + 3);
        assert!(
            features::prepare_rename(&workspace, &main, position)
                .unwrap()
                .is_none(),
            "{source}"
        );
    }
}

#[test]
fn range_completion_replaces_only_the_active_endpoint_including_incomplete_calls() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "chapter.tex", r"\label{sec:target}");
    for (marked, expected) in [
        (
            r"\vrefrange{sec:t¦BAD}{last}",
            r"\vrefrange{sec:target}{last}",
        ),
        (
            r"\vrefrange{first}{sec:t¦BAD}",
            r"\vrefrange{first}{sec:target}",
        ),
        (
            r"\vrefrange[same \ref{ignored}]{first}{ sec:t¦BAD }",
            r"\vrefrange[same \ref{ignored}]{first}{sec:target}",
        ),
        (
            "\\vpagerefrange % star\n * [same]{first}% between\n{sec:t¦BAD}",
            "\\vpagerefrange % star\n * [same]{first}% between\n{sec:target}",
        ),
        (
            r"\vpagerefrange*{first}{¦}",
            r"\vpagerefrange*{first}{sec:target}",
        ),
        (
            r"\vpagerefrange[same]{first}{sec:t¦",
            r"\vpagerefrange[same]{first}{sec:target",
        ),
        (r"\vrefrange{¦", r"\vrefrange{sec:target"),
        (
            r"\vrefrange[see [literal]{sec:t¦BAD}{last}",
            r"\vrefrange[see [literal]{sec:target}{last}",
        ),
        (
            r"\vpagerefrange*[see [literal]{first}{sec:t¦",
            r"\vpagerefrange*[see [literal]{first}{sec:target",
        ),
        (
            "\\vrefrange{first}{\n sec:t¦BAD\n }",
            r"\vrefrange{first}{sec:target}",
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
            panic!("missing edit")
        };
        let mut edits = item.additional_text_edits.clone().unwrap_or_default();
        edits.push(edit.clone());
        assert_eq!(apply(&source, &edits), expected, "{marked}");
    }
}

#[test]
fn range_signatures_and_snippets_expose_both_required_labels() {
    for name in ["vrefrange", "vpagerefrange"] {
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                snippets: true,
                ..Default::default()
            },
            ..Workspace::default()
        };
        for (arguments, active) in [
            ("[sa¦me]{first}{last}", 0),
            ("{fir¦st}{last}", 1),
            ("[same]{first}{la¦st}", 2),
            ("[see [li¦teral]{first}{last}", 0),
            ("[see [literal]{fir¦st}{last}", 1),
            ("[see [literal]{first}{la¦st}", 2),
        ] {
            let marked = format!("\\{name}{arguments}");
            let at = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            let main = insert(&mut workspace, "main.tex", &source);
            let help = signature::help(&workspace, &main, Text::new(source).position(at)).unwrap();
            assert_eq!(
                help.signatures[0].label,
                format!("\\{name}[same-page]{{first}}{{last}}")
            );
            assert_eq!(help.active_parameter, Some(active));
        }
        let source = format!("\\{name}");
        let main = insert(&mut workspace, "main.tex", &source);
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(source.len()),
        );
        let item = items.iter().find(|item| item.label == name).unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing snippet")
        };
        assert_eq!(
            edit.new_text,
            format!("{name}{{${{1:first}}}}{{${{2:last}}}}")
        );
    }
}

#[test]
fn multiline_range_warnings_select_and_repair_each_endpoint_at_the_final_input_line() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for command in ["vrefrange", "vpagerefrange", "vpagerefrange*"] {
        let source = format!("\\label{{sec:first}}\\label{{sec:last}}\n😀 \\{command}[same]{{sec:frist}}% note\n{{sec:lsat}} \\{command}[see [nested]]{{sec:first}}{{sec:lsat}}\n");
        std::fs::write(&main, &source).unwrap();
        let log = ["sec:frist", "sec:lsat"]
            .map(|key| {
                format!(
                    "{}:3: LaTeX Warning: Reference `{key}' on page 1 undefined on input line 3.\n",
                    main.display()
                )
            })
            .concat();
        let diagnostics = build_log::parse(&main, &log);
        assert_eq!(diagnostics[&uri].len(), 2);
        let text = Text::new(source.clone());
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                code_action_literals: true,
                ..Default::default()
            },
            ..Workspace::default()
        };
        workspace.open(uri.clone(), source.clone(), 4);
        workspace
            .project
            .build_diagnostics
            .insert(uri.clone(), diagnostics.clone());
        let mut edits = Vec::new();
        for (key, correct) in [("sec:frist", "sec:first"), ("sec:lsat", "sec:last")] {
            let diagnostic = diagnostics[&uri]
                .iter()
                .find(|diagnostic| diagnostic.message.contains(key))
                .unwrap();
            let at = source.find(key).unwrap();
            assert_eq!(
                diagnostic.range,
                text.range(at, at + key.len()),
                "{command}: {diagnostic:?}"
            );
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
            let edit = actions
                .iter()
                .find_map(|action| match action {
                    CodeActionOrCommand::CodeAction(action) => {
                        action.edit.as_ref()?.changes.as_ref()?[&uri]
                            .iter()
                            .find(|edit| edit.new_text == correct)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{command}: {actions:?}"));
            assert_eq!(edit.range, diagnostic.range);
            edits.push(edit.clone());
        }
        assert_eq!(
            apply(&source, &edits),
            source
                .replacen("sec:frist", "sec:first", 1)
                .replacen("sec:lsat", "sec:last", 1)
        );
    }
}

#[test]
fn identical_endpoints_are_distinct_references_and_warning_ranges() {
    let mut workspace = Workspace::default();
    let source = "\\label{sec:target}\n\\vrefrange{sec:target}\n{sec:target}\n";
    let main = insert(&mut workspace, "main.tex", source);
    let position = workspace.documents[&main]
        .text
        .position(source.rfind("sec:target").unwrap() + 2);
    let references = features::references(&workspace, &main, position, true);
    assert_eq!(references.len(), 3);
    assert_ne!(references[1].range, references[2].range);
    let edit = features::rename(&workspace, &main, position, "sec:new")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        apply(source, &edit[&main]),
        source.replace("sec:target", "sec:new")
    );

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.tex");
    std::fs::write(&path, source).unwrap();
    let diagnostics = build_log::parse(
        &path,
        &format!(
            "{}:3: LaTeX Warning: Reference `sec:target' on page 1 undefined on input line 3.\n",
            path.display()
        ),
    );
    let uri = Url::from_file_path(&path).unwrap();
    assert_eq!(diagnostics[&uri].len(), 2);
    let mut expected = references
        .iter()
        .filter(|location| location.range.start.line != 0)
        .map(|location| location.range)
        .collect::<Vec<_>>();
    expected.sort_by_key(|range| range.start);
    assert_eq!(
        diagnostics[&uri]
            .iter()
            .map(|diagnostic| diagnostic.range)
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn semantic_tokens_classify_both_static_range_endpoints_without_coloring_notes() {
    let mut workspace = Workspace::default();
    let source = "😀 \\vrefrange[note]{sec:first}{sec:last}\n\\vpagerefrange*[note]{sec:first}{sec:last}\n\\vrefrange{first}{ spaced }\n\\vrefrange*{invalid}{invalid}\n";
    let main = insert(&mut workspace, "main.tex", source);
    let doc = &workspace.documents[&main];
    let mut position = Position::default();
    let mut keys = Vec::new();
    let legend = semantic::options().legend;
    for token in semantic::tokens(doc, None).unwrap().data {
        position = Position::new(
            position.line + token.delta_line,
            if token.delta_line == 0 {
                position.character + token.delta_start
            } else {
                token.delta_start
            },
        );
        if legend.token_types[token.token_type as usize].as_str() == "enumMember" {
            let start = doc.text.offset(position).unwrap();
            let end = doc
                .text
                .offset(Position::new(
                    position.line,
                    position.character + token.length,
                ))
                .unwrap();
            keys.push(&source[start..end]);
        }
    }
    assert_eq!(
        keys,
        ["sec:first", "sec:last", "sec:first", "sec:last", "first"]
    );
}
