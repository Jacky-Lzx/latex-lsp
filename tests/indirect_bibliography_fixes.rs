use latex_lsp::{
    editing,
    text::Text,
    workspace::{Document, Workspace},
};
use std::path::Path;
use tower_lsp::lsp_types::*;
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn actions(workspace: &Workspace, document: &Url, diagnostic: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        document,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            ..Default::default()
        },
    )
}
fn edits(action: &CodeActionOrCommand, document: &Url, version: i32) -> Vec<TextEdit> {
    let CodeActionOrCommand::CodeAction(action) = action else {
        panic!("missing literal action")
    };
    let DocumentChanges::Edits(changes) = action
        .edit
        .as_ref()
        .unwrap()
        .document_changes
        .as_ref()
        .unwrap()
    else {
        panic!("missing versioned changes")
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].text_document.uri, *document);
    assert_eq!(changes[0].text_document.version, Some(version));
    changes[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            OneOf::Right(_) => panic!("unexpected annotation"),
        })
        .collect()
}
fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
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
        .source
}

fn call(form: &str, command: &str) -> String {
    match form {
        "named" => format!("\\csuse{{{command}}}"),
        "literal" => format!("\\csname {command}\\endcsname"),
        "alias" => r"\loadbib".into(),
        "named-alias" => r"\csuse{loadbib}".into(),
        "literal-alias" => r"\csname loadbib\endcsname".into(),
        _ => panic!(),
    }
}
fn fixture(
    root: &Path,
    command: &str,
    source: &str,
    spelling: &str,
) -> (Workspace, Url, Diagnostic) {
    let main = uri(root, "main.tex");
    let provider = uri(root, "definitions.sty");
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(
        root.join("definitions.sty"),
        format!("\\def\\loadbib{{\\{command}}}"),
    )
    .unwrap();
    std::fs::write(root.join("refs.bib"), "@book{fixture,title={Fixture}}").unwrap();
    std::fs::write(root.join("local,style.bst"), "").unwrap();
    let mut workspace = Workspace {
        code_action_literals: true,
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for doc in workspace.documents.values_mut() {
        doc.version = Some(7);
    }
    let style = command == "bibliographystyle";
    let file = if style {
        "local,styel.bst"
    } else {
        "reffs.bib"
    };
    let start = source.rfind(spelling).unwrap();
    let mut data = serde_json::json!({"file":file,"macroSources":[main,provider]});
    if !style {
        data["bibliographyCommand"] = serde_json::json!(command);
    }
    let diagnostic = Diagnostic {
        range: Text::new(source.into()).range(start, start + spelling.len()),
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(
            if style {
                "missing-bibliography-style"
            } else {
                "missing-bibliography-database"
            }
            .into(),
        )),
        source: Some("latex-build".into()),
        message: format!(
            "BibTeX: I couldn't open {} file {file}",
            if style { "style" } else { "database" }
        ),
        data: Some(data),
        ..Default::default()
    };
    let scope = std::collections::BTreeSet::from([main.clone(), provider]);
    workspace
        .build_diagnostic_sources
        .insert(main.clone(), scope);
    workspace.build_diagnostics.insert(
        main.clone(),
        std::collections::BTreeMap::from([(main.clone(), vec![diagnostic.clone()])]),
    );
    (workspace, main, diagnostic)
}

