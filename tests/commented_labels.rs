use latex_lsp::{
    build_log, editing, features, semantic,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///commented-labels/{name}")).unwrap();
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
fn commented_declarations_and_references_share_names_without_merging_spaced_labels() {
    let calls = [
        "\\ref{% old, }\nold% tail old\n}",
        "\\pageref*{% old\nold% tail\n}",
        "\\autoref{% old\nold}",
        "\\getrefbykeydefault{% old\nold}{anchor}{fallback}",
        "\\vrefrange{% old\nold% tail\n}{other}",
        "\\vpagerefrange*{other}{% old\nold% tail\n}",
        "\\crefrange*{% old\nold}{other}",
        "\\Cpagerefrange{other}{% old\nold}",
        "\\setcounterref{section}{% old\nold% tail\n}",
        "\\hyperref[% old, ]\nold% tail ]\n]{Body}",
        "\\hyperref[{% old\nold% tail\n}]{Body}",
        "\\hyperref[% outer\n{% old\nold% tail\n}% outer tail\n]{Body}",
    ];
    for declaration in [
        "\\label{% declaration old, }\nold% end old\n}",
        "\\hyperdef[% declaration old, ]\nold% end old\n]{category}{destination}{Body}",
        "\\hyperdef[% outer\n{% declaration old\nold% end old\n}% outer tail\n]{category}{destination}{Body}",
    ] {
        for call in calls {
            let mut workspace = Workspace::default();
            let declarations = format!("{declaration}\\label{{ old }}\\label{{other}}\n");
            let labels = insert(&mut workspace, "labels.tex", &declarations);
            assert_eq!(
                workspace.documents[&labels]
                    .syntax
                    .labels
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                ["old", "other"]
            );
            let source = format!("😀 {call}\n\\ref{{old}}\\ref{{ old }}\n");
            let main = insert(&mut workspace, "main.tex", &source);
            let at = Text::new(source.clone()).position(source.find("\nold").unwrap() + 2);
            let definitions = features::definitions(&workspace, &main, at);
            assert_eq!(
                definitions.len(),
                1,
                "{declaration}: {call}: {definitions:?}"
            );
            assert_eq!(definitions[0].uri, labels);
            assert!(features::hover(&workspace, &main, at).is_some());
            assert_eq!(features::references(&workspace, &main, at, true).len(), 3);
            let changes = features::rename(&workspace, &main, at, "new")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(
                apply(&source, &changes[&main]),
                source
                    .replace("\nold", "\nnew")
                    .replace("\\ref{old}", "\\ref{new}")
            );
            assert_eq!(
                apply(&declarations, &changes[&labels]),
                declarations.replace("\nold", "\nnew")
            );
            let from_definition =
                Text::new(declarations.clone()).position(declarations.find("\nold").unwrap() + 2);
            assert_eq!(
                features::references(&workspace, &labels, from_definition, false).len(),
                2
            );
        }
    }
}

#[test]
fn comment_aware_single_name_completion_removes_literal_spaces_but_keeps_comments() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "labels.tex", r"\label{old}");
    for (marked, expected) in [
        (
            "\\ref{% before, }\r\n ol¦BAD % tail\r\n}",
            "\\ref{% before, }\r\nold% tail\r\n}",
        ),
        (
            "\\ref{ \n% before\n ol¦BAD % tail\n \n}",
            "\\ref{% before\nold% tail\n}",
        ),
        ("\\ref{% before\n¦}", "\\ref{% before\nold}"),
        ("\\ref{¦% after\n}", "\\ref{old% after\n}"),
        ("\\ref{% before\nol¦", "\\ref{% before\nold"),
        (
            "\\hyperref[% before, ]\nol¦BAD% after ]\n]{Body}",
            "\\hyperref[% before, ]\nold% after ]\n]{Body}",
        ),
        (
            "\\hyperref[{% before\nol¦BAD% after\n}]{Body}",
            "\\hyperref[{% before\nold% after\n}]{Body}",
        ),
        (
            "\\hyperref[% outer\n{% before\nol¦BAD% after\n}% outer tail\n]{Body}",
            "\\hyperref[% outer\n{% before\nold% after\n}% outer tail\n]{Body}",
        ),
        (
            "\\hyperref[{% before\nol¦BAD% after\n}% outer tail\n]{Body}",
            "\\hyperref[{% before\nold% after\n}% outer tail\n]{Body}",
        ),
        (
            "\\setcounterref{section}{% before\n ol¦BAD % after\n}",
            "\\setcounterref{section}{% before\nold% after\n}",
        ),
        (
            "\\crefrange{first}{% before\n ol¦BAD % after\n}",
            "\\crefrange{first}{% before\nold% after\n}",
        ),
    ] {
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
            .find(|item| item.label == "old")
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.range.start.line, edit.range.end.line);
        let mut edits = item.additional_text_edits.clone().unwrap_or_default();
        edits.push(edit.clone());
        let mut ordered = edits.clone();
        ordered.sort_by_key(|edit| edit.range.start);
        assert!(ordered
            .windows(2)
            .all(|pair| pair[0].range.end <= pair[1].range.start));
        assert_eq!(apply(&source, &edits), expected, "{marked}");
    }
}

