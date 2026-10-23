use latex_lsp::{editing, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

#[test]
fn static_references_update_without_building_and_remain_project_scoped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.tex"),
        "\\documentclass{article}\n\\input{chapter}\n\\bibliography{refs}",
    )
    .unwrap();
    std::fs::write(dir.path().join("chapter.tex"), "\\label{known}").unwrap();
    std::fs::write(
        dir.path().join("refs.bib"),
        "@book{paper,title={Book},ids={alias}}",
    )
    .unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    let source = "\\documentclass{article}\n\\input{chapter}\n\\bibliography{refs}\n😀 \\cref{known,missing} \\crefrange{known}{other}\n\\cite{paper,alias,absent} \\nocite{*}\n% \\ref{comment}\n\\verb|\\ref{literal}|\n\\ref{\\dynamic}\n";
    ws.open(uri.clone(), source.into(), 1);
    let diagnostics = ws.diagnostics(&uri);
    let undefined: Vec<_> = diagnostics.iter().filter(|diag| matches!(&diag.code,Some(NumberOrString::String(code)) if code.starts_with("undefined-"))).collect();
    assert_eq!(undefined.len(), 3, "{diagnostics:?}");
    let text = Text::new(source.into());
    for key in ["missing", "other", "absent"] {
        assert!(undefined.iter().any(|diag| diag.range
            == text.range(
                source.find(key).unwrap(),
                source.find(key).unwrap() + key.len()
            )));
    }
    let other = Url::from_file_path(dir.path().join("independent.tex")).unwrap();
    std::fs::write(
        dir.path().join("independent.tex"),
        "\\documentclass{article}",
    )
    .unwrap();
    ws.open(other, "\\documentclass{article}\\label{missing}".into(), 1);
    assert!(ws
        .diagnostics(&uri)
        .iter()
        .any(|diag| diag.message == "Undefined reference `missing`"));
    ws.open(
        uri.clone(),
        format!("{source}\\label{{missing}}\\label{{other}}\\bibitem{{absent}}"),
        2,
    );
    assert!(!ws
        .diagnostics(&uri)
        .iter()
        .any(|diag| diag.message.starts_with("Undefined")));
}

#[test]
fn static_diagnostics_can_be_disabled_and_offer_existing_spelling_fixes() {
    let mut ws = Workspace::default();
    let uri = Url::parse("file:///unsaved/main.tex").unwrap();
    ws.open(
        uri.clone(),
        "\\label{introduction}\\ref{introdution}".into(),
        1,
    );
    ws.code_action_literals = true;
    let diagnostics = ws.diagnostics(&uri);
    let diagnostic = diagnostics
        .iter()
        .find(|diag| diag.message.starts_with("Undefined"))
        .unwrap();
    let actions = editing::code_actions(
        &ws,
        &uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            ..Default::default()
        },
    );
    assert!(!actions.is_empty(), "{actions:?}");
    ws.config.diagnostics.undefined_references = false;
    assert!(ws.diagnostics(&uri).is_empty());
}

#[test]
fn static_reference_checks_reject_data_definitions_and_changed_builtin_meanings() {
    for source in [
        r"\renewcommand{\ref}[1]{#1}\ref{ordinary}",
        r"\renewcommand{\cite}[1]{#1}\cite{ordinary}",
        r"\newcommand{\discard}[1]{}\discard{\ref{ordinary}\cite{ordinary}}",
        r"\newcommand{\discard}[1][]{}\discard[\ref{ordinary}\cite{ordinary}]",
        r"\def\discard#1STOP{}\discard\ref{ordinary}\cite{ordinary}STOP",
        r"\newcommand{\later}{\ref{ordinary}\cite{ordinary}}",
        r"\newenvironment{later}{\ref{ordinary}}{\cite{ordinary}}",
        r"\ExplSyntaxOn\cs_new:Npn \later: {\ref{ordinary}\cite{ordinary}}\ExplSyntaxOff",
        r"\detokenize{\ref{ordinary}\cite{ordinary}}",
        r"\noexpand\ref{ordinary}\csuse{noexpand}\cite{ordinary}",
        r"\let\saved=\ref{ordinary}\meaning\cite{ordinary}",
        r"\ifstrequal{\ref{ordinary}}{\cite{ordinary}}{}{}",
    ] {
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        let uri = Url::parse("untitled:reference-context.tex").unwrap();
        ws.open(uri.clone(), source.into(), 1);
        let diagnostics = ws.diagnostics(&uri);
        assert!(
            !diagnostics
                .iter()
                .any(|diag| diag.message.starts_with("Undefined reference")
                    || diag.message.starts_with("Undefined citation")),
            "{source}: {diagnostics:?}"
        );
    }
}

