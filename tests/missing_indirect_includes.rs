use latex_lsp::{
    compilation::build_log,
    editing,
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

fn call(form: &str) -> &str {
    match form {
        "named" => r"\csuse{include}",
        "literal" => r"\csname include\endcsname",
        "alias" => r"\readpart",
        "named-alias" => r"\csuse{readpart}",
        "literal-alias" => r"\csname readpart\endcsname",
        _ => panic!(),
    }
}

#[test]
fn indirect_missing_include_warnings_fix_only_the_filename_with_versioned_edits() {
    for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
        for (argument, expected, target, report) in [
            ("{chapte}", "{chapter}", "chapter", "chapte.tex"),
            ("{chapte.tex}", "{chapter.tex}", "chapter", "chapte.tex"),
            ("{my chapte}", "{my chapter}", "my chapter", "my chapte.tex"),
            ("{cafe}", "{café}", "café", "cafe.tex"),
            (
                "% keep 😀\r\n{% file\r\nchapte% keep\r\n}",
                "% keep 😀\r\n{% file\r\nchapter% keep\r\n}",
                "chapter",
                "chapte.tex",
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let uri = Url::from_file_path(&main).unwrap();
            let source = format!("\\def\\readpart{{\\include}}\n{}{argument}\n", call(form));
            std::fs::write(&main, &source).unwrap();
            std::fs::write(directory.path().join(format!("{target}.tex")), "Chapter.").unwrap();
            let log = format!("(./main.tex\nNo file main.aux.\nNo file {report}.\n)\n");
            let diagnostics = build_log::parse(&main, &log);
            let diagnostic = diagnostics[&uri]
                .iter()
                .find(|d| d.code == Some(NumberOrString::String("missing-include".into())))
                .unwrap();
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("missing-include".into())),
                "{form}/{argument}: {diagnostic:?}"
            );
            let mut workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
                    code_action_literals: true,
                    versioned_edits: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace
                .documents
                .insert(uri.clone(), Document::new(&uri, source.clone(), Some(7)));
            let diagnostic = diagnostic.clone();
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), diagnostics);
            let actions = editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &CodeActionContext {
                    diagnostics: vec![diagnostic],
                    ..Default::default()
                },
            );
            assert_eq!(actions.len(), 1, "{form}/{argument}: {actions:?}");
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!()
            };
            let DocumentChanges::Edits(changes) = action
                .edit
                .as_ref()
                .unwrap()
                .document_changes
                .as_ref()
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(changes[0].text_document.version, Some(7));
            let OneOf::Left(edit) = &changes[0].edits[0] else {
                panic!()
            };
            let changed = Text::new(source)
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text.clone(),
                }])
                .unwrap();
            assert_eq!(
                changed.source,
                format!("\\def\\readpart{{\\include}}\n{}{expected}\n", call(form)),
                "{form}/{argument}"
            );
        }
    }
}

#[test]
fn uncertain_indirect_includes_and_auxiliary_messages_do_not_create_chapter_warnings() {
    for source in [
        r"\newcommand{\later}{\csuse{include}{chapte}}",
        r"\newcommand{\hold}[1]{}\hold\readpart{chapte}",
        r"\renewcommand{\include}[1]{}\readpart{chapte}",
        r"\renewcommand{\csuse}[1]{}\csuse{include}{chapte}",
        r"\csuse{include}{\filename}",
        r"\csuse{include}chapte",
        r"\csuse{include}[optional]{chapte}",
        r"\endinput\readpart{chapte}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, format!("\\def\\readpart{{\\include}}\n{source}\n")).unwrap();
        let log = "(./main.tex\nNo file main.aux.\nNo file chapte.aux.\nNo file chapte.tex.\n)\n";
        let diagnostics = build_log::parse(&main, log);
        assert!(
            !diagnostics
                .values()
                .flatten()
                .any(|d| d.code == Some(NumberOrString::String("missing-include".into()))),
            "{source}: {diagnostics:?}"
        );
    }
}

