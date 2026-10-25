use latex_lsp::{
    compilation::build_log,
    editing,
    language::{features, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

const COMMANDS: &[&str] = &["ifentryexists", "ifentryinbib"];

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///entry-queries/{name}")).unwrap();
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
fn entry_queries_index_one_key_and_keep_both_branches_inside_the_complete_call() {
    for name in COMMANDS {
        for key in ["alpha", "O\"Connor", "book(old)", "中文😀"] {
            let mut workspace = Workspace::default();
            let bibliography = format!("@book{{{key},title={{Literal {key}}}}}");
            let bib = insert(&mut workspace, "refs.bib", &bibliography);
            let source = format!("% 😀\r\n\\{name}% boundary\r\n{{% before\r\n{key}% after\r\n}}{{Literal {key} \\cite{{{key}}}}}{{Literal {key}}}\\label{{{key}}}\\verb|\\{name}{{{key}}}{{}}{{}}|\r\n");
            let main = insert(&mut workspace, "main.tex", &source);
            let doc = &workspace.documents[&main];
            let start = source.find(&format!("\r\n{key}% after")).unwrap() + 2;
            let range = doc.text.range(start, start + key.len());
            let at = range.start;
            assert_eq!(features::definitions(&workspace, &main, at)[0].uri, bib);
            assert!(features::hover(&workspace, &main, at).is_some());
            assert_eq!(features::references(&workspace, &main, at, true).len(), 3);
            assert_eq!(editing::highlights(&workspace, &main, at).len(), 2);
            assert_eq!(doc.syntax.citation_references.len(), 1);
            assert_eq!(doc.syntax.citation_references[0].name, key);
            let mut selection = editing::selection_ranges(doc, &[at]).unwrap().remove(0);
            let mut ranges = Vec::new();
            loop {
                ranges.push(selection.range);
                let Some(parent) = selection.parent else {
                    break;
                };
                selection = *parent;
            }
            assert!(ranges.contains(&range));
            let call = source.find(&format!("\\{name}")).unwrap()..source.find("\\label").unwrap();
            assert!(
                ranges.contains(&doc.text.range(call.start, call.end)),
                "{source}: {ranges:?}"
            );
            let mut line = 0;
            let mut character = 0;
            let tokens = semantic::tokens(doc, None).unwrap();
            assert!(tokens.data.into_iter().any(|token| {
                character = if token.delta_line == 0 {
                    character + token.delta_start
                } else {
                    token.delta_start
                };
                line += token.delta_line;
                token.token_type == 3
                    && token.token_modifiers_bitset == 0
                    && Position::new(line, character) == at
                    && token.length == key.encode_utf16().count() as u32
            }));
            let changes = features::rename(&workspace, &main, at, "new\"key😀")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes[&main].len(), 2);
            let expected = source
                .replace(&format!("\r\n{key}% after"), "\r\nnew\"key😀% after")
                .replace(&format!("\\cite{{{key}}}"), "\\cite{new\"key😀}");
            assert_eq!(apply(&source, &changes[&main]), expected);
            assert_eq!(
                apply(&bibliography, &changes[&bib]),
                format!("@book{{new\"key😀,title={{Literal {key}}}}}")
            );
        }
    }
}

#[test]
fn entry_queries_complete_single_keys_and_hint_all_three_parameters() {
    for name in COMMANDS {
        for (tail, active) in [
            ("¦", 0),
            ("{al¦pha}{yes}{no}", 0),
            ("{alpha} ¦", 1),
            ("{alpha}{y¦es}{no}", 1),
            ("{alpha}{yes} ¦", 2),
            ("{alpha}{yes}{n¦o}", 2),
            ("{alpha}{yes}{n¦", 2),
        ] {
            let marked = format!("\\{name}{tail}");
            let source = marked.replace('¦', "");
            let mut workspace = Workspace::default();
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main]
                .text
                .position(marked.find('¦').unwrap());
            let help = signature::help(&workspace, &main, at).unwrap_or_else(|| panic!("{marked}"));
            assert_eq!(
                help.signatures[0].label,
                format!("\\{name}{{key}}{{true}}{{false}}")
            );
            assert_eq!(help.active_parameter, Some(active), "{marked}");
        }
        for tail in [
            "{al¦BAD}{yes}{no}",
            "{% before\r\n al¦BAD % after\r\n}{yes}{no}",
            "{% before\r\nal¦",
        ] {
            let marked = format!("😀 \\{name}{tail}");
            let source = marked.replace('¦', "");
            let mut workspace = Workspace::default();
            insert(&mut workspace, "refs.bib", "@book{alpha,title={Alpha}}");
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main]
                .text
                .position(marked.find('¦').unwrap());
            let item = features::completion(&workspace, &main, at)
                .into_iter()
                .find(|item| item.label == "alpha")
                .unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!()
            };
            assert_eq!(edit.range.start.line, at.line);
            assert_eq!(edit.range.end.line, at.line);
            let mut edits = item.additional_text_edits.unwrap_or_default();
            edits.push(edit);
            let expected = marked
                .replace(" al¦BAD ", "alpha")
                .replace("al¦BAD", "alpha")
                .replace("al¦", "alpha");
            assert_eq!(apply(&source, &edits), expected);
        }
        for snippets in [false, true] {
            for tail in ["", "{alpha}{yes}{no}"] {
                let mut workspace = Workspace {
                    snippets,
                    ..Default::default()
                };
                let source = format!("\\{name}{tail}");
                let main = insert(&mut workspace, "main.tex", &source);
                let at = workspace.documents[&main].text.position(name.len() + 1);
                let item = features::completion(&workspace, &main, at)
                    .into_iter()
                    .find(|item| item.label == *name)
                    .unwrap();
                let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                    panic!()
                };
                assert_eq!(
                    edit.new_text,
                    if snippets && tail.is_empty() {
                        format!("{name}{{${{1:key}}}}{{${{2:true}}}}{{${{3:false}}}}")
                    } else {
                        name.to_string()
                    }
                );
            }
        }
        let mut workspace = Workspace::default();
        let source = format!("\\newcommand\\{name}[1]{{}}\\{name}{{custom}}");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(source.find("custom").unwrap() + 2);
        assert_eq!(
            signature::help(&workspace, &main, at).unwrap().signatures[0].label,
            format!("\\{name}{{#1}}")
        );
    }
}

