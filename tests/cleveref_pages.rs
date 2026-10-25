use latex_lsp::{
    compilation::build_log,
    editing,
    language::{features, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

const COMMANDS: &[&str] = &["cpageref", "Cpageref", "cref", "Cref"];

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///cleveref-pages/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(9)));
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
fn label_lists_navigate_and_rename_each_key_preserving_separators_and_ignored_calls() {
    for name in COMMANDS {
        for star in ["", "*", " % star\r\n * % after\r\n "] {
            let mut workspace = Workspace::default();
            let declarations = r"\label{sec:first}\label{sec:last}";
            let labels = insert(&mut workspace, "labels.tex", declarations);
            let active = format!("😀 \\{name}{star}{{ sec:first ,\r\n sec:last, sec:first }}\r\n");
            let ignored = format!("% \\{name}{{sec:first,sec:last}}\r\n\\verb|\\{name}{{sec:first,sec:last}}|\r\n\\{name}[invalid]{{sec:first,sec:last}}\r\n");
            let source = format!("{active}{ignored}");
            let main = insert(&mut workspace, "main.tex", &source);
            for key in ["sec:first", "sec:last"] {
                for (offset, _) in active.match_indices(key) {
                    let position = Text::new(source.clone()).position(offset + 2);
                    let definitions = features::definitions(&workspace, &main, position);
                    assert_eq!(definitions.len(), 1, "{name}{star}: {key}");
                    assert_eq!(definitions[0].uri, labels);
                    assert!(features::hover(&workspace, &main, position).is_some());
                    assert_eq!(
                        features::references(&workspace, &main, position, true).len(),
                        if key == "sec:first" { 3 } else { 2 }
                    );
                    let changes = features::rename(&workspace, &main, position, "页码😀:new")
                        .unwrap()
                        .unwrap()
                        .changes
                        .unwrap();
                    assert_eq!(
                        apply(&source, &changes[&main]),
                        format!("{}{ignored}", active.replace(key, "页码😀:new"))
                    );
                    assert_eq!(
                        apply(declarations, &changes[&labels]),
                        declarations.replace(key, "页码😀:new")
                    );
                }
            }
        }
    }
}

#[test]
fn label_list_completion_edits_one_key_and_signatures_keep_a_single_list_parameter() {
    let mut workspace = Workspace {
        snippets: true,
        ..Workspace::default()
    };
    insert(&mut workspace, "labels.tex", r"\label{sec:target}");
    for name in COMMANDS {
        for (arguments, expected) in [
            ("{sec:t¦BAD,other}", "{sec:target,other}"),
            ("*{other, sec:t¦BAD ,last}", "*{other, sec:target ,last}"),
            ("*{other,\r\n sec:t¦", "*{other,\r\n sec:target"),
        ] {
            let marked = format!("\\{name}{arguments}");
            let offset = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            let main = insert(&mut workspace, "main.tex", &source);
            let position = Text::new(source.clone()).position(offset);
            let items = features::completion(&workspace, &main, position);
            let item = items
                .iter()
                .find(|item| item.label == "sec:target")
                .unwrap_or_else(|| panic!("{marked}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(
                apply(&source, std::slice::from_ref(edit)),
                format!("\\{name}{expected}")
            );
            let help = signature::help(&workspace, &main, position).unwrap();
            assert_eq!(help.signatures[0].label, format!("\\{name}{{keys}}"));
            assert_eq!(help.active_parameter, Some(0));
        }
        for following in ["", "{first,last}", "*{first,last}"] {
            let source = format!("\\{name}{following}");
            let main = insert(&mut workspace, "main.tex", &source);
            let items = features::completion(
                &workspace,
                &main,
                Text::new(source).position(1 + name.len()),
            );
            let item = items.iter().find(|item| item.label == *name).unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(
                edit.new_text,
                if following.is_empty() {
                    if matches!(*name, "cref" | "Cref") {
                        format!("{name}{{${{1:key}}}}")
                    } else {
                        format!("{name}{{${{1:keys}}}}")
                    }
                } else {
                    (*name).into()
                }
            );
        }
    }
}

#[test]
fn unsupported_label_list_options_and_flags_do_not_create_targets() {
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "labels.tex",
        r"\label{sec:first}\label{sec:last}",
    );
    for name in COMMANDS {
        for prefix in ["[note]", "*[note]", "**"] {
            let source = format!("\\{name}{prefix}{{sec:first,sec:last}}");
            let main = insert(&mut workspace, "main.tex", &source);
            for key in ["sec:first", "sec:last"] {
                let position = Text::new(source.clone()).position(source.find(key).unwrap() + 2);
                assert!(features::definitions(&workspace, &main, position).is_empty());
                assert!(features::prepare_rename(&workspace, &main, position)
                    .unwrap()
                    .is_none());
                assert!(signature::help(&workspace, &main, position).is_none());
                assert!(!features::completion(&workspace, &main, position)
                    .iter()
                    .any(|item| item.label == key));
            }
        }
    }
}

#[test]
fn label_list_warnings_fix_each_matching_occurrence_without_changing_other_keys() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for name in COMMANDS {
        let active = format!("\\label{{sec:first}}\\label{{sec:last}}\n😀 \\{name}*{{ sec:frist,\n sec:lsat ,sec:frist }}\n");
        let ignored = format!("\\{name}[invalid]{{sec:frist,sec:lsat}}\n");
        let source = format!("{active}{ignored}");
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
        assert_eq!(diagnostics[&uri].len(), 3);
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Workspace::default()
        };
        workspace.open(uri.clone(), source.clone(), 9);
        workspace
            .build_diagnostics
            .insert(uri.clone(), diagnostics.clone());
        let text = Text::new(source.clone());
        let mut edits = Vec::new();
        for diagnostic in &diagnostics[&uri] {
            let start = text.offset(diagnostic.range.start).unwrap();
            let end = text.offset(diagnostic.range.end).unwrap();
            let correct = match &source[start..end] {
                "sec:frist" => "sec:first",
                "sec:lsat" => "sec:last",
                value => panic!("unexpected key: {value}"),
            };
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
                .unwrap_or_else(|| panic!("{name}: {actions:?}"));
            assert_eq!(edit.range, diagnostic.range);
            edits.push(edit.clone());
        }
        assert_eq!(
            apply(&source, &edits),
            format!(
                "{}{ignored}",
                active
                    .replace("sec:frist", "sec:first")
                    .replace("sec:lsat", "sec:last")
            )
        );
    }
}

#[test]
fn label_list_semantic_tokens_split_keys_and_preserve_utf16_positions() {
    for name in COMMANDS {
        let source = format!("😀 \\{name}*{{ 章节😀:first, sec:last ,章节😀:first }}\r\n\\{name}[invalid]{{sec:last}}\r\n% \\{name}{{sec:last}}\r\n\\verb|\\{name}{{sec:last}}|\r\n");
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
        assert_eq!(keys, ["章节😀:first", "sec:last", "章节😀:first"]);
    }
}
