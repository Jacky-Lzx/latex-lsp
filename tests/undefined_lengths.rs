use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn log(call: &str, numbered: bool) -> String {
    let header = if numbered { "./main.tex:2:" } else { "!" };
    format!("(./main.tex\n{header} Package etoolbox Error: Length '\\measuer' undefined.\n\nSee the etoolbox package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\nl.2 {call}\n)\n")
}

#[test]
fn length_assignment_errors_select_the_control_sequence_and_preserve_the_expression() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for call in [
        r"\deflength{\measuer}{2pt+3pt}",
        r"\deflength\measuer{2pt+3pt}",
        "\\global\\deflength % keep\r\n{\\measuer}{2pt+3pt}",
    ] {
        let source = format!("\\documentclass{{article}}\n{call}\n");
        std::fs::write(&main, &source).unwrap();
        for numbered in [false, true] {
            let log = log(call.lines().last().unwrap(), numbered)
                .replace(":2:", &format!(":{}:", source.lines().count()))
                .replace("l.2 ", &format!("l.{} ", source.lines().count()));
            let diagnostics = build_log::parse(&main, &log);
            let diagnostic = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
            let start = source.find(r"\measuer").unwrap();
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + 8),
                "{call}: {diagnostic:?}"
            );
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("undefined-length".into()))
            );
            assert_eq!(diagnostic.data.as_ref().unwrap()["length"], "measuer");
        }
    }
}

#[test]
fn a_missing_length_offers_a_spelling_fix_from_its_project() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let call = r"\deflength{\measuer}{2pt+3pt}";
    let source = format!("\\newlength{{\\measure}}\n{call}\n");
    std::fs::write(&main, &source).unwrap();
    let diagnostics = build_log::parse(&main, &log(call, true));
    let uri = Url::from_file_path(&main).unwrap();
    let diagnostic = diagnostics[&uri][0].clone();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(directory.path().into());
    workspace.build_diagnostics.insert(uri.clone(), diagnostics);
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    );
    let edit = actions
        .into_iter()
        .find_map(|item| match item {
            CodeActionOrCommand::CodeAction(action)
                if action.title == r"Replace with length `\measure`" =>
            {
                action.edit
            }
            _ => None,
        })
        .expect("missing length spelling fix");
    let edits = &edit.changes.unwrap()[&uri];
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, r"\measure");
    assert_eq!(
        edits[0].range,
        Text::new(source.clone()).range(
            source.rfind(r"\measuer").unwrap(),
            source.rfind(r"\measuer").unwrap() + 8
        )
    );
}

fn fixture(declarations: &str) -> (tempfile::TempDir, Workspace, Url, Diagnostic) {
    fixture_name(declarations, "measuer")
}

