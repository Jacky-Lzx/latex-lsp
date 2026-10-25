use latex_lsp::{
    compilation::{build, build_log},
    config::BuildConfig,
    text::Text,
};
use lsp_types::{DiagnosticSeverity, Url};
use std::sync::atomic::AtomicBool;

#[test]
fn specific_causes_replace_fatal_summaries_across_sources_and_message_orderings() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "Main.\nSecond root line.\n").unwrap();
    std::fs::write(&child, "Child.\nSecond child line.\n").unwrap();
    let child_uri = Url::from_file_path(&child).unwrap();
    for numbered in [false, true] {
        let cause = if numbered {
            "./child.tex:2: Missing font resource."
        } else {
            "! Missing font resource."
        };
        let cause = format!("(./child.tex\n{cause}\n)\n");
        for summary in [
            "!  ==> Fatal error occurred, no output PDF file produced!",
            "./main.tex:1: ==> Fatal error occurred, no output PDF file produced!",
        ] {
            for summary_first in [false, true] {
                let log = if summary_first {
                    format!("(./main.tex\n{summary}\n{cause})\n")
                } else {
                    format!("(./main.tex\n{cause}{summary}\n)\n")
                };
                let diagnostics = build_log::parse(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{log}\n{diagnostics:?}");
                let entries = &diagnostics[&child_uri];
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].message, "Missing font resource.");
                assert_eq!(entries[0].range.start.line, u32::from(numbered));
            }
        }
    }
}

#[test]
fn fatal_summary_is_kept_when_it_is_the_only_known_error() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Main.\nSecond.\n").unwrap();
    for warning in ["", "Package audit Warning: Cosmetic issue.\n"] {
        for header in ["!  ", "./main.tex:2: "] {
            let log = format!("(./main.tex\n{warning}{header}==> Fatal error occurred, no output PDF file produced!\n)\n");
            let diagnostics = build_log::parse(&main, &log);
            let errors = diagnostics
                .values()
                .flatten()
                .filter(|entry| entry.severity == Some(DiagnosticSeverity::ERROR))
                .collect::<Vec<_>>();
            assert_eq!(errors.len(), 1, "{log}\n{diagnostics:?}");
            assert_eq!(
                errors[0].message,
                "==> Fatal error occurred, no output PDF file produced!"
            );
        }
    }
}

#[test]
fn fatal_details_are_not_removed_by_generic_summary_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Main.\n").unwrap();
    for detail in [
        "==> Fatal error occurred, no output PDF file produced! Missing font data.",
        "Fatal error occurred while loading font data.",
        "==> Fatal error occurred, no output PDF file produced: missing-font.pfb.",
    ] {
        let log = format!("! First error.\n! {detail}\n");
        let diagnostics = build_log::parse(&main, &log);
        let entries = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert!(entries.iter().any(|entry| entry.message == detail));
    }
}

#[test]
fn fatal_trailers_do_not_move_the_cause_to_a_later_line() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Main.\nUnrelated.\n").unwrap();
    let log = "(./main.tex\n! Missing font without punctuation\n!  ==> Fatal error occurred, no output PDF file produced!\nl.2 Unrelated.\n)\n";
    let diagnostics = build_log::parse(&main, log);
    let entries = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].message, "Missing font without punctuation");
    assert_eq!(entries[0].range.start.line, 0);
}

