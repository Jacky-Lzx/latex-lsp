use latex_lsp::{compilation::build_log, editing, project::workspace::Workspace};
use lsp_types::*;
use std::collections::BTreeMap;

const REQUESTS: &[&str] = &[
    "LaTeX Warning: Label(s) may have changed. Rerun to get cross-references right.",
    "Package natbib Warning: Citation(s) may have changed. Rerun to get citations correct.",
    "Package natbib Warning: Citation(s) may have changed.",
    "Package longtable Warning: Table widths have changed. Rerun LaTeX.",
    "Package hyperref Warning: Rerun to get /PageLabels entry.",
    "LaTeX Warning: Temporary extra page added at the end. Rerun to get it removed.",
    "LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (1 not 3). Rerun to correct this.",
    "LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (123456 not 345678). Rerun to correct this.",
];

fn actions(root: &Url, diagnostics: &BTreeMap<Url, Vec<Diagnostic>>) -> CodeActionResponse {
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.to_file_path().unwrap().parent().unwrap().into());
    workspace
        .project
        .build_diagnostics
        .insert(root.clone(), diagnostics.clone());
    // natbib reports its final check while rereading the generated .aux.
    // Request actions at the published source, as an editor would.
    diagnostics
        .keys()
        .flat_map(|uri| {
            workspace.reload(uri);
            editing::code_actions(
                &workspace,
                uri,
                Range::default(),
                &CodeActionContext {
                    diagnostics: vec![],
                    only: Some(vec![CodeActionKind::QUICKFIX]),
                    trigger_kind: None,
                },
            )
        })
        .collect()
}

fn wrapped(message: &str, width: usize, newline: &str) -> String {
    message
        .as_bytes()
        .chunks(width)
        .map(|bytes| std::str::from_utf8(bytes).unwrap())
        .collect::<Vec<_>>()
        .join(newline)
}

#[test]
fn hard_wrapped_standard_requests_keep_words_and_agree_in_analysis_and_actions() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    let root = Url::from_file_path(&main).unwrap();
    for width in 60..=82 {
        for newline in ["\n", "\r\n"] {
            for message in REQUESTS {
                let log = format!("(./main.tex{newline}{}{newline}{newline}Package local Warning: Sentinel.{newline}){newline}", wrapped(message, width, newline));
                let parsed = build_log::parse(&main, &log);
                assert_eq!(parsed[&root].len(), 2, "{width}: {parsed:?}");
                assert!(
                    parsed[&root].iter().any(|item| item.message == *message),
                    "{width}: {parsed:?}"
                );
                assert_eq!(
                    build_log::compilation_analysis(&parsed, false)["rerunRequired"],
                    true,
                    "{width}: {parsed:?}"
                );
                assert_eq!(actions(&root, &parsed).len(), 1, "{width}: {parsed:?}");
            }
        }
    }
}

#[test]
fn semantic_continuations_after_a_sentence_keep_the_entire_rerun_request() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    let root = Url::from_file_path(&main).unwrap();
    for (message, expected, rerun) in [
        ("LaTeX Warning: Label(s) may have changed.\n Rerun to get cross-references right.", REQUESTS[0], true),
        ("LaTeX Warning: Temporary extra page added at the end.\n               Rerun to get it removed.", REQUESTS[5], true),
        ("LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (1 not 3).\n               Rerun to correct this.", REQUESTS[6], true),
        ("LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (1 not 3).\nRerun to correct this.", REQUESTS[6], true),
        ("Package natbib Warning: Citation(s) may have changed.\n(natbib) Rerun to get citations correct.", "Package natbib Warning: Citation(s) may have changed. (natbib) Rerun to get citations correct.", true),
        ("Package longtable Warning: Table widths have changed.\n(longtable) Rerun LaTeX.", "Package longtable Warning: Table widths have changed. (longtable) Rerun LaTeX.", true),
        ("Package natbib Warning: Citation(s) may have changed.\n(natbib) This is only an example.", "Package natbib Warning: Citation(s) may have changed. (natbib) This is only an example.", false),
        ("Package natbib Warning: Citation(s) may have changed.\n(natbib) Rerun to get citations correct.\n(natbib) Example only.", "Package natbib Warning: Citation(s) may have changed. (natbib) Rerun to get citations correct. (natbib) Example only.", false),
    ] {
        let parsed = build_log::parse(&main, &format!("(./main.tex\n{message}\n\n)\n"));
        assert_eq!(parsed[&root].len(), 1, "{parsed:?}");
        assert_eq!(parsed[&root][0].message, expected);
        assert_eq!(build_log::compilation_analysis(&parsed, false)["rerunRequired"], rerun, "{parsed:?}");
        assert_eq!(actions(&root, &parsed).len(), usize::from(rerun), "{parsed:?}");
    }
}

