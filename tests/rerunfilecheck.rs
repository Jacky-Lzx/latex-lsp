use latex_lsp::{build_log, editing, workspace::Workspace};
use tower_lsp::lsp_types::*;

const CONFIRMED: &[&str] = &[
    "Package rerunfilecheck Warning: File `main.brf' has changed.\n\n(rerunfilecheck)                Rerun to get bibliographical\n references right.",
    "Package rerunfilecheck Warning: File `main.brf' has changed. Rerun to get bibliographical references right.",
    "Package rerunfilecheck Warning: File `main.brf' has changed.\r\n(rerunfilecheck) Rerun to get bibliographical references right.",
    "Package rerunfilecheck Warning: File `main.track' has changed. Rerun.",
    "Package rerunfilecheck Warning: File `main.track' has changed.\r\n(rerunfilecheck) Rerun.",
    "Package rerunfilecheck Warning: File `main.out' has changed.",
    "Package rerunfilecheck Warning: File `main.out' has changed. Rerun to get outlines right.",
    "Package rerunfilecheck Warning: File `main.out' has changed.\n(rerunfilecheck) Rerun to get outlines right\n(rerunfilecheck) or use package `bookmark'.",
];

const UNCONFIRMED: &[&str] = &[
    "Package rerunfilecheck Warning: File `main.idx' has changed.\n\n(rerunfilecheck) Rerun LaTeX/makeindex to get index right.",
    "Package rerunfilecheck Warning: File `main.glo' has changed.\n\n(rerunfilecheck) Rerun LaTeX/makeindex to get glossary right.",
    "Package rerunfilecheck Warning: File `main.brf' has changed.\n\n(rerunfilecheck) Unknown request.",
    "Package rerunfilecheck Warning: File `main.brf' has changed. Rerun to get bibliographical references right",
    "Package rerunfilecheck Warning: File `main.brf' has changed. Rerun to get bibliographical references right. Extra text.",
    "Package rerunfilecheck Warning: File `main.brf' has changed.\n(rerunfilecheck) Rerun to get bibliographical references right.\n(rerunfilecheck) Example only.",
    "Package rerunfilecheck Warning: File `main.brf' has changed. Example: Rerun to get bibliographical references right.",
    "Package backref Warning: File `main.brf' has changed. Rerun to get bibliographical references right.",
    "Package rerunfilecheck Warning: File `' has changed. Rerun.",
    "Package rerunfilecheck Warning: File `main.track' has changed. Rerun",
    "Package rerunfilecheck Warning: File `main.track' has changed. Rerun. Extra text.",
    "Package rerunfilecheck Warning: File `main.out' has changed. Rerun to get outlines right is only an example.",
    "Package rerunfilecheck Warning: File `main.out' has changed. (rerunfilecheck) Rerun to get outlines right or use another tool.",
    "Package rerunfilecheck Warning: File `main.out' has changed. Rerun to get outlines right or use package `bookmark'. Extra text.",
    "Package rerunfilecheck Warning: File `main.out' has changed.\n(rerunfilecheck) Example only.",
    "Package rerunfilecheck Warning: File `main.track' has changed. Rerun.\n(rerunfilecheck) Extra text.",
    "Package rerunfilecheck Warning: File `main.out' has changed. Rerun LaTeX/makeindex to get index right.",
    "Package rerunfilecheck Warning: File `main.out' has changed. Rerun LaTeX/makeindex to get glossary right.",
    "Package local Warning: File `main.track' has changed. Rerun.",
    "Class rerunfilecheck Warning: File `main.track' has changed. Rerun.",
];