#[test]
#[ignore = "requires luatex"]
fn real_luatex_failures_use_the_specific_cause_as_the_build_summary() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let name = format!("audit-{}missing-font.pfb", "long-name-".repeat(12));
    std::fs::write(&main, format!("% LuaTeX audit\n\\pdfextension mapline{{=cmr10 CMR10 <{name}}}\n\\font\\auditfont=cmr10 \\auditfont Body.\\bye\n")).unwrap();
    let config = BuildConfig {
        executable: "luatex".into(),
        arguments: Some(
            [
                "-synctex=1",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "-file-line-error",
                "-recorder",
                "-output-directory=%o",
                "%f",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error", "{}", outcome.output);
    let errors = outcome
        .diagnostics
        .values()
        .flatten()
        .filter(|entry| entry.severity == Some(DiagnosticSeverity::ERROR))
        .collect::<Vec<_>>();
    assert_eq!(errors.len(), 1, "{errors:?}\n{}", outcome.output);
    assert_eq!(outcome.message, errors[0].message);
    assert!(outcome.message.contains(&name));
    assert!(outcome.message.contains("cannot open file for reading"));
    assert!(!outcome.message.contains("Fatal error occurred"));
}

#[test]
fn pdftex_shipout_errors_keep_complete_font_names_across_byte_wraps() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Main.\nSecond.\n").unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    let message = "pdfTeX error: /opt/tex/bin/pdflatex (file assets/audit.font.pfb): cannot open Type 1 font file for reading";
    let file_start = message.find("assets/").unwrap();
    for split in file_start..message.len() {
        let log = format!("(./main.tex\n)\n!{}\n{}\n ==> Fatal error occurred, no output PDF file produced!\nTranscript written on main.log.\n", &message[..split], &message[split..]);
        let diagnostics = build_log::parse(&main, &log);
        let errors = &diagnostics[&uri];
        assert_eq!(errors.len(), 1, "{split}: {errors:?}");
        assert_eq!(errors[0].message, message, "{split}");
        assert_eq!(errors[0].range.start.line, 0);
        assert_eq!(errors[0].severity, Some(DiagnosticSeverity::ERROR));
        assert!(errors[0].code.is_none());
    }
}

#[test]
fn pdftex_engine_headers_separate_adjacent_warnings_and_traditional_errors() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Main.\nSecond.\n").unwrap();
    let log = "(./main.tex\nPackage audit Warning: Unfinished warning\n!pdfTeX error: /opt/tex/bin/pdflatex (file audit.pfb\n): cannot open Type 1 font file for reading\n! Ordinary error.\nl.2 Second.\n)\n";
    let diagnostics = build_log::parse(&main, log);
    let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors.iter().any(|entry| entry.message
        == "Package audit Warning: Unfinished warning"
        && entry.severity == Some(DiagnosticSeverity::WARNING)));
    assert!(errors.iter().any(|entry| entry.message == "pdfTeX error: /opt/tex/bin/pdflatex (file audit.pfb): cannot open Type 1 font file for reading"
        && entry.range.start.line == 0));
    assert!(errors
        .iter()
        .any(|entry| entry.message == "Ordinary error." && entry.range.start.line == 1));
}

#[test]
fn only_known_no_space_error_headers_create_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Main.\n").unwrap();
    let log =
        "!not an error\n!pdfTeX errorish: text\n!pdfTeX errors: text\n!pdfTeX error-free: text\n";
    assert!(build_log::parse(&main, log).is_empty());
    for header in [
        "!pdfTeX error (font expansion): Unavailable expansion.",
        "! pdfTeX error: Unavailable font.",
    ] {
        let diagnostics = build_log::parse(&main, header);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.starts_with("pdfTeX error"));
    }
}

