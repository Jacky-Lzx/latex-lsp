use latex_lsp::{
    build_log, editing, features, semantic,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///parenthesized-citations/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    uri
}
fn at(source: &str, needle: &str) -> Position {
    Text::new(source.into()).position(source.find(needle).unwrap() + 1)
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

#[test]
fn literal_bibtex_headers_keep_parentheses_and_field_boundaries() {
    for key in ["book(old)", "book(old", "book)old", "(中文)", "(", ")"] {
        for (open, close) in [('{', '}'), ('(', ')')] {
            let source = format!("% 😀\r\n@book{open}{key}, title={{Title @book{{fake,}} (literal)}}, author=\"A. Author\", year=2026{close}\r\n@article{{next, crossref={{{key}}}}}\r\n");
            let mut workspace = Workspace::default();
            let bib = insert(&mut workspace, "refs.bib", &source);
            let doc = &workspace.documents[&bib];
            assert_eq!(
                doc.syntax
                    .citations
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>(),
                [key, "next"],
                "{source}"
            );
            let first = &doc.syntax.citations[0];
            assert_eq!(&source[first.span.clone()], key);
            for expected in [
                "Title: Title @book{fake,} (literal)",
                "Author: A. Author",
                "Year: 2026",
            ] {
                assert!(first.detail.contains(expected), "{}", first.detail);
            }
            assert_eq!(doc.syntax.citation_references[0].name, key);
            assert_eq!(
                features::definitions(&workspace, &bib, at(&source, &format!("{key}}}}}"))).len(),
                1
            );
        }
    }
}

#[test]
fn unproven_headers_and_special_entries_keep_conservative_recovery() {
    for malformed in [
        "@book(bad)",
        "@book(bad)\r\n",
        "@book( bad name )",
        "@book(bad% comment\n)",
        "@book(\"bad)\")",
        "@book(bad={value})",
    ] {
        let source = format!("{malformed}\n@article{{next, title={{Kept}}}}\n");
        let mut workspace = Workspace::default();
        let bib = insert(&mut workspace, "refs.bib", &source);
        assert_eq!(
            workspace.documents[&bib]
                .syntax
                .citations
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["next"],
            "{source}"
        );
    }
    for source in [
        "@comment(ignored) @book{next, title={Kept}}",
        "@string(foo=\"ignored) @book{fake,}\") @book{next, title={Kept}}",
        "@preamble{\"@book(fake, title={Ignored})\"} @book{next, title={Kept}}",
        "@comment{ignored @book(fake,)} @book(next, title={Kept})",
    ] {
        let mut workspace = Workspace::default();
        let bib = insert(&mut workspace, "refs.bib", source);
        assert_eq!(
            workspace.documents[&bib]
                .syntax
                .citations
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["next"],
            "{source}"
        );
    }
}

#[test]
fn citation_variants_rename_exact_keys_across_declarations_and_bibtex_fields() {
    for old in ["book(old)", "book(old", "book)old", "(中文)"] {
        for declaration in [
            format!("\\bibitem[Display]{{% keep\r\n{old}% keep\r\n}} Body"),
            format!("@book({old}, title={{Metadata}}, year=2026)"),
            format!("@book{{{old}, title={{Metadata}}, year=2026}} "),
        ] {
            for call in [
                format!("\\cite[Note]{{other, % keep\r\n{old}% tail\r\n}}"),
                format!("\\citet[Before][After]{{{old}}}"),
                format!("\\nocite{{{old}}}"),
                format!("\\bibentry{{% keep\r\n{old}% tail\r\n}}"),
                format!("\\defcitealias{{{old}}}{{Display}}"),
                format!("\\citetalias{{{old}}}"),
            ] {
                let mut workspace = Workspace {
                    versioned_edits: true,
                    ..Default::default()
                };
                let name = if declaration.starts_with('@') {
                    "refs.bib"
                } else {
                    "refs.tex"
                };
                let bib = insert(&mut workspace, name, &declaration);
                let source =
                    format!("% 😀\r\n{call}\r\nLiteral {old}\r\n% {old}\r\n\\verb|{old}|\r\n");
                let main = insert(&mut workspace, "main.tex", &source);
                let fields = format!("@article{{child, title={{{old}}}, crossref={{{old}}}, xref=\"{old}\", xdata={{other, {old}}}, related=\"{old}, other\", entryset={{{old}}}}}");
                let child = insert(&mut workspace, "child.bib", &fields);
                let pos = at(&source, old);
                assert_eq!(
                    features::definitions(&workspace, &main, pos)
                        .iter()
                        .map(|d| &d.uri)
                        .collect::<Vec<_>>(),
                    [&bib],
                    "{call}: {declaration}"
                );
                assert!(features::hover(&workspace, &main, pos).is_some());
                assert!(features::prepare_rename(&workspace, &main, pos)
                    .unwrap()
                    .is_some());
                assert_eq!(
                    features::references(&workspace, &main, pos, true).len(),
                    7,
                    "{call}"
                );
                let edit = features::rename(&workspace, &main, pos, "book(renamed)")
                    .unwrap()
                    .unwrap();
                let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
                    panic!("missing versioned edits")
                };
                assert_eq!(changes.len(), 3);
                for change in changes {
                    assert_eq!(change.text_document.version, Some(7));
                    let uri = change.text_document.uri;
                    let original = &workspace.documents[&uri].text;
                    let edits = change
                        .edits
                        .into_iter()
                        .map(|e| {
                            let OneOf::Left(e) = e else { panic!() };
                            e
                        })
                        .collect::<Vec<_>>();
                    for e in &edits {
                        assert_eq!(
                            &original.source[original.offset(e.range.start).unwrap()
                                ..original.offset(e.range.end).unwrap()],
                            old
                        );
                    }
                    let expected = if uri == main {
                        source.replacen(old, "book(renamed)", 1)
                    } else if uri == bib {
                        declaration.replacen(old, "book(renamed)", 1)
                    } else {
                        assert_eq!(uri, child);
                        fields
                            .replace(&format!("{{{old}}}, crossref"), "{PROTECTED}, crossref")
                            .replace(old, "book(renamed)")
                            .replace("{PROTECTED}", &format!("{{{old}}}"))
                    };
                    assert_eq!(apply(&original.source, &edits), expected);
                }
            }
        }
    }
}

