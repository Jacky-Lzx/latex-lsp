use lsp_types::*;
use texquill::{compilation::build_log, editing, project::workspace::Workspace, text::Text};

const CODE: &str = "missing-minted-input";

fn message(name: &str) -> String {
    format!("Package minted Error: Cannot find input file \"{name}\"; inserting placeholder.")
}

fn parse(
    marked: &str,
    message: &str,
    numbered: bool,
) -> (tempfile::TempDir, Url, Text, Vec<Diagnostic>) {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let at = marked.find('¦').unwrap();
    let text = Text::new(marked.replace('¦', ""));
    std::fs::write(main.to_file_path().unwrap(), &text.source).unwrap();
    let line = text.position(at).line;
    let start = text.offset(Position::new(line, 0)).unwrap();
    let prefix = if numbered {
        format!("./main.tex:{}: ", line + 1)
    } else {
        "! ".into()
    };
    let log = format!(
        "(./main.tex\n{prefix}{message}\n\nl.{} {}\n)\n",
        line + 1,
        &text.source[start..at]
    );
    let items = build_log::parse(&main.to_file_path().unwrap(), &log)
        .remove(&main)
        .unwrap_or_default();
    (directory, main, text, items)
}

fn missing(items: &[Diagnostic]) -> Option<&Diagnostic> {
    items
        .iter()
        .find(|item| item.code == Some(NumberOrString::String(CODE.into())))
}

fn actions(workspace: &Workspace, main: &Url, item: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        main,
        item.range,
        &CodeActionContext {
            diagnostics: vec![item.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            ..Default::default()
        },
    )
}

#[test]
fn missing_minted_files_use_inner_utf16_ranges_for_native_and_verbatim_errors() {
    for name in [
        "smaple.rs",
        "README",
        "样例, one.rs",
        "😀 one.rs",
        "dir.with.dot/code.unknown",
    ] {
        for raw in [name.to_owned(), format!("\\detokenize{{{name}}}")] {
            let marked = format!(
                "% 😀\r\n\\inputminted[linenos] % language\r\n{{rust}} % path\r\n{{{raw}}}¦\r\n"
            );
            for numbered in [false, true] {
                for message in [
                    message(name),
                    message(&format!("\"{name}\"")),
                    format!("FancyVerb Error: No verbatim file \"{name}\"."),
                ] {
                    let (_, _, text, items) = parse(&marked, &message, numbered);
                    let item =
                        missing(&items).unwrap_or_else(|| panic!("{marked}/{message}: {items:?}"));
                    let at = text.source.find(name).unwrap();
                    assert_eq!(item.range, text.range(at, at + name.len()));
                    assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR));
                    assert_eq!(item.data.as_ref().unwrap()["file"], name);
                }
            }
        }
    }
}

#[test]
fn incomplete_templates_mismatched_paths_and_unsafe_calls_have_no_file_fix_code() {
    for (marked, reported) in [
        (r"\inputminted{rust}{other.rs}¦", message("smaple.rs")),
        (
            r"\inputminted{rust}{smaple.rs}¦",
            "Package minted Error: Cannot find input file \"smaple.rs\".".into(),
        ),
        (
            r"\inputminted{rust}{smaple.rs}¦",
            format!("{} Extra text.", message("smaple.rs")),
        ),
        (
            r"\inputminted{rust}{smaple.rs}¦",
            "Package minted Error: Missing Pygments output.".into(),
        ),
        (r"\inputminted*{rust}{smaple.rs}¦", message("smaple.rs")),
        (
            r"\inputminted[escapeinside=||]{rust}{smaple.rs}¦",
            message("smaple.rs"),
        ),
        (r"\inputminted{\language}{smaple.rs}¦", message("smaple.rs")),
        (
            r"\renewcommand{\inputminted}[2]{}\inputminted{rust}{smaple.rs}¦",
            message("smaple.rs"),
        ),
        (
            r"\setminted{texcomments}\inputminted{rust}{smaple.rs}¦",
            message("smaple.rs"),
        ),
        (r"% \inputminted{rust}{smaple.rs}¦", message("smaple.rs")),
        (
            r"\mintinline{tex}|\inputminted{rust}{smaple.rs}|¦",
            message("smaple.rs"),
        ),
    ] {
        let (_, _, _, items) = parse(marked, &reported, true);
        assert!(missing(&items).is_none(), "{marked}/{reported}: {items:?}");
    }
}

