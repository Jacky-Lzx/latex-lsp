use lsp_types::*;
use texquill::{compilation::build_log, editing, project::workspace::Workspace, text::Text};

const HELP: &str = "\nSee the etoolbox package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\n";

fn log(name: &str, line: usize, echo: &str, numbered: bool) -> String {
    let header = if numbered {
        format!("./main.tex:{line}:")
    } else {
        "!".into()
    };
    format!("(./main.tex\n{header} Package etoolbox Error: Toggle '{name}' undefined.\n{HELP}l.{line} {echo}\n)\n")
}

fn fixture(declarations: &str, call: &str) -> (tempfile::TempDir, Workspace, Url, Diagnostic) {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = format!("\\input{{definitions}}\n{call}\n");
    std::fs::write(&main, &source).unwrap();
    std::fs::write(directory.path().join("definitions.tex"), declarations).unwrap();
    let diagnostics = build_log::parse(&main, &log("darft", 2, call, true));
    let uri = Url::from_file_path(&main).unwrap();
    let diagnostic = diagnostics[&uri][0].clone();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(directory.path().into());
    workspace.open(uri.clone(), source, 9);
    workspace
        .project
        .build_diagnostics
        .insert(uri.clone(), diagnostics);
    (directory, workspace, uri, diagnostic)
}

fn actions(workspace: &Workspace, uri: &Url, diagnostic: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
}

#[test]
fn direct_toggle_errors_select_names_with_the_actual_consumed_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for (call, echo) in [
        (r"\settoggle{darft}{true}", r"\settoggle{darft}{true}"),
        (r"\toggletrue{darft}", r"\toggletrue{darft}"),
        (r"\togglefalse{darft}", r"\togglefalse{darft}"),
        (r"\iftoggle{darft}{Yes}{No}", r"\iftoggle{darft}"),
        (r"\nottoggle{darft}{No}{Yes}", r"\nottoggle{darft}"),
        ("\\global\\settoggle % keep\r\n{darft}\r\n{true}", "{true}"),
    ] {
        let source = format!("\\documentclass{{article}}\n😀 {call}\n");
        std::fs::write(&main, &source).unwrap();
        let line = source.lines().count();
        for numbered in [false, true] {
            let echo = if call.contains('\n') {
                echo.to_owned()
            } else {
                format!("😀 {echo}")
            };
            let diagnostics = build_log::parse(&main, &log("darft", line, &echo, numbered));
            let diagnostic = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
            let start = source.find("darft").unwrap();
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + 5),
                "{call}: {diagnostic:?}"
            );
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("undefined-toggle".into()))
            );
            assert_eq!(diagnostic.data.as_ref().unwrap()["toggle"], "darft");
        }
    }
}

#[test]
fn toggle_repairs_use_their_own_namespace_and_preserve_values_and_branches() {
    for (call, replacement) in [
        (r"\settoggle{darft}{true}", r"\settoggle{draft}{true}"),
        (r"\toggletrue{darft}", r"\toggletrue{draft}"),
        (r"\togglefalse{darft}", r"\togglefalse{draft}"),
        (r"\iftoggle{darft}", r"\iftoggle{draft}"),
        (r"\nottoggle{darft}", r"\nottoggle{draft}"),
    ] {
        let (_directory, workspace, uri, diagnostic) = fixture(r"\newtoggle{draft}", call);
        let edit = actions(&workspace, &uri, &diagnostic)
            .into_iter()
            .find_map(|item| match item {
                CodeActionOrCommand::CodeAction(action)
                    if action.title == "Replace with toggle `draft`" =>
                {
                    action.edit
                }
                _ => None,
            })
            .expect("missing toggle spelling repair");
        let changes = edit.changes.unwrap();
        let edits = &changes[&uri];
        assert_eq!(edits.len(), 1);
        let result = workspace.documents[&uri]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edits[0].range),
                range_length: None,
                text: edits[0].new_text.clone(),
            }])
            .unwrap();
        assert_eq!(
            result.source,
            format!("\\input{{definitions}}\n{replacement}\n")
        );
    }
}

