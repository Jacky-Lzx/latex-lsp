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
            "unresolvedCitations":["beta","章😀"],"unresolvedDestinations":[],"duplicateDestinations":[],"duplicateLabels":[],"requiredBibliographyTools":[],"missingCharacters":[],"truncated":false,
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
        "Package hyperref Warning: Rerun to get /PageLabels entry.",
        "Package hyperref Warning: Rerun to get /PageLabels\n entry.",
        "Package longtable Warning: Table widths have changed. Rerun LaTeX.",
        "Package longtable Warning: Table widths have changed.\r\n Rerun LaTeX.",
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
        "Package local Warning: Rerun to get /PageLabels entry.",
        "Class hyperref Warning: Rerun to get /PageLabels entry.",
        "Package hyperref Warning: Example: Rerun to get /PageLabels entry.",
        "Package hyperref Warning: Rerun to get /PageLabels entry. Example text.",
        "Package hyperref Warning: Rerun to get /PageLabels entry",
        "Package rerunfilecheck Warning: File `Rerun to get outlines right' could not open.",
        "Package local Warning: LaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.",
        "Package longtable Warning: Column widths have changed in table 1 on input line 1.",
        "Package longtable Warning: Table widths have changed.",
        "Class longtable Warning: Table widths have changed. Rerun LaTeX.",
        "Package local Warning: Package longtable Warning: Table widths have changed. Rerun LaTeX.",
    ] {
        assert_eq!(compilation_analysis(&diagnostics(vec![warning(message)]),false)["rerunRequired"],false,"{message}");
    }
}

#[test]
fn reference_examples_remain_warnings_without_retargeting_real_source_uses() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "\\ref{phantom}\n\\cite{phantom}\nwarning call\n").unwrap();
    for prefix in [
        "LaTeX Warning: The string",
        "Package local Warning: The string",
        "Class local Warning: The string",
        "LaTeX Font Warning:",
        "Package local Warning: LaTeX Warning:",
        "Class local Warning: Package nested Warning:",
        "Package local Warning: File example.tex contains",
    ] {
        for kind in ["Reference", "Hyper reference", "Citation"] {
            let message =
                format!("{prefix} {kind} `phantom' undefined is literal on input line 3.");
            let parsed = parse(&root, &format!("(./main.tex\n{message}\n)\n"));
            let items = &parsed[&Url::from_file_path(&root).unwrap()];
            assert_eq!(items.len(), 1, "{message}");
            assert_eq!(items[0].message, message);
            assert_eq!(items[0].severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(items[0].range.start.line, 2, "{message}");
            assert_eq!(
                compilation_analysis(&parsed, false),
                json!({"rerunRequired":false,"unresolvedReferences":[],"unresolvedCitations":[],"unresolvedDestinations":[],"duplicateDestinations":[],"duplicateLabels":[],"requiredBibliographyTools":[],"missingCharacters":[],"truncated":false}),
                "{message}"
            );
        }
    }
    for message in [
        "Wrapper: LaTeX Warning: Reference `phantom' undefined.",
        "Reference `phantom' undefined.tex:1: Example warning.",
        "Package Reference `phantom' undefined Warning: File unavailable.",
        "Class Citation `phantom' undefined Warning: Option ignored.",
        "Package  Warning: Reference `phantom' undefined.",
    ] {
        let result = compilation_analysis(&diagnostics(vec![warning(message)]), false);
        assert_eq!(result["unresolvedReferences"], json!([]), "{message}");
        assert_eq!(result["unresolvedCitations"], json!([]), "{message}");
    }
}

#[test]
fn standard_warning_bodies_keep_package_class_and_quote_variants() {
    for header in [
        "LaTeX Warning:",
        "Package custom Warning:",
        "Class custom Warning:",
    ] {
        for quote in ['`', '\''] {
            for (kind, citation) in [
                ("Reference", false),
                ("Hyper reference", false),
                ("Citation", true),
            ] {
                let message = format!(
                    "{header}  {kind} {quote}O'Connor' on page 1 undefined on input line 1."
                );
                let result = compilation_analysis(&diagnostics(vec![warning(&message)]), false);
                assert_eq!(
                    result["unresolvedReferences"],
                    if citation {
                        json!([])
                    } else {
                        json!(["O'Connor"])
                    },
                    "{message}"
                );
                assert_eq!(
                    result["unresolvedCitations"],
                    if citation {
                        json!(["O'Connor"])
                    } else {
                        json!([])
                    },
                    "{message}"
                );
            }
        }
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
        json!({"rerunRequired":true,"unresolvedReferences":["missing"],"unresolvedCitations":["lost"],"unresolvedDestinations":[],"duplicateDestinations":[],"duplicateLabels":[],"requiredBibliographyTools":[],"missingCharacters":[],"truncated":false})
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
        json!({"rerunRequired":false,"unresolvedReferences":[],"unresolvedCitations":[],"unresolvedDestinations":[],"duplicateDestinations":[],"duplicateLabels":[],"requiredBibliographyTools":[],"missingCharacters":[],"truncated":true})
    );
}