#[test]
fn literal_spaces_spliced_names_and_dynamic_names_are_not_normalized_into_static_labels() {
    let mut workspace = Workspace::default();
    let declarations = "\\label{old}\\label{% before\n old}\\label{old % after\n}\\label{% before\nold\n}\\label{ol% middle\nd}\\label{\\chosen}\n";
    let labels = insert(&mut workspace, "labels.tex", declarations);
    assert_eq!(workspace.documents[&labels].syntax.labels.len(), 1);
    for arguments in [
        "{% before\n old}",
        "{old % after\n}",
        "{% before\nold\n}",
        "{ol% middle\nd}",
        "{\\chosen}",
    ] {
        let source = format!("\\ref{arguments}");
        let main = insert(&mut workspace, "main.tex", &source);
        let offset = source
            .find("old")
            .or_else(|| source.find("ol%"))
            .or_else(|| source.find("chosen"))
            .unwrap()
            + 1;
        let at = Text::new(source.clone()).position(offset);
        assert!(
            features::prepare_rename(&workspace, &main, at)
                .unwrap()
                .is_none(),
            "{source}"
        );
        if arguments.contains("middle") {
            assert!(!features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.label == "old"));
        }
    }
    for source in [
        "\\hyperref[ {% before\nold}]{Body}",
        "\\hyperref[{% before\nold} % after\n]{Body}",
    ] {
        let main = insert(&mut workspace, "main.tex", source);
        let at = Text::new(source.into()).position(source.find("old").unwrap() + 1);
        assert!(features::prepare_rename(&workspace, &main, at)
            .unwrap()
            .is_none());
    }
}

#[test]
fn commented_hyperref_labels_remain_indexed_through_repeated_bracket_renames() {
    for declaration in [
        "\\label{% declaration old\nold% tail old\n}",
        "\\hyperdef[% declaration old\nold% tail old\n]{category}{destination}{Body}",
        "\\hyperdef[% outer\n{% declaration old\nold% tail old\n}% outer tail\n]{category}{destination}{Body}",
    ] {
        let mut workspace = Workspace::default();
        let mut declarations = format!("{declaration}\\label{{ old }}");
        let labels = insert(&mut workspace, "labels.tex", &declarations);
        let mut source = "\\hyperref[% before old\nold% after old\n]{Body}\n\\hyperref[% outer\n{% before old\nold% after old\n}% outer tail\n]{Protected}".to_string();
        let main = insert(&mut workspace, "main.tex", &source);
        let mut previous = "old";
        for next in ["renamed", "sec:[bracket]", "sec:[next]", "final"] {
            let at = Text::new(source.clone()).position(source.find(previous).unwrap() + 1);
            // Select a real name, rather than its unchanged occurrence in a comment.
            let at = if previous == "old" {
                Position::new(1, 1)
            } else {
                at
            };
            assert_eq!(features::definitions(&workspace, &main, at).len(), 1);
            assert_eq!(features::references(&workspace, &main, at, true).len(), 3);
            let changes = features::rename(&workspace, &main, at, next)
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 2);
            for uri in [&main, &labels] {
                let doc = &workspace.documents[uri];
                let comments: Vec<_> = doc
                    .syntax
                    .comments
                    .iter()
                    .map(|span| doc.text.source[span.clone()].to_string())
                    .collect();
                let updated = apply(&doc.text.source, &changes[uri]);
                insert(
                    &mut workspace,
                    if *uri == main { "main.tex" } else { "labels.tex" },
                    &updated,
                );
                let doc = &workspace.documents[uri];
                assert_eq!(
                    doc.syntax
                        .comments
                        .iter()
                        .map(|span| doc.text.source[span.clone()].to_string())
                        .collect::<Vec<_>>(),
                    comments
                );
            }
            source = workspace.documents[&main].text.source.clone();
            declarations = workspace.documents[&labels].text.source.clone();
            assert!(declarations.contains("\\label{ old }"));
            let at = Text::new(source.clone()).position(source.find(next).unwrap() + 1);
            assert_eq!(
                features::references(&workspace, &main, at, true).len(),
                3,
                "{declaration}: {next}: {source}"
            );
            previous = next;
        }
    }
}

