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
#[test]
fn completed_expl3_regions_and_deferred_switches_do_not_block_plain_body_text() {
    for (source, expected) in [
        (r"\ExplSyntaxOn\ExplSyntaxOff ¦plain text¦", 3),
        (r"{\ExplSyntaxOn} ¦plain text¦", 3),
        (r"\newcommand{\later}{\ExplSyntaxOn} ¦plain text¦", 3),
        (
            r"\ExplSyntaxOn\cs_new:Npn \demo_later: {\ExplSyntaxOff} ¦plain text¦",
            0,
        ),
    ] {
        let (ws, uri, range, context) = setup(source, CodeActionKind::REFACTOR);
        assert_eq!(
            editing::code_actions(&ws, &uri, range, &context).len(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn refactors_reject_bare_optional_delimited_and_named_parameter_tokens() {
    for source in [
        r"\newcommand{\take}[1]{}\begin{document}\take ¦Hello world¦\end{document}",
        r"\newcommand{\take}[1]{}\begin{document}\take ¦ Hello world¦\end{document}",
        r"\newcommand{\take}[1]{}\begin{document}\csuse{take}¦Hello world¦\end{document}",
        r"\newcommand{\take}[1]{}\begin{document}\csname take\endcsname ¦Hello world¦\end{document}",
        r"\newcommand{\take}[1]{}\def\forward{\take}\begin{document}\forward ¦Hello world¦\end{document}",
        r"\newcommand{\take}[1][default]{}\begin{document}\take[¦Hello world¦]\end{document}",
        r"\newcommand{\take}[1][default]{}\begin{document}\take¦[Hello world]¦\end{document}",
        r"\def\take#1STOP{}\begin{document}\take Before ¦Hello world¦STOP\end{document}",
        r"\NewDocumentCommand{\take}{d<>}{}\begin{document}\take<¦Hello world¦>\end{document}",
        r"\NewDocumentCommand{\take}{t+}{}\begin{document}\take¦+Hello world¦\end{document}",
        r"\begin{document}\section[¦Hello world¦]{Title}\end{document}",
        r"\begin{document}\section¦*¦{Title}\end{document}",
        r"\begin{document}\textbf ¦Hello world¦\end{document}",
        r"\begin{document}\noexpand ¦Hello world¦\end{document}",
        r"\begin{document}\csuse{noexpand}¦ Hello world¦\end{document}",
        r"\begin{document}\csname noexpand\endcsname ¦Hello world¦\end{document}",
        r"\begin{document}\unknown ¦Hello world¦\end{document}",
    ] {
        let (ws, uri, range, context) = setup(source, CodeActionKind::REFACTOR);
        assert!(
            editing::code_actions(&ws, &uri, range, &context).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn consumed_arguments_do_not_block_later_plain_text_or_complete_style_commands() {
    for source in [
        r"\newcommand{\take}[1]{}\begin{document}\take H¦ello world¦\end{document}",
        r"\newcommand{\take}[1][default]{}\begin{document}\take[argument] ¦Hello world¦\end{document}",
        r"\def\take#1STOP{}\begin{document}\take argumentSTOP ¦Hello world¦\end{document}",
        r"\begin{document}\csuse{noexpand} ¦Hello world¦\end{document}",
        r"\begin{document}¦Hello \textbf{world}¦\end{document}",
        r"\begin{document}¦\textbf{Hello world}¦\end{document}",
    ] {
        let (ws, uri, range, context) = setup(source, CodeActionKind::REFACTOR);
        assert_eq!(
            editing::code_actions(&ws, &uri, range, &context).len(),
            4,
            "{source}"
        );
    }
}

#[test]
fn refactor_parameter_context_follows_related_unsaved_definitions_and_ambiguity() {
    let dir = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let macros = Url::from_file_path(dir.path().join("macros.tex")).unwrap();
    let source = "\\documentclass{article}\r\n\\input{macros}\r\n\\begin{document}\r\n😀 \\take Hello world\r\n\\end{document}";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    std::fs::write(macros.to_file_path().unwrap(), "\\newcommand{\\take}[1]{}").unwrap();
    let mut ws = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), source.into(), 1);
    let at = source.find("Hello world").unwrap();
    let range = Text::new(source.into()).range(at, at + "Hello world".len());
    let context = CodeActionContext {
        diagnostics: vec![],
        only: Some(vec![CodeActionKind::REFACTOR]),
        trigger_kind: None,
    };
    assert!(editing::code_actions(&ws, &uri, range, &context).is_empty());
    ws.open(macros.clone(), "\\newcommand{\\take}{}".into(), 1);
    assert_eq!(editing::code_actions(&ws, &uri, range, &context).len(), 4);
    ws.open(
        macros,
        "\\newcommand{\\take}{}\n\\renewcommand{\\take}[1]{}".into(),
        2,
    );
    assert!(editing::code_actions(&ws, &uri, range, &context).is_empty());
}

#[test]
fn structural_refactors_skip_incomplete_project_contexts() {
    let (mut ws, uri, range, context) = setup(
        r"\begin{document}¦Hello¦\end{document}",
        CodeActionKind::REFACTOR,
    );
    ws.index_truncated.insert("maxFiles".into());
    assert!(editing::code_actions(&ws, &uri, range, &context).is_empty());
}

#[test]
#[ignore = "requires pdflatex"]
fn real_refactors_after_consumed_arguments_preserve_tex_tokenization() {
    let marked = r"\documentclass{article}\newcommand{\take}[1]{\typeout{CAPTURE:\detokenize{#1}}}\begin{document}\take H¦ello world¦\end{document}";
    let (ws, uri, range, context) = setup(marked, CodeActionKind::REFACTOR_REWRITE);
    let actions = editing::code_actions(&ws, &uri, range, &context);
    assert_eq!(actions.len(), 3);
    let original = &ws.documents[&uri].text.source;
    let rewritten = changed(&ws, &uri, &actions, "Wrap selection in \\emph");
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        ("original.tex", original.as_str()),
        ("rewritten.tex", rewritten.as_str()),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
        let output = std::process::Command::new("pdflatex")
            .args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                name,
            ])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let log = std::fs::read_to_string(dir.path().join(name.replace(".tex", ".log"))).unwrap();
        assert!(log.lines().any(|line| line == "CAPTURE:H"), "{log}");
    }
}
