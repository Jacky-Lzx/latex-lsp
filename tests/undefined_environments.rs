use latex_lsp::{compilation::build_log, text::Text};
use lsp_types::{DiagnosticSeverity, NumberOrString, Url};

const HELP: &str = "\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...                                              \n                                                  \n";

#[test]
fn environment_error_names_preserve_semantic_whitespace_and_reject_normalized_source_matches() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for (source, message) in [
        (
            r"\begin{ algin }",
            "LaTeX Error: Environment  algin  undefined.",
        ),
        (
            r"\begin{ algin }",
            "LaTeX Error: Environment algin undefined.",
        ),
        (
            r"\begin{algin}",
            "LaTeX Error: Environment  algin  undefined.",
        ),
        (
            r"\end{ algin }",
            r"LaTeX Error: \begin{equation} ended by \end{ algin }.",
        ),
        (
            r"\end{algin}",
            r"LaTeX Error: \begin{ equation } ended by \end{algin}.",
        ),
        (
            r"\end{ algin }",
            r"LaTeX Error: \begin{equation} ended by \end{algin}.",
        ),
    ] {
        std::fs::write(&main, source).unwrap();
        let log = format!("(./main.tex\n./main.tex:1: {message}\n{HELP}l.1 {source}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{source}: {error:?}"
        );
        assert_eq!(error.range.start.character, 0);
        assert_eq!(error.message, message);
    }
}

#[test]
fn repeated_undefined_environments_on_one_line_keep_separate_utf16_name_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "First.\r\n😀 \\begin{algin}a\\end{algin} \\begin{algin}b\\end{algin}\r\n";
    std::fs::write(&main, source).unwrap();
    for numbered in [false, true] {
        let header = if numbered { "./main.tex:2:" } else { "!" };
        let log = format!("(./main.tex\n{header} LaTeX Error: Environment algin undefined.\n{HELP}l.2 😀 \\begin{{algin}}\n{header} LaTeX Error: Environment algin undefined.\n{HELP}l.2 😀 \\begin{{algin}}a\\end{{algin}} \\begin{{algin}}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 2, "{numbered}: {errors:?}");
        let text = Text::new(source.into());
        for (error, (start, _)) in errors.iter().zip(source.match_indices("\\begin{algin}")) {
            let start = start + "\\begin{".len();
            assert_eq!(error.range, text.range(start, start + "algin".len()));
            assert_eq!(
                error.code,
                Some(NumberOrString::String("undefined-environment".into()))
            );
            assert_eq!(error.data.as_ref().unwrap()["environment"], "algin");
            assert_eq!(error.severity, Some(DiagnosticSeverity::ERROR));
        }
    }
}

#[test]
fn mismatched_environment_ends_use_the_actual_closing_name_and_preserve_the_expected_environment() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "😀 \\begin{equation}x\\end{algin} \\end{algin}\n";
    std::fs::write(&main, source).unwrap();
    for numbered in [false, true] {
        let header = if numbered { "./main.tex:1:" } else { "!" };
        let log = format!("(./main.tex\n{header} LaTeX Error: \\begin{{equation}} on input line 1 ended by \\end{{algin}}.\n{HELP}l.1 😀 \\begin{{equation}}x\\end{{algin}}\n{header} LaTeX Error: \\begin{{document}} ended by \\end{{algin}}.\n{HELP}l.1 😀 \\begin{{equation}}x\\end{{algin}} \\end{{algin}}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 2, "{numbered}: {errors:?}");
        let text = Text::new(source.into());
        for ((error, (start, _)), expected) in errors
            .iter()
            .zip(source.match_indices("\\end{algin}"))
            .zip(["equation", "document"])
        {
            let start = start + "\\end{".len();
            assert_eq!(error.range, text.range(start, start + 5));
            assert_eq!(
                error.code,
                Some(NumberOrString::String("mismatched-environment-end".into()))
            );
            assert_eq!(error.data.as_ref().unwrap()["environment"], "algin");
            assert_eq!(
                error.data.as_ref().unwrap()["expectedEnvironment"],
                expected
            );
        }
    }
}

