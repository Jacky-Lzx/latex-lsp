use latex_lsp::{
    compilation::{build, build_log},
    editing,
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;
use std::path::Path;

const STYLE: &str = r#"ENTRY { title } {} {}
FUNCTION {book} { "\bibitem{" write$ cite$ write$ "}" write$ newline$ title write$ newline$ }
READ
FUNCTION {begin.bib} { "\begin{thebibliography}{9}" write$ newline$ }
EXECUTE {begin.bib}
ITERATE {call.type$}
FUNCTION {end.bib} { "\end{thebibliography}" write$ newline$ }
EXECUTE {end.bib}
"#;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn log(name: &str) -> String {
    format!("I couldn't open style file {name}.bst\n---line 2 of file main.aux\n : \\bibstyle{{{name}\n : }}\n")
}
fn style_diagnostic(root: &Path, name: &str) -> Diagnostic {
    let parsed = build_log::parse_bibliography(&root.join("main.tex"), &log(name));
    parsed
        .values()
        .flatten()
        .find(|item| item.code == Some(NumberOrString::String("missing-bibliography-style".into())))
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
fn native_missing_style_errors_select_exact_commented_single_names_with_utf16_and_suffixes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in [
        "styles/local,styel".to_string(),
        "styles/local,styel.bst".to_string(),
        format!("目录/{}style", "中".repeat(30)),
        format!("styles/local.{}", "a".repeat(110)),
    ] {
        let source=format!("% emoji😀 \\bibliographystyle{{{name}}}\r\n\\bibliographystyle{{% before phantom\r\n  {name}% tail\r\n}}\r\n");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let diagnostic = style_diagnostic(root, &name);
        let start = source.find(&format!("\n  {name}")).unwrap() + 3;
        assert_eq!(
            diagnostic.range,
            Text::new(source).range(start, start + name.len())
        );
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(
            diagnostic.data,
            Some(serde_json::json!({"file":format!("{name}.bst")}))
        );
    }
}

#[test]
fn dynamic_ambiguous_ignored_and_mismatched_style_sources_keep_unresolved_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        r"\bibliographystyle{\chosen}",
        r"\bibliographystyle{local,styel}\bibliographystyle{local,styel}",
        r"\bibliographystyle{local,styel}\bibliographystyle{\chosen}",
        r"\newcommand{\bibliographystyle}[1]{}\bibliographystyle{local,styel}",
        r"\bibliographystyle{local,styel.bst}",
        r"\bibliographystyle*{local,styel}",
        r"\bibliographystyle[note]{local,styel}",
        r"\bibliographystyle{ local,styel}",
        "\\bibliographystyle{local,sty% joined\nel}",
        r"% \bibliographystyle{local,styel}",
        r"\verb|\bibliographystyle{local,styel}|",
    ] {
        std::fs::write(root.join("main.tex"), format!("% first\n{source}\n")).unwrap();
        let diagnostic = style_diagnostic(root, "local,styel");
        assert_eq!(diagnostic.range.start.line, 0, "{source}");
        assert_eq!(
            diagnostic.data.as_ref().unwrap()["unresolvedSource"],
            true,
            "{source}"
        );
    }
}

#[test]
fn style_errors_from_source_files_warnings_and_other_tools_are_not_auxiliary_missing_styles() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), r"\bibliographystyle{local,styel}").unwrap();
    std::fs::write(root.join("local.bst"), "invalid style\n").unwrap();
    for log in [
        "I couldn't open style file local,styel.bst\n---line 1 of file local.bst\n",
        "I couldn't open style file local,styel.bst\n---line 1 of file main.tex\n",
        "Warning--I couldn't open style file local,styel.bst\n",
        "ERROR - I couldn't open style file local,styel.bst\n",
        "I couldn't open style file \\chosen.bst\n---line 2 of file main.aux\n",
        "I couldn't open style file spaced name.bst\n---line 2 of file main.aux\n",
    ] {
        let entries = build_log::parse_bibliography(&root.join("main.tex"), log);
        assert!(
            !entries.values().flatten().any(|item| item.code
                == Some(NumberOrString::String("missing-bibliography-style".into()))),
            "{log}\n{entries:?}"
        );
    }
}

#[test]
fn style_fixes_preserve_comments_commas_and_suffixes_and_legacy_edits_revalidate_versions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("styles")).unwrap();
    std::fs::write(root.join("styles/local,style.bst"), STYLE).unwrap();
    let main = uri(root, "main.tex");
    for suffix in ["", ".bst"] {
        let name = format!("styles/local,styel{suffix}");
        let source =
            format!("% emoji😀 keep\r\n\\bibliographystyle{{% header\r\n  {name}% tail\r\n}}\r\n");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let mut workspace = Workspace {
            code_action_literals: true,
            apply_edits: true,
            versioned_edits: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        workspace.open(main.clone(), source.clone(), 7);
        let diagnostic = style_diagnostic(root, &name);
        workspace.build_diagnostics.insert(
            main.clone(),
            [(main.clone(), vec![diagnostic.clone()])].into(),
        );
        let result = actions(&workspace, &main, &diagnostic);
        assert_eq!(result.len(), 1, "{result:?}");
        let updated = apply(&source, edits(&result[0], &main, 7));
        assert_eq!(updated, source.replace("local,styel", "local,style"));
        let CodeActionOrCommand::CodeAction(action) = &result[0] else {
            panic!()
        };
        assert_eq!(
            action.title,
            format!("Replace with bibliography style file `styles/local,style{suffix}`")
        );
        let modern = action.edit.clone().unwrap();
        workspace.code_action_literals = false;
        let legacy = actions(&workspace, &main, &diagnostic);
        assert_eq!(legacy.len(), 1);
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
        std::fs::write(root.join("styles/local,styel.bst"), STYLE).unwrap();
        assert!(resolve(&workspace).is_none());
        std::fs::remove_file(root.join("styles/local,styel.bst")).unwrap();
        assert!(resolve(&workspace).is_some());
        std::fs::remove_file(root.join("styles/local,style.bst")).unwrap();
        assert!(resolve(&workspace).is_none());
        std::fs::write(root.join("styles/local,style.bst"), STYLE).unwrap();
    }
}

