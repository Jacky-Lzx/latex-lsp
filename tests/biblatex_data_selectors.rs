use lsp_types::*;
use texquill::{
    language::features,
    project::workspace::{Document, Workspace},
    text::Text,
};

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///data-selectors/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(6)));
    uri
}

fn edits(item: CompletionItem) -> Vec<TextEdit> {
    assert_ne!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
        panic!()
    };
    let mut edits = item.additional_text_edits.unwrap_or_default();
    edits.push(edit);
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    edits
}

fn apply(source: &str, edits: Vec<TextEdit>) -> String {
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

const COMMANDS: &[(&str, &str, &str)] = &[
    ("printfield", "", "title"),
    ("citefield", "{alpha}", "title"),
    ("printlist", "", "publisher"),
    ("citelist", "{alpha}", "publisher"),
    ("printnames", "", "author"),
    ("citename", "{alpha}", "author"),
];

#[test]
fn data_selector_candidates_follow_output_field_and_list_kinds() {
    for (name, before, expected) in COMMANDS {
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                snippets: true,
                ..Default::default()
            },
            ..Default::default()
        };
        insert(
            &mut workspace,
            "refs.bib",
            "@book{alpha,title={Book}} @book{unrelated,title={Other}}",
        );
        let source = format!("\\{name}{before}{{}}");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main].text.position(source.len() - 1);
        let items = features::completion(&workspace, &main, at);
        assert!(
            items.iter().any(|item| item.label == *expected),
            "{name}: {items:?}"
        );
        assert!(items
            .iter()
            .all(|item| item.kind == Some(CompletionItemKind::FIELD)));
        let labels: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
        assert!(!labels.contains(&"unrelated"));
        assert!(!labels.contains(&"sortname"));
        assert!(!labels.contains(&"date"));
        if *expected == "title" {
            assert!(labels.contains(&"year"));
            assert!(labels.contains(&"doi"));
            assert!(!labels.contains(&"author"));
            assert!(!labels.contains(&"publisher"));
            assert!(!labels.contains(&"journal"));
        } else if *expected == "publisher" {
            assert!(labels.contains(&"language"));
            assert!(!labels.contains(&"author"));
            assert!(!labels.contains(&"title"));
        } else {
            assert!(labels.contains(&"translator"));
            assert!(labels.contains(&"holder"));
            assert!(!labels.contains(&"title"));
            assert!(!labels.contains(&"publisher"));
        }
    }
}

#[test]
fn selector_completion_replaces_one_name_preserving_notes_comments_ranges_and_call_boundaries() {
    for (name, before, expected) in COMMANDS {
        let prefix = expected[..2].to_ascii_uppercase();
        let options = if before.is_empty() {
            if *name == "printfield" {
                "[fm[t]"
            } else {
                "[{fm[t]}][2-4]"
            }
        } else {
            "[see][12]"
        };
        let format = if before.is_empty() { "" } else { "[fmt]" };
        for (argument, completed) in [
            (format!("{{{prefix}¦BAD}}"), format!("{{{expected}}}")),
            (
                format!("{{% before😀\r\n {prefix}¦BAD % after\r\n}}"),
                format!("{{% before😀\r\n{expected}% after\r\n}}"),
            ),
            (
                format!("{{\r\n {prefix}¦BAD \r\n}}"),
                format!("{{{expected}}}"),
            ),
            (
                format!("{{% before\r\n{prefix}¦"),
                format!("{{% before\r\n{expected}"),
            ),
            (
                format!("{{% before\r\n{prefix}¦% after\r\n}}"),
                format!("{{% before\r\n{expected}% after\r\n}}"),
            ),
        ] {
            let call = format!("\\{name}{options}{before}{format}");
            let marked = format!("% 😀\r\n{call}{argument}");
            let source = marked.replace('¦', "");
            let mut workspace = Workspace::default();
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main]
                .text
                .position(marked.find('¦').unwrap());
            let item = features::completion(&workspace, &main, at)
                .into_iter()
                .find(|item| item.label == *expected)
                .unwrap_or_else(|| panic!("{marked}"));
            let changes = edits(item);
            let main_edit = changes
                .iter()
                .find(|edit| !edit.new_text.is_empty())
                .unwrap();
            assert_eq!(main_edit.range.start.line, at.line);
            assert_eq!(main_edit.range.end.line, at.line);
            assert_eq!(
                apply(&source, changes),
                format!("% 😀\r\n{call}{completed}"),
                "{marked}"
            );
        }
    }
}

