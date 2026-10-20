use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use std::path::Path;
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn database_diagnostic(root: &Path, name: &str) -> Diagnostic {
    let log = format!("I couldn't open database file {name}\n---line 3 of file main.aux\n");
    let parsed = build_log::parse_bibliography(&root.join("main.tex"), &log);
    parsed
        .values()
        .flatten()
        .find(|item| {
            item.code
                == Some(NumberOrString::String(
                    "missing-bibliography-database".into(),
                ))
        })
        .unwrap()
        .clone()
}
fn actions(workspace: &Workspace, document: &Url, diagnostic: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        document,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            ..Default::default()
        },
    )
}
fn edits(action: &CodeActionOrCommand, document: &Url, version: i32) -> Vec<TextEdit> {
    let CodeActionOrCommand::CodeAction(action) = action else {
        panic!("missing literal action")
    };
    let DocumentChanges::Edits(changes) = action
        .edit
        .as_ref()
        .unwrap()
        .document_changes
        .as_ref()
        .unwrap()
    else {
        panic!("missing versioned changes")
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].text_document.uri, *document);
    assert_eq!(changes[0].text_document.version, Some(version));
    changes[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            OneOf::Right(_) => panic!("unexpected annotation"),
        })
        .collect()
}
fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
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
fn missing_databases_select_list_items_with_comments_spaces_utf16_and_explicit_suffixes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for command in ["bibliography", "nobibliography"] {
        for suffix in ["", ".bib"] {
            let name = format!("db/re ffs{suffix}");
            let source = format!("% 😀 \\{command}{{db/reffs}}\r\n\\{command}{{first,% before phantom,\r\n {name}% after,\r\n ,second}}\r\n");
            std::fs::write(root.join("main.tex"), &source).unwrap();
            let diagnostic = database_diagnostic(root, "db/reffs.bib");
            let start = source.find(&format!(" {name}%")).unwrap() + 1;
            assert_eq!(
                diagnostic.range,
                Text::new(source).range(start, start + name.len())
            );
            assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
            assert_eq!(
                diagnostic.data,
                Some(serde_json::json!({"file":"db/reffs.bib","bibliographyCommand":command}))
            );
        }
    }
}

#[test]
fn dynamic_redefined_ambiguous_or_ignored_database_declarations_remain_unresolved() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        r"\bibliography{\chosen}",
        r"\bibliography{reffs,\chosen}",
        r"\bibliography{reffs}\nobibliography{reffs.bib}",
        r"\bibliography{reffs,reffs.bib}",
        r"\newcommand{\bibliography}[1]{}\bibliography{reffs}",
        r"\newcommand{\nobibliography}[1]{}\bibliography{reffs}",
        r"\bibliography{reffs}\nobibliography{\chosen}",
        r"\bibliography*{reffs}",
        r"\nobibliography*{reffs}",
        r"\bibliography[note]{reffs}",
        "\\bibliography{re% split\nffs}",
        r"% \bibliography{reffs}",
        r"\verb|\bibliography{reffs}|",
        r"\bibliography{other}",
        r"\bibliography{reffs.bib.bib}",
    ] {
        std::fs::write(root.join("main.tex"), format!("% first\n{source}\n")).unwrap();
        let diagnostic = database_diagnostic(root, "reffs.bib");
        assert_eq!(diagnostic.range.start.line, 0, "{source}");
        assert_eq!(
            diagnostic.data.as_ref().unwrap()["unresolvedSource"],
            true,
            "{source}"
        );
    }
}

#[test]
fn only_native_auxiliary_database_errors_receive_the_missing_database_code() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), r"\bibliography{reffs}").unwrap();
    std::fs::write(root.join("refs.bib"), "@book{one,title={Title}}\n").unwrap();
    for log in [
        "I couldn't open database file reffs.bib\n---line 1 of file refs.bib\n",
        "I couldn't open database file reffs.bib\n---line 1 of file main.tex\n",
        "Warning--I couldn't open database file reffs.bib\n",
        "ERROR - I couldn't open database file reffs.bib\n",
        "I couldn't open database file \\chosen.bib\n---line 3 of file main.aux\n",
        "I couldn't open database file spaced name.bib\n---line 3 of file main.aux\n",
        "I couldn't open database file .bib\n---line 3 of file main.aux\n",
    ] {
        let parsed = build_log::parse_bibliography(&root.join("main.tex"), log);
        assert!(
            !parsed.values().flatten().any(|item| item.code
                == Some(NumberOrString::String(
                    "missing-bibliography-database".into()
                ))),
            "{log}\n{parsed:?}"
        );
    }
}

