use latex_lsp::{compilation::build, config::BuildConfig};
use lsp_types::{DiagnosticSeverity, Url};
use std::{path::Path, sync::atomic::AtomicBool};

#[cfg(unix)]
fn compiler(root: &Path, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
#[cfg(unix)]
fn cached_latexmk_failures_keep_the_original_error_location() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\unknownLspCommand\n").unwrap();
    let config = BuildConfig {
        executable: compiler(
            root,
            "latexmk",
            r#"if [ ! -f "$1/cached-failure" ]; then
  printf '%s:2: Undefined control sequence.\nl.2 \\unknownLspCommand\n' "$2" > "$1/$3.log"
  touch "$1/cached-failure"
else
  printf "Latexmk: Nothing to do for '%s'.\n" "$2"
  printf 'pdflatex: gave an error in previous invocation of latexmk.\n'
fi
exit 12"#,
        ),
        arguments: Some(vec!["%o".into(), "%f".into(), "%j".into()]),
        output_directory: Some("build/custom output".into()),
        job_name: Some("draft.v2".into()),
        ..Default::default()
    };
    let first = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(first.status, "error");
    let uri = Url::from_file_path(&main).unwrap();
    assert_eq!(first.diagnostics[&uri][0].range.start.line, 1);
    assert!(first.diagnostics[&uri][0]
        .message
        .contains("Undefined control sequence"));
    for _ in 0..2 {
        let repeated = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(repeated.status, "error");
        assert_eq!(repeated.exit_code, Some(12));
        assert_eq!(repeated.diagnostics, first.diagnostics);
        assert_eq!(repeated.log, first.log);
        assert_eq!(repeated.analysis, first.analysis);
    }
}

#[test]
#[cfg(unix)]
fn an_unmodified_log_is_not_used_after_an_unrelated_compiler_failure() {
    for name in ["compiler", "latexmk"] {
        for mode in ["failure", "timeout", "spawn"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            std::fs::write(&main, "\\documentclass{article}\n\\unknownLspCommand\n").unwrap();
            let config = BuildConfig {
                executable: compiler(
                    root,
                    name,
                    if mode == "timeout" {
                        "printf \"Latexmk: Nothing to do for 'main.tex'.\\n\"\nsleep 30"
                    } else {
                        "printf 'invalid compiler option\\n' >&2\nexit 2"
                    },
                ),
                timeout_seconds: 1,
                ..Default::default()
            };
            let log = config.artifact_path(&main, "log").unwrap();
            std::fs::create_dir_all(log.parent().unwrap()).unwrap();
            std::fs::write(&log, "./main.tex:2: Undefined control sequence.\n").unwrap();
            if mode == "spawn" {
                std::fs::remove_file(&config.executable).unwrap();
            }
            let outcome = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(
                outcome.status,
                if mode == "timeout" {
                    "timeout"
                } else {
                    "error"
                }
            );
            assert!(outcome.log.is_none(), "{name}/{mode}: {}", outcome.message);
            assert!(outcome.analysis.is_none());
            assert!(outcome
                .diagnostics
                .values()
                .flatten()
                .all(|item| !item.message.contains("Undefined control sequence")));
        }
    }
}

#[test]
#[ignore = "requires latexmk, pdflatex and xelatex"]
fn real_latexmk_repeated_builds_keep_diagnostics_and_clear_repaired_errors() {
    for engine in [
        latex_lsp::config::Engine::PdfLatex,
        latex_lsp::config::Engine::XeLatex,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let child = root.join("chapter.tex");
        std::fs::write(
            &main,
            "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
        )
        .unwrap();
        let valid = "Hello.\n\\PackageWarning{example}{Persistent warning}\n";
        std::fs::write(&child, valid).unwrap();
        let config = BuildConfig {
            engine: Some(engine),
            output_directory: Some("build/custom output".into()),
            job_name: Some("draft.v2".into()),
            ..Default::default()
        };
        let initial = build::run(&main, &config, &Default::default());
        assert_eq!(initial.status, "success", "{}", initial.output);
        let noop = build::run(&main, &config, &Default::default());
        assert!(noop.output.contains("Nothing to do"), "{}", noop.output);
        assert_eq!(noop.status, "success", "{}", noop.output);
        assert_eq!(noop.diagnostics, initial.diagnostics);
        assert_eq!(noop.pages, Some(1));
        assert_eq!(noop.analysis, initial.analysis);
        std::fs::write(&child, format!("{valid}\\unknownLspCommand\n")).unwrap();
        let failed = build::run(&main, &config, &Default::default());
        assert_eq!(failed.status, "error", "{}", failed.output);
        let uri = Url::from_file_path(&child).unwrap();
        assert!(
            failed.diagnostics[&uri]
                .iter()
                .any(|item| item.range.start.line == 2
                    && item.severity == Some(DiagnosticSeverity::ERROR)
                    && item.message.contains("Undefined control sequence")),
            "{:?}",
            failed.diagnostics
        );
        let log = config.artifact_path(&main, "log").unwrap();
        let failed_log = std::fs::read(&log).unwrap();
        for _ in 0..2 {
            let repeated = build::run(&main, &config, &Default::default());
            assert!(
                repeated.output.contains("Nothing to do"),
                "{}",
                repeated.output
            );
            assert_eq!(repeated.status, "error");
            assert_eq!(repeated.exit_code, Some(12));
            assert_eq!(repeated.diagnostics, failed.diagnostics);
            assert_eq!(repeated.log, failed.log);
            assert_eq!(repeated.analysis, failed.analysis);
            assert_eq!(std::fs::read(&log).unwrap(), failed_log);
        }
        std::fs::write(&child, valid).unwrap();
        for _ in 0..2 {
            let repaired = build::run(&main, &config, &Default::default());
            assert_eq!(repaired.status, "success", "{}", repaired.output);
            assert_eq!(repaired.diagnostics, initial.diagnostics);
            assert_eq!(repaired.pages, Some(1));
        }
    }
}