#[test]
fn style_suggestions_are_bounded_and_reject_invalid_spaced_or_other_extension_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("styles")).unwrap();
    for name in [
        "localstyle",
        "localstylea",
        "localstyleb",
        "localstylec",
        "localstyled",
        "localstylee",
        "localstylef",
        "local style",
        "unrelated",
    ] {
        std::fs::write(root.join(format!("styles/{name}.bst")), STYLE).unwrap();
    }
    std::fs::write(root.join("styles/localstylex.tex"), "content").unwrap();
    let main = uri(root, "main.tex");
    let source = r"\bibliographystyle{styles/localstylex}";
    std::fs::write(root.join("main.tex"), source).unwrap();
    let mut workspace = Workspace {
        code_action_literals: true,
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(main.clone(), source.into(), 7);
    let diagnostic = style_diagnostic(root, "styles/localstylex");
    workspace.build_diagnostics.insert(
        main.clone(),
        [(main.clone(), vec![diagnostic.clone()])].into(),
    );
    let result = actions(&workspace, &main, &diagnostic);
    assert_eq!(result.len(), 5, "{result:?}");
    for action in &result {
        for edit in edits(action, &main, 7) {
            assert!(!edit.new_text.contains(' '));
            assert!(!edit.new_text.ends_with(".tex"));
            assert!(!edit.new_text.contains("unrelated"));
        }
    }
    let mut mismatch = diagnostic.clone();
    mismatch.data = Some(serde_json::json!({"file":"wrong.bst"}));
    workspace.build_diagnostics.insert(
        main.clone(),
        [(main.clone(), vec![mismatch.clone()])].into(),
    );
    assert!(actions(&workspace, &main, &mismatch).is_empty());
}

#[test]
#[ignore = "requires latexmk, pdflatex, xelatex and BibTeX"]
fn real_missing_style_errors_locate_only_read_sources_and_compile_after_versioned_fixes() {
    for engine in ["pdflatex", "xelatex"] {
        for suffix in ["", ".bst"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            std::fs::create_dir(root.join("styles")).unwrap();
            std::fs::write(root.join("styles/local,style.bst"), STYLE).unwrap();
            std::fs::write(
                root.join("styles/local,style.bst.bst"),
                "Invalid namesake style",
            )
            .unwrap();
            std::fs::write(
                root.join("references.bib"),
                "@book{example,title={Expected}}\n",
            )
            .unwrap();
            std::fs::write(root.join("main.tex"),"\\documentclass{article}\n\\begin{document}\n\\input{settings}\n\\cite{example}\\bibliography{references}\n\\end{document}\n").unwrap();
            let name = format!("styles/local,styel{suffix}");
            let source = format!("\\bibliographystyle{{% before\r\n  {name}% after\r\n}}\r\n");
            std::fs::write(root.join("settings.tex"), &source).unwrap();
            std::fs::write(root.join("unread.tex"), &source).unwrap();
            let main = uri(root, "main.tex");
            let settings = uri(root, "settings.tex");
            let unread = uri(root, "unread.tex");
            let config = latex_lsp::config::BuildConfig {
                engine: latex_lsp::config::Engine::from_program(engine),
                ..Default::default()
            };
            let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(outcome.status, "error", "{}", outcome.output);
            assert!(outcome.diagnostic_sources.contains(&settings));
            assert!(!outcome.diagnostic_sources.contains(&unread));
            let diagnostic = outcome.diagnostics[&settings]
                .iter()
                .find(|item| {
                    item.code == Some(NumberOrString::String("missing-bibliography-style".into()))
                })
                .unwrap()
                .clone();
            assert_eq!(
                diagnostic.data,
                Some(serde_json::json!({"file":format!("{name}.bst")}))
            );
            let start = source.find("  styles/").unwrap() + 2;
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + name.len())
            );
            let mut workspace = Workspace {
                code_action_literals: true,
                versioned_edits: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(settings.clone(), source.clone(), 7);
            workspace
                .build_diagnostics
                .insert(main.clone(), outcome.diagnostics);
            let result = actions(&workspace, &settings, &diagnostic);
            assert_eq!(result.len(), 1, "{result:?}");
            let updated = apply(&source, edits(&result[0], &settings, 7));
            assert_eq!(updated, source.replace("local,styel", "local,style"));
            std::fs::write(root.join("settings.tex"), updated).unwrap();
            let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(outcome.status, "success", "{}", outcome.output);
            assert!(
                outcome
                    .bibliography_inputs
                    .as_ref()
                    .is_some_and(|inputs| inputs.contains(&uri(root, "styles/local,style.bst"))),
                "{engine} suffix={suffix:?}, inputs={:?}, log={}",
                outcome.bibliography_inputs,
                std::fs::read_to_string(root.join("build/latex-lsp/main.blg"))
                    .unwrap_or_default()
                    .lines()
                    .take(12)
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            assert!(!outcome
                .bibliography_inputs
                .as_ref()
                .unwrap()
                .contains(&uri(root, "styles/local,style.bst.bst")));
        }
    }
}