#[test]
fn indirect_bibliography_spelling_fixes_preserve_forms_comments_lists_and_suffixes() {
    for command in ["bibliography", "nobibliography", "bibliographystyle"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            for suffix in if command == "bibliographystyle" {
                vec![""]
            } else {
                vec!["", ".bib"]
            } {
                let directory = tempfile::tempdir().unwrap();
                let spelling = if command == "bibliographystyle" {
                    "local,styel".to_string()
                } else {
                    format!("re ffs{suffix}")
                };
                let head = if command == "bibliographystyle" {
                    ""
                } else {
                    "other,"
                };
                let source=format!("% 😀\r\n\\documentclass{{article}}\\RequirePackage{{definitions}}\n{}{{{head}% before\r\n{spelling}% tail\r\n}}",call(form,command));
                let (workspace, main, diagnostic) =
                    fixture(directory.path(), command, &source, &spelling);
                let found = actions(&workspace, &main, &diagnostic);
                assert_eq!(found.len(), 1, "{command}/{form}/{suffix}: {found:?}");
                let expected = if command == "bibliographystyle" {
                    "local,style".into()
                } else {
                    format!("refs{suffix}")
                };
                assert_eq!(
                    apply(&source, edits(&found[0], &main, 7)),
                    source.replace(&spelling, &expected)
                );
                assert_eq!(
                    std::fs::read_to_string(directory.path().join("main.tex")).unwrap(),
                    source
                );
            }
        }
    }
}

#[test]
fn changed_alias_providers_and_redefined_bibliography_builtins_withhold_old_fixes() {
    for command in ["bibliography", "nobibliography", "bibliographystyle"] {
        for form in ["alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let spelling = if command == "bibliographystyle" {
                "local,styel"
            } else {
                "reffs"
            };
            let source = format!(
                "\\documentclass{{article}}\\RequirePackage{{definitions}}{}{{{spelling}}}",
                call(form, command)
            );
            let (mut workspace, main, diagnostic) =
                fixture(directory.path(), command, &source, spelling);
            assert_eq!(actions(&workspace, &main, &diagnostic).len(), 1);
            let provider = uri(directory.path(), "definitions.sty");
            for declarations in [
                r"\newcommand{\loadbib}[1]{Text}".into(),
                format!("\\def\\loadbib{{\\{command}}}\\renewcommand{{\\{command}}}[1]{{}}"),
                String::new(),
            ] {
                workspace.documents.insert(
                    provider.clone(),
                    Document::new(&provider, declarations, Some(8)),
                );
                assert!(
                    actions(&workspace, &main, &diagnostic).is_empty(),
                    "{command}/{form}"
                );
            }
        }
    }
}

#[test]
fn malformed_unconfirmed_or_outside_project_bibliography_scopes_cannot_offer_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let source = r"\documentclass{article}\RequirePackage{definitions}\loadbib{reffs}";
    let (mut workspace, main, diagnostic) =
        fixture(directory.path(), "bibliography", source, "reffs");
    assert_eq!(actions(&workspace, &main, &diagnostic).len(), 1);
    for scope in [
        serde_json::json!("invalid"),
        serde_json::json!([]),
        serde_json::json!([main, uri(directory.path(), "unread.sty")]),
    ] {
        let mut forged = diagnostic.clone();
        forged.data.as_mut().unwrap()["macroSources"] = scope;
        workspace
            .build_diagnostics
            .get_mut(&main)
            .unwrap()
            .insert(main.clone(), vec![forged.clone()]);
        assert!(actions(&workspace, &main, &forged).is_empty());
    }
    workspace
        .build_diagnostics
        .get_mut(&main)
        .unwrap()
        .insert(main.clone(), vec![diagnostic.clone()]);
    workspace.build_diagnostic_sources.insert(
        main.clone(),
        std::collections::BTreeSet::from([main.clone()]),
    );
    assert!(actions(&workspace, &main, &diagnostic).is_empty());
}

#[test]
fn indirect_bibliography_repairs_require_an_active_call_in_the_current_project() {
    for body in [
        r"\newcommand{\hold}[1]{}\hold{\loadbib{reffs}}",
        r"\newcommand{\later}{\loadbib{reffs}}",
        "\\endinput\n\\loadbib{reffs}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("\\documentclass{{article}}\\RequirePackage{{definitions}}{body}");
        let (workspace, main, diagnostic) =
            fixture(directory.path(), "bibliography", &source, "reffs");
        assert!(actions(&workspace, &main, &diagnostic).is_empty(), "{body}");
    }
}