#[test]
fn mismatched_end_ranges_require_valid_messages_matching_source_and_direct_context() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "Text \\end{algin} \\broken\n";
    std::fs::write(&main, source).unwrap();
    for (message, context) in [
        (
            r"LaTeX Error: \begin{equation} on input line nope ended by \end{algin}.",
            "l.1 Text \\end{algin}",
        ),
        (
            r"LaTeX Error: \begin{equation} ended by \end{other}.",
            "l.1 Text \\end{algin}",
        ),
        (
            r"LaTeX Error: \begin{\name} ended by \end{algin}.",
            "l.1 Text \\end{algin}",
        ),
        (
            r"LaTeX Error: \begin{equation} ended by \end{algin}. Extra text.",
            "l.1 Text \\end{algin}",
        ),
        (
            r"LaTeX Error: \begin{equation} ended by \end{algin}.",
            "<argument> \\end{algin}\nl.1 Text \\end{algin}",
        ),
        (
            r"LaTeX Error: \begin{equation} ended by \end{algin}.",
            "l.1 Text \\end{algin} \\broken",
        ),
    ] {
        let log = format!("(./main.tex\n./main.tex:1: {message}\n{HELP}{context}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert_eq!(error.range.start.character, 0, "{message}: {error:?}");
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{message}: {error:?}"
        );
    }
}

#[test]
fn multiline_environment_calls_locate_static_unicode_names_after_command_newlines() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "😀 \\begin\r\n{模式*}\r\n";
    std::fs::write(&main, source).unwrap();
    let log = format!(
        "(./main.tex\n./main.tex:2: LaTeX Error: Environment 模式* undefined.\n{HELP}l.2 {{模式*}}\n)\n"
    );
    let diagnostics = build_log::parse(&main, &log);
    let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
    let text = Text::new(source.into());
    let start = source.find("模式*").unwrap();
    assert_eq!(error.range, text.range(start, start + "模式*".len()));
    assert_eq!(error.data.as_ref().unwrap()["environment"], "模式*");
}

#[test]
fn shortened_environment_context_requires_a_unique_complete_argument() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = format!(
        "{}😀 unique \\begin{{algin}}a\\end{{algin}} \\begin{{algin}}b\\end{{algin}}\n",
        "prefix ".repeat(24)
    );
    std::fs::write(&main, &source).unwrap();
    for (context, precise) in [
        ("... unique \\begin{algin}", true),
        ("...�� unique \\begin{algin}", true),
        ("...{algin}", false),
        ("...gin}", false),
    ] {
        let log = format!("(./main.tex\n./main.tex:1: LaTeX Error: Environment algin undefined.\n{HELP}l.1 {context}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert_eq!(error.data.is_some(), precise, "{context}: {error:?}");
        if precise {
            let text = Text::new(source.clone());
            let start = source.find("algin").unwrap();
            assert_eq!(error.range, text.range(start, start + 5));
        } else {
            assert_eq!(error.range.start.character, 0);
            assert!(error.code.is_none());
        }
    }
}

#[test]
fn environment_context_falls_back_for_expansion_stale_sources_and_incomplete_help() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "Text \\begin{algin}a\\end{algin} \\broken\n";
    std::fs::write(&main, source).unwrap();
    for context in [
        format!("{HELP}<argument> \\begin{{algin}}\nl.1 Text \\begin{{algin}}"),
        format!("{HELP}\\broken ->\\begin{{algin}}\nl.1 Text \\begin{{algin}}"),
        format!("{HELP}l.1 Changed \\begin{{algin}}"),
        format!("{HELP}l.2 Text \\begin{{algin}}"),
        format!("{HELP}l.1 Text \\begin{{algin}}a\\end{{algin}} \\broken"),
        "Package audit Warning: Adjacent warning.\nl.1 Text \\begin{algin}".into(),
        "[latex-lsp: middle of compiler output omitted]\nl.1 Text \\begin{algin}".into(),
        "See the LaTeX manual or LaTeX Companion for explanation.\nType H <return> for help.\nl.1 Text \\begin{algin}".into(),
    ] {
        let log = format!("(./main.tex\n./main.tex:1: LaTeX Error: Environment algin undefined.\n{context}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let error = diagnostics[&Url::from_file_path(&main).unwrap()].iter().find(|entry| entry.message == "LaTeX Error: Environment algin undefined.").unwrap();
        assert_eq!(error.range.start.character, 0, "{context}: {error:?}");
        assert!(error.code.is_none() && error.data.is_none(), "{context}: {error:?}");
    }
}

