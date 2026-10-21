use latex_lsp::{
    editing, features, semantic,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str, version: i32) -> Url {
    let uri = Url::parse(&format!("file:///parenthesized-destinations/{name}")).unwrap();
    workspace.documents.insert(
        uri.clone(),
        Document::new(&uri, source.into(), Some(version)),
    );
    uri
}
fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));
    Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .map(|e| TextDocumentContentChangeEvent {
                    range: Some(e.range),
                    range_length: None,
                    text: e.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
}
fn links(category: &str, key: &str, full: &str, display: &str) -> String {
    format!("\\hyperlink{{{full}}}{{Display {display}}}\\hyperref{{}}{{{category}}}% keep link\r\n{{{key}}}{{Display {display}}}")
}

#[test]
fn parenthesized_pdf_destinations_rename_across_simple_compound_and_bookmark_forms() {
    for old in ["dest(old)", "dest(old", "dest)old"] {
        let fixtures = [
            (old.to_owned(), format!("\\hypertarget{{{old}}}{{Display {old}}}"), links("", old, old, old), "dest(new)".to_owned(), format!("\\hypertarget{{dest(new)}}{{Display {old}}}"), links("", "dest(new)", "dest(new)", old)),
            (format!("category.{old}"), format!("\\hyperdef{{category}}% keep declaration\r\n{{{old}}}{{Display category.{old}}}"), links("category", old, &format!("category.{old}"), &format!("category.{old}")), "category.dest(new)".to_owned(), format!("\\hyperdef{{category}}% keep declaration\r\n{{dest(new)}}{{Display category.{old}}}"), links("category", "dest(new)", "category.dest(new)", &format!("category.{old}"))),
            (format!("{old}.item"), format!("\\hyperdef{{{old}}}% keep declaration\r\n{{item}}{{Display {old}.item}}"), links(old, "item", &format!("{old}.item"), &format!("{old}.item")), "new(cat).item".to_owned(), format!("\\hyperdef{{}}% keep declaration\r\n{{new(cat).item}}{{Display {old}.item}}"), links("", "new(cat).item", "new(cat).item", &format!("{old}.item"))),
            (format!("{old}.2"), format!("\\pdfbookmark[2]{{Title {old}.2}}% keep declaration\r\n{{{old}}}"), links("", &format!("{old}.2"), &format!("{old}.2"), &format!("{old}.2")), "dest(new).1".to_owned(), format!("\\pdfbookmark[1]{{Title {old}.2}}% keep declaration\r\n{{dest(new)}}"), links("", "dest(new).1", "dest(new).1", &format!("{old}.2"))),
        ];
        for (full, declaration, calls, new_name, expected_declaration, expected_calls) in fixtures {
            let mut workspace = Workspace {
                versioned_edits: true,
                ..Default::default()
            };
            let protected_declaration =
                format!("\\label{{{full}}}\\label{{{new_name}}} Literal {full}");
            let definitions = format!("% 😀\r\n{declaration}{protected_declaration}");
            let protected = format!(" Literal {full}\r\n% {full}\r\n\\verb|{full}|\\ref{{{full}}}");
            let source = format!("😀 {calls}{protected}");
            let target = insert(&mut workspace, "targets.tex", &definitions, 11);
            let main = insert(&mut workspace, "main.tex", &source, 7);
            let position = Text::new(source.clone()).position(source.find(&full).unwrap() + 1);
            let found = features::definitions(&workspace, &main, position);
            assert_eq!(found.len(), 1, "{declaration}");
            assert_eq!(found[0].uri, target);
            assert!(features::hover(&workspace, &main, position).is_some());
            assert_eq!(
                features::references(&workspace, &main, position, true).len(),
                3
            );
            assert!(
                matches!(features::prepare_rename(&workspace, &main, position).unwrap().unwrap(), PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } if placeholder == full)
            );
            let Some(DocumentChanges::Edits(changes)) =
                features::rename(&workspace, &main, position, &new_name)
                    .unwrap()
                    .unwrap()
                    .document_changes
            else {
                panic!()
            };
            assert_eq!(changes.len(), 2);
            for change in changes {
                let uri = change.text_document.uri;
                assert_eq!(
                    change.text_document.version,
                    Some(if uri == main { 7 } else { 11 })
                );
                let edits = change
                    .edits
                    .into_iter()
                    .map(|e| {
                        let OneOf::Left(e) = e else { panic!() };
                        e
                    })
                    .collect::<Vec<_>>();
                let expected = if uri == main {
                    format!("😀 {expected_calls}{protected}")
                } else {
                    assert_eq!(uri, target);
                    format!("% 😀\r\n{expected_declaration}{protected_declaration}")
                };
                assert_eq!(
                    apply(&workspace.documents[&uri].text.source, &edits),
                    expected
                );
            }
        }
    }
}