#[test]
fn parenthesized_wrapped_keys_preserve_identity_and_diagnostic_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    for (kind, command, citation) in [
        ("Reference", "ref", false),
        ("Hyper reference", "hyperref", false),
        ("Citation", "cite", true),
    ] {
        for quote in ['`', '\''] {
            let prefix = format!("LaTeX Warning: {kind} {quote}");
            for width in [60, 79] {
                for mark in ['(', ')'] {
                    let key = format!(
                        "{}{mark}{}",
                        "a".repeat(width - prefix.len()),
                        "b".repeat(60)
                    );
                    let source = if command == "hyperref" {
                        format!("\\hyperref[{key}]{{Link}}\n")
                    } else {
                        format!("\\{command}{{{key}}}\n")
                    };
                    std::fs::write(&root, &source).unwrap();
                    let message = format!("{prefix}{key}' on page 1 undefined on input line 1.");
                    let wrapped = message
                        .as_bytes()
                        .chunks(width)
                        .map(|part| std::str::from_utf8(part).unwrap())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let parsed = parse(&root, &format!("(./main.tex\n{wrapped}\n)\n"));
                    let items = &parsed[&Url::from_file_path(&root).unwrap()];
                    assert_eq!(items.len(), 1, "{wrapped}");
                    assert_eq!(items[0].message, message, "{wrapped}");
                    let text = latex_lsp::text::Text::new(source.clone());
                    assert_eq!(
                        &source[text.offset(items[0].range.start).unwrap()
                            ..text.offset(items[0].range.end).unwrap()],
                        key,
                        "{wrapped}"
                    );
                    let result = compilation_analysis(&parsed, false);
                    assert_eq!(
                        result["unresolvedReferences"],
                        if citation { json!([]) } else { json!([key]) },
                        "{wrapped}"
                    );
                    assert_eq!(
                        result["unresolvedCitations"],
                        if citation { json!([key]) } else { json!([]) },
                        "{wrapped}"
                    );
                }
            }
        }
    }
}

#[test]
fn incomplete_wrapped_references_preserve_real_file_and_message_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&root, "\\ref{rootmissing}\n\\input{child}\n").unwrap();
    std::fs::write(&child, "\\ref{childmissing}\n").unwrap();
    let partial = format!("LaTeX Warning: Reference `{}", "a".repeat(53));
    assert_eq!(partial.len(), 79);
    let root_warning =
        "LaTeX Warning: Reference `rootmissing' on page 1 undefined on input line 1.";
    let child_warning =
        "LaTeX Warning: Reference `childmissing' on page 1 undefined on input line 1.";
    for (log, expected_file, expected_key) in [
        (
            format!("(./main.tex\n(./child.tex\n{partial}\n)\n{root_warning}\n)\n"),
            &root,
            "rootmissing",
        ),
        (
            format!("(./main.tex\n{partial}\n(./child.tex\n{child_warning}\n)\n)\n"),
            &child,
            "childmissing",
        ),
        (
            format!("(./main.tex\n{partial}\n! Other error.\nl.1 bad\n{root_warning}\n)\n"),
            &root,
            "rootmissing",
        ),
        (
            format!(
                "(./main.tex\n{partial}\nOutput written on main.pdf (1 page).\n{root_warning}\n)\n"
            ),
            &root,
            "rootmissing",
        ),
    ] {
        let parsed = parse(&root, &log);
        assert!(
            parsed
                .values()
                .flatten()
                .any(|item| item.message == partial),
            "{log}\n{parsed:?}"
        );
        let items = &parsed[&Url::from_file_path(expected_file).unwrap()];
        let complete = items
            .iter()
            .find(|item| item.message.contains(expected_key))
            .unwrap();
        let source = std::fs::read_to_string(expected_file).unwrap();
        let text = latex_lsp::text::Text::new(source.clone());
        assert_eq!(
            &source[text.offset(complete.range.start).unwrap()
                ..text.offset(complete.range.end).unwrap()],
            expected_key,
            "{log}"
        );
        assert_eq!(
            compilation_analysis(&parsed, false)["unresolvedReferences"],
            json!([expected_key]),
            "{log}"
        );
    }
}

#[test]
fn longtable_requests_another_pass_without_inventing_missing_references() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(
        &root,
        "\\begin{longtable}{ll}One & Two\\\\\\end{longtable}\n",
    )
    .unwrap();
    let columns = "Package longtable Warning: Column widths have changed\n(longtable) in table 1 on input line 1.\n";
    for (summary, rerun) in [
        (
            "Package longtable Warning: Table widths have changed. Rerun LaTeX.\n",
            true,
        ),
        (
            "Package longtable Warning: Table widths have changed. Rerun\n LaTeX.\n",
            true,
        ),
        ("", false),
        (
            "Package local Warning: Example: Table widths have changed. Rerun LaTeX.\n",
            false,
        ),
    ] {
        let parsed = parse(&root, &format!("(./main.tex\n{columns}\n{summary}\n)\n"));
        assert!(parsed
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.message.contains("Column widths have changed")));
        assert_eq!(
            compilation_analysis(&parsed, false),
            json!({"rerunRequired":rerun,"unresolvedReferences":[],"unresolvedCitations":[],"unresolvedDestinations":[],"duplicateDestinations":[],"duplicateLabels":[],"requiredBibliographyTools":[],"missingCharacters":[],"truncated":false})
        );
    }
}

