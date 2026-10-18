use latex_lsp::{
    editing,
    workspace::{Document, Workspace},
};
use std::collections::BTreeMap;
use tower_lsp::lsp_types::*;

fn setup(source: &str, definitions: &str) -> (Workspace, Url, Url) {
    let uri = Url::parse("file:///counter-fixes/main.tex").unwrap();
    let declarations = Url::parse("file:///counter-fixes/definitions.tex").unwrap();
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
        message: format!("LaTeX Error: No counter '{name}' defined."),
        code: Some(NumberOrString::String("undefined-counter".into())),
        data: Some(serde_json::json!({"counter":name})),
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
                if action.title == format!("Replace with counter `{replacement}`") =>
            {
                Some(action.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {replacement}: {response:?}"))
}

fn apply(workspace: &Workspace, uri: &Url, edit: WorkspaceEdit) -> String {
    apply_count(workspace, uri, edit, 1)
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

#[test]
fn counter_fixes_preserve_values_whitespace_utf16_and_other_calls() {
    for command in [
        "setcounter",
        "addtocounter",
        "stepcounter",
        "refstepcounter",
        "usecounter",
    ] {
        for versioned in [false, true] {
            let value = if matches!(command, "setcounter" | "addtocounter") {
                r"{\value{section}+1}"
            } else {
                ""
            };
            let source =
                format!("😀 \\{command}  {{equatoin}}{value}\r\n\\stepcounter{{equatoin}}\r\n");
            let (mut workspace, uri, _) = setup(&source, "");
            workspace.versioned_edits = versioned;
            let diagnostic = diagnostic(&mut workspace, &uri, "equatoin");
            let response = actions(&workspace, &uri, &diagnostic);
            assert!(response.len() <= 5);
            let action = action(&response, "equation");
            assert_eq!(action.is_preferred, Some(false));
            assert_eq!(action.diagnostics, Some(vec![diagnostic]));
            assert_eq!(
                apply(&workspace, &uri, action.edit.unwrap()),
                source.replacen("equatoin", "equation", 1)
            );
        }
    }
}

#[test]
fn counter_candidates_follow_the_build_project_and_include_aliases_and_owned_theorems() {
    for (misspelled, correct, definitions) in [
        ("seqeunce", "sequence", r"\newcounter{sequence}"),
        (
            "miroor",
            "mirror",
            r"\newcounter{base}\newaliascnt{mirror}{base}",
        ),
        ("cliam", "claim", r"\newtheorem{claim}{Claim}"),
        ("计数模势", "计数模式", r"\newcounter{计数模式}"),
    ] {
        let source = format!(r"\setcounter{{{misspelled}}}{{2}}");
        let (mut workspace, uri, _) = setup(&source, definitions);
        let unrelated = Url::parse("file:///other/main.tex").unwrap();
        workspace.documents.insert(
            unrelated.clone(),
            Document::new(&unrelated, format!(r"\newcounter{{{misspelled}}}"), None),
        );
        let diagnostic = diagnostic(&mut workspace, &uri, misspelled);
        let response = actions(&workspace, &uri, &diagnostic);
        assert_eq!(response.len(), 1, "{response:?}");
        assert_eq!(
            apply(&workspace, &uri, action(&response, correct).edit.unwrap()),
            source.replace(misspelled, correct)
        );
    }
    for definitions in [
        r"\newtheorem*{claim}{Claim}",
        r"\newtheorem{claim}[section]{Claim}",
        r"\newtheorem{ claim }{Claim}",
        r"\newcounter{ claim }",
        r"\newaliascnt{ claim }{section}",
    ] {
        let (mut workspace, uri, _) = setup(r"\stepcounter{cliam}", definitions);
        let diagnostic = diagnostic(&mut workspace, &uri, "cliam");
        assert!(actions(&workspace, &uri, &diagnostic).is_empty());
    }
}

#[test]
fn counter_fixes_reject_unknown_context_known_names_ambiguous_candidates_and_forged_diagnostics() {
    for (source, definitions, mode) in [
        (
            r"% \setcounter{seqeunce}{2}",
            r"\newcounter{sequence}",
            "precise",
        ),
        (
            r"\verb|\setcounter{seqeunce}{2}|",
            r"\newcounter{sequence}",
            "precise",
        ),
        (
            r"\setcounter{seqeunce\suffix}{2}",
            r"\newcounter{sequence}",
            "precise",
        ),
        (
            r"\setcounter{seqeunce}",
            r"\newcounter{sequence}",
            "precise",
        ),
        (r"\value{seqeunce}", r"\newcounter{sequence}", "precise"),
        (
            r"\setcounter{seqeunce}{2}",
            r"\newcounter{seqeunce}\newcounter{sequence}",
            "precise",
        ),
        (
            r"\setcounter{seqeunce}{2}",
            r"\newcounter{sequence}\newcounter{sequence}",
            "precise",
        ),
        (
            r"\setcounter{seqeunce}{2}",
            r"\newcounter{sequence}\newtheorem{sequence}{Sequence}",
            "precise",
        ),
        (
            r"\setcounter{seqeunce}{2}",
            r"\newcounter{sequence}",
            "code",
        ),
        (
            r"\setcounter{seqeunce}{2}",
            r"\newcounter{sequence}",
            "data",
        ),
        (
            r"\setcounter{seqeunce}{2}",
            r"\newcounter{sequence}",
            "message",
        ),
        (
            r"\setcounter{seqeunce}{2}",
            r"\newcounter{sequence}",
            "range",
        ),
        (
            r"\setcounter{seqeunce}{2}",
            r"\newcounter{sequence}",
            "source",
        ),
    ] {
        let (mut workspace, uri, _) = setup(source, definitions);
        let mut diagnostic = diagnostic(&mut workspace, &uri, "seqeunce");
        match mode {
            "code" => diagnostic.code = None,
            "data" => diagnostic.data = Some(serde_json::json!({"counter":"other"})),
            "message" => diagnostic.message = "LaTeX Error: No counter 'other' defined.".into(),
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
fn legacy_counter_fixes_revalidate_versions_candidates_and_source_arguments() {
    let source = r"\setcounter{seqeunce}{2}";
    let (mut workspace, uri, declarations) = setup(source, r"\newcounter{sequence}");
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let diagnostic = diagnostic(&mut workspace, &uri, "seqeunce");
    let response = actions(&workspace, &uri, &diagnostic);
    let CodeActionOrCommand::Command(command) = &response[0] else {
        panic!("missing command")
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let resolve = |workspace: &Workspace, value: serde_json::Value| {
        editing::resolve_fix(workspace, serde_json::from_value(value).unwrap())
    };
    assert_eq!(
        apply(
            &workspace,
            &uri,
            resolve(&workspace, value.clone()).unwrap()
        ),
        source.replace("seqeunce", "sequence")
    );
    let mut forged = value.clone();
    forged["replacement"] = serde_json::json!("other");
    assert!(resolve(&workspace, forged).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(resolve(&workspace, value.clone()).is_none());
    workspace.documents.insert(
        uri.clone(),
        Document::new(&uri, r"\setcounter{seqeunce}".into(), Some(9)),
    );
    assert!(resolve(&workspace, value.clone()).is_none());
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(9)));
    workspace.documents.remove(&declarations);
    assert!(resolve(&workspace, value.clone()).is_none());
    workspace.clear_build_diagnostics(&uri);
    assert!(resolve(&workspace, value).is_none());
}

#[test]
#[ignore = "requires pdflatex"]
fn counter_spelling_fixes_turn_a_real_failed_build_into_a_successful_one() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let child = root.join("章节.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\\input{章节}Witness.\\end{document}\n",
    )
    .unwrap();
    let source = "\\setcounter{equatoin}{2}\n";
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
            diagnostic.code == Some(NumberOrString::String("undefined-counter".into()))
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
    std::fs::write(
        &child,
        apply(
            &workspace,
            &uri,
            action(&response, "equation").edit.unwrap(),
        ),
    )
    .unwrap();
    let after = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(after.status, "success", "{}", after.output);
    assert_eq!(after.pages, Some(1));
}
