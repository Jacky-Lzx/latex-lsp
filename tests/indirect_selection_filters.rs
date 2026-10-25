use latex_lsp::{
    editing,
    workspace::{Document, Workspace},
};
use lsp_types::*;
use std::path::Path;

fn selector(form: &str) -> &str {
    match form {
        "direct" => r"\includeonly",
        "named" => r"\csuse{includeonly}",
        "literal" => r"\csname includeonly\endcsname",
        "alias" => r"\choose",
        "named-alias" => r"\csuse{choose}",
        "literal-alias" => r"\csname choose\endcsname",
        _ => panic!(),
    }
}

fn fixture(root: &Path, source: &str, literal: bool) -> (Workspace, Url, Diagnostic) {
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("definitions.sty"), r"\def\choose{\includeonly}").unwrap();
    std::fs::write(root.join("chapter.tex"), "Chapter").unwrap();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace {
        apply_edits: true,
        code_action_literals: literal,
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace
        .documents
        .insert(main.clone(), Document::new(&main, source.into(), Some(7)));
    let at = source.rfind("chapte").unwrap();
    let diagnostic = Diagnostic {
        range: workspace.documents[&main]
            .text
            .range(at, at + "chapte".len()),
        source: Some("latex-build".into()),
        code: Some(NumberOrString::String("missing-include".into())),
        message: "Included file `chapte.tex` not found".into(),
        data: Some(serde_json::json!({"file":"chapte.tex"})),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        main.clone(),
        [(main.clone(), vec![diagnostic.clone()])].into(),
    );
    (workspace, main, diagnostic)
}

fn fixes(workspace: &Workspace, main: &Url, diagnostic: &Diagnostic) -> Vec<CodeActionOrCommand> {
    editing::code_actions(
        workspace,
        main,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            ..Default::default()
        },
    )
}

#[test]
fn indirect_selection_allows_only_selected_repairs_and_preserves_versioned_source() {
    for form in [
        "direct",
        "named",
        "literal",
        "alias",
        "named-alias",
        "literal-alias",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("% 😀\r\n\\documentclass{{article}}\\RequirePackage{{definitions}}{}{{chapte,% selected\r\nchapter.tex% tail\r\n}}\\begin{{document}}\\include% keep\r\n{{chapte% file\r\n}}\\end{{document}}", selector(form));
        let (workspace, main, diagnostic) = fixture(directory.path(), &source, true);
        let actions = fixes(&workspace, &main, &diagnostic);
        assert_eq!(actions.len(), 1, "{form}: {actions:?}");
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!()
        };
        let Some(DocumentChanges::Edits(changes)) = &action.edit.as_ref().unwrap().document_changes
        else {
            panic!()
        };
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].text_document.uri, main);
        assert_eq!(changes[0].text_document.version, Some(7));
        assert_eq!(changes[0].edits.len(), 1);
        let OneOf::Left(edit) = &changes[0].edits[0] else {
            panic!()
        };
        assert_eq!(edit.new_text, "chapter");
        let changed = workspace.documents[&main]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        assert_eq!(
            changed.source,
            source.replace("{chapte% file", "{chapter% file"),
            "{form}"
        );
    }
}

#[test]
fn indirect_selection_rejects_excluded_candidates_and_unknown_lists() {
    for form in [
        "direct",
        "named",
        "literal",
        "alias",
        "named-alias",
        "literal-alias",
    ] {
        for names in ["chapte", "", r"chapte,\dynamic"] {
            let directory = tempfile::tempdir().unwrap();
            let source = format!("\\documentclass{{article}}\\RequirePackage{{definitions}}{}{{{names}}}\\begin{{document}}\\include{{chapte}}\\end{{document}}", selector(form));
            let (workspace, main, diagnostic) = fixture(directory.path(), &source, true);
            assert!(
                fixes(&workspace, &main, &diagnostic).is_empty(),
                "{form}/{names}"
            );
        }
    }
}

#[test]
fn legacy_selection_fixes_recheck_unsaved_lists_and_alias_providers() {
    for form in [
        "direct",
        "named",
        "literal",
        "alias",
        "named-alias",
        "literal-alias",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("\\documentclass{{article}}\\RequirePackage{{definitions}}{}{{chapte,chapter}}\\begin{{document}}\\include{{chapte}}\\end{{document}}", selector(form));
        let (mut workspace, main, diagnostic) = fixture(directory.path(), &source, false);
        let actions = fixes(&workspace, &main, &diagnostic);
        assert_eq!(actions.len(), 1, "{form}: {actions:?}");
        let CodeActionOrCommand::Command(command) = &actions[0] else {
            panic!()
        };
        let request = command.arguments.as_ref().unwrap()[0].clone();
        let resolve = |workspace: &Workspace| {
            editing::resolve_fix(workspace, serde_json::from_value(request.clone()).unwrap())
        };
        assert!(resolve(&workspace).is_some());
        // Keep the source version and diagnostic range unchanged: only the
        // current selection text can invalidate this legacy request.
        let excluded = source.replace("chapte,chapter", "chapte,excludd");
        workspace
            .documents
            .insert(main.clone(), Document::new(&main, excluded, Some(7)));
        assert!(resolve(&workspace).is_none(), "{form}: changed selection");
        workspace
            .documents
            .insert(main.clone(), Document::new(&main, source.clone(), Some(7)));
        assert!(resolve(&workspace).is_some());
        if matches!(form, "alias" | "named-alias" | "literal-alias") {
            let provider = Url::from_file_path(directory.path().join("definitions.sty")).unwrap();
            for replacement in [
                r"\def\choose#1{\includeonly{#1}}",
                r"\def\choose{\includeonly}\def\choose{\includeonly}",
                r"\def\includeonly#1{}\def\choose{\includeonly}",
            ] {
                workspace.documents.insert(
                    provider.clone(),
                    Document::new(&provider, replacement.into(), Some(4)),
                );
                assert!(resolve(&workspace).is_none(), "{form}: {replacement}");
            }
        }
    }
}