#[test]
fn bracket_name_completion_stays_navigable_with_comments_outside_protection() {
    let mut workspace = Workspace::default();
    let labels = insert(&mut workspace, "labels.tex", "\\label{sec:[target]}");
    for (marked, expected) in [
        (
            "\\hyperref[% before\nse¦BAD% tail\n]{Body}",
            "\\hyperref[% before\n{sec:[target]}% tail\n]{Body}",
        ),
        (
            "\\hyperref[% outer\n{% before\nse¦BAD% tail\n}% outer tail\n]{Body}",
            "\\hyperref[% outer\n{% before\nsec:[target]% tail\n}% outer tail\n]{Body}",
        ),
        (
            "\\hyperref[{% before\nse¦BAD% tail\n}% outer tail\n]{Body}",
            "\\hyperref[{% before\nsec:[target]% tail\n}% outer tail\n]{Body}",
        ),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let item = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        )
        .into_iter()
        .find(|item| item.label == "sec:[target]")
        .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        let updated = apply(&source, &edits);
        assert_eq!(updated, expected);
        insert(&mut workspace, "main.tex", &updated);
        let at = Text::new(updated.clone()).position(updated.find("sec:[target]").unwrap() + 2);
        let definitions = features::definitions(&workspace, &main, at);
        assert_eq!(definitions.len(), 1, "{updated}");
        assert_eq!(definitions[0].uri, labels);
    }
}

