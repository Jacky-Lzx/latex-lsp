#![cfg(unix)]

use latex_lsp::{build, config::BuildConfig};
use std::{io::Write, os::unix::fs::PermissionsExt, sync::atomic::AtomicBool};
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

fn run(channel: &str, failure: bool) -> (tempfile::TempDir, build::Outcome) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "First.\nSecond.\nThird.\n").unwrap();
    std::fs::write(root.join("early.tex"), "Early.\n").unwrap();
    std::fs::write(
        root.join("references.bib"),
        "@book{one,title={One}}\n@book{one,title={Two}}\n",
    )
    .unwrap();
    let noise = root.join("large output");
    let mut file = std::fs::File::create(&noise).unwrap();
    writeln!(
        file,
        "(./early.tex\nPackage early Warning: Beginning retained on input line 1."
    )
    .unwrap();
    // Both the 4 MiB pipe cap and the 8 MiB log cap are exceeded. A hidden
    // closing parenthesis means the prefix's file stack cannot reach the tail.
    let block = vec![b'.'; 4095];
    for index in 0..2560 {
        file.write_all(&block).unwrap();
        writeln!(file).unwrap();
        if index == 1280 {
            writeln!(file, ")").unwrap();
        }
    }
    writeln!(
        file,
        "Package final Warning: Ending retained on input line 3."
    )
    .unwrap();
    if failure {
        writeln!(file, "main.tex:2: Final compiler failure.").unwrap();
        if channel == "bibliography" {
            writeln!(file, "Repeated entry---line 2 of file references.bib").unwrap();
        }
    }
    writeln!(file, "Output written on main.pdf (2 pages, 9 bytes).").unwrap();
    drop(file);
    let executable = root.join("compiler");
    let route = match channel {
        "stdout" => "cat \"$2\"",
        "stderr" => "cat \"$2\" >&2",
        "log" => "cat \"$2\" > \"$1/main.log\"",
        "bibliography" => "cat \"$2\" > \"$1/main.blg\"\nprintf 'Database file #1: references.bib\\n' > \"$1/other.blg\"\ncat \"$2\" > \"$1/main.fls\"",
        _ => panic!(),
    };
    std::fs::write(
        &executable,
        format!("#!/bin/sh\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n{route}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = BuildConfig {
        executable: executable.to_string_lossy().into_owned(),
        arguments: Some(vec!["%o".into(), noise.to_string_lossy().into_owned()]),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    (directory, outcome)
}

#[test]
fn large_pipe_and_log_outputs_keep_late_errors_and_reset_missing_file_context() {
    for channel in ["stdout", "stderr", "log"] {
        let (directory, outcome) = run(channel, true);
        assert_eq!(outcome.status, "error", "{channel}: {}", outcome.output);
        assert!(outcome.diagnostics_truncated, "{channel}");
        assert!(outcome.pdf.is_none());
        let root = directory.path();
        let main = Url::from_file_path(root.join("main.tex")).unwrap();
        let early = Url::from_file_path(root.join("early.tex")).unwrap();
        assert!(outcome.diagnostics[&early]
            .iter()
            .any(|entry| entry.message.contains("Beginning retained")));
        assert!(outcome.diagnostics[&main]
            .iter()
            .any(|entry| entry.message.contains("Ending retained") && entry.range.start.line == 2));
        assert!(outcome.diagnostics[&main]
            .iter()
            .any(|entry| entry.severity == Some(DiagnosticSeverity::ERROR)
                && entry.range.start.line == 1));
        assert!(outcome.message.contains("Final compiler failure"));
        assert_eq!(outcome.pages, None);
    }
}

#[test]
fn large_pipe_and_log_outputs_keep_final_page_counts_and_output_tails() {
    for channel in ["stdout", "stderr", "log"] {
        let (_directory, outcome) = run(channel, false);
        assert_eq!(outcome.status, "success", "{channel}: {}", outcome.output);
        assert!(outcome.diagnostics_truncated, "{channel}");
        assert!(outcome.pdf.is_some());
        assert_eq!(outcome.pages, Some(2), "{channel}");
        if channel != "log" {
            assert!(outcome.output.contains("Ending retained"));
            assert!(outcome.output.contains("Output written on main.pdf"));
            assert!(outcome.output.chars().count() <= 8192);
        }
    }
}

#[test]
fn truncated_bibliography_errors_survive_and_partial_dependencies_are_not_authoritative() {
    let (directory, outcome) = run("bibliography", true);
    assert_eq!(outcome.status, "error");
    assert!(outcome.diagnostics_truncated);
    assert!(outcome.inputs.is_none());
    assert!(outcome.bibliography_inputs.is_none());
    let bib = Url::from_file_path(directory.path().join("references.bib")).unwrap();
    assert!(outcome.diagnostics[&bib]
        .iter()
        .any(|entry| entry.message.contains("Repeated entry") && entry.range.start.line == 1));
}

#[test]
#[ignore = "requires pdflatex"]
fn real_tex_logs_exceeding_eight_megabytes_keep_final_warnings_errors_and_page_counts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let preamble = "\\documentclass{article}\n\\newcount\\noise\\noise=0\n\\loop\\ifnum\\noise<140000\\typeout{progress:abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuv}\\advance\\noise by 1\\repeat\n\\begin{document}\n";
    for failure in [false, true] {
        let body = if failure {
            "\\missingFinalCommand"
        } else {
            "\\PackageWarning{final}{Last warning retained}Text."
        };
        std::fs::write(&main, format!("{preamble}{body}\n\\end{{document}}\n")).unwrap();
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        let log = config.artifact_path(&main, "log").unwrap();
        assert!(std::fs::metadata(log).unwrap().len() > 8 * 1024 * 1024);
        assert!(outcome.diagnostics_truncated);
        let uri = Url::from_file_path(&main).unwrap();
        if failure {
            assert_eq!(outcome.status, "error", "{}", outcome.output);
            assert!(outcome.diagnostics[&uri]
                .iter()
                .any(|entry| entry.message.contains("Undefined control sequence")
                    && entry.range.start.line == 4));
            assert!(outcome.pdf.is_none());
        } else {
            assert_eq!(outcome.status, "success", "{}", outcome.output);
            assert_eq!(outcome.pages, Some(1));
            assert!(outcome.diagnostics[&uri]
                .iter()
                .any(|entry| entry.message.contains("Last warning retained")
                    && entry.range.start.line == 4));
            assert!(outcome.pdf.is_some());
        }
    }
}