fn destination_warning(duplicate: bool, key: &str) -> Diagnostic {
    warning(if duplicate {
        format!("warning  (pdf backend): ignoring duplicate destination with the name '{key}'")
    } else {
        format!("warning  (pdf backend): unreferenced destination with name '{key}'")
    })
}

#[test]
fn pdf_destination_summary_decodes_and_deduplicates_names_across_files_and_backends() {
    let mut items = diagnostics(vec![
        destination_warning(false, r"dest\(old\)"),
        destination_warning(false, r"emoji\360\237\230\200"),
        destination_warning(true, r"dest\050old\051"),
        unresolved(false, "dest(old)"),
        unresolved(true, "emoji😀"),
    ]);
    items.insert(
        Url::parse("file:///tmp/latex-lsp-analysis/child.tex").unwrap(),
        vec![
            warning("pdfTeX warning (dest): name{dest\\(old\\)} has been referenced but does not exist, replaced by a fixed one"),
            destination_warning(false, "emoji😀"),
            warning("pdfTeX warning (ext4): destination with the same identifier (name{dest\\(old\\)}) has been already used, duplicate ignored"),
        ],
    );
    assert_eq!(
        compilation_analysis(&items, false),
        json!({
            "rerunRequired":false,"unresolvedReferences":["dest(old)"],
            "unresolvedCitations":["emoji😀"],"unresolvedDestinations":["dest(old)","emoji😀"],
            "duplicateDestinations":["dest(old)"],"duplicateLabels":[],"requiredBibliographyTools":[],"missingCharacters":[],"truncated":false
        })
    );
}

#[test]
fn pdf_destination_summary_rejects_uncertain_records_and_untrusted_diagnostic_data() {
    let mut messages = vec![];
    for message in [
        "Package local Warning: Example: warning  (pdf backend): unreferenced destination with name 'phantom'",
        "pdfTeX warning (dest): name{phantom} has been referenced",
        "warning  (pdf backend): unreferenced destination with name 'phantom' was resolved",
        "warning  (pdf backend): ignoring duplicate destination with the name 'phantom' was resolved",
    ] {
        messages.push(warning(message));
    }
    for key in [
        r"dynamic\name",
        r"bad\400",
        r"bad\377",
        r"bad\000",
        r"bad\040",
        " spaced ",
    ] {
        messages.push(destination_warning(false, key));
        messages.push(destination_warning(true, key));
    }
    for severity in [
        None,
        Some(DiagnosticSeverity::ERROR),
        Some(DiagnosticSeverity::INFORMATION),
    ] {
        let mut item = destination_warning(false, "phantom");
        item.severity = severity;
        messages.push(item);
    }
    let mut forged = warning("Package local Warning: Ordinary warning.");
    forged.code = Some(NumberOrString::String("undefined-pdf-destination".into()));
    forged.data = Some(json!({"destination":"phantom"}));
    messages.push(forged);
    assert_eq!(
        compilation_analysis(&diagnostics(messages), false),
        json!({
            "rerunRequired":false,"unresolvedReferences":[],"unresolvedCitations":[],
            "unresolvedDestinations":[],"duplicateDestinations":[],"duplicateLabels":[],"requiredBibliographyTools":[],"missingCharacters":[],"truncated":false
        })
    );
}

#[test]
fn pdf_destination_summary_has_independent_count_and_unicode_identity_budgets() {
    let messages = (0..128)
        .flat_map(|index| {
            [
                destination_warning(false, &format!("missing{index:03}")),
                destination_warning(true, &format!("duplicate{index:03}")),
            ]
        })
        .chain([
            destination_warning(false, "missing000"),
            destination_warning(true, "duplicate000"),
        ])
        .collect();
    let mut items = diagnostics(messages);
    let result = compilation_analysis(&items, false);
    assert_eq!(
        result["unresolvedDestinations"].as_array().unwrap().len(),
        128
    );
    assert_eq!(
        result["duplicateDestinations"].as_array().unwrap().len(),
        128
    );
    assert_eq!(result["unresolvedDestinations"][127], "missing127");
    assert_eq!(result["duplicateDestinations"][0], "duplicate000");
    assert_eq!(result["truncated"], false);
    items
        .values_mut()
        .next()
        .unwrap()
        .push(destination_warning(false, "overflow"));
    assert_eq!(compilation_analysis(&items, false)["truncated"], true);
    let accepted = "😀".repeat(128);
    let rejected = "😀".repeat(129);
    let messages = vec![
        destination_warning(false, &accepted),
        destination_warning(true, &accepted),
        destination_warning(false, &rejected),
        destination_warning(true, &rejected),
    ];
    let result = compilation_analysis(&diagnostics(messages), false);
    assert_eq!(result["unresolvedDestinations"], json!([accepted]));
    assert_eq!(result["duplicateDestinations"], json!([accepted]));
    assert_eq!(result["truncated"], true);
}

