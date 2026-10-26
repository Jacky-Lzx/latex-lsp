use latex_lsp::{
    compilation::{build, build_log},
    config::BuildConfig,
    editing,
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;
use std::sync::atomic::AtomicBool;

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
            if action.title == format!("Replace with command `\\{correct}`") =>
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
fn shortened_frames_reject_duplicate_source_suffixes_in_unknown_callers() {
    for (source, frame, suffix) in [
        (
            r"\iffalse\citefullauthor{\keynme}\fi\unknowncitefullauthor{\keynme}",
            r"\keynme",
            r"citefullauthor{\keynme}",
        ),
        (
            r"\iffalse\csuse{\keynme}\fi\unknowncsuse{\keynme}",
            r"\keynme",
            r"csuse{\keynme}",
        ),
        (
            r"\iffalse\UseVerb{\keynme}\fi\unknownUseVerb{\keynme}",
            r"FV@SV@\keynme",
            r"UseVerb{\keynme}",
        ),
        (
            r"\iffalse\vpageref{\keynme}\fi\unknownvpageref{\keynme}",
            r"r@\keynme",
            r"vpageref{\keynme}",
        ),
        (
            r"\iffalse\subref{\keynme}\fi\unknownsubref{\keynme}",
            r"sub@\keynme",
            r"subref{\keynme}",
        ),
        (
            r"\iffalse\hyperref[{\keynme}]{Text}\fi\unknownhyperref[{\keynme}]{Text}",
            r"\keynme",
            r"hyperref[{\keynme}]",
        ),
    ] {
        for numbered in [false, true] {
            let diagnostic = parse(
                source,
                &format!("<argument> {frame}\nl.1 ...{suffix}"),
                numbered,
            );
            assert!(
                diagnostic.code.is_none() && diagnostic.data.is_none(),
                "{source}: {diagnostic:?}"
            );
        }
    }
}

#[test]
fn unique_shortened_suffixes_and_full_echoes_still_disambiguate_repeated_words() {
    for (source, echo) in [
        (
            r"\iffalse\unknowncsuse{\keynme}\fi\csuse{\keynme}",
            r"...\csuse{\keynme}",
        ),
        (
            "😀 prefix \\csdef{\\keynme}{Body}",
            "...😀 prefix \\csdef{\\keynme}",
        ),
        (
            r"\iffalse\csuse{\keynme}\fi\csuse{\keynme}",
            r"\iffalse\csuse{\keynme}\fi\csuse{\keynme}",
        ),
        (
            r"Long prefix \letcs\keynme{\keynme}",
            r"...\letcs\keynme{\keynme}",
        ),
    ] {
        for numbered in [false, true] {
            let diagnostic = parse(
                source,
                &format!("<argument> \\keynme\nl.1 {echo}"),
                numbered,
            );
            let start = source.rfind(r"\keynme").unwrap();
            assert_eq!(
                diagnostic.range,
                Text::new(source.into()).range(start, start + 7),
                "{source}: {diagnostic:?}"
            );
            assert_eq!(diagnostic.data.as_ref().unwrap()["command"], "keynme");
        }
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_shortened_macro_contexts_reject_wrong_repairs_and_preserve_unique_repairs() {
    for engine in ["pdflatex", "xelatex"] {
        for size in [26, 30] {
            for ambiguous in [false, true] {
                let name = format!("key{}me", "x".repeat(size - 5));
                let correct = format!("{}ame", &name[..name.len() - 2]);
                let call = if ambiguous {
                    format!("\\iffalse\\citefullauthor{{\\{name}}}\\fi\\verylongcustomwrappercitefullauthor{{\\{name}}}")
                } else {
                    format!("\\iffalse\\unknownwrapperOther{{\\{name}}}\\fi\\citefullauthor{{\\{name}}}")
                };
                let source=format!("\\documentclass{{article}}\n\\usepackage[numbers]{{natbib}}\n\\newcommand{{\\verylongcustomwrappercitefullauthor}}[1]{{\\csname#1\\endcsname}}\n\\newcommand{{\\{correct}}}{{payload}}\n\\begin{{document}}\n\\section{{MarkerUnique}}\n\\begin{{thebibliography}}{{1}}\\bibitem{{payload}}Entry\\end{{thebibliography}}\n{call}\n\\end{{document}}\n");
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
                    .find(|d| d.message == "Undefined control sequence.")
                    .unwrap();
                let start = source.rfind(&format!("\\{name}")).unwrap();
                let mut expected = source.clone();
                expected.replace_range(start..start + name.len() + 1, &format!("\\{correct}"));
                let mut workspace = Workspace {
                    client: latex_lsp::project::state::ClientCapabilities {
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
                        Text::new(source).range(start, start + name.len() + 1),
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