#[test]
fn legacy_indirect_include_fixes_recheck_alias_dependencies_without_changing_the_parent_version() {
    for form in ["alias", "named-alias", "literal-alias"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let definitions = Url::from_file_path(root.join("definitions.tex")).unwrap();
        let source = format!("\\input{{definitions}}\n{}{{chapte}}\n", call(form));
        std::fs::write(root.join("main.tex"), &source).unwrap();
        std::fs::write(root.join("definitions.tex"), r"\def\readpart{\include}").unwrap();
        std::fs::write(root.join("chapter.tex"), "Chapter.").unwrap();
        let mut workspace = Workspace {
            client: latex_lsp::project::state::ClientCapabilities {
                apply_edits: true,
                versioned_edits: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(7)));
        workspace.documents.insert(
            definitions.clone(),
            Document::new(&definitions, r"\def\readpart{\include}".into(), Some(3)),
        );
        let at = source.rfind("chapte").unwrap();
        let diagnostic = Diagnostic {
            range: workspace.documents[&uri]
                .text
                .range(at, at + "chapte".len()),
            source: Some("latex-build".into()),
            code: Some(NumberOrString::String("missing-include".into())),
            message: "Included file `chapte.tex` not found".into(),
            data: Some(serde_json::json!({"file":"chapte.tex", "macroSources":[uri,definitions]})),
            ..Default::default()
        };
        workspace.project.build_diagnostics.insert(
            uri.clone(),
            [(uri.clone(), vec![diagnostic.clone()])].into(),
        );
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                ..Default::default()
            },
        );
        assert_eq!(actions.len(), 1, "{form}: {actions:?}");
        let CodeActionOrCommand::Command(action) = &actions[0] else {
            panic!()
        };
        let request = action.arguments.as_ref().unwrap()[0].clone();
        let resolve = |workspace: &Workspace| {
            editing::resolve_fix(workspace, serde_json::from_value(request.clone()).unwrap())
        };
        assert!(resolve(&workspace).is_some());
        for replacement in [
            r"\def\readpart#1{}",
            r"\def\readpart{\input}",
            r"\def\include#1{}\def\readpart{\include}",
        ] {
            workspace.documents.insert(
                definitions.clone(),
                Document::new(&definitions, replacement.into(), Some(4)),
            );
            assert_eq!(workspace.documents[&uri].version, Some(7));
            assert!(resolve(&workspace).is_none(), "{form}: {replacement}");
        }
        workspace.documents.insert(
            definitions.clone(),
            Document::new(&definitions, r"\def\readpart{\include}".into(), Some(5)),
        );
        assert!(resolve(&workspace).is_some());
        std::fs::remove_file(root.join("chapter.tex")).unwrap();
        assert!(resolve(&workspace).is_none());
    }
}

#[test]
fn indirect_include_fixes_respect_the_active_includeonly_filter() {
    for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
        for (names, expected) in [("chapte", 0), ("chapte,chapter", 1)] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let uri = Url::from_file_path(&main).unwrap();
            let source = format!(
                "\\def\\readpart{{\\include}}\\includeonly{{{names}}}\n{}{{chapte}}\n",
                call(form)
            );
            std::fs::write(&main, &source).unwrap();
            std::fs::write(directory.path().join("chapter.tex"), "Chapter").unwrap();
            let mut workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
                    code_action_literals: true,
                    versioned_edits: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace
                .documents
                .insert(uri.clone(), Document::new(&uri, source, Some(7)));
            let diagnostics = build_log::parse(&main, "(./main.tex\nNo file chapte.tex.\n)");
            let diagnostic = diagnostics[&uri][0].clone();
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), diagnostics);
            let actions = editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &CodeActionContext {
                    diagnostics: vec![diagnostic],
                    ..Default::default()
                },
            );
            assert_eq!(actions.len(), expected, "{form}/{names}: {actions:?}");
        }
    }
}

#[test]
fn missing_indirect_chapter_auxiliary_files_do_not_create_source_file_warnings() {
    for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(
            &main,
            format!("\\def\\readpart{{\\include}}\n{}{{chapte}}\n", call(form)),
        )
        .unwrap();
        let diagnostics = build_log::parse(
            &main,
            "(./main.tex\nNo file main.aux.\nNo file chapte.aux.\n)",
        );
        assert!(
            !diagnostics
                .values()
                .flatten()
                .any(|d| d.code == Some(NumberOrString::String("missing-include".into()))),
            "{form}: {diagnostics:?}"
        );
    }
}
