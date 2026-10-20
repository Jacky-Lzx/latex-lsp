use latex_lsp::{
    build_log, editing,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn call(form: &str, loader: &str) -> String {
    match form {
        "named" => format!("\\csuse{{{loader}}}"),
        "literal" => format!("\\csname {loader}\\endcsname"),
        "alias" => r"\loadpkg".into(),
        "named-alias" => r"\csuse{loadpkg}".into(),
        "literal-alias" => r"\csname loadpkg\endcsname".into(),
        _ => panic!(),
    }
}

#[test]
fn missing_indirect_packages_preserve_options_lists_comments_and_versions_in_fixes() {
    for loader in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            for (argument, expected, target, report, version) in [
                ("{fixtuer}", "{fixture}", "fixture", "fixtuer.sty", false),
                (
                    "{relax,fixtuer}",
                    "{relax,fixture}",
                    "fixture",
                    "fixtuer.sty",
                    false,
                ),
                (
                    "{% keep 😀\r\nrelax,fixtuer% keep\r\n}",
                    "{% keep 😀\r\nrelax,fixture% keep\r\n}",
                    "fixture",
                    "fixtuer.sty",
                    false,
                ),
                (
                    "{fixtuer}[2000/01/01]",
                    "{fixture}[2000/01/01]",
                    "fixture",
                    "fixtuer.sty",
                    true,
                ),
                (
                    "{sub/fixtuer}",
                    "{sub/fixture}",
                    "sub/fixture",
                    "sub/fixtuer.sty",
                    false,
                ),
                ("{cafe}", "{café}", "café", "cafe.sty", false),
                (
                    "{fixtuer.sty}",
                    "{fixture.sty}",
                    "fixture.sty",
                    "fixtuer.sty.sty",
                    false,
                ),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let uri = Url::from_file_path(&main).unwrap();
                let options = if loader == "RequirePackageWithOptions" {
                    ""
                } else {
                    "[draft]"
                };
                let header = format!("\\def\\loadpkg{{\\{loader}}}\n");
                let source = format!("{header}{}{options}{argument}\\relax\n", call(form, loader));
                std::fs::write(&main, &source).unwrap();
                let package = directory.path().join(format!("{target}.sty"));
                std::fs::create_dir_all(package.parent().unwrap()).unwrap();
                std::fs::write(&package, "Package").unwrap();
                let line = source.lines().count();
                let echo = source.lines().last().unwrap();
                let echo = if version {
                    echo.strip_suffix(r"\relax").unwrap()
                } else {
                    echo
                };
                let log = format!("(./main.tex\n! LaTeX Error: File `{report}' not found.\n! Emergency stop.\n<read *>\nl.{line} {echo}\n");
                let diagnostics = build_log::parse(&main, &log);
                let diagnostic = diagnostics[&uri]
                    .iter()
                    .find(|d| d.message.contains("not found"))
                    .unwrap();
                assert_eq!(
                    diagnostic.code,
                    Some(NumberOrString::String("missing-package".into())),
                    "{form}/{loader}/{argument}: {diagnostic:?}"
                );
                assert_eq!(diagnostic.data.as_ref().unwrap()["loaderCommand"], loader);
                assert_eq!(diagnostic.data.as_ref().unwrap()["file"], report);
                let mut workspace = Workspace {
                    code_action_literals: true,
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace
                    .documents
                    .insert(uri.clone(), Document::new(&uri, source.clone(), Some(7)));
                let diagnostic = diagnostic.clone();
                workspace.build_diagnostics.insert(uri.clone(), diagnostics);
                let actions = editing::code_actions(
                    &workspace,
                    &uri,
                    diagnostic.range,
                    &CodeActionContext {
                        diagnostics: vec![diagnostic],
                        ..Default::default()
                    },
                );
                assert_eq!(actions.len(), 1, "{form}/{loader}/{argument}: {actions:?}");
                let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                    panic!()
                };
                let DocumentChanges::Edits(changes) = action
                    .edit
                    .as_ref()
                    .unwrap()
                    .document_changes
                    .as_ref()
                    .unwrap()
                else {
                    panic!()
                };
                assert_eq!(changes.len(), 1);
                assert_eq!(changes[0].text_document.version, Some(7));
                let OneOf::Left(edit) = &changes[0].edits[0] else {
                    panic!()
                };
                let changed = Text::new(source)
                    .changed(vec![TextDocumentContentChangeEvent {
                        range: Some(edit.range),
                        range_length: None,
                        text: edit.new_text.clone(),
                    }])
                    .unwrap();
                assert_eq!(
                    changed.source,
                    format!("{header}{}{options}{expected}\\relax\n", call(form, loader)),
                    "{form}/{loader}/{argument}"
                );
            }
        }
    }
}

