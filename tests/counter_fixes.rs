use latex_lsp::{
    editing,
    workspace::{Document, Workspace},
};
use lsp_types::*;
use std::collections::BTreeMap;

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
fn counter_spelling_repairs_recheck_macro_operands_across_project_sources() {
    let source = r"\bridge\setcounter{equatoin}{7}";
    let declarations = r"\newcommand{\done}{}\def\bridge{\done}";
    let (mut workspace, uri, definitions) = setup(source, declarations);
    let mut error = diagnostic(&mut workspace, &uri, "equatoin");
    workspace
        .build_diagnostic_sources
        .insert(uri.clone(), [uri.clone()].into());
    assert!(actions(&workspace, &uri, &error).is_empty());
    workspace
        .build_diagnostic_sources
        .insert(uri.clone(), [uri.clone(), definitions.clone()].into());
    assert_eq!(
        apply(
            &workspace,
            &uri,
            action(&actions(&workspace, &uri, &error), "equation")
                .edit
                .unwrap()
        ),
        source.replace("equatoin", "equation")
    );
    let later = uri.join("later.sty").unwrap();
    workspace.documents.insert(
        later.clone(),
        Document::new(&later, r"\renewcommand{\done}[3]{}".into(), None),
    );
    workspace
        .recorded_inputs
        .get_mut(&uri)
        .unwrap()
        .insert(later.clone());
    workspace
        .build_diagnostic_sources
        .get_mut(&uri)
        .unwrap()
        .insert(later);
    error.data.as_mut().unwrap()["macroSources"] = serde_json::json!([uri, definitions]);
    store(&mut workspace, &uri, error.clone());
    assert!(!actions(&workspace, &uri, &error).is_empty());
    workspace.documents.insert(
        definitions.clone(),
        Document::new(
            &definitions,
            r"\newcommand{\done}[3]{}\def\bridge{\done}".into(),
            Some(4),
        ),
    );
    assert!(actions(&workspace, &uri, &error).is_empty());
    for (source, definitions) in [
        (
            r"\wrapped\setcounter{equatoin}{7}",
            r"\newcommand{\wrapped}[3]{}",
        ),
        (
            r"\wrapped{Unused}\setcounter{equatoin}{7}",
            r"\newcommand{\wrapped}[4]{}",
        ),
        (
            r"\bridge\setcounter{equatoin}{7}",
            r"\newcommand{\wrapped}[3]{}\def\bridge{\wrapped}",
        ),
        (
            r"\setcounter{equatoin}{7}",
            r"\renewcommand{\setcounter}[2]{}",
        ),
    ] {
        let (mut workspace, uri, _) = setup(source, definitions);
        let error = diagnostic(&mut workspace, &uri, "equatoin");
        assert!(
            actions(&workspace, &uri, &error).is_empty(),
            "{source}/{definitions}"
        );
    }
}

#[test]
fn counter_fixes_recheck_paused_parent_declarations_and_metadata() {
    let source = r"\bridge\setcounter{equatoin}{7}";
    let (mut workspace, root, definitions) = setup(
        r"\input{content}",
        r"\newcommand{\done}{}\def\bridge{\done}",
    );
    let child = root.join("content.tex").unwrap();
    workspace
        .documents
        .insert(child.clone(), Document::new(&child, source.into(), Some(3)));
    workspace
        .recorded_inputs
        .get_mut(&root)
        .unwrap()
        .insert(child.clone());
    workspace.build_diagnostic_sources.insert(
        root.clone(),
        [root.clone(), child.clone(), definitions.clone()].into(),
    );
    let start = source.find("equatoin").unwrap();
    let error = Diagnostic {
        range: workspace.documents[&child].text.range(start, start + 8),
        message: "LaTeX Error: No counter 'equatoin' defined.".into(),
        source: Some("latex-build".into()),
        code: Some(NumberOrString::String("undefined-counter".into())),
        data: Some(
            serde_json::json!({"counter":"equatoin", "macroSources":[child,definitions],"macroPausedSources":[root]}),
        ),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        root.clone(),
        BTreeMap::from([(child.clone(), vec![error.clone()])]),
    );
    assert!(!actions(&workspace, &child, &error).is_empty());
    for value in [
        serde_json::json!("invalid"),
        serde_json::json!([child]),
        serde_json::json!(["file:///outside.tex"]),
    ] {
        let mut malformed = error.clone();
        malformed.data.as_mut().unwrap()["macroPausedSources"] = value;
        workspace
            .build_diagnostics
            .get_mut(&root)
            .unwrap()
            .insert(child.clone(), vec![malformed.clone()]);
        assert!(actions(&workspace, &child, &malformed).is_empty());
    }
    workspace
        .build_diagnostics
        .get_mut(&root)
        .unwrap()
        .insert(child.clone(), vec![error.clone()]);
    for declaration in [
        r"\renewcommand{\done}[3]{}",
        r"\renewcommand{\bridge}[3]{}",
        r"\renewcommand{\setcounter}[2]{}",
    ] {
        workspace.documents.insert(
            root.clone(),
            Document::new(&root, format!("{declaration}\\input{{content}}"), Some(10)),
        );
        assert!(
            actions(&workspace, &child, &error).is_empty(),
            "{declaration}"
        );
    }
}