#[test]
fn citation_completion_and_semantics_use_complete_keys_and_preserve_comments() {
    let key = "book(target)";
    for marked in [
        "\\cite[Note]{other, % keep\r\nbook(t¦BAD) % tail\r\n}",
        "\\bibentry{% keep\r\n book(t¦BAD) % tail\r\n}",
        "\\defcitealias{book(t¦BAD)}{Display}",
        "\\citetalias{book(t¦BAD)}",
        "\\cite{book(t¦",
        "@article{child, crossref={book(t¦BAD)}}",
        "@article(child, related=\"other, book(t¦BAD), last\")",
    ] {
        let mut workspace = Workspace::default();
        insert(
            &mut workspace,
            "refs.bib",
            &format!("@book({key}, title={{Metadata}})"),
        );
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(
            &mut workspace,
            if source.starts_with('@') {
                "child.bib"
            } else {
                "main.tex"
            },
            &source,
        );
        let item = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        )
        .into_iter()
        .find(|i| i.label == key)
        .unwrap_or_else(|| panic!("{marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        let updated = apply(&source, &edits);
        assert!(updated.contains(key), "{updated}");
        assert!(!updated.contains("BAD"), "{updated}");
        for comment in ["% keep\r\n", "% tail\r\n"] {
            if source.contains(comment) {
                assert!(updated.contains(comment));
            }
        }
    }
    for (name, source, expected) in [
        ("main.tex", format!("% 😀\r\n\\bibitem{{{key}}} Text \\cite{{{key},other}}\\bibentry{{{key}}}\\defcitealias{{{key}}}{{Display}}"), vec![key, key, "other", key, key]),
        ("refs.bib", format!("@book({key}, title={{Metadata}})\r\n@article{{child, crossref={{{key}}}}}"), vec![key, "child", key]),
    ] {
        let mut workspace = Workspace::default(); let uri = insert(&mut workspace, name, &source); let doc = &workspace.documents[&uri];
        let legend = semantic::options().legend; let mut pos = Position::default(); let mut keys = Vec::new();
        for token in semantic::tokens(doc, None).unwrap().data {
            pos = Position::new(pos.line + token.delta_line, if token.delta_line == 0 { pos.character + token.delta_start } else { token.delta_start });
            if legend.token_types[token.token_type as usize].as_str() == "enumMember" {
                let end = Position::new(pos.line, pos.character + token.length);
                keys.push(&source[doc.text.offset(pos).unwrap()..doc.text.offset(end).unwrap()]);
            }
        }
        assert_eq!(keys, expected, "{source}");
        assert!(features::symbols(doc).iter().any(|s| s.name.contains(key)));
    }
}

#[test]
fn citation_rename_keeps_conflict_dynamic_and_wildcard_guards() {
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "refs.bib",
        "@book(book(old), title={Old})\n@book(book(used), title={Used})",
    );
    let main = insert(&mut workspace, "main.tex", r"\cite{book(old)}");
    for invalid in [
        "book(used)",
        "*",
        "bad name",
        "bad=key",
        "a,b",
        "a%comment",
        "a{b}",
        "\\chosen",
    ] {
        assert!(
            features::rename(&workspace, &main, Position::new(0, 8), invalid).is_err(),
            "{invalid}"
        );
    }
    for call in [
        r"\bibentry{ book(old)}",
        r"\bibentry{book(old) }",
        r"\cite{\chosen(old)}",
        "\\cite{book(o% splice\nld)}",
        r"\cite{{book(old)}}",
    ] {
        let main = insert(&mut workspace, "uses.tex", call);
        let needle = if call.contains("book") {
            "book"
        } else {
            "chosen"
        };
        assert!(
            features::prepare_rename(&workspace, &main, at(call, needle))
                .unwrap()
                .is_none(),
            "{call}"
        );
    }
}