#[test]
fn compiler_summaries_do_not_extend_unfinished_error_details() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Main.\n").unwrap();
    for summary in [
        " ==> Fatal error occurred, no output PDF file produced!",
        "Transcript written on main.log.",
        "Output written on main.pdf (1 page, 123 bytes).",
        "No pages of output.",
    ] {
        let log = format!("(./main.tex\n! Error without punctuation\n{summary}\n");
        let diagnostics = build_log::parse(&main, &log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Error without punctuation", "{summary}");
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_pdftex_shipout_font_errors_report_the_missing_resource_instead_of_exit_status() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let name = format!("audit-{}missing-font.pfb", "long-name-".repeat(12));
    std::fs::write(&main, format!("\\documentclass{{article}}\n\\pdfmapline{{=cmr10 CMR10 <{name}}}\n\\begin{{document}}Body.\\end{{document}}\n")).unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error", "{}", outcome.output);
    let uri = Url::from_file_path(&main).unwrap();
    let errors = &outcome.diagnostics[&uri];
    let error = errors
        .iter()
        .find(|error| error.message.starts_with("pdfTeX error:"))
        .unwrap_or_else(|| panic!("{errors:?}\n{}", outcome.output));
    assert!(error.message.contains(&name), "{}", error.message);
    assert!(
        error
            .message
            .contains("cannot open Type 1 font file for reading"),
        "{}",
        error.message
    );
    assert!(!error.message.contains("Fatal error occurred"));
    assert_eq!(error.range.start.line, 0);
    assert!(errors
        .iter()
        .all(|error| !error.message.contains("exited with exit status")));
}

#[test]
fn missing_file_causes_survive_numbered_and_traditional_emergency_stops() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "First.\nSecond.\n😀 \\input{missing}\n").unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    for cause in [
        "LaTeX Error: File `missing.tex' not found.",
        "I can't find file `missing'.",
    ] {
        for stop in ["! Emergency stop.", "./main.tex:3: Emergency stop."] {
            let log = format!("(./main.tex\n! {cause}\n\nType X to quit or <RETURN> to proceed,\nor enter new name. (Default extension: tex)\n\nEnter file name: \n{stop}\n<read *>\n\nl.3 😀 \\input{{missing}}\n                        ^^M\n");
            let diagnostics = build_log::parse(&main, &log);
            let entries = &diagnostics[&uri];
            assert_eq!(entries.len(), 2, "{log}\n{entries:?}");
            let original = entries.iter().find(|entry| entry.message == cause).unwrap();
            assert_eq!(original.range.start.line, 2, "{log}\n{entries:?}");
            assert_eq!(original.range.start.character, 10);
            assert_eq!(original.range.end.character, 17);
            assert_eq!(
                original.code,
                Some(lsp_types::NumberOrString::String("missing-input".into()))
            );
            assert_eq!(original.severity, Some(DiagnosticSeverity::ERROR));
            assert!(entries
                .iter()
                .any(|entry| entry.message == "Emergency stop." && entry.range.start.line == 2));
        }
    }
}

