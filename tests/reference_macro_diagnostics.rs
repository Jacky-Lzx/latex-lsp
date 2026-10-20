use latex_lsp::{build, build_log, config::BuildConfig, editing, text::Text, workspace::Workspace};
use std::sync::atomic::AtomicBool;
use tower_lsp::lsp_types::*;

fn parse(source: &str, context: &str, numbered: bool) -> Diagnostic {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    let header = if numbered {
        "./main.tex:1: Undefined control sequence."
    } else {
        "! Undefined control sequence."
    };
    let diagnostics = build_log::parse(&main, &format!("(./main.tex\n{header}\n{context}\n)\n"));
    diagnostics[&Url::from_file_path(main).unwrap()]
        .iter()
        .find(|item| item.message == "Undefined control sequence.")
        .unwrap()
        .clone()
}

fn repaired(workspace: &Workspace, uri: &Url, diagnostic: &Diagnostic) -> String {
    let edit = editing::code_actions(
        workspace,
        uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
    .into_iter()
    .find_map(|item| match item {
        CodeActionOrCommand::CodeAction(action)
            if action.title == "Replace with command `\\keyname`" =>
        {
            action.edit
        }
        _ => None,
    })
    .unwrap_or_else(|| panic!("missing spelling fix: {diagnostic:?}"));
    let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
        panic!("missing versioned edit")
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].text_document.uri, *uri);
    assert_eq!(changes[0].text_document.version, Some(7));
    assert_eq!(changes[0].edits.len(), 1);
    let OneOf::Left(edit) = &changes[0].edits[0] else {
        panic!("unexpected annotation")
    };
    workspace.documents[uri]
        .text
        .changed(vec![TextDocumentContentChangeEvent {
            range: Some(edit.range),
            range_length: None,
            text: edit.new_text.clone(),
        }])
        .unwrap()
        .source
}

#[test]
fn key_arguments_pinpoint_only_the_reported_control_word() {
    for call in [
        r"\ref{\keynme}",
        r"\pageref{\keynme}",
        r"\eqref{\keynme}",
        r"\cite{\keynme}",
        r"\nocite{\keynme}",
        r"\ref*{\keynme}",
        r"\pageref *{\keynme}",
        r"\cite[see]{\keynme}",
        r"\cite[see][p. {4]}]{\keynme}",
        r"\cite*[see][p. 4]{\keynme}",
    ] {
        let source = format!("\\typeout{{😀}} {call} Remains\r\n");
        let start = source.find(r"\keynme").unwrap();
        for numbered in [false, true] {
            let diagnostic = parse(
                &source,
                &format!("<argument> \\keynme \n \nl.1 \\typeout{{😀}} {call}"),
                numbered,
            );
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + 7),
                "{call}: {diagnostic:?}"
            );
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("undefined-control-sequence".into()))
            );
            assert_eq!(diagnostic.data.as_ref().unwrap()["command"], "keynme");
        }
    }
    let diagnostic = parse(
        r"Long prefix \ref{\keynme}",
        "<argument> \\keynme\nl.1 ...{\\keynme}",
        true,
    );
    assert!(diagnostic.code.is_some());
}

#[test]
fn uncertain_key_expansions_and_source_echoes_keep_line_diagnostics() {
    for (source, echo) in [
        (r"\ref{prefix\keynme}", r"\ref{prefix\keynme}"),
        (r"\ref{\keynme }", r"\ref{\keynme }"),
        (r"\ref{ \keynme}", r"\ref{ \keynme}"),
        (r"\ref{\keynme\suffix}", r"\ref{\keynme\suffix}"),
        (r"\cite{payload,\keynme}", r"\cite{payload,\keynme}"),
        (r"\cite{\keynme,payload}", r"\cite{\keynme,payload}"),
        (r"\ref\keynme", r"\ref\keynme"),
        (r"\label{\keynme}", r"\label{\keynme}"),
        (r"\crefrange{\keynme}{other}", r"\crefrange{\keynme}{other}"),
        (r"\eqref*{\keynme}", r"\eqref*{\keynme}"),
        (r"\nocite*{\keynme}", r"\nocite*{\keynme}"),
        (r"\nocite[note]{\keynme}", r"\nocite[note]{\keynme}"),
        (r"\ref[note]{\keynme}", r"\ref[note]{\keynme}"),
        (r"\cite[a][b][c]{\keynme}", r"\cite[a][b][c]{\keynme}"),
        (r"% \ref{\keynme}", r"% \ref{\keynme}"),
        (r"\verb|\ref{\keynme}|", r"\verb|\ref{\keynme}|"),
        (r"\detokenize{\ref{\keynme}}", r"\detokenize{\ref{\keynme}}"),
        (
            r"\def\wrapper{\ref{\keynme}}\wrapper",
            r"\def\wrapper{\ref{\keynme}}\wrapper",
        ),
        (r"\ref{\nested}", r"\ref{\nested}"),
        (r"\ref{\keynme}", r"Changed \ref{\keynme}"),
        (r"\ref{\keynme} \cite{\keynme}", r"...{\keynme}"),
        (r"\ref{\keynme}", r"...keynme}"),
    ] {
        for numbered in [false, true] {
            let diagnostic = parse(
                source,
                &format!("<argument> \\keynme\nl.1 {echo}"),
                numbered,
            );
            assert!(
                diagnostic.code.is_none() && diagnostic.data.is_none(),
                "{source}: {diagnostic:?}"
            );
        }
    }
    for context in [
        "<argument> \\keynme\n\\wrapper -> \\ref {\\keynme}\nl.1 \\ref{\\keynme}",
        "<argument> \\keynme\nl.2 \\ref{\\keynme}",
        "<argument> \\keynme\nl.1 ...{\\keynme",
    ] {
        let diagnostic = parse(r"\ref{\keynme}", context, true);
        assert!(diagnostic.code.is_none(), "{context}: {diagnostic:?}");
    }
}

