use lsp_types::*;
use std::path::Path;
use texquill::{
    compilation::{build, build_log},
    editing,
    project::workspace::Workspace,
    text::Text,
};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn parse(source: &str, report: &str) -> (tempfile::TempDir, Url, Text, Vec<Diagnostic>) {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    let items = build_log::parse(&main, &format!("(./main.tex\n{report}\n)\n"))
        .remove(&uri(directory.path(), "main.tex"))
        .unwrap_or_default();
    (
        directory,
        uri(main.parent().unwrap(), "main.tex"),
        Text::new(source.into()),
        items,
    )
}

fn actions(workspace: &Workspace, main: &Url, item: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        main,
        item.range,
        &CodeActionContext {
            diagnostics: vec![item.clone()],
            ..Default::default()
        },
    )
}

fn versioned_edit(action: &CodeActionOrCommand, main: &Url) -> TextEdit {
    let CodeActionOrCommand::CodeAction(action) = action else {
        panic!()
    };
    let Some(DocumentChanges::Edits(documents)) = &action.edit.as_ref().unwrap().document_changes
    else {
        panic!()
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *main);
    assert_eq!(documents[0].text_document.version, Some(7));
    assert_eq!(documents[0].edits.len(), 1);
    let OneOf::Left(edit) = &documents[0].edits[0] else {
        panic!()
    };
    edit.clone()
}

#[test]
fn missing_verbatim_reports_locate_literal_paths_with_utf16_ranges() {
    for name in [
        "smaple.rs",
        "README",
        "sample.unknown",
        ".hidden",
        "dir.with.dot/code.rs",
        "样例, one.rs",
        "😀, one.rs",
    ] {
        for star in ["", "*", " * % keep\r\n"] {
            let source = format!("% 😀\r\n\\verbatiminput{star}{{{name}}}\r\n");
            let (_directory, _main, text, items) = parse(&source, &format!("No file {name}."));
            assert_eq!(items.len(), 1, "{source}: {items:?}");
            let item = &items[0];
            assert_eq!(
                item.range,
                text.range(
                    source.find(name).unwrap(),
                    source.find(name).unwrap() + name.len()
                )
            );
            assert_eq!(
                item.code,
                Some(NumberOrString::String("missing-verbatim-input".into()))
            );
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(item.source.as_deref(), Some("latex-build"));
            assert_eq!(
                item.message,
                format!("Verbatim input file `{name}` not found")
            );
            assert_eq!(item.data, Some(serde_json::json!({"file":name})));
        }
    }
}