#[test]
fn optional_counter_repairs_protect_closing_brackets_without_changing_titles() {
    for (source, expected) in [
        (
            r"\newcounter{child}[counterNnew]",
            r"\newcounter{child}[{counter]new}]",
        ),
        (
            r"\newtheorem{shared}[counterNnew]{Title counterNnew}",
            r"\newtheorem{shared}[{counter]new}]{Title counterNnew}",
        ),
        (
            r"\newtheorem{within}{Title counterNnew}[counterNnew]",
            r"\newtheorem{within}{Title counterNnew}[\detokenize{counter]new}]",
        ),
        (
            r"\newtheorem{within}{Title}[{counterNnew}]",
            r"\newtheorem{within}{Title}[{\detokenize{counter]new}}]",
        ),
        (
            r"\newtheorem{within}{Title}[{{counterNnew}}]",
            r"\newtheorem{within}{Title}[{\detokenize{counter]new}}]",
        ),
    ] {
        for versioned in [false, true] {
            let (mut workspace, uri, _) = setup(source, r"\newcounter{counter]new}");
            workspace.versioned_edits = versioned;
            let mut diagnostic = diagnostic(&mut workspace, &uri, "counterNnew");
            if source.starts_with(r"\newtheorem{within}") {
                let start = source.rfind("counterNnew").unwrap();
                diagnostic.range = workspace.documents[&uri]
                    .text
                    .range(start, start + "counterNnew".len());
                store(&mut workspace, &uri, diagnostic.clone());
            }
            let response = actions(&workspace, &uri, &diagnostic);
            let action = action(&response, "counter]new");
            assert_eq!(
                apply(&workspace, &uri, action.edit.unwrap()),
                expected,
                "{source}"
            );
        }
    }
}

#[test]
fn counter_fixes_preserve_values_whitespace_utf16_and_other_calls() {
    for command in [
        "setcounter",
        "addtocounter",
        "stepcounter",
        "refstepcounter",
        "usecounter",
        "setcounterref",
        "addtocounterref",
        "setcounterpageref",
        "addtocounterpageref",
    ] {
        for versioned in [false, true] {
            let value = if matches!(command, "setcounter" | "addtocounter") {
                r"{\value{section}+1}"
            } else if command.ends_with("ref") {
                "{equatoin}"
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
fn counter_relation_fixes_preserve_other_names_titles_formatting_and_optional_delimiters() {
    for source in [
        r"\newcounter{inner}[equatoin]",
        r"\newaliascnt{mirror}{equatoin}",
        r"\newtheorem{claim}[equatoin]{Claim \textbf{Title}}",
        r"\newtheorem{claim}{Claim \textbf{Title}}[equatoin]",
        r"\counterwithin{equatoin}{section}",
        r"\counterwithin*[\Roman]{section}{equatoin}",
        r"\counterwithout{equatoin}{section}",
        r"\counterwithout*[\Roman]{section}{equatoin}",
        r"\numberwithin[\Roman]{equatoin}{section}",
        r"\numberwithin{section}{equatoin}",
    ] {
        let (mut workspace, uri, _) = setup(source, "");
        let diagnostic = diagnostic(&mut workspace, &uri, "equatoin");
        let response = actions(&workspace, &uri, &diagnostic);
        assert_eq!(
            apply(
                &workspace,
                &uri,
                action(&response, "equation").edit.unwrap()
            ),
            source.replace("equatoin", "equation")
        );
    }
}

#[test]
fn counter_relation_fixes_do_not_create_self_aliases_or_reset_relationships() {
    for (source, name) in [
        (r"\newcounter{inner}[innre]", "innre"),
        (r"\newaliascnt{mirror}{miroor}", "miroor"),
        (r"\newtheorem{claim}{Claim}[cliam]", "cliam"),
        (r"\counterwithin{equatoin}{equation}", "equatoin"),
        (r"\counterwithin{equation}{equatoin}", "equatoin"),
        (r"\counterwithout*{equatoin}{equation}", "equatoin"),
        (r"\numberwithin[\Roman]{equation}{equatoin}", "equatoin"),
    ] {
        let (mut workspace, uri, _) = setup(source, "");
        let diagnostic = diagnostic(&mut workspace, &uri, name);
        assert!(
            actions(&workspace, &uri, &diagnostic).is_empty(),
            "{source}"
        );
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
