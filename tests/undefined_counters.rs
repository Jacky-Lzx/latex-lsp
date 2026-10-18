use latex_lsp::{build_log, text::Text};
use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString, Url};

const HELP: &str = "\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...                                              \n                                                  \n";

#[test]
fn counter_errors_do_not_normalize_significant_whitespace_inside_names() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for name in [" missing", "missing ", " missing ", "mis sing"] {
        let source = format!("\\setcounter{{{name}}}{{2}}\n");
        std::fs::write(&main, &source).unwrap();
        let log = format!("(./main.tex\n./main.tex:1: LaTeX Error: No counter '{name}' defined.\n{HELP}l.1 {}\n)\n", source.trim_end());
        let diagnostics = build_log::parse(&main, &log);
        let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert!(error.code.is_none() && error.data.is_none(), "{error:?}");
        assert_eq!(error.range.start.character, 0);
        assert_eq!(
            error.message,
            format!("LaTeX Error: No counter '{name}' defined.")
        );
    }
}

#[test]
fn counter_operation_errors_select_the_name_and_leave_value_expressions_out_of_the_range() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for (command, value) in [
        ("setcounter", "{1}"),
        ("addtocounter", r"{\value{other}+1}"),
        ("stepcounter", ""),
        ("refstepcounter", ""),
        ("usecounter", ""),
    ] {
        let prefix = format!("😀 \\{command}  {{missing}}{value}");
        let source = format!("{prefix} trailing text.\r\n");
        std::fs::write(&main, &source).unwrap();
        for (header, quote) in [("./main.tex:1:", "'"), ("!", "`")] {
            let log = format!("(./main.tex\n{header} LaTeX Error: No counter {quote}missing' defined.\n{HELP}l.1 {prefix}\n)\n");
            let diagnostics = build_log::parse(&main, &log);
            let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(errors.len(), 1);
            let error = &errors[0];
            let text = Text::new(source.clone());
            let start = source.find("missing").unwrap();
            assert_eq!(
                error.range,
                text.range(start, start + 7),
                "{command}: {error:?}"
            );
            assert_eq!(
                error.code,
                Some(NumberOrString::String("undefined-counter".into()))
            );
            assert_eq!(error.data.as_ref().unwrap()["counter"], "missing");
            assert_eq!(error.severity, Some(DiagnosticSeverity::ERROR));
        }
    }
}

#[test]
fn repeated_counter_errors_on_one_line_have_separate_name_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "😀 \\setcounter{missing}{1} \\stepcounter{missing}\n";
    std::fs::write(&main, source).unwrap();
    let log = format!("(./main.tex\n./main.tex:1: LaTeX Error: No counter 'missing' defined.\n{HELP}l.1 😀 \\setcounter{{missing}}{{1}}\n./main.tex:1: LaTeX Error: No counter 'missing' defined.\n{HELP}l.1 ...counter{{missing}}{{1}} \\stepcounter{{missing}}\n)\n");
    let diagnostics = build_log::parse(&main, &log);
    let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(errors.len(), 2, "{errors:?}");
    let text = Text::new(source.into());
    for (error, (start, _)) in errors.iter().zip(source.match_indices("missing")) {
        assert_eq!(error.range, text.range(start, start + 7));
    }
}

#[test]
fn counter_names_can_be_on_an_earlier_line_than_the_consumed_value_argument() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "\\setcounter\r\n{计数模式}\r\n\r\n{2}\r\n";
    std::fs::write(&main, source).unwrap();
    let log = format!("(./main.tex\n./main.tex:4: LaTeX Error: No counter '计数模式' defined.\n{HELP}l.4 {{2}}\n)\n");
    let diagnostics = build_log::parse(&main, &log);
    let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
    let start = source.find("计数模式").unwrap();
    assert_eq!(
        error.range,
        Text::new(source.into()).range(start, start + "计数模式".len())
    );
    assert_eq!(error.data.as_ref().unwrap()["counter"], "计数模式");
}

#[test]
fn uncertain_counter_errors_keep_line_ranges_without_machine_readable_name_data() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for (source, context) in [
        (r"\setcounter{missing}", "l.1 \\setcounter{missing}"),
        (
            r"\setcounter{ missing }{1}",
            "l.1 \\setcounter{ missing }{1}",
        ),
        (
            r"\setcounter{missing\suffix}{1}",
            "l.1 \\setcounter{missing\\suffix}{1}",
        ),
        (
            r"% \setcounter{missing}{1}",
            "l.1 % \\setcounter{missing}{1}",
        ),
        (
            r"\verb|\setcounter{missing}{1}|",
            "l.1 \\verb|\\setcounter{missing}{1}|",
        ),
        (r"\setcounter{missing}{1}", "l.1 \\setcounter{missing}{2}"),
        (r"\setcounter{missing}{1}", "l.1 ...ssing}{1}"),
        (
            r"\stepcounter{missing}\stepcounter{missing}",
            "l.1 ...{missing}",
        ),
        (
            r"\setcounter{missing}{1}",
            "<argument> \\setcounter{missing}{1}\nl.1 \\setcounter{missing}{1}",
        ),
        (r"\setcounter{missing}{1}", "l.2 \\setcounter{missing}{1}"),
        (r"\broken", "l.1 \\broken"),
    ] {
        std::fs::write(&main, source).unwrap();
        let log = format!("(./main.tex\n./main.tex:1: LaTeX Error: No counter 'missing' defined.\n{HELP}{context}\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert_eq!(
            error.range.start.character, 0,
            "{source}/{context}: {error:?}"
        );
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{source}/{context}: {error:?}"
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_engines_separate_direct_counter_errors_and_leave_macro_expansion_errors_on_the_call_line() {
    use latex_lsp::{build, config::BuildConfig};
    use std::{os::unix::fs::PermissionsExt, sync::atomic::AtomicBool};
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for numbered in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let child = root.join("章节.tex");
            let source = "\\typeout{😀} \\setcounter{missing}{1} \\stepcounter{missing}\n\\addtocounter{missing}{2} \\refstepcounter{missing}\n\\broken\n";
            std::fs::write(&main, "\\documentclass{article}\n\\newcommand{\\broken}{\\setcounter{hidden}{2}}\n\\begin{document}\\input{章节}Witness.\\end{document}\n").unwrap();
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
                    .filter(|error| error.message.starts_with("LaTeX Error: No counter "))
                    .collect();
                assert_eq!(errors.len(), 5, "{engine}/{numbered}: {errors:?}\n{log}");
                let text = Text::new(source.into());
                for (error, (start, _)) in errors[..4].iter().zip(source.match_indices("missing")) {
                    assert_eq!(
                        error.range,
                        text.range(start, start + 7),
                        "{engine}/{numbered}: {error:?}\n{log}"
                    );
                    assert_eq!(
                        error.code,
                        Some(NumberOrString::String("undefined-counter".into()))
                    );
                    assert_eq!(error.data.as_ref().unwrap()["counter"], "missing");
                }
                assert_eq!(
                    errors[4].range,
                    text.range(source.find("\\broken").unwrap(), source.len() - 1)
                );
                assert!(errors[4].code.is_none() && errors[4].data.is_none());
            }
        }
    }
}
