use lsp_types::*;
use std::sync::atomic::AtomicBool;
use texquill::{
    compilation::{build, build_log},
    config::BuildConfig,
    editing,
    project::workspace::Workspace,
    text::Text,
};

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
            client: texquill::project::state::ClientCapabilities {
                versioned_edits: true,
                code_action_literals: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.open(uri.clone(), source.clone(), 7);
        workspace
            .project
            .build_diagnostics
            .insert(uri.clone(), diagnostics);
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
                client: texquill::project::state::ClientCapabilities {
                    versioned_edits: true,
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.open(uri.clone(), source, 7);
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), before.diagnostics.clone());
            std::fs::write(&main, repaired(&workspace, &uri, diagnostic)).unwrap();
            let after = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(main
                .parent()
                .unwrap()
                .join("build/texquill/main.pdf")
                .is_file());
        }
    }
}

#[test]
fn additional_key_interfaces_pinpoint_direct_macros_with_their_own_argument_grammar() {
    for call in [
        "\\autoref{\\keynme}",
        "\\autoref*{\\keynme}",
        "\\nameref{\\keynme}",
        "\\nameref*{\\keynme}",
        "\\vref{\\keynme}",
        "\\Vref{\\keynme}",
        "\\citet{\\keynme}",
        "\\citep{\\keynme}",
        "\\citeauthor{\\keynme}",
        "\\citeyear{\\keynme}",
        "\\citenum{\\keynme}",
        "\\hyperref[\\keynme]{Text}",
    ] {
        let consumed = if call.starts_with(r"\hyperref[") {
            &call[..call.find(']').unwrap() + 1]
        } else {
            call
        };
        let source = format!("\\typeout{{😀}} {call} Remains\r\n");
        let start = source.find(r"\keynme").unwrap();
        for numbered in [false, true] {
            let diagnostic = parse(
                &source,
                &format!("<argument> \\keynme\nl.1 \\typeout{{😀}} {consumed}"),
                numbered,
            );
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + 7),
                "{call}: {diagnostic:?}"
            );
            assert_eq!(diagnostic.data.as_ref().unwrap()["command"], "keynme");
        }
    }
    for call in [
        r"\autoref{prefix\keynme}",
        r"\nameref{\keynme }",
        r"\citep{payload,\keynme}",
        r"\citenum*[see]{\keynme}",
        r"\hyperref*[\keynme]{Text}",
        r"\hyperref{\keynme}{category}{name}{Text}",
    ] {
        let diagnostic = parse(call, &format!("<argument> \\keynme\nl.1 {call}"), true);
        assert!(diagnostic.code.is_none(), "{call}: {diagnostic:?}");
    }
    let diagnostic = parse(
        r"\vpageref{\keynme}",
        "<argument> sub@\\keynme\nl.1 \\vpageref{\\keynme}",
        true,
    );
    assert!(diagnostic.code.is_none());
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_additional_key_macro_repairs_restore_successful_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for (packages, call) in [
            ("\\usepackage{hyperref}\n", "\\autoref{\\keynme}"),
            ("\\usepackage{hyperref}\n", "\\autoref*{\\keynme}"),
            ("\\usepackage{hyperref}\n", "\\nameref{\\keynme}"),
            ("\\usepackage{hyperref}\n", "\\nameref*{\\keynme}"),
            ("\\usepackage{varioref}\n", "\\vref{\\keynme}"),
            ("\\usepackage{varioref}\n", "\\Vref{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citet{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citep{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citeauthor{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citeyear{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citenum{\\keynme}"),
            ("\\usepackage{hyperref}\n", "\\hyperref[\\keynme]{Text}"),
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
                client: texquill::project::state::ClientCapabilities {
                    versioned_edits: true,
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.open(uri.clone(), source, 7);
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), before.diagnostics.clone());
            std::fs::write(&main, repaired(&workspace, &uri, diagnostic)).unwrap();
            let after = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(main
                .parent()
                .unwrap()
                .join("build/texquill/main.pdf")
                .is_file());
        }
    }
}

