use latex_lsp::build_log::{compilation_analysis, parse};
use serde_json::json;

#[test]
fn biblatex_backend_requests_report_the_tool_and_required_latex_pass() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n").unwrap();
    let log = "(./main.tex\nPackage biblatex Warning: Please (re)run BibTeX on the file(s):\n(biblatex)                main\n(biblatex)                and rerun LaTeX afterwards.\n)\n";
    let analysis = compilation_analysis(&parse(&main, log), false);
    assert_eq!(analysis["rerunRequired"], true, "{analysis}");
    assert_eq!(analysis["requiredBibliographyTools"], json!(["bibtex"]));
}

fn analyze(message: &str, severity: lsp_types::DiagnosticSeverity) -> serde_json::Value {
    use lsp_types::{Diagnostic, Url};
    use std::collections::BTreeMap;
    compilation_analysis(
        &BTreeMap::from([(
            Url::parse("file:///biblatex-rerun/main.tex").unwrap(),
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
fn confirmed_latex_requests_do_not_claim_a_bibliography_tool_is_required() {
    use lsp_types::DiagnosticSeverity;
    for message in [
        "Package biblatex Warning: Please rerun LaTeX.",
        "Package biblatex Warning: Please rerun LaTeX.\r\n(biblatex) Page breaks have changed.",
        "Package biblatex Warning: No localisation for language 'english' loaded.\n(biblatex) Please rerun LaTeX to ensure languages can\n(biblatex) be requested properly.",
    ] {
        let analysis = analyze(message, DiagnosticSeverity::WARNING);
        assert_eq!(analysis["rerunRequired"], true, "{message}");
        assert_eq!(analysis["requiredBibliographyTools"], json!([]));
        assert_eq!(analyze(message, DiagnosticSeverity::ERROR)["rerunRequired"], false);
    }
}

#[test]
fn backend_requests_require_the_complete_known_biblatex_message() {
    use lsp_types::DiagnosticSeverity;
    for (message, expected) in [
        ("Package biblatex Warning: Please (re)run Biber on the file: main and rerun LaTeX afterwards.", "biber"),
        ("Package biblatex Warning: Please (re)run Biber on the file:\r\n(biblatex) 论文 draft.v2\r\n(biblatex) and rerun LaTeX afterwards.", "biber"),
        ("Package biblatex Warning: Please (re)run BibTeX on the file(s):\n(biblatex) main\n(biblatex) main1-blx\n(biblatex) and rerun LaTeX afterwards.", "bibtex"),
    ] {
        let analysis = analyze(message, DiagnosticSeverity::WARNING);
        assert_eq!(analysis["rerunRequired"], true, "{message}");
        assert_eq!(analysis["requiredBibliographyTools"], json!([expected]));
        for severity in [DiagnosticSeverity::ERROR, DiagnosticSeverity::INFORMATION, DiagnosticSeverity::HINT] {
            let analysis = analyze(message, severity);
            assert_eq!(analysis["rerunRequired"], false);
            assert_eq!(analysis["requiredBibliographyTools"], json!([]));
        }
    }
    for body in [
        "Please rerun LaTeX",
        "Example: Please rerun LaTeX.",
        "Please rerun LaTeX. This is a literal example.",
        "Please (re)run Biber on the file: and rerun LaTeX afterwards.",
        "Please (re)run BibTeX on the file(s): main",
        "Please (re)run Biber on the file: main and rerun LaTeX afterwards",
        "Please (re)run Biber on the file: main and rerun LaTeX afterwards. Extra text.",
        "Please (re)run Unknown on the file: main and rerun LaTeX afterwards.",
        "No localisation for language 'english' loaded. Please rerun LaTeX someday.",
    ] {
        let message = format!("Package biblatex Warning: {body}");
        let analysis = analyze(&message, DiagnosticSeverity::WARNING);
        assert_eq!(analysis["rerunRequired"], false, "{message}");
        assert_eq!(
            analysis["requiredBibliographyTools"],
            json!([]),
            "{message}"
        );
    }
    for prefix in [
        "Package local Warning:",
        "Class biblatex Warning:",
        "LaTeX Warning:",
        "Package local Warning: Package biblatex Warning:",
    ] {
        let message =
            format!("{prefix} Please (re)run Biber on the file: main and rerun LaTeX afterwards.");
        assert_eq!(
            analyze(&message, DiagnosticSeverity::WARNING)["rerunRequired"],
            false,
            "{message}"
        );
    }
}

#[test]
fn required_tools_are_deduplicated_sorted_and_cleared_between_analyses() {
    use lsp_types::{Diagnostic, DiagnosticSeverity, Url};
    use std::collections::BTreeMap;
    let make = |tool| {
        Diagnostic { message: format!("Package biblatex Warning: Please (re)run {tool} on the {} main and rerun LaTeX afterwards.", if tool == "Biber" { "file:" } else { "file(s):" }), severity: Some(DiagnosticSeverity::WARNING), ..Default::default() }
    };
    let diagnostics = BTreeMap::from([
        (
            Url::parse("file:///biblatex-rerun/a.tex").unwrap(),
            vec![make("BibTeX"), make("Biber")],
        ),
        (
            Url::parse("file:///biblatex-rerun/b.tex").unwrap(),
            vec![make("Biber")],
        ),
    ]);
    let analysis = compilation_analysis(&diagnostics, false);
    assert_eq!(
        analysis["requiredBibliographyTools"],
        json!(["biber", "bibtex"])
    );
    assert_eq!(analysis["truncated"], false);
    assert_eq!(compilation_analysis(&diagnostics, true)["truncated"], true);
    let analysis = compilation_analysis(&BTreeMap::new(), false);
    assert_eq!(analysis["requiredBibliographyTools"], json!([]));
    assert_eq!(analysis["rerunRequired"], false);
}

#[test]
fn continuation_sentences_are_kept_without_crossing_log_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n").unwrap();
    let uri = lsp_types::Url::from_file_path(&main).unwrap();
    for (body, suffix) in [
        ("Please rerun LaTeX.", "Page breaks have changed."),
        (
            "No localisation for language 'english' loaded.",
            "Please rerun LaTeX to ensure languages can\n(biblatex) be requested properly.",
        ),
    ] {
        let message = format!("Package biblatex Warning: {body}\n(biblatex) {suffix}");
        let log = format!("(./main.tex\n{message}\nLaTeX Warning: Keep this warning.\n)\n");
        let diagnostics = parse(&main, &log);
        assert_eq!(diagnostics[&uri].len(), 2, "{diagnostics:?}");
        assert!(
            diagnostics[&uri]
                .iter()
                .any(|item| item.message.contains(suffix.split('\n').next().unwrap())),
            "{diagnostics:?}"
        );
        assert_eq!(
            compilation_analysis(&diagnostics, false)["rerunRequired"],
            true,
            "{diagnostics:?}"
        );
    }
    let header = "Package biblatex Warning: Please (re)run Biber on the file:\n(biblatex) main\n";
    for boundary in [
        "\n",
        "LaTeX Warning: Another warning.\n",
        "! Undefined control sequence.\nl.1 \\bad\n",
        "[latex-lsp: middle of compiler output omitted]\n",
        ")\n(./main.tex\n",
    ] {
        let log =
            format!("(./main.tex\n{header}{boundary}(biblatex) and rerun LaTeX afterwards.\n)\n");
        assert_eq!(
            compilation_analysis(&parse(&main, &log), false)["requiredBibliographyTools"],
            json!([]),
            "{log}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex, bibtex, biblatex and logreq"]
fn real_biblatex_requests_distinguish_backends_and_clear_after_required_passes() {
    use std::{fs, process::Command};
    for backend in ["bibtex", "biber"] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = format!("\\documentclass{{article}}\n\\usepackage[backend={backend},style=numeric]{{biblatex}}\n\\addbibresource{{refs.bib}}\n\\begin{{document}}\n\\cite{{alpha}}\n\\printbibliography\n\\end{{document}}\n");
        fs::write(&main, &source).unwrap();
        fs::write(
            directory.path().join("refs.bib"),
            "@book{alpha,author={Alpha, Alice},title={Example},year={2020}}\n",
        )
        .unwrap();
        let run = |program: &str, args: &[&str]| {
            let output = Command::new(program)
                .args(args)
                .current_dir(directory.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{program}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let tex = || {
            run(
                "pdflatex",
                &["-interaction=nonstopmode", "-halt-on-error", "main.tex"],
            )
        };
        let analyze = || {
            let log = fs::read_to_string(directory.path().join("main.log")).unwrap();
            compilation_analysis(&parse(&main, &log), false)
        };
        tex();
        let first = analyze();
        assert_eq!(first["rerunRequired"], true, "{first}");
        assert_eq!(
            first["requiredBibliographyTools"],
            json!([backend]),
            "{first}"
        );
        if backend == "bibtex" {
            run("bibtex", &["main"]);
            tex();
            let second = analyze();
            assert_eq!(second["rerunRequired"], true, "{second}");
            assert_eq!(second["requiredBibliographyTools"], json!([]), "{second}");
            tex();
            let final_analysis = analyze();
            assert_eq!(final_analysis["rerunRequired"], false, "{final_analysis}");
            assert_eq!(final_analysis["requiredBibliographyTools"], json!([]));
            assert_eq!(final_analysis["unresolvedCitations"], json!([]));
        }
    }
}
