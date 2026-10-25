use latex_lsp::{
    build_log, editing, features, semantic,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

const CALLS: &[(&str, bool)] = &[
    ("cite", true),
    ("citep*", true),
    ("textcite", true),
    ("fullcite", true),
    ("footfullcite", true),
    ("nocite", true),
    ("cref", false),
    ("Cref*", false),
    ("cpageref", false),
    ("Cpageref*", false),
];

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///commented-keys/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    uri
}

fn declarations(workspace: &mut Workspace) -> (Url, Url) {
    (
        insert(workspace, "labels.tex", r"\label{parent}\label{other}"),
        insert(
            workspace,
            "refs.bib",
            "@book{parent,title={Parent}}\n@book{other,title={Other}}\n",
        ),
    )
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
fn commented_lists_navigate_and_rename_only_contiguous_keys_in_the_correct_namespace() {
    for &(command, citation) in CALLS {
        let mut workspace = Workspace::default();
        let (labels, bib) = declarations(&mut workspace);
        let source = format!("😀 \\{command}{{% parent, other }} \\ref{{fake}}\r\nparent% parent, other\r\n,% comma, }}\r\nother,% parent\r\nparent}}\r\n");
        let main = insert(&mut workspace, "main.tex", &source);
        for needle in ["\r\nparent%", "\r\nparent}"] {
            let offset = source.find(needle).unwrap() + 4;
            let position = Text::new(source.clone()).position(offset);
            let definitions = features::definitions(&workspace, &main, position);
            assert_eq!(definitions.len(), 1, "{command}: {definitions:?}");
            assert_eq!(
                definitions[0].uri,
                if citation {
                    bib.clone()
                } else {
                    labels.clone()
                }
            );
            assert!(features::hover(&workspace, &main, position).is_some());
            assert_eq!(
                features::references(&workspace, &main, position, false).len(),
                2
            );
            let changes = features::rename(&workspace, &main, position, "renamed")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 2);
            assert_eq!(
                apply(&source, &changes[&main]),
                source
                    .replace("\r\nparent%", "\r\nrenamed%")
                    .replace("\r\nparent}", "\r\nrenamed}")
            );
            let declaration = if citation { &bib } else { &labels };
            let original = &workspace.documents[declaration].text.source;
            assert_eq!(
                apply(original, &changes[declaration]),
                original.replace("{parent", "{renamed")
            );
        }
        let in_comment =
            Text::new(source.clone()).position(source.find("parent, other").unwrap() + 2);
        assert!(features::prepare_rename(&workspace, &main, in_comment)
            .unwrap()
            .is_none());
    }
}

