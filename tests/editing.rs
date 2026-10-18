use latex_lsp::{
    editing, features,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn setup(source: &str, versioned: bool) -> (Workspace, Url) {
    let uri = Url::parse("file:///editing/main.tex").unwrap();
    let mut workspace = Workspace {
        versioned_edits: versioned,
        code_action_literals: true,
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(9)));
    (workspace, uri)
}

fn context(diagnostics: Vec<Diagnostic>) -> CodeActionContext {
    CodeActionContext {
        diagnostics,
        only: None,
        trigger_kind: None,
    }
}

fn action_edit(action: &CodeActionOrCommand, uri: &Url) -> TextEdit {
    let CodeActionOrCommand::CodeAction(action) = action else {
        panic!("missing code action");
    };
    let workspace_edit = action.edit.as_ref().unwrap();
    if let Some(changes) = &workspace_edit.changes {
        return changes[uri][0].clone();
    }
    let Some(DocumentChanges::Edits(documents)) = &workspace_edit.document_changes else {
        panic!("missing edit");
    };
    assert_eq!(documents[0].text_document.uri, *uri);
    assert_eq!(documents[0].text_document.version, Some(9));
    let OneOf::Left(edit) = &documents[0].edits[0] else {
        panic!("unexpected annotation");
    };
    edit.clone()
}

#[test]
fn hyperref_label_fixes_preserve_link_text_and_reject_forged_text_ranges() {
    for versioned in [false, true] {
        let source = "\\label{sec:intro}\n\\hyperref[sec:itnro]{sec:itnro}\n";
        let (mut workspace, uri) = setup(source, versioned);
        let text = &workspace.documents[&uri].text;
        let key = source.find("sec:itnro").unwrap();
        let body = source.rfind("sec:itnro").unwrap();
        let body_range = text.range(body, body + 9);
        let mut diagnostic = Diagnostic {
            range: text.range(key, key + 9),
            source: Some("latex-build".into()),
            message:
                "LaTeX Warning: Hyper reference `sec:itnro' on page 1 undefined on input line 2."
                    .into(),
            ..Default::default()
        };
        // hyperref uses a distinct warning prefix, which must still identify a label.
        workspace.build_diagnostics.insert(
            uri.clone(),
            [(uri.clone(), vec![diagnostic.clone()])].into(),
        );
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic.clone()]),
        );
        assert_eq!(actions.len(), 1, "{actions:?}");
        let edit = action_edit(&actions[0], &uri);
        assert_eq!(edit.range, diagnostic.range);
        assert_eq!(edit.new_text, "sec:intro");
        diagnostic.range = body_range;
        workspace.build_diagnostics.insert(
            uri.clone(),
            [(uri.clone(), vec![diagnostic.clone()])].into(),
        );
        assert!(editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic])
        )
        .is_empty());
    }
    let source = "\\label{sec:intro]}\n\\hyperref[sec:intro]{Text}\n";
    let (mut workspace, uri) = setup(source, false);
    let at = source.rfind("sec:intro").unwrap();
    let diagnostic = Diagnostic {
        range: workspace.documents[&uri].text.range(at, at + 9),
        source: Some("latex-build".into()),
        message: "LaTeX Warning: Hyper reference `sec:intro' on page 1 undefined on input line 2."
            .into(),
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
        &context(vec![diagnostic]),
    );
    assert_eq!(actions.len(), 1, "{actions:?}");
    let edit = action_edit(&actions[0], &uri);
    assert_eq!(edit.new_text, "{sec:intro]}");
    let changed = workspace.documents[&uri]
        .text
        .changed(vec![TextDocumentContentChangeEvent {
            range: Some(edit.range),
            range_length: None,
            text: edit.new_text,
        }])
        .unwrap();
    assert_eq!(
        changed.source,
        "\\label{sec:intro]}\n\\hyperref[{sec:intro]}]{Text}\n"
    );
    let position = changed.position(changed.source.rfind("sec:intro]").unwrap() + 3);
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, changed.source, Some(10)));
    assert_eq!(features::definitions(&workspace, &uri, position).len(), 1);
}