#[test]
fn database_fixes_preserve_other_items_comments_directories_suffixes_and_legacy_guards() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("db")).unwrap();
    let bib = "@book{one,title={Title}}\n";
    std::fs::write(root.join("db/refs.bib"), bib).unwrap();
    let main = uri(root, "main.tex");
    for command in ["bibliography", "nobibliography"] {
        for suffix in ["", ".bib", ".b i b"] {
            let name = format!("d b/re ffs{suffix}");
            let source = format!(
                "% 😀 keep\r\n\\{command}{{first,% before,\r\n {name}% after,\r\n ,second}}\r\n"
            );
            std::fs::write(root.join("main.tex"), &source).unwrap();
            let mut workspace = Workspace {
                code_action_literals: true,
                apply_edits: true,
                versioned_edits: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            let diagnostic = database_diagnostic(root, "db/reffs.bib");
            workspace.build_diagnostics.insert(
                main.clone(),
                [(main.clone(), vec![diagnostic.clone()])].into(),
            );
            let result = actions(&workspace, &main, &diagnostic);
            assert_eq!(result.len(), 1, "{command} {suffix}: {result:?}");
            let replacement = if suffix.is_empty() {
                "db/refs"
            } else {
                "db/refs.bib"
            };
            assert_eq!(
                apply(&source, edits(&result[0], &main, 7)),
                source.replace(&name, replacement)
            );
            let CodeActionOrCommand::CodeAction(action) = &result[0] else {
                panic!()
            };
            assert_eq!(
                action.title,
                format!("Replace with bibliography database file `{replacement}`")
            );
            let modern = action.edit.clone().unwrap();
            workspace.code_action_literals = false;
            let legacy = actions(&workspace, &main, &diagnostic);
            let CodeActionOrCommand::Command(command) = &legacy[0] else {
                panic!()
            };
            let request = command.arguments.as_ref().unwrap()[0].clone();
            let resolve = |workspace: &Workspace| {
                editing::resolve_fix(workspace, serde_json::from_value(request.clone()).unwrap())
            };
            assert_eq!(resolve(&workspace), Some(modern));
            workspace.documents.get_mut(&main).unwrap().version = Some(8);
            assert!(resolve(&workspace).is_none());
            workspace.documents.get_mut(&main).unwrap().version = Some(7);
            std::fs::write(root.join("db/reffs.bib"), bib).unwrap();
            assert!(resolve(&workspace).is_none());
            std::fs::remove_file(root.join("db/reffs.bib")).unwrap();
            assert!(resolve(&workspace).is_some());
            std::fs::remove_file(root.join("db/refs.bib")).unwrap();
            assert!(resolve(&workspace).is_none());
            std::fs::write(root.join("db/refs.bib"), bib).unwrap();
        }
    }
}

#[test]
fn database_candidates_are_bounded_and_cannot_introduce_commas_spaces_or_wrong_extensions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in [
        "refs",
        "refsa",
        "refsb",
        "refsc",
        "refsd",
        "refse",
        "refsf",
        "ref,g",
        "ref g",
        "unrelated",
    ] {
        std::fs::write(
            root.join(format!("{name}.bib")),
            "@book{one,title={Title}}\n",
        )
        .unwrap();
    }
    std::fs::write(root.join("refsg.tex"), "Wrong type.").unwrap();
    let source = r"\bibliography{refsg}";
    std::fs::write(root.join("main.tex"), source).unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace {
        code_action_literals: true,
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(main.clone(), source.into(), 7);
    let diagnostic = database_diagnostic(root, "refsg.bib");
    workspace.build_diagnostics.insert(
        main.clone(),
        [(main.clone(), vec![diagnostic.clone()])].into(),
    );
    let result = actions(&workspace, &main, &diagnostic);
    assert_eq!(result.len(), 5, "{result:?}");
    for action in &result {
        for edit in edits(action, &main, 7) {
            assert!(!edit.new_text.contains([',', ' ']));
            assert!(!edit.new_text.ends_with(".tex"));
            assert!(!edit.new_text.contains("unrelated"));
        }
    }
    for data in [
        serde_json::json!({"file":"refsg.bib"}),
        serde_json::json!({"file":"wrong.bib","bibliographyCommand":"bibliography"}),
        serde_json::json!({"file":"refsg.bib","bibliographyCommand":"addbibresource"}),
        serde_json::json!({"file":"refsg.bib","bibliographyCommand":"nobibliography"}),
    ] {
        let mut mismatch = diagnostic.clone();
        mismatch.data = Some(data);
        workspace.build_diagnostics.insert(
            main.clone(),
            [(main.clone(), vec![mismatch.clone()])].into(),
        );
        assert!(actions(&workspace, &main, &mismatch).is_empty());
    }
}