#[test]
fn no_file_reports_require_a_supported_literal_call_in_the_confirmed_source() {
    for source in [
        r"\verbatiminput[options]{smaple.rs}",
        r"\verbatiminput**{smaple.rs}",
        r"\verbatiminput{\macro}",
        r"\verbatiminput{other.rs}",
        r"% \verbatiminput{smaple.rs}",
        r"\verb|\verbatiminput{smaple.rs}|",
        r"\renewcommand{\verbatiminput}[1]{}\verbatiminput{smaple.rs}",
        r"\def\verbatim@input#1#2{}\verbatiminput{smaple.rs}",
        r"\let\verbatim@readfile\input\verbatiminput{smaple.rs}",
    ] {
        let (_, _, _, items) = parse(source, "No file smaple.rs.");
        assert!(items.is_empty(), "{source}: {items:?}");
    }
    for report in [
        "No file main.aux.",
        "No file main.bbl.",
        "No file optional.tex.",
        "No file smaple.rs.tex.",
        "No file \"smaple.rs\".",
        "No file smaple.\nPackage local Warning: Adjacent warning.",
    ] {
        let (_, _, _, items) = parse(r"\verbatiminput{smaple.rs}", report);
        assert!(
            items
                .iter()
                .all(|item| item.code
                    != Some(NumberOrString::String("missing-verbatim-input".into()))),
            "{report}: {items:?}"
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, r"\verbatiminput{smaple.rs}").unwrap();
    for log in [
        "[texquill: middle of compiler output omitted]\nNo file smaple.rs.\n",
        "(./unavailable.tex\nNo file smaple.rs.\n",
    ] {
        let items = build_log::parse(&main, log);
        assert!(items.is_empty(), "{items:?}");
    }
}

#[test]
fn wrapped_names_keep_child_context_and_do_not_consume_adjacent_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let child = root.join("child.tex");
    std::fs::write(&main, r"\input{child}\verbatiminput{parent.rs}").unwrap();
    let name = format!("code/{} (中文, file).rs", "long.part.".repeat(10));
    let source = format!("😀 \\verbatiminput * % keep\r\n{{{name}}}\r\n");
    std::fs::write(&child, &source).unwrap();
    for width in [16, 79, 121] {
        let report = format!("No file {name}.")
            .chars()
            .collect::<Vec<_>>()
            .chunks(width)
            .map(|chunk| chunk.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        let log = format!("(./main.tex\n(./child.tex\nNo file optional(broken).tex.\n{report}\nNo file code/long.\nPackage local Warning: Adjacent warning on input line 2.\n)\nNo file parent.rs.\n)");
        let result = build_log::parse(&main, &log);
        let items = &result[&uri(root, "child.tex")];
        assert_eq!(items.len(), 2, "{width}: {result:?}");
        let missing = items
            .iter()
            .find(|item| item.code == Some(NumberOrString::String("missing-verbatim-input".into())))
            .unwrap();
        assert_eq!(missing.data, Some(serde_json::json!({"file":name})));
        let start = source.find(&name).unwrap();
        assert_eq!(
            missing.range,
            Text::new(source.clone()).range(start, start + name.len())
        );
        assert!(items
            .iter()
            .any(|item| item.message.contains("Adjacent warning")));
        assert_eq!(
            result[&uri(root, "main.tex")][0].data,
            Some(serde_json::json!({"file":"parent.rs"}))
        );
    }
}

#[test]
fn dotted_line_wraps_choose_the_full_name_and_repeated_reports_do_not_duplicate_ranges() {
    for command in ["include", "verbatiminput"] {
        let short = "a".repeat(68);
        let long = format!("{short}.continued.rs");
        let short_argument = if command == "include" {
            format!("{short}.tex")
        } else {
            short.clone()
        };
        let long_argument = if command == "include" {
            format!("{short_argument}.continued.rs.tex")
        } else {
            long.clone()
        };
        let source = format!("\\{command}{{{short_argument}}}\n\\{command}{{{long_argument}}}\n");
        let report = format!(
            "No file {short_argument}.\n{}.\nNo file {long_argument}.",
            &long_argument[short_argument.len() + 1..]
        );
        let (_, _, text, items) = parse(&source, &report);
        assert!(!items.is_empty());
        // Both physical reports describe the longer path, never its short prefix.
        assert!(
            items.iter().all(|item| {
                let start = text.offset(item.range.start).unwrap();
                let end = text.offset(item.range.end).unwrap();
                source[start..end] == long_argument
            }),
            "{command}: {items:?}"
        );
        if command == "verbatiminput" {
            assert_eq!(items.len(), 1);
        }
        let (_, _, _, items) = parse(&source, &format!("No file {short_argument}.\nPackage local Warning: Adjacent warning on input line 1."));
        assert_eq!(items.len(), 2, "{command}: {items:?}");
    }
}

#[test]
fn shared_missing_files_locate_both_command_families_without_duplicate_ranges() {
    let source = r"\include{missing}\verbatiminput*{missing.tex}";
    let (_, _, text, items) = parse(source, "No file missing.tex.\nNo file missing.tex.");
    assert_eq!(items.len(), 2, "{items:?}");
    let include = items
        .iter()
        .find(|item| item.code == Some(NumberOrString::String("missing-include".into())))
        .unwrap();
    let verbatim = items
        .iter()
        .find(|item| item.code == Some(NumberOrString::String("missing-verbatim-input".into())))
        .unwrap();
    let include_start = source.find("missing").unwrap();
    let verbatim_start = source.find("missing.tex").unwrap();
    assert_eq!(include.range, text.range(include_start, include_start + 7));
    assert_eq!(
        verbatim.range,
        text.range(verbatim_start, verbatim_start + 11)
    );
}

#[test]
fn wrapped_names_do_not_borrow_a_shorter_path_from_the_other_command_family() {
    let short = "short.tex";
    for (long_command, short_command, suffix, expected) in [
        (
            "include",
            "verbatiminput",
            "continued.tex",
            "missing-include",
        ),
        (
            "verbatiminput",
            "include",
            "continued.rs",
            "missing-verbatim-input",
        ),
    ] {
        let long = format!("{short}.{suffix}");
        let source = format!("\\{short_command}{{{short}}}\n\\{long_command}{{{long}}}\n");
        let report = format!(
            "No file {short}.\n{suffix}.\nPackage local Warning: Adjacent warning on input line 2."
        );
        let (_, _, text, items) = parse(&source, &report);
        assert_eq!(items.len(), 2, "{source}: {items:?}");
        let missing = items
            .iter()
            .find(|item| item.code == Some(NumberOrString::String(expected.into())))
            .unwrap();
        assert_eq!(
            source[text.offset(missing.range.start).unwrap()
                ..text.offset(missing.range.end).unwrap()],
            long
        );
        assert!(items.iter().all(|item| item
            .code
            .as_ref()
            .is_none_or(|code| *code == NumberOrString::String(expected.into()))));
        assert!(items
            .iter()
            .any(|item| item.message.contains("Adjacent warning")));
    }
}

#[test]
fn spelling_fixes_keep_full_names_stars_comments_and_document_versions() {
    for (wrong, right) in [
        ("smaple.rs", "sample.rs"),
        ("smaple.unknown", "sample.unknown"),
        ("CODEDOCUMETN", "CODEDOCUMENT"),
        (".hiddne", ".hidden"),
        ("样例, oen.py", "样例, one.py"),
    ] {
        let source = format!(
            "% 😀\r\n\\documentclass{{article}}\r\n\\verbatiminput * % keep\r\n{{{wrong}}}\r\n"
        );
        let (directory, main, text, items) = parse(&source, &format!("No file {wrong}."));
        let item = items[0].clone();
        std::fs::write(directory.path().join(right), r"\input{main}").unwrap();
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                code_action_literals: true,
                versioned_edits: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        workspace.open(main.clone(), source.clone(), 7);
        workspace
            .project
            .build_diagnostics
            .insert(main.clone(), [(main.clone(), vec![item.clone()])].into());
        let fixes = actions(&workspace, &main, &item);
        assert_eq!(fixes.len(), 1, "{wrong}: {fixes:?}");
        let edit = versioned_edit(&fixes[0], &main);
        assert_eq!(edit.range, item.range);
        assert_eq!(edit.new_text, right);
        let updated = text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text,
            }])
            .unwrap();
        assert_eq!(updated.source, source.replace(wrong, right));
        std::fs::write(directory.path().join(wrong), "Now present.").unwrap();
        assert!(actions(&workspace, &main, &item).is_empty());
    }
}