#[test]
fn remaining_direct_key_interfaces_and_braced_hyperref_macros_keep_precise_ranges() {
    for call in [
        "\\Ref{\\keynme}",
        "\\autopageref{\\keynme}",
        "\\bibentry{\\keynme}",
        "\\hyperref[{\\keynme}]{Text}",
        "\\citeyearpar{\\keynme}",
        "\\citealt{\\keynme}",
        "\\citealp{\\keynme}",
        "\\citefullauthor{\\keynme}",
        "\\Citet{\\keynme}",
        "\\Citep{\\keynme}",
        "\\Citealt{\\keynme}",
        "\\Citealp{\\keynme}",
        "\\Citeauthor{\\keynme}",
        "\\citetalias{\\keynme}",
        "\\citepalias{\\keynme}",
    ] {
        let consumed = if call.starts_with(r"\hyperref[") {
            &call[..call.find(']').unwrap() + 1]
        } else {
            call
        };
        let source = format!("\\typeout{{😀}} {call} Remains\r\n");
        let start = source.find(r"\keynme").unwrap();
        for numbered in [false, true] {
            let diagnostic = parse(
                &source,
                &format!("<argument> \\keynme\nl.1 \\typeout{{😀}} {consumed}"),
                numbered,
            );
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + 7),
                "{call}: {diagnostic:?}"
            );
            assert_eq!(diagnostic.data.as_ref().unwrap()["command"], "keynme");
        }
    }
    for call in [
        "\\bibentry*{\\keynme}",
        "\\bibentry[note]{\\keynme}",
        "\\citefullauthor*{\\keynme}",
        "\\Citep{payload,\\keynme}",
        "\\hyperref[{\\keynme}suffix]{Text}",
        "\\hyperref[{{\\keynme}}]{Text}",
        "\\hyperref[{\\keynme }]{Text}",
    ] {
        let diagnostic = parse(call, &format!("<argument> \\keynme\nl.1 {call}"), true);
        assert!(diagnostic.code.is_none(), "{call}: {diagnostic:?}");
    }
    let diagnostic = parse(
        r"\vpageref{\keynme}",
        "<argument> sub@\\keynme\nl.1 \\vpageref{\\keynme}",
        true,
    );
    assert!(diagnostic.code.is_none());
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_remaining_direct_key_macro_repairs_restore_successful_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for (packages, call) in [
            ("\\usepackage{hyperref}\n", "\\Ref{\\keynme}"),
            ("\\usepackage{hyperref}\n", "\\autopageref{\\keynme}"),
            ("\\usepackage{bibentry}\n", "\\bibentry{\\keynme}"),
            ("\\usepackage{hyperref}\n", "\\hyperref[{\\keynme}]{Text}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citeyearpar{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citealt{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citealp{\\keynme}"),
            (
                "\\usepackage[numbers]{natbib}\n",
                "\\citefullauthor{\\keynme}",
            ),
            ("\\usepackage[numbers]{natbib}\n", "\\Citet{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\Citep{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\Citealt{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\Citealp{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\Citeauthor{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citetalias{\\keynme}"),
            ("\\usepackage[numbers]{natbib}\n", "\\citepalias{\\keynme}"),
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
                client: texquill::project::state::ClientCapabilities {
                    versioned_edits: true,
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.open(uri.clone(), source, 7);
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), before.diagnostics.clone());
            std::fs::write(&main, repaired(&workspace, &uri, diagnostic)).unwrap();
            let after = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(main
                .parent()
                .unwrap()
                .join("build/texquill/main.pdf")
                .is_file());
        }
    }
}