#[test]
fn selector_hover_describes_the_exact_name_without_creating_citation_navigation() {
    for (name, before, field) in COMMANDS {
        let source =
            format!("% 😀\r\n\\{name}{before}{{% before\r\n{field}% after\r\n}}\\label{{{field}}}");
        let mut workspace = Workspace::default();
        insert(
            &mut workspace,
            "refs.bib",
            &format!("@book{{{field},title={{Book}}}} @book{{alpha,title={{Alpha}}}}"),
        );
        let main = insert(&mut workspace, "main.tex", &source);
        let doc = &workspace.documents[&main];
        let start = source.find(&format!("\r\n{field}% after")).unwrap() + 2;
        let at = doc.text.position(start + 2);
        let hover = features::hover(&workspace, &main, at).unwrap_or_else(|| panic!("{source}"));
        assert_eq!(
            hover.range,
            Some(doc.text.range(start, start + field.len()))
        );
        let HoverContents::Markup(contents) = hover.contents else {
            panic!()
        };
        assert!(contents.value.starts_with(&format!("{field}\n\n")));
        assert!(features::definitions(&workspace, &main, at).is_empty());
        assert!(features::prepare_rename(&workspace, &main, at)
            .unwrap()
            .is_none());
        assert!(features::references(&workspace, &main, at, true).is_empty());
    }
}

#[test]
fn unsupported_arguments_prose_literal_commands_and_user_definitions_do_not_get_selectors() {
    for marked in [
        "\\printfield*[fmt]{ti¦tle}",
        "\\printfield[one][two]{ti¦tle}",
        "\\printnames[one][two][three]{au¦thor}",
        "\\printfield[ti¦tle]{author}",
        "\\printnames[fmt][au¦thor]{author}",
        "\\printfield{{ti¦tle}}",
        "\\printfield{ti¦tle,year}",
        "\\printfield{ti¦tle\\suffix}",
        "\\printfield{ti¦% joined\r\ntle}",
        "\\printfield{ti¦tle author}",
        "\\printfield{title}{ti¦tle}",
        "\\printfield{title} prose {ti¦tle}",
        "\\printfield\n\n{ti¦tle}",
        "\\printfield[fmt\n{ti¦tle}",
        "\\citefield[one][two][three]{alpha}{ti¦tle}",
        "\\citefield{alpha}[ti¦tle]{title}",
        "\\citefield{al¦pha}{title}",
        "\\meaning\\printfield{ti¦tle}",
        "\\verb|\\printfield{ti¦tle}|",
        "% \\printfield{ti¦tle}\r\n",
        "\\begin{verbatim}\\printfield{ti¦tle}\\end{verbatim}",
        "\\newcommand\\printfield[1]{}\\printfield{ti¦tle}",
        "\\newcommand\\citefield[2]{}\\citefield{alpha}{ti¦tle}",
        "\\newcommand\\custom[1]{}\\let\\printnames\\custom\\printnames{au¦thor}",
    ] {
        let source = marked.replace('¦', "");
        let mut workspace = Workspace::default();
        insert(&mut workspace, "refs.bib", "@book{alpha,title={Alpha}}");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        assert!(
            !features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.kind == Some(CompletionItemKind::FIELD)),
            "{marked}"
        );
        let hover = features::hover(&workspace, &main, at);
        if !marked.contains("al¦pha") {
            assert!(hover.is_none(), "{marked}: {hover:?}");
        }
    }
}