#[test]
fn refcount_assignment_label_fixes_select_the_second_argument_and_reject_counter_ranges() {
    for command in [
        "setcounterref",
        "addtocounterref",
        "setcounterpageref",
        "addtocounterpageref",
    ] {
        for versioned in [false, true] {
            let source = format!("\\label{{sec:intro}}\n\\{command}{{sec:itnro}}{{sec:itnro}}\n");
            let (mut workspace, uri) = setup(&source, versioned);
            let text = &workspace.documents[&uri].text;
            let second = source.rfind("sec:itnro").unwrap();
            let first = source.find("sec:itnro").unwrap();
            let first_range = text.range(first, first + 9);
            let mut diagnostic = Diagnostic {
                range: text.range(second, second + 9),
                source: Some("latex-build".into()),
                message:
                    "LaTeX Warning: Reference `sec:itnro' on page 1 undefined on input line 2."
                        .into(),
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
                &context(vec![diagnostic.clone()]),
            );
            assert_eq!(actions.len(), 1, "{actions:?}");
            let edit = action_edit(&actions[0], &uri);
            assert_eq!(edit.range, diagnostic.range);
            assert_eq!(edit.new_text, "sec:intro");
            let changed = workspace.documents[&uri]
                .text
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap()
                .source;
            assert_eq!(
                changed,
                format!("\\label{{sec:intro}}\n\\{command}{{sec:itnro}}{{sec:intro}}\n")
            );
            diagnostic.range = first_range;
            workspace.build_diagnostics.insert(
                uri.clone(),
                [(uri.clone(), vec![diagnostic.clone()])].into(),
            );
            assert!(editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &context(vec![diagnostic])
            )
            .is_empty());
        }
    }
}

#[test]
fn quick_fixes_repair_braces_and_environments_including_comments_raw_text_and_crlf() {
    for source in [
        "😀 }",
        "{中文",
        "{content % trailing comment",
        "\\begin{equation}\nx=1\n",
        "\\begin{equation}\r\nx=1",
        "\\begin{itemize}\n\\begin{equation}\nx=1\n\\end{itemize}",
        "\\end{itemize}",
        "\\begin{verbatim}\n{ % raw",
    ] {
        for versioned in [false, true] {
            let (workspace, uri) = setup(source, versioned);
            let diagnostics = features::diagnostics(&workspace.documents[&uri]);
            assert_eq!(diagnostics.len(), 1, "{source}");
            let actions = editing::code_actions(
                &workspace,
                &uri,
                diagnostics[0].range,
                &context(diagnostics.clone()),
            );
            assert_eq!(actions.len(), 1, "{source}");
            let edit = action_edit(&actions[0], &uri);
            if source.contains("\r\n") {
                assert!(edit.new_text.contains("\r\n"));
            }
            let repaired = Text::new(source.into())
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap();
            let document = Document::new(&uri, repaired.source, Some(10));
            assert!(
                features::diagnostics(&document).is_empty(),
                "{source}: {:?}",
                document.syntax.diagnostics
            );
        }
    }
}

#[test]
fn fixes_filter_requested_kinds_ranges_and_stale_or_forged_diagnostics() {
    let (mut workspace, uri) = setup("ok\n{broken", false);
    let diagnostic = features::diagnostics(&workspace.documents[&uri])[0].clone();
    let mut request = context(vec![diagnostic.clone()]);
    request.only = Some(vec![CodeActionKind::REFACTOR]);
    assert!(editing::code_actions(&workspace, &uri, diagnostic.range, &request).is_empty());
    request.only = Some(vec![CodeActionKind::QUICKFIX]);
    assert_eq!(
        editing::code_actions(&workspace, &uri, diagnostic.range, &request).len(),
        1
    );
    assert!(editing::code_actions(&workspace, &uri, Range::default(), &request).is_empty());
    request.diagnostics[0].source = Some("latex-build".into());
    assert!(editing::code_actions(&workspace, &uri, diagnostic.range, &request).is_empty());
    request.diagnostics = vec![diagnostic.clone()];
    request.diagnostics[0].data = Some(serde_json::json!({"edit":{"newText":"malicious"}}));
    let action = editing::code_actions(&workspace, &uri, diagnostic.range, &request);
    assert_eq!(action_edit(&action[0], &uri).new_text, "}");
    workspace.documents.insert(
        uri.clone(),
        Document::new(&uri, "ok\n{fixed}".into(), Some(10)),
    );
    assert!(editing::code_actions(&workspace, &uri, diagnostic.range, &request).is_empty());
}