#[test]
fn pending_errors_are_preserved_without_borrowing_unrelated_file_or_line_context() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "First.\nSecond.\nThird.\n").unwrap();
    std::fs::write(&child, "Child.\nSecond child line.\nThird child line.\n").unwrap();
    let main_uri = Url::from_file_path(&main).unwrap();
    let child_uri = Url::from_file_path(&child).unwrap();
    for (cause, stop, tail) in [
        (
            "Unfinished original error.",
            "./main.tex:3: Unrelated error.",
            "",
        ),
        (
            "LaTeX Error: File `missing.tex' not found.",
            "./child.tex:3: Emergency stop.",
            "",
        ),
        (
            "LaTeX Error: File `missing.tex' not found.",
            "./unknown.tex:3: Emergency stop.",
            "",
        ),
        (
            "LaTeX Error: File `missing.tex' not found.",
            "! Emergency stop.",
            "[latex-lsp: middle of compiler output omitted]\nl.3 Third.\n",
        ),
        (
            "LaTeX Error: File `missing.tex' not found.",
            "! Emergency stop.",
            "./child.tex:3: Another error.\nl.3 Third.\n",
        ),
    ] {
        let log = format!("(./main.tex\n! {cause}\n{stop}\n{tail}");
        let diagnostics = build_log::parse(&main, &log);
        let original = diagnostics[&main_uri]
            .iter()
            .find(|entry| entry.message == cause)
            .unwrap_or_else(|| panic!("{log}\n{diagnostics:?}"));
        assert_eq!(original.range.start.line, 0, "{log}\n{diagnostics:?}");
        assert!(diagnostics
            .get(&child_uri)
            .into_iter()
            .flatten()
            .all(|entry| entry.message != cause));
    }
    for gap in [
        "(./child.tex\n)\n".to_owned(),
        "Package demo Warning: Unrelated warning on input line 1.\n".to_owned(),
        "\n".repeat(33),
    ] {
        let cause = "LaTeX Error: File `missing.tex' not found.";
        let log =
            format!("(./main.tex\n! {cause}\n{gap}./main.tex:3: Emergency stop.\nl.3 Third.\n");
        let diagnostics = build_log::parse(&main, &log);
        let original = diagnostics[&main_uri]
            .iter()
            .find(|entry| entry.message == cause)
            .unwrap();
        assert_eq!(original.range.start.line, 0, "{log}\n{diagnostics:?}");
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_missing_input_and_package_causes_survive_emergency_stops_in_the_correct_source() {
    for file_line in [false, true] {
        for package in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let child = root.join("child.tex");
            let (source, cause_name, expected_line) = if package {
                ("\\documentclass{article}\n\\usepackage{missing-lsp-package}\n\\begin{document}\nText.\\end{document}\n", "missing-lsp-package.sty", 1)
            } else {
                std::fs::write(&child, "Child.\n\\input{missing-lsp-chapter}\n").unwrap();
                ("\\documentclass{article}\n\\begin{document}\n\\input{child}\n\\end{document}\n", "missing-lsp-chapter.tex", 1)
            };
            std::fs::write(&main, source).unwrap();
            let config = BuildConfig {
                executable: "pdflatex".into(),
                arguments: Some(vec![
                    "-interaction=nonstopmode".into(),
                    if file_line {
                        "-file-line-error"
                    } else {
                        "-no-file-line-error"
                    }
                    .into(),
                    "-output-directory=%o".into(),
                    "%f".into(),
                ]),
                ..Default::default()
            };
            let outcome = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(outcome.status, "error", "{}", outcome.output);
            let expected = format!("LaTeX Error: File `{cause_name}' not found.");
            let uri = Url::from_file_path(if package { &main } else { &child }).unwrap();
            let errors = outcome
                .diagnostics
                .get(&uri)
                .unwrap_or_else(|| panic!("{:?}\n{}", outcome.diagnostics, outcome.output));
            let cause = errors
                .iter()
                .find(|entry| entry.message == expected)
                .unwrap_or_else(|| panic!("{file_line}/{package}: {errors:?}\n{}", outcome.output));
            assert_eq!(
                cause.range.start.line, expected_line,
                "{file_line}/{package}: {errors:?}\n{}",
                outcome.output
            );
            assert_eq!(cause.severity, Some(DiagnosticSeverity::ERROR));
            if package {
                assert_eq!(
                    cause.code,
                    Some(lsp_types::NumberOrString::String("missing-package".into()))
                );
                let stop = errors
                    .iter()
                    .find(|entry| entry.message == "Emergency stop.")
                    .unwrap();
                assert_eq!(stop.range.start.line, 2);
            }
            assert!(errors
                .iter()
                .any(|entry| entry.message == "Emergency stop."));
        }
    }
}

#[test]
fn wrapped_file_line_errors_recover_long_quoted_unicode_paths_and_complete_messages() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "Main.\nSecond.\n").unwrap();
    for suffix in ["chapter.tex", "章节 (旧).tex", "generated contents.toc"] {
        let name = format!("{}-{suffix}", "long-path-".repeat(12));
        let child = root.join(&name);
        let source = "First.\n😀 \\unknownLspCommand\n";
        std::fs::write(&child, source).unwrap();
        for quoted in [false, true] {
            let path = format!("./{name}");
            let path = if quoted { format!("\"{path}\"") } else { path };
            for split in [76, path.find('.').unwrap_or(0) + 1, path.len() - 2] {
                // Only genuine full print-width lines need reconstruction.
                if split < 60 || !path.is_char_boundary(split) {
                    continue;
                }
                let log = format!("(./main.tex\n{}\n{}:2: Undefined \ncontrol sequence.\nl.2 \\unknownLspCommand\n./main.tex:2: Adjacent error.\n)\n", &path[..split], &path[split..]);
                let diagnostics = build_log::parse(&main, &log);
                let uri = Url::from_file_path(&child).unwrap();
                let errors = diagnostics
                    .get(&uri)
                    .unwrap_or_else(|| panic!("{log}\n{diagnostics:?}"));
                assert_eq!(errors.len(), 1, "{diagnostics:?}");
                assert_eq!(errors[0].range.start.line, 1);
                assert_eq!(
                    errors[0].range.end.character,
                    Text::new(source.into())
                        .position(source.len() - 1)
                        .character
                );
                assert_eq!(errors[0].message, "Undefined control sequence.");
                assert_eq!(errors[0].severity, Some(DiagnosticSeverity::ERROR));
                let main_uri = Url::from_file_path(&main).unwrap();
                assert_eq!(diagnostics[&main_uri].len(), 1);
                assert_eq!(diagnostics[&main_uri][0].message, "Adjacent error.");
            }
        }
    }
}