#[test]
fn commented_list_completion_preserves_comments_and_replaces_only_the_active_key() {
    let mut workspace = Workspace::default();
    declarations(&mut workspace);
    for &(command, _) in CALLS {
        for (arguments, expected) in [
            (
                "{other,% parent, fake}\r\npar¦BAD% tail , }\r\n}",
                "{other,% parent, fake}\r\nparent% tail , }\r\n}",
            ),
            ("{% leading, }\n¦}", "{% leading, }\nparent}"),
            ("{¦% trailing, }\n}", "{parent% trailing, }\n}"),
            ("{other,% leading\n par¦", "{other,% leading\n parent"),
            (
                "{\\dynamic{other,parent},% comma, }\npar¦BAD}",
                "{\\dynamic{other,parent},% comma, }\nparent}",
            ),
        ] {
            let marked = format!("\\{command}{arguments}");
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
                .find(|item| item.label == "parent")
                .unwrap_or_else(|| panic!("{marked}: {items:?}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(edit.range.start.line, edit.range.end.line);
            assert_eq!(
                apply(&source, std::slice::from_ref(edit)),
                format!("\\{command}{expected}")
            );
        }
    }
}

#[test]
fn dynamic_groups_escaped_delimiters_and_comment_spliced_names_are_not_partial_keys() {
    let mut workspace = Workspace::default();
    declarations(&mut workspace);
    for &(command, _) in CALLS {
        for arguments in [
            "{par% comment, }\nent}",
            "{\\dynamic{other,parent,other}}",
            "{other\\,parent}",
            "{\\% parent}",
            "{{parent}}",
        ] {
            let source = format!("\\{command}{arguments}");
            let main = insert(&mut workspace, "main.tex", &source);
            for needle in ["parent", "par%", "ent}"] {
                if let Some(offset) = source.find(needle) {
                    let position = Text::new(source.clone()).position(offset + 2);
                    assert!(
                        features::prepare_rename(&workspace, &main, position)
                            .unwrap()
                            .is_none(),
                        "{source}: {needle}"
                    );
                    assert!(
                        !features::completion(&workspace, &main, position)
                            .iter()
                            .any(|item| item.label == "parent"),
                        "{source}: {needle}"
                    );
                }
            }
        }
    }
}

#[test]
fn commented_list_warning_ranges_and_fixes_preserve_all_comment_text() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for &(command, citation) in CALLS.iter().filter(|(name, _)| *name != "nocite") {
        let source = format!(
            "\\label{{parent}}\\bibitem{{parent}} 😀 \\{command}{{other,% parnet, parent }}\n parnet% parnet\n,% comma,\nparnet}}\n"
        );
        std::fs::write(&main, &source).unwrap();
        let kind = if citation { "Citation" } else { "Reference" };
        let log = format!(
            "{}:4: LaTeX Warning: {kind} `parnet' on page 1 undefined on input line 4.\n",
            main.display()
        );
        let diagnostics = build_log::parse(&main, &log);
        assert_eq!(diagnostics[&uri].len(), 2, "{command}: {diagnostics:?}");
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Workspace::default()
        };
        workspace.open(uri.clone(), source.clone(), 7);
        workspace
            .build_diagnostics
            .insert(uri.clone(), diagnostics.clone());
        let text = Text::new(source.clone());
        let mut edits = Vec::new();
        for diagnostic in &diagnostics[&uri] {
            let start = text.offset(diagnostic.range.start).unwrap();
            let end = text.offset(diagnostic.range.end).unwrap();
            assert_eq!(&source[start..end], "parnet");
            assert!(matches!(diagnostic.range.start.line, 1 | 3));
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
                            .find(|edit| edit.new_text == "parent")
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{command}: {actions:?}"));
            edits.push(edit.clone());
        }
        assert_eq!(
            apply(&source, &edits),
            source
                .replace("\n parnet%", "\n parent%")
                .replace("\nparnet}", "\nparent}")
        );
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_commented_citations_complete_and_rename_without_changing_comments_or_aux_keys() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let refs = directory.path().join("refs.tex");
    let main_uri = Url::from_file_path(&main).unwrap();
    let refs_uri = Url::from_file_path(&refs).unwrap();
    let source = "\\documentclass{article}\n\\begin{document}\n\\cite{alpha,% keep beta, brace }\n betBAD}\n\\input{refs}\n\\end{document}\n";
    let bibliography = "\\begin{thebibliography}{9}\n\\bibitem{alpha} First.\n\\bibitem{beta} Second.\n\\end{thebibliography}\n";
    std::fs::write(&main, source).unwrap();
    std::fs::write(&refs, bibliography).unwrap();
    let mut workspace = Workspace::default();
    workspace.open(main_uri.clone(), source.into(), 7);
    workspace.open(refs_uri.clone(), bibliography.into(), 11);
    let position = Position::new(3, 4);
    let items = features::completion(&workspace, &main_uri, position);
    let item = items.iter().find(|item| item.label == "beta").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing edit")
    };
    let fixed = apply(source, std::slice::from_ref(edit));
    assert_eq!(fixed, source.replace("betBAD", "beta"));
    std::fs::write(&main, &fixed).unwrap();
    workspace.open(main_uri.clone(), fixed.clone(), 8);
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let compile = || {
        let result = latex_lsp::build::run(&main, &config, &Default::default());
        assert_eq!(result.status, "success", "{}", result.output);
        result
    };
    compile();
    assert!(compile().diagnostics.values().all(Vec::is_empty));
    let definitions = features::definitions(&workspace, &main_uri, position);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].uri, refs_uri);
    let changes = features::rename(&workspace, &main_uri, position, "beta-new")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    let renamed = apply(&fixed, &changes[&main_uri]);
    assert_eq!(renamed, fixed.replace("\n beta}", "\n beta-new}"));
    let renamed_bib = apply(bibliography, &changes[&refs_uri]);
    assert_eq!(renamed_bib, bibliography.replace("{beta}", "{beta-new}"));
    std::fs::write(&main, renamed).unwrap();
    std::fs::write(&refs, renamed_bib).unwrap();
    compile();
    let result = compile();
    assert!(result.diagnostics.values().all(Vec::is_empty));
    let aux = std::fs::read_to_string(result.pdf.unwrap().with_extension("aux")).unwrap();
    assert!(aux.contains("\\bibcite{beta-new}"));
    assert!(!aux.contains("\\bibcite{beta}"));
}

#[test]
fn semantic_tokens_ignore_comment_commas_and_keep_utf16_key_ranges() {
    for &(command, _) in CALLS {
        let source = format!(
            "😀 \\{command}{{% 章节😀, }}\r\n章节😀% other, }}\r\n,other,% 章节😀\r\n章节😀}}\r\n"
        );
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
        assert_eq!(keys, ["章节😀", "other", "章节😀"], "{command}");
    }
}