#[test]
fn key_macro_diagnostics_reuse_versioned_command_spelling_fixes() {
    for call in [
        r"\ref{\keynme}",
        r"\pageref{\keynme}",
        r"\eqref{\keynme}",
        r"\cite[see]{\keynme}",
        r"\nocite{\keynme}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let uri = Url::from_file_path(&main).unwrap();
        let source = format!("\\newcommand{{\\keyname}}{{payload}}{call}");
        std::fs::write(&main, &source).unwrap();
        let diagnostics = build_log::parse(&main, &format!("(./main.tex\n./main.tex:1: Undefined control sequence.\n<argument> \\keynme\nl.1 {source}\n)\n"));
        let diagnostic = diagnostics[&uri][0].clone();
        let mut workspace = Workspace {
            versioned_edits: true,
            code_action_literals: true,
            ..Default::default()
        };
        workspace.open(uri.clone(), source.clone(), 7);
        workspace.build_diagnostics.insert(uri.clone(), diagnostics);
        assert_eq!(
            repaired(&workspace, &uri, &diagnostic),
            source.replace(r"\keynme", r"\keyname")
        );
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_key_macro_repairs_restore_successful_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for (packages, call) in [
            ("", r"\ref{\keynme}"),
            ("", r"\pageref{\keynme}"),
            ("", r"\eqref{\keynme}"),
            ("", r"\cite{\keynme}"),
            ("", r"\nocite{\keynme}"),
            ("\\usepackage{hyperref}\r\n", r"\ref{\keynme}"),
            ("\\usepackage{hyperref}\r\n", r"\pageref{\keynme}"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let uri = Url::from_file_path(&main).unwrap();
            let source = format!("\\documentclass{{article}}\r\n\\usepackage{{amsmath}}\r\n{packages}\\newcommand{{\\keyname}}{{payload}}\r\n\\begin{{document}}\r\n\\section{{MarkerUnique}}\\label{{payload}}\r\n\\begin{{thebibliography}}{{1}}\\bibitem{{payload}}Entry\\end{{thebibliography}}\r\n{call}\r\n\\end{{document}}\r\n");
            std::fs::write(&main, &source).unwrap();
            let config = BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let before = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(before.status, "error", "{engine}/{call}: {}", before.output);
            let start = source.find(r"\keynme").unwrap();
            let range = Text::new(source.clone()).range(start, start + 7);
            let diagnostic = before.diagnostics[&uri]
                .iter()
                .find(|item| item.message == "Undefined control sequence." && item.range == range)
                .unwrap_or_else(|| {
                    panic!(
                        "{engine}/{call}: {}\n{:?}",
                        before.output, before.diagnostics
                    )
                });
            let mut workspace = Workspace {
                versioned_edits: true,
                code_action_literals: true,
                ..Default::default()
            };
            workspace.open(uri.clone(), source, 7);
            workspace
                .build_diagnostics
                .insert(uri.clone(), before.diagnostics.clone());
            std::fs::write(&main, repaired(&workspace, &uri, diagnostic)).unwrap();
            let after = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(main
                .parent()
                .unwrap()
                .join("build/latex-lsp/main.pdf")
                .is_file());
        }
    }
}