#[test]
fn raw_field_accessors_and_undefined_tests_complete_only_their_first_name_argument() {
    for (name, field, label, branches) in [
        ("thefield", "title", "field", false),
        ("strfield", "title", "field", false),
        ("iffieldundef", "title", "field", true),
        ("iflistundef", "publisher", "literal list", true),
        ("ifnameundef", "author", "name list", true),
    ] {
        let mut workspace = Workspace::default();
        insert(
            &mut workspace,
            "refs.bib",
            &format!("@book{{{field},title={{Book}}}}"),
        );
        let tail = if branches {
            format!("{{True {field}}}{{False {field}}}")
        } else {
            String::new()
        };
        let source = format!("% 😀\r\n\\{name}{{% before\r\n{field}% after\r\n}}{tail} prose");
        let main = insert(&mut workspace, "main.tex", &source);
        let doc = &workspace.documents[&main];
        let start = source.find(&format!("\r\n{field}% after")).unwrap() + 2;
        let at = doc.text.position(start + 2);
        let item = features::completion(&workspace, &main, at)
            .into_iter()
            .find(|item| item.label == field)
            .unwrap_or_else(|| panic!("{source}"));
        assert_eq!(item.kind, Some(CompletionItemKind::FIELD));
        assert_eq!(apply(&source, edits(item)), source);
        assert_eq!(
            features::hover(&workspace, &main, at).unwrap().range,
            Some(doc.text.range(start, start + field.len()))
        );
        assert!(features::definitions(&workspace, &main, at).is_empty());
        assert!(features::prepare_rename(&workspace, &main, at)
            .unwrap()
            .is_none());
        let help = texquill::language::signature::help(&workspace, &main, at).unwrap();
        assert_eq!(help.active_parameter, Some(0));
        assert_eq!(
            help.signatures[0].label,
            format!(
                "\\{name}{{{label}}}{}",
                if branches { "{true}{false}" } else { "" }
            )
        );
        if branches {
            for (marker, parameter) in [("True", 1), ("False", 2)] {
                let at = doc.text.position(
                    source.find(&format!("{marker} {field}")).unwrap() + marker.len() + 2,
                );
                assert!(!features::completion(&workspace, &main, at)
                    .iter()
                    .any(|item| item.kind == Some(CompletionItemKind::FIELD)));
                assert!(features::hover(&workspace, &main, at).is_none());
                assert_eq!(
                    texquill::language::signature::help(&workspace, &main, at)
                        .unwrap()
                        .active_parameter,
                    Some(parameter)
                );
            }
        }
    }
}

#[test]
fn generated_counter_theorem_and_conditional_commands_disable_conflicting_builtin_selectors() {
    for (declaration, name, field) in [
        ("\\newcounter{field}", "thefield", "title"),
        ("\\newaliascnt{field}{page}", "thefield", "title"),
        ("\\newtheorem{field}{Field}", "thefield", "title"),
        ("\\newif\\iffieldundef", "iffieldundef", "title"),
        ("\\newif\\iflistundef", "iflistundef", "publisher"),
        ("\\newif\\ifnameundef", "ifnameundef", "author"),
    ] {
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                snippets: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let source = format!("{declaration}\\{name}{{{field}}}");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(source.rfind(field).unwrap() + 2);
        assert!(
            !features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.kind == Some(CompletionItemKind::FIELD)),
            "{source}"
        );
        assert!(features::hover(&workspace, &main, at).is_none(), "{source}");
        assert!(
            texquill::language::signature::help(&workspace, &main, at).is_none(),
            "{source}"
        );
        let source = format!("{declaration}\\{name}");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main].text.position(source.len());
        let item = features::completion(&workspace, &main, at)
            .into_iter()
            .find(|item| item.label == name)
            .unwrap();
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
        assert!(item.documentation.is_none());
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, name);
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("a.tex"),
        "\\documentclass{article}\\newcounter{field}\\thefield{title}",
    )
    .unwrap();
    let source = "\\documentclass{article}\\thefield{title}";
    std::fs::write(root.join("b.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("b.tex")).unwrap();
    let at = workspace.documents[&main]
        .text
        .position(source.find("title").unwrap() + 2);
    assert!(features::completion(&workspace, &main, at)
        .iter()
        .any(|item| item.label == "title" && item.kind == Some(CompletionItemKind::FIELD)));
}

#[test]
fn selector_candidates_and_hovers_refresh_with_unsaved_argument_and_definition_changes() {
    let mut workspace = Workspace::default();
    let source = "\\printnames{author}".to_string();
    let main = insert(&mut workspace, "main.tex", &source);
    for (replacement, candidate) in [
        ("\\printnames{author}", Some("author")),
        ("\\printlist{publisher}", Some("publisher")),
        ("\\printfield{title}", Some("title")),
        ("\\printfield{customfield}", None),
        ("\\newcommand\\printfield[1]{}\\printfield{title}", None),
    ] {
        let source = replacement.to_owned();
        workspace.open(main.clone(), source.clone(), 7);
        let doc = &workspace.documents[&main];
        let at = doc.text.position(source.rfind('}').unwrap());
        let items = features::completion(&workspace, &main, at);
        assert_eq!(
            features::hover(&workspace, &main, at).is_some(),
            candidate.is_some(),
            "{source}"
        );
        if let Some(candidate) = candidate {
            assert!(items.iter().any(
                |item| item.label == candidate && item.kind == Some(CompletionItemKind::FIELD)
            ));
        } else {
            assert!(!items
                .iter()
                .any(|item| item.kind == Some(CompletionItemKind::FIELD)));
        }
    }
}
