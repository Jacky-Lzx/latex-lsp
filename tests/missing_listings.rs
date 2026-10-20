use latex_lsp::{build, build_log, editing, text::Text, workspace::Workspace};
use std::path::Path;
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn parse(
    marked: &str,
    reported: &str,
    numbered: bool,
    frames: &str,
    interrupted: &str,
) -> (tempfile::TempDir, Url, Text, Vec<Diagnostic>) {
    let directory = tempfile::tempdir().unwrap();
    let end = marked.find('¦').unwrap();
    let text = Text::new(marked.replace('¦', ""));
    let main = directory.path().join("main.tex");
    std::fs::write(&main, &text.source).unwrap();
    let line = text.position(end).line;
    let line_start = text.offset(Position::new(line, 0)).unwrap();
    let number = line + 1;
    let emergency = if numbered {
        format!("./main.tex:{number}: Emergency stop.")
    } else {
        "! Emergency stop.".into()
    };
    let log = format!("(./main.tex\n! Package Listings Error: File `{reported}' not found.\n\nType X to quit or <RETURN> to proceed,\nor enter new name. (Default extension: rs)\n\nEnter file name: \n{interrupted}{emergency}\n<read *>\n{frames}l.{number} {}\n)\n",&text.source[line_start..end]);
    let main_uri = uri(directory.path(), "main.tex");
    let items = build_log::parse(&main, &log)
        .remove(&main_uri)
        .unwrap_or_default();
    (directory, main_uri, text, items)
}

