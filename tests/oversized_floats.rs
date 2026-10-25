use latex_lsp::compilation::build_log::{compilation_analysis, parse};
use lsp_types::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn warning(amount: &str) -> Diagnostic {
    Diagnostic {
        message: format!("LaTeX Warning: Float too large for page by {amount}pt on input line 1."),
        severity: Some(DiagnosticSeverity::WARNING),
        ..Default::default()
    }
}
fn analyze(items: Vec<Diagnostic>) -> Value {
    compilation_analysis(
        &BTreeMap::from([(
            Url::parse("file:///float-analysis/main.tex").unwrap(),
            items,
        )]),
        false,
    )
}

#[test]
fn oversized_floats_report_counts_and_exact_maximum_without_changing_build_requests() {
    for (amounts, maximum) in [
        (vec!["9.9", "10.0", "2.1"], "10.0"),
        (
            vec!["9007199254740992.1", "9007199254740992.2"],
            "9007199254740992.2",
        ),
        (vec![".5", "0.500", "00.5", "0.5"], ".5"),
        (
            vec!["0.00000000000000000002", "0.00000000000000000001"],
            "0.00000000000000000002",
        ),
    ] {
        let mut items: Vec<_> = amounts.iter().map(|amount| warning(amount)).collect();
        for _ in 0..2 {
            let result = analyze(items.clone());
            assert_eq!(
                result["oversizedFloats"],
                json!({"count": amounts.len(), "maxExcessPt":maximum})
            );
            assert_eq!(result["boxWarnings"], json!([]));
            assert_eq!(result["rerunRequired"], false);
            assert_eq!(result["truncated"], false);
            items.reverse();
        }
    }
    assert_eq!(
        analyze(vec![])["oversizedFloats"],
        json!({"count":0,"maxExcessPt":null})
    );
}

#[test]
fn oversized_float_metrics_require_complete_standard_warning_messages() {
    for amount in ["-1", "+1", "1e2", "NaN", "inf", "1..2", ".", "", "１"] {
        let result = analyze(vec![warning(amount)]);
        assert_eq!(result["oversizedFloats"]["count"], 0, "{amount}");
    }
    for message in [
        "LaTeX Warning: Float too large for page by 10.0pt",
        "LaTeX Warning: Float too large for page by 10.0pt on input line 1",
        "LaTeX Warning: Float too large for page by 10.0pt on input line 1. More text.",
        "LaTeX Warning: Float too large for page by 10.0pt on input line 4294967296.",
        "Package local Warning: Float too large for page by 10.0pt on input line 1.",
        "LaTeX Warning: Example: Float too large for page by 10.0pt on input line 1.",
    ] {
        let mut item = warning("10.0");
        item.message = message.into();
        assert_eq!(
            analyze(vec![item])["oversizedFloats"]["count"],
            0,
            "{message}"
        );
    }
    for severity in [
        None,
        Some(DiagnosticSeverity::ERROR),
        Some(DiagnosticSeverity::INFORMATION),
        Some(DiagnosticSeverity::HINT),
    ] {
        let mut item = warning("10.0");
        item.severity = severity;
        assert_eq!(analyze(vec![item])["oversizedFloats"]["count"], 0);
    }
}

#[test]
fn oversized_float_budget_omits_whole_metrics_and_propagates_truncation() {
    let accepted = "1".repeat(128);
    let rejected = "2".repeat(129);
    let result = analyze(vec![warning(&accepted), warning(&rejected)]);
    assert_eq!(
        result["oversizedFloats"],
        json!({"count":1,"maxExcessPt":accepted})
    );
    assert_eq!(result["truncated"], true);
    let result = compilation_analysis(&BTreeMap::new(), true);
    assert_eq!(
        result["oversizedFloats"],
        json!({"count":0,"maxExcessPt":null})
    );
    assert_eq!(result["truncated"], true);
}

#[test]
fn wrapped_float_logs_keep_child_lines_deduplicate_and_classify_without_resizing() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "Main\nSecond\nThird\n").unwrap();
    std::fs::write(&child, "Child\nSecond\nThird\n").unwrap();
    let uri = Url::from_file_path(&child).unwrap();
    let message = "LaTeX Warning: Float too large for page by 123.45678pt on input line 2.";
    for width in [60, 79] {
        for numbered in [false, true] {
            let header = if numbered {
                format!("./child.tex:2: {message}")
            } else {
                message.into()
            };
            let wrapped = header
                .as_bytes()
                .chunks(width)
                .map(|chunk| std::str::from_utf8(chunk).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            let log = format!("(./main.tex\n(./child.tex\n{wrapped}\n\n{wrapped}\n\n)\n)\n");
            let diagnostics = parse(&main, &log);
            assert_eq!(diagnostics[&uri].len(), 1, "{log}: {diagnostics:?}");
            let item = &diagnostics[&uri][0];
            assert_eq!(item.range.start.line, 1);
            assert_eq!(
                item.code,
                Some(NumberOrString::String("oversized-float".into()))
            );
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(item.data.as_ref().unwrap()["excessPt"], "123.45678");
            assert_eq!(
                compilation_analysis(&diagnostics, false)["oversizedFloats"],
                json!({"count":1,"maxExcessPt":"123.45678"})
            );
        }
    }
}