#[test]
fn citation_spelling_fixes_preserve_compiler_ranges_for_parenthesized_keys() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "\\bibitem{book(target)} Body\n😀 \\cite{other, % keep\nbook(targte)% tail\n}\n";
    std::fs::write(&main, source).unwrap();
    let diagnostics = build_log::parse(&main, "(./main.tex\nLaTeX Warning: Citation `book(targte)' on page 1 undefined on input line 4.\n)\n");
    let diagnostic = diagnostics[&uri][0].clone();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.open(uri.clone(), source.into(), 7);
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
        .filter_map(|a| match a {
            CodeActionOrCommand::CodeAction(a) => a.edit,
            _ => None,
        })
        .flat_map(|e| {
            e.changes
                .unwrap_or_default()
                .remove(&uri)
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    assert_eq!(edits.len(), 1, "{edits:?}");
    assert_eq!(
        apply(source, &edits),
        source.replace("book(targte)", "book(target)")
    );
}

#[test]
fn parenthesized_citation_rename_respects_independent_projects_and_shared_uses() {
    for shared in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("shared.tex"), r"\cite{book(old)}").unwrap();
        for name in ["a", "b"] {
            std::fs::create_dir(root.join(name)).unwrap();
            let included = if shared { "../shared" } else { "child" };
            std::fs::write(
                root.join(format!("{name}/main.tex")),
                format!("\\documentclass{{article}}\\bibliography{{refs}}\\input{{{included}}}"),
            )
            .unwrap();
            std::fs::write(root.join(format!("{name}/child.tex")), r"\cite{book(old)}").unwrap();
            std::fs::write(
                root.join(format!("{name}/refs.bib")),
                format!(
                    "@book(book(old), title={{Old}}){}",
                    if name == "b" {
                        "\n@book(book(new), title={Used})"
                    } else {
                        ""
                    }
                ),
            )
            .unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let other = Url::from_file_path(root.join("b/main.tex")).unwrap();
        workspace.refresh_project(&other);
        let main = Url::from_file_path(root.join("a/main.tex")).unwrap();
        workspace.refresh_project(&main);
        let bib = Url::from_file_path(root.join("a/refs.bib")).unwrap();
        let doc = &workspace.documents[&bib];
        let result = features::rename(
            &workspace,
            &bib,
            at(&doc.text.source, "book(old)"),
            "book(new)",
        );
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
fn multiline_bibtex_headers_keep_metadata_navigation_and_exact_rename_ranges() {
    for old in ["ordinary", "book(old)", "book(old", "book)old", "(中文)"] {
        for gap in ["\n", "\r\n  ", " \t\n\t", "\n\n"] {
            for (open, close) in [('{', '}'), ('(', ')')] {
                let mut workspace = Workspace {
                    versioned_edits: true,
                    ..Default::default()
                };
                let source = format!("% 😀\r\n@book{open}{old}{gap},title={{Literal {old}}}, author={{A. Author}}, year=2026{close}\r\n@article{{child, crossref={{{old}}}}}");
                let bib = insert(&mut workspace, "refs.bib", &source);
                let main_source = format!("\\cite{{{old}}}");
                let main = insert(&mut workspace, "main.tex", &main_source);
                let doc = &workspace.documents[&bib];
                assert_eq!(
                    doc.syntax
                        .citations
                        .iter()
                        .map(|e| e.name.as_str())
                        .collect::<Vec<_>>(),
                    [old, "child"],
                    "{source}"
                );
                let entry = &doc.syntax.citations[0];
                assert_eq!(&source[entry.span.clone()], old);
                for expected in [
                    format!("Title: Literal {old}"),
                    "Author: A. Author".into(),
                    "Year: 2026".into(),
                ] {
                    assert!(entry.detail.contains(&expected), "{}", entry.detail);
                }
                let pos = at(&main_source, old);
                let definitions = features::definitions(&workspace, &main, pos);
                assert_eq!(definitions.len(), 1);
                assert_eq!(definitions[0].uri, bib);
                assert_eq!(
                    definitions[0].range,
                    doc.text.range(entry.span.start, entry.span.end)
                );
                assert!(features::hover(&workspace, &main, pos).is_some());
                assert_eq!(features::references(&workspace, &main, pos, true).len(), 3);
                let edit = features::rename(&workspace, &main, pos, "book(next)")
                    .unwrap()
                    .unwrap();
                let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
                    panic!()
                };
                assert_eq!(changes.len(), 2);
                for change in changes {
                    assert_eq!(change.text_document.version, Some(7));
                    let uri = change.text_document.uri;
                    let original = &workspace.documents[&uri].text;
                    let edits = change
                        .edits
                        .into_iter()
                        .map(|e| {
                            let OneOf::Left(e) = e else { panic!() };
                            e
                        })
                        .collect::<Vec<_>>();
                    for e in &edits {
                        assert_eq!(
                            &original.source[original.offset(e.range.start).unwrap()
                                ..original.offset(e.range.end).unwrap()],
                            old
                        );
                    }
                    let expected = if uri == main {
                        "\\cite{book(next)}".into()
                    } else {
                        assert_eq!(uri, bib);
                        format!("% 😀\r\n@book{open}book(next){gap},title={{Literal {old}}}, author={{A. Author}}, year=2026{close}\r\n@article{{child, crossref={{book(next)}}}}")
                    };
                    assert_eq!(apply(&original.source, &edits), expected);
                }
            }
        }
    }
}

#[test]
fn multiline_header_proof_rejects_spliced_names_and_recovers_following_entries() {
    for malformed in [
        "@book(broken)\n",
        "@book(broken)\r\n\t",
        "@book(broken\n other, title={Invalid})\n",
        "@book(broken\n% comment\n, title={Invalid})\n",
        "@book(broken\n\\macro, title={Invalid})\n",
        "@book(broken\n{group}, title={Invalid})\n",
        "@book(broken\n=other, title={Invalid})\n",
        "@book(broken)\n@comment{Ignored, @book(fake,)}\n",
    ] {
        let mut workspace = Workspace::default();
        let source = format!("{malformed}@book(next, title={{Kept}})");
        let uri = insert(&mut workspace, "refs.bib", &source);
        assert_eq!(
            workspace.documents[&uri]
                .syntax
                .citations
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["next"],
            "{source}"
        );
    }
    for incomplete in [
        "@book(broken\n",
        "@book{broken\r\n  ",
        "@book(broken\n% comment\n",
    ] {
        let mut workspace = Workspace::default();
        let uri = insert(&mut workspace, "refs.bib", incomplete);
        assert!(workspace.documents[&uri].syntax.citations.is_empty());
    }
}

#[test]
fn bibtex_warnings_resolve_multiline_headers_to_the_complete_source_key() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("refs.bib");
    let bib_uri = Url::from_file_path(&bib).unwrap();
    std::fs::write(&main, "\\documentclass{article}").unwrap();
    for key in ["ordinary", "book(old)", "book(old", "book)old", "(中文)"] {
        for gap in ["\n", "\r\n  "] {
            for (open, close) in [('{', '}'), ('(', ')')] {
                let source = format!("% 😀\r\n@book{open}{key}{gap},title={{Example}},publisher={{Press}},year=2026{close}");
                std::fs::write(&bib, &source).unwrap();
                let diagnostics = build_log::parse_bibliography(
                    &main,
                    &format!(
                        "Database file #1: refs.bib\nWarning--empty author and editor in {key}\n"
                    ),
                );
                assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
                let diagnostic = &diagnostics[&bib_uri][0];
                let start = source.find(key).unwrap();
                assert_eq!(
                    diagnostic.range,
                    Text::new(source.clone()).range(start, start + key.len()),
                    "{source}"
                );
                assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::WARNING));
            }
        }
    }
}

