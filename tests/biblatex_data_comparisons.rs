use latex_lsp::{
    language::{features, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

fn document(workspace: &mut Workspace, source: &str) -> Url {
    let uri = Url::parse("file:///data-comparisons/main.tex").unwrap();
    workspace.open(uri.clone(), source.into(), 1);
    uri
}

fn complete(marked: &str, candidate: &str) -> String {
    let source = marked.replace('¦', "");
    let mut workspace = Workspace::default();
    let uri = document(&mut workspace, &source);
    let text = &workspace.documents[&uri].text;
    let at = text.position(marked.find('¦').unwrap());
    let item = features::completion(&workspace, &uri, at)
        .into_iter()
        .find(|item| item.label == candidate && item.kind == Some(CompletionItemKind::FIELD))
        .unwrap_or_else(|| panic!("{marked}"));
    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
        panic!()
    };
    assert_eq!(edit.range.start.line, at.line);
    assert_eq!(edit.range.end.line, at.line);
    let mut edits = item.additional_text_edits.unwrap_or_default();
    edits.push(edit);
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    text.changed(
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
fn comparison_names_have_typed_completion_hover_and_four_parameter_signatures() {
    for (name, first, second, label) in [
        ("iffieldequalstr", "title", "A title", "field}{string"),
        ("iffieldequals", "title", "\\saved", "field}{command"),
        ("iffieldsequal", "title", "year", "field 1}{field 2"),
        (
            "iflistequals",
            "publisher",
            "\\saved",
            "literal list}{command",
        ),
        ("ifnameequals", "author", "\\saved", "name list}{command"),
    ] {
        let mut workspace = Workspace::default();
        let bib = Url::parse("file:///data-comparisons/refs.bib").unwrap();
        workspace.documents.insert(
            bib.clone(),
            Document::new(
                &bib,
                format!("@book{{{first},title={{Book}}}} @book{{year,title={{Year}}}}"),
                Some(1),
            ),
        );
        let source = format!("% 😀\r\n\\{name}{{{first}}}{{{second}}}{{True text}}{{False text}}");
        let uri = document(&mut workspace, &source);
        let text = &workspace.documents[&uri].text;
        let at = text.position(source.find(&format!("{{{first}}}")).unwrap() + 3);
        let items = features::completion(&workspace, &uri, at);
        assert!(
            items
                .iter()
                .all(|item| item.kind == Some(CompletionItemKind::FIELD)),
            "{name}: {items:?}"
        );
        assert!(items.iter().any(|item| item.label == first));
        for other in ["title", "publisher", "author"] {
            assert_eq!(items.iter().any(|item| item.label == other), other == first);
        }
        assert!(features::hover(&workspace, &uri, at).is_some());
        assert!(features::definitions(&workspace, &uri, at).is_empty());
        assert!(features::references(&workspace, &uri, at, true).is_empty());
        assert!(features::prepare_rename(&workspace, &uri, at)
            .unwrap()
            .is_none());
        let help = signature::help(&workspace, &uri, at).unwrap();
        assert_eq!(help.active_parameter, Some(0));
        assert_eq!(
            help.signatures[0].label,
            format!("\\{name}{{{label}}}{{true}}{{false}}")
        );
        for (marker, parameter) in [(second, 1), ("True text", 2), ("False text", 3)] {
            let at = text.position(source.find(&format!("{{{marker}}}")).unwrap() + 3);
            assert_eq!(
                signature::help(&workspace, &uri, at)
                    .unwrap()
                    .active_parameter,
                Some(parameter),
                "{source}"
            );
            let field = parameter == 1 && name == "iffieldsequal";
            assert_eq!(
                features::completion(&workspace, &uri, at)
                    .iter()
                    .any(|item| item.kind == Some(CompletionItemKind::FIELD)),
                field,
                "{source}: {marker}"
            );
            if marker != "\\saved" {
                assert_eq!(features::hover(&workspace, &uri, at).is_some(), field);
            }
        }
    }
}

#[test]
fn both_field_arguments_complete_without_changing_other_operands_or_comments() {
    for (marked, expected) in [
        ("\\iffieldsequal{TI¦BAD}{year}{yes}{no}", "\\iffieldsequal{title}{year}{yes}{no}"),
        ("\\iffieldsequal{year}{TI¦BAD}{yes}{no}", "\\iffieldsequal{year}{title}{yes}{no}"),
        ("% 😀\r\n\\iffieldsequal{\\dynamic}% between\r\n{% before\r\n TI¦BAD % after\r\n}{yes}{no}", "% 😀\r\n\\iffieldsequal{\\dynamic}% between\r\n{% before\r\ntitle% after\r\n}{yes}{no}"),
        ("\\iffieldsequal{{dynamic} value}{TI¦", "\\iffieldsequal{{dynamic} value}{title"),
        ("\\iffieldsequal{year}{TI¦% after\r\n}", "\\iffieldsequal{year}{title% after\r\n}"),
        ("\\iffieldequalstr{% before\r\n TI¦BAD % after\r\n}{title}{yes}{no}", "\\iffieldequalstr{% before\r\ntitle% after\r\n}{title}{yes}{no}"),
    ] {
        assert_eq!(complete(marked, "title"), expected, "{marked}");
    }
}

#[test]
fn comparison_literals_branches_invalid_calls_and_command_operands_do_not_get_field_names() {
    for marked in [
        "\\iffieldequalstr{title}{ti¦tle}{yes}{no}",
        "\\iffieldequals{title}{ti¦tle}{yes}{no}",
        "\\iflistequals{publisher}{pu¦blisher}{yes}{no}",
        "\\ifnameequals{author}{au¦thor}{yes}{no}",
        "\\iffieldsequal{title}{year}{ti¦tle}{no}",
        "\\iffieldsequal{title}{year}{yes}{ti¦tle}",
        "\\iffieldsequal{title}{year}{yes}{no}{ti¦tle}",
        "\\iffieldsequal{title}\n\n{ti¦tle}",
        "\\iffieldsequal*{title}{ti¦tle}",
        "\\iffieldsequal[fmt]{ti¦tle}{year}",
        "\\iffieldsequal{title} prose {ti¦tle}",
        "\\iffieldsequal{title}{ti¦tle,year}",
        "\\iffieldsequal{title}{ti¦% joined\r\ntle}",
        "\\iffieldsequal{title}{ti¦tle\\suffix}",
        "\\meaning\\iffieldsequal{title}{ti¦tle}",
        "% \\iffieldsequal{title}{ti¦tle}\r\n",
        "\\verb|\\iffieldsequal{title}{ti¦tle}|",
    ] {
        let mut workspace = Workspace::default();
        let source = marked.replace('¦', "");
        let uri = document(&mut workspace, &source);
        let at = workspace.documents[&uri]
            .text
            .position(marked.find('¦').unwrap());
        assert!(
            !features::completion(&workspace, &uri, at)
                .iter()
                .any(|item| item.kind == Some(CompletionItemKind::FIELD)),
            "{marked}"
        );
        assert!(features::hover(&workspace, &uri, at).is_none(), "{marked}");
    }
}

#[test]
fn nested_selectors_keep_their_own_types_inside_comparison_operands_and_branches() {
    for (marked, candidate) in [
        (
            "\\iffieldequalstr{title}{\\printnames{au¦BAD}}{yes}{no}",
            "author",
        ),
        (
            "\\iffieldsequal{\\printlist{pu¦BAD}}{year}{yes}{no}",
            "publisher",
        ),
        (
            "\\ifnameequals{author}{\\saved}{\\printfield{ti¦BAD}}{no}",
            "title",
        ),
        (
            "\\iffieldequals{title}{\\saved}{yes}{\\iffieldsequal{year}{ti¦BAD}{yes}{no}}",
            "title",
        ),
    ] {
        let original = marked.replace('¦', "");
        let start = marked.find('¦').unwrap() - 2;
        let mut expected = original.clone();
        expected.replace_range(start..start + 5, candidate);
        assert_eq!(complete(marked, candidate), expected, "{marked}");
    }
}

#[test]
fn compared_macro_tokens_do_not_execute_citations_declarations_or_verbatim_commands() {
    let mut workspace = Workspace::default();
    let source = "\\iffieldequals{title}\\cite{alpha}{\\cite{alpha}}\n\\iflistequals{publisher}\\newcommand{\\real}{text}\n\\ifnameequals{author}\\verb{\\label{real}}{no}";
    let uri = document(&mut workspace, source);
    let doc = &workspace.documents[&uri];
    let citations: Vec<_> = doc
        .syntax
        .commands
        .iter()
        .filter(|command| command.name == "cite" && command.argument.is_some())
        .collect();
    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].argument.as_ref().unwrap().value, "alpha");
    assert!(doc.syntax.definitions.is_empty());
    assert_eq!(doc.syntax.labels.len(), 1);
    assert_eq!(doc.syntax.labels[0].name, "real");
    for name in ["cite", "newcommand", "verb"] {
        let start = source.find(&format!("\\{name}")).unwrap();
        assert!(doc.syntax.control_operands.contains(&start));
        let help = signature::help(&workspace, &uri, doc.text.position(start + 3)).unwrap();
        assert_eq!(help.active_parameter, Some(1));
        assert!(help.signatures[0].label.ends_with("{command}{true}{false}"));
    }
}

#[test]
fn comparison_overrides_disable_builtins_and_removing_them_restores_both_fields() {
    for name in [
        "iffieldequalstr",
        "iffieldequals",
        "iffieldsequal",
        "iflistequals",
        "ifnameequals",
    ] {
        let field = match name {
            "iflistequals" => "publisher",
            "ifnameequals" => "author",
            _ => "title",
        };
        for declaration in [
            format!("\\newcommand\\{name}[4]{{}}"),
            format!("\\newif\\{name}"),
            format!("\\newcommand\\custom[4]{{}}\\let\\{name}\\custom"),
        ] {
            let call = format!("\\{name}{{{field}}}{{year}}{{yes}}{{no}}");
            let source = format!("{declaration}{call}");
            let mut workspace = Workspace::default();
            let uri = document(&mut workspace, &source);
            for marker in [field, "year"] {
                let at = workspace.documents[&uri]
                    .text
                    .position(source.rfind(marker).unwrap() + 2);
                assert!(
                    !features::completion(&workspace, &uri, at)
                        .iter()
                        .any(|item| item.kind == Some(CompletionItemKind::FIELD)),
                    "{source}"
                );
                assert!(features::hover(&workspace, &uri, at).is_none(), "{source}");
                if declaration.starts_with("\\newif") {
                    assert!(signature::help(&workspace, &uri, at).is_none(), "{source}");
                }
            }
            let restored = call;
            workspace.open(uri.clone(), restored.clone(), 2);
            let at = Text::new(restored.clone()).position(restored.find(field).unwrap() + 2);
            assert!(features::completion(&workspace, &uri, at)
                .iter()
                .any(|item| item.label == field));
            assert!(features::hover(&workspace, &uri, at).is_some());
            assert_eq!(
                signature::help(&workspace, &uri, at).unwrap().signatures[0]
                    .parameters
                    .as_ref()
                    .unwrap()
                    .len(),
                4
            );
        }
    }
}