#[test]
fn pdf_destination_summary_uses_complete_byte_wrapped_warnings_and_observation_limits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "Main source\n").unwrap();
    for (message, field) in [
        (
            r"warning  (pdf backend): unreferenced destination with name 'mix\(\347\233\256\346\240\207\360\237\230\200\)'",
            "unresolvedDestinations",
        ),
        (
            r"warning  (pdf backend): ignoring duplicate destination with the name 'mix\(\347\233\256\346\240\207\360\237\230\200\)'",
            "duplicateDestinations",
        ),
    ] {
        let wrapped = message
            .as_bytes()
            .chunks(79)
            .map(|part| std::str::from_utf8(part).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = parse(&root, &format!("(./main.tex\n{wrapped}\n)\n"));
        let result = compilation_analysis(&parsed, false);
        assert_eq!(result[field], json!(["mix(目标😀)"]));
        assert_eq!(result["rerunRequired"], false);
        assert_eq!(result["truncated"], false);
        assert_eq!(compilation_analysis(&parsed, true)["truncated"], true);
    }
}

#[test]
fn parsed_hyperref_page_label_requests_clear_with_the_final_log() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "Main source\n").unwrap();
    for message in [
        "Package hyperref Warning: Rerun to get /PageLabels entry.\n",
        "Package hyperref Warning: Rerun to get /PageLabels\n entry.\n",
    ] {
        let parsed = parse(&root, &format!("(./main.tex\n{message})\n"));
        assert_eq!(parsed.values().map(Vec::len).sum::<usize>(), 1);
        let analysis = compilation_analysis(&parsed, false);
        assert_eq!(analysis["rerunRequired"], true);
        for field in [
            "unresolvedReferences",
            "unresolvedCitations",
            "unresolvedDestinations",
            "duplicateDestinations",
        ] {
            assert_eq!(analysis[field], json!([]));
        }
        assert_eq!(analysis["truncated"], false);
    }
    assert_eq!(
        compilation_analysis(
            &parse(
                &root,
                "(./main.tex\nOutput written on main.pdf (1 page).\n)\n"
            ),
            false
        )["rerunRequired"],
        false
    );
}

fn label_warning(key: &str) -> Diagnostic {
    warning(format!("LaTeX Warning: Label `{key}' multiply defined."))
}

#[test]
fn duplicate_label_summary_preserves_literal_names_and_keeps_other_key_categories_independent() {
    let mut items = diagnostics(vec![
        label_warning("章😀"),
        label_warning("O'Connor"),
        label_warning(" spaced "),
        unresolved(false, "O'Connor"),
        unresolved(true, "章😀"),
        destination_warning(false, "lost"),
        destination_warning(true, "dup"),
    ]);
    items.insert(
        Url::parse("file:///tmp/latex-lsp-analysis/child.tex").unwrap(),
        vec![label_warning("O'Connor"), label_warning("章😀")],
    );
    assert_eq!(
        compilation_analysis(&items, false),
        json!({
            "rerunRequired":false,"unresolvedReferences":["O'Connor"],"unresolvedCitations":["章😀"],
            "unresolvedDestinations":["lost"],"duplicateDestinations":["dup"],
            "duplicateLabels":[" spaced ","O'Connor","章😀"],"requiredBibliographyTools":[],"missingCharacters":[],"truncated":false
        })
    );
}

#[test]
fn duplicate_label_summary_requires_a_complete_standard_warning_body() {
    for message in [
        "Package local Warning: Label `phantom' multiply defined.",
        "Class local Warning: Label `phantom' multiply defined.",
        "LaTeX Warning: Example: Label `phantom' multiply defined.",
        "LaTeX Warning: Label `phantom' multiply defined. Extra text.",
        "LaTeX Warning: Label `phantom' multiply defined",
        "LaTeX Warning: Label `' multiply defined.",
        "LaTeX Warning: There were multiply-defined labels.",
    ] {
        let mut item = warning(message);
        item.code = Some(NumberOrString::String("duplicate-label".into()));
        item.data = Some(json!({"label":"phantom"}));
        assert_eq!(
            compilation_analysis(&diagnostics(vec![item]), false)["duplicateLabels"],
            json!([]),
            "{message}"
        );
    }
    for quote in ['`', '\''] {
        let message = format!("LaTeX Warning: Label {quote}O'Connor' multiply defined.");
        assert_eq!(
            compilation_analysis(&diagnostics(vec![warning(&message)]), false)["duplicateLabels"],
            json!(["O'Connor"])
        );
        for severity in [
            None,
            Some(DiagnosticSeverity::ERROR),
            Some(DiagnosticSeverity::INFORMATION),
        ] {
            let mut item = warning(&message);
            item.severity = severity;
            assert_eq!(
                compilation_analysis(&diagnostics(vec![item]), false)["duplicateLabels"],
                json!([])
            );
        }
    }
}

