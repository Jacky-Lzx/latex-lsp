use latex_lsp::build_log;
use lsp_types::*;
use serde_json::json;
use std::collections::BTreeMap;

fn analysis(message: &str, severity: DiagnosticSeverity) -> serde_json::Value {
    build_log::compilation_analysis(
        &BTreeMap::from([(
            Url::parse("file:///tmp/index-tools/main.tex").unwrap(),
            vec![Diagnostic {
                message: message.into(),
                severity: Some(severity),
                ..Default::default()
            }],
        )]),
        false,
    )
}

#[test]
fn complete_index_and_glossary_requests_report_makeindex_and_a_later_tex_pass() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Text.\n").unwrap();
    for kind in ["index", "glossary"] {
        for width in [60, 79] {
            for newline in ["\n", "\r\n"] {
                for separator in [" ".to_owned(), format!("{newline}(rerunfilecheck) ")] {
                    let message = format!("Package rerunfilecheck Warning: File `main.track' has changed.{separator}Rerun LaTeX/makeindex to get {kind} right.");
                    let wrapped = message
                        .lines()
                        .map(|line| {
                            line.as_bytes()
                                .chunks(width)
                                .map(|bytes| std::str::from_utf8(bytes).unwrap())
                                .collect::<Vec<_>>()
                                .join(newline)
                        })
                        .collect::<Vec<_>>()
                        .join(newline);
                    let parsed = build_log::parse(
                        &main,
                        &format!("(./main.tex{newline}{wrapped}{newline}){newline}"),
                    );
                    let result = build_log::compilation_analysis(&parsed, false);
                    assert_eq!(
                        result["requiredIndexTools"],
                        json!(["makeindex"]),
                        "{kind}/{width}: {parsed:?}"
                    );
                    assert_eq!(result["requiredBibliographyTools"], json!([]));
                    assert_eq!(result["rerunRequired"], true);
                    assert_eq!(result["truncated"], false);
                }
            }
        }
    }
}

#[test]
fn index_tools_require_complete_known_requests_at_warning_severity() {
    let known = "Package rerunfilecheck Warning: File `main.idx' has changed. Rerun LaTeX/makeindex to get index right.";
    // Tool metadata must use the same normalization as the rerun flag.
    for message in [
        known.to_owned(),
        known.replace("Package ", "Package  "),
        known.replace(" Warning:", "  Warning:\t"),
    ] {
        let result = analysis(&message, DiagnosticSeverity::WARNING);
        assert_eq!(result["rerunRequired"], true);
        assert_eq!(
            result["requiredIndexTools"],
            json!(["makeindex"]),
            "{message}"
        );
    }
    for severity in [
        DiagnosticSeverity::ERROR,
        DiagnosticSeverity::INFORMATION,
        DiagnosticSeverity::HINT,
    ] {
        let result = analysis(known, severity);
        assert_eq!(result["requiredIndexTools"], json!([]));
        assert_eq!(result["rerunRequired"], false);
    }
    for message in [
        "Package rerunfilecheck Warning: File `' has changed. Rerun LaTeX/makeindex to get index right.",
        "Package rerunfilecheck Warning: File `main.idx' has changed. Rerun LaTeX/makeindex to get index right",
        "Package rerunfilecheck Warning: File `main.idx' has changed. Rerun LaTeX/makeindex to get index right. Extra text.",
        "Package rerunfilecheck Warning: File `main.glo' has changed. Example: Rerun LaTeX/makeindex to get glossary right.",
        "Package rerunfilecheck Warning: File `main.idx' has changed. Rerun LaTeX/xindy to get index right.",
        "Package local Warning: File `main.idx' has changed. Rerun LaTeX/makeindex to get index right.",
        "Class rerunfilecheck Warning: File `main.idx' has changed. Rerun LaTeX/makeindex to get index right.",
        "LaTeX Warning: Rerun LaTeX/makeindex to get index right.",
        "Package rerunfilecheck Warning: File `Rerun LaTeX/makeindex to get index right.' has changed. Rerun.",
    ] {
        assert_eq!(analysis(message, DiagnosticSeverity::WARNING)["requiredIndexTools"], json!([]), "{message}");
    }
}

#[test]
fn index_tools_are_deduplicated_independent_of_bibliography_and_cleared_by_the_next_log() {
    let root = Url::parse("file:///tmp/index-tools/main.tex").unwrap();
    let child = Url::parse("file:///tmp/index-tools/child.tex").unwrap();
    let warning = |kind| {
        Diagnostic {
        message: format!("Package rerunfilecheck Warning: File `main.track' has changed. (rerunfilecheck) Rerun LaTeX/makeindex to get {kind} right."),
        severity: Some(DiagnosticSeverity::WARNING), ..Default::default()
    }
    };
    let items = BTreeMap::from([
        (root, vec![warning("index"), warning("glossary")]),
        (child, vec![warning("index"), Diagnostic {
            message: "Package biblatex Warning: Please (re)run Biber on the file: main and rerun LaTeX afterwards.".into(),
            severity: Some(DiagnosticSeverity::WARNING), ..Default::default()
        }]),
    ]);
    for truncated in [false, true] {
        let result = build_log::compilation_analysis(&items, truncated);
        assert_eq!(result["requiredIndexTools"], json!(["makeindex"]));
        assert_eq!(result["requiredBibliographyTools"], json!(["biber"]));
        assert_eq!(result["rerunRequired"], true);
        assert_eq!(result["truncated"], truncated);
    }
    let result = build_log::compilation_analysis(&BTreeMap::new(), false);
    assert_eq!(result["requiredIndexTools"], json!([]));
    assert_eq!(result["requiredBibliographyTools"], json!([]));
    assert_eq!(result["rerunRequired"], false);
}