#[test]
fn parenthesized_destination_completion_and_semantics_keep_complete_names() {
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "targets.tex",
        r"\hypertarget{dest(target)}{Target}",
        11,
    );
    for marked in [r"\hyperlink{dest(t¦BAD)}{Text}", r"\hyperlink{dest(t¦"] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source, 7);
        let item = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        )
        .into_iter()
        .find(|i| i.label == "dest(target)")
        .unwrap_or_else(|| panic!("{marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        let updated = apply(&source, &edits);
        assert!(updated.contains("dest(target)"));
        assert!(!updated.contains("BAD"));
    }
    let source = "😀 \\hypertarget{dest(target)}{Target}\\hyperlink{dest(target)}{Link}\\hyperdef{cat(old)}{key)}{Target}\\hyperref{}{cat(old)}{key)}{Link}\\pdfbookmark[2]{Title}{key(index)}\\hyperlink{key(index).2}{Link}";
    let main = insert(&mut workspace, "semantics.tex", source, 7);
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
            keys.push(
                &source[doc.text.offset(position).unwrap()
                    ..doc
                        .text
                        .offset(Position::new(
                            position.line,
                            position.character + token.length,
                        ))
                        .unwrap()],
            );
        }
    }
    assert_eq!(
        keys,
        [
            "dest(target)",
            "dest(target)",
            "cat(old)",
            "key)",
            "cat(old)",
            "key)",
            "key(index)",
            "key(index).2"
        ]
    );
    assert!(features::symbols(doc)
        .iter()
        .any(|s| s.name.contains("key(index).2")));
}

#[test]
fn parenthesized_destinations_keep_conflicts_dynamic_arguments_and_bookmark_level_guards() {
    let mut workspace = Workspace::default();
    let source = r"\hypertarget{dest(old)}{Target}\hypertarget{dest(used)}{Other}\hyperlink{dest(old)}{Link}";
    let main = insert(&mut workspace, "main.tex", source, 7);
    let at = Text::new(source.into()).position(source.rfind("dest(old)").unwrap() + 1);
    for invalid in [
        "dest(used)",
        "bad name",
        "bad=key",
        "a,b",
        "a%comment",
        "\\chosen",
        "a{b}",
    ] {
        assert!(
            features::rename(&workspace, &main, at, invalid).is_err(),
            "{invalid}"
        );
    }
    for source in [
        r"\hypertarget{ dest(old)}{Target}",
        r"\hyperlink{dest(old) }{Link}",
        "\\hyperlink{dest(o% splice\nld)}{Link}",
        r"\hyperlink{\chosen(old)}{Link}",
    ] {
        let uri = insert(&mut workspace, "dynamic.tex", source, 7);
        let position = Text::new(source.into()).position(source.find('{').unwrap() + 1);
        assert!(
            features::prepare_rename(&workspace, &uri, position)
                .unwrap()
                .is_none(),
            "{source}"
        );
    }
    let source = r"\pdfbookmark[2]{Title}{book(old)}\hyperlink{book(old).2}{Link}";
    let main = insert(&mut workspace, "bookmarks.tex", source, 7);
    let at = Text::new(source.into()).position(source.rfind("book(old)").unwrap() + 1);
    for invalid in [
        "plain(paren)",
        ".1",
        "new(paren).invalid",
        "new(paren).-4",
        "new(paren).2147483648",
    ] {
        assert!(
            features::rename(&workspace, &main, at, invalid).is_err(),
            "{invalid}"
        );
    }
    for declaration in [
        r"\newcommand{\macro}{}",
        r"\newenvironment{panel}{}{}",
        r"\newcounter{count}",
    ] {
        let uri = insert(&mut workspace, "other.tex", declaration, 7);
        let name = if declaration.contains("macro") {
            "macro"
        } else if declaration.contains("panel") {
            "panel"
        } else {
            "count"
        };
        let offset = if name == "macro" {
            declaration.find(name).unwrap()
        } else {
            declaration.find(&format!("{{{name}}}")).unwrap() + 1
        };
        let at = Text::new(declaration.into()).position(offset + 1);
        assert!(features::rename(&workspace, &uri, at, "name(paren)").is_err());
    }
}