#[test]
fn environment_diagnostics_ignore_comments_verbatim_dynamic_names_and_end_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for source in [
        r"% \begin{algin}",
        r"\verb|\begin{algin}|",
        r"\begin{\name}",
        r"\end{algin}",
    ] {
        std::fs::write(&main, source).unwrap();
        let log = format!("(./main.tex\n./main.tex:1: LaTeX Error: Environment algin undefined.\n{HELP}l.1 {source}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{source}: {error:?}"
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_engines_pinpoint_environment_names_and_keep_macro_expansion_errors_on_the_call_line() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::{os::unix::fs::PermissionsExt, sync::atomic::AtomicBool};
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for numbered in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let child = root.join("章节.tex");
            let source =
                "\\typeout{😀} \\begin{algin}A\\end{algin} \\begin{algin}B\\end{algin}\n\\begin{ algin }Spaced\\end{ algin }\n\\broken\n";
            std::fs::write(&main, "\\documentclass{article}\n\\newcommand{\\broken}{\\begin{hidden}\\end{hidden}}\n\\begin{document}\\input{章节}\\end{document}\n").unwrap();
            std::fs::write(&child, source).unwrap();
            let compiler = root.join(engine);
            std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = BuildConfig {
                executable: compiler.to_string_lossy().into_owned(),
                arguments: Some(vec![
                    "-interaction=nonstopmode".into(),
                    if numbered {
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
            let log = &outcome.output;
            for diagnostics in [build_log::parse(&main, log), outcome.diagnostics] {
                let errors: Vec<_> = diagnostics[&Url::from_file_path(&child).unwrap()]
                    .iter()
                    .filter(|error| error.message.starts_with("LaTeX Error: Environment "))
                    .collect();
                assert_eq!(errors.len(), 4, "{engine}/{numbered}: {errors:?}\n{log}");
                let text = Text::new(source.into());
                for (error, (start, _)) in errors[..2]
                    .iter()
                    .zip(source.match_indices("\\begin{algin}"))
                {
                    let start = start + "\\begin{".len();
                    assert_eq!(
                        error.range,
                        text.range(start, start + 5),
                        "{engine}/{numbered}: {error:?}\n{log}"
                    );
                    assert_eq!(error.data.as_ref().unwrap()["environment"], "algin");
                }
                assert_eq!(
                    errors[3].range,
                    text.range(source.find("\\broken").unwrap(), source.len() - 1)
                );
                for error in &errors[2..] {
                    assert!(error.code.is_none() && error.data.is_none(), "{error:?}");
                    assert_eq!(error.range.start.character, 0);
                }
                let ends: Vec<_> = diagnostics[&Url::from_file_path(&child).unwrap()]
                    .iter()
                    .filter(|error| error.message.starts_with("LaTeX Error: \\begin{"))
                    .collect();
                assert_eq!(ends.len(), 4, "{engine}/{numbered}: {ends:?}\n{log}");
                for (error, (start, _)) in
                    ends[..2].iter().zip(source.match_indices("\\end{algin}"))
                {
                    let start = start + "\\end{".len();
                    assert_eq!(
                        error.range,
                        text.range(start, start + 5),
                        "{engine}/{numbered}: {error:?}\n{log}"
                    );
                    assert_eq!(
                        error.code,
                        Some(NumberOrString::String("mismatched-environment-end".into()))
                    );
                    assert_eq!(
                        error.data.as_ref().unwrap()["expectedEnvironment"],
                        "document"
                    );
                }
                for (end, error) in ends[2..].iter().zip(&errors[2..]) {
                    assert_eq!(end.range, error.range);
                    assert!(end.code.is_none() && end.data.is_none(), "{end:?}");
                }
            }
        }
    }
}

#[test]
fn environment_commands_consumed_by_macros_keep_line_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main.tex");
    for command in ["begin", "end"] {
        for (declaration, prefix) in [
            (r"\newcommand{\wrapped}[2]{}", r"\wrapped"),
            (r"\newcommand{\wrapped}[3]{}", r"\wrapped{Unused}"),
            (r"\newcommand{\wrapped}[3][default]{}", r"\wrapped[note]"),
            (r"\def\wrapped#1#2{}", r"\wrapped"),
            (
                r"\NewDocumentCommand{\wrapped}{s t+ O{default} m m}{}",
                r"\wrapped*+[note]",
            ),
            (
                r"\NewDocumentCommand{\wrapped}{d() m m}{}",
                r"\wrapped(note)",
            ),
            (
                r"\newcommand{\wrapped}[2]{}\def\bridge{\wrapped}",
                r"\bridge",
            ),
            (r"\def\bridge{\missingtarget}", r"\bridge"),
            (r"\def\bridge{\bridge}", r"\bridge"),
            ("", r"\unknown{Unused}"),
            (r"\renewcommand{\begin}[1]{}", ""),
            (r"\renewcommand{\end}[1]{}", ""),
        ] {
            let call = format!("{prefix}\\{command}{{algin}}");
            let source = format!("{declaration}\n{call}\n");
            std::fs::write(&root, source).unwrap();
            let message = if command == "begin" {
                "LaTeX Error: Environment algin undefined."
            } else {
                r"LaTeX Error: \begin{equation} ended by \end{algin}."
            };
            let log = format!("(./main.tex\n./main.tex:2: {message}\n{HELP}l.2 {call}\n)\n");
            let result = build_log::parse(&root, &log);
            let error = &result[&Url::from_file_path(&root).unwrap()][0];
            assert!(
                error.code.is_none() && error.data.is_none(),
                "{declaration}/{call}: {error:?}"
            );
        }
    }
}

#[test]
fn completed_macro_calls_preserve_environment_error_ranges() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main.tex");
    for command in ["begin", "end"] {
        for (declaration, prefix) in [
            (r"\newcommand{\done}{}", r"\done"),
            (r"\newcommand{\done}{}\def\bridge{\done}", r"\bridge"),
            (r"\newcommand{\done}[1]{}", r"\done{Unused}"),
            (r"\newcommand{\done}[1][default]{}", r"\done[note]"),
            (
                r"\NewDocumentCommand{\done}{s t+ O{default} m}{}",
                r"\done*+[note]{Unused}",
            ),
            (
                r"\NewDocumentCommand{\done}{d() m}{}",
                r"\done(note){Unused}",
            ),
            ("", r"\relax\noindent\bfseries"),
        ] {
            let call = format!("{prefix}\\{command}{{algin}}");
            let source = format!("{declaration}\n{call}\n");
            std::fs::write(&root, &source).unwrap();
            let message = if command == "begin" {
                "LaTeX Error: Environment algin undefined."
            } else {
                r"LaTeX Error: \begin{equation} ended by \end{algin}."
            };
            let log = format!("(./main.tex\n./main.tex:2: {message}\n{HELP}l.2 {call}\n)\n");
            let result = build_log::parse(&root, &log);
            let error = &result[&Url::from_file_path(&root).unwrap()][0];
            let start = source.rfind("algin").unwrap();
            assert_eq!(
                error.range,
                Text::new(source).range(start, start + 5),
                "{declaration}/{call}: {error:?}"
            );
            assert!(error.code.is_some());
        }
    }
}

#[test]
fn shortened_environment_echoes_cannot_borrow_a_suffix_from_another_command() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main.tex");
    for command in ["begin", "end"] {
        let source =
            format!("\\iffalse\\{command}{{algin}}\\fi\\unknownwrapper{command}{{algin}}\n");
        std::fs::write(&root, &source).unwrap();
        let message = if command == "begin" {
            "LaTeX Error: Environment algin undefined."
        } else {
            r"LaTeX Error: \begin{equation} ended by \end{algin}."
        };
        let log = format!("(./main.tex\n./main.tex:1: {message}\n{HELP}l.1 ...{{algin}}\n)\n");
        let result = build_log::parse(&root, &log);
        let error = &result[&Url::from_file_path(&root).unwrap()][0];
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{command}: {error:?}"
        );
    }
}
