#![cfg(unix)]

use lsp_types::DiagnosticSeverity;
use std::{
    fs::FileTimes,
    sync::atomic::AtomicBool,
    time::{Duration, SystemTime},
};
use texquill::{
    compilation::{build, build_log},
    config::BuildConfig,
};

fn run(log: &str, channel: &str) -> (tempfile::TempDir, build::Outcome) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let output = root.join("build/texquill");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(
        &main,
        "\\documentclass{article}\\begin{document}\\end{document}\n",
    )
    .unwrap();
    let pdf = output.join("main.pdf");
    std::fs::write(&pdf, "%PDF-1.4\nprevious output\n").unwrap();
    let now = SystemTime::now();
    for (path, age) in [(&main, 120), (&pdf, 60)] {
        std::fs::File::open(path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(now - Duration::from_secs(age)))
            .unwrap();
    }
    let fixture = root.join("fixture.log");
    std::fs::write(&fixture, log).unwrap();
    let script = match channel {
        "stdout" => "cat \"$2\"",
        "stderr" => "cat \"$2\" >&2",
        "log" => "cat \"$2\" > \"$1/main.log\"",
        "log_after_pages" => {
            "printf 'Output written on earlier.pdf (5 pages).\\n'\ncat \"$2\" > \"$1/main.log\""
        }
        "log_after_no_pages" => "printf 'No pages of output.\\n'\ncat \"$2\" > \"$1/main.log\"",
        _ => panic!("unknown channel"),
    };
    let config = BuildConfig {
        executable: "/bin/sh".into(),
        arguments: Some(vec![
            "-c".into(),
            script.into(),
            "compiler".into(),
            "%o".into(),
            fixture.to_string_lossy().into_owned(),
        ]),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    (directory, outcome)
}

#[test]
fn explicit_no_output_reports_reject_old_pdfs_without_deleting_them() {
    for report in [
        "No pages of output.",
        "warning  (pdf backend): no pages of output.",
        "warning (pdf backend): no pages of output.",
    ] {
        for channel in ["stdout", "stderr", "log"] {
            let (directory, outcome) = run(&format!("{report}\n"), channel);
            assert_eq!(outcome.exit_code, Some(0));
            assert_eq!(
                outcome.status, "error",
                "{channel}/{report}: {}",
                outcome.message
            );
            assert!(outcome.pdf.is_none());
            assert!(outcome.pages.is_none());
            assert!(!outcome.pdf_updated);
            assert!(outcome.message.contains("no pages of output"));
            assert!(outcome
                .diagnostics
                .values()
                .flatten()
                .any(|item| item.severity == Some(DiagnosticSeverity::ERROR)));
            assert_eq!(
                std::fs::read_to_string(directory.path().join("build/texquill/main.pdf")).unwrap(),
                "%PDF-1.4\nprevious output\n"
            );
        }
    }
}

#[test]
fn a_final_no_output_report_clears_an_earlier_pass_page_count() {
    for report in [
        "No pages of output.",
        "warning  (pdf backend): no pages of output.",
    ] {
        let log = format!("Output written on main.pdf (5 pages, 123 bytes).\nTranscript written on main.log.\n{report}\n");
        assert_eq!(build_log::page_count(&log), None, "{log}");
        let (_directory, outcome) = run(&log, "log");
        assert_eq!(outcome.status, "error");
        assert!(outcome.pdf.is_none());
        assert!(outcome.pages.is_none());
    }
}

#[test]
fn earlier_passes_and_warning_examples_do_not_reject_a_final_output() {
    for (log, pages) in [
        (
            "No pages of output.\nOutput written on main.pdf (1 page, 123 bytes).\n",
            Some(1),
        ),
        (
            "warning  (pdf backend): no pages of output.\nOutput written on main.pdf (2 pages).\n",
            Some(2),
        ),
        (
            "Package local Warning: Example: No pages of output.\n",
            None,
        ),
        ("No pages of output. Example text.\n", None),
        (
            "warning  (pdf backend): no pages of output. Example text.\n",
            None,
        ),
        (
            "Output written on \"No pages of output.pdf\" (3 pages).\n",
            Some(3),
        ),
    ] {
        let (_directory, outcome) = run(log, "log");
        assert_eq!(outcome.status, "success", "{log}: {}", outcome.message);
        assert!(outcome.pdf.is_some());
        assert_eq!(outcome.pages, pages);
    }
}