#[test]
fn ordinary_file_line_errors_preserve_wrapped_words_and_adjacent_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "One.\nTwo.\n").unwrap();
    let log = "(./main.tex\n./main.tex:2: Undefined \ncontrol sequence.\nl.2 \\bad\n./main.tex:1: Another error.\n./main.tex:2: LaTeX Warning: A warning\n on input line 2.\n)\n";
    let diagnostics = build_log::parse(&main, log);
    let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert!(errors
        .iter()
        .any(|error| error.message == "Undefined control sequence."));
    assert!(errors.iter().any(|error| error.message == "Another error."));
    assert!(errors.iter().any(|error| error.message
        == "LaTeX Warning: A warning on input line 2."
        && error.severity == Some(DiagnosticSeverity::WARNING)));
}

#[test]
fn error_message_wraps_inside_words_keep_exact_text_and_end_at_file_announcements() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "One.\nTwo.\n").unwrap();
    std::fs::write(&child, "Child.\n").unwrap();
    let log = "(./main.tex\n./main.tex:2: LaTeX Error: File `long-file\nname.pdf' not found.\nl.2 bad\n./main.tex:2: Error without punctuation\n(./child.tex\nPackage child Warning: Child warning on input line 1.\n)\n)\n";
    let diagnostics = build_log::parse(&main, log);
    let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(errors.len(), 2, "{diagnostics:?}");
    assert!(errors
        .iter()
        .any(|error| error.message == "LaTeX Error: File `long-filename.pdf' not found."));
    assert!(errors
        .iter()
        .any(|error| error.message == "Error without punctuation"));
    assert_eq!(
        diagnostics[&Url::from_file_path(&child).unwrap()][0].message,
        "Package child Warning: Child warning on input line 1."
    );
}

#[test]
fn wrapped_error_headers_are_boundaries_for_unfinished_package_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let name = format!("{}chapter.tex", "long-".repeat(24));
    let child = directory.path().join(&name);
    std::fs::write(&main, "Main.\n").unwrap();
    std::fs::write(&child, "Child.\nError.\n").unwrap();
    let header = format!("./{name}:2: Undefined control sequence.");
    let log = format!(
        "(./main.tex\nPackage main Warning: Unfinished message\n{}\n{}\nl.2 bad\n)\n",
        &header[..79],
        &header[79..]
    );
    let diagnostics = build_log::parse(&main, &log);
    let warnings = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].message,
        "Package main Warning: Unfinished message"
    );
    assert_eq!(
        diagnostics[&Url::from_file_path(&child).unwrap()][0].message,
        "Undefined control sequence."
    );
}

#[test]
fn file_line_headers_can_wrap_inside_extensions_or_the_line_number_separator() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let name = format!("{}chapter.tex", "long-".repeat(24));
    let child = directory.path().join(&name);
    std::fs::write(&main, "Main.\n").unwrap();
    std::fs::write(&child, "Child.\nError.\n").unwrap();
    let header = format!("./{name}:2: Undefined control sequence.");
    let separator = header.find(":2:").unwrap();
    for split in [
        separator - 2,
        separator,
        separator + 1,
        separator + 2,
        separator + 3,
    ] {
        let log = format!(
            "(./main.tex\n{}\n{}\nl.2 bad\n)\n",
            &header[..split],
            &header[split..]
        );
        let diagnostics = build_log::parse(&main, &log);
        let errors = diagnostics
            .get(&Url::from_file_path(&child).unwrap())
            .unwrap_or_else(|| panic!("{split}: {log}\n{diagnostics:?}"));
        assert_eq!(errors[0].message, "Undefined control sequence.");
        assert_eq!(errors[0].range.start.line, 1);
    }
}