#[test]
fn duplicate_label_summary_applies_count_and_unicode_budgets_without_truncating_names() {
    let mut messages: Vec<_> = (0..128)
        .map(|i| label_warning(&format!("label{i:03}")))
        .collect();
    messages.push(label_warning("label000"));
    messages.push(unresolved(false, "other"));
    let result = compilation_analysis(&diagnostics(messages.clone()), false);
    assert_eq!(result["duplicateLabels"].as_array().unwrap().len(), 128);
    assert_eq!(result["duplicateLabels"][0], "label000");
    assert_eq!(result["duplicateLabels"][127], "label127");
    assert_eq!(result["unresolvedReferences"], json!(["other"]));
    assert_eq!(result["truncated"], false);
    messages.push(label_warning("overflow"));
    assert_eq!(
        compilation_analysis(&diagnostics(messages), false)["truncated"],
        true
    );
    let accepted = "😀".repeat(128);
    let rejected = "😀".repeat(129);
    let items = diagnostics(vec![
        label_warning(&rejected),
        label_warning(&accepted),
        label_warning(&accepted),
    ]);
    let result = compilation_analysis(&items, false);
    assert_eq!(result["duplicateLabels"], json!([accepted]));
    assert_eq!(result["truncated"], true);
}

#[test]
fn duplicate_label_byte_wraps_preserve_parentheses_apostrophes_and_unicode_names() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "Main source\n").unwrap();
    let prefix = "LaTeX Warning: Label `";
    for key in [
        format!("{}(tail)", "x".repeat(79 - prefix.len())),
        format!("{})tail", "x".repeat(79 - prefix.len())),
        format!("{}'tail", "x".repeat(79 - prefix.len())),
        "章节😀".repeat(20),
    ] {
        let message = format!("{prefix}{key}' multiply defined.");
        let mut lines = vec![];
        let mut start = 0;
        while start < message.len() {
            let mut end = (start + 79).min(message.len());
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            lines.push(&message[start..end]);
            start = end;
        }
        let parsed = parse(
            &root,
            &format!(
                "(./main.tex\n{}\nLaTeX Warning: Keep.\n)\n",
                lines.join("\n")
            ),
        );
        let result = compilation_analysis(&parsed, false);
        assert_eq!(result["duplicateLabels"], json!([key]), "{parsed:?}");
        assert_eq!(result["rerunRequired"], false);
        assert_eq!(result["truncated"], false);
        assert!(
            parsed.values().flatten().any(|d| d.message == message),
            "{parsed:?}"
        );
        assert!(parsed
            .values()
            .flatten()
            .any(|d| d.message == "LaTeX Warning: Keep."));
        assert_eq!(compilation_analysis(&parsed, true)["truncated"], true);
    }
}

#[test]
fn duplicate_label_footer_at_the_print_width_cannot_hide_the_full_key_or_following_error() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    let child = directory.path().join("错误.tex");
    std::fs::write(&root, "Main source\n").unwrap();
    std::fs::write(&child, "Child source\n").unwrap();
    let prefix = "LaTeX Warning: Label `";
    let footer = "' multiply defined.";
    let short = "x".repeat(79 - prefix.len() - footer.len());
    for suffix in ["-tail", "😀tail"] {
        let full = format!("{short}{footer}{suffix}");
        let log = format!(
            "{prefix}{short}{footer}\n{suffix}{footer}\n错误.tex:1: LaTeX Error: Boundary.\n"
        );
        let parsed = parse(&root, &log);
        assert_eq!(
            compilation_analysis(&parsed, false)["duplicateLabels"],
            json!([full]),
            "{parsed:?}"
        );
        assert_eq!(
            parsed[&Url::from_file_path(&child).unwrap()][0].message,
            "LaTeX Error: Boundary."
        );
    }
    let parsed = parse(
        &root,
        &format!("{prefix}{short}{footer}\n错误.tex:1: LaTeX Error: Boundary.\n"),
    );
    assert_eq!(
        compilation_analysis(&parsed, false)["duplicateLabels"],
        json!([short])
    );
    assert_eq!(
        parsed[&Url::from_file_path(&child).unwrap()][0].message,
        "LaTeX Error: Boundary."
    );
    let partial = format!("{prefix}{}", "x".repeat(79 - prefix.len()));
    let parsed=parse(&root,&format!("{partial}\n(./错误.tex\nLaTeX Warning: Child sentinel.\n)\nLaTeX Warning: Main sentinel.\n"));
    assert_eq!(
        compilation_analysis(&parsed, false)["duplicateLabels"],
        json!([])
    );
    assert!(parsed[&Url::from_file_path(&child).unwrap()]
        .iter()
        .any(|d| d.message == "LaTeX Warning: Child sentinel."));
    assert!(parsed[&Url::from_file_path(&root).unwrap()]
        .iter()
        .any(|d| d.message == "LaTeX Warning: Main sentinel."));
}

#[test]
fn undefined_words_in_keys_pages_and_prose_do_not_imply_unresolved_references() {
    for header in [
        "LaTeX Warning:",
        "Package local Warning:",
        "Class local Warning:",
    ] {
        for kind in ["Reference", "Hyper reference", "Citation"] {
            for quote in ['`', '\''] {
                for body in [
                    "undefined' on page 1 is documented on input line 1.",
                    "undefinedness' on page 1 is documented on input line 1.",
                    "healthy' on page undefined is documented on input line 1.",
                    "healthy' on page 1 without undefined behavior on input line 1.",
                    "healthy' on page 1 undefinedness on input line 1.",
                    "healthy' undefinedness on input line 1.",
                    "healthy' undefined is an example on input line 1.",
                    "healthy' on page 1 undefined on input line invalid.",
                ] {
                    let message = format!("{header} {kind} {quote}{body}");
                    let analysis =
                        compilation_analysis(&diagnostics(vec![warning(&message)]), false);
                    assert_eq!(analysis["unresolvedReferences"], json!([]), "{message}");
                    assert_eq!(analysis["unresolvedCitations"], json!([]), "{message}");
                }
            }
        }
    }
}