#[test]
fn parenthesized_pdf_destination_spelling_fixes_preserve_compound_comments_and_labels() {
    for (declaration, link, missing, expected_link) in [
        (
            r"\hypertarget{dest(target)}{Target}",
            r"\hyperlink{dest(targte)}{Link}",
            "dest(targte)",
            r"\hyperlink{dest(target)}{Link}",
        ),
        (
            r"\hyperdef{cat(old)}{target}{Target}",
            "\\hyperref{}{cat(old)}% keep\r\n{targte}{Link}",
            "cat(old).targte",
            "\\hyperref{}{cat(old)}% keep\r\n{target}{Link}",
        ),
        (
            r"\pdfbookmark[2]{Title}{dest(target)}",
            r"\hyperlink{dest(targte).2}{Link}",
            "dest(targte).2",
            r"\hyperlink{dest(target).2}{Link}",
        ),
        (
            r"\hypertarget{章节.target}{Target}",
            r"\hyperlink{章节.targte}{Link}",
            "章节.targte",
            r"\hyperlink{章节.target}{Link}",
        ),
        (
            r"\hyperdef{emoji😀}{target}{Target}",
            "\\hyperref{}{emoji😀}% keep\r\n{targte}{Link}",
            "emoji😀.targte",
            "\\hyperref{}{emoji😀}% keep\r\n{target}{Link}",
        ),
        (
            r"\pdfbookmark[2]{Title}{书签(😀).target}",
            r"\hyperlink{书签(😀).targte.2}{Link}",
            "书签(😀).targte.2",
            r"\hyperlink{书签(😀).target.2}{Link}",
        ),
    ] {
        let encoded: String = missing
            .bytes()
            .map(|byte| match byte {
                b'(' => "\\(".to_owned(),
                b')' => "\\)".to_owned(),
                128..=255 => format!("\\{byte:03o}"),
                _ => char::from(byte).to_string(),
            })
            .collect();
        for message in [
            format!("warning  (pdf backend): unreferenced destination with name '{missing}'"),
            format!("pdfTeX warning (dest): name{{{encoded}}} has been referenced but does not exist, replaced by a fixed one"),
        ] {
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        let source = format!("{declaration}\n😀 {link}\\label{{{missing}}}");
        let main = insert(&mut workspace, "main.tex", &source, 7);
        let doc = &workspace.documents[&main];
        let entry = &doc.syntax.anchor_references[0];
        assert_eq!(entry.name, missing);
        let diagnostic = Diagnostic {
            range: doc.text.range(entry.span.start, entry.span.end),
            source: Some("latex-build".into()),
            code: Some(NumberOrString::String("undefined-pdf-destination".into())),
            data: Some(serde_json::json!({"destination":missing})),
            message,
            ..Default::default()
        };
        workspace.build_diagnostics.insert(
            main.clone(),
            [(main.clone(), vec![diagnostic.clone()])].into(),
        );
        let actions = editing::code_actions(
            &workspace,
            &main,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                only: None,
                trigger_kind: None,
            },
        );
        let edits = actions
            .into_iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(a) => a.edit,
                _ => None,
            })
            .flat_map(|e| {
                e.changes
                    .unwrap_or_default()
                    .remove(&main)
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        assert_eq!(edits.len(), 1, "{edits:?}");
        assert_eq!(
            apply(&source, &edits),
            format!("{declaration}\n😀 {expected_link}\\label{{{missing}}}")
        );
        }
    }
}
