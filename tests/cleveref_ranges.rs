use lsp_types::*;
use texquill::{
    compilation::build_log,
    editing,
    language::{features, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};

const COMMANDS: &[&str] = &["crefrange", "Crefrange", "cpagerefrange", "Cpagerefrange"];

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///cleveref-ranges/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
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
fn cleveref_ranges_rename_both_endpoints_independently_and_preserve_nonreferences() {
    for name in COMMANDS {
        let mut workspace = Workspace::default();
        let declarations = r"\label{sec:first}\label{sec:last}";
        let labels = insert(&mut workspace, "labels.tex", declarations);
        let active = format!("😀 \\{name}{{sec:first}}{{sec:last}}\r\n\\{name} % star\r\n * {{sec:first}}% between\r\n{{sec:last}}\r\n");
        let ignored = format!("% \\{name}{{sec:first}}{{sec:last}}\r\n\\verb|\\{name}{{sec:first}}{{sec:last}}|\r\n\\{name}[sec:first]{{sec:first}}{{sec:last}}\r\n");
        let source = format!("{active}{ignored}");
        let main = insert(&mut workspace, "main.tex", &source);
        for key in ["sec:first", "sec:last"] {
            for (offset, _) in active.match_indices(key) {
                let position = workspace.documents[&main].text.position(offset + 2);
                let definitions = features::definitions(&workspace, &main, position);
                assert_eq!(definitions.len(), 1, "{name}: {key}");
                assert_eq!(definitions[0].uri, labels);
                assert!(features::hover(&workspace, &main, position).is_some());
                assert_eq!(
                    features::references(&workspace, &main, position, false).len(),
                    2
                );
                let changes = features::rename(&workspace, &main, position, "章节😀:new")
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(
                    apply(&source, &changes[&main]),
                    format!("{}{ignored}", active.replace(key, "章节😀:new"))
                );
                assert_eq!(
                    apply(declarations, &changes[&labels]),
                    declarations.replace(key, "章节😀:new")
                );
                assert!(features::rename(
                    &workspace,
                    &main,
                    position,
                    if key == "sec:first" {
                        "sec:last"
                    } else {
                        "sec:first"
                    }
                )
                .is_err());
            }
        }
    }
}

#[test]
fn range_completions_replace_only_the_active_endpoint_even_across_lines() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "labels.tex", r"\label{sec:target}");
    for name in COMMANDS {
        for (arguments, expected) in [
            ("{sec:t¦BAD}{last}", "{sec:target}{last}"),
            ("*{first}{ sec:t¦BAD }", "*{first}{sec:target}"),
            ("{first}% keep\r\n{sec:t¦", "{first}% keep\r\n{sec:target"),
            ("{sec:t¦", "{sec:target"),
            ("*{first}{\n sec:t¦BAD\n }", "*{first}{sec:target}"),
        ] {
            let marked = format!("\\{name}{arguments}");
            let offset = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            let main = insert(&mut workspace, "main.tex", &source);
            let items = features::completion(
                &workspace,
                &main,
                Text::new(source.clone()).position(offset),
            );
            let item = items
                .iter()
                .find(|item| item.label == "sec:target")
                .unwrap_or_else(|| panic!("{marked}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(edit.range.start.line, edit.range.end.line);
            let mut edits = item.additional_text_edits.clone().unwrap_or_default();
            edits.push(edit.clone());
            assert_eq!(apply(&source, &edits), format!("\\{name}{expected}"));
        }
    }
}

#[test]
fn signatures_and_command_snippets_match_two_required_labels_and_keep_existing_arguments() {
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            snippets: true,
            ..Default::default()
        },
        ..Workspace::default()
    };
    for name in COMMANDS {
        for (arguments, active) in [
            ("{fir¦st}{last}", 0),
            ("*{first}{la¦st}", 1),
            (" % star\n * {first}\n{¦", 1),
        ] {
            let marked = format!("\\{name}{arguments}");
            let offset = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            let main = insert(&mut workspace, "main.tex", &source);
            let help = signature::help(
                &workspace,
                &main,
                Text::new(source.clone()).position(offset),
            )
            .unwrap();
            assert_eq!(
                help.signatures[0].label,
                format!("\\{name}{{first}}{{last}}")
            );
            assert_eq!(help.active_parameter, Some(active));
        }
        for following in [
            "",
            "{first}{last}",
            "*{first}{last}",
            " % star\n * {first}{last}",
        ] {
            let source = format!("\\{name}{following}");
            let main = insert(&mut workspace, "main.tex", &source);
            let items = features::completion(
                &workspace,
                &main,
                Text::new(source.clone()).position(1 + name.len()),
            );
            let item = items.iter().find(|item| item.label == *name).unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(
                edit.new_text,
                if following.is_empty() {
                    format!("{name}{{${{1:first}}}}{{${{2:last}}}}")
                } else {
                    (*name).to_owned()
                }
            );
        }
        let source = format!("\\{name}*{{first}}{{last}} prose");
        let main = insert(&mut workspace, "main.tex", &source);
        assert!(signature::help(
            &workspace,
            &main,
            Text::new(source.clone()).position(source.len())
        )
        .is_none());
    }
}

