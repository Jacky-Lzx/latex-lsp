use lsp_types::{DiagnosticSeverity, NumberOrString, Url};
use std::sync::atomic::AtomicBool;
use texquill::{
    compilation::{build, build_log},
    config::BuildConfig,
    text::Text,
};

#[test]
fn unresolved_numbered_sources_do_not_borrow_precise_error_context_from_the_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let child = root.join("child.tex");
    std::fs::create_dir(root.join("folder.tex")).unwrap();
    let unknown_sources = [
        "./gone.tex".to_owned(),
        "./folder.tex".to_owned(),
        "\"./missing file.tex\"".to_owned(),
        root.join("missing absolute.tex")
            .to_string_lossy()
            .into_owned(),
    ];
    for (source, message, context, code) in [
        (
            "😀 \\bad{x}",
            "Undefined control sequence.",
            "l.1 😀 \\bad",
            "undefined-control-sequence",
        ),
        (
            "😀 $\\bad{x}$",
            "Undefined control sequence.",
            "<recently read> \\bad\n \nl.1 😀 $\\bad",
            "undefined-control-sequence",
        ),
        (
            "😀 \\begin{bad}Text",
            "LaTeX Error: Environment bad undefined.",
            "l.1 😀 \\begin{bad}",
            "undefined-environment",
        ),
        (
            "😀 \\setcounter{bad}{1}",
            "LaTeX Error: No counter 'bad' defined.",
            "l.1 😀 \\setcounter{bad}{1}",
            "undefined-counter",
        ),
        (
            "😀 \\input{bad}",
            "I can't find file `bad.tex'.",
            "l.1 😀 \\input{bad}",
            "missing-input",
        ),
    ] {
        std::fs::write(&main, format!("{source}\n")).unwrap();
        std::fs::write(&child, format!("{source}\n")).unwrap();
        for (name, path) in [("./main.tex", &main), ("./child.tex", &child)] {
            let log = format!("(./main.tex\n{name}:1: {message}\n{context}\n)\n");
            let diagnostics = build_log::parse(&main, &log);
            let error = &diagnostics[&Url::from_file_path(path).unwrap()][0];
            assert_eq!(
                error.code,
                Some(NumberOrString::String(code.into())),
                "{name}: {error:?}"
            );
            assert!(error.data.is_some());
        }
        for name in &unknown_sources {
            let log = format!("(./main.tex\n{name}:1: {message}\n{context}\n)\n");
            let diagnostics = build_log::parse(&main, &log);
            assert_eq!(diagnostics.len(), 1);
            let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
            assert_eq!(error.message, message);
            assert_eq!(error.severity, Some(DiagnosticSeverity::ERROR));
            assert!(
                error.code.is_none()
                    && error.data == Some(serde_json::json!({"unresolvedSource":true})),
                "{name}: {error:?}"
            );
            assert_eq!(error.range.start.character, 0);
            assert_eq!(
                error.range.end.character,
                source.encode_utf16().count() as u32
            );
        }
    }
}

#[test]
fn direct_math_commands_with_matching_recent_tokens_keep_distinct_utf16_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "\\typeout{😀} $\\firstbad +\\secondbad$\r\n\\hbox{$\\thirdbad$}\r\n";
    std::fs::write(&main, source).unwrap();
    let text = Text::new(source.into());
    for numbered in [false, true] {
        let mut log = String::from("(./main.tex\n");
        for (line, name, prefix) in [
            (1, "firstbad", "\\typeout{😀} $\\firstbad"),
            (1, "secondbad", "\\typeout{😀} $\\firstbad +\\secondbad"),
            (2, "thirdbad", "\\hbox{$\\thirdbad"),
        ] {
            let header = if numbered {
                format!("./main.tex:{line}: Undefined control sequence.")
            } else {
                "! Undefined control sequence.".into()
            };
            log.push_str(&format!("{header}\n<recently read> \\{name} \n                         \nl.{line} {prefix}\n"));
        }
        log.push_str(")\n");
        let diagnostics = build_log::parse(&main, &log);
        let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 3, "{numbered}: {errors:?}");
        for (error, name) in errors.iter().zip(["firstbad", "secondbad", "thirdbad"]) {
            let command = format!("\\{name}");
            let start = source.find(&command).unwrap();
            assert_eq!(
                error.range,
                text.range(start, start + command.len()),
                "{numbered}: {name}"
            );
            assert_eq!(
                error.code,
                Some(NumberOrString::String("undefined-control-sequence".into()))
            );
            assert_eq!(error.data.as_ref().unwrap()["command"], name);
        }
    }
}

