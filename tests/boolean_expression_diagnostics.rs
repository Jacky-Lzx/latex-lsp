use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

const ERROR: &str = "Package etoolbox Error: Invalid boolean expression.";
const HELP: &str = "\nSee the etoolbox package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\n";

fn log(line: usize, echo: &str, detail: &str, numbered: bool) -> String {
    let header = if numbered {
        format!("./main.tex:{line}:")
    } else {
        "!".into()
    };
    format!(
        "(./main.tex\n{header} {ERROR}\n{HELP}l.{line} {echo}\n    {{Yes}}{{No}}\n{detail}\n\n)\n"
    )
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
fn mixed_expression_errors_keep_distinct_namespaces_and_exact_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let call = r"\ifboolexpr{bool {darft} or togl {darft}}";
    let source = format!("\\documentclass{{article}}\n{call}{{Yes}}{{No}}\n");
    std::fs::write(&main, &source).unwrap();
    for numbered in [false, true] {
        let result = build_log::parse(
            &main,
            &(log(2, call, "Boolean 'darft' undefined.", numbered)
                + &log(2, call, "Toggle 'darft' undefined.", numbered)),
        );
        let errors = &result[&Url::from_file_path(&main).unwrap()];
        assert_eq!(errors.len(), 2, "{errors:?}");
        for ((error, (start, _)), kind) in errors
            .iter()
            .zip(source.match_indices("darft"))
            .zip(["boolean", "toggle"])
        {
            assert_eq!(
                error.range,
                Text::new(source.clone()).range(start, start + 5)
            );
            assert_eq!(
                error.code,
                Some(NumberOrString::String(format!("undefined-{kind}")))
            );
            assert_eq!(error.data.as_ref().unwrap()[kind], "darft");
            assert!(error.message.starts_with(&format!("{ERROR}\n")));
        }
    }
}

#[test]
fn expression_repairs_preserve_operators_branches_and_the_other_namespace() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let call = r"\ifboolexpr{not (bool {darft} and togl {darft})}";
    let source = format!("\\newbool{{draft}}\\newtoggle{{draft}}\n{call}{{Yes}}{{No}}\n");
    std::fs::write(&main, &source).unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    for (kind, detail) in [
        ("boolean", "Boolean 'darft' undefined."),
        ("toggle", "Toggle 'darft' undefined."),
    ] {
        let diagnostics = build_log::parse(&main, &log(2, call, detail, true));
        let error = diagnostics[&uri][0].clone();
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        workspace.build_diagnostics.insert(uri.clone(), diagnostics);
        let edit = actions(&workspace, &uri, &error)
            .into_iter()
            .find_map(|item| match item {
                CodeActionOrCommand::CodeAction(action)
                    if action.title == format!("Replace with {kind} `draft`") =>
                {
                    action.edit
                }
                _ => None,
            })
            .expect("missing expression spelling repair");
        let edits = &edit.changes.unwrap()[&uri];
        assert_eq!(edits.len(), 1);
        let repaired = workspace.documents[&uri]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edits[0].range),
                range_length: None,
                text: edits[0].new_text.clone(),
            }])
            .unwrap()
            .source;
        let marker = if kind == "boolean" {
            "bool {darft}"
        } else {
            "togl {darft}"
        };
        assert_eq!(
            repaired,
            source.replace(marker, &marker.replace("darft", "draft"))
        );
    }
}