fn fixture_name(declarations: &str, name: &str) -> (tempfile::TempDir, Workspace, Url, Diagnostic) {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = format!("\\input{{definitions}}\n\\deflength{{\\{name}}}{{2pt+3pt}}\n");
    std::fs::write(&main, &source).unwrap();
    std::fs::write(directory.path().join("definitions.tex"), declarations).unwrap();
    let diagnostics = build_log::parse(
        &main,
        &log(source.lines().nth(1).unwrap(), true).replace("measuer", name),
    );
    let uri = Url::from_file_path(&main).unwrap();
    let diagnostic = diagnostics[&uri][0].clone();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(directory.path().into());
    workspace.open(uri.clone(), source, 9);
    workspace.build_diagnostics.insert(uri.clone(), diagnostics);
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
fn length_candidates_include_dimensions_and_glue_but_exclude_other_types_and_conflicts() {
    for (declarations, expected) in [
        (r"\newlength{\measure}", true),
        (r"\newskip\measure", true),
        (r"\newdimen\measure", true),
        (r"\skipdef\measure=0", true),
        (r"\dimendef\measure=0", true),
        (r"\expandafter\newskip\csname measure\endcsname", true),
        (r"\newcommand{\measure}{}", false),
        (r"\newcount\measure", false),
        (r"\newmuskip\measure", false),
        (r"\newbox\measure", false),
        (r"\newlength{\measure}\newcommand{\measure}{}", false),
        (r"\newlength{\measure}\newlength{\measure}", false),
        (r"\newlength{\measure}\newlength{\measuer}", false),
        (r"\newlength{\measure}\renewenvironment{measure}{}{}", false),
        (r"\newlength{\measure}\newtheorem{measure}{Measure}", false),
        (r"\newlength{\measure}\newenvironment{measuer}{}{}", false),
    ] {
        let (_directory, workspace, uri, diagnostic) = fixture(declarations);
        let found = actions(&workspace, &uri, &diagnostic).iter().any(|item| matches!(item, CodeActionOrCommand::CodeAction(action) if action.title == r"Replace with length `\measure`"));
        assert_eq!(found, expected, "{declarations}");
    }
}

#[test]
fn uncertain_dynamic_incomplete_and_macro_operand_errors_keep_line_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for call in [
        r"\deflength*{\measuer}{1pt}",
        r"\deflength{\measuer\other}{1pt}",
        r"\deflength{ \measuer }{1pt}",
        r"\deflength{\csname measuer\endcsname}{1pt}",
        r"\deflength{\measuer}",
        r"\deflength{\measuer}{1pt",
        r"\setlength{\measuer}{1pt}",
        r"\newcommand{\wrapper}[3]{}\wrapper\deflength{\measuer}{1pt}",
        r"\def\wrapper#1#2#3{}\wrapper\deflength{\measuer}{1pt}",
        r"\renewcommand{\deflength}[2]{}\deflength{\measuer}{1pt}",
        r"\let\deflength\relax\deflength{\measuer}{1pt}",
        r"\verb|\deflength{\measuer}{1pt}|",
        r"% \deflength{\measuer}{1pt}",
    ] {
        std::fs::write(&main, format!("\\documentclass{{article}}\n{call}\n")).unwrap();
        let diagnostics = build_log::parse(&main, &log(call, true));
        let diagnostic = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert!(
            diagnostic.code.is_none() && diagnostic.data.is_none(),
            "{call}: {diagnostic:?}"
        );
    }
}

#[test]
fn builtin_length_repairs_respect_local_shadowing_and_other_projects() {
    for (declarations, expected) in [
        ("", true),
        (r"\newcommand{\textwidth}{}", false),
        (r"\newcount\textwidth", false),
    ] {
        let (_directory, workspace, uri, diagnostic) = fixture_name(declarations, "textwdith");
        let found = actions(&workspace, &uri, &diagnostic).iter().any(|item| matches!(item, CodeActionOrCommand::CodeAction(action) if action.title == r"Replace with length `\textwidth`"));
        assert_eq!(found, expected, "{declarations}");
    }
    let (directory, mut workspace, uri, diagnostic) = fixture("");
    std::fs::write(
        directory.path().join("other.tex"),
        "\\documentclass{article}\\newlength{\\measure}\\begin{document}Other.\\end{document}\n",
    )
    .unwrap();
    workspace.add_root(directory.path().into());
    assert!(actions(&workspace, &uri, &diagnostic).is_empty());
}

#[test]
fn repeated_length_errors_require_a_unique_consumed_source_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let first = r"\deflength{\measuer}{1pt}";
    let call = format!("{first}\\deflength\\measuer{{2pt}}");
    let source = format!("\\documentclass{{article}}\n{call}\n");
    std::fs::write(&main, &source).unwrap();
    let diagnostics = build_log::parse(&main, &(log(first, true) + &log(&call, true)));
    let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(errors.len(), 2);
    for (diagnostic, (start, _)) in errors.iter().zip(source.match_indices(r"\measuer")) {
        assert_eq!(
            diagnostic.range,
            Text::new(source.clone()).range(start, start + 8)
        );
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("undefined-length".into()))
        );
    }
    for (call, echo, precise) in [
        (format!("\\relax{first}"), format!("...{first}"), true),
        (format!("{first}{first}"), format!("...{first}"), false),
        (first.into(), r"...{\measuer}{1pt}".into(), true),
        (first.into(), r"...measuer}{1pt}".into(), false),
    ] {
        std::fs::write(&main, format!("\\documentclass{{article}}\n{call}\n")).unwrap();
        let diagnostics = build_log::parse(&main, &log(&echo, true));
        assert_eq!(
            diagnostics[&Url::from_file_path(&main).unwrap()][0]
                .code
                .is_some(),
            precise,
            "{call}/{echo}"
        );
    }
}