#[test]
fn prefixed_reference_frames_require_matching_interfaces_and_exact_macro_operands() {
    for call in [
        "\\vpageref{\\keynme}",
        "\\vpageref*{\\keynme}",
        "\\vpageref[here]{\\keynme}",
        "\\vpageref[here][prefix]{\\keynme}",
        "\\vpageref*[here][prefix]{\\keynme}",
        "\\getrefnumber{\\keynme}",
        "\\getpagerefnumber{\\keynme}",
        "\\refused{\\keynme}",
        "\\subref{\\keynme}",
        "\\subref*{\\keynme}",
    ] {
        let prefix = if call.starts_with(r"\subref") {
            "sub@"
        } else {
            "r@"
        };
        let source = format!("\\typeout{{😀}} {call} Remains\r\n");
        let start = source.find(r"\keynme").unwrap();
        for numbered in [false, true] {
            let diagnostic = parse(
                &source,
                &format!("<argument> {prefix}\\keynme\nl.1 \\typeout{{😀}} {call}"),
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
    for (call, frame) in [
        (r"\ref{\keynme}", r"r@\keynme"),
        (r"\vpageref{\keynme}", r"sub@\keynme"),
        (r"\subref{\keynme}", r"r@\keynme"),
        (r"\subref{\keynme}", r"\keynme"),
        (r"\vpageref{\keynme}", r"\keynme"),
        (r"\getrefnumber{prefix\keynme}", r"r@prefix\keynme"),
        (r"\getrefnumber{\keynme\other}", r"r@\keynme"),
        (r"\getrefnumber*{\keynme}", r"r@\keynme"),
        (r"\getpagerefnumber[note]{\keynme}", r"r@\keynme"),
        (r"\vpageref[a][b][c]{\keynme}", r"r@\keynme"),
        (r"\subref{\keynme}", r"sub@\other"),
        (r"\unknown{\keynme}", r"r@\keynme"),
        (r"\setcounterref{counter}{\keynme}", r"\keynme"),
        (r"\def\wrapper{\subref{\keynme}}\wrapper", r"sub@\keynme"),
        (r"\subref{\keynme}", r"r@sub@\keynme"),
    ] {
        for numbered in [false, true] {
            let diagnostic = parse(call, &format!("<argument> {frame}\nl.1 {call}"), numbered);
            assert!(diagnostic.code.is_none(), "{call}/{frame}: {diagnostic:?}");
        }
    }
    let diagnostic = parse(
        r"\vpageref{\keynme}\getrefnumber{\keynme}",
        "<argument> r@\\keynme\nl.1 ...{\\keynme}",
        true,
    );
    assert!(diagnostic.code.is_none());
    let diagnostic = parse(
        r"\subref{\keynme}",
        "<argument> sub@\\keynme\n\\wrapper -> \\subref {\\keynme}\nl.1 \\subref{\\keynme}",
        true,
    );
    assert!(diagnostic.code.is_none());
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_prefixed_key_macro_repairs_restore_successful_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for (packages, call) in [
            ("\\usepackage{varioref}\n", "\\vpageref{\\keynme}"),
            ("\\usepackage{varioref}\n", "\\vpageref*{\\keynme}"),
            ("\\usepackage{varioref}\n", "\\vpageref[here]{\\keynme}"),
            (
                "\\usepackage{varioref}\n",
                "\\vpageref[here][prefix]{\\keynme}",
            ),
            (
                "\\usepackage{varioref}\n",
                "\\vpageref*[here][prefix]{\\keynme}",
            ),
            ("\\usepackage{hyperref}\n", "\\getrefnumber{\\keynme}"),
            ("\\usepackage{hyperref}\n", "\\getpagerefnumber{\\keynme}"),
            ("\\usepackage{hyperref}\n", "\\refused{\\keynme}"),
            ("\\usepackage{subcaption}\n", "\\subref{\\keynme}"),
            ("\\usepackage{subcaption}\n", "\\subref*{\\keynme}"),
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
                client: texquill::project::state::ClientCapabilities {
                    versioned_edits: true,
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.open(uri.clone(), source, 7);
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), before.diagnostics.clone());
            std::fs::write(&main, repaired(&workspace, &uri, diagnostic)).unwrap();
            let after = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(main
                .parent()
                .unwrap()
                .join("build/texquill/main.pdf")
                .is_file());
        }
    }
}

#[test]
fn refcount_macro_errors_follow_each_interfaces_actual_consumption_boundary() {
    for (call, consumed, frame) in [
        (
            r"\getrefbykeydefault{\keynme}{}{\other}",
            r"\getrefbykeydefault{\keynme}{}",
            r"\keynme",
        ),
        (
            r"\getrefbykeydefault{\keynme}{page}{0}",
            r"\getrefbykeydefault{\keynme}{page}",
            r"\keynme",
        ),
        (
            r"\getrefbykeydefault{\keynme}{anchor}{Missing}",
            r"\getrefbykeydefault{\keynme}{anchor}",
            r"\keynme",
        ),
        (
            r"\IfRefUndefinedExpandable{\keynme}{Missing}{Found}",
            r"\IfRefUndefinedExpandable{\keynme}",
            r"r@\keynme",
        ),
        (
            r"\IfRefUndefinedBabel{\keynme}{Missing}{Found}",
            r"\IfRefUndefinedBabel{\keynme}",
            r"\keynme",
        ),
        (
            r"\setcounterref{counter}{\keynme}",
            r"\setcounterref{counter}{\keynme}",
            r"r@\keynme",
        ),
        (
            r"\addtocounterref{counter}{\keynme}",
            r"\addtocounterref{counter}{\keynme}",
            r"r@\keynme",
        ),
        (
            r"\setcounterpageref{counter}{\keynme}",
            r"\setcounterpageref{counter}{\keynme}",
            r"r@\keynme",
        ),
        (
            r"\addtocounterpageref{counter}{\keynme}",
            r"\addtocounterpageref{counter}{\keynme}",
            r"r@\keynme",
        ),
        (
            r"\getrefbykeydefault {\keynme} {page} {\other}",
            r"\getrefbykeydefault {\keynme} {page}",
            r"\keynme",
        ),
        (
            r"\IfRefUndefinedBabel {\keynme}{\other}{\other}",
            r"\IfRefUndefinedBabel {\keynme}",
            r"\keynme",
        ),
    ] {
        let source = format!("\\typeout{{😀}} {call} Remains\r\n");
        let start = source.find(r"\keynme").unwrap();
        for numbered in [false, true] {
            let diagnostic = parse(
                &source,
                &format!("<argument> {frame}\nl.1 \\typeout{{😀}} {consumed}"),
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
}

#[test]
fn refcount_macro_errors_reject_dynamic_peers_wrong_frames_and_inexact_contexts() {
    for (call, echo, frame) in [
        (
            r"\getrefbykeydefault{\keynme}{ }{0}",
            r"\getrefbykeydefault{\keynme}{ }",
            r"\keynme",
        ),
        (
            r"\getrefbykeydefault{\keynme}{\selector}{0}",
            r"\getrefbykeydefault{\keynme}{\selector}",
            r"\keynme",
        ),
        (
            r"\getrefbykeydefault{\keynme}{ page}{0}",
            r"\getrefbykeydefault{\keynme}{ page}",
            r"\keynme",
        ),
        (
            r"\getrefbykeydefault{\keynme}{page}{0}",
            r"\getrefbykeydefault{\keynme}",
            r"\keynme",
        ),
        (
            r"\getrefbykeydefault{\keynme}{page}{0}",
            r"\getrefbykeydefault{\keynme}{page}{0}",
            r"\keynme",
        ),
        (
            r"\getrefbykeydefault{\keynme}{page}{0}",
            r"\getrefbykeydefault{\keynme}{page}",
            r"r@\keynme",
        ),
        (
            r"\getrefbykeydefault*{\keynme}{page}{0}",
            r"\getrefbykeydefault*{\keynme}{page}",
            r"\keynme",
        ),
        (
            r"\IfRefUndefinedExpandable{\keynme}{Missing}{Found}",
            r"\IfRefUndefinedExpandable{\keynme}",
            r"\keynme",
        ),
        (
            r"\IfRefUndefinedBabel{\keynme}{Missing}{Found}",
            r"\IfRefUndefinedBabel{\keynme}",
            r"r@\keynme",
        ),
        (
            r"\IfRefUndefinedBabel{\keynme}{Missing}{Found}",
            r"\IfRefUndefinedBabel{\keynme}{Missing}{Found}",
            r"\keynme",
        ),
        (
            r"\IfRefUndefinedBabel[note]{\keynme}{Missing}{Found}",
            r"\IfRefUndefinedBabel[note]{\keynme}",
            r"\keynme",
        ),
        (
            r"\IfRefUndefinedExpandable{prefix\keynme}{Missing}{Found}",
            r"\IfRefUndefinedExpandable{prefix\keynme}",
            r"r@prefix\keynme",
        ),
        (
            r"\setcounterref{\counter}{\keynme}",
            r"\setcounterref{\counter}{\keynme}",
            r"r@\keynme",
        ),
        (
            r"\addtocounterref{ counter}{\keynme}",
            r"\addtocounterref{ counter}{\keynme}",
            r"r@\keynme",
        ),
        (
            r"\setcounterpageref{counter}{\keynme\other}",
            r"\setcounterpageref{counter}{\keynme\other}",
            r"r@\keynme",
        ),
        (
            r"\addtocounterpageref{counter}{\keynme}",
            r"\addtocounterpageref{counter}{\keynme}",
            r"sub@\keynme",
        ),
        (
            r"\setcounterref*{counter}{\keynme}",
            r"\setcounterref*{counter}{\keynme}",
            r"r@\keynme",
        ),
        (
            r"\def\wrapper{\setcounterref{counter}{\keynme}}\wrapper",
            r"\def\wrapper{\setcounterref{counter}{\keynme}}\wrapper",
            r"r@\keynme",
        ),
    ] {
        for numbered in [false, true] {
            let diagnostic = parse(call, &format!("<argument> {frame}\nl.1 {echo}"), numbered);
            assert!(
                diagnostic.code.is_none() && diagnostic.data.is_none(),
                "{call}/{echo}/{frame}: {diagnostic:?}"
            );
        }
    }
    let source = r"\IfRefUndefinedExpandable{\keynme}{}{}\IfRefUndefinedExpandable{\keynme}{}{}";
    let diagnostic = parse(source, "<argument> r@\\keynme\nl.1 ...{\\keynme}", true);
    assert!(diagnostic.code.is_none() && diagnostic.data.is_none());
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_refcount_key_macro_repairs_restore_successful_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for (packages, call) in [
            (
                "\\usepackage{hyperref}\n",
                "\\getrefbykeydefault{\\keynme}{}{0}",
            ),
            (
                "\\usepackage{hyperref}\n\\newcounter{mycounter}\n",
                "\\getrefbykeydefault{\\keynme}{page}{0}",
            ),
            (
                "\\usepackage{hyperref}\n\\newcounter{mycounter}\n",
                "\\getrefbykeydefault{\\keynme}{anchor}{Missing}",
            ),
            (
                "\\usepackage{hyperref}\n\\newcounter{mycounter}\n",
                "\\IfRefUndefinedExpandable{\\keynme}{Missing}{Found}",
            ),
            (
                "\\usepackage{hyperref}\n\\newcounter{mycounter}\n",
                "\\IfRefUndefinedBabel{\\keynme}{Missing}{Found}",
            ),
            (
                "\\usepackage{hyperref}\n\\newcounter{mycounter}\n",
                "\\setcounterref{mycounter}{\\keynme}",
            ),
            (
                "\\usepackage{hyperref}\n\\newcounter{mycounter}\n",
                "\\addtocounterref{mycounter}{\\keynme}",
            ),
            (
                "\\usepackage{hyperref}\n\\newcounter{mycounter}\n",
                "\\setcounterpageref{mycounter}{\\keynme}",
            ),
            (
                "\\usepackage{hyperref}\n\\newcounter{mycounter}\n",
                "\\addtocounterpageref{mycounter}{\\keynme}",
            ),
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
                client: texquill::project::state::ClientCapabilities {
                    versioned_edits: true,
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.open(uri.clone(), source, 7);
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), before.diagnostics.clone());
            std::fs::write(&main, repaired(&workspace, &uri, diagnostic)).unwrap();
            let after = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(main
                .parent()
                .unwrap()
                .join("build/texquill/main.pdf")
                .is_file());
        }
    }
}
