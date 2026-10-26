use lsp_types::*;
use texquill::{compilation::build_log, editing, project::workspace::Workspace, text::Text};

fn uri(path: &std::path::Path) -> Url {
    Url::from_file_path(path).unwrap()
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
fn actions(workspace: &Workspace, main: &Url, diagnostic: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        main,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            ..Default::default()
        },
    )
}
#[test]
fn missing_local_resources_from_biber_and_bibtex_offer_exact_versioned_filename_fixes() {
    for loader in ["addbibresource", "addglobalbib"] {
        for backend in ["Biber", "BibTeX"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = uri(&root.join("main.tex"));
            let source =
                format!("\\documentclass{{article}}\n\\{loader}[label=reffs]{{reffs.bib}}\n");
            std::fs::write(root.join("main.tex"), &source).unwrap();
            std::fs::write(root.join("refs.bib"), "@book{alpha,title={Alpha}}\n").unwrap();
            let log = if backend == "Biber" {
                "[0] Utils.pm:395> ERROR - Cannot find 'reffs.bib'!\n"
            } else {
                "I couldn't open database file reffs.bib\n---line 3 of file main.aux\n"
            };
            let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), log);
            let diagnostic = &diagnostics[&main][0];
            let start = source.find("reffs.bib").unwrap();
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + 9),
                "{backend}/{loader}: {diagnostic:?}"
            );
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String(
                    "missing-bibliography-database".into()
                ))
            );
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
                .insert(main.clone(), diagnostics.clone());
            let fixes = actions(&workspace, &main, diagnostic);
            assert_eq!(fixes.len(), 1, "{backend}/{loader}: {fixes:?}");
            let CodeActionOrCommand::CodeAction(action) = &fixes[0] else {
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
            assert_eq!(
                apply(&source, std::slice::from_ref(edit)),
                source.replace("{reffs.bib}", "{refs.bib}")
            );
        }
    }
}

