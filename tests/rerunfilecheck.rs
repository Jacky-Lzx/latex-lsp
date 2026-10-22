use latex_lsp::{build_log, editing, workspace::Workspace};
use tower_lsp::lsp_types::*;

const CONFIRMED: &[&str] = &[
    "Package rerunfilecheck Warning: File `main.track' has changed. Rerun.",
    "Package rerunfilecheck Warning: File `main.track' has changed.\r\n(rerunfilecheck) Rerun.",
    "Package rerunfilecheck Warning: File `main.out' has changed.",
    "Package rerunfilecheck Warning: File `main.out' has changed. Rerun to get outlines right.",
    "Package rerunfilecheck Warning: File `main.out' has changed.\n(rerunfilecheck) Rerun to get outlines right\n(rerunfilecheck) or use package `bookmark'.",
];

const UNCONFIRMED: &[&str] = &[
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