#[test]
fn fixes_reject_tampered_metadata_shadowed_candidates_and_project_redefinitions() {
    let (directory, main, text, items) = parse(
        r"\documentclass{article}\verbatiminput{smaple.rs}",
        "No file smaple.rs.",
    );
    let item = items[0].clone();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            code_action_literals: true,
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    std::fs::write(directory.path().join("sample.rs"), "Code.").unwrap();
    workspace.add_root(directory.path().into());
    workspace.open(main.clone(), text.source.clone(), 7);
    let store = |workspace: &mut Workspace, item: Diagnostic| {
        workspace
            .project
            .build_diagnostics
            .insert(main.clone(), [(main.clone(), vec![item])].into());
    };
    store(&mut workspace, item.clone());
    assert_eq!(actions(&workspace, &main, &item).len(), 1);
    for field in ["file", "message", "range"] {
        let mut changed = item.clone();
        match field {
            "file" => changed.data = Some(serde_json::json!({"file":"another.rs"})),
            "message" => changed.message = "Verbatim input file `different.rs` not found".into(),
            _ => changed.range.start.character += 1,
        }
        store(&mut workspace, changed.clone());
        assert!(actions(&workspace, &main, &changed).is_empty(), "{field}");
    }
    store(&mut workspace, item.clone());
    std::fs::write(directory.path().join("sample.rs.tex"), "Shadow.").unwrap();
    // The shadow itself is a different valid candidate; the hidden .rs is never offered.
    assert!(actions(&workspace, &main, &item)
        .iter()
        .all(|action| versioned_edit(action, &main).new_text != "sample.rs"));
    std::fs::remove_file(directory.path().join("sample.rs.tex")).unwrap();
    let settings = uri(directory.path(), "settings.tex");
    workspace.open(
        settings.clone(),
        r"\renewcommand{\verbatiminput}[1]{}".into(),
        1,
    );
    workspace
        .project
        .recorded_inputs
        .insert(main.clone(), [settings].into());
    assert!(actions(&workspace, &main, &item).is_empty());
}