#[test]
fn highlights_distinguish_local_declarations_and_references_and_skip_opaque_text() {
    let source =
        "\\label{sec:x} \\ref{sec:x} \\cref{other,sec:x}\n% \\ref{sec:x}\n\\verb|\\ref{sec:x}|";
    let (workspace, uri) = setup(source, false);
    let position = workspace.documents[&uri]
        .text
        .position(source.find("ref{sec:x}").unwrap() + 6);
    let highlights = editing::highlights(&workspace, &uri, position);
    assert_eq!(highlights.len(), 3);
    assert_eq!(
        highlights
            .iter()
            .filter(|h| h.kind == Some(DocumentHighlightKind::WRITE))
            .count(),
        1
    );
    assert_eq!(
        highlights
            .iter()
            .filter(|h| h.kind == Some(DocumentHighlightKind::READ))
            .count(),
        2
    );
}

#[test]
fn selection_ranges_expand_through_nested_groups_sections_and_the_document() {
    let source = "\\section{Intro}\r\n😀 \\textbf{中文 {nested words}}\r\n% comment\r\n\\section{Next}\r\nend\r\n";
    let (workspace, uri) = setup(source, false);
    let doc = &workspace.documents[&uri];
    let word = source.find("nested").unwrap();
    let positions = [
        doc.text.position(word + 2),
        Position::new(1, 2),
        doc.text.position(source.len()),
    ];
    let selections = editing::selection_ranges(doc, &positions).unwrap();
    assert_eq!(selections.len(), positions.len());
    assert_eq!(selections[0].range, doc.text.range(word, word + 6));
    let mut found_group = false;
    let mut found_command = false;
    for (selection, position) in selections.iter().zip(positions) {
        let mut current = selection;
        assert!(current.range.start <= position && position <= current.range.end);
        while let Some(parent) = &current.parent {
            assert!(
                parent.range.start <= current.range.start && current.range.end <= parent.range.end
            );
            assert_ne!(parent.range, current.range);
            let start = doc.text.offset(parent.range.start).unwrap();
            let end = doc.text.offset(parent.range.end).unwrap();
            found_group |= &source[start..end] == "{nested words}";
            found_command |= &source[start..end] == "\\textbf{中文 {nested words}}";
            current = parent;
        }
        assert_eq!(current.range, doc.text.range(0, source.len()));
    }
    assert!(found_group && found_command);
    assert!(editing::selection_ranges(doc, &[Position::new(1, 1)]).is_none());
    let (workspace, uri) = setup("", false);
    assert_eq!(
        editing::selection_ranges(&workspace.documents[&uri], &[Position::default()]).unwrap()[0]
            .range,
        Range::default()
    );
}

#[test]
fn compiler_reference_fixes_use_current_project_keys_and_revalidate_command_requests() {
    let source = "😀 \\cref{sec:itnro, sec:other} \\cite{smith2025}\n";
    let (mut workspace, uri) = setup(source, true);
    let labels = Url::parse("file:///editing/chapter.tex").unwrap();
    let bibliography = Url::parse("file:///editing/refs.bib").unwrap();
    let unrelated = Url::parse("file:///unrelated/main.tex").unwrap();
    for (uri, source) in [
        (&labels, r"\label{sec:intro}\label{sec:other}"),
        (&bibliography, "@book{smith2026,title={Example}}"),
        (&unrelated, r"\label{sec:itnro}\label{sec:itnor}"),
    ] {
        workspace
            .documents
            .insert(uri.clone(), Document::new(uri, source.into(), None));
    }
    workspace.recorded_inputs.insert(
        uri.clone(),
        [uri.clone(), labels.clone(), bibliography].into(),
    );
    let text = &workspace.documents[&uri].text;
    let label_start = source.find("sec:itnro").unwrap();
    let citation_start = source.find("smith2025").unwrap();
    let label = Diagnostic {
        range: text.range(label_start, label_start + 9),
        source: Some("latex-build".into()),
        message: "LaTeX Warning: Reference `sec:itnro' on page 1 undefined on input line 1.".into(),
        ..Default::default()
    };
    let citation = Diagnostic {
        range: text.range(citation_start, citation_start + 9),
        source: Some("latex-build".into()),
        message: "LaTeX Warning: Citation `smith2025' on page 1 undefined on input line 1.".into(),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        uri.clone(),
        std::collections::BTreeMap::from([(uri.clone(), vec![label.clone(), citation.clone()])]),
    );
    let actions =
        editing::code_actions(&workspace, &uri, label.range, &context(vec![label.clone()]));
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(action_edit(&actions[0], &uri).new_text, "sec:intro");
    let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
        panic!();
    };
    assert_eq!(action.is_preferred, Some(false));
    let actions = editing::code_actions(
        &workspace,
        &uri,
        citation.range,
        &context(vec![citation.clone()]),
    );
    assert_eq!(actions.len(), 1);
    assert_eq!(action_edit(&actions[0], &uri).new_text, "smith2026");
    let mut forged = label.clone();
    forged.message =
        "LaTeX Warning: Reference `forged' on page 1 undefined on input line 1.".into();
    assert!(
        editing::code_actions(&workspace, &uri, label.range, &context(vec![forged])).is_empty()
    );
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let actions =
        editing::code_actions(&workspace, &uri, label.range, &context(vec![label.clone()]));
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!();
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let request = serde_json::from_value(value.clone()).unwrap();
    assert!(editing::resolve_fix(&workspace, request).is_some());
    let mut forged = value.clone();
    forged["replacement"] = serde_json::json!("sec:itnor");
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace.documents.get_mut(&uri).unwrap().version = Some(9);
    workspace.documents.remove(&labels);
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace.clear_build_diagnostics(&uri);
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(value).unwrap()).is_none());
}