#[test]
fn expression_and_loop_boundaries_support_literal_tests_multiline_and_utf16() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for (call, echo, kind) in [
        (
            r"\ifboolexpe{nottogl{darft}}{Yes}{No}",
            r"\ifboolexpe{nottogl{darft}}",
            "toggle",
        ),
        (
            r"\ifboolexpr{not(bool{darft}or bool{true})}{Yes}{No}",
            r"\ifboolexpr{not(bool{darft}or bool{true})}",
            "boolean",
        ),
        (
            r"\whileboolexpr{bool{darft}}{Body}",
            r"\whileboolexpr{bool{darft}}{Body}",
            "boolean",
        ),
        (
            r"\unlessboolexpr{togl{darft}}{Body}",
            r"\unlessboolexpr{togl{darft}}{Body}",
            "toggle",
        ),
        (
            "\\whileboolexpr % keep\r\n{not (\r\nbool {darft}\r\n)}\r\n{Body}",
            "{Body}",
            "boolean",
        ),
        (
            r"\ifboolexpr{test{\ifstrequal{togl {darft}}{literal}}and togl{darft}}{Yes}{No}",
            r"\ifboolexpr{test{\ifstrequal{togl {darft}}{literal}}and togl{darft}}",
            "toggle",
        ),
        (
            r"\ifboolexpe{test{\ifblank{ }}or bool{darft}}{Yes}{No}",
            r"\ifboolexpe{test{\ifblank{ }}or bool{darft}}",
            "boolean",
        ),
        (
            r"\ifboolexpr{test{\ifstrequal{\input{extra}}{A}}or bool{darft}}{Yes}{No}",
            r"\ifboolexpr{test{\ifstrequal{\input{extra}}{A}}or bool{darft}}",
            "boolean",
        ),
        (
            r"\ifboolexpr{test{\ifstrempty{}}and togl{darft}}{Yes}{No}",
            r"\ifboolexpr{test{\ifstrempty{}}and togl{darft}}",
            "toggle",
        ),
        (
            r"\ifboolexpr{test{\notblank{Text}}and togl{darft}}{Yes}{No}",
            r"\ifboolexpr{test{\notblank{Text}}and togl{darft}}",
            "toggle",
        ),
    ] {
        let source = format!("\\documentclass{{article}}\n😀 {call}\n");
        std::fs::write(&main, &source).unwrap();
        let prefix = if call.contains('\n') {
            echo.to_owned()
        } else {
            format!("😀 {echo}")
        };
        let detail = format!(
            "{} 'darft' undefined.",
            if kind == "boolean" {
                "Boolean"
            } else {
                "Toggle"
            }
        );
        for numbered in [false, true] {
            let diagnostics = build_log::parse(
                &main,
                &log(source.lines().count(), &prefix, &detail, numbered),
            );
            let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
            let start = source.rfind("darft").unwrap();
            assert_eq!(
                error.range,
                Text::new(source.clone()).range(start, start + 5),
                "{call}: {error:?}"
            );
            assert_eq!(
                error.code,
                Some(NumberOrString::String(format!("undefined-{kind}")))
            );
        }
    }
}

#[test]
fn dynamic_tests_names_redefinitions_ambiguous_operands_and_incomplete_grammar_stay_unlocated() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for (call, kind) in [
        (
            r"\def\name{darft}\ifboolexpr{bool{\name} or bool{darft}}",
            "Boolean",
        ),
        (
            r"\def\name{darft}\ifboolexpe{togl{\name} or togl{darft}}",
            "Toggle",
        ),
        (r"\ifboolexpr{test{\probe}or togl{darft}}", "Toggle"),
        (r"\ifboolexpe{\expression or bool{darft}}", "Boolean"),
        (
            r"\renewcommand{\ifstrequal}[2]{}\ifboolexpr{test{\ifstrequal{A}{A}}and togl{darft}}",
            "Toggle",
        ),
        (
            r"\renewcommand{\ifboolexpr}[1]{}\ifboolexpr{bool{darft}}",
            "Boolean",
        ),
        (r"\let\ifboolexpe\relax\ifboolexpe{togl{darft}}", "Toggle"),
        (
            r"\newcommand{\wrapper}[2]{}\wrapper\ifboolexpr{togl{darft}}",
            "Toggle",
        ),
        (r"\ifboolexpr{bool{darft}or bool{darft}}", "Boolean"),
        (r"\ifboolexpr{togl{darft}and not togl{darft}}", "Toggle"),
        (r"\ifboolexpr{bool{darft}and}", "Boolean"),
        (r"\ifboolexpr{or togl{darft}}", "Toggle"),
        (r"\ifboolexpr{(bool{darft}}", "Boolean"),
        (r"\ifboolexpr{bool{darft} bool{other}}", "Boolean"),
        (r"\ifboolexpr{togl{ darft }}", "Toggle"),
        (r"\whileboolexpr{bool{darft}}", "Boolean"),
        (r"\unlessboolexpr{togl{darft}}{Body", "Toggle"),
        (r"\verb|\ifboolexpr{bool{darft}}|", "Boolean"),
        (r"% \ifboolexpr{togl{darft}}", "Toggle"),
    ] {
        std::fs::write(&main, format!("\\documentclass{{article}}\n{call}\n")).unwrap();
        let diagnostics = build_log::parse(
            &main,
            &log(2, call, &format!("{kind} 'darft' undefined."), true),
        );
        let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{call}: {error:?}"
        );
    }
}