fn action_edits(action: &CodeActionOrCommand) -> Vec<TextEdit> {
    let CodeActionOrCommand::CodeAction(action) = action else {
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
    changes[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect()
}

#[test]
fn biber_resource_fixes_preserve_literal_names_options_and_explicit_extensions() {
    for loader in ["addbibresource", "addglobalbib"] {
        for (missing, correct) in [
            ("refs", "refs.bib"),
            ("ref,part.bib", "refs,part.bib"),
            ("author's reffs.bib", "author's refs.bib"),
            ("refs%parr.bib", "refs%part.bib"),
            ("中文 reffs.bib", "中文 refs.bib"),
            ("reffs.data", "refs.data"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = uri(&root.join("main.tex"));
            let source=format!("% 😀\r\n\\{loader}[bibencoding={{utf8}},label={{meta, data}}]{{{missing}}}\\cite{{alpha}}\r\n");
            std::fs::write(root.join("main.tex"), &source).unwrap();
            std::fs::write(root.join(correct), "@book{alpha,title={Alpha}}").unwrap();
            let log = format!("[15] Utils.pm:395> ERROR - Cannot find '{missing}'!\n");
            let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
            let diagnostic = &diagnostics[&main][0];
            let start = source.find(missing).unwrap();
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + missing.len()),
                "{missing}: {diagnostic:?}"
            );
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
                .insert(main.clone(), diagnostics.clone());
            let fixes = actions(&workspace, &main, diagnostic);
            assert_eq!(fixes.len(), 1, "{missing}: {fixes:?}");
            assert_eq!(
                apply(&source, &action_edits(&fixes[0])),
                source.replace(&format!("{{{missing}}}"), &format!("{{{correct}}}"))
            );
            // Restored inputs and changed command bindings make old repairs obsolete.
            std::fs::write(root.join(missing), "restored").unwrap();
            assert!(actions(&workspace, &main, diagnostic).is_empty());
            std::fs::remove_file(root.join(missing)).unwrap();
            let overridden = format!("{source}\\renewcommand\\{loader}[1]{{}}\n");
            workspace.open(main.clone(), overridden, 8);
            assert!(actions(&workspace, &main, diagnostic).is_empty());
        }
    }
}

#[test]
fn malformed_biber_messages_and_unconfirmed_resource_sources_do_not_offer_edits() {
    for message in [
        "WARN - Cannot find 'reffs.bib'!",
        "ERROR - Example: Cannot find 'reffs.bib'!",
        "ERROR - Cannot find 'reffs.bib'",
        "ERROR - Cannot find 'reffs.bib'! additional text",
        "ERROR - Cannot find control file 'main.bcf'! - Did latex run successfully?",
        "ERROR - Cannot find ''!",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("main.tex"), "\\addbibresource{reffs.bib}").unwrap();
        let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), message);
        assert!(
            diagnostics.values().flatten().all(|d| d.code.is_none()),
            "{message}: {diagnostics:?}"
        );
    }
    for body in [
        "\\newcommand\\later{\\addbibresource{reffs.bib}}",
        "\\newcommand\\hold[1]{}\\hold\\addbibresource{reffs.bib}",
        "\\addbibresource[location=remote]{reffs.bib}",
        "\\addglobalbib[glob]{reffs.bib}",
        "\\addbibresource{reffs.bib}\\addglobalbib{reffs.bib}",
        "\\addbibresource{reffs.bib}\\renewcommand\\addbibresource[1]{}",
        "\\addbibresource{\\filename}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("main.tex"), body).unwrap();
        std::fs::write(root.join("refs.bib"), "@book{alpha,title={Alpha}}").unwrap();
        let main = uri(&root.join("main.tex"));
        let diagnostics = build_log::parse_bibliography(
            &root.join("main.tex"),
            "ERROR - Cannot find 'reffs.bib'!\n",
        );
        let diagnostic = &diagnostics[&main][0];
        assert_eq!(
            diagnostic.data.as_ref().unwrap()["unresolvedSource"],
            true,
            "{body}: {diagnostic:?}"
        );
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                code_action_literals: true,
                versioned_edits: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.add_root(root.into());
        workspace
            .project
            .build_diagnostics
            .insert(main.clone(), diagnostics.clone());
        assert!(actions(&workspace, &main, diagnostic).is_empty(), "{body}");
    }
}

