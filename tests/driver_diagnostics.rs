use latex_lsp::{build, build_log, config::BuildConfig};
use lsp_types::{DiagnosticSeverity, Url};
use std::{process::Command, sync::atomic::AtomicBool};

#[test]
fn pdf_driver_errors_use_the_root_without_borrowing_tex_source_context() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "Root.\nSecond root line.\n").unwrap();
    std::fs::write(&child, "Child.\nSecond child line.\n").unwrap();
    for driver in ["xdvipdfmx", "dvipdfmx"] {
        let message = format!("{driver}:fatal: Cannot proceed without a physical font.");
        let log = format!("(./main.tex\n(./child.tex\n! Earlier TeX error without punctuation\n{message}\nl.2 Second child line.\n)\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let root = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].message, message);
        assert_eq!(root[0].range.start.line, 0);
        assert_eq!(root[0].severity, Some(DiagnosticSeverity::ERROR));
        assert!(root[0].code.is_none());
        assert_eq!(
            root[0].data,
            Some(serde_json::json!({"unresolvedSource": true}))
        );
        let child_errors = &diagnostics[&Url::from_file_path(&child).unwrap()];
        assert_eq!(child_errors.len(), 1);
        assert_eq!(
            child_errors[0].message,
            "Earlier TeX error without punctuation"
        );
        assert_eq!(child_errors[0].range.start.line, 0);
    }
}

#[test]
fn driver_warning_details_group_only_same_driver_continuations() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Root.\n").unwrap();
    let log = "xdvipdfmx:warning: Could not locate a physical font.\nxdvipdfmx:warning: >> Mapped to missing-font.pfb.\nxdvipdfmx:warning: >> Please check the font map.\ndvipdfmx:warning: >> A different driver's detail.\nxdvipdfmx:warning: A separate warning.\nxdvipdfmx:fatal: Cannot render this page.\nNo output PDF file written.\n";
    let diagnostics = build_log::parse(&main, log);
    let entries = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(entries.len(), 4, "{entries:?}");
    assert!(entries.iter().any(|entry| entry.message == "xdvipdfmx:warning: Could not locate a physical font.\n>> Mapped to missing-font.pfb.\n>> Please check the font map."
        && entry.severity == Some(DiagnosticSeverity::WARNING)));
    assert!(entries
        .iter()
        .any(|entry| entry.message == "dvipdfmx:warning: >> A different driver's detail."));
    assert!(entries
        .iter()
        .any(|entry| entry.message == "xdvipdfmx:warning: A separate warning."));
    assert!(entries.iter().any(|entry| entry.message
        == "xdvipdfmx:fatal: Cannot render this page."
        && entry.severity == Some(DiagnosticSeverity::ERROR)));
}

#[test]
fn driver_headers_do_not_extend_unfinished_tex_messages_or_match_lookalikes() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Root.\n").unwrap();
    let log = "(./main.tex\nPackage audit Warning: Unfinished warning\nxdvipdfmx:warning: Driver warning.\n! Unfinished error\nxdvipdfmx:fatal: Driver failure.\n)\n";
    let diagnostics = build_log::parse(&main, log);
    let entries = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(entries.len(), 4, "{entries:?}");
    assert!(entries
        .iter()
        .any(|entry| entry.message == "Package audit Warning: Unfinished warning"));
    assert!(entries
        .iter()
        .any(|entry| entry.message == "Unfinished error"));
    let lookalikes = "xdvipdfmx:fatality: text\nxdvipdfmx:warning: \ndvipdfmx:fatal:\nother:warning: text\n% xdvipdfmx:fatal: quoted text\npath/xdvipdfmx:fatal: text\n";
    assert!(build_log::parse(&main, lookalikes).is_empty());
}

#[test]
fn grouped_driver_warning_details_obey_the_message_budget() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Root.\n").unwrap();
    let log = format!(
        "dvipdfmx:warning: First warning.\n{}dvipdfmx:fatal: Still reported.\n",
        format!("dvipdfmx:warning: >> {}\n", "x".repeat(1024)).repeat(8)
    );
    let diagnostics = build_log::parse(&main, &log);
    let entries = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert!(entries.iter().all(|entry| entry.message.len() <= 4096));
    assert!(entries
        .iter()
        .any(|entry| entry.message == "dvipdfmx:fatal: Still reported."));
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.message.matches(">>").count())
            .sum::<usize>(),
        8
    );
}

