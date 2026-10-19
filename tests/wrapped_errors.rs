use latex_lsp::{build, build_log, config::BuildConfig, text::Text};
use std::sync::atomic::AtomicBool;
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

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
                Some(tower_lsp::lsp_types::NumberOrString::String(
                    "missing-input".into()
                ))
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
                ("\\documentclass{article}\n\\usepackage{missing-lsp-package}\n\\begin{document}\nText.\\end{document}\n", "missing-lsp-package.sty", 2)
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