fn missing(items: &[Diagnostic]) -> Option<&Diagnostic> {
    items
        .iter()
        .find(|item| item.code == Some(NumberOrString::String("missing-listing".into())))
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

fn edits(action: &CodeActionOrCommand, main: &Url) -> Vec<TextEdit> {
    let CodeActionOrCommand::CodeAction(action) = action else {
        panic!()
    };
    let edit = action.edit.as_ref().unwrap();
    if let Some(changes) = &edit.changes {
        return changes[main].clone();
    }
    let Some(DocumentChanges::Edits(documents)) = &edit.document_changes else {
        panic!()
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *main);
    assert_eq!(documents[0].text_document.version, Some(7));
    documents[0]
        .edits
        .iter()
        .map(|item| match item {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect()
}

#[test]
fn listing_prompts_locate_exact_file_arguments_for_both_emergency_stop_formats() {
    for (name, report) in [
        ("smaple.rs", "smaple(.rs)"),
        (
            "MissingCodeDocumentWithoutSuffix",
            "MissingCodeDocumentWithoutSuffix(.tex)",
        ),
        (".hidden", "(.hidden)"),
        ("multi.part.rs", "multi.part(.rs)"),
        ("dir.with.dot/missing.rs", "dir.with.dot/missing(.rs)"),
        ("code,one/my code.rs", "code,one/my code(.rs)"),
        ("name.", "name(.)"),
        ("parent.dir/README", "parent.dir/README(.tex)"),
        ("样例.py", "样例(.py)"),
    ] {
        for numbered in [false, true] {
            for marked in [format!("% 😀\n\\lstinputlisting[title={{A,B [ C]}}]{{{name}}}¦ after"),format!("% 😀\r\n\\lstinputlisting % keep\r\n[columns=fullflexible] % keep\r\n{{{name}}}¦\r\n")] {
                let (_directory,_main,text,items) = parse(&marked,report,numbered,"","");
                let item = missing(&items).unwrap_or_else(||panic!("{marked}: {items:?}"));
                let start = text.source.find(name).unwrap();
                assert_eq!(item.range,text.range(start,start+name.len()));
                assert_eq!(item.data,Some(serde_json::json!({"file":name})));
                assert_eq!(item.severity,Some(DiagnosticSeverity::ERROR));
            }
        }
    }
}

#[test]
fn uncertain_listing_contexts_keep_generic_errors_without_path_corrections() {
    for (marked, frames, interrupted) in [
        (r"\lstinputlisting*{smaple.rs}¦", "", ""),
        (r"\lstinputlisting[a][b]{smaple.rs}¦", "", ""),
        (r"\lstinputlisting[title=[nested]]{smaple.rs}¦", "", ""),
        (r"\lstinputlisting[inputpath=other]{smaple.rs}¦", "", ""),
        (r"\lstinputlisting[style=custom]{smaple.rs}¦", "", ""),
        (r"\lstinputlisting{\macro.rs}¦", "", ""),
        (r"\lstinputlisting{smaple.rs}\later¦", "", ""),
        (
            r"\lstinputlisting{smaple.rs}¦",
            "<argument> \\expanded\n",
            "",
        ),
        (
            r"\lstinputlisting{smaple.rs}¦",
            "",
            "LaTeX Warning: unrelated message.\n",
        ),
        (
            r"\renewcommand{\lstinputlisting}[1]{}\lstinputlisting{smaple.rs}¦",
            "",
            "",
        ),
        (
            r"\lstset{inputpath=other}\lstinputlisting{smaple.rs}¦",
            "",
            "",
        ),
        (r"% \lstinputlisting{smaple.rs}¦", "", ""),
        (r"\verb|\lstinputlisting{smaple.rs}|¦", "", ""),
    ] {
        let (_directory, _main, _text, items) =
            parse(marked, "smaple(.rs)", true, frames, interrupted);
        assert!(missing(&items).is_none(), "{marked}: {items:?}");
    }
}

#[test]
fn listing_spelling_fixes_preserve_full_names_options_and_versioned_ranges() {
    for (wrong, right, report) in [
        ("smaple.rs", "sample.rs", "smaple(.rs)"),
        ("smaple.unknown", "sample.unknown", "smaple(.unknown)"),
        ("CODEDOCUMETN", "CODEDOCUMENT", "CODEDOCUMETN(.tex)"),
        (".hiddne", ".hidden", "(.hiddne)"),
        ("样例, oen.py", "样例, one.py", "样例, oen(.py)"),
    ] {
        let marked = format!("% 😀\r\n\\documentclass{{article}}\r\n\\lstinputlisting[columns=fullflexible] % keep\r\n{{{wrong}}}¦\r\n");
        let (directory, main, text, items) = parse(&marked, report, true, "", "");
        let item = missing(&items).unwrap().clone();
        std::fs::write(directory.path().join(right), r"\input{main}").unwrap();
        let mut workspace = Workspace {
            code_action_literals: true,
            versioned_edits: true,
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        workspace.open(main.clone(), text.source.clone(), 7);
        workspace
            .build_diagnostics
            .insert(main.clone(), [(main.clone(), vec![item.clone()])].into());
        let fixes = actions(&workspace, &main, &item);
        assert_eq!(fixes.len(), 1, "{wrong}: {fixes:?}");
        let edit = edits(&fixes[0], &main).remove(0);
        assert_eq!(edit.range, item.range);
        assert_eq!(edit.new_text, right);
        let updated = text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text,
            }])
            .unwrap();
        assert_eq!(
            updated.source,
            marked.replace('¦', "").replace(wrong, right)
        );
        std::fs::write(directory.path().join(wrong), "Now present.").unwrap();
        assert!(
            actions(&workspace, &main, &item).is_empty(),
            "restored files must invalidate the correction"
        );
    }
}

#[test]
fn listing_fixes_reject_tampered_metadata_and_tex_suffix_shadowing() {
    let (directory, main, text, items) = parse(
        r"\documentclass{article}\lstinputlisting{smaple.rs}¦",
        "smaple(.rs)",
        true,
        "",
        "",
    );
    let item = missing(&items).unwrap().clone();
    std::fs::write(directory.path().join("sample.rs"), "Intended.").unwrap();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(directory.path().into());
    workspace.open(main.clone(), text.source, 7);
    workspace
        .build_diagnostics
        .insert(main.clone(), [(main.clone(), vec![item.clone()])].into());
    assert_eq!(actions(&workspace, &main, &item).len(), 1);
    for field in ["metadata", "range", "message"] {
        let mut forged = item.clone();
        match field {
            "metadata" => forged.data = Some(serde_json::json!({"file":"different.rs"})),
            "range" => forged.range.start.character += 1,
            _ => forged.message = "Package Listings Error: File `different(.rs)' not found.".into(),
        }
        workspace
            .build_diagnostics
            .insert(main.clone(), [(main.clone(), vec![forged.clone()])].into());
        assert!(actions(&workspace, &main, &forged).is_empty(), "{field}");
    }
    workspace
        .build_diagnostics
        .insert(main.clone(), [(main.clone(), vec![item.clone()])].into());
    std::fs::write(
        directory.path().join("sample.rs.tex"),
        "TeX preferred decoy.",
    )
    .unwrap();
    assert!(actions(&workspace, &main, &item).is_empty());
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_missing_listing_fixes_compile_the_intended_code_file() {
    for engine in ["pdflatex", "xelatex"] {
        for (wrong, right) in [
            ("smaple.rs", "sample.rs"),
            ("smaple.unknown", "sample.unknown"),
            ("CODEDOCUMETN", "CODEDOCUMENT"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = uri(root, "main.tex");
            let source = format!("% 😀\n\\documentclass{{article}}\\usepackage{{listings}}\\begin{{document}}\n\\lstinputlisting[columns=fullflexible]{{{wrong}}}\n\\end{{document}}\n");
            std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
            std::fs::write(root.join(right), "Original code\n").unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let first = build::run(&main.to_file_path().unwrap(), &config, &Default::default());
            assert_eq!(first.status, "error", "{}", first.output);
            let diagnostic = missing(&first.diagnostics[&main]).unwrap().clone();
            let text = Text::new(source.clone());
            let start = source.find(wrong).unwrap();
            assert_eq!(diagnostic.range, text.range(start, start + wrong.len()));
            let mut workspace = Workspace {
                code_action_literals: true,
                versioned_edits: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            workspace
                .build_diagnostics
                .insert(main.clone(), first.diagnostics);
            let fixes = actions(&workspace, &main, &diagnostic);
            assert_eq!(fixes.len(), 1, "{engine}/{wrong}: {fixes:?}");
            let edit = edits(&fixes[0], &main).remove(0);
            let updated = text
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap();
            std::fs::write(main.to_file_path().unwrap(), updated.source).unwrap();
            let fixed = build::run(&main.to_file_path().unwrap(), &config, &Default::default());
            assert_eq!(fixed.status, "success", "{}", fixed.output);
            assert!(
                fixed
                    .diagnostics
                    .values()
                    .flatten()
                    .all(|item| item.severity != Some(DiagnosticSeverity::ERROR)),
                "{:?}",
                fixed.diagnostics
            );
        }
    }
}

#[test]
fn wrapped_listing_reports_continue_past_periods_inside_filenames() {
    let (_directory, _main, text, items) = parse(
        r"\lstinputlisting{prefix.part.smaple.rs}¦",
        "prefix.\npart.smaple(.rs)",
        true,
        "",
        "",
    );
    let diagnostic = missing(&items).unwrap_or_else(|| panic!("{items:?}"));
    let start = text.source.find("prefix.part.smaple.rs").unwrap();
    assert_eq!(
        diagnostic.range,
        text.range(start, start + "prefix.part.smaple.rs".len())
    );
    assert_eq!(
        diagnostic.data,
        Some(serde_json::json!({"file":"prefix.part.smaple.rs"}))
    );
    assert_eq!(
        diagnostic.message,
        "Package Listings Error: File `prefix.part.smaple(.rs)' not found."
    );
}