#[test]
fn recent_tokens_require_one_matching_literal_token_and_immediate_trustworthy_context() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "$\\bad +\\bad$\n";
    std::fs::write(&main, source).unwrap();
    for context in [
        "<recently read> \\other\nl.1 $\\bad",
        "<recently read> \\bad extra\nl.1 $\\bad",
        "<recently read> \\bad\\other\nl.1 $\\bad",
        "<recently read> \\?\nl.1 $\\bad",
        "<recently read> \\bad\n<argument> \\bad\nl.1 $\\bad",
        "<argument> \\bad\n<recently read> \\bad\nl.1 $\\bad",
        "<recently read> \\bad\n<recently read> \\bad\nl.1 $\\bad",
        "<recently read> \\bad\nl.2 $\\bad",
        "<recently read> \\bad\nl.1 Changed $\\bad",
        "<recently read> \\bad\nl.1 ...\\bad",
        "<recently read> \\bad\nPackage audit Warning: Adjacent.\nl.1 $\\bad",
        "<recently read> \\bad\n[texquill: middle of compiler output omitted]\nl.1 $\\bad",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([format!(
        "<recently read> \\bad\n{}l.1 $\\bad",
        " \n".repeat(20)
    )]) {
        for numbered in [false, true] {
            let header = if numbered {
                "./main.tex:1: Undefined control sequence."
            } else {
                "! Undefined control sequence."
            };
            let log = format!("(./main.tex\n{header}\n{context}\n)\n");
            let diagnostics = build_log::parse(&main, &log);
            let error = diagnostics[&Url::from_file_path(&main).unwrap()]
                .iter()
                .find(|error| error.message == "Undefined control sequence.")
                .unwrap();
            assert!(
                error.code.is_none() && error.data.is_none(),
                "{numbered}: {context}: {error:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_math_errors_pinpoint_recently_read_tokens_without_blaming_expanding_macros() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for numbered in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let child = directory.path().join("章节.tex");
            let source =
                "\\typeout{😀} $\\firstbad +\\secondbad$\n\\hbox{$\\thirdbad$}\n$\\broken$\n";
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
            let errors = outcome.diagnostics[&Url::from_file_path(&child).unwrap()]
                .iter()
                .filter(|error| error.message == "Undefined control sequence.")
                .collect::<Vec<_>>();
            assert_eq!(
                errors.len(),
                4,
                "{engine}/{numbered}: {:?}\n{}",
                outcome.diagnostics,
                outcome.output
            );
            let text = Text::new(source.into());
            for (error, name) in errors[..3]
                .iter()
                .zip(["firstbad", "secondbad", "thirdbad"])
            {
                let command = format!("\\{name}");
                let start = source.find(&command).unwrap();
                assert_eq!(
                    error.range,
                    text.range(start, start + command.len()),
                    "{engine}/{numbered}: {}",
                    outcome.output
                );
                assert_eq!(error.data.as_ref().unwrap()["command"], name);
            }
            assert_eq!(errors[3].range.start.line, 2);
            assert_eq!(errors[3].range.start.character, 0);
            assert!(errors[3].code.is_none() && errors[3].data.is_none());
        }
    }
}

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
            assert_eq!(
                error.data.as_ref().unwrap()["command"],
                command.strip_prefix('\\').unwrap()
            );
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
        "[texquill: middle of compiler output omitted]\nl.2 Text \\broken and \\bad",
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
        "{}😀 unique \\badone and \\badagain repeated \\badagain.\n",
        "prefix ".repeat(24)
    );
    std::fs::write(&main, &source).unwrap();
    let log = "(./main.tex\n./main.tex:1: Undefined control sequence.\nl.1 ...�� unique \\badone\n./main.tex:1: Undefined control sequence.\nl.1 ...\\badagain\n)\n";
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