#[test]
fn partial_requests_and_extra_prose_do_not_borrow_adjacent_file_errors_or_other_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    std::fs::write(&child, "\\unknown\n").unwrap();
    let root = Url::from_file_path(&main).unwrap();
    let child = Url::from_file_path(&child).unwrap();
    for width in [60, 79] {
        for message in REQUESTS {
            let invalid = format!("{message} Extra text.");
            let log = format!("(./main.tex\n{}\n\n(./child.tex\n./child.tex:1: Undefined control sequence.\nl.1 \\unknown\n)\nPackage local Warning: Sentinel.\n)\n", wrapped(&invalid, width, "\n"));
            let parsed = build_log::parse(&main, &log);
            assert_eq!(parsed[&root].len(), 2, "{width}: {parsed:?}");
            assert_eq!(parsed[&child].len(), 1, "{width}: {parsed:?}");
            assert_eq!(parsed[&child][0].severity, Some(DiagnosticSeverity::ERROR));
            assert_eq!(
                build_log::compilation_analysis(&parsed, false)["rerunRequired"],
                false,
                "{width}: {parsed:?}"
            );
            assert!(actions(&root, &parsed).is_empty());
        }
    }
    let partial = &REQUESTS[0][..60];
    for boundary in [
        "(./child.tex\n./child.tex:1: Undefined control sequence.\nl.1 \\unknown\n)",
        "[latex-lsp: middle of compiler output omitted]",
    ] {
        let parsed = build_log::parse(
            &main,
            &format!("(./main.tex\n{partial}\n{boundary}\nPackage local Warning: Sentinel.\n)\n"),
        );
        assert_eq!(parsed[&root].len(), 2, "{parsed:?}");
        assert!(parsed[&root].iter().any(|item| item.message == partial));
        assert_eq!(
            build_log::compilation_analysis(&parsed, false)["rerunRequired"],
            false
        );
    }
}

#[test]
fn last_page_requests_require_complete_kernel_messages_and_distinct_page_counts() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    let root = Url::from_file_path(&main).unwrap();
    let hook = "LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page";
    for (message, expected) in [
        (REQUESTS[5].to_owned(), true),
        (format!("{hook} (12 not 345). Rerun to correct this."), true),
        (format!("{hook} (345 not 12). Rerun to correct this."), true),
        (format!("{hook} (1 not 3)."), false),
        (format!("{hook} (1 not 3). Rerun to correct this"), false),
        (format!("{hook} (1 not 3). Rerun to correct this. Example only."), false),
        (format!("{hook} (1 not 3).\n               Example: Rerun to correct this."), false),
        (format!("{hook} (1 not 3).\n               Rerun to correct this.\n               Example only."), false),
        (REQUESTS[6].replace("(1 not 3)", "(3 not 3)"), false),
        (REQUESTS[6].replace("(1 not 3)", "(0 not 3)"), false),
        (REQUESTS[6].replace("(1 not 3)", "(1 not 0)"), false),
        (REQUESTS[6].replace("(1 not 3)", "(-1 not 3)"), false),
        (REQUESTS[6].replace("(1 not 3)", "(+1 not 3)"), false),
        (REQUESTS[6].replace("(1 not 3)", "(one not three)"), false),
        (REQUESTS[6].replace("(1 not 3)", "(1 not 3 not 4)"), false),
        (REQUESTS[6].replace("(1 not 3)", "(1 not 4294967296)"), false),
        (REQUESTS[6].replace("shipout/lastpage", "shipout/firstpage"), false),
        (REQUESTS[6].replace("LaTeX Warning:", "Package local Warning:"), false),
        (REQUESTS[5].replace("LaTeX Warning:", "Package local Warning:"), false),
        (REQUESTS[5].replace("LaTeX Warning:", "LaTeX Warning: Example:"), false),
        ("LaTeX Warning: Temporary extra page added at the end.".into(), false),
        (format!("{}\n               Example only.", REQUESTS[5]), false),
    ] {
        let parsed = build_log::parse(&main, &format!("(./main.tex\n{message}\n\n)\n"));
        assert_eq!(parsed[&root].len(), 1, "{message}: {parsed:?}");
        assert_eq!(build_log::compilation_analysis(&parsed, false)["rerunRequired"], expected, "{message}: {parsed:?}");
        assert_eq!(actions(&root, &parsed).len(), usize::from(expected), "{message}: {parsed:?}");
        let mut errors = parsed;
        for diagnostic in errors.values_mut().flatten() {
            diagnostic.severity = Some(DiagnosticSeverity::ERROR);
        }
        assert_eq!(build_log::compilation_analysis(&errors, false)["rerunRequired"], false);
        assert!(actions(&root, &errors).is_empty());
    }
}

