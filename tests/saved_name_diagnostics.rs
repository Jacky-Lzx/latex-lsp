use latex_lsp::{build, build_log, config::BuildConfig, editing, text::Text, workspace::Workspace};
use lsp_types::*;
use std::sync::atomic::AtomicBool;

fn undefined(diagnostics: &[Diagnostic]) -> &Diagnostic {
    diagnostics
        .iter()
        .find(|item| item.message == "Undefined control sequence.")
        .unwrap()
}

fn parse(source: &str, context: &str, numbered: bool) -> Diagnostic {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    let header = if numbered {
        "./main.tex:1: Undefined control sequence."
    } else {
        "! Undefined control sequence."
    };
    let log = format!("(./main.tex\n{header}\n{context}\n)\n");
    let diagnostics = build_log::parse(&main, &log);
    undefined(&diagnostics[&Url::from_file_path(main).unwrap()]).clone()
}

#[test]
fn known_saved_name_frames_pinpoint_one_direct_macro_with_utf16_source_ranges() {
    for (call, frame, consumed) in [
        (r"\UseVerb{\keynme}", "FV@SV@", r"\UseVerb{\keynme}"),
        (r"\UseVerb\keynme", "FV@SV@", r"\UseVerb\keynme"),
        (r"\UseVerbatim{\keynme}", "FV@SV@", r"\UseVerbatim{\keynme}"),
        (
            r"\LUseVerbatim{\keynme}",
            "FV@SV@",
            r"\LUseVerbatim{\keynme}",
        ),
        (
            r"\BUseVerbatim{\keynme}",
            "FV@SV@",
            r"\BUseVerbatim{\keynme}",
        ),
        (r"\UseMVerb{\keynme}", "", r"\UseMVerb{\keynme}"),
        (r"\pUseMVerb{\keynme}", "", r"\pUseMVerb{\keynme}"),
        (
            r"\SaveVerb{\keynme}|Body|",
            "FV@SV@",
            r"\SaveVerb{\keynme}|",
        ),
        (
            r"\SaveGVerb{\keynme}|Body|",
            "FV@SVG@",
            r"\SaveGVerb{\keynme}|",
        ),
        (
            r"\SaveVerb{\keynme}{界}Body界",
            "FV@SV@",
            r"\SaveVerb{\keynme}{界}",
        ),
        (
            r"\UseVerb[formatcom=\relax]{\keynme}",
            "FV@SV@",
            r"\UseVerb[formatcom=\relax]{\keynme}",
        ),
    ] {
        let source = format!("\\typeout{{😀}} {call} Rest\r\n");
        let text = Text::new(source.clone());
        let start = source.find(r"\keynme").unwrap();
        for numbered in [false, true] {
            let context = format!("<argument> {frame}\\keynme \n                         \nl.1 \\typeout{{😀}} {consumed}");
            let diagnostic = parse(&source, &context, numbered);
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
fn uncertain_frames_and_opaque_names_never_produce_a_command_spelling_target() {
    for (source, frame, echo) in [
        (
            r"\UseVerb{\keynme}",
            "FV@SVG@\\keynme",
            r"\UseVerb{\keynme}",
        ),
        (r"\UseVerb{\keynme}", "\\keynme", r"\UseVerb{\keynme}"),
        (
            r"\UseMVerb{\keynme}",
            "FV@SV@\\keynme",
            r"\UseMVerb{\keynme}",
        ),
        (r"\UseVerb{\keynme}", "FV@SV@\\other", r"\UseVerb{\keynme}"),
        (
            r"\UseVerb{\keynme}",
            "FV@SV@\\keynme",
            r"Changed \UseVerb{\keynme}",
        ),
        (r"\UseVerb{\keynme}", "FV@SV@\\keynme", r"...keynme}"),
        (
            r"\def\wrapper{\UseVerb{\keynme}}\wrapper",
            "FV@SV@\\keynme",
            r"\def\wrapper{\UseVerb{\keynme}}\wrapper",
        ),
        (r"\UseVerb{\nested}", "FV@SV@\\keynme", r"\UseVerb{\nested}"),
        (
            r"\UseVerb{\keynme suffix}",
            "FV@SV@\\keynme",
            r"\UseVerb{\keynme suffix}",
        ),
        (
            r"\UseVerb{\detokenize{\keynme}}",
            "FV@SV@\\keynme",
            r"\UseVerb{\detokenize{\keynme}}",
        ),
        (
            r"\UseVerb{\string\keynme}",
            "FV@SV@\\keynme",
            r"\UseVerb{\string\keynme}",
        ),
        (
            r"\SaveVerb{saved}|\keynme|",
            "FV@SV@\\keynme",
            r"\SaveVerb{saved}|\keynme|",
        ),
        (
            r"\UseVerb[commandchars=\\\{\}]{\keynme}",
            "FV@SV@\\keynme",
            r"\UseVerb[commandchars=\\\{\}]{\keynme}",
        ),
        (
            r"\SaveVerb{\keynme}|Body|",
            "FV@SV@\\keynme",
            r"\SaveVerb{\keynme}",
        ),
        (
            r"\UseVerb{\keynme}",
            "FV@SV@\\keynme\n\\wrapper ->Something",
            r"\UseVerb{\keynme}",
        ),
        (
            r"\UseVerb{\keynme}",
            "FV@SV@\\keynme\nPackage audit Warning: Adjacent.",
            r"\UseVerb{\keynme}",
        ),
    ] {
        for numbered in [false, true] {
            let context = format!("<argument> {frame}\nl.1 {echo}");
            let diagnostic = parse(source, &context, numbered);
            assert!(
                diagnostic.code.is_none() && diagnostic.data.is_none(),
                "{source}/{frame}: {diagnostic:?}"
            );
        }
    }
}

#[test]
fn shortened_saved_name_context_requires_a_complete_unique_argument() {
    for (source, suffix, precise) in [
        (r"Long prefix \UseVerb{\keynme}", r"{\keynme}", true),
        (r"\UseVerb{\keynme} \UseVerb{\keynme}", r"{\keynme}", false),
        (r"Long prefix \UseVerb{\keynme}", r"keynme}", false),
        (r"Long prefix \SaveVerb{\keynme}|Body|", r"{\keynme}|", true),
        (r"Long prefix \SaveVerb{\keynme}|Body|", r"{\keynme}", false),
    ] {
        for numbered in [false, true] {
            let context = format!("<argument> FV@SV@\\keynme\nl.1 ...{suffix}");
            let diagnostic = parse(source, &context, numbered);
            assert_eq!(
                diagnostic.code.is_some(),
                precise,
                "{source}: {diagnostic:?}"
            );
        }
    }
}

fn repair(workspace: &Workspace, uri: &Url, diagnostic: &Diagnostic) -> WorkspaceEdit {
    editing::code_actions(
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
    .unwrap_or_else(|| panic!("missing spelling repair: {diagnostic:?}"))
}

fn apply(workspace: &Workspace, uri: &Url, edit: WorkspaceEdit) -> String {
    let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
        panic!("missing versioned changes")
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].text_document.uri, *uri);
    assert_eq!(changes[0].text_document.version, Some(7));
    assert_eq!(changes[0].edits.len(), 1);
    let OneOf::Left(edit) = &changes[0].edits[0] else {
        panic!("unexpected annotated edit")
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
fn compiler_confirmed_saved_name_macro_errors_offer_versioned_spelling_repairs() {
    let source =
        "\\newcommand{\\keyname}{saved}\r\n\\SaveVerb{saved}|MarkerUnique|\\UseVerb{\\keynme}\r\n";
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    let log = "(./main.tex\n./main.tex:2: Undefined control sequence.\n<argument> FV@SV@\\keynme \n \nl.2 \\SaveVerb{saved}|MarkerUnique|\\UseVerb{\\keynme}\n)\n";
    let diagnostics = build_log::parse(&main, log);
    assert_eq!(
        undefined(&diagnostics[&uri]).code,
        Some(NumberOrString::String("undefined-control-sequence".into()))
    );
    let mut workspace = Workspace {
        versioned_edits: true,
        code_action_literals: true,
        ..Default::default()
    };
    workspace.open(uri.clone(), source.into(), 7);
    workspace.build_diagnostics.insert(uri.clone(), diagnostics);
    let diagnostic = workspace.build_diagnostics[&uri][&uri]
        .iter()
        .find(|item| item.message == "Undefined control sequence.")
        .unwrap();
    let result = apply(&workspace, &uri, repair(&workspace, &uri, diagnostic));
    assert_eq!(result, source.replace(r"\keynme", r"\keyname"));
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_saved_name_macro_spelling_repairs_restore_successful_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for call in [
            r"\SaveVerb{saved}|MarkerUnique|\UseVerb{\keynme}",
            r"\SaveVerb{\keynme}|MarkerUnique|\UseVerb{saved}",
            r"\SaveGVerb{saved}|MarkerUnique|\UseMVerb{\keynme}",
            r"\SaveGVerb{\keynme}|MarkerUnique|\UseMVerb{saved}",
            "\\begin{SaveVerbatim}{saved}\r\nMarkerUnique\r\n\\end{SaveVerbatim}\r\n\\UseVerbatim{\\keynme}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let source = format!("\\documentclass{{article}}\r\n\\usepackage{{fancyvrb}}\r\n\\newcommand{{\\keyname}}{{saved}}\r\n\\begin{{document}}\r\n{call}\r\n\\end{{document}}\r\n");
            std::fs::write(&main, &source).unwrap();
            let uri = Url::from_file_path(&main).unwrap();
            let config = BuildConfig { executable: engine.into(), ..Default::default() };
            let before = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(before.status, "error", "{engine}/{call}: {}", before.output);
            let diagnostic = undefined(&before.diagnostics[&uri]);
            let start = source.find(r"\keynme").unwrap();
            assert_eq!(diagnostic.range, Text::new(source.clone()).range(start, start + 7), "{engine}/{call}: {}", before.output);
            let mut workspace = Workspace { versioned_edits: true, code_action_literals: true, ..Default::default() };
            workspace.open(uri.clone(), source.clone(), 7);
            workspace.build_diagnostics.insert(uri.clone(), before.diagnostics.clone());
            let result = apply(&workspace, &uri, repair(&workspace, &uri, diagnostic));
            std::fs::write(&main, result).unwrap();
            let after = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(main.parent().unwrap().join("build/latex-lsp/main.pdf").is_file());
        }
    }
}
