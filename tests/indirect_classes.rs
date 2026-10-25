use latex_lsp::{
    build_log, editing, features, file_operations, navigation,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn call(form: &str, loader: &str) -> String {
    match form {
        "named" => format!("\\csuse{{{loader}}}"),
        "literal" => format!("\\csname {loader}\\endcsname"),
        "alias" => r"\loadclass".into(),
        "named-alias" => r"\csuse{loadclass}".into(),
        "literal-alias" => r"\csname loadclass\endcsname".into(),
        _ => panic!(),
    }
}
fn apply(workspace: &Workspace, uri: &Url, edit: &WorkspaceEdit) -> String {
    let Some(DocumentChanges::Edits(changes)) = &edit.document_changes else {
        panic!()
    };
    let change = changes
        .iter()
        .find(|c| &c.text_document.uri == uri)
        .unwrap();
    assert_eq!(
        change.text_document.version,
        workspace.documents[uri].version
    );
    let mut edits: Vec<_> = change
        .edits
        .iter()
        .map(|e| match e {
            OneOf::Left(e) => e.clone(),
            _ => panic!(),
        })
        .collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));
    workspace.documents[uri]
        .text
        .changed(
            edits
                .into_iter()
                .map(|e| TextDocumentContentChangeEvent {
                    range: Some(e.range),
                    range_length: None,
                    text: e.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
}
fn actions(workspace: &Workspace, uri: &Url, diagnostic: Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic],
            ..Default::default()
        },
    )
}

#[test]
fn indirect_class_errors_repair_literal_names_preserving_options_comments_and_versions() {
    for loader in ["documentclass", "LoadClass", "LoadClassWithOptions"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            for (argument, expected, target, report, version) in [
                ("{fixtuer}", "{fixture}", "fixture", "fixtuer.cls", false),
                (
                    "{% head 😀\r\nfixtuer% keep\r\n}",
                    "{% head 😀\r\nfixture% keep\r\n}",
                    "fixture",
                    "fixtuer.cls",
                    false,
                ),
                (
                    "{fixtuer}[2000/01/01]",
                    "{fixture}[2000/01/01]",
                    "fixture",
                    "fixtuer.cls",
                    true,
                ),
                (
                    "{my clss,name}",
                    "{my class,name}",
                    "my class,name",
                    "my clss,name.cls",
                    false,
                ),
                (
                    "{fixtuer.cls}",
                    "{fixture.cls}",
                    "fixture.cls",
                    "fixtuer.cls.cls",
                    false,
                ),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let uri = Url::from_file_path(&main).unwrap();
                let options = if loader == "LoadClassWithOptions" {
                    ""
                } else {
                    "[draft]"
                };
                let head = format!("\\def\\loadclass{{\\{loader}}}\n");
                let source = format!("{head}{}{options}{argument}\\relax\n", call(form, loader));
                std::fs::write(&main, &source).unwrap();
                std::fs::write(root.join(format!("{target}.cls")), "Class").unwrap();
                let line = source.lines().count();
                let echo = source.lines().last().unwrap();
                let echo = if version {
                    echo.strip_suffix(r"\relax").unwrap()
                } else {
                    echo
                };
                let log=format!("(./main.tex\n! LaTeX Error: File `{report}' not found.\n! Emergency stop.\n<read *>\nl.{line} {echo}\n");
                let diagnostics = build_log::parse(&main, &log);
                let diagnostic = diagnostics[&uri]
                    .iter()
                    .find(|d| d.message.contains("not found"))
                    .unwrap()
                    .clone();
                assert_eq!(
                    diagnostic.code,
                    Some(NumberOrString::String("missing-class".into())),
                    "{loader}/{form}/{argument}: {diagnostic:?}"
                );
                assert_eq!(diagnostic.data.as_ref().unwrap()["loaderCommand"], loader);
                let mut workspace = Workspace {
                    code_action_literals: true,
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace
                    .documents
                    .insert(uri.clone(), Document::new(&uri, source, Some(7)));
                workspace.build_diagnostics.insert(uri.clone(), diagnostics);
                let choices = actions(&workspace, &uri, diagnostic);
                assert_eq!(choices.len(), 1, "{loader}/{form}/{argument}: {choices:?}");
                let CodeActionOrCommand::CodeAction(action) = &choices[0] else {
                    panic!()
                };
                assert_eq!(
                    apply(&workspace, &uri, action.edit.as_ref().unwrap()),
                    format!("{head}{}{options}{expected}\\relax\n", call(form, loader))
                );
            }
        }
    }
}