#[test]
fn standard_undefined_status_tails_preserve_literal_keys_pages_and_package_continuations() {
    for key in [
        "undefined",
        "O'Connor",
        "chapter' on paper",
        "chapter' undefined marker",
        "章😀",
    ] {
        for status in [
            "undefined.",
            "undefined on input line 1.",
            "undefined\n(biblatex) on input line 1.",
            "on page 1 undefined on input line 1.",
            "on page iv undefined on input line 1.",
            "on page  undefined on input line 1.",
            "on page Page 1 undefined on input line 1.",
            "on page undefined undefined on input line 1.",
        ] {
            for (kind, field) in [
                ("Reference", "unresolvedReferences"),
                ("Hyper reference", "unresolvedReferences"),
                ("Citation", "unresolvedCitations"),
            ] {
                let message = format!("Package local Warning: {kind} `{key}' {status}");
                let analysis = compilation_analysis(&diagnostics(vec![warning(&message)]), false);
                assert_eq!(analysis[field], json!([key]), "{message}");
            }
        }
    }
}

#[test]
fn ordinary_warning_status_text_keeps_line_ranges_without_borrowing_valid_use_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    let source = "😀 \\ref{undefined} \\cite{undefined}\n";
    std::fs::write(&root, source).unwrap();
    for kind in ["Reference", "Citation"] {
        let message = format!(
            "Package local Warning: {kind} `undefined' on page 1 is documented on input line 1."
        );
        let parsed = parse(&root, &format!("(./main.tex\n{message}\n)\n"));
        let items = &parsed[&Url::from_file_path(&root).unwrap()];
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].message, message);
        assert_eq!(items[0].severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(
            items[0].range,
            Range::new(Position::new(0, 0), Position::new(0, 35))
        );
        let analysis = compilation_analysis(&parsed, false);
        assert_eq!(analysis["unresolvedReferences"], json!([]));
        assert_eq!(analysis["unresolvedCitations"], json!([]));
    }
}

#[test]
fn full_warning_footers_inside_byte_wrapped_reference_keys_do_not_name_healthy_prefixes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    let footer = "' on page 1 undefined on input line 1.";
    for (kind, command, field) in [
        ("Reference", "ref", "unresolvedReferences"),
        ("Hyper reference", "hyperref", "unresolvedReferences"),
        ("Citation", "cite", "unresolvedCitations"),
    ] {
        for quote in ['`', '\''] {
            let prefix = format!("LaTeX Warning: {kind} {quote}");
            let short = "x".repeat(79 - prefix.len() - footer.len());
            for suffix in ["-tail", "(tail)", ")tail", "😀tail"] {
                let key = format!("{short}{footer}{suffix}");
                let source = if command == "hyperref" {
                    format!("\\hyperref[{short}]{{Healthy}}\\hyperref[{key}]{{Missing}}\n")
                } else {
                    format!("\\{command}{{{short}}}\\{command}{{{key}}}\n")
                };
                std::fs::write(&root, &source).unwrap();
                let message = format!("{prefix}{key}{footer}");
                let boundary = prefix.len() + short.len() + footer.len();
                assert_eq!(boundary, 79);
                let wrapped = format!("{}\n{}", &message[..boundary], &message[boundary..]);
                let parsed = parse(&root, &format!("(./main.tex\n{wrapped}\n)\n"));
                let items = &parsed[&Url::from_file_path(&root).unwrap()];
                assert_eq!(items.len(), 1, "{wrapped}");
                assert_eq!(items[0].message, message, "{wrapped}");
                assert_eq!(items[0].range.start, Position::new(0, 0), "{wrapped}");
                assert_eq!(
                    items[0].range.end,
                    Position::new(0, source.trim_end().encode_utf16().count() as u32)
                );
                let analysis = compilation_analysis(&parsed, false);
                assert_eq!(analysis[field], json!([key]), "{wrapped}");
                assert_ne!(analysis[field], json!([short]), "{wrapped}");
            }
        }
    }
}

