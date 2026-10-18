use latex_lsp::{build_log, text::Text};
use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString, Url};

const HELP: &str = "\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...                                              \n                                                  \n";

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
fn multiline_environment_arguments_can_point_back_to_the_unicode_name() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "😀 \\begin\r\n{\r\n 模式*\r\n }\r\n";
    std::fs::write(&main, source).unwrap();
    let log = format!(
        "(./main.tex\n./main.tex:4: LaTeX Error: Environment 模式* undefined.\n{HELP}l.4  }}\n)\n"
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
    use latex_lsp::{build, config::BuildConfig};
    use std::{os::unix::fs::PermissionsExt, sync::atomic::AtomicBool};
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for numbered in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let child = root.join("章节.tex");
            let source =
                "\\typeout{😀} \\begin{algin}A\\end{algin} \\begin{algin}B\\end{algin}\n\\broken\n";
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
                assert_eq!(errors.len(), 3, "{engine}/{numbered}: {errors:?}\n{log}");
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
                    errors[2].range,
                    text.range(source.find("\\broken").unwrap(), source.len() - 1)
                );
                assert!(errors[2].code.is_none() && errors[2].data.is_none());
            }
        }
    }
}