fn actions(
    root: &Url,
    diagnostics: &std::collections::BTreeMap<Url, Vec<Diagnostic>>,
) -> CodeActionResponse {
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.to_file_path().unwrap().parent().unwrap().into());
    workspace
        .build_diagnostics
        .insert(root.clone(), diagnostics.clone());
    editing::code_actions(
        &workspace,
        root,
        Range::default(),
        &CodeActionContext {
            diagnostics: vec![],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
}

#[test]
fn file_change_requests_agree_in_analysis_and_rebuild_actions() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}Text.\\end{document}\n",
    )
    .unwrap();
    let root = Url::from_file_path(&main).unwrap();
    for (messages, expected) in [(CONFIRMED, true), (UNCONFIRMED, false)] {
        for message in messages {
            let diagnostics = build_log::parse(&main, &format!("(./main.tex\n{message}\n)\n"));
            assert_eq!(diagnostics.values().flatten().count(), 1, "{message}");
            assert_eq!(
                build_log::compilation_analysis(&diagnostics, false)["rerunRequired"],
                expected,
                "{message}"
            );
            let rebuilds = actions(&root, &diagnostics);
            assert_eq!(
                rebuilds.len(),
                usize::from(expected),
                "{message}: {rebuilds:?}"
            );
            if expected {
                let CodeActionOrCommand::CodeAction(action) = &rebuilds[0] else {
                    panic!("{rebuilds:?}")
                };
                assert_eq!(action.command.as_ref().unwrap().command, "latex.rebuild");
                let mut errors = diagnostics.clone();
                for diagnostic in errors.values_mut().flatten() {
                    diagnostic.severity = Some(DiagnosticSeverity::ERROR);
                }
                assert_eq!(
                    build_log::compilation_analysis(&errors, false)["rerunRequired"],
                    false
                );
                assert!(actions(&root, &errors).is_empty());
            }
        }
    }
}

#[test]
fn byte_wrapped_file_change_requests_preserve_names_and_adjacent_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n").unwrap();
    for width in [60, 79] {
        for file in [
            "main.track".to_owned(),
            "prefix' has changed. still.track".into(),
            format!("{}.track", "long-name-".repeat(20)),
        ] {
            for request in [
                " Rerun.",
                "\n(rerunfilecheck) Rerun to get bibliographical references right.",
                "\n(rerunfilecheck) Rerun to get outlines right\n(rerunfilecheck) or use package `bookmark'.",
            ] {
                let message = format!("Package rerunfilecheck Warning: File `{file}' has changed.{request}");
                let wrapped = message.lines().map(|line| {
                    line.as_bytes().chunks(width).map(|chunk| std::str::from_utf8(chunk).unwrap()).collect::<Vec<_>>().join("\n")
                }).collect::<Vec<_>>().join("\n");
                let diagnostics = build_log::parse(&main, &format!("(./main.tex\n{wrapped}\nLaTeX Warning: There were undefined references.\n)\n"));
                let warnings = diagnostics.values().flatten().collect::<Vec<_>>();
                assert_eq!(warnings.len(), 2, "{width}: {diagnostics:?}");
                assert!(warnings.iter().any(|item| item.message.contains(&format!("File `{file}' has changed."))), "{width}: {diagnostics:?}");
                assert!(warnings.iter().any(|item| item.message == "LaTeX Warning: There were undefined references."), "{width}: {diagnostics:?}");
                assert_eq!(build_log::compilation_analysis(&diagnostics, false)["rerunRequired"], true, "{width}: {diagnostics:?}");
            }
        }
    }
}

#[test]
fn parenthesized_filename_wraps_do_not_change_the_diagnostic_source() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("section.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    std::fs::write(&child, "Child text.\n").unwrap();
    std::fs::write(directory.path().join("shadow.tex"), "Healthy text.\n").unwrap();
    let main_uri = Url::from_file_path(&main).unwrap();
    let child_uri = Url::from_file_path(&child).unwrap();
    let prefix = "Package rerunfilecheck Warning: File `";
    for width in [60, 79] {
        for (embedded, suffix) in [
            ("", "(shadow.tex).track"),
            ("", ")tail.track"),
            ("", "(missing.tex.track"),
            ("' has changed. Rerun.", "(shadow.tex).track"),
        ] {
            let filename = format!(
                "{}{embedded}{suffix}",
                "a".repeat(width - prefix.len() - embedded.len())
            );
            for request in ["Rerun.", "Rerun to get bibliographical references right."] {
                let message = format!("{prefix}{filename}' has changed. {request}");
                let wrapped = message
                    .as_bytes()
                    .chunks(width)
                    .map(|chunk| std::str::from_utf8(chunk).unwrap())
                    .collect::<Vec<_>>()
                    .join("\n");
                let parsed = build_log::parse(&main, &format!("(./main.tex\n(./section.tex\n{wrapped}\n\nPackage local Warning: Child sentinel on input line 1.\n)\nPackage local Warning: Root sentinel on input line 1.\n)\n"));
                assert_eq!(parsed.len(), 2, "{width}/{filename}: {parsed:?}");
                let child_items = &parsed[&child_uri];
                assert_eq!(child_items.len(), 2, "{width}/{filename}: {parsed:?}");
                assert!(
                    child_items.iter().any(|item| item.message == message),
                    "{width}/{filename}: {parsed:?}"
                );
                assert!(
                    child_items
                        .iter()
                        .any(|item| item.message.contains("Child sentinel")),
                    "{width}/{filename}: {parsed:?}"
                );
                assert_eq!(parsed[&main_uri].len(), 1, "{width}/{filename}: {parsed:?}");
                assert!(
                    parsed[&main_uri][0].message.contains("Root sentinel"),
                    "{width}/{filename}: {parsed:?}"
                );
                assert_eq!(
                    build_log::compilation_analysis(&parsed, false)["rerunRequired"],
                    true,
                    "{width}/{filename}: {parsed:?}"
                );
            }
        }
    }
}