#[test]
fn traditional_error_messages_join_byte_wraps_and_preserve_tex_context_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "One.\nTwo.\n").unwrap();
    for context in [
        "<inserted text>",
        "<recently read>",
        "<to be read again>",
        "<argument>",
        "<*>",
        "See the LaTeX manual or LaTeX Companion for explanation.",
        "Type  H <return>  for immediate help.",
    ] {
        let log = format!("(./main.tex\n! LaTeX Error: File `long-\nfilename.pdf' not found.\nl.2 bad\n! Error without punctuation\n{context}\nl.1 other\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|error| error.message
            == "LaTeX Error: File `long-filename.pdf' not found."
            && error.range.start.line == 1));
        assert!(
            errors
                .iter()
                .any(|error| error.message == "Error without punctuation"
                    && error.range.start.line == 0),
            "{context}: {errors:?}"
        );
    }
}

#[test]
fn incomplete_file_line_headers_do_not_absorb_real_errors_or_cross_truncated_output() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "One.\nTwo.\n").unwrap();
    let prefix = format!("./{}", "missing-path-".repeat(8));
    for boundary in [
        "./main.tex:2: Actual error.",
        "[latex-lsp: middle of compiler output omitted]\n./main.tex:2: Actual error.",
        "! Actual error.\nl.2 bad",
    ] {
        let log = format!("(./main.tex\n{prefix}\n{boundary}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 1, "{diagnostics:?}");
        assert_eq!(errors[0].message, "Actual error.");
        assert_eq!(errors[0].range.start.line, 1);
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_long_file_line_errors_keep_the_child_file_line_and_full_error() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let name = format!("chapter-{}.tex", "long-name-".repeat(13));
    let main = root.join("main.tex");
    let child = root.join(&name);
    std::fs::write(&main, format!("\\documentclass{{article}}\n\\begin{{document}}\n\\input{{{name}}}\n\\end{{document}}\n")).unwrap();
    std::fs::write(&child, "First.\n\\unknownLspCommand\n").unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error", "{}", outcome.output);
    let uri = Url::from_file_path(&child).unwrap();
    let errors = outcome
        .diagnostics
        .get(&uri)
        .unwrap_or_else(|| panic!("{:?}\n{}", outcome.diagnostics, outcome.output));
    assert!(
        errors
            .iter()
            .any(|error| error.range.start.line == 1
                && error.message == "Undefined control sequence."),
        "{errors:?}"
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn real_package_errors_keep_wrapped_words_with_and_without_file_line_reporting() {
    for file_line in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let name = format!("{}-章节 (旧).tex", "long-path-".repeat(12));
        let child = root.join(&name);
        let message = format!("Missing {} filename", "VeryLongErrorWord".repeat(12));
        std::fs::write(&main, format!("\\documentclass{{article}}\n\\begin{{document}}\n\\input{{{name}}}\n\\end{{document}}\n")).unwrap();
        std::fs::write(
            &child,
            format!("First.\n\\PackageError{{audit}}{{{message}}}{{Fix the input.}}\n"),
        )
        .unwrap();
        let config = BuildConfig {
            executable: "pdflatex".into(),
            arguments: Some(vec![
                "-interaction=nonstopmode".into(),
                if file_line {
                    "-file-line-error"
                } else {
                    "-no-file-line-error"
                }
                .into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ]),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(outcome.status, "error", "{}", outcome.output);
        let uri = Url::from_file_path(&child).unwrap();
        let errors = outcome.diagnostics.get(&uri).unwrap_or_else(|| {
            panic!("{file_line}: {:?}\n{}", outcome.diagnostics, outcome.output)
        });
        let expected = format!("Package audit Error: {message}.");
        assert!(
            errors
                .iter()
                .any(|error| error.range.start.line == 1 && error.message == expected),
            "{file_line}: {errors:?}\n{}",
            outcome.output
        );
    }
}