#[test]
fn spelling_repairs_preserve_headers_and_are_rechecked_after_global_settings_change() {
    for (wrong, right) in [
        ("smaple.rs", "sample.rs"),
        ("样例, oen.rs", "样例, one.rs"),
        ("CODEDOCUMETN", "CODEDOCUMENT"),
    ] {
        let marked = format!("% 😀\r\n\\documentclass{{article}}\r\n\\inputminted[linenos]{{rust}}% path\r\n{{\\detokenize{{{wrong}}}}}¦\r\n");
        let (directory, main, text, items) = parse(&marked, &message(wrong), true);
        let item = missing(&items).unwrap().clone();
        std::fs::write(directory.path().join(right), "\\label{data-only}").unwrap();
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                code_action_literals: true,
                versioned_edits: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        workspace.open(main.clone(), text.source.clone(), 7);
        workspace
            .project
            .build_diagnostics
            .insert(main.clone(), [(main.clone(), items)].into());
        let fixes = actions(&workspace, &main, &item);
        assert_eq!(fixes.len(), 1, "{wrong}: {fixes:?}");
        let CodeActionOrCommand::CodeAction(action) = &fixes[0] else {
            panic!()
        };
        let Some(DocumentChanges::Edits(documents)) =
            &action.edit.as_ref().unwrap().document_changes
        else {
            panic!()
        };
        assert_eq!(documents[0].text_document.version, Some(7));
        let OneOf::Left(edit) = &documents[0].edits[0] else {
            panic!()
        };
        let updated = text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        assert_eq!(updated.source, text.source.replace(wrong, right));
        let settings = Url::from_file_path(directory.path().join("settings.tex")).unwrap();
        // Recheck project options even if the old diagnostic and source range
        // are retained by the client after an unrelated file was edited.
        std::fs::write(
            directory.path().join("settings.tex"),
            r"\setminted{escapeinside=||}",
        )
        .unwrap();
        workspace.open(settings, r"\setminted{escapeinside=||}".into(), 8);
        // A recorded source can install settings without a literal input call.
        workspace.project.recorded_inputs.insert(
            main.clone(),
            [Url::from_file_path(directory.path().join("settings.tex")).unwrap()].into(),
        );
        assert!(actions(&workspace, &main, &item).is_empty(), "{wrong}");
    }
}

#[test]
#[ignore = "requires pdflatex, minted v3, and latexminted on PATH"]
fn real_missing_inputminted_files_offer_repairs_that_compile_in_both_reader_modes() {
    for raw in [true, false] {
        let options = if raw {
            "verbatim"
        } else {
            "highlightmode=immediate"
        };
        let source = format!("\\documentclass{{article}}\n\\usepackage[{options}]{{minted}}\n\\begin{{document}}\n\\inputminted{{rust}}{{smaple.rs}}\n\\end{{document}}\n");
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("_minted")).unwrap();
        let main = Url::from_file_path(root.join("main.tex")).unwrap();
        std::fs::write(root.join("main.tex"), &source).unwrap();
        std::fs::write(root.join("sample.rs"), "Code.\n").unwrap();
        let compile = || {
            std::process::Command::new("pdflatex")
                .args([
                    "-interaction=nonstopmode",
                    if raw {
                        "-no-shell-escape"
                    } else {
                        "-shell-restricted"
                    },
                    "-file-line-error",
                    "main.tex",
                ])
                .current_dir(root)
                .output()
                .unwrap()
        };
        assert!(!compile().status.success());
        let log = std::fs::read_to_string(root.join("main.log")).unwrap();
        let diagnostics = build_log::parse(&root.join("main.tex"), &log);
        let item = missing(&diagnostics[&main])
            .unwrap_or_else(|| panic!("{log}\n{diagnostics:?}"))
            .clone();
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
            .insert(main.clone(), diagnostics);
        let fixes = actions(&workspace, &main, &item);
        assert_eq!(fixes.len(), 1, "{fixes:?}");
        let CodeActionOrCommand::CodeAction(action) = &fixes[0] else {
            panic!()
        };
        let Some(DocumentChanges::Edits(documents)) =
            &action.edit.as_ref().unwrap().document_changes
        else {
            panic!()
        };
        let OneOf::Left(edit) = &documents[0].edits[0] else {
            panic!()
        };
        let updated = Text::new(source)
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        std::fs::write(root.join("main.tex"), updated.source).unwrap();
        let output = compile();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let log = std::fs::read_to_string(root.join("main.log")).unwrap();
        assert!(build_log::parse(&root.join("main.tex"), &log)
            .values()
            .flatten()
            .all(|item| item.code != Some(NumberOrString::String(CODE.into()))));
    }
}