#[test]
fn query_lists_dynamic_keys_stars_notes_and_branch_text_never_become_guessed_references() {
    for name in COMMANDS {
        for tail in [
            "*{al¦pha}{yes}{no}",
            "[option]{al¦pha}{yes}{no}",
            "{al¦pha,beta}{yes}{no}",
            "{al¦pha beta}{yes}{no}",
            "{al¦pha\\suffix}{yes}{no}",
            "{{al¦pha}}{yes}{no}",
            "{al¦% joined\r\npha}{yes}{no}",
            "{beta}{al¦pha}{no}",
            "{beta}{yes}{al¦pha}",
            "{beta}{yes}{no}{al¦pha}",
        ] {
            let marked = format!("\\{name}{tail}");
            let source = marked.replace('¦', "");
            let mut workspace = Workspace::default();
            insert(
                &mut workspace,
                "refs.bib",
                "@book{alpha,title={Alpha}} @book{beta,title={Beta}}",
            );
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main]
                .text
                .position(marked.find('¦').unwrap());
            assert!(
                features::definitions(&workspace, &main, at).is_empty(),
                "{marked}"
            );
            assert!(
                features::prepare_rename(&workspace, &main, at)
                    .unwrap()
                    .is_none(),
                "{marked}"
            );
            assert!(
                !features::completion(&workspace, &main, at)
                    .iter()
                    .any(|item| item.label == "alpha"),
                "{marked}"
            );
            assert!(
                !workspace.documents[&main]
                    .syntax
                    .citation_references
                    .iter()
                    .any(|entry| entry.name == "alpha"),
                "{marked}"
            );
        }
    }
}

#[test]
fn entry_query_alias_renames_observe_project_scope_and_shared_source_ambiguity() {
    for name in COMMANDS {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for project in ["a", "b"] {
            std::fs::write(root.join(format!("{project}.tex")), format!("\\documentclass{{article}}\\addbibresource{{{project}.bib}}\\{name}{{legacy}}{{legacy}}{{legacy}}")).unwrap();
            std::fs::write(
                root.join(format!("{project}.bib")),
                "@book{owner,ids={legacy},title={Book}}",
            )
            .unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let main = Url::from_file_path(root.join("a.tex")).unwrap();
        let bib = Url::from_file_path(root.join("a.bib")).unwrap();
        let doc = &workspace.documents[&main];
        let at = doc
            .text
            .position(doc.text.source.find("{legacy}").unwrap() + 2);
        let changes = features::rename(&workspace, &main, at, "new-alias")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(
            apply(&doc.text.source, &changes[&main]),
            doc.text.source.replacen("{legacy}", "{new-alias}", 1)
        );
        assert_eq!(
            apply(&workspace.documents[&bib].text.source, &changes[&bib]),
            "@book{owner,ids={new-alias},title={Book}}"
        );
        std::fs::write(
            root.join("shared.tex"),
            format!("\\{name}{{legacy}}{{yes}}{{no}}"),
        )
        .unwrap();
        for project in ["a", "b"] {
            std::fs::write(
                root.join(format!("{project}.tex")),
                format!(
                    "\\documentclass{{article}}\\addbibresource{{{project}.bib}}\\input{{shared}}"
                ),
            )
            .unwrap();
        }
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        let shared = Url::from_file_path(root.join("shared.tex")).unwrap();
        assert!(features::rename(
            &workspace,
            &shared,
            Position::new(0, name.len() as u32 + 3),
            "new-alias"
        )
        .is_err());
    }
}

#[test]
fn entry_queries_do_not_claim_nested_citations_ordinary_warning_ranges() {
    for name in COMMANDS {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let uri = Url::from_file_path(&main).unwrap();
        let source = format!("\\{name}{{missing}}{{Literal missing}}{{\\cite{{missing}}}}\n");
        std::fs::write(&main, &source).unwrap();
        let diagnostics = build_log::parse(
            &main,
            "Package biblatex Warning: Citation 'missing' on page 1 undefined on input line 1.\n",
        );
        let start = source.rfind("missing").unwrap();
        assert_eq!(diagnostics[&uri].len(), 1);
        assert_eq!(
            diagnostics[&uri][0].range,
            Text::new(source).range(start, start + 7)
        );
    }
}