#[test]
fn static_reference_checks_keep_executable_styling_and_single_token_boundaries() {
    for source in [
        r"\textbf{\ref{missing}}\emph{\cite{absent}}",
        r"\newcommand{\take}[1]{}\take X\ref{missing}\cite{absent}",
        r"\noexpand\relax\ref{missing}\cite{absent}",
        r"\csuse{noexpand} \ref{missing}\cite{absent}",
        r"\begingroup\ref{missing}\cite{absent}\endgroup",
    ] {
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        let uri = Url::parse("untitled:reference-context.tex").unwrap();
        ws.open(uri.clone(), source.into(), 1);
        let diagnostics = ws.diagnostics(&uri);
        let undefined: Vec<_> = diagnostics
            .iter()
            .filter(|diag| {
                diag.message.starts_with("Undefined reference")
                    || diag.message.starts_with("Undefined citation")
            })
            .collect();
        assert_eq!(undefined.len(), 2, "{source}: {diagnostics:?}");
        for name in ["missing", "absent"] {
            let at = source.find(name).unwrap();
            assert!(undefined
                .iter()
                .any(|diag| diag.range == Text::new(source.into()).range(at, at + name.len())));
        }
    }
}

#[test]
fn reference_context_uses_related_unsaved_definitions_and_project_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let macros = Url::from_file_path(dir.path().join("macros.tex")).unwrap();
    std::fs::write(dir.path().join("macros.tex"), r"\newcommand{\take}[1]{}").unwrap();
    std::fs::write(
        dir.path().join("other.tex"),
        r"\documentclass{article}\renewcommand{\ref}[1]{#1}",
    )
    .unwrap();
    let source = r"\documentclass{article}\input{macros}\take{\ref{missing}}";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    ws.open(uri.clone(), source.into(), 1);
    let undefined = |ws: &Workspace| {
        ws.diagnostics(&uri)
            .iter()
            .filter(|diag| diag.message == "Undefined reference `missing`")
            .count()
    };
    assert_eq!(undefined(&ws), 0);
    ws.open(macros.clone(), r"\newcommand{\take}{}".into(), 1);
    assert_eq!(undefined(&ws), 1);
    ws.open(
        macros.clone(),
        r"\newcommand{\take}{}\renewcommand{\ref}[1]{#1}".into(),
        2,
    );
    assert_eq!(undefined(&ws), 0);
    ws.open(macros, r"\newcommand{\take}[1]{}".into(), 3);
    assert_eq!(undefined(&ws), 0);
}

#[test]
fn incomplete_reference_contexts_do_not_claim_undefined_builtin_calls() {
    let uri = Url::parse("untitled:incomplete-reference.tex").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), r"\ref{missing}\cite{absent}".into(), 1);
    ws.index_truncated.insert("maxFiles".into());
    assert!(!ws
        .diagnostics(&uri)
        .iter()
        .any(|diag| diag.message.starts_with("Undefined")));
    ws.index_truncated.clear();
    assert_eq!(
        ws.diagnostics(&uri)
            .iter()
            .filter(|diag| diag.message.starts_with("Undefined"))
            .count(),
        2
    );
}
