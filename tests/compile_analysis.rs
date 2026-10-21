use latex_lsp::build_log::{compilation_analysis, parse};
use serde_json::json;
use std::collections::BTreeMap;
use tower_lsp::lsp_types::*;

fn warning(message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: Some(DiagnosticSeverity::WARNING),
        message: message.into(),
        ..Default::default()
    }
}
fn diagnostics(messages: Vec<Diagnostic>) -> BTreeMap<Url, Vec<Diagnostic>> {
    BTreeMap::from([(
        Url::parse("file:///tmp/latex-lsp-analysis/main.tex").unwrap(),
        messages,
    )])
}
fn unresolved(citation: bool, key: &str) -> Diagnostic {
    warning(format!(
        "LaTeX Warning: {} `{key}' on page 1 undefined on input line 1.",
        if citation { "Citation" } else { "Reference" }
    ))
}

#[test]
fn summary_keeps_distinct_literal_keys_without_counting_repeated_uses() {
    let mut items = diagnostics(vec![
        unresolved(false, "章😀"),
        unresolved(false, "O'Connor"),
        unresolved(false, " spaced "),
        unresolved(true, "beta"),
        unresolved(true, "章😀"),
    ]);
    items.insert(
        Url::parse("file:///tmp/latex-lsp-analysis/child.tex").unwrap(),
        vec![unresolved(false, "章😀"), unresolved(true, "beta")],
    );
    let mut ignored = unresolved(false, "not-a-warning");
    ignored.severity = Some(DiagnosticSeverity::ERROR);
    items.values_mut().next().unwrap().push(ignored);
    assert_eq!(
        compilation_analysis(&items, false),
        json!({
            "rerunRequired":false,"unresolvedReferences":[" spaced ","O'Connor","章😀"],
            "unresolvedCitations":["beta","章😀"],"truncated":false,
        })
    );
}

#[test]
fn rerun_requests_require_explicit_known_warning_messages() {
    for message in [
        "LaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.",
        "LaTeX Warning: Label(s) may have changed.\r\n Rerun to get cross-references right.",
        "Package natbib Warning: Citation(s) may have changed. Rerun to get citations correct.",
        "Package natbib Warning: Citation(s) may have changed.",
        "Package rerunfilecheck Warning: File `main.out' has changed.",
        "Package rerunfilecheck Warning: File `main.out' has changed. (rerunfilecheck) Rerun to get outlines right or use package `bookmark'.",
    ] {
        assert_eq!(compilation_analysis(&diagnostics(vec![warning(message)]),false)["rerunRequired"],true,"{message}");
        let mut error=warning(message);error.severity=Some(DiagnosticSeverity::ERROR);
        assert_eq!(compilation_analysis(&diagnostics(vec![error]),false)["rerunRequired"],false);
    }
    for message in [
        "LaTeX Warning: There were undefined references.",
        "LaTeX Warning: Reference `Rerun' on page 1 undefined on input line 1.",
        "Package local Warning: Rerun to get outlines right.",
        "Package rerunfilecheck Warning: File `Rerun to get outlines right' could not open.",
        "Package local Warning: LaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.",
    ] {
        assert_eq!(compilation_analysis(&diagnostics(vec![warning(message)]),false)["rerunRequired"],false,"{message}");
    }
}

#[test]
fn key_budgets_skip_oversize_identities_and_report_partial_analysis() {
    let messages: Vec<_> = (0..130)
        .flat_map(|number| {
            [
                unresolved(false, &format!("ref{number:03}")),
                unresolved(true, &format!("cite{number:03}")),
            ]
        })
        .collect();
    let result = compilation_analysis(&diagnostics(messages), false);
    assert_eq!(
        result["unresolvedReferences"].as_array().unwrap().len(),
        128
    );
    assert_eq!(result["unresolvedCitations"].as_array().unwrap().len(), 128);
    assert_eq!(result["unresolvedReferences"][0], "ref000");
    assert_eq!(result["unresolvedReferences"][127], "ref127");
    assert_eq!(result["truncated"], true);
    let accepted = "😀".repeat(128);
    let skipped = "😀".repeat(129);
    let result = compilation_analysis(
        &diagnostics(vec![
            unresolved(false, &skipped),
            unresolved(false, &accepted),
            unresolved(false, &accepted),
        ]),
        false,
    );
    assert_eq!(result["unresolvedReferences"], json!([accepted]));
    assert_eq!(result["truncated"], true);
}

#[test]
fn parsed_final_log_warnings_and_observation_limits_are_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(
        &root,
        r"\documentclass{article}\begin{document}\ref{missing}\cite{lost}\end{document}",
    )
    .unwrap();
    let log="(./main.tex\nLaTeX Warning: Reference `missing' on page 1 undefined on input line 1.\nLaTeX Warning: Citation `lost' on page 1 undefined on input line 1.\nLaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.\n)\n";
    assert_eq!(
        compilation_analysis(&parse(&root, log), false),
        json!({"rerunRequired":true,"unresolvedReferences":["missing"],"unresolvedCitations":["lost"],"truncated":false})
    );
    let package_log = "(./main.tex\nPackage natbib Warning: Citation(s) may have changed.\n(natbib) Rerun to get citations correct.\n)\n";
    assert_eq!(
        compilation_analysis(&parse(&root, package_log), false)["rerunRequired"],
        true
    );
    let long_key = "a".repeat(96);
    let long_warning =
        format!("LaTeX Warning: Reference `{long_key}' on page 1 undefined on input line 1.");
    let wrapped = long_warning
        .as_bytes()
        .chunks(79)
        .map(|part| std::str::from_utf8(part).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        compilation_analysis(
            &parse(&root, &format!("(./main.tex\n{wrapped}\n)\n")),
            false
        )["unresolvedReferences"],
        json!([long_key])
    );
    assert_eq!(
        compilation_analysis(
            &parse(
                &root,
                "(./main.tex\nOutput written on main.pdf (1 page).\n)\n"
            ),
            true
        ),
        json!({"rerunRequired":false,"unresolvedReferences":[],"unresolvedCitations":[],"truncated":true})
    );
}
