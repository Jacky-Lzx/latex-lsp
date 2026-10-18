use latex_lsp::{
    editing,
    workspace::{Document, Workspace},
};
use std::collections::BTreeMap;
use tower_lsp::lsp_types::*;

fn setup(source: &str, definitions: &str) -> (Workspace, Url, Url) {
    let uri = Url::parse("file:///environment-fixes/main.tex").unwrap();
    let declarations = Url::parse("file:///environment-fixes/definitions.tex").unwrap();
    let mut workspace = Workspace {
        versioned_edits: true,
        code_action_literals: true,
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(9)));
    workspace.documents.insert(
        declarations.clone(),
        Document::new(&declarations, definitions.into(), None),
    );
    workspace
        .recorded_inputs
        .insert(uri.clone(), [uri.clone(), declarations.clone()].into());
    (workspace, uri, declarations)
}

fn diagnostic(workspace: &mut Workspace, uri: &Url, name: &str) -> Diagnostic {
    let text = &workspace.documents[uri].text;
    let start = text.source.find(name).unwrap();
    let diagnostic = Diagnostic {
        range: text.range(start, start + name.len()),
        source: Some("latex-build".into()),
        message: format!("LaTeX Error: Environment {name} undefined."),
        code: Some(NumberOrString::String("undefined-environment".into())),
        data: Some(serde_json::json!({"environment":name})),
        ..Default::default()
    };
    store(workspace, uri, diagnostic.clone());
    diagnostic
}