#[test]
fn the_observed_final_log_takes_precedence_over_earlier_pipe_output() {
    let (_directory, empty) = run("No pages of output.\n", "log_after_pages");
    assert_eq!(empty.status, "error");
    assert!(empty.pdf.is_none());
    assert!(empty.pages.is_none());
    let (_directory, output) = run(
        "Output written on main.pdf (2 pages).\n",
        "log_after_no_pages",
    );
    assert_eq!(output.status, "success", "{}", output.message);
    assert!(output.pdf.is_some());
    assert_eq!(output.pages, Some(2));
}

#[test]
fn an_omitted_later_pass_cannot_prove_that_a_pdf_was_not_produced() {
    for report in [
        "No pages of output.",
        "warning  (pdf backend): no pages of output.",
    ] {
        for (log, expected) in [
            (
                format!(
                    "{report}\n[texquill: middle of compiler output omitted]\nPDF statistics:\n"
                ),
                "success",
            ),
            (
                format!("[texquill: middle of compiler output omitted]\n{report}\n"),
                "error",
            ),
        ] {
            let (_directory, outcome) = run(&log, "log");
            assert_eq!(outcome.status, expected, "{log}: {}", outcome.message);
            assert!(outcome.pages.is_none());
            assert_eq!(outcome.pdf.is_some(), expected == "success");
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_empty_documents_do_not_reuse_a_pdf_and_can_build_again_after_adding_text() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let cache = root.join("tex-cache");
    let with_page = "\\documentclass{article}\n\\begin{document}\n\\typeout{No pages of output.}\n\\typeout{warning  (pdf backend): no pages of output.}\nOne page.\\end{document}\n";
    let without_page = "\\documentclass{article}\n\\begin{document}\\end{document}\n";
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let executable = root.join(format!("{engine}-wrapper"));
        std::fs::write(&executable, format!("#!/bin/sh\nexport max_print_line=\"$1\"\nexport TEXMFVAR=\"$2\"\nexport TEXMFCACHE=\"$2\"\nshift 2\nexec {engine} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        for width in [60, 79] {
            std::fs::write(&main, with_page).unwrap();
            let old_time = SystemTime::now() - Duration::from_secs(120);
            std::fs::File::open(&main)
                .unwrap()
                .set_times(FileTimes::new().set_modified(old_time))
                .unwrap();
            let config = BuildConfig {
                executable: executable.to_string_lossy().into_owned(),
                arguments: Some(vec![
                    width.to_string(),
                    cache.to_string_lossy().into_owned(),
                    "-interaction=nonstopmode".into(),
                    "-no-shell-escape".into(),
                    "-file-line-error".into(),
                    "-recorder".into(),
                    "-output-directory=%o".into(),
                    "%f".into(),
                ]),
                ..Default::default()
            };
            let initial = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(
                initial.status, "success",
                "{engine}/{width}: {}",
                initial.output
            );
            assert_eq!(initial.pages, Some(1));
            let pdf = initial.pdf.unwrap();
            let previous = std::fs::read(&pdf).unwrap();
            std::fs::write(&main, without_page).unwrap();
            std::fs::File::open(&main)
                .unwrap()
                .set_times(FileTimes::new().set_modified(old_time))
                .unwrap();
            let empty = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(empty.exit_code, Some(0));
            assert_eq!(empty.status, "error", "{engine}/{width}: {}", empty.output);
            assert!(
                empty.message.contains("no pages of output"),
                "{}",
                empty.message
            );
            assert!(empty.pdf.is_none());
            assert!(empty.pages.is_none());
            assert!(!empty.pdf_updated);
            assert_eq!(std::fs::read(&pdf).unwrap(), previous);
            assert!(empty
                .diagnostics
                .values()
                .flatten()
                .any(|item| item.severity == Some(DiagnosticSeverity::ERROR)));
            std::fs::write(&main, with_page).unwrap();
            let restored = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(
                restored.status, "success",
                "{engine}/{width}: {}",
                restored.output
            );
            assert_eq!(restored.pages, Some(1));
            assert!(restored.pdf.is_some());
        }
    }
}