#[test]
fn trailing_comments_do_not_hide_the_reported_closing_line_of_missing_labels() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for (call, closing) in [
        ("\\ref{% before\nolld% tail\n}", 3),
        ("\\vrefrange{% before\nolld% tail\n}{other}", 3),
        ("\\crefrange{other}{% before\nolld% tail\n}", 3),
        ("\\setcounterref{section}{% before\nolld% tail\n}", 3),
        ("\\hyperref[% before\nolld% tail\n]{Body\nmore}", 4),
        (
            "\\hyperref[% outer\n{% before\nolld% tail\n}% outer tail\n]{Body\nmore}",
            6,
        ),
    ] {
        let source = format!("\\label{{old}}\\label{{other}}\n😀 {call}\n");
        std::fs::write(&main, &source).unwrap();
        let warning = if call.starts_with("\\hyperref") {
            "Hyper reference"
        } else {
            "Reference"
        };
        let log = format!(
            "{}:{}: LaTeX Warning: {warning} `olld' on page 1 undefined on input line {}.\n",
            main.display(),
            closing + 1,
            closing + 1
        );
        let diagnostics = build_log::parse(&main, &log);
        let diagnostic = &diagnostics[&uri][0];
        let offset = source.find("\nolld").unwrap() + 1;
        assert_eq!(
            diagnostic.range,
            Text::new(source.clone()).range(offset, offset + 4),
            "{call}: {diagnostic:?}"
        );
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Workspace::default()
        };
        workspace.open(uri.clone(), source.clone(), 7);
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
        let edit = actions
            .iter()
            .find_map(|action| match action {
                CodeActionOrCommand::CodeAction(action) => {
                    action.edit.as_ref()?.changes.as_ref()?[&uri]
                        .iter()
                        .find(|edit| edit.new_text == "old")
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{call}: {actions:?}"));
        assert_eq!(
            apply(&source, std::slice::from_ref(edit)),
            source.replace("\nolld", "\nold")
        );
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_commented_labels_complete_and_rename_while_spaced_labels_remain_distinct() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let labels = directory.path().join("labels.tex");
    let child = directory.path().join("child.tex");
    let main_source = "\\documentclass{article}\n\\begin{document}\n\\input{labels}\n\\input{child}\n\\end{document}\n";
    let declaration = "\\section{Hi}\\label{% old, }\nold% tail old\n}\\label{ old }\n";
    let reference = "See \\ref{% old, }\n oldBAD % tail old\n}, plus \\ref{ old }.\n";
    for (path, source) in [
        (&main, main_source),
        (&labels, declaration),
        (&child, reference),
    ] {
        std::fs::write(path, source).unwrap();
    }
    let main_uri = Url::from_file_path(&main).unwrap();
    let labels_uri = Url::from_file_path(&labels).unwrap();
    let child_uri = Url::from_file_path(&child).unwrap();
    let mut workspace = Workspace::default();
    workspace.open(main_uri, main_source.into(), 1);
    workspace.open(labels_uri.clone(), declaration.into(), 5);
    workspace.open(child_uri.clone(), reference.into(), 7);
    let items = features::completion(&workspace, &child_uri, Position::new(1, 3));
    let item = items.iter().find(|item| item.label == "old").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing edit")
    };
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    let fixed = apply(reference, &edits);
    assert_eq!(fixed, reference.replace("\n oldBAD %", "\nold%"));
    std::fs::write(&child, &fixed).unwrap();
    workspace.open(child_uri.clone(), fixed.clone(), 8);
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let compile = || {
        let outcome = latex_lsp::build::run(&main, &config, &Default::default());
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        outcome
    };
    compile();
    assert!(compile().diagnostics.values().all(Vec::is_empty));
    let position = Position::new(1, 2);
    let definitions = features::definitions(&workspace, &child_uri, position);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].uri, labels_uri);
    let changes = features::rename(&workspace, &child_uri, position, "new")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    let changed_declaration = apply(declaration, &changes[&labels_uri]);
    let changed_reference = apply(&fixed, &changes[&child_uri]);
    assert_eq!(changed_declaration, declaration.replace("\nold%", "\nnew%"));
    assert_eq!(changed_reference, fixed.replace("\nold%", "\nnew%"));
    std::fs::write(&labels, changed_declaration).unwrap();
    std::fs::write(&child, changed_reference).unwrap();
    compile();
    let outcome = compile();
    assert!(outcome.diagnostics.values().all(Vec::is_empty));
    let aux = std::fs::read_to_string(outcome.pdf.unwrap().with_extension("aux")).unwrap();
    assert!(aux.contains("\\newlabel{new}"));
    assert!(aux.contains("\\newlabel{ old }"));
    assert!(!aux.contains("\\newlabel{old}"));
}

#[test]
fn semantic_tokens_and_symbols_use_the_actual_commented_label_name_ranges() {
    let source = "😀 \\label{% 章节😀, }\r\n章节😀% tail\r\n}\r\n\\ref{% 章节😀\r\n章节😀}\r\n\\crefrange{other}{% 章节😀\r\n章节😀% tail\r\n}\r\n\\hyperref[% 章节😀\r\n章节😀]{Body}\r\n\\ref{% before\r\n 章节😀}\r\n";
    let mut workspace = Workspace::default();
    let main = insert(&mut workspace, "main.tex", source);
    let doc = &workspace.documents[&main];
    let symbols = features::symbols(doc);
    assert_eq!(symbols.len(), 1);
    assert!(symbols[0].name.contains("章节😀"));
    let start = doc.text.offset(symbols[0].selection_range.start).unwrap();
    let end = doc.text.offset(symbols[0].selection_range.end).unwrap();
    assert_eq!(&source[start..end], "章节😀");
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
    assert_eq!(keys, ["章节😀", "章节😀", "other", "章节😀", "章节😀"]);
}

