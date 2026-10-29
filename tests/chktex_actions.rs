use lsp_types::*;
use texquill::{
    editing,
    project::workspace::{Document, Workspace},
    text::Text,
};

fn setup(marked: &str) -> (Workspace, Url, Diagnostic) {
    let start = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let uri = Url::parse("untitled:chktex.tex").unwrap();
    let text = Text::new(source.clone());
    // ChkTeX clamps a multiline formula's range to its opening line.
    let end = source[start..]
        .find(['\r', '\n'])
        .map_or(source.len(), |i| start + i);
    let diagnostic = Diagnostic {
        range: text.range(start, end),
        source: Some("chktex".into()),
        code: Some(NumberOrString::String("46".into())),
        message: r"Use \( ... \) instead of $ ... $.".into(),
        ..Default::default()
    };
    let mut ws = Workspace {
        client: texquill::project::state::ClientCapabilities {
            code_action_literals: true,
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    ws.documents
        .insert(uri.clone(), Document::new(&uri, source, Some(7)));
    ws.project
        .lint_diagnostics
        .insert(uri.clone(), vec![diagnostic.clone()]);
    (ws, uri, diagnostic)
}

fn actions(ws: &Workspace, uri: &Url, diagnostic: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        ws,
        uri,
        Range::new(diagnostic.range.start, diagnostic.range.start),
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
}

fn apply(ws: &Workspace, uri: &Url, edit: &WorkspaceEdit) -> String {
    let Some(DocumentChanges::Edits(changes)) = &edit.document_changes else {
        panic!()
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].text_document.version, Some(7));
    let mut edits = changes[0]
        .edits
        .iter()
        .map(|edit| {
            let OneOf::Left(edit) = edit else { panic!() };
            edit.clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(edits.len(), 2);
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let text = &ws.documents[uri].text;
    let mut source = text.source.clone();
    for edit in edits {
        source.replace_range(
            text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
            &edit.new_text,
        );
    }
    source
}

#[test]
fn converts_only_the_selected_pair_preserving_unicode_crlf_comments_and_escaped_dollars() {
    for (marked, expected) in [
        ("😀 ¦$ x + 1 $ and $y$\r\n", "😀 \\( x + 1 \\) and $y$\r\n"),
        (
            "😀 ¦$ x + % $ ignored\r\n y + \\$ $\r\n",
            "😀 \\( x + % $ ignored\r\n y + \\$ \\)\r\n",
        ),
        (
            r"\textbf{before ¦$x$ after}",
            r"\textbf{before \(x\) after}",
        ),
        (r"$first$ and ¦$second$", r"$first$ and \(second\)"),
        (r"¦$x+\mbox{$y$}$", r"\(x+\mbox{$y$}\)"),
    ] {
        let (ws, uri, diagnostic) = setup(marked);
        let items = actions(&ws, &uri, &diagnostic);
        assert_eq!(items.len(), 1, "{marked}: {items:?}");
        let CodeActionOrCommand::CodeAction(action) = &items[0] else {
            panic!()
        };
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(action.diagnostics, Some(vec![diagnostic]));
        assert_eq!(
            apply(&ws, &uri, action.edit.as_ref().unwrap()),
            expected,
            "{marked}"
        );
    }
}

#[test]
fn skips_display_unbalanced_mixed_literal_and_parameter_dollars() {
    for marked in [
        "¦$$x$$",
        "¦$x",
        r"¦$x\)",
        r"\(x¦$",
        r"¦$x\(y$",
        r"\¦$x$",
        "% ¦$x$\n",
        r"\verb|¦$x$|",
        "\\begin{verbatim}\n¦$x$\n\\end{verbatim}",
        r"\newcommand{\cash}{¦$x$}",
        r"\detokenize{¦$x$}",
        r"\string¦$ x$",
        r"\newcommand{\take}[1]{}\take¦$x$",
        r"\label{¦$x$}",
        r"\ensuremath{¦$x$}",
        r"\begin{equation}¦$x$\end{equation}",
        r"\catcode`\$=12 ¦$x$",
        r"\def\({other} ¦$x$",
        r"\let\)=\relax ¦$x$",
        r"¦$x\input{child}y$",
    ] {
        let (ws, uri, diagnostic) = setup(marked);
        assert!(actions(&ws, &uri, &diagnostic).is_empty(), "{marked}");
    }
}

#[test]
fn requires_current_matching_chktex_warning_and_never_enters_fix_all() {
    let (mut ws, uri, diagnostic) = setup("¦$x$");
    assert_eq!(actions(&ws, &uri, &diagnostic).len(), 1);
    let context = CodeActionContext {
        diagnostics: vec![],
        only: Some(vec![CodeActionKind::SOURCE_FIX_ALL]),
        trigger_kind: None,
    };
    assert!(editing::code_actions(&ws, &uri, diagnostic.range, &context).is_empty());
    let mut other = diagnostic.clone();
    other.message = "old warning".into();
    assert!(actions(&ws, &uri, &other).is_empty());
    for (source, code) in [("latex-build", "46"), ("chktex", "45")] {
        let current = &mut ws.project.lint_diagnostics.get_mut(&uri).unwrap()[0];
        current.source = Some(source.into());
        current.code = Some(NumberOrString::String(code.into()));
        assert!(actions(&ws, &uri, &diagnostic).is_empty());
    }
    ws.project.lint_diagnostics.remove(&uri);
    assert!(actions(&ws, &uri, &diagnostic).is_empty());
}

#[test]
fn legacy_command_recomputes_both_edits_and_rejects_stale_version() {
    let (mut ws, uri, diagnostic) = setup("¦$x$");
    ws.client.code_action_literals = false;
    ws.client.apply_edits = true;
    let items = actions(&ws, &uri, &diagnostic);
    let CodeActionOrCommand::Command(command) = &items[0] else {
        panic!()
    };
    assert_eq!(command.command, "latex.applyQuickFix");
    let request = command.arguments.as_ref().unwrap()[0].clone();
    let edit = editing::resolve_fix(&ws, serde_json::from_value(request.clone()).unwrap()).unwrap();
    assert_eq!(apply(&ws, &uri, &edit), r"\(x\)");
    ws.documents.get_mut(&uri).unwrap().version = Some(8);
    assert!(editing::resolve_fix(&ws, serde_json::from_value(request).unwrap()).is_none());
}

#[test]
fn project_inputs_and_related_unsaved_delimiter_redefinitions_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let root = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let macros = Url::from_file_path(dir.path().join("macros.tex")).unwrap();
    let (mut ws, uri, diagnostic) = setup(r"\input{macros} before ¦$x$ after");
    let source = ws.documents.remove(&uri).unwrap().text.source.clone();
    std::fs::write(root.to_file_path().unwrap(), &source).unwrap();
    std::fs::write(macros.to_file_path().unwrap(), "").unwrap();
    ws.project.lint_diagnostics.remove(&uri);
    ws.add_root(dir.path().into());
    ws.open(root.clone(), source, 7);
    ws.load_dependencies(&root, 0);
    ws.project
        .lint_diagnostics
        .insert(root.clone(), vec![diagnostic.clone()]);
    assert_eq!(actions(&ws, &root, &diagnostic).len(), 1);
    ws.open(macros, r"\let\)=\relax".into(), 2);
    assert!(actions(&ws, &root, &diagnostic).is_empty());
}
