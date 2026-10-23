use latex_lsp::{
    editing,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;
fn setup(marked: &str, only: CodeActionKind) -> (Workspace, Url, Range, CodeActionContext) {
    let start = marked.find('¦').unwrap();
    let marked = marked.replacen('¦', "", 1);
    let end = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let uri = Url::parse("file:///refactor/main.tex").unwrap();
    let text = Text::new(source.clone());
    let range = text.range(start, end);
    let mut ws = Workspace {
        code_action_literals: true,
        versioned_edits: true,
        ..Default::default()
    };
    ws.documents
        .insert(uri.clone(), Document::new(&uri, source, Some(4)));
    (
        ws,
        uri,
        range,
        CodeActionContext {
            diagnostics: vec![],
            only: Some(vec![only]),
            trigger_kind: None,
        },
    )
}
fn changed(ws: &Workspace, uri: &Url, actions: &[CodeActionOrCommand], title: &str) -> String {
    let action = actions
        .iter()
        .find_map(|action| match action {
            CodeActionOrCommand::CodeAction(a) if a.title == title => Some(a),
            _ => None,
        })
        .unwrap();
    let Some(DocumentChanges::Edits(edits)) = &action.edit.as_ref().unwrap().document_changes
    else {
        panic!()
    };
    assert_eq!(edits[0].text_document.version, Some(4));
    let mut edits: Vec<_> = edits[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    ws.documents[uri]
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
fn wrap_and_extract_preserve_utf16_crlf_and_control_word_boundaries() {
    let (ws, uri, range, context) = setup("\\documentclass{article}\r\n\\newcommand{\\extractedcommand}{old}\r\n\\begin{document}\r\n😀 ¦hello \\textbf{world}¦ next\r\n\\end{document}", CodeActionKind::REFACTOR);
    let actions = editing::code_actions(&ws, &uri, range, &context);
    assert_eq!(actions.len(), 4, "{actions:?}");
    let wrapped = changed(&ws, &uri, &actions, "Wrap selection in \\emph");
    assert!(wrapped.contains("😀 \\emph{hello \\textbf{world}} next\r\n"));
    let extracted = changed(&ws, &uri, &actions, "Extract selection into a new command");
    assert!(extracted
        .contains("\\newcommand{\\extractedcommandb}{hello \\textbf{world}}\r\n\\begin{document}"));
    assert!(extracted.contains("😀 \\extractedcommandb{} next"));
}
#[test]
fn partial_tokens_arguments_math_special_contexts_and_raw_data_do_not_refactor() {
    for source in [
        r"\textbf{¦partial¦}",
        r"\tex¦tbf{word}¦",
        r"¦\textbf¦{word}",
        r"$¦math¦$",
        r"\(¦math¦\)",
        r"\begin{equation}¦text¦\end{equation}",
        r"\verb|¦raw¦|",
        "% ¦comment¦\n",
        r"¦unbalanced{¦",
        r"¦#1¦",
        r"¦\label{x}¦",
        r"\documentclass{article}¦preamble¦\begin{document}\end{document}",
        r"\begin{document}\end{document}¦after¦",
    ] {
        let (ws, uri, range, context) = setup(source, CodeActionKind::REFACTOR);
        assert!(
            editing::code_actions(&ws, &uri, range, &context).is_empty(),
            "{source}"
        );
    }
}
#[test]
fn fix_all_applies_independent_errors_but_skips_conflicting_insertions() {
    let (ws, uri, range, context) = setup(
        r"¦\end{quote} text \end{itemize}¦",
        CodeActionKind::SOURCE_FIX_ALL,
    );
    let actions = editing::code_actions(&ws, &uri, range, &context);
    let result = changed(
        &ws,
        &uri,
        &actions,
        "Fix all unambiguous LaTeX syntax errors",
    );
    assert_eq!(result, " text ");
    let (ws, uri, range, context) = setup(
        r"¦\begin{quote}\begin{itemize}¦",
        CodeActionKind::SOURCE_FIX_ALL,
    );
    assert!(editing::code_actions(&ws, &uri, range, &context).is_empty());
    let (ws, uri, range, mut context) = setup(r"¦\end{quote}¦", CodeActionKind::REFACTOR);
    assert!(editing::code_actions(&ws, &uri, range, &context).is_empty());
    context.only = None;
    assert!(!editing::code_actions(&ws,&uri,range,&context).iter().any(|a| matches!(a,CodeActionOrCommand::CodeAction(a) if a.kind==Some(CodeActionKind::SOURCE_FIX_ALL))));
}