#[test]
fn parenthesized_labels_rename_across_commented_single_list_and_separate_arguments() {
    for old in ["sec:(old)", "sec:old(", "sec:old)", "章节(😀)"] {
        for call in [
            "\\ref{% before\r\nKEY% tail\r\n}",
            "\\pageref*{KEY}",
            "\\autoref{KEY}",
            "\\cref{other,% before\r\nKEY% tail\r\n}",
            "\\cpageref*{KEY,other}",
            "\\crefrange{KEY}{other}",
            "\\vrefrange{other}{KEY}",
            "\\setcounterref{section}{KEY}",
            "\\getrefbykeydefault{KEY}{page}{0}",
            "\\hyperref[% before\r\nKEY% tail\r\n]{Text}",
            "\\hyperref[{KEY}]{Text}",
        ] {
            for declaration in [
                "\\label{% before\r\nKEY% tail\r\n}",
                "\\hyperdef[{KEY}]{cat}{dest}{Text}",
            ] {
                let mut workspace = Workspace::default();
                let declarations = format!("{}\\label{{other}}", declaration.replace("KEY", old));
                let labels = insert(&mut workspace, "labels.tex", &declarations);
                let source = format!(
                    "😀 {}\r\n\\ref{{{old}}}\r\nOrdinary {old}\r\n% {old}\r\n\\verb|{old}|\r\n",
                    call.replace("KEY", old)
                );
                let main = insert(&mut workspace, "main.tex", &source);
                let at = Text::new(source.clone()).position(source.find(old).unwrap() + 1);
                assert_eq!(
                    features::definitions(&workspace, &main, at)
                        .iter()
                        .map(|d| &d.uri)
                        .collect::<Vec<_>>(),
                    vec![&labels],
                    "{source}"
                );
                assert!(features::prepare_rename(&workspace, &main, at)
                    .unwrap()
                    .is_some());
                assert!(features::hover(&workspace, &main, at).is_some());
                assert_eq!(
                    features::references(&workspace, &main, at, true).len(),
                    3,
                    "{declaration}: {source}"
                );
                let next = "sec:(renamed)";
                let changes = features::rename(&workspace, &main, at, next)
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(changes.values().map(Vec::len).sum::<usize>(), 3);
                for (uri, name, original) in [
                    (&main, "main.tex", &source),
                    (&labels, "labels.tex", &declarations),
                ] {
                    let text = Text::new(original.clone());
                    for edit in &changes[uri] {
                        assert_eq!(
                            &original[text.offset(edit.range.start).unwrap()
                                ..text.offset(edit.range.end).unwrap()],
                            old
                        );
                    }
                    let updated = apply(original, &changes[uri]);
                    if uri == &main {
                        assert!(updated
                            .contains(&format!("Ordinary {old}\r\n% {old}\r\n\\verb|{old}|")));
                    }
                    insert(&mut workspace, name, &updated);
                }
                let updated = &workspace.documents[&main].text;
                let at = updated.position(updated.source.find(next).unwrap() + 1);
                assert_eq!(features::references(&workspace, &main, at, true).len(), 3);
            }
        }
    }
}

#[test]
fn parenthesized_label_completion_and_semantics_use_complete_literal_keys() {
    let key = "sec:(target)";
    let mut workspace = Workspace::default();
    insert(&mut workspace, "labels.tex", &format!("\\label{{{key}}}"));
    for marked in [
        "\\ref{sec:(t¦BAD)}",
        "\\ref{% before\r\n sec:(t¦BAD) % tail\r\n}",
        "\\hyperref[{sec:(t¦BAD)}]{Body}",
        "\\crefrange{other}{sec:(t¦BAD)}",
        "\\cref{other, sec:(t¦BAD)}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let item = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        )
        .into_iter()
        .find(|i| i.label == key)
        .unwrap_or_else(|| panic!("{marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        let updated = apply(&source, &edits);
        assert!(updated.contains(key));
        assert!(!updated.contains("BAD"));
        if source.contains("% before") {
            assert!(updated.contains("% before\r\n"));
            assert!(updated.contains("% tail\r\n"));
        }
        insert(&mut workspace, "main.tex", &updated);
        let at = Text::new(updated.clone()).position(updated.find(key).unwrap() + 1);
        assert_eq!(features::definitions(&workspace, &main, at).len(), 1);
    }
    let source=format!("% 😀\r\n\\label{{{key}}}\\ref{{{key}}}\\cref{{{key},other}}\\hyperref[{{{key}}}]{{Body}}\\crefrange{{other}}{{{key}}}");
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
            let end = Position::new(position.line, position.character + token.length);
            keys.push(&source[doc.text.offset(position).unwrap()..doc.text.offset(end).unwrap()]);
        }
    }
    assert_eq!(keys, vec![key, key, key, "other", key, "other", key]);
    assert!(features::symbols(doc)
        .iter()
        .any(|symbol| symbol.name.contains(key)));
}