#[test]
fn expression_details_do_not_borrow_adjacent_errors_or_unconsumed_source() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let call = r"\ifboolexpr{togl{darft}}";
    std::fs::write(
        &main,
        format!("\\documentclass{{article}}\n{call}{{Yes}}{{No}}\n"),
    )
    .unwrap();
    let valid = log(2, call, "Toggle 'darft' undefined.", true);
    for invalid in [
        valid.replace("Toggle 'darft' undefined.", ""),
        valid.replace(
            "Toggle 'darft' undefined.",
            "Toggle 'darft' undefined. Extra.",
        ),
        valid.replace("Toggle 'darft' undefined.", "The invalid part is: 'togl'."),
        valid.replace(
            "    {Yes}{No}\nToggle 'darft' undefined.",
            "Toggle 'darft' undefined.",
        ),
        valid.replace(HELP, "\n<argument> togl{darft}\n"),
        valid.replace("immediate help.", "more help."),
        valid.replace("l.2 ", "l.3 "),
        valid
            .replace("    {Yes}{No}", "    Toggle 'darft' undefined.")
            .replace("\nToggle 'darft' undefined.\n", "\n\n"),
    ] {
        let diagnostics = build_log::parse(&main, &invalid);
        let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{invalid}: {error:?}"
        );
    }
    let missing = valid.replace("Toggle 'darft' undefined.", "");
    let diagnostics = build_log::parse(&main, &(missing + &valid));
    let errors = &diagnostics[&Url::from_file_path(&main).unwrap()];
    assert_eq!(errors.len(), 2);
    assert_eq!(
        errors.iter().filter(|error| error.code.is_some()).count(),
        1
    );
    assert!(errors.iter().any(|error| error.message == ERROR));
}

#[test]
fn wrapped_details_keep_names_and_do_not_pop_the_source_stack() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let name = format!("{}darft", "long".repeat(24));
    let call = format!("\\ifboolexpr{{bool{{{name}}}}}");
    let source = format!("\\documentclass{{article}}\n{call}\n");
    std::fs::write(&main, &source).unwrap();
    let detail = format!("Boolean '{name}' undefined.");
    for width in [60, 79] {
        let detail = detail
            .as_bytes()
            .chunks(width)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let diagnostics = build_log::parse(&main, &log(2, &call, &detail, true));
        let error = &diagnostics[&uri][0];
        assert_eq!(error.data.as_ref().unwrap()["boolean"], name);
        let start = source.find(&name).unwrap();
        assert_eq!(
            error.range,
            Text::new(source.clone()).range(start, start + name.len())
        );
    }
    let child = directory.path().join("child.tex");
    let child_call = r"\ifboolexpr{togl{darft)}}";
    std::fs::write(
        &child,
        format!("{child_call}{{Yes}}{{No}}\n\\booltrue{{childMissing}}\n"),
    )
    .unwrap();
    std::fs::write(&main, "\\input{child}\n\\booltrue{missing}\n").unwrap();
    let output = format!("(./main.tex\n(./child.tex\n! {ERROR}\n{HELP}l.1 {child_call}\n    {{Yes}}{{No}}\nToggle 'darft)' undefined.\n\n! Package etoolbox Error: Boolean '\\ifchildMissing' undefined.\n{HELP}l.2 \\booltrue{{childMissing}}\n    \n)\n! Package etoolbox Error: Boolean '\\ifmissing' undefined.\n{HELP}l.2 \\booltrue{{missing}}\n    \n)\n");
    let diagnostics = build_log::parse(&main, &output);
    let child_errors = &diagnostics[&Url::from_file_path(child).unwrap()];
    assert_eq!(child_errors.len(), 2, "{diagnostics:?}");
    let child_error = &child_errors[0];
    assert!(child_error.code.is_none());
    assert!(child_error.message.ends_with("Toggle 'darft)' undefined."));
    assert_eq!(
        child_errors[1].data.as_ref().unwrap()["boolean"],
        "childMissing"
    );
    assert_eq!(
        diagnostics[&uri][0].code,
        Some(NumberOrString::String("undefined-boolean".into())),
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics[&uri][0].data.as_ref().unwrap()["boolean"],
        "missing"
    );
}

