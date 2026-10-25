use latex_lsp::build_log::{compilation_analysis, parse};
use lsp_types::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn warning(message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: Some(DiagnosticSeverity::WARNING),
        message: message.into(),
        ..Default::default()
    }
}

fn excess(amount: &str) -> Diagnostic {
    warning(format!(
        "Overfull \\hbox ({amount}pt too wide) detected at line 1"
    ))
}

fn analyze(items: Vec<Diagnostic>) -> Value {
    compilation_analysis(
        &BTreeMap::from([(Url::parse("file:///box-analysis/main.tex").unwrap(), items)]),
        false,
    )
}

#[test]
fn box_summary_groups_axes_and_kinds_with_independent_maximum_metrics() {
    let mut items = BTreeMap::from([(
        Url::parse("file:///box-analysis/main.tex").unwrap(),
        vec![
            excess("9.9"),
            warning("Overfull \\vbox (5.0pt too high) detected at line 3"),
            warning("Underfull \\hbox (badness 1500) in paragraph at lines 4--5"),
            warning("Underfull \\vbox (badness 10000) has occurred while \\output is active"),
        ],
    )]);
    items.insert(
        Url::parse("file:///box-analysis/child.tex").unwrap(),
        vec![
            excess("12.34567"),
            warning("Underfull \\hbox (badness 9000) in alignment at lines 4--5 []"),
        ],
    );
    let result = compilation_analysis(&items, false);
    assert_eq!(
        result["boxWarnings"],
        json!([
            {"kind":"overfull","axis":"horizontal","count":2,"maxExcessPt":"12.34567"},
            {"kind":"overfull","axis":"vertical","count":1,"maxExcessPt":"5.0"},
            {"kind":"underfull","axis":"horizontal","count":2,"maxBadness":9000},
            {"kind":"underfull","axis":"vertical","count":1,"maxBadness":10000},
        ])
    );
    assert_eq!(result["rerunRequired"], false);
    assert_eq!(result["truncated"], false);
}

#[test]
fn excess_maximum_uses_exact_decimal_comparison_and_a_stable_compiler_spelling() {
    for (amounts, maximum) in [
        (vec!["9.99", "10.0", "2.1"], "10.0"),
        (vec!["0.12", "0.1201", "0.119999"], "0.1201"),
        (vec![".5", "0.500", "00.5", "0.5"], ".5"),
        (vec!["00", "0.", ".0", "0"], ".0"),
        (
            vec!["9007199254740992.1", "9007199254740992.2"],
            "9007199254740992.2",
        ),
        (
            vec!["0.00000000000000000002", "0.00000000000000000001"],
            "0.00000000000000000002",
        ),
    ] {
        let mut items: Vec<_> = amounts.iter().map(|value| excess(value)).collect();
        for _ in 0..2 {
            let result = analyze(items.clone());
            assert_eq!(
                result["boxWarnings"][0]["maxExcessPt"], maximum,
                "{amounts:?}"
            );
            assert_eq!(result["boxWarnings"][0]["count"], amounts.len());
            items.reverse();
        }
    }
}

#[test]
fn partial_nonstandard_and_non_warning_messages_do_not_create_box_groups() {
    for message in [
        "Package local Warning: Overfull \\hbox (19.0pt too wide) detected at line 3",
        "Overfull \\hbox (19.0pt too wide)",
        "Overfull \\hbox (19.0pt too high) detected at line 3",
        "Overfull \\vbox (19.0pt too wide) detected at line 3",
        "Overfull \\hbox (-1pt too wide) detected at line 3",
        "Overfull \\hbox (1..2pt too wide) detected at line 3",
        "Overfull \\hbox (NaNpt too wide) detected at line 3",
        "Overfull \\hbox (infpt too wide) detected at line 3",
        "Overfull \\hbox (1e2pt too wide) detected at line 3",
        "Overfull \\hbox (19.0pt too wide) in paragraph at lines 3",
        "Underfull \\hbox (badness -1) detected at line 3",
        "Underfull \\hbox (badness 1.5) detected at line 3",
        "Underfull \\hbox (badness 4294967296) detected at line 3",
        "Overfull \\hbox (19.0pt too wide) detected at line 3 Extra prose.",
        "Underfull \\hbox (badness 10000) has occurred while \\output is active [] extra",
    ] {
        let result = analyze(vec![warning(message)]);
        assert_eq!(result["boxWarnings"], json!([]), "{message}");
        assert_eq!(result["truncated"], false);
    }
    for severity in [
        None,
        Some(DiagnosticSeverity::ERROR),
        Some(DiagnosticSeverity::INFORMATION),
    ] {
        let mut item = excess("19.0");
        item.severity = severity;
        assert_eq!(analyze(vec![item])["boxWarnings"], json!([]));
    }
}

