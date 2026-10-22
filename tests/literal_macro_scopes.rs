#![cfg(unix)]
use latex_lsp::{build, config::BuildConfig};
use std::os::unix::fs::PermissionsExt;
use tower_lsp::lsp_types::*;

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
                let compiler = root.join("compiler");
                std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
                let mut arguments = vec![
                    "-interaction=nonstopmode".into(),
                    "-recorder".into(),
                    "-output-directory=%o".into(),
                    "%f".into(),
                ];
                if numbered {
                    arguments.insert(0, "-file-line-error".into());
                }
                let config = BuildConfig {
                    executable: compiler.to_string_lossy().into(),
                    arguments: Some(arguments),
                    ..Default::default()
                };
                let source = "\\documentclass{article}\n\\input{macros}\n\\begin{document}\n\\begin{tabular}{ll}A & B\\bad x& C\\\\\\end{tabular}\n\\end{document}\n";
                std::fs::write(&main, source).unwrap();
                let uri = Url::from_file_path(&main).unwrap();
                for definition in [r"\def\bad#1#2{&}", r"\def\bad#1&{&}"] {
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