#[test]
fn invalid_options_flags_and_dynamic_or_list_endpoints_do_not_become_label_edits() {
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "labels.tex",
        r"\label{sec:first}\label{sec:last}",
    );
    for name in COMMANDS {
        for arguments in [
            "[note]{sec:first}{sec:last}",
            "*[note]{sec:first}{sec:last}",
            "**{sec:first}{sec:last}",
        ] {
            let source = format!("\\{name}{arguments}");
            let main = insert(&mut workspace, "main.tex", &source);
            for key in ["sec:first", "sec:last"] {
                let position = Text::new(source.clone()).position(source.find(key).unwrap() + 2);
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
        for (arguments, key) in [
            ("{ sec:first }{sec:last}", "sec:first"),
            ("{sec:first}{ sec:last }", "sec:last"),
            ("{sec:first,sec:last}{sec:last}", "sec:first"),
            ("{sec:first}{sec:last,sec:first}", "sec:last"),
            ("{sec:first}{\\chosen}", "chosen"),
        ] {
            let source = format!("\\{name}{arguments}");
            let main = insert(&mut workspace, "main.tex", &source);
            let position = Text::new(source.clone()).position(source.find(key).unwrap() + 2);
            assert!(
                features::prepare_rename(&workspace, &main, position)
                    .unwrap()
                    .is_none(),
                "{source}"
            );
        }
    }
}

#[test]
fn multiline_warnings_locate_and_offer_spelling_fixes_for_each_endpoint() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for name in COMMANDS {
        for star in ["", "*"] {
            let source = format!("\\label{{sec:first}}\\label{{sec:last}}\n😀 \\{name}{star}{{sec:frist}}% between\n{{sec:lsat}} \\{name}[invalid]{{sec:frist}}{{sec:lsat}}\n");
            std::fs::write(&main, &source).unwrap();
            let log = ["sec:frist", "sec:lsat"].map(|key| format!("{}:3: LaTeX Warning: Reference `{key}' on page 1 undefined on input line 3.\n", main.display())).concat();
            let diagnostics = build_log::parse(&main, &log);
            assert_eq!(diagnostics[&uri].len(), 2);
            let mut workspace = Workspace {
                client: texquill::project::state::ClientCapabilities {
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Workspace::default()
            };
            workspace.open(uri.clone(), source.clone(), 7);
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), diagnostics.clone());
            let text = Text::new(source.clone());
            let mut edits = Vec::new();
            for (key, correct) in [("sec:frist", "sec:first"), ("sec:lsat", "sec:last")] {
                let diagnostic = diagnostics[&uri]
                    .iter()
                    .find(|entry| entry.message.contains(key))
                    .unwrap();
                let offset = source.find(key).unwrap();
                assert_eq!(
                    diagnostic.range,
                    text.range(offset, offset + key.len()),
                    "{name}{star}: {diagnostic:?}"
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
                    .unwrap_or_else(|| panic!("{name}{star}: {actions:?}"));
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
}

#[test]
fn identical_endpoints_remain_distinct_references_and_warning_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.tex");
    for name in COMMANDS {
        let source = format!("\\label{{sec:target}}\n\\{name}*{{sec:target}}\n{{sec:target}}\n");
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", &source);
        let position = Text::new(source.clone()).position(source.rfind("sec:target").unwrap() + 2);
        let references = features::references(&workspace, &main, position, true);
        assert_eq!(references.len(), 3);
        let changes = features::rename(&workspace, &main, position, "sec:new")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            apply(&source, &changes[&main]),
            source.replace("sec:target", "sec:new")
        );
        std::fs::write(&path, &source).unwrap();
        let diagnostics = build_log::parse(&path, &format!("{}:3: LaTeX Warning: Reference `sec:target' on page 1 undefined on input line 3.\n", path.display()));
        let entries = &diagnostics[&Url::from_file_path(&path).unwrap()];
        assert_eq!(entries.len(), 2);
        let mut expected = references
            .iter()
            .filter(|entry| entry.range.start.line > 0)
            .map(|entry| entry.range)
            .collect::<Vec<_>>();
        expected.sort_by_key(|range| range.start);
        assert_eq!(
            entries.iter().map(|entry| entry.range).collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn semantic_tokens_mark_only_static_endpoints_in_valid_calls() {
    for name in COMMANDS {
        let source = format!("😀 \\{name}*{{sec:first}}{{sec:last}}\r\n\\{name}{{sec:first}}{{ spaced }}\r\n\\{name}[invalid]{{sec:first}}{{sec:last}}\r\n% \\{name}{{sec:first}}{{sec:last}}\r\n\\verb|\\{name}{{sec:first}}{{sec:last}}|\r\n");
        let mut workspace = Workspace::default();
        let main = insert(&mut workspace, "main.tex", &source);
        let doc = &workspace.documents[&main];
        let legend = semantic::options().legend;
        let mut position = Position::default();
        let mut keys = Vec::new();
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
        assert_eq!(keys, ["sec:first", "sec:last", "sec:first"], "{name}");
    }
}