#[test]
fn box_metric_budget_omits_complete_records_without_hiding_other_categories() {
    let accepted = "1".repeat(128);
    let rejected = "2".repeat(129);
    let result = analyze(vec![
        excess(&accepted),
        excess(&rejected),
        warning("Underfull \\vbox (badness 10000) detected at line 1"),
    ]);
    assert_eq!(
        result["boxWarnings"],
        json!([
            {"kind":"overfull","axis":"horizontal","count":1,"maxExcessPt":accepted},
            {"kind":"underfull","axis":"vertical","count":1,"maxBadness":10000},
        ])
    );
    assert_eq!(result["truncated"], true);
    assert_eq!(
        compilation_analysis(&BTreeMap::new(), true)["boxWarnings"],
        json!([])
    );
    assert_eq!(
        compilation_analysis(&BTreeMap::new(), true)["truncated"],
        true
    );
}

#[test]
fn byte_wrapped_and_numbered_logs_count_diagnostics_across_files_without_box_dumps() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "Main\nSecond\nThird\n").unwrap();
    std::fs::write(&child, "Child\nSecond\nThird\n").unwrap();
    let message = "Overfull \\hbox (12.34567pt too wide) in paragraph at lines 2--3";
    for width in [60, 79] {
        let numbered = format!("./child.tex:2: {message}");
        let wrapped = numbered
            .as_bytes()
            .chunks(width)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let log = format!("(./main.tex\n{wrapped}\n []\n\n{wrapped}\n []\n\n{message}\n []\n\n)\n");
        let parsed = parse(&main, &log);
        assert_eq!(
            parsed.values().map(Vec::len).sum::<usize>(),
            2,
            "{parsed:?}"
        );
        assert_eq!(
            compilation_analysis(&parsed, false)["boxWarnings"],
            json!([
                {"kind":"overfull","axis":"horizontal","count":2,"maxExcessPt":"12.34567"}
            ])
        );
    }
}

#[test]
fn unconfirmed_source_and_generated_line_notes_still_summarize_observed_metrics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let aux = directory.path().join("main.aux");
    std::fs::write(&main, "Main\n").unwrap();
    std::fs::write(&aux, "Generated\n").unwrap();
    let log = "(./main.tex\n./gone.tex:900: Overfull \\hbox (19.0pt too wide) detected at line 900\n./main.aux:900: Underfull \\vbox (badness 10000) detected at line 900\n)\n";
    let parsed = parse(&main, log);
    assert!(parsed.values().flatten().any(|item| item
        .data
        .as_ref()
        .is_some_and(|data| data["unresolvedSource"] == true)));
    assert!(
        parsed.values().flatten().any(|item| item
            .message
            .contains("no longer present in the generated file")),
        "{parsed:?}"
    );
    assert_eq!(
        compilation_analysis(&parsed, false)["boxWarnings"],
        json!([
            {"kind":"overfull","axis":"horizontal","count":1,"maxExcessPt":"19.0"},
            {"kind":"underfull","axis":"vertical","count":1,"maxBadness":10000}
        ])
    );
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_box_summaries_distinguish_four_categories_and_clear_after_layout_repair() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let compiler = root.join(engine);
            std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
            let source = "\\documentclass{article}\n\\begin{document}\n\\setbox0=\\hbox to 1pt{\\hskip 20pt}\n\\setbox1=\\vbox to 1pt{\\vskip 20pt}\n\\setbox2=\\hbox to 20pt{A}\n\\setbox3=\\vbox to 20pt{\\vskip 1pt}\nHello.\n\\end{document}\n";
            std::fs::write(&main, source).unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: compiler.to_string_lossy().into(),
                ..Default::default()
            };
            let outcome = latex_lsp::build::run(&main, &config, &Default::default());
            assert_eq!(
                outcome.status, "success",
                "{engine}/{width}: {}",
                outcome.output
            );
            let analysis = outcome.analysis.unwrap();
            assert_eq!(
                analysis["boxWarnings"],
                json!([
                    {"kind":"overfull","axis":"horizontal","count":1,"maxExcessPt":"19.0"},
                    {"kind":"overfull","axis":"vertical","count":1,"maxExcessPt":"19.0"},
                    {"kind":"underfull","axis":"horizontal","count":1,"maxBadness":10000},
                    {"kind":"underfull","axis":"vertical","count":1,"maxBadness":10000}
                ]),
                "{engine}/{width}: {:?}",
                outcome.diagnostics
            );
            assert_eq!(analysis["rerunRequired"], false);
            assert_eq!(analysis["truncated"], false);
            let fixed = "\\documentclass{article}\n\\begin{document}\nHello.\n\\end{document}\n";
            std::fs::write(&main, fixed).unwrap();
            let outcome = latex_lsp::build::run(&main, &config, &Default::default());
            assert_eq!(
                outcome.status, "success",
                "{engine}/{width}: {}",
                outcome.output
            );
            assert_eq!(
                outcome.analysis.unwrap()["boxWarnings"],
                json!([]),
                "{engine}/{width}"
            );
        }
    }
}
