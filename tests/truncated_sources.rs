use latex_lsp::{compilation::build_log, text::Text};
use lsp_types::{DiagnosticSeverity, NumberOrString, Url};

const GAP: &str = "[latex-lsp: middle of compiler output omitted]";

#[test]
fn truncated_traditional_contexts_remain_unconfirmed_until_a_source_is_observed_again() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    let source = "😀 \\bad{x} \\ref{target}";
    std::fs::write(&main, source).unwrap();
    std::fs::write(&child, source).unwrap();
    let log = format!("(./main.tex\n(./child.tex\n{GAP}\n(note)\n! Undefined control sequence.\nl.1 😀 \\bad\n(./child.tex\n! Undefined control sequence.\nl.1 😀 \\bad\n)\nLaTeX Warning: Reference `target' undefined on input line 1.\n(./main.tex\n! Undefined control sequence.\nl.1 😀 \\bad\n)\n");
    let diagnostics = build_log::parse(&main, &log);
    let root_errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(root_errors.len(), 3, "{diagnostics:?}");
    let unknown = root_errors
        .iter()
        .filter(|d| d.data == Some(serde_json::json!({"unresolvedSource":true})))
        .collect::<Vec<_>>();
    assert_eq!(unknown.len(), 2);
    for diagnostic in unknown {
        assert_eq!(diagnostic.code, None);
        assert_eq!(
            diagnostic.range,
            Text::new(source.into()).range(0, source.len())
        );
    }
    assert!(root_errors
        .iter()
        .any(|d| d.severity == Some(DiagnosticSeverity::ERROR)
            && d.code == Some(NumberOrString::String("undefined-control-sequence".into()))));
    assert_eq!(
        diagnostics[&Url::from_file_path(&child).unwrap()][0].code,
        Some(NumberOrString::String("undefined-control-sequence".into()))
    );
}

#[test]
fn numbered_sources_after_a_truncation_still_confirm_their_own_locations() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\bad").unwrap();
    let log = format!("(./main.tex\n{GAP}\n./main.tex:1: Undefined control sequence.\nl.1 \\bad\n! Undefined control sequence.\nl.1 \\bad\n");
    let diagnostics = build_log::parse(&main, &log);
    let diagnostics = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics
        .iter()
        .any(|d| d.code == Some(NumberOrString::String("undefined-control-sequence".into()))));
    assert!(diagnostics
        .iter()
        .any(|d| d.code.is_none() && d.data == Some(serde_json::json!({"unresolvedSource":true}))));
}

#[cfg(unix)]
#[test]
fn real_bounded_compiler_streams_and_logs_do_not_borrow_root_fixes_after_omission() {
    use latex_lsp::{
        compilation::build, config::BuildConfig, editing, project::workspace::Workspace,
    };
    use lsp_types::{CodeActionContext, CodeActionKind};
    use std::{io::Write, os::unix::fs::PermissionsExt};
    for channel in ["stdout", "log"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let source = "\\documentclass{article}\\begin{document}\\bad\\end{document}";
        std::fs::write(&main, source).unwrap();
        std::fs::write(root.join("child.tex"), source).unwrap();
        let noise = root.join("noise");
        let mut file = std::fs::File::create(&noise).unwrap();
        writeln!(file, "(./main.tex\n(./child.tex").unwrap();
        let block = vec![b'.'; 4095];
        for _ in 0..2560 {
            file.write_all(&block).unwrap();
            writeln!(file).unwrap();
        }
        writeln!(file, "! Undefined control sequence.\nl.1 \\documentclass{{article}}\\begin{{document}}\\bad\n)\n)").unwrap();
        drop(file);
        let executable = root.join("compiler");
        let route = if channel == "stdout" {
            "cat \"$2\""
        } else {
            "cat \"$2\" > \"$1/main.log\""
        };
        std::fs::write(&executable, format!("#!/bin/sh\n{route}\nexit 1\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = BuildConfig {
            executable: executable.to_string_lossy().into_owned(),
            arguments: Some(vec!["%o".into(), noise.to_string_lossy().into_owned()]),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &Default::default());
        assert_eq!(outcome.status, "error");
        assert!(outcome.diagnostics_truncated);
        let uri = Url::from_file_path(&main).unwrap();
        let diagnostic = outcome.diagnostics[&uri]
            .iter()
            .find(|d| d.message == "Undefined control sequence.")
            .unwrap()
            .clone();
        assert_eq!(diagnostic.code, None, "{channel}: {diagnostic:?}");
        assert_eq!(
            diagnostic.data,
            Some(serde_json::json!({"unresolvedSource":true}))
        );
        assert_eq!(
            diagnostic.range,
            Text::new(source.into()).range(0, source.len())
        );
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
            .insert(uri.clone(), outcome.diagnostics);
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                only: Some(vec![CodeActionKind::QUICKFIX]),
                trigger_kind: None,
            },
        );
        assert!(actions.is_empty(), "{channel}: {actions:?}");
    }
}