#[test]
fn uncertain_toggle_calls_and_malformed_contexts_keep_line_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for call in [
        r"\settoggle{darft}",
        r"\settoggle{darft}{true",
        r"\settoggle*{darft}{true}",
        r"\toggletrue{ darft }",
        r"\toggletrue{darft\suffix}",
        r"\toggletrue{\name}",
        r"\toggletrue{darft",
        r"\newcommand{\wrapper}[2]{}\wrapper\toggletrue{darft}",
        r"\def\wrapper#1#2{}\wrapper\togglefalse{darft}",
        r"\NewDocumentCommand{\wrapper}{m m m}{}\wrapper\settoggle{darft}{true}",
        r"\newcommand{\wrapper}[2]{}\def\bridge{\wrapper}\bridge\iftoggle{darft}",
        r"\renewcommand{\toggletrue}[1]{}\toggletrue{darft}",
        r"\let\nottoggle\relax\nottoggle{darft}",
        r"\verb|\toggletrue{darft}|",
        r"% \toggletrue{darft}",
        r"\ifboolexpr{togl {darft}}{Yes}{No}",
    ] {
        std::fs::write(&main, format!("\\documentclass{{article}}\n{call}\n")).unwrap();
        let result = build_log::parse(&main, &log("darft", 2, call, true));
        let error = &result[&Url::from_file_path(&main).unwrap()][0];
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{call}: {error:?}"
        );
    }
    let call = r"\toggletrue{darft}";
    std::fs::write(&main, format!("\\documentclass{{article}}\n{call}\n")).unwrap();
    for invalid in [
        log("darft", 2, call, true).replace(HELP, "\n<argument> \\toggletrue{darft}\n"),
        log("darft", 2, call, true).replace("immediate help.", "more help."),
        log("darft", 2, call, true).replace("' undefined.", "' undefined. Extra."),
        log("darft", 2, call, true).replace("etoolbox Error:", "another Error:"),
        log("darft", 2, r"\togglefalse{darft}", true),
        log("darft ", 2, call, true),
    ] {
        let result = build_log::parse(&main, &invalid);
        let error = &result[&Url::from_file_path(&main).unwrap()][0];
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{invalid}: {error:?}"
        );
    }
}

#[test]
fn repeated_toggle_errors_require_unique_complete_consumed_echoes() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let first = r"\toggletrue{darft}";
    let second = r"\settoggle{darft}{false}";
    let source = format!("\\documentclass{{article}}\n{first}{second}\n");
    std::fs::write(&main, &source).unwrap();
    let result = build_log::parse(
        &main,
        &(log("darft", 2, first, true) + &log("darft", 2, &format!("{first}{second}"), true)),
    );
    let errors = &result[&Url::from_file_path(&main).unwrap()];
    assert_eq!(errors.len(), 2);
    for (error, (start, _)) in errors.iter().zip(source.match_indices("darft")) {
        assert_eq!(
            error.range,
            Text::new(source.clone()).range(start, start + 5)
        );
        assert_eq!(
            error.code,
            Some(NumberOrString::String("undefined-toggle".into()))
        );
    }
    for (call, echo, precise) in [
        (format!("\\relax{first}"), format!("...{first}"), true),
        (format!("{first}{first}"), format!("...{first}"), false),
        (first.into(), "...{darft}".into(), true),
        (first.into(), "...darft}".into(), false),
        (second.into(), "...{darft}{false}".into(), true),
        (second.into(), "...{false}".into(), false),
    ] {
        std::fs::write(&main, format!("\\documentclass{{article}}\n{call}\n")).unwrap();
        let result = build_log::parse(&main, &log("darft", 2, &echo, true));
        assert_eq!(
            result[&Url::from_file_path(&main).unwrap()][0]
                .code
                .is_some(),
            precise,
            "{call}/{echo}"
        );
    }
}

#[test]
fn toggle_candidates_exclude_booleans_other_projects_and_internal_command_conflicts() {
    for (declarations, expected) in [
        (r"\newtoggle{draft}", true),
        (r"\providetoggle{draft}", true),
        (r"\newtoggle{draft}\newbool{draft}", true),
        (r"\newbool{draft}", false),
        (r"\newcommand{\draft}{}", false),
        (r"\newtoggle{draft}\newtoggle{draft}", false),
        (r"\newtoggle{draft}\newtoggle{darft}", false),
        (r"\newtoggle{draft}\newcommand{\etb@tgl@draft}{}", false),
        (r"\newtoggle{draft}\newlength{\etb@tgl@draft}", false),
        (
            r"\newtoggle{draft}\newenvironment{etb@tgl@draft}{}{}",
            false,
        ),
        (
            r"\newtoggle{draft}\newenvironment{etb@tgl@darft}{}{}",
            false,
        ),
    ] {
        let (_directory, workspace, uri, diagnostic) = fixture(declarations, r"\toggletrue{darft}");
        let found = actions(&workspace, &uri, &diagnostic).iter().any(|item| matches!(item, CodeActionOrCommand::CodeAction(action) if action.title == "Replace with toggle `draft`"));
        assert_eq!(found, expected, "{declarations}");
    }
    let (directory, mut workspace, uri, diagnostic) = fixture("", r"\toggletrue{darft}");
    std::fs::write(
        directory.path().join("other.tex"),
        "\\documentclass{article}\\newtoggle{draft}\\begin{document}Other.\\end{document}\n",
    )
    .unwrap();
    workspace.add_root(directory.path().into());
    assert!(actions(&workspace, &uri, &diagnostic).is_empty());
}