#[test]
fn repeated_calls_shortened_echoes_and_analysis_budgets_require_a_unique_operand() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let first = r"\ifboolexpr{togl{darft}}";
    for (call, echo, precise) in [
        (format!("\\relax{first}"), format!("...{first}"), true),
        (
            format!("{first}{{Y}}{{N}}{first}"),
            format!("...{first}"),
            false,
        ),
        (first.into(), "...{togl{darft}}".into(), true),
        (first.into(), "...{darft}}".into(), true),
        (first.into(), "...ft}}".into(), true),
        (first.into(), "...}}".into(), false),
        (
            r"\whileboolexpr{togl{darft}}{\stepcounter{iterations}\togglefalse{draft}}".into(),
            r"...\togglefalse{draft}}".into(),
            false,
        ),
        (
            format!(
                "\\ifboolexpr{{{}togl{{darft}}{}}}",
                "(".repeat(129),
                ")".repeat(129)
            ),
            String::new(),
            false,
        ),
        (
            format!("\\ifboolexpr{{{}togl{{darft}}}}", " ".repeat(16_384)),
            String::new(),
            false,
        ),
    ] {
        std::fs::write(&main, format!("\\documentclass{{article}}\n{call}\n")).unwrap();
        let echo = if echo.is_empty() { &call } else { &echo };
        let diagnostics = build_log::parse(&main, &log(2, echo, "Toggle 'darft' undefined.", true));
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
fn expression_fixes_revalidate_tests_candidates_versions_and_readonly_builtins() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let call = r"\ifboolexpr{bool{darft}and test{\ifstrequal{A}{A}}}";
    let source = format!("\\input{{definitions}}\n{call}\n");
    std::fs::write(&main, &source).unwrap();
    std::fs::write(directory.path().join("definitions.tex"), "\\newbool{draft}").unwrap();
    let diagnostics = build_log::parse(&main, &log(2, call, "Boolean 'darft' undefined.", true));
    let uri = Url::from_file_path(&main).unwrap();
    let error = diagnostics[&uri][0].clone();
    let mut workspace = Workspace {
        code_action_literals: true,
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(directory.path().into());
    workspace.open(uri.clone(), source.clone(), 9);
    workspace.build_diagnostics.insert(uri.clone(), diagnostics);
    let response = actions(&workspace, &uri, &error);
    let CodeActionOrCommand::CodeAction(action) = &response[0] else {
        panic!("missing action")
    };
    let Some(DocumentChanges::Edits(edits)) = &action.edit.as_ref().unwrap().document_changes
    else {
        panic!("missing versioned edit")
    };
    assert_eq!(edits[0].text_document.version, Some(9));
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let response = actions(&workspace, &uri, &error);
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
    let definitions = Url::from_file_path(directory.path().join("definitions.tex")).unwrap();
    workspace.open(
        definitions.clone(),
        "\\newbool{draft}\\renewcommand{\\ifstrequal}[2]{}".into(),
        1,
    );
    assert!(resolve(&workspace).is_none());
    workspace.open(definitions.clone(), "\\newtoggle{draft}".into(), 2);
    assert!(resolve(&workspace).is_none());
    workspace.open(definitions, "\\newbool{draft}".into(), 3);
    workspace.open(
        uri.clone(),
        source.replace("bool{darft}", "bool{darft}or bool{darft}"),
        9,
    );
    assert!(resolve(&workspace).is_none());

    let call = r"\ifboolexpr{bool{ture}and not bool{flase}}";
    std::fs::write(&main, format!("\\documentclass{{article}}\n{call}\n")).unwrap();
    let diagnostics = build_log::parse(
        &main,
        &(log(2, call, "Boolean 'ture' undefined.", true)
            + &log(2, call, "Boolean 'flase' undefined.", true)),
    );
    let errors = diagnostics[&uri].clone();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(directory.path().into());
    workspace.build_diagnostics.insert(uri.clone(), diagnostics);
    for (error, value) in errors.iter().zip(["true", "false"]) {
        assert!(actions(&workspace, &uri, error).iter().any(|item| matches!(item, CodeActionOrCommand::CodeAction(action) if action.title == format!("Replace with boolean `{value}`"))));
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_expression_repairs_preserve_logic_loops_literals_and_wrapped_names_in_all_engines() {
    use latex_lsp::{build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    let long_name = format!("{}draft", "audit".repeat(16));
    let long_typo = long_name.replace("draft", "darft");
    let marked = format!("{}\\ifboolexpr{{bool{{¦boolean:{long_typo}¦}}}}{{\\typeout{{EXPR:long:true}}}}{{Wrong.}}\n{}", concat!(
        "\\ifboolexpr{bool{¦boolean:darft¦}or togl{¦toggle:darft¦}}{\\typeout{EXPR:mixed:true}}{Wrong.}\n",
        "\\ifboolexpe{not(bool{¦boolean:darft¦})and togl{¦toggle:darft¦}}{\\typeout{EXPR:expanded:true}}{Wrong.}\n",
        "\\ifboolexpr{test{\\ifstrequal{togl{darft}}{togl{darft}}}and togl{¦toggle:darft¦}}{\\typeout{EXPR:literal:true}}{Wrong.}\n",
        "\\whileboolexpr{togl{¦toggle:darft¦}}{\\tick}\n",
        "\\typeout{EXPR:iterations:\\arabic{iterations}}\\toggletrue{draft}\n",
        "\\ifboolexpr{bool{¦boolean:ture¦}and not bool{¦boolean:flase¦}}{\\typeout{EXPR:builtin:true}}{Wrong.}\n",
        "\\ifboolexpr\n{not(\nbool{¦boolean:darft¦}\n)}\n{\\typeout{EXPR:multiline:true}}{Wrong.}\n",
    ), "\\unlessboolexpr{not bool{¦boolean:darft¦}}{\\end{document}}\n");
    let mut source = String::new();
    let mut expected = vec![];
    let mut repaired_source = String::new();
    for (index, part) in marked.split('¦').enumerate() {
        if index % 2 == 0 {
            source.push_str(part);
            repaired_source.push_str(part);
        } else {
            let (kind, name) = part.split_once(':').unwrap();
            let replacement = match name {
                "ture" => "true".to_owned(),
                "flase" => "false".to_owned(),
                _ => name.replace("darft", "draft"),
            };
            expected.push((source.len(), kind, name, replacement.clone()));
            source.push_str(name);
            repaired_source.push_str(&replacement);
        }
    }
    assert_eq!(expected.len(), 11);
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let child = root.join("child.tex");
                std::fs::write(&main, "\\documentclass{article}\n\\usepackage{etoolbox}\n\\input{definitions}\n\\begin{document}\\input{child}Witness.\\end{document}\n").unwrap();
                std::fs::write(root.join("definitions.tex"), format!("\\newbool{{draft}}\\newtoggle{{draft}}\\toggletrue{{draft}}\\newcounter{{iterations}}\\newcommand{{\\tick}}{{\\stepcounter{{iterations}}\\togglefalse{{draft}}}}\n\\newbool{{{long_name}}}\\booltrue{{{long_name}}}\n")).unwrap();
                std::fs::write(&child, &source).unwrap();
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
                    expected.len(),
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
                let mut edits = vec![];
                for (error, (start, kind, name, replacement)) in errors.iter().zip(&expected) {
                    assert_eq!(
                        error.code,
                        Some(NumberOrString::String(format!("undefined-{kind}"))),
                        "{engine}/{width}/{numbered}: {error:?}"
                    );
                    assert_eq!(
                        error.range,
                        Text::new(source.clone()).range(*start, start + name.len())
                    );
                    assert_eq!(error.data.as_ref().unwrap()[kind], *name);
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
                                if action.title
                                    == format!("Replace with {kind} `{replacement}`") =>
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
                let repaired = Text::new(source.clone())
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
                assert_eq!(repaired, repaired_source);
                std::fs::write(&child, repaired).unwrap();
                let after = build::run(&main, &config, &Default::default());
                assert_eq!(
                    after.status, "success",
                    "{engine}/{width}/{numbered}: {}",
                    after.output
                );
                assert_eq!(after.pages, Some(1));
                for marker in [
                    "EXPR:mixed:true",
                    "EXPR:expanded:true",
                    "EXPR:literal:true",
                    "EXPR:iterations:1",
                    "EXPR:builtin:true",
                    "EXPR:multiline:true",
                    "EXPR:long:true",
                ] {
                    assert!(
                        after.output.contains(marker),
                        "{engine}/{width}/{numbered}: {marker}"
                    );
                }
                std::fs::write(&child, concat!(
                    "\\def\\name{darft}\\ifboolexpr{togl{\\name}or togl{darft}}{Yes}{No}\n",
                    "\\newcommand\\probe[2]{\\ifboolexpr{bool{darft}}{#1}{#2}}\\ifboolexpr{test{\\probe}or bool{darft}}{Yes}{No}\n",
                )).unwrap();
                let uncertain = build::run(&main, &config, &Default::default());
                assert_eq!(uncertain.status, "error", "{}", uncertain.output);
                let errors: Vec<_> = uncertain.diagnostics[&uri]
                    .iter()
                    .filter(|error| error.message.starts_with(ERROR))
                    .collect();
                assert!(errors.len() >= 2, "{}", uncertain.output);
                assert!(
                    errors.iter().all(|error| error.code.is_none()),
                    "{errors:?}"
                );
            }
        }
    }
}