#[test]
fn class_diagnostics_do_not_infer_dynamic_names_macro_operands_or_deferred_calls() {
    for body in [
        r"\newcommand{\later}{\csuse{documentclass}{fixtuer}\relax}",
        r"\newcommand{\hold}[1]{}\hold\loadclass{fixtuer}\relax",
        r"\def\documentclass#1{}\loadclass{fixtuer}\relax",
        r"\csuse{documentclass}{\filename}\relax",
        r"\csname \loader\endcsname{fixtuer}\relax",
        r"\endinput\loadclass{fixtuer}\relax",
        r"\loadclass{fixtuer}[broken",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let uri = Url::from_file_path(&main).unwrap();
        std::fs::write(
            &main,
            format!("\\def\\loadclass{{\\documentclass}}\n{body}\n"),
        )
        .unwrap();
        let log=format!("(./main.tex\n! LaTeX Error: File `fixtuer.cls' not found.\n! Emergency stop.\n<read *>\nl.2 {body}\n");
        let diagnostics = build_log::parse(&main, &log);
        let diagnostic = diagnostics[&uri]
            .iter()
            .find(|d| d.message.contains("not found"))
            .unwrap();
        assert!(diagnostic.code.is_none(), "{body}: {diagnostic:?}");
    }
}

#[test]
fn class_dependencies_links_completion_and_rename_preserve_single_filename_semantics() {
    for loader in ["documentclass", "LoadClass", "LoadClassWithOptions"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let uri = Url::from_file_path(&main).unwrap();
            let class = Url::from_file_path(root.join("my class,name.cls")).unwrap();
            let part = Url::from_file_path(root.join("part.tex")).unwrap();
            let options = if loader == "LoadClassWithOptions" {
                ""
            } else {
                "[draft]"
            };
            let source=format!("\\def\\loadclass{{\\{loader}}}\n{}{options}{{% head 😀\r\nmy class,name% keep\r\n}}[2000/01/01]\\relax\n\\readpart{{part}}",call(form,loader));
            std::fs::write(&main, &source).unwrap();
            std::fs::write(class.to_file_path().unwrap(), r"\def\readpart{\input}").unwrap();
            std::fs::write(part.to_file_path().unwrap(), "Part").unwrap();
            let mut workspace = Workspace {
                versioned_edits: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.load_dependencies(&uri, 0);
            for doc in workspace.documents.values_mut() {
                doc.version = Some(7);
            }
            let documents = workspace.project_documents(&uri);
            assert!(documents.contains(&class), "{loader}/{form}: {documents:?}");
            assert!(documents.contains(&part));
            let links = navigation::document_links(&workspace, &uri);
            let link = links
                .iter()
                .find(|link| link.target.as_ref() == Some(&class))
                .unwrap();
            assert_eq!(
                features::definitions(&workspace, &uri, link.range.start)[0].uri,
                class
            );
            let rename = |name: &str| FileRename {
                old_uri: class.clone().into(),
                new_uri: Url::from_file_path(root.join(name)).unwrap().into(),
            };
            let edit = file_operations::will_rename(&workspace, &[rename("new class,name.cls")])
                .unwrap()
                .unwrap();
            assert_eq!(
                apply(&workspace, &uri, &edit),
                source.replace("my class,name", "new class,name")
            );
            assert!(file_operations::will_rename(&workspace, &[rename("renamed.sty")]).is_err());
            let marked = source.replace("my class,name", "my cl|ass,name");
            let at = marked.find('|').unwrap();
            workspace.documents.insert(
                uri.clone(),
                Document::new(&uri, marked.replace('|', ""), Some(8)),
            );
            let candidates = features::completion(
                &workspace,
                &uri,
                workspace.documents[&uri].text.position(at),
            );
            let item = candidates
                .iter()
                .find(|item| item.label == "my class,name.cls")
                .unwrap_or_else(|| panic!("{loader}/{form}: {candidates:?}"));
            let Some(CompletionTextEdit::Edit(main)) = &item.text_edit else {
                panic!()
            };
            let mut edits = item.additional_text_edits.clone().unwrap_or_default();
            edits.push(main.clone());
            edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));
            let updated = workspace.documents[&uri]
                .text
                .changed(
                    edits
                        .into_iter()
                        .map(|e| TextDocumentContentChangeEvent {
                            range: Some(e.range),
                            range_length: None,
                            text: e.new_text,
                        })
                        .collect(),
                )
                .unwrap();
            assert_eq!(updated.source, source);
        }
    }
}

