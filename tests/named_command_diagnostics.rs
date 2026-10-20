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

#[test]
fn single_name_interfaces_pinpoint_the_reported_macro_and_respect_consumed_operands() {
    for (call, consumed) in [
        (r"\csuse{\keynme}", r"\csuse{\keynme}"),
        (r"\csmeaning{\keynme}", r"\csmeaning{\keynme}"),
        (r"\csdef{\keynme}{Body}", r"\csdef{\keynme}"),
        (r"\csgdef{\keynme}{Body}", r"\csgdef{\keynme}"),
        (r"\csedef{\keynme}{Body}", r"\csedef{\keynme}"),
        (r"\csxdef{\keynme}{Body}", r"\csxdef{\keynme}"),
        (r"\cslet{\keynme}\payload", r"\cslet{\keynme}\payload"),
        (r"\cslet{\keynme}{\payload}", r"\cslet{\keynme}{\payload}"),
        (r"\csappto{\keynme}{Body}", r"\csappto{\keynme}"),
        (r"\cspreto{\keynme}{Body}", r"\cspreto{\keynme}"),
        (r"\ifcsdef{\keynme}{Yes}{No}", r"\ifcsdef{\keynme}"),
        (r"\csundef{\keynme}", r"\csundef{\keynme}"),
    ]
    .into_iter()
    .flat_map(|(call, consumed)| {
        [
            (call.to_owned(), consumed.to_owned()),
            (
                call.replace(r"{\keynme}", "\\keynme "),
                consumed.replace(r"{\keynme}", "\\keynme "),
            ),
        ]
    }) {
        let source = format!("\\typeout{{😀}} {call} Remains\r\n");
        let text = Text::new(source.clone());
        let start = source.find(r"\keynme").unwrap();
        for numbered in [false, true] {
            let diagnostic = parse(
                &source,
                &format!("<argument> \\keynme \n \nl.1 \\typeout{{😀}} {consumed}"),
                numbered,
            );
            assert_eq!(
                diagnostic.range,
                text.range(start, start + 7),
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
fn dynamic_lists_unknown_interfaces_wrappers_and_opaque_text_keep_line_diagnostics() {
    for (source, echo) in [
        (r"\csuse{prefix\keynme}", r"\csuse{prefix\keynme}"),
        (r"\csuse{\keynme }", r"\csuse{\keynme }"),
        (r"\csuse{\keynme\suffix}", r"\csuse{\keynme\suffix}"),
        (r"\csuse\keynmee", r"\csuse\keynmee"),
        (r"\cslet\keynme\payload", r"\cslet\keynme"),
        (r"\unknown\keynme", r"\unknown\keynme"),
        (r"% \csuse\keynme", r"% \csuse\keynme"),
        (
            r"\SaveVerb{saved}|\csuse\keynme|",
            r"\SaveVerb{saved}|\csuse\keynme|",
        ),
        (
            r"\UseVerb{\detokenize{\csuse\keynme}}",
            r"\UseVerb{\detokenize{\csuse\keynme}}",
        ),
        (r"\csletcs{\keynme}{source}", r"\csletcs{\keynme}{source}"),
        (
            r"\ifcsequal{\keynme}{\other}{Yes}{No}",
            r"\ifcsequal{\keynme}{\other}",
        ),
        (r"\unknown{\keynme}", r"\unknown{\keynme}"),
        (
            r"\def\wrapper{\csuse{\keynme}}\wrapper",
            r"\def\wrapper{\csuse{\keynme}}\wrapper",
        ),
        (r"\csuse{\nested}", r"\csuse{\nested}"),
        (r"\verb|\csuse{\keynme}|", r"\verb|\csuse{\keynme}|"),
        (r"% \csuse{\keynme}", r"% \csuse{\keynme}"),
        (
            r"\UseVerb{\detokenize{\csuse{\keynme}}}",
            r"\UseVerb{\detokenize{\csuse{\keynme}}}",
        ),
        (
            r"\SaveVerb{saved}|\csuse{\keynme}|",
            r"\SaveVerb{saved}|\csuse{\keynme}|",
        ),
        (r"\cslet{\keynme}\payload", r"\cslet{\keynme}"),
        (r"\cslet{\keynme}{\payload}", r"\cslet{\keynme}{\other}"),
        (r"\csuse{\keynme}", r"Changed \csuse{\keynme}"),
        (r"\csuse{\keynme} \csuse{\keynme}", r"...{\keynme}"),
        (r"\csuse{\keynme}", r"...keynme}"),
        (r"\csuse\keynme \csuse\keynme", r"...\keynme"),
        (r"\csuse\keynme", r"...keynme"),
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
    let diagnostic = parse(
        r"Long prefix \csuse{\keynme}",
        "<argument> \\keynme\nl.1 ...{\\keynme}",
        true,
    );
    assert!(diagnostic.code.is_some());
    let diagnostic = parse(
        r"Long prefix \csuse\keynme",
        "<argument> \\keynme\nl.1 ...\\keynme",
        true,
    );
    assert!(diagnostic.code.is_some());
}

#[test]
fn multiple_name_interfaces_require_one_dynamic_operand_and_static_peer_names() {
    for (call, consumed) in [
        (
            r"\csletcs{alias}{\keynme}\alias",
            r"\csletcs{alias}{\keynme}",
        ),
        (
            r"\ifcsequal{\keynme}{payload}{Yes}{No}",
            r"\ifcsequal{\keynme}{payload}",
        ),
        (
            r"\ifcsequal{payload}{\keynme}{Yes}{No}",
            r"\ifcsequal{payload}{\keynme}",
        ),
        (
            r"\ifcsstrequal{\keynme}{payload}{Yes}{No}",
            r"\ifcsstrequal{\keynme}{payload}",
        ),
        (
            r"\ifcsstrequal{payload}{\keynme}{Yes}{No}",
            r"\ifcsstrequal{payload}{\keynme}",
        ),
        (r"\letcs\alias{\keynme}\alias", r"\letcs\alias{\keynme}"),
        (r"\letcs{\alias}{\keynme}\alias", r"\letcs{\alias}{\keynme}"),
        (r"\letcs\keynme{\keynme}", r"\letcs\keynme{\keynme}"),
    ]
    .into_iter()
    .flat_map(|(call, consumed)| {
        [
            (call.to_owned(), consumed.to_owned()),
            (
                call.replace(r"{\keynme}", "\\keynme "),
                consumed.replace(r"{\keynme}", "\\keynme "),
            ),
        ]
    }) {
        let source = format!("\\typeout{{😀}} {call} Rest\r\n");
        for numbered in [false, true] {
            let diagnostic = parse(
                &source,
                &format!("<argument> \\keynme\nl.1 \\typeout{{😀}} {consumed}"),
                numbered,
            );
            let start = source.rfind(r"\keynme").unwrap();
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + 7),
                "{call}: {diagnostic:?}"
            );
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("undefined-control-sequence".into()))
            );
        }
    }
    for call in [
        r"\ifcsequal{\keynme}{\keynme}",
        r"\ifcsequal{\keynme}{\other}",
        r"\ifcsequal{\keynme}{prefix\other}",
        r"\ifcsequal{\keynme}",
        r"\ifcsequal*{\keynme}{payload}",
        r"\csletcs{\keynme}{payload}",
        r"\csletcs{\other}{\keynme}",
        r"\letcs{\csname alias\endcsname}{\keynme}",
    ] {
        let diagnostic = parse(call, &format!("<argument> \\keynme\nl.1 {call}"), true);
        assert!(
            diagnostic.code.is_none() && diagnostic.data.is_none(),
            "{call}: {diagnostic:?}"
        );
    }
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
fn named_argument_errors_reuse_versioned_macro_spelling_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = r"\newcommand{\keyname}{payload}\csuse{\keynme}";
    std::fs::write(&main, source).unwrap();
    let diagnostics = build_log::parse(&main, &format!("(./main.tex\n./main.tex:1: Undefined control sequence.\n<argument> \\keynme\nl.1 {source}\n)\n"));
    let diagnostic = diagnostics[&uri][0].clone();
    let mut workspace = Workspace {
        versioned_edits: true,
        code_action_literals: true,
        ..Default::default()
    };
    workspace.open(uri.clone(), source.into(), 7);
    workspace.build_diagnostics.insert(uri.clone(), diagnostics);
    assert_eq!(
        repaired(&workspace, &uri, &diagnostic),
        source.replace(r"\keynme", r"\keyname")
    );
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_named_argument_macro_fixes_restore_successful_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for call in [
            r"\csuse{\keynme}",
            r"\csdef{\keynme}{MarkerUnique}\payload",
            r"\csgdef{\keynme}{MarkerUnique}\payload",
            r"\csedef{\keynme}{MarkerUnique}\payload",
            r"\csxdef{\keynme}{MarkerUnique}\payload",
            r"\cslet{\keynme}\payload\payload",
            r"\cslet{\keynme}{\payload}\payload",
            r"\csappto{\keynme}{MarkerUnique}\payload",
            r"\cspreto{\keynme}{MarkerUnique}\payload",
            r"\ifcsdef{\keynme}{MarkerUnique}{Other}",
            r"\csmeaning{\keynme}",
            r"\csundef{\keynme}MarkerUnique",
        ]
        .into_iter()
        .flat_map(|call| [call.to_owned(), call.replace(r"{\keynme}", "\\keynme ")])
        {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let uri = Url::from_file_path(&main).unwrap();
            let source = format!("\\documentclass{{article}}\r\n\\usepackage{{etoolbox}}\r\n\\newcommand{{\\keyname}}{{payload}}\\newcommand{{\\payload}}{{MarkerUnique}}\r\n\\begin{{document}}\r\n{call}\r\n\\end{{document}}\r\n");
            std::fs::write(&main, &source).unwrap();
            let config = BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let before = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(before.status, "error", "{engine}/{call}: {}", before.output);
            let diagnostic = before.diagnostics[&uri]
                .iter()
                .find(|item| item.message == "Undefined control sequence.")
                .unwrap();
            let start = source.find(r"\keynme").unwrap();
            assert_eq!(
                diagnostic.range,
                Text::new(source.clone()).range(start, start + 7),
                "{engine}/{call}: {}",
                before.output
            );
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

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_multiple_name_operand_repairs_restore_successful_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for call in [
            r"\csletcs{alias}{\keynme}\alias",
            r"\ifcsequal{\keynme}{payload}{MarkerUnique}{Other}",
            r"\ifcsequal{payload}{\keynme}{MarkerUnique}{Other}",
            r"\ifcsstrequal{\keynme}{payload}{MarkerUnique}{Other}",
            r"\ifcsstrequal{payload}{\keynme}{MarkerUnique}{Other}",
            r"\letcs\alias{\keynme}\alias",
        ]
        .into_iter()
        .flat_map(|call| [call.to_owned(), call.replace(r"{\keynme}", "\\keynme ")])
        {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let uri = Url::from_file_path(&main).unwrap();
            let source = format!("\\documentclass{{article}}\r\n\\usepackage{{etoolbox}}\r\n\\newcommand{{\\keyname}}{{payload}}\\newcommand{{\\payload}}{{MarkerUnique}}\r\n\\begin{{document}}\r\n{call}\r\n\\end{{document}}\r\n");
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
