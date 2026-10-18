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