#[test]
fn legacy_class_repairs_revalidate_alias_definitions_without_parent_version_changes() {
    for form in ["alias", "named-alias", "literal-alias"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let uri = Url::from_file_path(&main).unwrap();
        let package = Url::from_file_path(root.join("definitions.sty")).unwrap();
        let source = format!(
            "\\RequirePackage{{definitions}}\n{}{{fixtuer}}\\relax\n",
            call(form, "documentclass")
        );
        std::fs::write(&main, &source).unwrap();
        std::fs::write(
            package.to_file_path().unwrap(),
            r"\def\loadclass{\documentclass}",
        )
        .unwrap();
        std::fs::write(root.join("fixture.cls"), "Class").unwrap();
        let mut workspace = Workspace {
            apply_edits: true,
            versioned_edits: true,
            ..Default::default()
        };
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(7)));
        workspace.documents.insert(
            package.clone(),
            Document::new(&package, r"\def\loadclass{\documentclass}".into(), Some(3)),
        );
        let at = source.rfind("fixtuer").unwrap();
        let diagnostic = Diagnostic {
            range: workspace.documents[&uri]
                .text
                .range(at, at + "fixtuer".len()),
            source: Some("latex-build".into()),
            code: Some(NumberOrString::String("missing-class".into())),
            message: "LaTeX Error: File `fixtuer.cls' not found.".into(),
            data: Some(
                serde_json::json!({"file":"fixtuer.cls","loaderCommand":"documentclass","macroSources":[uri,package]}),
            ),
            ..Default::default()
        };
        workspace.build_diagnostics.insert(
            uri.clone(),
            [(uri.clone(), vec![diagnostic.clone()])].into(),
        );
        let choices = actions(&workspace, &uri, diagnostic);
        assert_eq!(choices.len(), 1, "{form}: {choices:?}");
        let CodeActionOrCommand::Command(action) = &choices[0] else {
            panic!()
        };
        let request = action.arguments.as_ref().unwrap()[0].clone();
        let resolve = |workspace: &Workspace| {
            editing::resolve_fix(workspace, serde_json::from_value(request.clone()).unwrap())
        };
        assert!(resolve(&workspace).is_some());
        for replacement in [
            r"\def\loadclass#1{}",
            r"\def\loadclass{\LoadClass}",
            r"\def\documentclass#1{}\def\loadclass{\documentclass}",
        ] {
            workspace.documents.insert(
                package.clone(),
                Document::new(&package, replacement.into(), Some(4)),
            );
            assert_eq!(workspace.documents[&uri].version, Some(7));
            assert!(resolve(&workspace).is_none(), "{form}: {replacement}");
        }
    }
}

#[test]
fn uncertain_class_calls_do_not_introduce_definitions_or_file_rename_edits() {
    for body in [
        r"\newcommand{\later}{\csuse{documentclass}{fixture}}",
        r"\newcommand{\hold}[1]{}\hold\csuse{documentclass}{fixture}",
        r"\csuse{documentclass}{\filename}",
        r"\def\documentclass#1{}\csuse{documentclass}{fixture}",
        r"\loadclass{fixture}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let uri = Url::from_file_path(&main).unwrap();
        let class = Url::from_file_path(root.join("fixture.cls")).unwrap();
        std::fs::write(
            &main,
            format!("\\def\\loadclass{{\\documentclass}}\n{body}"),
        )
        .unwrap();
        std::fs::write(class.to_file_path().unwrap(), r"\def\loadclass#1{}").unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        assert!(
            !workspace.project_documents(&uri).contains(&class),
            "{body}"
        );
        assert!(
            !navigation::document_links(&workspace, &uri)
                .iter()
                .any(|link| link.target.as_ref() == Some(&class)),
            "{body}"
        );
        let rename = FileRename {
            old_uri: class.into(),
            new_uri: Url::from_file_path(root.join("renamed.cls"))
                .unwrap()
                .into(),
        };
        assert!(
            file_operations::will_rename(&workspace, &[rename])
                .unwrap()
                .is_none(),
            "{body}"
        );
    }
}