#[test]
fn wrapped_index_requests_inside_filenames_do_not_change_the_actual_tool_or_source() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    std::fs::write(directory.path().join("child.tex"), "Child text.\n").unwrap();
    std::fs::write(directory.path().join("shadow.tex"), "Healthy text.\n").unwrap();
    let root = Url::from_file_path(&main).unwrap();
    let child = Url::from_file_path(directory.path().join("child.tex")).unwrap();
    let prefix = "Package rerunfilecheck Warning: File `";
    let embedded = "' has changed. Rerun LaTeX/makeindex to get index right.";
    for width in [60, 79] {
        let padding = (width - (prefix.len() + embedded.len()) % width) % width;
        let file = format!("{}{embedded}(shadow.tex).track", "x".repeat(padding));
        for (request, tools) in [
            ("Rerun.", json!([])),
            (
                "Rerun LaTeX/makeindex to get glossary right.",
                json!(["makeindex"]),
            ),
        ] {
            let message = format!("{prefix}{file}' has changed. {request}");
            let wrapped = message
                .as_bytes()
                .chunks(width)
                .map(|bytes| std::str::from_utf8(bytes).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            let log = format!("(./main.tex\n(./child.tex\n{wrapped}\n\nPackage local Warning: Child sentinel on input line 1.\n)\nPackage local Warning: Root sentinel on input line 1.\n)\n");
            let parsed = build_log::parse(&main, &log);
            assert_eq!(parsed.len(), 2, "{width}: {parsed:?}");
            assert_eq!(parsed[&root].len(), 1, "{width}: {parsed:?}");
            assert_eq!(parsed[&child].len(), 2, "{width}: {parsed:?}");
            assert!(
                parsed[&child].iter().any(|item| item.message == message),
                "{width}: {parsed:?}"
            );
            assert_eq!(
                build_log::compilation_analysis(&parsed, false)["requiredIndexTools"],
                tools,
                "{width}: {parsed:?}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX, LuaLaTeX, makeindex, makeidx and rerunfilecheck"]
fn real_index_tool_requests_follow_processing_and_later_index_changes_in_three_engines() {
    use latex_lsp::{build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;
    let cache = tempfile::tempdir().unwrap();
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            std::fs::write(
                directory.path().join("glossary.ist"),
                "keyword \"\\\\glossaryentry\"\n",
            )
            .unwrap();
            let wrapper = directory.path().join("engine");
            std::fs::write(&wrapper, format!("#!/bin/sh\nexport max_print_line={width}\nexport TEXMFVAR=\"$1\"\nexport TEXMFCACHE=\"$1\"\nshift\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = BuildConfig {
                executable: wrapper.to_string_lossy().into_owned(),
                output_directory: Some(".".into()),
                arguments: Some(vec![
                    cache.path().to_string_lossy().into_owned(),
                    "-interaction=nonstopmode".into(),
                    "-halt-on-error".into(),
                    "-no-shell-escape".into(),
                    "-file-line-error".into(),
                    "-recorder".into(),
                    "%f".into(),
                ]),
                ..Default::default()
            };
            for (stage, index, glossary) in [(1, "Alpha", "Gamma"), (2, "Beta", "Delta")] {
                std::fs::write(&main, format!("\\documentclass{{article}}\n\\usepackage{{makeidx}}\n\\usepackage[index,glossary]{{rerunfilecheck}}\n\\makeindex\\makeglossary\n\\begin{{document}}\nText.\\index{{{index}}}\\glossary{{{glossary}}}\n\\printindex\n\\InputIfFileExists{{main.gls}}{{}}{{}}\n\\end{{document}}\n")).unwrap();
                for pass in 1..=3 {
                    let outcome = build::run(&main, &config, &Default::default());
                    assert_eq!(
                        outcome.status, "success",
                        "{engine}/{width}/{stage}/{pass}: {}",
                        outcome.output
                    );
                    let analysis = outcome.analysis.as_ref().unwrap();
                    assert_eq!(
                        analysis["requiredIndexTools"],
                        if pass == 1 {
                            json!(["makeindex"])
                        } else {
                            json!([])
                        },
                        "{engine}/{width}/{stage}/{pass}: {:?}",
                        outcome.diagnostics
                    );
                    assert_eq!(analysis["requiredBibliographyTools"], json!([]));
                    assert_eq!(
                        analysis["rerunRequired"],
                        pass == 1,
                        "{engine}/{width}/{stage}/{pass}: {:?}",
                        outcome.diagnostics
                    );
                    assert_eq!(
                        outcome.pages,
                        Some(if stage == 1 && pass == 1 { 1 } else { 3 })
                    );
                    if pass == 1 {
                        for arguments in [
                            vec!["main.idx"],
                            vec!["-s", "glossary.ist", "-o", "main.gls", "main.glo"],
                        ] {
                            let output = std::process::Command::new("makeindex")
                                .current_dir(directory.path())
                                .args(arguments)
                                .output()
                                .unwrap();
                            assert!(
                                output.status.success(),
                                "{engine}/{width}/{stage}: {}",
                                String::from_utf8_lossy(&output.stderr)
                            );
                        }
                        assert!(std::fs::read_to_string(directory.path().join("main.ind"))
                            .unwrap()
                            .contains(&format!("\\item {index}, 1")));
                        assert!(std::fs::read_to_string(directory.path().join("main.gls"))
                            .unwrap()
                            .contains(&format!("\\item {glossary}, 1")));
                    }
                }
            }
        }
    }
}