#[test]
fn incomplete_last_page_requests_preserve_following_source_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child_path = directory.path().join("child.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    std::fs::write(&child_path, "\\unknown\n").unwrap();
    let root = Url::from_file_path(&main).unwrap();
    let child = Url::from_file_path(&child_path).unwrap();
    for partial in [
        "LaTeX Warning: Temporary extra page added at the end.",
        "LaTeX Warning: Temporary extra page added at the end. Rerun ",
        "LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (1 not 3).",
        "LaTeX Warning: Hook 'shipout/lastpage' executed on wrong pag",
    ] {
        let parsed = build_log::parse(&main, &format!("(./main.tex\n{partial}\n(./child.tex\n./child.tex:1: Undefined control sequence.\nl.1 \\unknown\n)\nPackage local Warning: Root sentinel.\n)\n"));
        assert_eq!(parsed[&root].len(), 2, "{partial}: {parsed:?}");
        assert_eq!(parsed[&child].len(), 1, "{partial}: {parsed:?}");
        assert!(
            parsed[&root].iter().any(|item| item.message == partial),
            "{parsed:?}"
        );
        assert_eq!(parsed[&child][0].severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(
            build_log::compilation_analysis(&parsed, false)["rerunRequired"],
            false
        );
        assert!(actions(&root, &parsed).is_empty());
    }
    for partial in [
        "LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (1 not 3",
        "LaTeX Warning: Hook 'shipout/lastpage' executed on wrong page (1 not 3).",
    ] {
        let parsed = build_log::parse(&main, &format!("(./main.tex\n(./child.tex\n{partial}\n)\nPackage local Warning: Root sentinel.\n)\n"));
        assert_eq!(parsed[&root].len(), 1, "{partial}: {parsed:?}");
        assert_eq!(
            parsed[&root][0].message,
            "Package local Warning: Root sentinel."
        );
        assert_eq!(parsed[&child].len(), 1, "{partial}: {parsed:?}");
        assert_eq!(parsed[&child][0].message, partial);
        assert_eq!(
            build_log::compilation_analysis(&parsed, false)["rerunRequired"],
            false
        );
        assert!(actions(&root, &parsed).is_empty());
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX and LuaLaTeX"]
fn real_last_page_requests_follow_shrinking_and_growing_documents_until_stable() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    let cache = tempfile::tempdir().unwrap();
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 70, 79] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let child = directory.path().join("child.tex");
            let root = Url::from_file_path(&main).unwrap();
            std::fs::write(&main, "\\documentclass{article}\n\\AddToHook{shipout/lastpage}{\\typeout{LAST-HOOK-PAGE: \\the\\ReadonlyShipoutCounter}\\put(20,-30){Last page marker}}\n\\begin{document}\n\\input{child}\n\\end{document}\n").unwrap();
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
            for (stage, source_pages, pdf_pages, hook_page, request) in [
                ("initial", 3, 3, 3, None),
                ("shrink", 1, 2, 1, Some(REQUESTS[5])),
                ("shrink-stable", 1, 1, 1, None),
                ("grow", 3, 3, 1, Some(REQUESTS[6])),
                ("grow-stable", 3, 3, 3, None),
            ] {
                let body = (1..=source_pages)
                    .map(|page| format!("Page {page}.\n"))
                    .collect::<Vec<_>>()
                    .join("\\newpage\n");
                std::fs::write(&child, body).unwrap();
                let outcome = build::run(&main, &config, &Default::default());
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{width}/{stage}: {}",
                    outcome.output
                );
                assert_eq!(
                    outcome.pages,
                    Some(pdf_pages),
                    "{engine}/{width}/{stage}: {}",
                    outcome.output
                );
                assert!(outcome.pdf_updated);
                assert!(outcome.pdf.as_ref().unwrap().is_file());
                let analysis = outcome.analysis.as_ref().unwrap();
                assert_eq!(
                    analysis["rerunRequired"],
                    request.is_some(),
                    "{engine}/{width}/{stage}: {:?}",
                    outcome.diagnostics
                );
                assert_eq!(analysis["requiredBibliographyTools"], serde_json::json!([]));
                assert_eq!(analysis["requiredIndexTools"], serde_json::json!([]));
                let last_page_warnings = outcome
                    .diagnostics
                    .values()
                    .flatten()
                    .filter(|item| {
                        item.message
                            .starts_with("LaTeX Warning: Temporary extra page")
                            || item
                                .message
                                .starts_with("LaTeX Warning: Hook 'shipout/lastpage'")
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    last_page_warnings.len(),
                    usize::from(request.is_some()),
                    "{engine}/{width}/{stage}: {:?}",
                    outcome.diagnostics
                );
                if let Some(request) = request {
                    assert_eq!(
                        last_page_warnings[0].message, request,
                        "{engine}/{width}/{stage}"
                    );
                    assert!(outcome.diagnostics[&root]
                        .iter()
                        .any(|item| item.message == request));
                }
                assert_eq!(
                    actions(&root, &outcome.diagnostics).len(),
                    usize::from(request.is_some()),
                    "{engine}/{width}/{stage}: {:?}",
                    outcome.diagnostics
                );
                let log = std::fs::read_to_string(outcome.log.unwrap()).unwrap();
                assert!(
                    log.contains(&format!("LAST-HOOK-PAGE: {hook_page}")),
                    "{engine}/{width}/{stage}: {log}"
                );
            }
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX, LuaLaTeX, natbib and longtable"]
fn real_standard_requests_follow_cross_file_renames_and_table_changes_until_stable() {
    use latex_lsp::{compilation::build, config::BuildConfig, language::features};
    use std::os::unix::fs::PermissionsExt;

    let cache = tempfile::tempdir().unwrap();
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 63, 79] {
            for (kind, package, body, child, request, name) in [
                ("label", "", r"\ref{anchor}\input{child}", r"\section{Heading}\label{anchor}", REQUESTS[0], Some(("anchor", "renamed"))),
                ("citation", r"\usepackage[numbers]{natbib}", r"\citep{book}\input{child}", "\\begin{thebibliography}{1}\n\\bibitem{book} Author. Book.\n\\end{thebibliography}\n", REQUESTS[1], Some(("book", "volume"))),
                ("table", r"\usepackage{longtable}\setcounter{LTchunksize}{1}", r"\input{child}", "\\begin{longtable}{ll}\nShort & Two\\\\\nLonger sample & Other\\\\\n\\end{longtable}\n", REQUESTS[3], None),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let child_path = directory.path().join("child.tex");
                let root = Url::from_file_path(&main).unwrap();
                let source = format!("\\documentclass{{article}}\n{package}\n\\begin{{document}}\n{body}\n\\end{{document}}\n");
                std::fs::write(&main, source).unwrap();
                std::fs::write(&child_path, child).unwrap();
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
                for stage in 1..=2 {
                    for pass in 1..=3 {
                        let outcome = build::run(&main, &config, &Default::default());
                        assert_eq!(outcome.status, "success", "{engine}/{width}/{kind}/{stage}/{pass}: {}", outcome.output);
                        assert_eq!(outcome.pages, Some(1));
                        let requests = outcome.diagnostics.iter().filter_map(|(uri, items)| {
                            let items = items.iter().filter(|item| item.message.starts_with(request.split_once('.').unwrap().0)).cloned().collect::<Vec<_>>();
                            (!items.is_empty()).then_some((uri.clone(), items))
                        }).collect::<BTreeMap<_, _>>();
                        assert_eq!(!requests.is_empty(), pass == 1, "{engine}/{width}/{kind}/{stage}/{pass}: {:?}", outcome.diagnostics);
                        assert_eq!(build_log::compilation_analysis(&requests, false)["rerunRequired"], pass == 1, "{engine}/{width}/{kind}/{stage}/{pass}: {requests:?}");
                        assert_eq!(actions(&root, &requests).len(), usize::from(pass == 1), "{engine}/{width}/{kind}/{stage}/{pass}: {requests:?}");
                        if pass == 1 {
                            if kind == "citation" {
                                assert!(requests.values().flatten().any(|item| item.message.contains("Rerun to get citations correct.")), "{engine}/{width}: {requests:?}");
                            } else {
                                assert!(requests.values().flatten().any(|item| item.message == request), "{engine}/{width}: {requests:?}");
                            }
                        }
                        if pass == 1 || pass == 3 {
                            assert_eq!(outcome.analysis.as_ref().unwrap()["rerunRequired"], pass == 1, "{engine}/{width}/{kind}/{stage}/{pass}: {:?}", outcome.diagnostics);
                        }
                    }
                    if stage == 1 {
                        if let Some((old, new)) = name {
                            let mut workspace = Workspace::default();
                            workspace.add_root(directory.path().into());
                            let text = &workspace.documents[&root].text;
                            let edit = features::rename(&workspace, &root, text.position(text.source.find(old).unwrap() + 1), new).unwrap().unwrap();
                            let changes = edit.changes.unwrap();
                            assert_eq!(changes.len(), 2, "{kind}: {changes:?}");
                            for (uri, mut edits) in changes {
                                edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
                                let text = workspace.documents[&uri].text.changed(edits.into_iter().map(|edit| TextDocumentContentChangeEvent {
                                    range: Some(edit.range), range_length: None, text: edit.new_text,
                                }).collect()).unwrap();
                                std::fs::write(uri.to_file_path().unwrap(), text.source).unwrap();
                            }
                        } else {
                            std::fs::write(&child_path, child.replace("Longer sample", "A considerably wider column")).unwrap();
                        }
                    }
                }
            }
        }
    }
}