#[test]
fn parenthesized_labels_preserve_conflicts_and_dynamic_or_other_namespace_guards() {
    let mut workspace = Workspace::default();
    let source="\\label{sec:(old)}\\label{sec:(used)}\\newcommand{\\macro}{}\\newenvironment{panel}{}{}\\newcounter{count}\\bibitem{book} Text\n\\ref{sec:(old)}";
    let main = insert(&mut workspace, "main.tex", source);
    let text = Text::new(source.into());
    let at = text.position(source.rfind("sec:(old)").unwrap() + 1);
    for invalid in [
        "sec:(used)",
        "bad name",
        "bad=key",
        "\\chosen",
        "a,b",
        "a%comment",
        "a{b}",
    ] {
        assert!(
            features::rename(&workspace, &main, at, invalid).is_err(),
            "{invalid}"
        );
    }
    for name in ["macro", "panel", "count"] {
        let offset = if name == "macro" {
            source.find(name).unwrap()
        } else {
            source.find(&format!("{{{name}}}")).unwrap() + 1
        };
        let at = text.position(offset + 1);
        assert!(
            features::rename(&workspace, &main, at, "name(paren)").is_err(),
            "{name}"
        );
    }
    for argument in [
        "{ sec:(old)}",
        "{sec:(old) }",
        "{sec:(o% splice\nld)}",
        "{\\chosen(old)}",
        "{{sec:(old)}}",
    ] {
        let source = format!("\\ref{argument}");
        let main = insert(&mut workspace, "uses.tex", &source);
        let at = Text::new(source.clone())
            .position(source.find("old").or_else(|| source.find("sec:")).unwrap() + 1);
        assert!(
            features::prepare_rename(&workspace, &main, at)
                .unwrap()
                .is_none(),
            "{source}"
        );
    }
}

#[test]
fn parenthesized_label_rename_respects_projects_and_shared_reference_conflicts() {
    for shared in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("shared.tex"), r"\ref{sec:(old)}").unwrap();
        for name in ["a", "b"] {
            std::fs::create_dir(root.join(name)).unwrap();
            let included = if shared { "../shared" } else { "child" };
            std::fs::write(
                root.join(format!("{name}/main.tex")),
                format!(
                    "\\documentclass{{article}}\\label{{sec:(old)}}\\input{{{included}}}{}",
                    if name == "b" {
                        r"\label{sec:(new)}"
                    } else {
                        ""
                    }
                ),
            )
            .unwrap();
            std::fs::write(root.join(format!("{name}/child.tex")), r"\ref{sec:(old)}").unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let other = Url::from_file_path(root.join("b/main.tex")).unwrap();
        workspace.refresh_project(&other);
        assert!(workspace.documents.contains_key(&other));
        let main = Url::from_file_path(root.join("a/main.tex")).unwrap();
        workspace.refresh_project(&main);
        let doc = &workspace.documents[&main];
        let at = doc
            .text
            .position(doc.text.source.find("sec:(old)").unwrap() + 1);
        let result = features::rename(&workspace, &main, at, "sec:(new)");
        if shared {
            assert!(result.is_err());
        } else {
            let changes = result.unwrap().unwrap().changes.unwrap();
            assert_eq!(changes.len(), 2);
            assert!(changes.keys().all(|uri| uri.path().contains("/a/")));
        }
    }
}

#[test]
fn parenthesized_label_spelling_fixes_preserve_the_compilers_exact_key_range() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for (call, kind) in [
        ("\\ref{sec:(targte)}", "Reference"),
        ("\\hyperref[{sec:(targte)}]{Body}", "Hyper reference"),
    ] {
        let source = format!("\\label{{sec:(target)}}\n{call}\n");
        std::fs::write(&main, &source).unwrap();
        let diagnostics = build_log::parse(&main, &format!("(./main.tex\nLaTeX Warning: {kind} `sec:(targte)' on page 1 undefined on input line 2.\n)\n"));
        let diagnostic = diagnostics[&uri][0].clone();
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Workspace::default()
        };
        workspace.open(uri.clone(), source.clone(), 7);
        workspace.build_diagnostics.insert(uri.clone(), diagnostics);
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                only: None,
                trigger_kind: None,
            },
        );
        let edits = actions
            .into_iter()
            .filter_map(|action| match action {
                CodeActionOrCommand::CodeAction(action) => action.edit,
                _ => None,
            })
            .flat_map(|edit| {
                edit.changes
                    .unwrap_or_default()
                    .remove(&uri)
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        assert_eq!(edits.len(), 1, "{call}: {edits:?}");
        assert_eq!(
            apply(&source, &edits),
            source.replace("sec:(targte)", "sec:(target)")
        );
    }
}