#[test]
fn compiler_reference_suggestions_reject_ambiguous_dynamic_known_and_inexact_keys() {
    for (key, definitions, expected, precise) in [
        (
            "sec:valve",
            r"\label{sec:value}\label{sec:value}",
            None,
            true,
        ),
        (
            "sec:value",
            r"\label{sec:value}\label{sec:valve}",
            None,
            true,
        ),
        ("sec:value", r"\label{sec:\value}", None, true),
        (r"sec:\value", r"\label{sec:value}", None, true),
        ("ab", r"\label{ac}", None, true),
        ("章节序眼", r"\label{章节序言}", Some("章节序言"), true),
        ("sec:valve", r"\label{sec:value}", None, false),
    ] {
        let source = format!("\\ref{{{key}}}");
        let (mut workspace, uri) = setup(&source, false);
        let definitions_uri = Url::parse("file:///editing/definitions.tex").unwrap();
        workspace.documents.insert(
            definitions_uri.clone(),
            Document::new(&definitions_uri, definitions.into(), None),
        );
        workspace
            .recorded_inputs
            .insert(uri.clone(), [uri.clone(), definitions_uri].into());
        let range = if precise {
            workspace.documents[&uri].text.range(5, 5 + key.len())
        } else {
            workspace.documents[&uri].text.range(0, source.len())
        };
        let diagnostic = Diagnostic {
            range,
            source: Some("latex-build".into()),
            message: format!(
                "LaTeX Warning: Reference `{key}' on page 1 undefined on input line 1."
            ),
            ..Default::default()
        };
        workspace.build_diagnostics.insert(
            uri.clone(),
            std::collections::BTreeMap::from([(uri.clone(), vec![diagnostic.clone()])]),
        );
        let actions = editing::code_actions(&workspace, &uri, range, &context(vec![diagnostic]));
        if let Some(expected) = expected {
            assert_eq!(actions.len(), 1);
            assert_eq!(action_edit(&actions[0], &uri).new_text, expected);
        } else {
            assert!(actions.is_empty(), "{key}: {actions:?}");
        }
    }
}

fn undefined_command_diagnostic(workspace: &mut Workspace, uri: &Url, name: &str) -> Diagnostic {
    let text = &workspace.documents[uri].text;
    let start = text.source.find(&format!("\\{name}")).unwrap();
    let diagnostic = Diagnostic {
        range: text.range(start, start + name.len() + 1),
        source: Some("latex-build".into()),
        message: "Undefined control sequence.".into(),
        code: Some(NumberOrString::String("undefined-control-sequence".into())),
        data: Some(serde_json::json!({"command":name})),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        uri.clone(),
        std::collections::BTreeMap::from([(uri.clone(), vec![diagnostic.clone()])]),
    );
    diagnostic
}

