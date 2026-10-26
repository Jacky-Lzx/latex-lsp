use latex_lsp::{
    compilation::{build, build_log},
    editing,
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

#[test]
fn missing_input_diagnostics_select_literal_paths_in_braces_quotes_and_bare_forms() {
    for (command, name, shown, expected) in [
        (
            "😀 \\input{ chapetr.tex }",
            "chapetr.tex",
            "😀 \\input{ chapetr.tex }",
            "chapetr.tex",
        ),
        ("\\input chapetr", "chapetr", "\\input chapetr", "chapetr"),
        (
            "\\input\"my chapetr\"",
            "\"my chapetr\"",
            "\\input\"my chapetr\"",
            "my chapetr",
        ),
        (
            "long prefix \\input{chapetr}",
            "chapetr.tex",
            "...\\input{chapetr}",
            "chapetr",
        ),
        (
            "\\input % keep\r\n{chapetr}",
            "chapetr.tex",
            "{chapetr}",
            "chapetr",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = format!("First.\n{command}\n");
        std::fs::write(&main, &source).unwrap();
        let line = source.lines().count();
        let message = format!("I can't find file `{name}'.");
        let log = format!("(./main.tex\n./main.tex:{line}: {message}\nl.{line} {shown}\n");
        let entries = build_log::parse(&main, &log);
        let diagnostic = &entries[&Url::from_file_path(&main).unwrap()][0];
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("missing-input".into())),
            "{log}\n{diagnostic:?}"
        );
        let text = Text::new(source);
        let range = diagnostic.range;
        let value =
            &text.source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()];
        assert_eq!(value, expected);
        assert_eq!(
            diagnostic.data.as_ref().unwrap()["file"],
            name.trim_matches('"')
        );
    }
}

#[test]
fn uncertain_missing_input_contexts_keep_line_diagnostics_without_file_fixes() {
    for (source, shown, stack) in [
        (r"\input{\path}", r"\input{\path}", ""),
        (r"% \input{chapetr}", r"% \input{chapetr}", ""),
        (r"\verb|\input{chapetr}|", r"\verb|\input{chapetr}|", ""),
        (r#"\input"chapetr"#, r#"\input"chapetr"#, ""),
        (r"\input{chapetr}", r"\different{chapetr}", ""),
        (r"\input{chapetr}\input{chapetr}", r"...\input{chapetr}", ""),
        (
            r"\input{chapetr}",
            r"\input{chapetr}",
            "<argument> \\expanded\n",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, format!("First.\n{source}\n")).unwrap();
        let log = format!("(./main.tex\n! LaTeX Error: File `chapetr.tex' not found.\n! Emergency stop.\n<read *>\n{stack}l.2 {shown}\n");
        let entries = build_log::parse(&main, &log);
        let diagnostic = entries[&Url::from_file_path(&main).unwrap()]
            .iter()
            .find(|entry| entry.message.contains("not found"))
            .unwrap();
        assert!(diagnostic.code.is_none(), "{log}\n{diagnostic:?}");
        assert!(diagnostic.data.is_none());
        assert_eq!(diagnostic.range.start.line, 1);
        assert_eq!(diagnostic.range.start.character, 0);
    }
}

fn input_warning(workspace: &mut Workspace, root: &Url, uri: &Url, file: &str) -> Diagnostic {
    let doc = &workspace.documents[uri];
    let argument = doc
        .syntax
        .commands
        .iter()
        .find(|command| command.name == "input")
        .unwrap()
        .argument
        .as_ref()
        .unwrap();
    let leading = argument.value.len() - argument.value.trim_start().len();
    let end = argument.value.trim_end().len();
    let diagnostic = Diagnostic {
        range: doc
            .text
            .range(argument.span.start + leading, argument.span.start + end),
        source: Some("latex-build".into()),
        code: Some(NumberOrString::String("missing-input".into())),
        message: format!("LaTeX Error: File `{file}' not found."),
        data: Some(serde_json::json!({"file":file})),
        ..Default::default()
    };
    workspace.project.build_diagnostics.insert(
        root.clone(),
        [(uri.clone(), vec![diagnostic.clone()])].into(),
    );
    diagnostic
}

fn actions(workspace: &Workspace, uri: &Url, diagnostic: Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic],
            ..Default::default()
        },
    )
}