#[test]
fn unconfirmed_float_locations_and_generated_line_notes_preserve_observed_metrics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let aux = directory.path().join("main.aux");
    std::fs::write(&main, "Main\n").unwrap();
    std::fs::write(&aux, "Generated\n").unwrap();
    let log = "(./main.tex\n./gone.tex:900: LaTeX Warning: Float too large for page by 9.5pt on input line 900.\n./main.aux:900: LaTeX Warning: Float too large for page by 10.0pt on input line 900.\n)\n";
    let diagnostics = parse(&main, log);
    assert!(diagnostics.values().flatten().any(|item| item
        .data
        .as_ref()
        .is_some_and(|data| data["unresolvedSource"] == true)));
    assert!(diagnostics.values().flatten().any(|item| item
        .message
        .contains("no longer present in the generated file")));
    assert_eq!(
        compilation_analysis(&diagnostics, false)["oversizedFloats"],
        json!({"count":2,"maxExcessPt":"10.0"})
    );
}

#[test]
fn float_byte_wraps_cover_decimal_points_and_preserve_adjacent_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "First\nSecond\n").unwrap();
    for width in [60, 79] {
        for digits in 1..=80 {
            let amount = format!("{}.25", "1".repeat(digits));
            let header = format!("./main.tex:2: LaTeX Warning: Float too large for page by {amount}pt on input line 2.");
            let wrapped = header
                .as_bytes()
                .chunks(width)
                .map(|chunk| std::str::from_utf8(chunk).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            let log = format!("(./main.tex\n{wrapped}\nLaTeX Warning: Keep the following warning on input line 2.\n)\n");
            let diagnostics = parse(&main, &log);
            assert_eq!(
                diagnostics.values().flatten().count(),
                2,
                "{width}/{digits}: {diagnostics:?}"
            );
            assert_eq!(
                compilation_analysis(&diagnostics, false)["oversizedFloats"],
                json!({"count":1,"maxExcessPt":amount}),
                "{width}/{digits}: {diagnostics:?}"
            );
        }
    }
    for boundary in [
        "LaTeX Warning: Keep the following warning on input line 2.\n",
        "./main.tex:2: Undefined control sequence.\nl.2 \\bad\n",
        "[... log content omitted ...]\n",
    ] {
        let log = format!("(./main.tex\nLaTeX Warning: Float too large for page by 123.45678pt on input\n{boundary})\n");
        let diagnostics = parse(&main, &log);
        assert_eq!(
            compilation_analysis(&diagnostics, false)["oversizedFloats"]["count"],
            0,
            "{diagnostics:?}"
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_oversized_figures_and_tables_clear_after_resizing_child_content() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let child = root.join("child.tex");
                let compiler = root.join(engine);
                std::fs::write(&compiler,format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
                let mut arguments = vec![
                    "-interaction=nonstopmode".into(),
                    "-recorder".into(),
                    "-output-directory=%o".into(),
                    "%f".into(),
                ];
                if numbered {
                    arguments.insert(0, "-file-line-error".into());
                }
                let config = latex_lsp::config::BuildConfig {
                    executable: compiler.to_string_lossy().into(),
                    arguments: Some(arguments),
                    ..Default::default()
                };
                std::fs::write(&main,"\\documentclass{article}\n\\begin{document}\n\\input{child}\n\\end{document}\n").unwrap();
                let source = concat!(
                    "\\begin{figure}\n\\rule{1cm}{1000pt}\n\\end{figure}\n",
                    "\\begin{table}\n\\rule{1cm}{1100pt}\n\\end{table}\n",
                    "Normal.\n\\begin{figure}\\rule{1cm}{10pt}\\end{figure}\n",
                );
                std::fs::write(&child, source).unwrap();
                let outcome =
                    latex_lsp::compilation::build::run(&main, &config, &Default::default());
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{width}/{numbered}: {}",
                    outcome.output
                );
                let uri = Url::from_file_path(&child).unwrap();
                let items: Vec<_> = outcome.diagnostics[&uri]
                    .iter()
                    .filter(|item| {
                        item.code == Some(NumberOrString::String("oversized-float".into()))
                    })
                    .collect();
                assert_eq!(
                    items.len(),
                    2,
                    "{engine}/{width}/{numbered}: {:?}",
                    outcome.diagnostics
                );
                assert_eq!(
                    items
                        .iter()
                        .map(|item| item.range.start.line)
                        .collect::<Vec<_>>(),
                    vec![2, 5]
                );
                let amounts: Vec<_> = items
                    .iter()
                    .map(|item| item.data.as_ref().unwrap()["excessPt"].as_str().unwrap())
                    .collect();
                assert!(amounts[0].parse::<f64>().unwrap() > 400.0);
                assert!(
                    (amounts[1].parse::<f64>().unwrap()
                        - amounts[0].parse::<f64>().unwrap()
                        - 100.0)
                        .abs()
                        < 0.001
                );
                let analysis = outcome.analysis.unwrap();
                assert_eq!(
                    analysis["oversizedFloats"],
                    json!({"count":2,"maxExcessPt":amounts[1]})
                );
                assert_eq!(analysis["rerunRequired"], false);
                assert_eq!(analysis["truncated"], false);
                std::fs::write(
                    &child,
                    source.replace("1000pt", "10pt").replace("1100pt", "20pt"),
                )
                .unwrap();
                let repaired =
                    latex_lsp::compilation::build::run(&main, &config, &Default::default());
                assert_eq!(
                    repaired.status, "success",
                    "{engine}/{width}/{numbered}: {}",
                    repaired.output
                );
                assert_eq!(
                    repaired.analysis.unwrap()["oversizedFloats"],
                    json!({"count":0,"maxExcessPt":null})
                );
                assert!(repaired.diagnostics.values().flatten().all(
                    |item| item.code != Some(NumberOrString::String("oversized-float".into()))
                ));
            }
        }
    }
}