#[test]
fn undefined_command_fixes_use_builtin_and_project_commands_preserving_arguments_and_utf16() {
    for (name, replacement) in [
        ("sectoin", "section"),
        ("reporttitel", "reporttitle"),
        ("documentwidht", "documentwidth"),
        ("ifdrafft", "ifdraft"),
        ("thecliam", "theclaim"),
        ("endpanle", "endpanel"),
        ("wriet", "write"),
        ("escapcehar", "escapechar"),
    ] {
        let source = format!("😀 \\{name}{{Existing}} trailing text.\n");
        let (mut workspace, uri) = setup(&source, true);
        let declarations = Url::parse("file:///editing/definitions.tex").unwrap();
        let unrelated = Url::parse("file:///unrelated/main.tex").unwrap();
        workspace.documents.insert(declarations.clone(), Document::new(&declarations, r"\newcommand{\reporttitle}[1]{#1}\newlength{\documentwidth}\newif\ifdraft\newtheorem{claim}{Claim}\newenvironment{panel}{}{}".into(), None));
        workspace.documents.insert(
            unrelated.clone(),
            Document::new(&unrelated, format!("\\newcommand{{\\{name}}}{{}}"), None),
        );
        workspace
            .recorded_inputs
            .insert(uri.clone(), [uri.clone(), declarations].into());
        let diagnostic = undefined_command_diagnostic(&mut workspace, &uri, name);
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic]),
        );
        let expected = format!("\\{replacement}");
        let action = actions
            .iter()
            .find(|action| action_edit(action, &uri).new_text == expected)
            .unwrap_or_else(|| panic!("{name}: {actions:?}"));
        let edit = action_edit(action, &uri);
        let changed = workspace.documents[&uri]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text,
            }])
            .unwrap();
        assert_eq!(
            changed.source,
            format!("😀 \\{replacement}{{Existing}} trailing text.\n")
        );
        let CodeActionOrCommand::CodeAction(action) = action else {
            panic!()
        };
        assert_eq!(action.is_preferred, Some(false));
        assert!(action.title.contains(&expected));
        assert!(actions.len() <= 5);
    }
}

#[test]
fn command_spelling_fixes_reject_known_ambiguous_ignored_and_unproven_names() {
    for (source, name, definitions, mode) in [
        (r"\section{Title}", "section", "", "precise"),
        (r"\write{Known primitive}", "write", "", "precise"),
        (
            r"\reporttitle{Title}",
            "reporttitle",
            r"\newcommand{\reporttitle}[1]{#1}",
            "precise",
        ),
        (
            r"\reporttitel{Title}",
            "reporttitel",
            r"\newcommand{\reporttitle}[1]{#1}\newcommand{\reporttitle}[1]{#1}",
            "precise",
        ),
        (r"% \sectoin{Title}", "sectoin", "", "precise"),
        (r"\verb|\sectoin{Title}|", "sectoin", "", "precise"),
        (r"\fr{Title}", "fr", "", "precise"),
        (r"\sectoin{Title}", "sectoin", "", "missing-data"),
        (r"\sectoin{Title}", "sectoin", "", "wrong-data"),
        (r"\sectoin{Title}", "sectoin", "", "line-range"),
        (r"\sectoin{Title}", "sectoin", "", "macro-expansion"),
    ] {
        let (mut workspace, uri) = setup(source, false);
        let declarations = Url::parse("file:///editing/definitions.tex").unwrap();
        workspace.documents.insert(
            declarations.clone(),
            Document::new(&declarations, definitions.into(), None),
        );
        workspace
            .recorded_inputs
            .insert(uri.clone(), [uri.clone(), declarations].into());
        let mut diagnostic = undefined_command_diagnostic(&mut workspace, &uri, name);
        match mode {
            "missing-data" => diagnostic.data = None,
            "wrong-data" => diagnostic.data = Some(serde_json::json!({"command":"other"})),
            "line-range" => {
                diagnostic.range = workspace.documents[&uri].text.range(0, source.len())
            }
            "macro-expansion" => diagnostic.code = None,
            _ => {}
        }
        workspace
            .build_diagnostics
            .get_mut(&uri)
            .unwrap()
            .get_mut(&uri)
            .unwrap()[0] = diagnostic.clone();
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic]),
        );
        assert!(actions.is_empty(), "{name}/{mode}: {actions:?}");
    }
}

#[test]
fn command_spelling_requests_revalidate_version_diagnostic_and_replacement() {
    let (mut workspace, uri) = setup(r"\sectoin{Existing}", true);
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let diagnostic = undefined_command_diagnostic(&mut workspace, &uri, "sectoin");
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic]),
    );
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!()
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    assert_eq!(value["replacement"], "\\section");
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_some()
    );
    let mut forged = value.clone();
    forged["replacement"] = serde_json::json!("\\subsection");
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace.documents.get_mut(&uri).unwrap().version = Some(9);
    workspace.clear_build_diagnostics(&uri);
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(value).unwrap()).is_none());
}
