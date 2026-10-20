use latex_lsp::{build, build_log, config::BuildConfig, editing, text::Text, workspace::Workspace};
use std::sync::atomic::AtomicBool;
use tower_lsp::lsp_types::*;

fn parse(source: &str, context: &str, numbered: bool) -> Diagnostic {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    let header = if numbered {
        "./main.tex:1: LaTeX Error: No counter 'missing' defined."
    } else {
        "! LaTeX Error: No counter 'missing' defined."
    };
    let diagnostics = build_log::parse(&main, &format!("(./main.tex\n{header}\n{context}\n)\n"));
    diagnostics[&Url::from_file_path(main).unwrap()]
        .iter()
        .find(|item| item.message.starts_with("LaTeX Error: No counter "))
        .unwrap()
        .clone()
}

fn repaired(
    workspace: &Workspace,
    uri: &Url,
    diagnostic: &Diagnostic,
    correct: &str,
) -> Option<String> {
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
            if action.title == format!("Replace with counter `{correct}`") =>
        {
            action.edit
        }
        _ => None,
    })?;
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
    Some(
        workspace.documents[uri]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap()
            .source,
    )
}

#[test]
fn shortened_counter_contexts_require_uniqueness_across_unknown_and_opaque_calls() {
    for (source, echo) in [
        (
            r"\iffalse\stepcounter{missing}\fi\unknownstepcounter{missing}",
            r"...stepcounter{missing}",
        ),
        (
            r"\iffalse\setcounter{missing}{1}\fi\unknownsetcounter{missing}{1}",
            r"...{missing}{1}",
        ),
        (
            r"\iffalse\addtocounterref{missing}{label}\fi\unknownaddtocounterref{missing}{label}",
            r"...{missing}{label}",
        ),
        (
            r"\verb|stepcounter{missing}|\stepcounter{missing}",
            r"...stepcounter{missing}",
        ),
        ("\\setcounter{missing}\n{1} \\unknown{1}", r"...{1}"),
    ] {
        for numbered in [false, true] {
            let diagnostic = if source.contains('\n') {
                // Keep the reported line aligned with the consumed value.
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                std::fs::write(&main, source).unwrap();
                let header = if numbered { "./main.tex:2:" } else { "!" };
                let log=format!("(./main.tex\n{header} LaTeX Error: No counter 'missing' defined.\nl.2 {echo}\n)\n");
                build_log::parse(&main, &log)[&Url::from_file_path(main).unwrap()][0].clone()
            } else {
                parse(source, &format!("l.1 {echo}"), numbered)
            };
            assert!(
                diagnostic.code.is_none() && diagnostic.data.is_none(),
                "{source}: {diagnostic:?}"
            );
        }
    }
}

#[test]
fn unique_shortened_counter_echoes_and_full_echoes_keep_precise_ranges() {
    for (source, echo) in [
        (
            r"\iffalse\unknownstepcounter{missing}\fi\stepcounter{missing}",
            r"...\stepcounter{missing}",
        ),
        (
            r"\iffalse\stepcounter{missing}\fi\stepcounter{missing}",
            r"\iffalse\stepcounter{missing}\fi\stepcounter{missing}",
        ),
        (
            "😀 prefix \\stepcounter{missing}",
            "...😀 prefix \\stepcounter{missing}",
        ),
        (r"Long prefix \setcounter{missing}{1}", r"...{missing}{1}"),
    ] {
        for numbered in [false, true] {
            let diagnostic = parse(source, &format!("l.1 {echo}"), numbered);
            let start = source.rfind("missing").unwrap();
            assert_eq!(
                diagnostic.range,
                Text::new(source.into()).range(start, start + 7),
                "{source}: {diagnostic:?}"
            );
            assert_eq!(diagnostic.data.as_ref().unwrap()["counter"], "missing");
        }
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_shortened_counter_contexts_reject_wrong_repairs_and_keep_unique_repairs() {
    for engine in ["pdflatex", "xelatex"] {
        for size in [30, 34] {
            for ambiguous in [false, true] {
                let name = format!("key{}me", "x".repeat(size - 5));
                let correct = format!("{}ame", &name[..name.len() - 2]);
                let call = if ambiguous {
                    format!("\\iffalse\\stepcounter{{{name}}}\\fi\\verylongcustomwrapperstepcounter{{{name}}}")
                } else {
                    format!("\\iffalse\\unknownwrapperOther{{{name}}}\\fi\\stepcounter{{{name}}}")
                };
                let source=format!("\\documentclass{{article}}\n\\newcommand{{\\verylongcustomwrapperstepcounter}}[1]{{\\stepcounter{{#1}}}}\n\\newcounter{{{correct}}}\n\\begin{{document}}\n\\section{{MarkerUnique}}\n{call}\n\\end{{document}}\n");
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let uri = Url::from_file_path(&main).unwrap();
                std::fs::write(&main, &source).unwrap();
                let config = BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                };
                let before = build::run(&main, &config, &AtomicBool::new(false));
                assert_eq!(
                    before.status, "error",
                    "{engine}/{size}/{ambiguous}: {}",
                    before.output
                );
                let diagnostic = before.diagnostics[&uri]
                    .iter()
                    .find(|d| d.message.starts_with("LaTeX Error: No counter "))
                    .unwrap();
                let start = source.rfind(&name).unwrap();
                let mut expected = source.clone();
                expected.replace_range(start..start + name.len(), &correct);
                let mut workspace = Workspace {
                    versioned_edits: true,
                    code_action_literals: true,
                    ..Default::default()
                };
                workspace.open(uri.clone(), source.clone(), 7);
                workspace
                    .build_diagnostics
                    .insert(uri.clone(), before.diagnostics.clone());
                let repair = repaired(&workspace, &uri, diagnostic, &correct);
                if ambiguous {
                    assert!(
                        diagnostic.code.is_none() && diagnostic.data.is_none(),
                        "{engine}/{size}: {diagnostic:?}"
                    );
                    assert!(repair.is_none(), "{engine}/{size}: {repair:?}");
                } else {
                    assert_eq!(
                        diagnostic.range,
                        Text::new(source).range(start, start + name.len()),
                        "{engine}/{size}: {diagnostic:?}"
                    );
                    assert_eq!(repair.as_ref(), Some(&expected), "{engine}/{size}");
                }
                std::fs::write(&main, expected).unwrap();
                let after = build::run(&main, &config, &AtomicBool::new(false));
                assert_eq!(
                    after.status, "success",
                    "{engine}/{size}/{ambiguous}: {}",
                    after.output
                );
                assert!(directory.path().join("build/latex-lsp/main.pdf").is_file());
            }
        }
    }
}