#[test]
fn toggle_repairs_fail_closed_when_the_candidate_budget_is_exhausted() {
    let declarations = format!(
        "\\newtoggle{{draft}}{}",
        r"\newcommand{\irrelevant}{}".repeat(4096)
    );
    let (_directory, workspace, uri, diagnostic) = fixture(&declarations, r"\toggletrue{darft}");
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("undefined-toggle".into()))
    );
    assert!(actions(&workspace, &uri, &diagnostic).is_empty());
}

#[test]
fn toggle_fixes_recheck_versions_operands_candidate_types_and_macro_sources() {
    let (directory, mut workspace, uri, diagnostic) =
        fixture(r"\newtoggle{draft}", r"\toggletrue{darft}");
    workspace.client.versioned_edits = true;
    let response = actions(&workspace, &uri, &diagnostic);
    let CodeActionOrCommand::CodeAction(action) = &response[0] else {
        panic!("missing action")
    };
    let Some(DocumentChanges::Edits(edits)) = &action.edit.as_ref().unwrap().document_changes
    else {
        panic!("missing versioned edit")
    };
    assert_eq!(edits[0].text_document.version, Some(9));
    assert_eq!(edits[0].text_document.uri, uri);
    workspace.client.code_action_literals = false;
    workspace.client.apply_edits = true;
    let response = actions(&workspace, &uri, &diagnostic);
    let CodeActionOrCommand::Command(command) = &response[0] else {
        panic!("missing legacy action")
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let resolve = |workspace: &Workspace| {
        editing::resolve_fix(workspace, serde_json::from_value(value.clone()).unwrap())
    };
    assert!(resolve(&workspace).is_some());
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(resolve(&workspace).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(9);
    let source = workspace.documents[&uri].text.source.clone();
    workspace.open(uri.clone(), source.replace("{darft}", "{darft\\suffix}"), 9);
    assert!(resolve(&workspace).is_none());
    workspace.open(uri.clone(), source, 9);
    let definitions = Url::from_file_path(directory.path().join("definitions.tex")).unwrap();
    workspace.open(definitions.clone(), "\\newbool{draft}".into(), 1);
    assert!(resolve(&workspace).is_none());
    workspace.open(
        definitions,
        "\\newtoggle{draft}\\renewcommand{\\toggletrue}[1]{}".into(),
        2,
    );
    assert!(actions(&workspace, &uri, &diagnostic).is_empty());
    assert!(resolve(&workspace).is_none());
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_toggle_repairs_preserve_scopes_branches_and_boolean_independence_in_all_engines() {
    use std::os::unix::fs::PermissionsExt;
    use texquill::{compilation::build, config::BuildConfig};

    let source = concat!(
        "{\\global\\settoggle{darft}{true}}\n",
        "\\iftoggle{darft}{\\typeout{TOGGLE:initial:true}}{Wrong.}\n",
        "{\\settoggle{darft}{false}\\nottoggle{darft}{\\typeout{TOGGLE:local:false}}{Wrong.}}\n",
        "\\iftoggle{darft}{\\typeout{TOGGLE:restored:true}}{Wrong.}\n",
        "\\togglefalse\n{darft}\n",
        "\\nottoggle{darft}{\\typeout{TOGGLE:final:false}}{Wrong.}\n",
        "\\toggletrue{darft}\n",
        "\\ifbool{draft}{Wrong.}{\\typeout{TOGGLE:boolean:false}}\n",
    );
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let child = root.join("child.tex");
                std::fs::write(&main, "\\documentclass{article}\n\\usepackage{etoolbox}\n\\input{definitions}\n\\begin{document}\\input{child}Witness.\\end{document}\n").unwrap();
                std::fs::write(
                    root.join("definitions.tex"),
                    "\\newtoggle{draft}\\newbool{draft}\\boolfalse{draft}\n",
                )
                .unwrap();
                std::fs::write(&child, source).unwrap();
                let wrapper = root.join("engine-wrapper");
                std::fs::write(&wrapper, format!("#!/bin/sh\nexport max_print_line={width}\nexport TEXMFVAR=\"${{0%/*}}/tex-cache\"\nexport TEXMFCACHE=\"$TEXMFVAR\"\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
                let config = BuildConfig {
                    executable: wrapper.to_string_lossy().into_owned(),
                    arguments: Some(vec![
                        "-interaction=nonstopmode".into(),
                        "-no-shell-escape".into(),
                        if numbered {
                            "-file-line-error"
                        } else {
                            "-no-file-line-error"
                        }
                        .into(),
                        "-recorder".into(),
                        "-output-directory=%o".into(),
                        "%f".into(),
                    ]),
                    ..Default::default()
                };
                let before = build::run(&main, &config, &Default::default());
                assert_eq!(
                    before.status, "error",
                    "{engine}/{width}/{numbered}: {}",
                    before.output
                );
                let uri = Url::from_file_path(&child).unwrap();
                let errors: Vec<_> = before.diagnostics[&uri]
                    .iter()
                    .filter(|error| error.severity == Some(DiagnosticSeverity::ERROR))
                    .cloned()
                    .collect();
                assert_eq!(
                    errors.len(),
                    8,
                    "{engine}/{width}/{numbered}: {}",
                    before.output
                );
                let mut workspace = Workspace {
                    client: texquill::project::state::ClientCapabilities {
                        code_action_literals: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace
                    .project
                    .build_diagnostics
                    .insert(Url::from_file_path(&main).unwrap(), before.diagnostics);
                workspace.project.build_diagnostic_sources.insert(
                    Url::from_file_path(&main).unwrap(),
                    before.diagnostic_sources,
                );
                let mut edits = vec![];
                for (error, (start, _)) in errors.iter().zip(source.match_indices("darft")) {
                    assert_eq!(
                        error.code,
                        Some(NumberOrString::String("undefined-toggle".into())),
                        "{engine}/{width}/{numbered}: {error:?}"
                    );
                    assert_eq!(
                        error.range,
                        Text::new(source.into()).range(start, start + 5)
                    );
                    assert!(error.data.as_ref().unwrap()["macroSources"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|value| value
                            .as_str()
                            .is_some_and(|uri| uri.ends_with("/definitions.tex"))));
                    let edit = actions(&workspace, &uri, error)
                        .into_iter()
                        .find_map(|item| match item {
                            CodeActionOrCommand::CodeAction(action)
                                if action.title == "Replace with toggle `draft`" =>
                            {
                                action.edit
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| {
                            panic!("missing repair: {engine}/{width}/{numbered}: {error:?}")
                        });
                    edits.extend(edit.changes.unwrap()[&uri].clone());
                }
                edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
                let repaired = Text::new(source.into())
                    .changed(
                        edits
                            .into_iter()
                            .map(|edit| TextDocumentContentChangeEvent {
                                range: Some(edit.range),
                                range_length: None,
                                text: edit.new_text,
                            })
                            .collect(),
                    )
                    .unwrap()
                    .source;
                assert_eq!(repaired, source.replace("{darft}", "{draft}"));
                std::fs::write(&child, repaired).unwrap();
                let after = build::run(&main, &config, &Default::default());
                assert_eq!(
                    after.status, "success",
                    "{engine}/{width}/{numbered}: {}",
                    after.output
                );
                assert_eq!(after.pages, Some(1));
                for marker in [
                    "TOGGLE:initial:true",
                    "TOGGLE:local:false",
                    "TOGGLE:restored:true",
                    "TOGGLE:final:false",
                    "TOGGLE:boolean:false",
                ] {
                    assert!(
                        after.output.contains(marker),
                        "{engine}/{width}/{numbered}: {marker}"
                    );
                }
                assert!(!after
                    .diagnostics
                    .values()
                    .flatten()
                    .any(|error| error.severity == Some(DiagnosticSeverity::ERROR)));
            }
        }
    }
}