#[test]
fn incomplete_file_change_warnings_keep_real_file_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("section.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    std::fs::write(&child, "Child text.\n").unwrap();
    let main_uri = Url::from_file_path(&main).unwrap();
    let child_uri = Url::from_file_path(&child).unwrap();
    let prefix = "Package rerunfilecheck Warning: File `";
    for width in [60, 79] {
        let incomplete = format!("{prefix}{}", "a".repeat(width - prefix.len()));
        let parsed = build_log::parse(&main, &format!("(./main.tex\n{incomplete}\n(./section.tex\nPackage local Warning: Child sentinel on input line 1.\n)\nPackage local Warning: Root sentinel on input line 1.\n)\n"));
        assert_eq!(parsed.len(), 2, "{width}: {parsed:?}");
        assert_eq!(parsed[&child_uri].len(), 1, "{width}: {parsed:?}");
        assert!(parsed[&child_uri][0].message.contains("Child sentinel"));
        assert_eq!(parsed[&main_uri].len(), 2, "{width}: {parsed:?}");
        assert!(parsed[&main_uri]
            .iter()
            .any(|item| item.message == incomplete));
        assert!(parsed[&main_uri]
            .iter()
            .any(|item| item.message.contains("Root sentinel")));
        assert_eq!(
            build_log::compilation_analysis(&parsed, false)["rerunRequired"],
            false
        );
        assert!(actions(&main_uri, &parsed).is_empty());
    }
}