#[test]
#[cfg(unix)]
fn failed_builds_merge_only_driver_output_and_successful_passes_keep_the_final_log() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let executable = directory.path().join("audit-compiler");
    std::fs::write(&main, "Root.\n").unwrap();
    for (failed, driver_log) in [
        (false, ""),
        (true, ""),
        (true, "dvipdfmx:fatal: Driver failure.\n"),
        (true, "! Final TeX error.\nl.1 Root.\n"),
    ] {
        let script = format!("#!/usr/bin/env sh\ncat > build/latex-lsp/main.log <<'AUDIT_LOG'\n(./main.tex\n)\n{driver_log}AUDIT_LOG\nprintf '%s\\n' '%PDF-1.4' > build/latex-lsp/main.pdf\ncat <<'AUDIT_OUTPUT'\n! Stale error from an earlier TeX pass.\nl.1 Root.\nAUDIT_OUTPUT\ncat >&2 <<'AUDIT_DRIVER'\ndvipdfmx:warning: Driver warning.\ndvipdfmx:warning: >> A font mapping detail.\ndvipdfmx:fatal: Driver failure.\nAUDIT_DRIVER\nexit {}\n", u8::from(failed));
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = BuildConfig {
            executable: executable.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status,
            if failed { "error" } else { "success" },
            "{}",
            outcome.output
        );
        let entries = outcome.diagnostics.values().flatten().collect::<Vec<_>>();
        if failed {
            assert_eq!(
                entries.len(),
                if driver_log.is_empty() { 2 } else { 1 },
                "{entries:?}"
            );
            assert_eq!(
                entries
                    .iter()
                    .filter(|entry| entry.severity == Some(DiagnosticSeverity::ERROR))
                    .count(),
                1
            );
            assert_eq!(
                outcome.message,
                if driver_log.starts_with("! ") {
                    "Final TeX error."
                } else {
                    "dvipdfmx:fatal: Driver failure."
                }
            );
            if driver_log.is_empty() {
                assert!(entries
                    .iter()
                    .any(|entry| entry.message.contains("A font mapping detail")));
            }
            assert!(entries
                .iter()
                .all(|entry| !entry.message.contains("Stale error")));
        } else {
            assert!(entries.is_empty(), "{entries:?}");
        }
    }
}

#[test]
#[ignore = "requires xelatex"]
fn real_xelatex_driver_failures_have_specific_diagnostics_instead_of_exit_status() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\special{pdf:mapline cmr10 default audit-missing-font.pfb}\n\\begin{document}\n\\font\\auditfont=cmr10 \\auditfont Body.\\end{document}\n").unwrap();
    let config = BuildConfig {
        executable: "xelatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error", "{}", outcome.output);
    let errors = &outcome.diagnostics[&Url::from_file_path(&main).unwrap()];
    assert!(
        errors
            .iter()
            .any(|error| error.message.starts_with("xdvipdfmx:fatal:")
                && error.message.contains("physical")
                && error.range.start.line == 0),
        "{errors:?}\n{}",
        outcome.output
    );
    assert!(errors
        .iter()
        .all(|error| !error.message.contains("exited with exit status")));
}

#[test]
#[ignore = "requires xelatex and dvipdfmx"]
fn real_dvipdfmx_warning_details_keep_the_physical_font_name() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\special{pdf:mapline cmr10 default audit-missing-font.pfb}\n\\begin{document}\n\\font\\auditfont=cmr10 \\auditfont Body.\\end{document}\n").unwrap();
    let tex = Command::new("xelatex")
        .args([
            "-no-pdf",
            "-interaction=nonstopmode",
            "-halt-on-error",
            "main.tex",
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        tex.status.success(),
        "{}",
        String::from_utf8_lossy(&tex.stdout)
    );
    let driver = Command::new("dvipdfmx")
        .args(["-o", "main.pdf", "main.xdv"])
        .env("MKTEXPK", "0")
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(!driver.status.success());
    let log = format!(
        "{}\n{}",
        String::from_utf8_lossy(&driver.stdout),
        String::from_utf8_lossy(&driver.stderr)
    );
    let diagnostics = build_log::parse(&main, &log);
    let entries = &diagnostics[&Url::from_file_path(&main).unwrap()];
    let warning = entries
        .iter()
        .find(|entry| entry.severity == Some(DiagnosticSeverity::WARNING))
        .unwrap_or_else(|| panic!("{log}"));
    assert!(
        warning.message.contains("audit-missing-font.pfb"),
        "{}",
        warning.message
    );
    assert!(warning.message.contains("Could not locate"));
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.severity == Some(DiagnosticSeverity::WARNING))
            .count(),
        1,
        "{entries:?}"
    );
    assert!(entries
        .iter()
        .any(|entry| entry.severity == Some(DiagnosticSeverity::ERROR)
            && entry.message.starts_with("dvipdfmx:fatal:")));
}
