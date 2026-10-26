use lsp_types::*;
use std::collections::BTreeMap;
use texquill::{
    compilation::{build, build_log},
    config::BuildConfig,
    editing,
    project::workspace::{Document, Workspace},
    text::Text,
};

const MESSAGE: &str = "Package xcolor Error: Undefined color model `rbg'.";
const HELP: &str = "See the xcolor package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\n";
fn diagnostic(source: &str, context: &str, numbered: bool) -> Diagnostic {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    let header = if numbered { "./main.tex:1:" } else { "!" };
    let log = format!("(./main.tex\n{header} {MESSAGE}\n{HELP}{context}\n)\n");
    let result = build_log::parse(&main, &log);
    result[&Url::from_file_path(main).unwrap()]
        .iter()
        .find(|d| d.message == MESSAGE)
        .unwrap()
        .clone()
}
fn setup(source: &str, definitions: &str) -> (Workspace, Url) {
    let uri = Url::parse("file:///model-fixes/main.tex").unwrap();
    let palette = Url::parse("file:///model-fixes/palette.tex").unwrap();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            code_action_literals: true,
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    workspace.documents.insert(
        palette.clone(),
        Document::new(&palette, definitions.into(), None),
    );
    workspace
        .project
        .recorded_inputs
        .insert(uri.clone(), [uri.clone(), palette].into());
    (workspace, uri)
}
fn store(workspace: &mut Workspace, uri: &Url, diagnostic: Diagnostic) {
    workspace.project.build_diagnostics.insert(
        uri.clone(),
        BTreeMap::from([(uri.clone(), vec![diagnostic])]),
    );
}
fn actions(workspace: &Workspace, uri: &Url, diagnostic: &Diagnostic) -> CodeActionResponse {
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
}
fn repair(workspace: &Workspace, uri: &Url, diagnostic: &Diagnostic, name: &str) -> Option<String> {
    let action = actions(workspace, uri, diagnostic)
        .into_iter()
        .find_map(|item| match item {
            CodeActionOrCommand::CodeAction(action)
                if action.title == format!("Replace with color model `{name}`") =>
            {
                Some(action)
            }
            _ => None,
        })?;
    let Some(DocumentChanges::Edits(changes)) = action.edit.unwrap().document_changes else {
        panic!()
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].text_document.uri, *uri);
    assert_eq!(changes[0].text_document.version, Some(7));
    assert_eq!(changes[0].edits.len(), 1);
    let OneOf::Left(edit) = &changes[0].edits[0] else {
        panic!()
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

fn calls() -> Vec<(&'static str, &'static str)> {
    vec![
        (r"\definecolor{tone}{rbg}{.2,.4,.6}", "MarkerUnique"),
        (r"\providecolor{tone}{rbg}{.2,.4,.6}", "MarkerUnique"),
        (r"\xdefinecolor{tone}{rbg}{.2,.4,.6}", "MarkerUnique"),
        (r"\color[rbg]{.2,.4,.6}", "MarkerUnique"),
        (r"\pagecolor[rbg]{.2,.4,.6}", "MarkerUnique"),
        (r"\textcolor[rbg]{.2,.4,.6}{MarkerUnique}", ""),
        (r"\colorbox[rbg]{.2,.4,.6}{MarkerUnique}", ""),
        (r"\fcolorbox[rbg]{.2,.4,.6}{.3,.5,.7}{MarkerUnique}", ""),
        (
            r"\fcolorbox[rgb]{.2,.4,.6}[rbg]{.3,.5,.7}{MarkerUnique}",
            "",
        ),
    ]
}

#[test]
fn model_diagnostics_distinguish_bare_callback_operands_from_completed_calls() {
    for (prefix, direct) in [
        (
            r"\NewDocumentCommand{\wrapped}{m O{rgb} m m}{\textcolor[rbg]{.2,.4,.6}{MarkerUnique}}\wrapped",
            false,
        ),
        (
            r"\NewDocumentCommand{\wrapped}{m m O{rgb} m m}{\textcolor[rbg]{.2,.4,.6}{MarkerUnique}}\wrapped{Unused}",
            false,
        ),
        (
            r"\NewDocumentCommand{\wrapped}{s t+ m O{rgb} m m}{\textcolor[rbg]{.2,.4,.6}{MarkerUnique}}\wrapped*+",
            false,
        ),
        (r"\unknown{Unused}", false),
        (r"\relax\noindent\bfseries", true),
        (r"\newcommand{\done}[1]{}\done{Unused}", true),
        (r"\newcommand{\done}{}\done", true),
        (r"\extractcolorspecs{red}\model\spec", true),
    ] {
        let source = format!("{prefix}\\textcolor[rbg]{{.2,.4,.6}}{{MarkerUnique}}");
        let error = diagnostic(&source, &format!("l.1 {source}"), false);
        let (mut workspace, uri) = setup(&source, "");
        store(&mut workspace, &uri, error.clone());
        if direct {
            let start = source.rfind("rbg").unwrap();
            assert_eq!(
                error.range,
                Text::new(source.clone()).range(start, start + 3),
                "{source}"
            );
            assert!(
                repair(&workspace, &uri, &error, "rgb").is_some(),
                "{source}"
            );
        } else {
            assert!(
                error.code.is_none() && error.data.is_none(),
                "{source}: {error:?}"
            );
            assert!(actions(&workspace, &uri, &error).is_empty(), "{source}");
        }
    }
}

#[test]
fn model_errors_select_only_the_misspelled_model_and_restore_literal_color_previews() {
    for (call, rest) in calls() {
        let source = format!("😀 {call}{rest} % rbg\r\n");
        let text = Text::new(source.clone());
        for numbered in [true, false] {
            for echo in [format!("😀 {call}"), format!("...{call}")] {
                let error = diagnostic(&source, &format!("l.1 {echo}"), numbered);
                let start = source.find("rbg").unwrap();
                assert_eq!(
                    error.range,
                    text.range(start, start + 3),
                    "{call}: {error:?}"
                );
                assert_eq!(
                    error.code,
                    Some(NumberOrString::String("undefined-color-model".into()))
                );
                assert_eq!(error.data.as_ref().unwrap()["model"], "rbg");
                let (mut workspace, uri) = setup(
                    &source,
                    r"\definecolor{rbg}{HTML}{123456}\definecolor{rpb}{HTML}{ABCDEF}",
                );
                store(&mut workspace, &uri, error.clone());
                let updated = repair(&workspace, &uri, &error, "rgb").unwrap();
                assert_eq!(updated, source.replacen("rbg", "rgb", 1));
                assert!(repair(&workspace, &uri, &error, "rpb").is_none());
                let doc = Document::new(&uri, updated, Some(8));
                if !call.starts_with("\\providecolor") && !call.starts_with("\\xdefinecolor") {
                    let colors = texquill::language::colors::document_colors(&doc);
                    assert!(
                        colors.iter().any(|c| c.color.red == 0.2
                            && c.color.green == 0.4
                            && c.color.blue == 0.6),
                        "{call}: {colors:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn dynamic_models_values_redefinitions_and_ambiguous_echoes_keep_line_ranges() {
    for (source, echo) in [
        (
            r"\textcolor[\model]{.2,.4,.6}{Text}",
            r"\textcolor[\model]{.2,.4,.6}{Text}",
        ),
        (
            r"\textcolor[rbg]{\value}{Text}",
            r"\textcolor[rbg]{\value}{Text}",
        ),
        (
            r"\fcolorbox[\model]{.2,.4,.6}[rbg]{.3,.5,.7}{Text}",
            r"\fcolorbox[\model]{.2,.4,.6}[rbg]{.3,.5,.7}{Text}",
        ),
        (
            r"\fcolorbox[rbg]{.2,.4,.6}[rbg]{.3,.5,.7}{Text}",
            r"\fcolorbox[rbg]{.2,.4,.6}[rbg]{.3,.5,.7}{Text}",
        ),
        (
            r"\iffalse\color[rbg]{.2,.4,.6}\fi\unknowncolor[rbg]{.2,.4,.6}",
            r"...color[rbg]{.2,.4,.6}",
        ),
        (
            r"\verb|color[rbg]{.2,.4,.6}|\unknowncolor[rbg]{.2,.4,.6}",
            r"...color[rbg]{.2,.4,.6}",
        ),
        (
            r"\color[rbg]{.2,.4,.6} % color[rbg]{.2,.4,.6}",
            r"...color[rbg]{.2,.4,.6}",
        ),
        (
            r"\newcommand{\paint}[1]{\color[#1]{.2,.4,.6}}\paint{rbg}",
            r"...\color[#1]{.2,.4,.6}}\paint{rbg}",
        ),
        (
            r"\renewcommand{\color}[1]{}\color[rbg]{.2,.4,.6}",
            r"\renewcommand{\color}[1]{}\color[rbg]{.2,.4,.6}",
        ),
        (
            r"\definecolor{\name}{rbg}{.2,.4,.6}",
            r"\definecolor{\name}{rbg}{.2,.4,.6}",
        ),
        (
            r"\definecolor{tone}[\prefix]{rbg}{.2,.4,.6}",
            r"\definecolor{tone}[\prefix]{rbg}{.2,.4,.6}",
        ),
        (
            r"\textcolor[ rbg ]{.2,.4,.6}{Text}",
            r"\textcolor[ rbg ]{.2,.4,.6}{Text}",
        ),
        (
            r"\textcolor[{rbg}]{.2,.4,.6}{Text}",
            r"\textcolor[{rbg}]{.2,.4,.6}{Text}",
        ),
        (
            r"\textcolor[[rbg]{.2,.4,.6}{Text}",
            r"\textcolor[[rbg]{.2,.4,.6}{Text}",
        ),
        (
            r"\detokenize{\color[rbg]{.2,.4,.6}}",
            r"\detokenize{\color[rbg]{.2,.4,.6}}",
        ),
    ] {
        let error = diagnostic(source, &format!("l.1 {echo}"), true);
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{source}: {error:?}"
        );
    }
    for echo in [
        "<argument> rbg\nl.1 \\color[rbg]{.2,.4,.6}",
        "l.2 \\color[rbg]{.2,.4,.6}",
        "l.1 \\color[rbg]",
    ] {
        let error = diagnostic(r"\color[rbg]{.2,.4,.6}", echo, true);
        assert!(error.code.is_none(), "{echo}: {error:?}");
    }
}

#[test]
fn model_suggestions_are_case_sensitive_and_metadata_must_match_the_current_operand() {
    for (wrong, correct, value) in [("HTLM", "HTML", "123456"), ("gary", "gray", ".5")] {
        let source = format!("\\textcolor[{wrong}]{{{value}}}{{Keep {wrong}}}");
        let (mut workspace, uri) = setup(&source, "");
        let error = Diagnostic {
            range: workspace.documents[&uri].text.range(11, 11 + wrong.len()),
            source: Some("latex-build".into()),
            message: format!("Package xcolor Error: Undefined color model `{wrong}'."),
            code: Some(NumberOrString::String("undefined-color-model".into())),
            data: Some(serde_json::json!({"model":wrong})),
            ..Default::default()
        };
        store(&mut workspace, &uri, error.clone());
        assert_eq!(
            repair(&workspace, &uri, &error, correct).unwrap(),
            source.replacen(wrong, correct, 1)
        );
        let mut guards = vec![];
        let mut e = error.clone();
        e.code = None;
        guards.push(e);
        let mut e = error.clone();
        e.data = Some(serde_json::json!({"model":"different"}));
        guards.push(e);
        let mut e = error.clone();
        e.range.start.character = 0;
        guards.push(e);
        let mut e = error.clone();
        e.message = "Package xcolor Error: Undefined color model `different'.".into();
        guards.push(e);
        let mut e = error.clone();
        e.data = Some(serde_json::json!({"model":wrong,"unresolvedLine":true}));
        guards.push(e);
        for e in guards {
            store(&mut workspace, &uri, e.clone());
            assert!(actions(&workspace, &uri, &e).is_empty(), "{e:?}");
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and xcolor"]
fn real_model_spelling_repairs_compile_and_preserve_color_specifications() {
    for engine in ["pdflatex", "xelatex"] {
        for (call, rest) in calls() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let main = root.join("main.tex");
            let source = format!("\\documentclass{{article}}\n\\usepackage{{xcolor}}\n\\begin{{document}}\n{call}{rest}\n\\end{{document}}");
            std::fs::write(&main, &source).unwrap();
            let config = BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let before = build::run(&main, &config, &Default::default());
            assert_eq!(before.status, "error", "{engine}/{call}: {}", before.output);
            let uri = Url::from_file_path(&main).unwrap();
            let error = before.diagnostics[&uri]
                .iter()
                .find(|e| e.message == MESSAGE)
                .unwrap_or_else(|| panic!("{engine}/{call}: {:?}", before.diagnostics))
                .clone();
            assert_eq!(
                error.code,
                Some(NumberOrString::String("undefined-color-model".into())),
                "{engine}/{call}: {error:?}"
            );
            let mut workspace = Workspace {
                client: texquill::project::state::ClientCapabilities {
                    versioned_edits: true,
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(uri.clone(), source.clone(), 7);
            store(&mut workspace, &uri, error.clone());
            let updated = repair(&workspace, &uri, &error, "rgb").unwrap();
            assert_eq!(updated, source.replacen("rbg", "rgb", 1));
            std::fs::write(&main, updated).unwrap();
            let after = build::run(&main, &config, &Default::default());
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(after.pdf.as_ref().unwrap().is_file());
        }
    }
}

#[test]
fn project_macro_arities_revalidate_cross_file_color_fixes_and_reject_changed_targets() {
    let source = "\\bridge\\textcolor[rbg]{.2,.4,.6}{MarkerUnique}";
    let (mut workspace, uri) = setup(
        source,
        "\\definecolor{accent}{HTML}{123456}\\newcommand{\\done}{}\\def\\bridge{\\done}",
    );
    let palette = Url::parse("file:///model-fixes/palette.tex").unwrap();
    let start = source.find("rbg").unwrap();
    let mut error = Diagnostic {
        range: Text::new(source.into()).range(start, start + "rbg".len()),
        source: Some("latex-build".into()),
        message: MESSAGE.into(),
        code: Some(NumberOrString::String("undefined-color-model".into())),
        data: Some(serde_json::json!({"model":"rbg"})),
        ..Default::default()
    };
    store(&mut workspace, &uri, error.clone());
    workspace
        .project
        .build_diagnostic_sources
        .insert(uri.clone(), [uri.clone()].into());
    assert!(actions(&workspace, &uri, &error).is_empty());
    workspace
        .project
        .build_diagnostic_sources
        .insert(uri.clone(), [uri.clone(), palette.clone()].into());
    assert_eq!(
        repair(&workspace, &uri, &error, "rgb").unwrap(),
        source.replacen("rbg", "rgb", 1)
    );

    let later = uri.join("later.sty").unwrap();
    workspace.documents.insert(
        later.clone(),
        Document::new(&later, r"\renewcommand{\bridge}[3]{}".into(), Some(9)),
    );
    workspace
        .project
        .recorded_inputs
        .get_mut(&uri)
        .unwrap()
        .insert(later.clone());
    workspace
        .project
        .build_diagnostic_sources
        .get_mut(&uri)
        .unwrap()
        .insert(later);
    error.data.as_mut().unwrap()["macroSources"] = serde_json::json!([uri, palette]);
    store(&mut workspace, &uri, error.clone());
    assert!(repair(&workspace, &uri, &error, "rgb").is_some());
    let mut malformed = error.clone();
    malformed.data.as_mut().unwrap()["macroSources"] = serde_json::json!("invalid");
    store(&mut workspace, &uri, malformed.clone());
    assert!(actions(&workspace, &uri, &malformed).is_empty());
    store(&mut workspace, &uri, error.clone());
    workspace.documents.insert(
        palette.clone(),
        Document::new(
            &palette,
            r"\definecolor{accent}{HTML}{123456}\newcommand{\done}[3]{}\def\bridge{\done}".into(),
            Some(4),
        ),
    );
    assert!(actions(&workspace, &uri, &error).is_empty());
}

#[test]
fn paused_parent_declarations_are_rechecked_without_lending_signatures_to_children() {
    let source = r"\bridge\textcolor[rbg]{.2,.4,.6}{MarkerUnique}";
    let (mut workspace, root) = setup(
        r"\input{content}",
        r"\newcommand{\done}{}\def\bridge{\done}",
    );
    let child = root.join("content.tex").unwrap();
    let palette = root.join("palette.tex").unwrap();
    workspace
        .documents
        .insert(child.clone(), Document::new(&child, source.into(), Some(7)));
    workspace
        .project
        .recorded_inputs
        .get_mut(&root)
        .unwrap()
        .insert(child.clone());
    workspace.project.build_diagnostic_sources.insert(
        root.clone(),
        [root.clone(), child.clone(), palette.clone()].into(),
    );
    let start = source.find("rbg").unwrap();
    let error = Diagnostic {
        range: Text::new(source.into()).range(start, start + 3),
        source: Some("latex-build".into()),
        message: MESSAGE.into(),
        code: Some(NumberOrString::String("undefined-color-model".into())),
        data: Some(
            serde_json::json!({"model":"rbg", "macroSources":[child,palette], "macroPausedSources":[root]}),
        ),
        ..Default::default()
    };
    workspace.project.build_diagnostics.insert(
        root.clone(),
        BTreeMap::from([(child.clone(), vec![error.clone()])]),
    );
    assert!(repair(&workspace, &child, &error, "rgb").is_some());
    for declaration in [
        r"\renewcommand{\bridge}[3]{}",
        r"\renewcommand{\done}[3]{}",
        r"\renewcommand{\textcolor}[2]{}",
    ] {
        workspace.documents.insert(
            root.clone(),
            Document::new(&root, format!("{declaration}\\input{{content}}"), Some(8)),
        );
        assert!(
            actions(&workspace, &child, &error).is_empty(),
            "{declaration}"
        );
    }
}