#[test]
fn indirect_package_diagnostics_reject_macro_data_dynamic_lists_and_redefined_loaders() {
    for source in [
        r"\newcommand{\later}{\csuse{usepackage}{fixtuer}\relax}",
        r"\newcommand{\hold}[1]{}\hold\loadpkg{fixtuer}\relax",
        r"\renewcommand{\usepackage}[1]{}\loadpkg{fixtuer}\relax",
        r"\renewcommand{\csuse}[1]{}\csuse{usepackage}{fixtuer}\relax",
        r"\csuse{usepackage}{\filename}\relax",
        r"\csuse{usepackage}{fixtuer,\filename}\relax",
        r"\csname \loader\endcsname{fixtuer}\relax",
        r"\endinput\loadpkg{fixtuer}\relax",
        r"\loadpkg{fixtuer}\relax\loadpkg{fixtuer}\relax",
        r"\loadpkg{fixtuer}[broken",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let uri = Url::from_file_path(&main).unwrap();
        std::fs::write(&main, format!("\\def\\loadpkg{{\\usepackage}}\n{source}\n")).unwrap();
        let echo = if source.ends_with(r"\relax\loadpkg{fixtuer}\relax") {
            r"...\loadpkg{fixtuer}\relax"
        } else {
            source
        };
        let log=format!("(./main.tex\n! LaTeX Error: File `fixtuer.sty' not found.\n! Emergency stop.\n<read *>\nl.2 {echo}\n");
        let diagnostics = build_log::parse(&main, &log);
        let diagnostic = diagnostics[&uri]
            .iter()
            .find(|d| d.message.contains("not found"))
            .unwrap();
        assert!(diagnostic.code.is_none(), "{source}: {diagnostic:?}");
    }
}

