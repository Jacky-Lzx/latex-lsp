use latex_lsp::build_log::page_count;

#[test]
fn filenames_cannot_supply_page_counts_at_log_wrap_boundaries() {
    for extension in ["pdf", "dvi", "xdv"] {
        for quote in ["", "\""] {
            for fake in [
                "(9 pages).",
                "main.pdf (9 pages).",
                "main.xdv (9 pages, 123 bytes).",
            ] {
                let log = format!(
                    "Output written on {quote}directory/{fake}\n/actual.{extension}{quote} (1 page, 456 bytes).\nPDF statistics:\n 13 PDF objects\n"
                );
                assert_eq!(page_count(&log), Some(1), "{log}");
            }
        }
    }
}

#[test]
fn page_summaries_require_a_complete_output_filename_and_observed_count() {
    for log in [
        "Output written on directory/(9 pages).\n/actual.pdf\n",
        "Output written on directory/actual.tex (9 pages).\n",
        "Output written on \"directory/actual.pdf (9 pages).\n",
        "Output written on directory/actual.pdf (9 pages).\n/unfinished.pdf\n",
    ] {
        assert_eq!(page_count(log), None, "{log}");
    }
}

fn wrap(summary: &str, width: usize) -> String {
    summary
        .as_bytes()
        .chunks(width)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn complete_summaries_keep_all_engine_formats_and_stop_at_real_footers() {
    for extension in ["pdf", "dvi", "xdv"] {
        for quote in ["", "\""] {
            for (pages, suffix) in [(1, "page, 1 byte"), (2, "pages, 123 bytes"), (3, "pages")] {
                for width in [60, 79] {
                    let summary = wrap(
                        &format!("Output written on {quote}{}actual.{extension}{quote} ({pages} {suffix}).", "directory/".repeat(120)),
                        width,
                    );
                    for footer in [
                        "",
                        "\n",
                        "\nPDF statistics:\n 13 PDF objects",
                        "\nPDF statistics: 13 PDF objects",
                        "\nTranscript written on main.log.",
                        "\nSyncTeX written on main.synctex.gz.",
                        "\nLatexmk: All targets are up-to-date",
                        "\n)",
                    ] {
                        assert_eq!(
                            page_count(&format!("{summary}{footer}\n")),
                            Some(pages),
                            "{summary}{footer}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn page_summaries_do_not_cross_missing_output_or_exceed_reading_budgets() {
    for summary in [
        "Output written on dir/main.pdf (9 pages).\n[latex-lsp: middle of compiler output omitted]\n/actual.pdf (1 page).".into(),
        format!("Output written on {}/actual.pdf (1 page).", "x".repeat(8192)),
        format!("Output written on {}/actual.pdf (1 page).", "x\n".repeat(128)),
    ] {
        assert_eq!(page_count(&summary), None);
    }
}

#[test]
fn an_incomplete_final_summary_never_reuses_an_earlier_pass_count() {
    for (final_summary, expected) in [
        ("Output written on final.pdf (2 pages, 456 bytes).", Some(2)),
        ("Output written on final.pdf (2 pages, invalid bytes).", None),
        ("Output written on directory/main.pdf (9 pages).\n/unfinished.pdf", None),
        ("Output written on \"directory/final.pdf (9 pages).", None),
        ("Output written on final.pdf (2 pages,\n[latex-lsp: middle of compiler output omitted]\n456 bytes).", None),
    ] {
        let log = format!("Output written on earlier.pdf (5 pages, 123 bytes).\nTranscript written on earlier.log.\n{final_summary}\n");
        assert_eq!(page_count(&log), expected, "{log}");
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_engine_page_counts_ignore_wrapped_filename_examples() {
    use latex_lsp::{build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let cache = root.join("tex-cache");
    for (engine, extra_column) in [("pdflatex", -1), ("xelatex", 0), ("lualatex", 1)] {
        let executable = root.join(format!("{engine}-wrapper"));
        std::fs::write(
            &executable,
            format!("#!/bin/sh\nexport max_print_line=\"$1\"\nexport TEXMFVAR=\"$2\"\nexport TEXMFCACHE=\"$2\"\nshift 2\nexec {engine} \"$@\"\n"),
        ).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        for width in [60, 79] {
            for fake in ["(9 pages).", "main.pdf (9 pages)."] {
                let padding =
                    (width - "Output written on ".len() - fake.len()) as i32 + extra_column;
                let name = format!("{}{fake}continued", "a".repeat(padding as usize));
                let main = root.join(format!("{name}.tex"));
                std::fs::write(
                    &main,
                    "\\documentclass{article}\n\\begin{document}One page.\\end{document}\n",
                )
                .unwrap();
                let config = BuildConfig {
                    executable: executable.to_string_lossy().into_owned(),
                    output_directory: Some(".".into()),
                    arguments: Some(vec![
                        width.to_string(),
                        cache.to_string_lossy().into_owned(),
                        "-interaction=nonstopmode".into(),
                        "-no-shell-escape".into(),
                        "-file-line-error".into(),
                        "-recorder".into(),
                        "%f".into(),
                    ]),
                    ..Default::default()
                };
                let outcome = build::run(&main, &config, &Default::default());
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{width}/{fake}: {}",
                    outcome.output
                );
                let log =
                    std::fs::read_to_string(config.artifact_path(&main, "log").unwrap()).unwrap();
                let lines: Vec<_> = log.lines().collect();
                let at = lines
                    .iter()
                    .position(|line| line.starts_with("Output written on "))
                    .unwrap();
                assert!(
                    lines[at].ends_with(fake),
                    "{engine}/{width}/{fake}: {:?}",
                    &lines[at..]
                );
                assert_eq!(outcome.pages, Some(1), "{engine}/{width}/{fake}: {log}");
                assert_eq!(page_count(&log), Some(1), "{engine}/{width}/{fake}");
                assert!(outcome.pdf.is_some_and(|pdf| pdf.is_file()));
            }
        }
    }
}