#[test]
fn resource_candidates_are_bounded_and_recheck_diagnostic_identity() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(&root.join("main.tex"));
    let source = "\\addglobalbib{refsx.bib}";
    std::fs::write(root.join("main.tex"), source).unwrap();
    for name in [
        "refsa", "refsb", "refsc", "refsd", "refse", "refsf", "refsg",
    ] {
        std::fs::write(
            root.join(format!("{name}.bib")),
            "@book{alpha,title={Alpha}}",
        )
        .unwrap();
    }
    std::fs::write(root.join("refsx.tex"), "wrong type").unwrap();
    let diagnostics =
        build_log::parse_bibliography(&root.join("main.tex"), "ERROR - Cannot find 'refsx.bib'!\n");
    let diagnostic = &diagnostics[&main][0];
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            code_action_literals: true,
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(main.clone(), source.into(), 7);
    workspace
        .project
        .build_diagnostics
        .insert(main.clone(), diagnostics.clone());
    let fixes = actions(&workspace, &main, diagnostic);
    assert_eq!(fixes.len(), 5);
    for fix in fixes {
        let edits = action_edits(&fix);
        assert!(edits[0].new_text.ends_with(".bib"));
    }
    for data in [
        serde_json::json!({"file":"refsx.bib"}),
        serde_json::json!({"file":"wrong.bib","bibliographyCommand":"addglobalbib"}),
        serde_json::json!({"file":"refsx.bib","bibliographyCommand":"addbibresource"}),
    ] {
        let mut changed = diagnostic.clone();
        changed.data = Some(data);
        workspace
            .project
            .build_diagnostics
            .insert(main.clone(), [(main.clone(), vec![changed.clone()])].into());
        assert!(actions(&workspace, &main, &changed).is_empty());
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_missing_resources_offer_fixes_that_load_the_selected_database() {
    use std::{fs, process::Command};
    for loader in ["addbibresource", "addglobalbib"] {
        for (missing, correct) in [
            ("reffs.bib", "refs.bib"),
            ("author's reffs,part.bib", "author's refs,part.bib"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = uri(&root.join("main.tex"));
            let source = format!("\\{loader}{{{missing}}}");
            fs::write(root.join("main.tex"), &source).unwrap();
            fs::write(
                root.join(correct),
                "@book{alpha,author={Alpha, Alice},title={Alpha},year={2020}}\n",
            )
            .unwrap();
            let output = Command::new("biber")
                .args(["--tool", missing])
                .current_dir(root)
                .output()
                .unwrap();
            assert!(!output.status.success());
            let path = root.join(format!("{missing}.blg"));
            let log = fs::read_to_string(&path).unwrap();
            let diagnostics =
                build_log::parse_bibliography_log(&root.join("main.tex"), &path, &log);
            let diagnostic = diagnostics[&main]
                .iter()
                .find(|d| {
                    d.code
                        == Some(NumberOrString::String(
                            "missing-bibliography-database".into(),
                        ))
                })
                .unwrap();
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
                .insert(main.clone(), diagnostics.clone());
            let fixes = actions(&workspace, &main, diagnostic);
            assert_eq!(fixes.len(), 1, "{log}: {fixes:?}");
            let edits = action_edits(&fixes[0]);
            assert_eq!(apply(&source, &edits), source.replace(missing, correct));
            let repaired = &edits[0].new_text;
            let output = Command::new("biber")
                .args(["--tool", repaired, "--output-file", "fixed.bib"])
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(fs::read_to_string(root.join("fixed.bib"))
                .unwrap()
                .contains("alpha"));
        }
    }
}

#[test]
#[ignore = "requires pdflatex, bibtex, biblatex and logreq"]
fn real_bibtex_resource_file_fixes_restore_compilation() {
    use std::{fs, process::Command};
    for loader in ["addbibresource", "addglobalbib"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = uri(&root.join("main.tex"));
        let source=format!("\\documentclass{{article}}\n\\usepackage[backend=bibtex]{{biblatex}}\n\\{loader}{{reffs.bib}}\n\\begin{{document}}\\cite{{alpha}}\\printbibliography\\end{{document}}\n");
        fs::write(root.join("main.tex"), &source).unwrap();
        fs::write(
            root.join("refs.bib"),
            "@book{alpha,author={Alpha, Alice},title={Alpha},year={2020}}\n",
        )
        .unwrap();
        let run = |program: &str, args: &[&str]| {
            Command::new(program)
                .args(args)
                .current_dir(root)
                .output()
                .unwrap()
        };
        let tex = || {
            let output = run(
                "pdflatex",
                &["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
            );
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        };
        tex();
        let output = run("bibtex", &["main"]);
        assert!(!output.status.success());
        let log = fs::read_to_string(root.join("main.blg")).unwrap();
        let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let diagnostic = diagnostics[&main]
            .iter()
            .find(|d| {
                d.code
                    == Some(NumberOrString::String(
                        "missing-bibliography-database".into(),
                    ))
            })
            .unwrap();
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
            .insert(main.clone(), diagnostics.clone());
        let fixes = actions(&workspace, &main, diagnostic);
        assert_eq!(fixes.len(), 1, "{log}: {fixes:?}");
        fs::write(
            root.join("main.tex"),
            apply(&source, &action_edits(&fixes[0])),
        )
        .unwrap();
        tex();
        let output = run("bibtex", &["main"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        tex();
        tex();
        let log = fs::read_to_string(root.join("main.log")).unwrap();
        assert!(!log.contains("undefined"), "{log}");
        assert!(fs::metadata(root.join("main.pdf")).unwrap().len() > 1000);
    }
}