#[test]
fn legacy_package_repairs_recheck_loaded_aliases_and_the_selected_project() {
    for form in ["alias", "named-alias", "literal-alias"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let definitions = Url::from_file_path(root.join("definitions.sty")).unwrap();
        let source = format!(
            "\\usepackage{{definitions}}\n{}{{fixtuer}}\\relax\n",
            call(form, "usepackage")
        );
        std::fs::write(root.join("main.tex"), &source).unwrap();
        std::fs::write(root.join("definitions.sty"), r"\def\loadpkg{\usepackage}").unwrap();
        std::fs::write(root.join("fixture.sty"), "Package").unwrap();
        let mut workspace = Workspace {
            apply_edits: true,
            versioned_edits: true,
            ..Default::default()
        };
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(7)));
        workspace.documents.insert(
            definitions.clone(),
            Document::new(&definitions, r"\def\loadpkg{\usepackage}".into(), Some(3)),
        );
        let at = source.rfind("fixtuer").unwrap();
        let diagnostic = Diagnostic {
            range: workspace.documents[&uri]
                .text
                .range(at, at + "fixtuer".len()),
            source: Some("latex-build".into()),
            code: Some(NumberOrString::String("missing-package".into())),
            message: "LaTeX Error: File `fixtuer.sty' not found.".into(),
            data: Some(
                serde_json::json!({"file":"fixtuer.sty","loaderCommand":"usepackage","macroSources":[uri,definitions]}),
            ),
            ..Default::default()
        };
        workspace.build_diagnostics.insert(
            uri.clone(),
            [(uri.clone(), vec![diagnostic.clone()])].into(),
        );
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                ..Default::default()
            },
        );
        assert_eq!(actions.len(), 1, "{form}: {actions:?}");
        let CodeActionOrCommand::Command(action) = &actions[0] else {
            panic!()
        };
        let request = action.arguments.as_ref().unwrap()[0].clone();
        let resolve = |workspace: &Workspace| {
            editing::resolve_fix(workspace, serde_json::from_value(request.clone()).unwrap())
        };
        assert!(resolve(&workspace).is_some());
        for replacement in [
            r"\def\loadpkg#1{}",
            r"\def\loadpkg{\RequirePackage}",
            r"\def\usepackage#1{}\def\loadpkg{\usepackage}",
        ] {
            workspace.documents.insert(
                definitions.clone(),
                Document::new(&definitions, replacement.into(), Some(4)),
            );
            assert_eq!(workspace.documents[&uri].version, Some(7));
            assert!(resolve(&workspace).is_none(), "{form}: {replacement}");
        }
        workspace.documents.insert(
            definitions.clone(),
            Document::new(&definitions, r"\def\loadpkg{\usepackage}".into(), Some(5)),
        );
        assert!(resolve(&workspace).is_some());
        workspace.documents.insert(
            uri.clone(),
            Document::new(
                &uri,
                source.replace(r"\usepackage{definitions}", ""),
                Some(7),
            ),
        );
        assert!(
            resolve(&workspace).is_none(),
            "removed package still certifies its alias"
        );
    }
}

#[test]
fn static_package_dependencies_discover_local_definitions_before_any_build() {
    for loader in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let uri = Url::from_file_path(root.join("main.tex")).unwrap();
            let package = Url::from_file_path(root.join("definitions.sty")).unwrap();
            let second = Url::from_file_path(root.join("second.sty")).unwrap();
            let part = Url::from_file_path(root.join("part.tex")).unwrap();
            let options = if loader == "RequirePackageWithOptions" {
                ""
            } else {
                "[draft]"
            };
            let source=format!("\\documentclass{{article}}\\def\\loadpkg{{\\{loader}}}\n{}{options}{{% keep\n definitions, second% trailing\n}}\n\\readpart{{part}}",call(form,loader));
            for (name, text) in [
                ("main.tex", source.as_str()),
                ("definitions.sty", r"\def\readpart{\input}"),
                ("second.sty", r"\newcommand{\marker}{OK}"),
                ("part.tex", "Part"),
            ] {
                std::fs::write(root.join(name), text).unwrap();
            }
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let documents = workspace.project_documents(&uri);
            assert!(
                documents.contains(&package),
                "{loader}/{form}: {documents:?}"
            );
            assert!(
                documents.contains(&second),
                "{loader}/{form}: {documents:?}"
            );
            assert!(documents.contains(&part), "{loader}/{form}: {documents:?}");
        }
    }
}

#[test]
fn uncertain_indirect_package_loads_do_not_introduce_project_definitions() {
    for source in [
        r"\newcommand{\later}{\csuse{usepackage}{local}}",
        r"\newcommand{\hold}[1]{}\hold\csuse{usepackage}{local}",
        r"\csuse{usepackage}{local,\filename}",
        r"\renewcommand{\usepackage}[1]{}\csuse{usepackage}{local}",
        r"\endinput\csuse{usepackage}{local}",
        r"\def\loadpkg{\usepackage}\loadpkg{local}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let package = Url::from_file_path(root.join("local.sty")).unwrap();
        std::fs::write(root.join("main.tex"), source).unwrap();
        // The last case invalidates the call that discovered this very file.
        std::fs::write(root.join("local.sty"), r"\def\loadpkg#1{}").unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        assert!(
            !workspace.project_documents(&uri).contains(&package),
            "{source}"
        );
    }
}
