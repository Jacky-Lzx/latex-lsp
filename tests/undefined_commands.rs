use latex_lsp::{build, build_log, config::BuildConfig, text::Text};
use std::sync::atomic::AtomicBool;
use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString, Url};

#[test]
fn undefined_commands_on_the_same_line_keep_distinct_utf16_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "First.\r\n😀 \\firstbad and \\secondbad.\r\n";
    std::fs::write(&main, source).unwrap();
    for numbered in [false, true] {
        let header = if numbered {
            "./main.tex:2: Undefined control sequence."
        } else {
            "! Undefined control sequence."
        };
        let log = format!("(./main.tex\n{header}\nl.2 😀 \\firstbad\n                   and \\secondbad.\n{header}\nl.2 😀 \\firstbad and \\secondbad\n                              .\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 2, "{numbered}: {errors:?}");
        let text = Text::new(source.into());
        for (error, command) in errors.iter().zip([r"\firstbad", r"\secondbad"]) {
            let start = source.find(command).unwrap();
            assert_eq!(error.range, text.range(start, start + command.len()));
            assert_eq!(
                error.code,
                Some(NumberOrString::String("undefined-control-sequence".into()))
            );
            assert_eq!(error.message, "Undefined control sequence.");
            assert_eq!(error.severity, Some(DiagnosticSeverity::ERROR));
        }
    }
}

#[test]
fn uncertain_expansion_stale_context_and_output_boundaries_keep_line_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "\\newcommand{\\broken}{\\missinginside}\nText \\broken and \\bad.\n";
    std::fs::write(&main, source).unwrap();
    for context in [
        "\\broken ->\\missinginside\n                         \nl.2 Text \\broken",
        "<argument> \\missinginside\nl.2 Text \\broken",
        "<recently read> \\missinginside\nl.2 Text \\broken",
        "l.2 Changed \\bad",
        "l.1 Text \\bad",
        "l.2 ...ad",
        "Package audit Warning: Adjacent warning.\nl.2 Text \\broken and \\bad",
        "[latex-lsp: middle of compiler output omitted]\nl.2 Text \\broken and \\bad",
    ] {
        let log = format!("(./main.tex\n./main.tex:2: Undefined control sequence.\n{context}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let error = diagnostics[&Url::from_file_path(&main).unwrap()]
            .iter()
            .find(|entry| entry.message == "Undefined control sequence.")
            .unwrap();
        assert_eq!(error.range.start.line, 1, "{context}");
        assert_eq!(error.range.start.character, 0, "{context}");
        assert_eq!(
            error.range.end.character,
            "Text \\broken and \\bad.".len() as u32,
            "{context}"
        );
    }
}

#[test]
fn shortened_tex_context_is_used_only_when_it_matches_a_unique_command() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = format!(
        "{} unique \\badone and \\badagain repeated \\badagain.\n",
        "prefix ".repeat(24)
    );
    std::fs::write(&main, &source).unwrap();
    let log = "(./main.tex\n./main.tex:1: Undefined control sequence.\nl.1 ... unique \\badone\n./main.tex:1: Undefined control sequence.\nl.1 ...\\badagain\n)\n";
    let diagnostics = build_log::parse(&main, log);
    let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(errors.len(), 2);
    let text = Text::new(source.clone());
    let start = source.find(r"\badone").unwrap();
    assert!(errors
        .iter()
        .any(|error| error.range == text.range(start, start + r"\badone".len())));
    assert!(errors.iter().any(|error| error.range.start.character == 0));
}

#[test]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_engines_pinpoint_direct_errors_without_blame_on_macro_calls() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for numbered in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let child = directory.path().join("章节.tex");
            let source = "\\typeout{😀} \\firstbad and \\secondbad.\n\\broken\n";
            std::fs::write(&main, "\\documentclass{article}\n\\newcommand{\\broken}{\\missinginside}\n\\begin{document}\\input{章节}\\end{document}\n").unwrap();
            std::fs::write(&child, source).unwrap();
            let config = BuildConfig {
                executable: engine.into(),
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
            assert_eq!(outcome.status, "error", "{engine}: {}", outcome.output);
            let uri = Url::from_file_path(&child).unwrap();
            let errors: Vec<_> = outcome.diagnostics[&uri]
                .iter()
                .filter(|error| error.message == "Undefined control sequence.")
                .collect();
            assert_eq!(
                errors.len(),
                3,
                "{engine}/{numbered}: {errors:?}\n{}",
                outcome.output
            );
            let text = Text::new(source.into());
            for (error, command) in errors[..2].iter().zip([r"\firstbad", r"\secondbad"]) {
                let start = source.find(command).unwrap();
                assert_eq!(
                    error.range,
                    text.range(start, start + command.len()),
                    "{engine}/{numbered}: {}",
                    outcome.output
                );
            }
            assert_eq!(
                errors[2].range,
                text.range(source.find("\\broken").unwrap(), source.len() - 1)
            );
            assert_eq!(errors[2].code, None);
        }
    }
}