#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX, LuaLaTeX and rerunfilecheck"]
fn real_parenthesized_file_changes_preserve_following_source_contexts() {
    let cache = tempfile::tempdir().unwrap();
    let prefix = "Package rerunfilecheck Warning: File `";
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for (embedded, suffix) in [
                ("", "(shadow.tex).track"),
                ("", ")tail.track"),
                ("", "(missing.tex.track"),
                ("' has changed. Rerun.", "(shadow.tex).track"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let filename = format!(
                    "{}{embedded}{suffix}",
                    "a".repeat(width - prefix.len() - embedded.len())
                );
                let source = format!("\\documentclass{{article}}\n\\usepackage{{rerunfilecheck}}\n\\RerunFileCheck{{{filename}}}{{}}{{}}\n\\AddToHook{{enddocument/info}}{{\\input{{section}}\\PackageWarningNoLine{{local}}{{Root sentinel}}}}\n\\begin{{document}}\n\\newwrite\\trackfile\n\\immediate\\openout\\trackfile=\"{filename}\"\n\\immediate\\write\\trackfile{{Changed content}}\n\\immediate\\closeout\\trackfile\nText.\n\\end{{document}}\n");
                std::fs::write(&main, source).unwrap();
                std::fs::write(
                    directory.path().join("section.tex"),
                    "\\PackageWarningNoLine{local}{Child sentinel}\n",
                )
                .unwrap();
                std::fs::write(directory.path().join("shadow.tex"), "Healthy text.\n").unwrap();
                let root = Url::from_file_path(&main).unwrap();
                let child = Url::from_file_path(directory.path().join("section.tex")).unwrap();
                for pass in 1..=2 {
                    let output = std::process::Command::new(engine)
                        .current_dir(directory.path())
                        .env("TEXMFVAR", cache.path())
                        .env("TEXMFCACHE", cache.path())
                        .env("max_print_line", width.to_string())
                        .args([
                            "-interaction=nonstopmode",
                            "-halt-on-error",
                            "-no-shell-escape",
                            "main.tex",
                        ])
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{engine}/{width}/{suffix}/{pass}: {}",
                        String::from_utf8_lossy(&output.stdout)
                    );
                    let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
                    let parsed = build_log::parse(&main, &log);
                    assert_eq!(
                        parsed.len(),
                        2,
                        "{engine}/{width}/{suffix}/{pass}: {parsed:?}"
                    );
                    assert_eq!(
                        parsed[&child].len(),
                        1,
                        "{engine}/{width}/{suffix}/{pass}: {parsed:?}"
                    );
                    assert_eq!(
                        parsed[&child][0].message,
                        "Package local Warning: Child sentinel."
                    );
                    assert!(
                        parsed[&root]
                            .iter()
                            .any(|item| item.message == "Package local Warning: Root sentinel."),
                        "{engine}/{width}/{suffix}/{pass}: {parsed:?}"
                    );
                    let warning = parsed[&root]
                        .iter()
                        .find(|item| item.message.starts_with(prefix));
                    assert_eq!(
                        warning.is_some(),
                        pass == 1,
                        "{engine}/{width}/{suffix}/{pass}: {parsed:?}"
                    );
                    if let Some(warning) = warning {
                        assert_eq!(
                            warning.message,
                            format!("{prefix}{filename}' has changed. Rerun.")
                        );
                    }
                    assert_eq!(
                        build_log::compilation_analysis(&parsed, false)["rerunRequired"],
                        pass == 1,
                        "{engine}/{width}/{suffix}/{pass}: {parsed:?}"
                    );
                    assert_eq!(
                        actions(&root, &parsed).len(),
                        usize::from(pass == 1),
                        "{engine}/{width}/{suffix}/{pass}: {parsed:?}"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX, LuaLaTeX, rerunfilecheck and hyperref"]
fn real_file_changes_request_rebuild_until_the_next_unchanged_pass() {
    let cache = tempfile::tempdir().unwrap();
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for outlines in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let setup = if outlines {
                    "\\usepackage{hyperref}\n"
                } else {
                    "\\usepackage{rerunfilecheck}\n\\RerunFileCheck{main.track}{}{}\n"
                };
                let body = if outlines {
                    "\\section{Title}Text.\n"
                } else {
                    "\\newwrite\\trackfile\n\\immediate\\openout\\trackfile=main.track\n\\immediate\\write\\trackfile{Changed content}\n\\immediate\\closeout\\trackfile\nText.\n"
                };
                std::fs::write(&main, format!("\\documentclass{{article}}\n{setup}\\begin{{document}}\n{body}\\end{{document}}\n")).unwrap();
                let root = Url::from_file_path(&main).unwrap();
                for pass in 1..=2 {
                    let output = std::process::Command::new(engine)
                        .current_dir(directory.path())
                        .env("TEXMFVAR", cache.path())
                        .env("TEXMFCACHE", cache.path())
                        .env("max_print_line", width.to_string())
                        .args([
                            "-interaction=nonstopmode",
                            "-halt-on-error",
                            "-no-shell-escape",
                            "main.tex",
                        ])
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{engine}/{width}/{outlines}/{pass}: {}",
                        String::from_utf8_lossy(&output.stdout)
                    );
                    let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
                    let diagnostics = build_log::parse(&main, &log);
                    let changed = diagnostics.values().flatten().any(|diagnostic| {
                        diagnostic
                            .message
                            .starts_with("Package rerunfilecheck Warning:")
                    });
                    assert_eq!(
                        changed,
                        pass == 1,
                        "{engine}/{width}/{outlines}/{pass}: {diagnostics:?}"
                    );
                    assert_eq!(
                        build_log::compilation_analysis(&diagnostics, false)["rerunRequired"],
                        pass == 1,
                        "{engine}/{width}/{outlines}/{pass}: {diagnostics:?}"
                    );
                    assert_eq!(
                        actions(&root, &diagnostics).len(),
                        usize::from(pass == 1),
                        "{engine}/{width}/{outlines}/{pass}: {diagnostics:?}"
                    );
                }
            }
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX, LuaLaTeX and hyperref"]
fn real_backreferences_request_rebuild_after_citation_pages_change_and_clear_when_stable() {
    use latex_lsp::{build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    let cache = tempfile::tempdir().unwrap();
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let root = Url::from_file_path(&main).unwrap();
            let wrapper = directory.path().join("engine");
            std::fs::write(&wrapper, format!("#!/bin/sh\nexport max_print_line={width}\nexport TEXMFVAR=\"$1\"\nexport TEXMFCACHE=\"$1\"\nshift\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = BuildConfig {
                executable: wrapper.to_string_lossy().into_owned(),
                output_directory: ".".into(),
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
            for (stage, pagebreak) in [(1, ""), (2, r"\newpage")] {
                let source = format!("\\documentclass{{article}}\n\\usepackage[backref=page]{{hyperref}}\n\\begin{{document}}\nIntro.{pagebreak}\nText \\cite{{book}}.\n\\begin{{thebibliography}}{{1}}\n\\bibitem{{book}} Author. Book.\n\n\\end{{thebibliography}}\n\\end{{document}}\n");
                std::fs::write(&main, source).unwrap();
                for pass in 1..=3 {
                    let outcome = build::run(&main, &config, &Default::default());
                    assert_eq!(
                        outcome.status, "success",
                        "{engine}/{width}/{stage}/{pass}: {}",
                        outcome.output
                    );
                    assert_eq!(outcome.pages, Some(stage));
                    let backreferences = outcome
                        .diagnostics
                        .iter()
                        .filter_map(|(uri, items)| {
                            let warnings = items
                                .iter()
                                .filter(|item| {
                                    item.message.starts_with("Package rerunfilecheck Warning:")
                                        && item.message.contains("bibliographical references")
                                })
                                .cloned()
                                .collect::<Vec<_>>();
                            (!warnings.is_empty()).then_some((uri.clone(), warnings))
                        })
                        .collect::<std::collections::BTreeMap<_, _>>();
                    assert_eq!(
                        !backreferences.is_empty(),
                        pass == 1,
                        "{engine}/{width}/{stage}/{pass}: {:?}",
                        outcome.diagnostics
                    );
                    let analysis = build_log::compilation_analysis(&backreferences, false);
                    assert_eq!(analysis["rerunRequired"], pass == 1);
                    assert_eq!(analysis["requiredBibliographyTools"], serde_json::json!([]));
                    let rebuilds = actions(&root, &backreferences);
                    assert_eq!(
                        rebuilds.len(),
                        usize::from(pass == 1),
                        "{engine}/{width}/{stage}/{pass}: {rebuilds:?}"
                    );
                    if pass == 1 {
                        assert_eq!(outcome.analysis.as_ref().unwrap()["rerunRequired"], true);
                    } else if pass == 3 {
                        assert_eq!(
                            outcome.analysis.as_ref().unwrap()["rerunRequired"],
                            false,
                            "{engine}/{width}/{stage}/{pass}: {:?}",
                            outcome.diagnostics
                        );
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX, LuaLaTeX and rerunfilecheck"]
fn real_index_and_glossary_requests_do_not_offer_a_tex_only_rebuild() {
    let cache = tempfile::tempdir().unwrap();
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let root = Url::from_file_path(&main).unwrap();
            std::fs::write(&main, "\\documentclass{article}\n\\usepackage[index,glossary]{rerunfilecheck}\n\\makeindex\\makeglossary\n\\begin{document}Text.\\index{Alpha}\\glossary{Gamma}\\end{document}\n").unwrap();
            for pass in 1..=2 {
                let output = std::process::Command::new(engine)
                    .current_dir(directory.path())
                    .env("TEXMFVAR", cache.path())
                    .env("TEXMFCACHE", cache.path())
                    .env("max_print_line", width.to_string())
                    .args([
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "-no-shell-escape",
                        "main.tex",
                    ])
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{engine}/{width}/{pass}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
                let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
                let diagnostics = build_log::parse(&main, &log);
                let requests = diagnostics
                    .values()
                    .flatten()
                    .filter(|item| item.message.starts_with("Package rerunfilecheck Warning:"))
                    .collect::<Vec<_>>();
                assert_eq!(
                    requests.len(),
                    if pass == 1 { 2 } else { 0 },
                    "{engine}/{width}/{pass}: {diagnostics:?}"
                );
                if pass == 1 {
                    for kind in ["index", "glossary"] {
                        assert!(
                            requests.iter().any(|item| item
                                .message
                                .contains(&format!("Rerun LaTeX/makeindex to get {kind} right."))),
                            "{engine}/{width}/{pass}: {requests:?}"
                        );
                    }
                }
                assert_eq!(
                    build_log::compilation_analysis(&diagnostics, false)["rerunRequired"],
                    false,
                    "{engine}/{width}/{pass}: {diagnostics:?}"
                );
                assert!(
                    actions(&root, &diagnostics).is_empty(),
                    "{engine}/{width}/{pass}"
                );
            }
        }
    }
}