#[test]
fn a_complete_reference_warning_at_print_width_preserves_following_file_and_error_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    let prefix = "LaTeX Warning: Reference `";
    let footer = "' on page 1 undefined on input line 1.";
    let key = "x".repeat(79 - prefix.len() - footer.len());
    std::fs::write(&root, format!("\\ref{{{key}}}\n")).unwrap();
    std::fs::write(&child, "\\ref{childmissing}\n\\bad\n").unwrap();
    let warning = format!("{prefix}{key}{footer}");
    assert_eq!(warning.len(), 79);
    for entry in ["(./child.tex", "./child.tex:1: LaTeX Warning: Reference `childmissing' on page 1 undefined on input line 1."] {
        let following = if entry.starts_with('(') { "\nLaTeX Warning: Reference `childmissing' on page 1 undefined on input line 1.\n./child.tex:2: Undefined control sequence.\nl.2 \\bad\n)" } else { "\n./child.tex:2: Undefined control sequence.\nl.2 \\bad" };
        let parsed = parse(&root, &format!("(./main.tex\n{warning}\n{entry}{following}\n)\n"));
        let main_items = &parsed[&Url::from_file_path(&root).unwrap()];
        assert_eq!(main_items.len(), 1);
        assert_eq!(main_items[0].message, warning);
        let child_items = &parsed[&Url::from_file_path(&child).unwrap()];
        assert_eq!(child_items.len(), 2, "{child_items:?}");
        assert!(child_items.iter().any(|item| item.message.contains("childmissing") && item.range.start.line == 0));
        assert!(child_items.iter().any(|item| item.message.contains("Undefined control sequence") && item.range.start.line == 1));
        let analysis = compilation_analysis(&parsed, false);
        let mut expected = vec![key.as_str(), "childmissing"];expected.sort();
        assert_eq!(analysis["unresolvedReferences"], json!(expected));
    }
}

fn missing_glyph(character: char, font: &str, severity: DiagnosticSeverity) -> Diagnostic {
    Diagnostic {
        severity: Some(severity),
        message: format!(
            "Missing character: There is no {character} (U+{:04X}) in font {font}.",
            character as u32
        ),
        ..Default::default()
    }
}

#[test]
fn missing_character_summary_combines_fonts_files_and_highest_severity() {
    let mut items = diagnostics(vec![
        missing_glyph('中', "font-b", DiagnosticSeverity::WARNING),
        missing_glyph('😀', "emoji-font", DiagnosticSeverity::WARNING),
        missing_glyph('中', "font-b", DiagnosticSeverity::WARNING),
    ]);
    items.insert(
        Url::parse("file:///tmp/latex-lsp-analysis/child.tex").unwrap(),
        vec![missing_glyph('中', "font-a", DiagnosticSeverity::ERROR)],
    );
    let result = compilation_analysis(&items, false);
    assert_eq!(
        result["missingCharacters"],
        json!([
            {"character":"中", "codepoint":"U+4E2D", "severity":1, "fonts":["font-a", "font-b"]},
            {"character":"😀", "codepoint":"U+1F600", "severity":2, "fonts":["emoji-font"]},
        ])
    );
    assert_eq!(result["truncated"], false);
    assert_eq!(result["rerunRequired"], false);
}

#[test]
fn missing_character_summary_rejects_examples_bad_identity_and_untrusted_data() {
    let mut messages = Vec::new();
    let valid = missing_glyph('中', "font", DiagnosticSeverity::WARNING);
    for prefix in [
        "LaTeX Warning: Example: ",
        "Package font Warning: ",
        "Example: ",
    ] {
        messages.push(warning(format!("{prefix}{}", valid.message)));
    }
    for message in [
        "Missing character: There is no 中 (U+1F600) in font font.",
        "Missing character: There is no 中 (U+D800) in font font.",
        "Missing character: There is no 中 (U+110000) in font font.",
        "Missing character: There is no 中 (U+4E2D) in font .",
        "Missing character: There is no ^^@ in font nullfont!",
    ] {
        messages.push(warning(message));
    }
    for severity in [
        None,
        Some(DiagnosticSeverity::INFORMATION),
        Some(DiagnosticSeverity::HINT),
    ] {
        let mut item = valid.clone();
        item.severity = severity;
        messages.push(item);
    }
    let mut forged = warning("Some unrelated font warning.");
    forged.code = Some(NumberOrString::String("missing-character".into()));
    forged.data = Some(json!({"character":"中", "codepoint":"U+4E2D", "fonts":["phantom"]}));
    messages.push(forged);
    let result = compilation_analysis(&diagnostics(messages), false);
    assert_eq!(result["missingCharacters"], json!([]));
    assert_eq!(result["truncated"], false);
}

#[test]
fn missing_character_budgets_bound_output_and_still_update_known_characters() {
    let mut messages: Vec<_> = (0x4E00..0x4E80)
        .map(|code| {
            missing_glyph(
                char::from_u32(code).unwrap(),
                "font",
                DiagnosticSeverity::WARNING,
            )
        })
        .collect();
    let at_limit = compilation_analysis(&diagnostics(messages.clone()), false);
    assert_eq!(at_limit["missingCharacters"].as_array().unwrap().len(), 128);
    assert_eq!(at_limit["truncated"], false);
    messages.push(missing_glyph('中', "font", DiagnosticSeverity::WARNING));
    // 中 is already one of the 128 accepted characters.
    let duplicate = compilation_analysis(&diagnostics(messages.clone()), false);
    assert_eq!(duplicate["truncated"], false);
    messages.push(missing_glyph(
        '😀',
        "overflow-font",
        DiagnosticSeverity::ERROR,
    ));
    messages.push(missing_glyph(
        '中',
        "second-font",
        DiagnosticSeverity::ERROR,
    ));
    let result = compilation_analysis(&diagnostics(messages), false);
    let glyphs = result["missingCharacters"].as_array().unwrap();
    assert_eq!(glyphs.len(), 128);
    assert_eq!(result["truncated"], true);
    assert!(!glyphs.iter().any(|glyph| glyph["character"] == "😀"));
    assert_eq!(
        glyphs
            .iter()
            .find(|glyph| glyph["character"] == "中")
            .unwrap(),
        &json!({"character":"中", "codepoint":"U+4E2D", "severity":1, "fonts":["font", "second-font"]})
    );
    let characters: Vec<_> = glyphs
        .iter()
        .map(|glyph| glyph["character"].as_str().unwrap())
        .collect();
    let mut sorted = characters.clone();
    sorted.sort();
    assert_eq!(characters, sorted);
}