#[test]
fn whitespace_after_bibtex_at_keeps_entries_metadata_and_rename_ranges() {
    for gap in [" ", "\t", "\n", "\r\n  "] {
        for (open, close) in [('{', '}'), ('(', ')')] {
            for key in ["ordinary", "book(old)", "book(old", "book)old", "(中文)"] {
                let mut workspace = Workspace::default();
                let source = format!("% 😀\r\n@{gap}Book{open}{key}\r\n  , title={{Literal {key}}}, author={{A. Author}}, year=2026{close}\n@{gap}article{{child, crossref={{{key}}}}}");
                let bib = insert(&mut workspace, "refs.bib", &source);
                let main_source = format!("\\cite{{{key}}}");
                let main = insert(&mut workspace, "main.tex", &main_source);
                let doc = &workspace.documents[&bib];
                assert_eq!(
                    doc.syntax
                        .citations
                        .iter()
                        .map(|e| e.name.as_str())
                        .collect::<Vec<_>>(),
                    [key, "child"],
                    "{source}"
                );
                assert!(doc.syntax.citations[0]
                    .detail
                    .contains(&format!("Title: Literal {key}")));
                let position = at(&main_source, key);
                let definition = features::definitions(&workspace, &main, position);
                assert_eq!(definition.len(), 1);
                assert_eq!(definition[0].uri, bib);
                assert_eq!(
                    features::references(&workspace, &main, position, true).len(),
                    3
                );
                let changes = features::rename(&workspace, &main, position, "book(next)")
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(changes.len(), 2);
                assert_eq!(apply(&main_source, &changes[&main]), "\\cite{book(next)}");
                let expected = format!("% 😀\r\n@{gap}Book{open}book(next)\r\n  , title={{Literal {key}}}, author={{A. Author}}, year=2026{close}\n@{gap}article{{child, crossref={{book(next)}}}}");
                assert_eq!(apply(&source, &changes[&bib]), expected);
            }
        }
    }
}