fn store(workspace: &mut Workspace, uri: &Url, diagnostic: Diagnostic) {
    workspace.build_diagnostics.insert(
        uri.clone(),
        BTreeMap::from([(uri.clone(), vec![diagnostic])]),
    );
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

fn action(response: &CodeActionResponse, replacement: &str) -> CodeAction {
    response
        .iter()
        .find_map(|item| match item {
            CodeActionOrCommand::CodeAction(action)
                if action.title == format!("Replace with environment `{replacement}`") =>
            {
                Some(action.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {replacement}: {response:?}"))
}

fn apply(workspace: &Workspace, uri: &Url, edit: WorkspaceEdit) -> String {
    apply_count(workspace, uri, edit, 2)
}

fn apply_count(workspace: &Workspace, uri: &Url, edit: WorkspaceEdit, count: usize) -> String {
    let mut edits = if let Some(changes) = edit.changes {
        changes[uri].clone()
    } else {
        let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
            panic!("missing changes")
        };
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].text_document.uri, *uri);
        assert_eq!(
            changes[0].text_document.version,
            workspace.documents[uri].version
        );
        changes[0]
            .edits
            .iter()
            .map(|edit| match edit {
                OneOf::Left(edit) => edit.clone(),
                _ => panic!("unexpected annotation"),
            })
            .collect()
    };
    assert_eq!(edits.len(), count);
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    for pair in edits.windows(2) {
        assert_eq!(pair[0].new_text, pair[1].new_text);
        assert!(pair[1].range.end < pair[0].range.start);
    }
    workspace.documents[uri]
        .text
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

fn closing_diagnostic(
    workspace: &mut Workspace,
    uri: &Url,
    name: &str,
    expected: &str,
) -> Diagnostic {
    let text = &workspace.documents[uri].text;
    let start = text.source.rfind(name).unwrap();
    let diagnostic = Diagnostic {
        range: text.range(start, start + name.len()),
        source: Some("latex-build".into()),
        message: format!(
            "LaTeX Error: \\begin{{{expected}}} on input line 1 ended by \\end{{{name}}}."
        ),
        code: Some(NumberOrString::String("mismatched-environment-end".into())),
        data: Some(serde_json::json!({"environment":name,"expectedEnvironment":expected})),
        ..Default::default()
    };
    store(workspace, uri, diagnostic.clone());
    diagnostic
}

#[test]
fn closing_fixes_use_the_confirmed_environment_stack_and_change_only_the_end_name() {
    let source = "😀 \\begin{equation}\r\n\\begin{aligned}x&=y\\end{aligned}\r\n\\begin{verbatim}\r\n\\end{algin}\r\n\\end{verbatim}\r\n\\end{ algin }\r\n";
    for versioned in [false, true] {
        let (mut workspace, uri, _) = setup(source, "");
        workspace.versioned_edits = versioned;
        let diagnostic = closing_diagnostic(&mut workspace, &uri, "algin", "equation");
        let response = actions(&workspace, &uri, &diagnostic);
        assert_eq!(response.len(), 1, "{response:?}");
        let CodeActionOrCommand::CodeAction(action) = &response[0] else {
            panic!("missing action")
        };
        assert_eq!(action.title, "Close environment `equation`");
        assert_eq!(action.is_preferred, Some(false));
        assert_eq!(
            apply_count(&workspace, &uri, action.edit.clone().unwrap(), 1),
            source.replace("end{ algin }", "end{ equation }")
        );
    }
}

#[test]
fn closing_fixes_reject_cascading_undefined_environment_errors_dynamic_stacks_and_forged_metadata()
{
    for (source, expected, mode) in [
        (
            r"\begin{document}\begin{algin}Content\end{algin}\end{document}",
            "document",
            "precise",
        ),
        (r"\end{algin}", "equation", "precise"),
        (r"\begin{\name}Content\end{algin}", "equation", "precise"),
        (
            r"\begin{equation}\begin{aligned}\end{other}\end{algin}",
            "equation",
            "precise",
        ),
        (
            r"\begin{equation}\begin{verbatim}\end{algin}",
            "equation",
            "precise",
        ),
        (r"\begin{equation}\verb|\end{algin}|", "equation", "precise"),
        (r"\begin{equation}\end{algin}", "equation", "data"),
        (r"\begin{equation}\end{algin}", "equation", "code"),
        (r"\begin{equation}\end{algin}", "equation", "range"),
    ] {
        let (mut workspace, uri, _) = setup(source, "");
        let mut diagnostic = closing_diagnostic(&mut workspace, &uri, "algin", expected);
        match mode {
            "data" => {
                diagnostic.data =
                    Some(serde_json::json!({"environment":"algin","expectedEnvironment":"align"}))
            }
            "code" => diagnostic.code = None,
            "range" => diagnostic.range.end.character += 1,
            _ => {}
        }
        store(&mut workspace, &uri, diagnostic.clone());
        assert!(
            actions(&workspace, &uri, &diagnostic).is_empty(),
            "{source}/{mode}"
        );
    }
}

#[test]
fn legacy_closing_fixes_recheck_the_stack_and_document_version() {
    let source = r"\begin{equation}Content\end{algin}";
    let (mut workspace, uri, _) = setup(source, "");
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let diagnostic = closing_diagnostic(&mut workspace, &uri, "algin", "equation");
    let response = actions(&workspace, &uri, &diagnostic);
    let CodeActionOrCommand::Command(command) = &response[0] else {
        panic!("missing command")
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let edit =
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).unwrap();
    assert_eq!(
        apply_count(&workspace, &uri, edit, 1),
        source.replace("end{algin}", "end{equation}")
    );
    let mut forged = value.clone();
    forged["replacement"] = serde_json::json!("document");
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace.documents.insert(
        uri.clone(),
        Document::new(
            &uri,
            source.replace("begin{equation}", "begin{tabular*}"),
            Some(9),
        ),
    );
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(value).unwrap()).is_none());
}

#[test]
#[ignore = "requires pdflatex"]
fn closing_name_fixes_repair_a_real_compiler_error() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let source = "\\documentclass{article}\n\\begin{document}\n\\begin{equation}x=1\\end{equatoin}\n\\end{document}\n";
    std::fs::write(&main, source).unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let before = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(before.status, "error", "{}", before.output);
    let uri = Url::from_file_path(&main).unwrap();
    let diagnostic = before.diagnostics[&uri]
        .iter()
        .find(|diagnostic| {
            diagnostic.code == Some(NumberOrString::String("mismatched-environment-end".into()))
        })
        .unwrap()
        .clone();
    let mut workspace = Workspace {
        versioned_edits: true,
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(uri.clone(), source.into(), 9);
    workspace
        .build_diagnostics
        .insert(uri.clone(), before.diagnostics);
    let response = actions(&workspace, &uri, &diagnostic);
    let CodeActionOrCommand::CodeAction(action) = &response[0] else {
        panic!("missing closing action: {response:?}")
    };
    std::fs::write(
        &main,
        apply_count(&workspace, &uri, action.edit.clone().unwrap(), 1),
    )
    .unwrap();
    let after = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(after.pages, Some(1));
}

#[test]
fn environment_fixes_update_only_the_selected_pair_and_preserve_nested_and_opaque_text() {
    let source = "😀 \\begin{ algin }\r\n\\begin{aligned}x&=y\\end{aligned}\r\n\\begin{algin}Nested\\end{algin}\r\n\\begin{verbatim}\r\n\\end{algin}\r\n\\begin{fake}\r\n\\end{verbatim}\r\n\\verb|\\end{algin}|\r\n% \\end{algin}\r\n\\end{ algin }\r\n\\begin{algin}Other\\end{algin}\r\n";
    for versioned in [false, true] {
        let (mut workspace, uri, _) = setup(source, "");
        workspace.versioned_edits = versioned;
        let diagnostic = diagnostic(&mut workspace, &uri, "algin");
        let response = actions(&workspace, &uri, &diagnostic);
        assert!(response.len() <= 5);
        let action = action(&response, "align");
        assert_eq!(action.is_preferred, Some(false));
        assert_eq!(action.diagnostics, Some(vec![diagnostic]));
        let changed = apply(&workspace, &uri, action.edit.unwrap());
        assert_eq!(
            changed,
            source
                .replacen("begin{ algin }", "begin{ align }", 1)
                .replacen("end{ algin }", "end{ align }", 1)
        );
    }
}

#[test]
fn environment_candidates_follow_the_compiled_project_and_support_unicode() {
    for (misspelled, correct) in [("panle", "panel"), ("数学模势", "数学模式")] {
        let source = format!(r"\begin{{{misspelled}}}Content\end{{{misspelled}}}");
        let definitions = format!(r"\newenvironment{{{correct}}}{{}}{{}}");
        let (mut workspace, uri, _) = setup(&source, &definitions);
        let unrelated = Url::parse("file:///unrelated/main.tex").unwrap();
        workspace.documents.insert(
            unrelated.clone(),
            Document::new(
                &unrelated,
                format!(r"\newenvironment{{{misspelled}}}{{}}{{}}"),
                None,
            ),
        );
        let diagnostic = diagnostic(&mut workspace, &uri, misspelled);
        let response = actions(&workspace, &uri, &diagnostic);
        assert_eq!(response.len(), 1, "{response:?}");
        assert_eq!(
            apply(&workspace, &uri, action(&response, correct).edit.unwrap()),
            source.replace(misspelled, correct)
        );
    }
}

#[test]
fn unsafe_unproven_known_and_ambiguous_environment_fixes_are_not_offered() {
    for (source, definitions, mode) in [
        (
            r"\begin{panle}Content",
            r"\newenvironment{panel}{}{}",
            "precise",
        ),
        (
            r"\begin{panle}Content\end{other}",
            r"\newenvironment{panel}{}{}",
            "precise",
        ),
        (
            r"\begin{panle}\begin{other}\end{panle}\end{other}",
            r"\newenvironment{panel}{}{}",
            "precise",
        ),
        (
            r"\begin{panle}\begin{\name}\end{\name}\end{panle}",
            r"\newenvironment{panel}{}{}",
            "precise",
        ),
        (
            r"\begin{panle}\begin{verbatim}unclosed",
            r"\newenvironment{panel}{}{}",
            "precise",
        ),
        (
            r"% \begin{panle}Content\end{panle}",
            r"\newenvironment{panel}{}{}",
            "precise",
        ),
        (
            r"\verb|\begin{panle}Content\end{panle}|",
            r"\newenvironment{panel}{}{}",
            "precise",
        ),
        (
            r"\begin{panle\suffix}Content\end{panle\suffix}",
            r"\newenvironment{panel}{}{}",
            "precise",
        ),
        (
            r"\begin{panle}Content\end{panle}",
            r"\newenvironment{panle}{}{}",
            "precise",
        ),
        (
            r"\begin{panle}Content\end{panle}",
            r"\newenvironment{panel}{}{}\renewenvironment{panel}{}{}",
            "precise",
        ),
        (
            r"\begin{panle}Content\end{panle}",
            r"\newenvironment{panel}{}{}",
            "code",
        ),
        (
            r"\begin{panle}Content\end{panle}",
            r"\newenvironment{panel}{}{}",
            "data",
        ),
        (
            r"\begin{panle}Content\end{panle}",
            r"\newenvironment{panel}{}{}",
            "range",
        ),
        (
            r"\begin{panle}Content\end{panle}",
            r"\newenvironment{panel}{}{}",
            "source",
        ),
    ] {
        let (mut workspace, uri, _) = setup(source, definitions);
        let mut diagnostic = diagnostic(&mut workspace, &uri, "panle");
        match mode {
            "code" => diagnostic.code = None,
            "data" => diagnostic.data = Some(serde_json::json!({"environment":"other"})),
            "range" => diagnostic.range.end.character += 1,
            "source" => diagnostic.source = Some("untrusted".into()),
            _ => {}
        }
        store(&mut workspace, &uri, diagnostic.clone());
        assert!(
            actions(&workspace, &uri, &diagnostic).is_empty(),
            "{source}/{mode}"
        );
    }
}

#[test]
fn legacy_environment_fixes_revalidate_version_replacement_candidate_and_pair() {
    let source = r"\begin{panle}Content\end{panle}";
    let (mut workspace, uri, declarations) = setup(source, r"\newenvironment{panel}{}{}");
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let diagnostic = diagnostic(&mut workspace, &uri, "panle");
    let response = actions(&workspace, &uri, &diagnostic);
    let CodeActionOrCommand::Command(command) = &response[0] else {
        panic!("missing command")
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let edit =
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).unwrap();
    assert_eq!(
        apply(&workspace, &uri, edit),
        source.replace("panle", "panel")
    );
    let mut forged = value.clone();
    forged["replacement"] = serde_json::json!("other");
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace.documents.insert(
        uri.clone(),
        Document::new(&uri, r"\begin{panle}Content".into(), Some(9)),
    );
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(9)));
    workspace.documents.remove(&declarations);
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace.clear_build_diagnostics(&uri);
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(value).unwrap()).is_none());
}

#[test]
#[ignore = "requires pdflatex and amsmath"]
fn paired_environment_spelling_fixes_turn_a_real_failed_build_into_a_successful_one() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let child = root.join("章节.tex");
    std::fs::write(&main, "\\documentclass{article}\\usepackage{amsmath}\n\\begin{document}\\input{章节}\\end{document}\n").unwrap();
    let source = "\\begin{algin}\nx &= 1\n\\end{algin}\n";
    std::fs::write(&child, source).unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let before = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(before.status, "error", "{}", before.output);
    let uri = Url::from_file_path(&child).unwrap();
    let diagnostic = before.diagnostics[&uri]
        .iter()
        .find(|diagnostic| {
            diagnostic.code == Some(NumberOrString::String("undefined-environment".into()))
        })
        .unwrap()
        .clone();
    let mut workspace = Workspace {
        versioned_edits: true,
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(uri.clone(), source.into(), 9);
    workspace
        .build_diagnostics
        .insert(Url::from_file_path(&main).unwrap(), before.diagnostics);
    let response = actions(&workspace, &uri, &diagnostic);
    let changed = apply(&workspace, &uri, action(&response, "align").edit.unwrap());
    std::fs::write(&child, changed).unwrap();
    let after = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(after.pages, Some(1));
}