#[test]
fn missing_character_font_budgets_preserve_identity_and_signal_omissions() {
    let mut messages: Vec<_> = (0..8)
        .map(|n| missing_glyph('中', &format!("font-{n}"), DiagnosticSeverity::WARNING))
        .collect();
    messages.push(missing_glyph('中', "font-0", DiagnosticSeverity::WARNING));
    let at_limit = compilation_analysis(&diagnostics(messages.clone()), false);
    assert_eq!(at_limit["truncated"], false);
    assert_eq!(
        at_limit["missingCharacters"][0]["fonts"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    messages.push(missing_glyph(
        '中',
        "font-overflow",
        DiagnosticSeverity::ERROR,
    ));
    let result = compilation_analysis(&diagnostics(messages), false);
    assert_eq!(result["missingCharacters"][0]["severity"], 1);
    assert_eq!(
        result["missingCharacters"][0]["fonts"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(result["truncated"], true);
    let accepted = "😀".repeat(256);
    let oversize = "😀".repeat(257);
    let result = compilation_analysis(
        &diagnostics(vec![
            missing_glyph('中', &oversize, DiagnosticSeverity::ERROR),
            missing_glyph('文', &accepted, DiagnosticSeverity::WARNING),
        ]),
        false,
    );
    assert_eq!(
        result["missingCharacters"],
        json!([
            {"character":"中", "codepoint":"U+4E2D", "severity":1, "fonts":[]},
            {"character":"文", "codepoint":"U+6587", "severity":2, "fonts":[accepted]},
        ])
    );
    assert_eq!(result["truncated"], true);
}

#[test]
fn missing_character_fonts_remove_only_engine_terminators_and_preserve_unresolved_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "Main document.").unwrap();
    let log = "(./main.tex\nMissing character: There is no 中 (U+4e2d) in font dotted..\n./gone.tex:99: Missing character: There is no 中 (U+4E2D) in font bang!!\n)\n";
    let result = compilation_analysis(&parse(&root, log), true);
    assert_eq!(
        result["missingCharacters"],
        json!([
            {"character":"中", "codepoint":"U+4E2D", "severity":1, "fonts":["bang!", "dotted."]},
        ])
    );
    assert_eq!(result["truncated"], true);
}

#[test]
fn generated_source_line_notes_do_not_become_part_of_missing_font_descriptors() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    let auxiliary = directory.path().join("main.aux");
    std::fs::write(&root, "Main document.").unwrap();
    std::fs::write(&auxiliary, "").unwrap();
    let parsed = parse(
        &root,
        "./main.aux:99: Missing character: There is no 中 (U+4E2D) in font nullfont!\n",
    );
    let item = &parsed[&Url::from_file_path(&auxiliary).unwrap()][0];
    assert!(item
        .message
        .ends_with(" (reported line 99 is no longer present in the generated file)"));
    assert_eq!(
        compilation_analysis(&parsed, false)["missingCharacters"],
        json!([
            {"character":"中", "codepoint":"U+4E2D", "severity":1, "fonts":["nullfont"]},
        ])
    );
}

#[test]
fn missing_character_byte_wraps_preserve_complete_font_descriptions_and_file_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&root, "中\n").unwrap();
    std::fs::write(&child, "Child text.\n").unwrap();
    for width in [60, 79] {
        for prefix in ["", "! ", "./main.tex:1: "] {
            let font = format!("[{} Family Name]:mapping=tex-text;", "Font".repeat(20));
            let message = format!(
                "{prefix}{}",
                missing_glyph('中', &font, DiagnosticSeverity::WARNING).message
            );
            let wrapped = message
                .as_bytes()
                .chunks(width)
                .map(|bytes| std::str::from_utf8(bytes).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            let log = format!("(./main.tex\n{wrapped}\n\n(./child.tex\nLaTeX Warning: Child warning on input line 1.\n)\n)\n");
            let parsed = parse(&root, &log);
            let summary = compilation_analysis(&parsed, false);
            assert_eq!(
                summary["missingCharacters"],
                json!([
                    {"character":"中", "codepoint":"U+4E2D", "severity":if prefix.is_empty() { 2 } else { 1 }, "fonts":[font]},
                ]),
                "{wrapped}"
            );
            let parent = &parsed[&Url::from_file_path(&root).unwrap()];
            assert_eq!(parent.len(), 1);
            assert!(!parent[0].message.contains("child.tex"));
            let items = &parsed[&Url::from_file_path(&child).unwrap()];
            assert_eq!(items.len(), 1);
            assert!(items[0].message.contains("Child warning"));
        }
    }
}