#[test]
fn spaced_special_bibtex_entries_remain_opaque_and_comment_ranges_start_at_at_sign() {
    for gap in [" ", "\n", "\r\n\t"] {
        for (open, close) in [('{', '}'), ('(', ')')] {
            let comment =
                format!("@{gap}CoMmEnT{open}Ignored @book{{fake, title={{Hidden}}}}{close}");
            let source = format!("% prefix\n{comment}\n@{gap}string{{macro=\"@book(fake, title={{Hidden}})\"}}\n@{gap}preamble{{\"@book{{fake, title={{Hidden}}}}\"}}\n@{gap}{{invalid, title={{@book{{fake,}}}}}}\n@{gap}book{{real, title={{Kept @article{{fake,}}}}}}");
            let mut workspace = Workspace::default();
            let uri = insert(&mut workspace, "refs.bib", &source);
            let doc = &workspace.documents[&uri];
            assert_eq!(
                doc.syntax
                    .citations
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>(),
                ["real"],
                "{source}"
            );
            let span = doc
                .syntax
                .comments
                .iter()
                .find(|span| source[(*span).clone()].starts_with('@'))
                .unwrap();
            assert_eq!(&source[span.clone()], comment);
            assert!(doc.syntax.ignored.contains(span));
            assert!(doc.syntax.citations[0]
                .detail
                .contains("Title: Kept @article{fake,}"));
        }
    }
}
