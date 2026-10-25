#![cfg(unix)]
use latex_lsp::{build, config::BuildConfig};
use lsp_types::*;
use std::os::unix::fs::PermissionsExt;

fn build_fixture(
    root: &std::path::Path,
    source: &str,
    definitions: &str,
    log: &str,
) -> build::Outcome {
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("macros.tex"), definitions).unwrap();
    std::fs::write(root.join("fixture.log"), log).unwrap();
    let compiler = root.join("compiler");
    std::fs::write(
        &compiler,
        "#!/bin/sh\nif [ -f \"${2%/*}/replacement.tex\" ]; then cp \"${2%/*}/replacement.tex\" \"${2%/*}/macros.tex\"; fi\ncat \"${2%/*}/fixture.log\" > \"$1/main.log\"\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = BuildConfig {
        executable: compiler.to_string_lossy().into(),
        arguments: Some(vec!["%o".into(), "%f".into()]),
        ..Default::default()
    };
    build::run(&root.join("main.tex"), &config, &Default::default())
}

#[test]
fn hash_arguments_require_declared_macros_and_keep_executed_box_contents_precise() {
    for (definition, row, precise) in [
        (r"\def\bad#1#2{#2}", r"\bad x#", false),
        (r"\def\bad#1;{#1}", r"\bad x#;", false),
        (r"\def\bad{}\def\bad#1#2{#2}", r"\bad x#", false),
        (
            r"\def\other#1#2{#2}\def\bad{}\def\bad{\other}",
            r"\bad x#",
            false,
        ),
        (r"\def\bad{}", r"\bad x#", true),
        ("", r"\hbox{#}", true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("\\input{{macros}}\n{row}\n");
        let prefix = &row[..row.rfind('#').unwrap() + 1];
        let log = format!("(./main.tex\n(./macros.tex)\n! You can't use `macro parameter character #' in restricted horizontal mode.\nl.2 {prefix}\n\n)\n");
        let outcome = build_fixture(directory.path(), &source, definition, &log);
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        assert_eq!(
            outcome.diagnostics[&uri][0].code.is_some(),
            precise,
            "{definition}/{row}: {:?}",
            outcome.diagnostics
        );
    }
}

#[test]
fn copied_commands_preserve_possible_bare_and_delimited_arguments() {
    for (definition, call) in [
        (r"\def\consume#1#2{&}\let\bad\consume", r"\bad x&"),
        (
            r"\def\consume#1#2{&}\expandafter\let\csname bad\endcsname\consume",
            r"\bad x&",
        ),
        (r"\def\consume#1#2{&}\let\bad=\consume", r"\bad x&"),
        (r"\def\consume#1&{&}\let\bad\consume", r"\bad x&"),
        (
            r"\def\consume#1#2{&}\NewCommandCopy{\bad}{\consume}",
            r"\bad x&",
        ),
        (r"\def\consume#1#2{&}\cslet{bad}\consume", r"\bad x&"),
        (r"\def\consume#1#2{&}\cslet{bad}{\consume}", r"\bad x&"),
        (r"\def\consume#1#2{&}\letcs\bad{consume}", r"\bad x&"),
        (r"\def\consume#1#2{&}\csletcs{bad}{consume}", r"\bad x&"),
        (
            r"\def\consume#1#2{&}\let\bad\consume\def\consume{}",
            r"\bad x&",
        ),
        (
            r"\def\consume#1#2{&}\let\middle\consume\let\bad\middle",
            r"\bad x&",
        ),
        (
            r"\newenvironment{original}[2]{&}{}\NewEnvironmentCopy{bad}{original}",
            r"\begin{bad}x&\end{bad}",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let row = format!("\\begin{{tabular}}{{ll}}A & B{call} C\\\\\\end{{tabular}}");
        let source = format!("\\input{{macros}}\n{row}\n");
        let prefix = &row[..row.rfind('&').unwrap() + 1];
        let log = format!("(./main.tex\n(./macros.tex)\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.2 {prefix}\n                 following source\n\n)\n");
        let outcome = build_fixture(directory.path(), &source, definition, &log);
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        assert!(
            outcome.diagnostics[&uri][0].code.is_none(),
            "{definition}: {:?}",
            outcome.diagnostics
        );
    }
}

#[test]
fn copying_a_source_is_not_an_invocation_and_completed_copies_keep_tab_repairs() {
    for copy in [
        r"\let\bad\consume",
        r"\let\bad=\consume",
        r"\NewCommandCopy{\bad}{\consume}",
        r"\cslet{bad}\consume",
        r"\letcs\bad{consume}",
        r"\csletcs{bad}{consume}",
    ] {
        for call in ["x&", r"\bad{x}{y}&"] {
            let directory = tempfile::tempdir().unwrap();
            let row = format!("\\begin{{tabular}}{{ll}}A & B\\def\\consume#1#2{{}}{copy} {call} C\\\\\\end{{tabular}}");
            let source = format!("{row}\n");
            let prefix = &row[..row.rfind('&').unwrap() + 1];
            let log = format!("(./main.tex\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.1 {prefix}\n                 following source\n\n)\n");
            let outcome = build_fixture(directory.path(), &source, "", &log);
            let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
            let item = outcome.diagnostics[&uri][0].clone();
            assert_eq!(
                item.code,
                Some(NumberOrString::String("extra-alignment-tab".into())),
                "{copy}/{call}: {item:?}"
            );
            let mut workspace = latex_lsp::workspace::Workspace {
                code_action_literals: true,
                ..Default::default()
            };
            workspace.add_root(directory.path().into());
            workspace.open(uri.clone(), source, 9);
            workspace
                .build_diagnostic_sources
                .insert(uri.clone(), outcome.diagnostic_sources);
            workspace
                .build_diagnostics
                .insert(uri.clone(), outcome.diagnostics);
            assert_eq!(
                selected_actions(&workspace, &uri, &item).len(),
                1,
                "{copy}/{call}"
            );
        }
    }
}

#[test]
fn repeated_declarations_preserve_possible_bare_delimited_and_forwarded_arguments() {
    for (definition, call) in [
        (r"\def\bad{}\def\bad#1#2{&}", r"\bad x&"),
        (r"\newcommand{\bad}{}\renewcommand{\bad}[2]{&}", r"\bad x&"),
        (
            r"\NewDocumentCommand{\bad}{}{}\RenewDocumentCommand{\bad}{mm}{&}",
            r"\bad x&",
        ),
        (r"\def\bad{}\def\bad#1&{&}", r"\bad x&"),
        (r"\def\bad{}\def\bad#1#2{&}\def\bridge{\bad}", r"\bridge x&"),
        (r"\def\other#1#2{&}\def\bad{}\def\bad{\other}", r"\bad x&"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let row = format!("\\begin{{tabular}}{{ll}}A & B{call} C\\\\\\end{{tabular}}");
        let source = format!("\\input{{macros}}\n{row}\n");
        let prefix = &row[..row.rfind('&').unwrap() + 1];
        let log = format!("(./main.tex\n(./macros.tex)\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.2 {prefix}\n                 following source\n\n)\n");
        let outcome = build_fixture(directory.path(), &source, definition, &log);
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        assert!(
            outcome.diagnostics[&uri][0].code.is_none(),
            "{definition}: {:?}",
            outcome.diagnostics
        );
    }
}

#[test]
fn completed_or_argument_free_redeclarations_still_offer_literal_tab_repairs() {
    for (definition, call) in [
        (r"\def\bad{}\def\bad{}", r"\bad x&"),
        (r"\def\bad#1#2{}\def\bad#1#2{}", r"\bad xy&"),
        (r"\def\bad#1#2{}\def\bad#1#2{}", r"\bad{x}{y}&"),
        (r"\def\bad#1#2{}\def\bridge{\bad}", r"\bridge{x}{y}&"),
        (r"\def\bad#1;{}\def\bad#1;{}", r"\bad x;&"),
        (
            r"\newenvironment{bad}[2]{}{}\renewenvironment{bad}[2]{}{}",
            r"\begin{bad}xy&\end{bad}",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let row = format!("\\begin{{tabular}}{{ll}}A & B{call} C\\\\\\end{{tabular}}");
        let source = format!("\\input{{macros}}\n{row}\n");
        let prefix = &row[..row.rfind('&').unwrap() + 1];
        let log = format!("(./main.tex\n(./macros.tex)\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.2 {prefix}\n                 following source\n\n)\n");
        let outcome = build_fixture(directory.path(), &source, definition, &log);
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        assert_eq!(
            outcome.diagnostics[&uri][0].code,
            Some(NumberOrString::String("extra-alignment-tab".into())),
            "{definition}: {:?}",
            outcome.diagnostics
        );
        let mut workspace = latex_lsp::workspace::Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        workspace.open(uri.clone(), source, 9);
        let item = outcome.diagnostics[&uri][0].clone();
        workspace
            .build_diagnostic_sources
            .insert(uri.clone(), outcome.diagnostic_sources);
        workspace
            .build_diagnostics
            .insert(uri.clone(), outcome.diagnostics);
        assert_eq!(selected_actions(&workspace, &uri, &item).len(), 1);
    }
}

#[test]
fn cross_file_aliases_use_complete_project_arguments_for_diagnostics_and_repairs() {
    for (arguments, precise) in [("{x}{y}&", true), ("{x}&", false)] {
        let directory = tempfile::tempdir().unwrap();
        let row = format!("\\begin{{tabular}}{{ll}}A & B\\bridge{arguments} C\\\\\\end{{tabular}}");
        let source = format!("\\input{{macros}}\n\\def\\bridge{{\\bad}}\n{row}\n");
        let prefix = &row[..row.rfind('&').unwrap() + 1];
        let log = format!("(./main.tex\n(./macros.tex)\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.3 {prefix}\n                 following source\n\n)\n");
        let outcome = build_fixture(directory.path(), &source, r"\def\bad#1#2{}", &log);
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let item = outcome.diagnostics[&uri][0].clone();
        assert_eq!(item.code.is_some(), precise, "{arguments}: {item:?}");
        let mut workspace = latex_lsp::workspace::Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        workspace.open(uri.clone(), source, 9);
        workspace
            .build_diagnostic_sources
            .insert(uri.clone(), outcome.diagnostic_sources);
        workspace
            .build_diagnostics
            .insert(uri.clone(), outcome.diagnostics);
        assert_eq!(
            selected_actions(&workspace, &uri, &item).len(),
            usize::from(precise)
        );
    }
}

#[test]
fn paused_parent_environment_parameters_do_not_blame_their_second_bare_argument() {
    let directory = tempfile::tempdir().unwrap();
    let row = r"\begin{tabular}{ll}A & B\begin{bad}x&\end{bad} C\\\end{tabular}";
    std::fs::write(directory.path().join("child.tex"), format!("{row}\n")).unwrap();
    let source = r"\newenvironment{bad}[2]{&}{}\input{child}";
    let log = format!("(./main.tex\n(./child.tex\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.1 {}\n                 following source\n\n)\n)\n", &row[..row.rfind('&').unwrap()+1]);
    let outcome = build_fixture(directory.path(), source, "", &log);
    let uri = Url::from_file_path(directory.path().join("child.tex")).unwrap();
    assert!(
        outcome.diagnostics[&uri][0].code.is_none(),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
fn completed_cross_file_macros_do_not_blame_their_second_bare_or_delimited_argument() {
    for (definition, call) in [
        (r"\def\bad#1#2{&}", r"\bad x&"),
        (r"\def\bad#1&{&}", r"\bad x&"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let row = format!("\\begin{{tabular}}{{ll}}A & B{call} C\\\\\\end{{tabular}}");
        let source = format!("\\input{{macros}}\n{row}\n");
        let prefix = &row[..row.rfind('&').unwrap() + 1];
        let log=format!("(./main.tex\n(./macros.tex)\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.2 {prefix}\n                 following source\n\n)\n");
        let outcome = build_fixture(root, &source, definition, &log);
        assert_eq!(outcome.status, "error");
        let items: Vec<_> = outcome
            .diagnostics
            .values()
            .flatten()
            .filter(|item| item.message.starts_with("Extra alignment tab"))
            .collect();
        assert_eq!(items.len(), 1, "{:?}", outcome.diagnostics);
        assert!(items[0].code.is_none(), "{definition}: {:?}", items[0]);
    }
}

fn selected_actions(
    workspace: &latex_lsp::workspace::Workspace,
    uri: &Url,
    diagnostic: &Diagnostic,
) -> CodeActionResponse {
    latex_lsp::editing::code_actions(
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
fn literal_fixes_record_loaded_scopes_and_recheck_macro_arities_before_legacy_execution() {
    use latex_lsp::{editing, workspace::Workspace};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\input{macros}\n\\begin{tabular}{ll}A & B\\bad x& C\\\\\\end{tabular}\n";
    let log="(./main.tex\n(./macros.tex)\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.2 \\begin{tabular}{ll}A & B\\bad x&\n                 following source\n\n)\n";
    let outcome = build_fixture(root, source, r"\def\bad{}", log);
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let macros = Url::from_file_path(root.join("macros.tex")).unwrap();
    let item = outcome.diagnostics[&main]
        .iter()
        .find(|item| item.code == Some(NumberOrString::String("extra-alignment-tab".into())))
        .unwrap()
        .clone();
    let data = item.data.as_ref().unwrap();
    let scope: std::collections::BTreeSet<Url> =
        serde_json::from_value(data["macroSources"].clone()).unwrap();
    assert_eq!(scope, [main.clone(), macros.clone()].into());
    let mut workspace = Workspace {
        code_action_literals: false,
        apply_edits: true,
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(main.clone(), source.into(), 9);
    workspace
        .build_diagnostic_sources
        .insert(main.clone(), outcome.diagnostic_sources);
    workspace
        .build_diagnostics
        .insert(main.clone(), outcome.diagnostics);
    let actions = selected_actions(&workspace, &main, &item);
    assert_eq!(actions.len(), 1, "{actions:?}");
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!()
    };
    let argument = command.arguments.as_ref().unwrap()[0].clone();
    let request = || serde_json::from_value::<editing::FixRequest>(argument.clone()).unwrap();
    assert!(editing::resolve_fix(&workspace, request()).is_some());
    std::fs::write(macros.to_file_path().unwrap(), r"\def\bad#1#2{&}").unwrap();
    workspace.reload(&macros);
    assert!(selected_actions(&workspace, &main, &item).is_empty());
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    std::fs::write(macros.to_file_path().unwrap(), r"\def\bad{}").unwrap();
    workspace.reload(&macros);
    assert_eq!(selected_actions(&workspace, &main, &item).len(), 1);
    let saved = workspace.build_diagnostics[&main][&main].clone();
    workspace
        .build_diagnostics
        .get_mut(&main)
        .unwrap()
        .get_mut(&main)
        .unwrap()[0]
        .data
        .as_mut()
        .unwrap()["macroSources"] = serde_json::json!([macros]);
    assert!(selected_actions(&workspace, &main, &item).is_empty());
    workspace
        .build_diagnostics
        .get_mut(&main)
        .unwrap()
        .insert(main.clone(), saved);
    workspace
        .build_diagnostic_sources
        .get_mut(&main)
        .unwrap()
        .remove(&macros);
    assert!(selected_actions(&workspace, &main, &item).is_empty());
}

#[test]
fn paused_parent_declarations_only_exclude_their_own_potential_macro_arguments() {
    for (call, precise) in [(r"\bad x&", false), (r"\good x&", true)] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let row = format!("\\begin{{tabular}}{{ll}}A & B{call} C\\\\\\end{{tabular}}");
        let child = format!("\\def\\good{{}}\n{row}\n");
        std::fs::write(root.join("child.tex"), &child).unwrap();
        let source = r"\def\bad#1#2{&}\input{child}";
        let log=format!("(./main.tex\n(./child.tex\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.2 {}\n                 following source\n\n)\n)\n",&row[..row.rfind('&').unwrap()+1]);
        let outcome = build_fixture(root, source, "", &log);
        let uri = Url::from_file_path(root.join("child.tex")).unwrap();
        let item = &outcome.diagnostics[&uri][0];
        assert_eq!(item.code.is_some(), precise, "{call}: {item:?}");
        if precise {
            assert_eq!(
                item.data.as_ref().unwrap()["macroSources"],
                serde_json::json!([uri])
            );
            assert_eq!(
                item.data.as_ref().unwrap()["macroPausedSources"],
                serde_json::json!([Url::from_file_path(root.join("main.tex")).unwrap()])
            );
        }
    }
}

#[test]
fn unloaded_future_sources_do_not_supply_literal_argument_signatures() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source =
        "\\def\\bad{}\n\\begin{tabular}{ll}A & B\\bad x& C\\\\\\end{tabular}\n\\input{macros}\n";
    let log="(./main.tex\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.2 \\begin{tabular}{ll}A & B\\bad x&\n                 following source\n\n(./macros.tex)\n)\n";
    let outcome = build_fixture(root, source, r"\def\bad#1#2{&}", log);
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let item = &outcome.diagnostics[&uri][0];
    assert_eq!(
        item.code,
        Some(NumberOrString::String("extra-alignment-tab".into()))
    );
    assert_eq!(
        item.data.as_ref().unwrap()["macroSources"],
        serde_json::json!([uri])
    );
}

#[test]
fn dynamic_or_changed_macro_sources_keep_literal_errors_at_line_scope() {
    for case in ["dynamic", "changed"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = if case == "dynamic" {
            "\\def\\path{macros}\\input{\\path}\n\\begin{tabular}{ll}A & B\\bad x& C\\\\\\end{tabular}\n"
        } else {
            "\\input{macros}\n\\begin{tabular}{ll}A & B\\bad x& C\\\\\\end{tabular}\n"
        };
        if case == "changed" {
            std::fs::write(root.join("replacement.tex"), r"\def\bad#1#2{&}").unwrap();
        }
        let log="(./main.tex\n(./macros.tex)\n! Extra alignment tab has been changed to \\cr.\n<recently read> \\endtemplate\n\nl.2 \\begin{tabular}{ll}A & B\\bad x&\n                 following source\n\n)\n";
        let outcome = build_fixture(root, source, r"\def\bad{}", log);
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        assert!(
            outcome.diagnostics[&uri][0].code.is_none(),
            "{case}: {:?}",
            outcome.diagnostics
        );
    }
}

fn real_engine_config(
    root: &std::path::Path,
    engine: &str,
    width: usize,
    numbered: bool,
) -> BuildConfig {
    let compiler = root.join("compiler");
    std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut arguments = vec![
        "-interaction=nonstopmode".into(),
        "-recorder".into(),
        "-output-directory=%o".into(),
        "%f".into(),
    ];
    if numbered {
        arguments.insert(0, "-file-line-error".into());
    }
    BuildConfig {
        executable: compiler.to_string_lossy().into(),
        arguments: Some(arguments),
        ..Default::default()
    }
}

#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX and LuaLaTeX"]
fn real_copied_commands_exclude_arguments_and_repair_following_literal_tabs() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let macros = root.join("macros.tex");
                let child = root.join("child.tex");
                let config = real_engine_config(root, engine, width, numbered);
                std::fs::write(&main,"\\documentclass{article}\n\\usepackage{etoolbox}\n\\input{macros}\n\\begin{document}\n\\input{child}\n\\end{document}\n").unwrap();
                for (definition, call) in [
                    (r"\def\consume#1#2{&}\let\bad\consume", r"\bad x&"),
                    (r"\def\consume#1&{&}\let\bad\consume", r"\bad x&"),
                    (
                        r"\def\consume#1#2{&}\NewCommandCopy{\bad}{\consume}",
                        r"\bad x&",
                    ),
                    (r"\def\consume#1#2{&}\cslet{bad}\consume", r"\bad x&"),
                    (r"\def\consume#1#2{&}\letcs\bad{consume}", r"\bad x&"),
                    (r"\def\consume#1#2{&}\csletcs{bad}{consume}", r"\bad x&"),
                    (
                        r"\def\consume#1#2{&}\let\bad\consume\def\consume{}",
                        r"\bad x&",
                    ),
                    (
                        r"\newenvironment{original}[2]{&}{}\NewEnvironmentCopy{bad}{original}",
                        r"\begin{bad}x&\end{bad}",
                    ),
                    (
                        r"\newenvironment{original}[2]{&}{}\let\bad\begin\def\begin{}\let\begin\bad",
                        r"\bad{original}x&\end{original}",
                    ),
                ] {
                    std::fs::write(&macros, definition).unwrap();
                    std::fs::write(
                        &child,
                        format!("\\begin{{tabular}}{{ll}}A & B{call} C\\\\\\end{{tabular}}\n"),
                    )
                    .unwrap();
                    let outcome = build::run(&main, &config, &Default::default());
                    assert_eq!(
                        outcome.status, "error",
                        "{engine}/{width}/{numbered}/{definition}: {}",
                        outcome.output
                    );
                    let errors: Vec<_> = outcome
                        .diagnostics
                        .values()
                        .flatten()
                        .filter(|item| item.message.starts_with("Extra alignment tab"))
                        .collect();
                    assert_eq!(
                        errors.len(),
                        1,
                        "{engine}/{width}/{numbered}/{definition}: {:?}",
                        outcome.diagnostics
                    );
                    assert!(errors[0].code.is_none(), "{definition}: {:?}", errors[0]);
                }
                for (definition, call) in [
                    (r"\def\consume#1#2{}\let\bad\consume", r"\bad{x}{y}&"),
                    (
                        r"\def\consume#1#2{}\NewCommandCopy{\bad}{\consume}",
                        r"\bad{x}{y}&",
                    ),
                    ("", r"\def\consume#1#2{}\let\bad\consume x&"),
                ] {
                    std::fs::write(&macros, definition).unwrap();
                    let source =
                        format!("\\begin{{tabular}}{{ll}}A & B{call} C\\\\\\end{{tabular}}\n");
                    std::fs::write(&child, &source).unwrap();
                    let outcome = build::run(&main, &config, &Default::default());
                    let uri = Url::from_file_path(&child).unwrap();
                    let item = outcome.diagnostics[&uri]
                        .iter()
                        .find(|item| {
                            item.code == Some(NumberOrString::String("extra-alignment-tab".into()))
                        })
                        .unwrap_or_else(|| {
                            panic!(
                                "{engine}/{width}/{numbered}/{call}: {:?}",
                                outcome.diagnostics
                            )
                        })
                        .clone();
                    let main_uri = Url::from_file_path(&main).unwrap();
                    let mut workspace = latex_lsp::workspace::Workspace {
                        code_action_literals: true,
                        versioned_edits: true,
                        ..Default::default()
                    };
                    workspace.add_root(root.into());
                    workspace.open(uri.clone(), source.clone(), 9);
                    workspace
                        .build_diagnostic_sources
                        .insert(main_uri.clone(), outcome.diagnostic_sources);
                    workspace
                        .build_diagnostics
                        .insert(main_uri, outcome.diagnostics);
                    let actions = selected_actions(&workspace, &uri, &item);
                    assert_eq!(actions.len(), 1, "{definition}/{call}");
                    let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                        panic!()
                    };
                    let Some(DocumentChanges::Edits(changes)) =
                        action.edit.as_ref().unwrap().document_changes.as_ref()
                    else {
                        panic!()
                    };
                    let OneOf::Left(edit) = &changes[0].edits[0] else {
                        panic!()
                    };
                    let text = latex_lsp::text::Text::new(source.clone());
                    let mut fixed = source;
                    fixed.replace_range(
                        text.offset(edit.range.start).unwrap()
                            ..text.offset(edit.range.end).unwrap(),
                        &edit.new_text,
                    );
                    std::fs::write(&child, fixed).unwrap();
                    let repaired = build::run(&main, &config, &Default::default());
                    assert_eq!(
                        repaired.status, "success",
                        "{engine}/{width}/{numbered}/{definition}/{call}: {}",
                        repaired.output
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX and LuaLaTeX"]
fn real_paused_environment_parameters_and_completed_redeclarations_keep_distinct_repairs() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let child = root.join("child.tex");
                let config = real_engine_config(root, engine, width, numbered);
                for (definition, call, precise) in [
                    (
                        r"\newenvironment{bad}[2][default]{&}{}",
                        r"\begin{bad}&\end{bad}",
                        false,
                    ),
                    (
                        r"\NewDocumentEnvironment{bad}{mm}{&}{}",
                        r"\begin{bad}x&\end{bad}",
                        false,
                    ),
                    (
                        r"\newenvironment{bad}[2][default]{}{}",
                        r"\begin{bad}{x}&\end{bad}",
                        true,
                    ),
                    (
                        r"\newenvironment{bad}[2]{&}{}",
                        r"\begin{bad}x&\end{bad}",
                        false,
                    ),
                    (
                        r"\newenvironment{bad}{}{}\renewenvironment{bad}[2]{&}{}",
                        r"\begin{bad}x&\end{bad}",
                        false,
                    ),
                    (
                        r"\newenvironment{bad}{}{}\def\bad#1#2{&}",
                        r"\begin{bad}x&\end{bad}",
                        false,
                    ),
                    (
                        r"\newenvironment{bad}[2]{&}{}\def\start{\begin}",
                        r"\start{bad}x&\end{bad}",
                        false,
                    ),
                    (
                        r"\newenvironment{bad}{}{}\renewenvironment{bad}{}{}",
                        r"\begin{bad}x&\end{bad}",
                        true,
                    ),
                    (
                        r"\newenvironment{bad}[2]{}{}\renewenvironment{bad}[2]{}{}",
                        r"\begin{bad}xy&\end{bad}",
                        true,
                    ),
                ] {
                    std::fs::write(&main, format!("\\documentclass{{article}}\n{definition}\n\\begin{{document}}\n\\input{{child}}\n\\end{{document}}\n")).unwrap();
                    let source =
                        format!("\\begin{{tabular}}{{ll}}A & B{call} C\\\\\\end{{tabular}}\n");
                    std::fs::write(&child, &source).unwrap();
                    let outcome = build::run(&main, &config, &Default::default());
                    assert_eq!(
                        outcome.status, "error",
                        "{engine}/{width}/{numbered}/{definition}: {}",
                        outcome.output
                    );
                    let uri = Url::from_file_path(&child).unwrap();
                    let items: Vec<_> = outcome.diagnostics[&uri]
                        .iter()
                        .filter(|item| item.message.starts_with("Extra alignment tab"))
                        .collect();
                    assert_eq!(items.len(), 1, "{definition}: {:?}", outcome.diagnostics);
                    assert_eq!(
                        items[0].code.is_some(),
                        precise,
                        "{engine}/{width}/{numbered}/{definition}: {:?}",
                        items[0]
                    );
                    if !precise {
                        continue;
                    }
                    let item = items[0].clone();
                    let main_uri = Url::from_file_path(&main).unwrap();
                    assert_eq!(
                        item.data.as_ref().unwrap()["macroPausedSources"],
                        serde_json::json!([main_uri])
                    );
                    let mut workspace = latex_lsp::workspace::Workspace {
                        code_action_literals: true,
                        versioned_edits: true,
                        ..Default::default()
                    };
                    workspace.add_root(root.into());
                    workspace.open(uri.clone(), source.clone(), 9);
                    workspace
                        .build_diagnostic_sources
                        .insert(main_uri.clone(), outcome.diagnostic_sources);
                    workspace
                        .build_diagnostics
                        .insert(main_uri, outcome.diagnostics);
                    let actions = selected_actions(&workspace, &uri, &item);
                    assert_eq!(actions.len(), 1, "{definition}: {actions:?}");
                    let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                        panic!()
                    };
                    let Some(DocumentChanges::Edits(changes)) =
                        action.edit.as_ref().unwrap().document_changes.as_ref()
                    else {
                        panic!()
                    };
                    let OneOf::Left(edit) = &changes[0].edits[0] else {
                        panic!()
                    };
                    assert_eq!(edit.new_text, r"\&");
                    let text = latex_lsp::text::Text::new(source.clone());
                    let mut fixed = source;
                    fixed.replace_range(
                        text.offset(edit.range.start).unwrap()
                            ..text.offset(edit.range.end).unwrap(),
                        &edit.new_text,
                    );
                    std::fs::write(&child, fixed).unwrap();
                    let repaired = build::run(&main, &config, &Default::default());
                    assert_eq!(
                        repaired.status, "success",
                        "{engine}/{width}/{numbered}/{definition}: {}",
                        repaired.output
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX and LuaLaTeX"]
fn real_cross_file_macro_arguments_are_excluded_and_literal_tabs_can_be_repaired() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let macros = root.join("macros.tex");
                let config = real_engine_config(root, engine, width, numbered);
                let source = "\\documentclass{article}\n\\input{macros}\n\\begin{document}\n\\begin{tabular}{ll}A & B\\bad x& C\\\\\\end{tabular}\n\\end{document}\n";
                std::fs::write(&main, source).unwrap();
                let uri = Url::from_file_path(&main).unwrap();
                for definition in [
                    r"\def\bad#1#2{&}",
                    r"\def\bad#1&{&}",
                    r"\def\bad{}\def\bad#1#2{&}",
                    r"\newcommand{\bad}{}\renewcommand{\bad}[2]{&}",
                    r"\def\other#1#2{&}\def\bad{}\def\bad{\other}",
                ] {
                    std::fs::write(&macros, definition).unwrap();
                    let outcome = build::run(&main, &config, &Default::default());
                    assert_eq!(
                        outcome.status, "error",
                        "{engine}/{width}/{numbered}: {}",
                        outcome.output
                    );
                    let items: Vec<_> = outcome
                        .diagnostics
                        .values()
                        .flatten()
                        .filter(|item| item.message.starts_with("Extra alignment tab"))
                        .collect();
                    assert_eq!(
                        items.len(),
                        1,
                        "{engine}/{width}/{numbered}: {:?}",
                        outcome.diagnostics
                    );
                    assert!(items[0].code.is_none(), "{definition}: {:?}", items[0]);
                }
                std::fs::write(&macros, r"\def\bad{}").unwrap();
                let outcome = build::run(&main, &config, &Default::default());
                assert_eq!(outcome.status, "error");
                let item = outcome.diagnostics[&uri]
                    .iter()
                    .find(|item| {
                        item.code == Some(NumberOrString::String("extra-alignment-tab".into()))
                    })
                    .unwrap()
                    .clone();
                let scope: std::collections::BTreeSet<Url> =
                    serde_json::from_value(item.data.as_ref().unwrap()["macroSources"].clone())
                        .unwrap();
                assert_eq!(
                    scope,
                    [uri.clone(), Url::from_file_path(&macros).unwrap()].into()
                );
                let mut workspace = latex_lsp::workspace::Workspace {
                    code_action_literals: true,
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace.open(uri.clone(), source.into(), 9);
                workspace
                    .build_diagnostic_sources
                    .insert(uri.clone(), outcome.diagnostic_sources);
                workspace
                    .build_diagnostics
                    .insert(uri.clone(), outcome.diagnostics);
                let actions = selected_actions(&workspace, &uri, &item);
                assert_eq!(actions.len(), 1);
                let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                    panic!()
                };
                let Some(DocumentChanges::Edits(changes)) =
                    action.edit.as_ref().unwrap().document_changes.as_ref()
                else {
                    panic!()
                };
                let OneOf::Left(edit) = &changes[0].edits[0] else {
                    panic!()
                };
                assert_eq!(edit.new_text, r"\&");
                let text = latex_lsp::text::Text::new(source.into());
                let mut fixed = source.to_owned();
                fixed.replace_range(
                    text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
                    &edit.new_text,
                );
                std::fs::write(&main, fixed).unwrap();
                let repaired = build::run(&main, &config, &Default::default());
                assert_eq!(
                    repaired.status, "success",
                    "{engine}/{width}/{numbered}: {}",
                    repaired.output
                );
            }
        }
    }
}