#[test]
#[ignore = "requires pdflatex, xelatex and verbatim"]
fn real_missing_verbatim_inputs_warn_without_failing_and_fixes_restore_code() {
    for engine in ["pdflatex", "xelatex"] {
        for (wrong, right) in [
            ("smaple.rs", "sample.rs"),
            ("smaple.unknown", "sample.unknown"),
            ("CODEDOCUMETN", "CODEDOCUMENT"),
            ("样例, oen.rs", "样例, one.rs"),
        ] {
            let source=format!("% 😀\n\\documentclass{{article}}\\usepackage{{verbatim}}\\begin{{document}}Before.\n\\verbatiminput *{{{wrong}}}\nAfter.\\end{{document}}\n");
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main_path = root.join("main.tex");
            let main = uri(root, "main.tex");
            std::fs::write(&main_path, &source).unwrap();
            std::fs::write(root.join(right), "Expected code.\n").unwrap();
            let config = texquill::config::BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let outcome = build::run(&main_path, &config, &Default::default());
            assert_eq!(
                outcome.status, "success",
                "{engine}/{wrong}: {}",
                outcome.output
            );
            let items = &outcome.diagnostics[&main];
            let item = items
                .iter()
                .find(|item| {
                    item.code == Some(NumberOrString::String("missing-verbatim-input".into()))
                })
                .unwrap();
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            let mut workspace = Workspace {
                client: texquill::project::state::ClientCapabilities {
                    code_action_literals: true,
                    versioned_edits: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            workspace
                .project
                .build_diagnostics
                .insert(main.clone(), outcome.diagnostics.clone());
            let fixes = actions(&workspace, &main, item);
            assert_eq!(fixes.len(), 1, "{engine}/{wrong}: {fixes:?}");
            let edit = versioned_edit(&fixes[0], &main);
            let updated = Text::new(source)
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap();
            std::fs::write(&main_path, updated.source).unwrap();
            let rebuilt = build::run(&main_path, &config, &Default::default());
            assert_eq!(rebuilt.status, "success", "{}", rebuilt.output);
            assert!(rebuilt
                .diagnostics
                .values()
                .flatten()
                .all(|item| item.code
                    != Some(NumberOrString::String("missing-verbatim-input".into()))));
        }
    }
}