#[test]
fn input_spelling_fixes_protect_spaces_and_preserve_existing_delimiters_and_comments() {
    for (command, expected) in [
        (
            "\\input % keep\r\n{ mychapter } % keep",
            "\\input % keep\r\n{ my chapter } % keep",
        ),
        ("\\input mychapter trailing", "\\input {my chapter}trailing"),
        (
            "\\input\"mychapter\" trailing",
            "\\input\"my chapter\" trailing",
        ),
        ("\\input{mychapter.tex}", "\\input{my chapter.tex}"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let source = format!("😀 {command}\n");
        std::fs::write(directory.path().join("main.tex"), &source).unwrap();
        std::fs::write(directory.path().join("my chapter.tex"), "Chapter.").unwrap();
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
        let diagnostic = input_warning(&mut workspace, &uri, &uri, "mychapter.tex");
        let result = actions(&workspace, &uri, diagnostic);
        assert_eq!(result.len(), 1, "{command}: {result:?}");
        let CodeActionOrCommand::CodeAction(action) = &result[0] else {
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
        assert_eq!(changed.source, format!("😀 {expected}\n"));
    }
}

#[test]
fn unicode_input_fixes_group_paths_and_legacy_commands_revalidate_the_expanded_edit() {
    for (command, expected) in [
        (r"\input cafe trailing", r"\input {café}trailing"),
        (r#"\input"cafe" trailing"#, r"\input{café}trailing"),
        (r"\input{ cafe } trailing", r"\input{ café } trailing"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let source = format!("% 😀\r\n{command}\r\n");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        std::fs::write(root.join("café.tex"), "Chapter.").unwrap();
        let mut workspace = Workspace {
            client: latex_lsp::project::state::ClientCapabilities {
                code_action_literals: true,
                apply_edits: true,
                versioned_edits: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(7)));
        let diagnostic = input_warning(&mut workspace, &uri, &uri, "cafe.tex");
        let result = actions(&workspace, &uri, diagnostic.clone());
        assert_eq!(result.len(), 1, "{result:?}");
        let CodeActionOrCommand::CodeAction(action) = &result[0] else {
            panic!()
        };
        let modern = action.edit.clone().unwrap();
        let DocumentChanges::Edits(changes) = modern.document_changes.as_ref().unwrap() else {
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
        assert_eq!(changed.source, format!("% 😀\r\n{expected}\r\n"));
        workspace.client.code_action_literals = false;
        let legacy = actions(&workspace, &uri, diagnostic);
        assert_eq!(legacy.len(), 1);
        let CodeActionOrCommand::Command(command) = &legacy[0] else {
            panic!()
        };
        let request = command.arguments.as_ref().unwrap()[0].clone();
        let resolve = |workspace: &Workspace| {
            editing::resolve_fix(workspace, serde_json::from_value(request.clone()).unwrap())
        };
        assert_eq!(resolve(&workspace), Some(modern));
        workspace.documents.get_mut(&uri).unwrap().version = Some(8);
        assert!(resolve(&workspace).is_none());
        workspace.documents.get_mut(&uri).unwrap().version = Some(7);
        std::fs::remove_file(root.join("café.tex")).unwrap();
        assert!(resolve(&workspace).is_none());
        std::fs::write(root.join("café.tex"), "Chapter.").unwrap();
        assert!(resolve(&workspace).is_some());
        workspace.clear_build_diagnostics(&uri);
        assert!(resolve(&workspace).is_none());
    }
}

#[test]
fn input_file_fixes_ignore_includeonly_and_revalidate_files_and_command_requests() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let source = "\\includeonly{\\dynamic}\\input{chapetr}\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("chapter.tex"), "Chapter.").unwrap();
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            apply_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    let diagnostic = input_warning(&mut workspace, &uri, &uri, "chapetr.tex");
    let result = actions(&workspace, &uri, diagnostic.clone());
    assert_eq!(result.len(), 1, "{result:?}");
    let CodeActionOrCommand::Command(command) = &result[0] else {
        panic!()
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let resolve = |workspace: &Workspace| {
        editing::resolve_fix(workspace, serde_json::from_value(value.clone()).unwrap())
    };
    assert!(resolve(&workspace).is_some());
    std::fs::remove_file(root.join("chapter.tex")).unwrap();
    assert!(resolve(&workspace).is_none());
    std::fs::write(root.join("chapter.tex"), "Chapter.").unwrap();
    std::fs::write(root.join("chapetr"), "An extensionless input now exists.").unwrap();
    assert!(resolve(&workspace).is_none());
    std::fs::remove_file(root.join("chapetr")).unwrap();
    workspace.documents.get_mut(&uri).unwrap().version = Some(8);
    assert!(resolve(&workspace).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(7);
    workspace.clear_build_diagnostics(&uri);
    assert!(resolve(&workspace).is_none());
}

#[test]
fn shared_input_source_uses_the_diagnostic_build_root_for_filename_suggestions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for folder in ["a", "b", "shared"] {
        std::fs::create_dir(root.join(folder)).unwrap();
    }
    for name in ["a/main.tex", "b/main.tex"] {
        std::fs::write(
            root.join(name),
            "\\documentclass{article}\\input{../shared/source}",
        )
        .unwrap();
    }
    for (name, source) in [
        ("shared/source.tex", "\\input{chapetr}"),
        ("a/chapter.tex", "Project A."),
        ("b/chapert.tex", "Project B."),
        (
            "shared/chapetr.tex",
            "Source directory is not the compiler working directory.",
        ),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let uri = Url::from_file_path(root.join("shared/source.tex")).unwrap();
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    for (folder, expected) in [("a", "chapter"), ("b", "chapert")] {
        workspace.project.build_diagnostics.clear();
        let main = Url::from_file_path(root.join(folder).join("main.tex")).unwrap();
        let diagnostic = input_warning(&mut workspace, &main, &uri, "chapetr.tex");
        let result = actions(&workspace, &uri, diagnostic);
        assert_eq!(result.len(), 1, "{folder}: {result:?}");
        let CodeActionOrCommand::CodeAction(action) = &result[0] else {
            panic!()
        };
        assert_eq!(
            action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri][0].new_text,
            expected
        );
    }
}

#[test]
fn input_fixes_reject_incomplete_quotes_and_mismatched_compiler_filename_data() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    std::fs::write(root.join("chapter.tex"), "Chapter.").unwrap();
    for source in [r#"\input"chapetr"#, r"\input{chapetr}"] {
        std::fs::write(root.join("main.tex"), source).unwrap();
        let mut workspace = Workspace {
            client: latex_lsp::project::state::ClientCapabilities {
                code_action_literals: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
        let mut diagnostic = input_warning(&mut workspace, &uri, &uri, "chapetr.tex");
        if source.ends_with('}') {
            diagnostic.data = Some(serde_json::json!({"file":"another.tex"}));
            workspace
                .project
                .build_diagnostics
                .get_mut(&uri)
                .unwrap()
                .get_mut(&uri)
                .unwrap()[0] = diagnostic.clone();
        }
        assert!(actions(&workspace, &uri, diagnostic).is_empty(), "{source}");
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_input_filename_fixes_read_the_intended_source_for_braced_bare_and_quoted_inputs() {
    for (missing, target) in [("mychapter", "my chapter"), ("cafe", "café")] {
        for template in [r"\input{NAME}", r"\input NAME", r#"\input"NAME""#] {
            let command = template.replace("NAME", missing);
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let uri = Url::from_file_path(&main).unwrap();
            let source = format!(
                "\\documentclass{{article}}\n\\begin{{document}}\n{command}\n\\end{{document}}\n"
            );
            std::fs::write(&main, &source).unwrap();
            std::fs::write(
                root.join(format!("{target}.tex")),
                "\\typeout{REPAIRED INPUT READ}Chapter.\n",
            )
            .unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: "pdflatex".into(),
                ..Default::default()
            };
            let failed = build::run(&main, &config, &Default::default());
            assert_eq!(failed.status, "error", "{}", failed.output);
            let diagnostic = failed.diagnostics[&uri]
                .iter()
                .find(|entry| entry.code == Some(NumberOrString::String("missing-input".into())))
                .unwrap_or_else(|| panic!("{command}: {:?}\n{}", failed.diagnostics, failed.output))
                .clone();
            let mut workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), failed.diagnostics);
            let result = actions(&workspace, &uri, diagnostic);
            assert_eq!(result.len(), 1, "{result:?}");
            let CodeActionOrCommand::CodeAction(action) = &result[0] else {
                panic!()
            };
            let edit = &action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri][0];
            let changed = Text::new(source)
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text.clone(),
                }])
                .unwrap();
            std::fs::write(&main, changed.source).unwrap();
            let repaired = build::run(&main, &config, &Default::default());
            assert_eq!(repaired.status, "success", "{}", repaired.output);
            assert!(repaired.output.contains("REPAIRED INPUT READ"));
            assert!(repaired
                .inputs
                .unwrap()
                .contains(&Url::from_file_path(root.join(format!("{target}.tex"))).unwrap()));
            assert!(
                repaired.diagnostics.values().all(Vec::is_empty),
                "{:?}",
                repaired.diagnostics
            );
        }
    }
}