#[test]
fn versioned_and_legacy_length_fixes_revalidate_candidates_versions_and_operands() {
    let (directory, mut workspace, uri, diagnostic) = fixture(r"\newlength{\measure}");
    workspace.versioned_edits = true;
    let response = actions(&workspace, &uri, &diagnostic);
    let CodeActionOrCommand::CodeAction(action) = &response[0] else {
        panic!("missing action")
    };
    let Some(DocumentChanges::Edits(edits)) = &action.edit.as_ref().unwrap().document_changes
    else {
        panic!("missing versioned edits")
    };
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].text_document.version, Some(9));
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let response = actions(&workspace, &uri, &diagnostic);
    let CodeActionOrCommand::Command(command) = &response[0] else {
        panic!("missing legacy command")
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let resolve = |workspace: &Workspace, value: serde_json::Value| {
        editing::resolve_fix(workspace, serde_json::from_value(value).unwrap())
    };
    assert!(resolve(&workspace, value.clone()).is_some());
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(resolve(&workspace, value.clone()).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(9);
    let source = workspace.documents[&uri].text.source.clone();
    workspace.open(uri.clone(), source.replace("{2pt+3pt}", ""), 9);
    assert!(resolve(&workspace, value.clone()).is_none());
    workspace.open(uri.clone(), source, 9);
    let definitions = Url::from_file_path(directory.path().join("definitions.tex")).unwrap();
    workspace.open(definitions, r"\newcommand{\measure}{}".into(), 1);
    assert!(resolve(&workspace, value).is_none());
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_length_errors_offer_repairs_and_compile_with_original_expressions_in_all_engines() {
    use latex_lsp::{build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    let source = concat!(
        "\\ifcslength{measuer}{Yes}{No}\\deflength{\\measuer}{2pt+3pt}\n",
        "\\global\\deflength\\textwdith{100pt}\n",
        "\\deflength\n{\\distnace}\n{2pt+3pt}\n",
        "\\typeout{LENGTH-AUDIT:\\the\\measure/\\the\\distance/\\the\\textwidth}\n",
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
                    "\\newlength{\\measure}\\newdimen\\distance\n",
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
                    .filter(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
                    .cloned()
                    .collect();
                assert_eq!(
                    errors.len(),
                    3,
                    "{engine}/{width}/{numbered}: {}",
                    before.output
                );
                let mut workspace = Workspace {
                    code_action_literals: true,
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace
                    .build_diagnostics
                    .insert(Url::from_file_path(&main).unwrap(), before.diagnostics);
                workspace.build_diagnostic_sources.insert(
                    Url::from_file_path(&main).unwrap(),
                    before.diagnostic_sources,
                );
                let mut edits = Vec::new();
                for (diagnostic, (typo, replacement)) in errors.iter().zip([
                    ("measuer", "measure"),
                    ("textwdith", "textwidth"),
                    ("distnace", "distance"),
                ]) {
                    assert_eq!(
                        diagnostic.code,
                        Some(NumberOrString::String("undefined-length".into())),
                        "{engine}/{width}/{numbered}: {diagnostic:?}"
                    );
                    let start = source.find(&format!("\\{typo}")).unwrap();
                    assert_eq!(
                        diagnostic.range,
                        Text::new(source.into()).range(start, start + typo.len() + 1)
                    );
                    let response = actions(&workspace, &uri, diagnostic);
                    let edit = response
                        .into_iter()
                        .find_map(|item| match item {
                            CodeActionOrCommand::CodeAction(action)
                                if action.title
                                    == format!("Replace with length `\\{replacement}`") =>
                            {
                                action.edit
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| {
                            panic!("missing repair: {engine}/{width}/{numbered}: {diagnostic:?}")
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
                assert_eq!(
                    repaired,
                    source
                        .replace(r"\measuer", r"\measure")
                        .replace(r"\textwdith", r"\textwidth")
                        .replace(r"\distnace", r"\distance")
                );
                std::fs::write(&child, repaired).unwrap();
                let after = build::run(&main, &config, &Default::default());
                assert_eq!(
                    after.status, "success",
                    "{engine}/{width}/{numbered}: {}",
                    after.output
                );
                assert_eq!(after.pages, Some(1));
                assert!(after.output.contains("LENGTH-AUDIT:5.0pt/5.0pt/100.0pt"));
                assert!(!after
                    .diagnostics
                    .values()
                    .flatten()
                    .any(|diagnostic| diagnostic.code
                        == Some(NumberOrString::String("undefined-length".into()))));
            }
        }
    }
}
