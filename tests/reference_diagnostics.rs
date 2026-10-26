use latex_lsp::{editing, project::workspace::Workspace, text::Text};
use lsp_types::*;

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
    ws.client.code_action_literals = true;
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

#[test]
fn static_named_reference_and_citation_calls_report_exact_key_ranges() {
    for (call, keys) in [
        (r"\csuse{ref}{missing}", vec!["missing"]),
        (r"\csname ref\endcsname{missing}", vec!["missing"]),
        (r"\csuse{cref}{first, second}", vec!["first", "second"]),
        (r"\csuse{crefrange}{first}{second}", vec!["first", "second"]),
        (r"\csuse{vref}[note]{missing}", vec!["missing"]),
        (r"\csuse{hyperref}[missing]{link text}", vec!["missing"]),
        (r"\csuse{setcounterref}{counter}{missing}", vec!["missing"]),
        (
            r"\csuse{cite}[before][after]{first, second}",
            vec!["first", "second"],
        ),
        (
            r"\csname cites\endcsname[prefix]{first}[suffix]{second}",
            vec!["first", "second"],
        ),
        (
            r"\csuse{ref}{%comment
missing%comment
}",
            vec!["missing"],
        ),
        (r"\csuse{ref}{literal(key)}", vec!["literal(key)"]),
    ] {
        let source = format!("😀\r\n{call}");
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        let uri = Url::parse("untitled:named-reference.tex").unwrap();
        ws.open(uri.clone(), source.clone(), 1);
        let diagnostics = ws.diagnostics(&uri);
        let undefined: Vec<_> = diagnostics
            .iter()
            .filter(|diag| {
                diag.message.starts_with("Undefined reference")
                    || diag.message.starts_with("Undefined citation")
            })
            .collect();
        assert_eq!(undefined.len(), keys.len(), "{source}: {diagnostics:?}");
        let text = Text::new(source.clone());
        for key in keys {
            let at = source.find(key).unwrap();
            assert!(
                undefined
                    .iter()
                    .any(|diag| diag.range == text.range(at, at + key.len())),
                "{key}: {diagnostics:?}"
            );
        }
    }
}

#[test]
fn static_named_references_respect_interface_target_and_parameter_semantics() {
    for source in [
        r"\renewcommand{\ref}[1]{#1}\csuse{ref}{ordinary}\csname ref\endcsname{ordinary}",
        r"\renewcommand{\cite}[1]{#1}\csuse{cite}{ordinary}",
        r"\def\csuse#1#2{}\csuse{ref}{ordinary}",
        r"\def\endcsname{}\csname ref\endcsname{ordinary}",
        r"\newcommand{\discard}[1]{}\discard{\csuse{ref}{ordinary}\csname cite\endcsname{ordinary}}",
        r"\noexpand\csuse{ref}{ordinary}\noexpand\csname ref\endcsname{ordinary}",
        r"\csuse{\dynamic}{ordinary}\csname ref\dynamic\endcsname{ordinary}",
        r"\newcommand{\later}{\csuse{ref}{ordinary}}",
        r"\csuse{ref}*{\dynamic}",
    ] {
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        let uri = Url::parse("untitled:named-reference.tex").unwrap();
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
fn named_reference_spelling_repairs_replace_only_the_actual_key() {
    for (source, replacement) in [
        (
            r"\label{introduction}\csuse{ref}{introdution}",
            "introduction",
        ),
        (
            r"\bibitem{introduction}\csname cite\endcsname{introdution}",
            "introduction",
        ),
    ] {
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        ws.client.code_action_literals = true;
        let uri = Url::parse("untitled:named-reference-fix.tex").unwrap();
        ws.open(uri.clone(), source.into(), 1);
        let diagnostic = ws
            .diagnostics(&uri)
            .into_iter()
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
        let action = actions
            .iter()
            .find_map(|item| match item {
                CodeActionOrCommand::CodeAction(action) if action.title.contains(replacement) => {
                    Some(action)
                }
                _ => None,
            })
            .unwrap();
        let edits = &action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri];
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range, diagnostic.range);
        assert_eq!(edits[0].new_text, replacement);
        let updated = ws.documents[&uri]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edits[0].range),
                range_length: None,
                text: edits[0].new_text.clone(),
            }])
            .unwrap()
            .source;
        ws.open(uri.clone(), updated, 2);
        assert!(!ws
            .diagnostics(&uri)
            .iter()
            .any(|diag| diag.message.starts_with("Undefined")));
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn real_static_named_references_agree_with_tex_undefined_key_warnings() {
    let source = "\\documentclass{article}\n\\usepackage{etoolbox}\n\\begin{document}\n\\csuse{ref}{missing} \\csname ref\\endcsname{other} \\csuse{cite}{absent}\n\\end{document}";
    let dir = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), source.into(), 1);
    let diagnostics = ws.diagnostics(&uri);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diag| diag.message.starts_with("Undefined"))
            .count(),
        3
    );
    let output = std::process::Command::new("pdflatex")
        .args([
            "-no-shell-escape",
            "-interaction=nonstopmode",
            "-halt-on-error",
            "main.tex",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
    for (kind, key) in [
        ("Reference", "missing"),
        ("Reference", "other"),
        ("Citation", "absent"),
    ] {
        assert!(log.contains(&format!("{kind} `{key}'")), "{log}");
    }
}
