use latex_lsp::{
    build, build_log,
    config::BuildConfig,
    editing,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;
use std::collections::BTreeMap;

const MESSAGE: &str = "Package xcolor Error: Undefined color `accnet'.";
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
    let uri = Url::parse("file:///color-fixes/main.tex").unwrap();
    let palette = Url::parse("file:///color-fixes/palette.tex").unwrap();
    let mut workspace = Workspace {
        code_action_literals: true,
        versioned_edits: true,
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
        .recorded_inputs
        .insert(uri.clone(), [uri.clone(), palette].into());
    (workspace, uri)
}
fn store(workspace: &mut Workspace, uri: &Url, diagnostic: Diagnostic) {
    workspace.build_diagnostics.insert(
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
                if action.title == format!("Replace with color `{name}`") =>
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

fn direct_calls() -> Vec<(&'static str, &'static str)> {
    vec![
        (r"\color{accnet}", "MarkerUnique"),
        (r"\textcolor{accnet}{MarkerUnique}", ""),
        (r"\textcolor{-red!35!-accnet!50!white}{MarkerUnique}", ""),
        (r"\colorbox{accnet}{MarkerUnique}", ""),
        (r"\fcolorbox{accnet}{red}{MarkerUnique}", ""),
        (r"\fcolorbox{red}{accnet}{MarkerUnique}", ""),
        (r"\pagecolor{accnet}", "MarkerUnique"),
        (r"\colorlet{alias}{accnet!30!white}", "MarkerUnique"),
        (r"\definecolor{alias}{named}{accnet}", "MarkerUnique"),
        (r"\providecolor{alias}{named}{accnet}", "MarkerUnique"),
        (r"\xdefinecolor{alias}{named}{accnet}", "MarkerUnique"),
        (r"\maskcolors[rgb]{accnet}", "MarkerUnique"),
        (r"\extractcolorspec{accnet}\out", " MarkerUnique"),
        (r"\extractcolorspecs{accnet}\model\spec", " MarkerUnique"),
        (
            r"\convertcolorspec{named}{accnet}{rgb}\out",
            " MarkerUnique",
        ),
    ]
}

#[test]
fn color_commands_consumed_as_unbraced_macro_arguments_are_not_error_sites() {
    for prefix in [
        r"\newcommand{\wrapped}[3]{\textcolor{accnet}{#3}}\wrapped",
        r"\newcommand{\wrapped}[4]{\textcolor{accnet}{#4}}\wrapped{Unused}",
        r"\newcommand{\wrapped}[4][default]{\textcolor{accnet}{#4}}\wrapped[note]",
        r"\newcommand{\wrapped}[4][default]{\textcolor{accnet}{#4}}\wrapped",
        r"\def\wrapped#1#2#3{\textcolor{accnet}{#3}}\wrapped",
        r"\NewDocumentCommand{\wrapped}{O{default} m m m}{\textcolor{accnet}{#4}}\wrapped[note]",
        r"\NewDocumentCommand{\wrapped}{s t+ m m m}{\textcolor{accnet}{#5}}\wrapped*+",
        r"\NewDocumentCommand{\wrapped}{d() m m m}{\textcolor{accnet}{#4}}\wrapped(note)",
        r"\unknown{Unused}",
        r"\unknown[note]{Unused}",
        r"\expandafter",
        r"\newcommand{\noindent}[3]{\textcolor{accnet}{#3}}\noindent",
        r"\newcommand{\wrapped}[3]{\textcolor{accnet}{#3}}\renewcommand{\wrapped}[3]{\textcolor{accnet}{#3}}\wrapped",
    ] {
        let source = format!("{prefix}\\textcolor{{accnet}}{{MarkerUnique}}");
        let error = diagnostic(&source, &format!("l.1 {source}"), false);
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{source}: {error:?}"
        );
        let (mut workspace, uri) = setup(&source, r"\definecolor{accent}{HTML}{123456}");
        store(&mut workspace, &uri, error.clone());
        assert!(actions(&workspace, &uri, &error).is_empty());
    }
}

#[test]
fn completed_macro_calls_and_argument_free_switches_keep_exact_color_fixes() {
    for prefix in [
        r"\newcommand{\done}{}\done",
        r"\def\done{}\done",
        r"\NewDocumentCommand{\done}{}{}\done",
        r"\newcommand{\done}[1]{}\done{Unused}",
        r"\newcommand{\done}[1][default]{}\done[note]",
        r"\newcommand{\done}[1][default]{}\done",
        r"\NewDocumentCommand{\done}{s t+ O{default} m}{}\done*+[note]{Unused}",
        r"\NewDocumentCommand{\done}{d() m}{}\done(note){Unused}",
        r"\NewDocumentCommand{\done}{t\flag}{}\done\flag",
        r"\NewDocumentCommand{\done}{d\open\close}{}\done\open note\close",
        r"\relax\noindent\bfseries\itshape",
        r"\textbf{Heading}",
        r"\typeout{\detokenize{😀}}",
        r"\extractcolorspec{red}\spec",
        r"\extractcolorspecs{red}\model\spec",
        r"\convertcolorspec{named}{red}{rgb}\out",
    ] {
        let source = format!("{prefix}\\textcolor{{accnet}}{{MarkerUnique}}");
        let error = diagnostic(&source, &format!("l.1 {source}"), false);
        let start = source.rfind("accnet").unwrap();
        assert_eq!(
            error.range,
            Text::new(source.clone()).range(start, start + 6),
            "{source}"
        );
        let (mut workspace, uri) = setup(&source, r"\definecolor{accent}{HTML}{123456}");
        store(&mut workspace, &uri, error.clone());
        assert_eq!(
            repair(&workspace, &uri, &error, "accent").unwrap(),
            source.replacen("{accnet}", "{accent}", 1)
        );
    }
}

#[test]
fn single_control_word_aliases_inherit_argument_rules_for_color_diagnostics() {
    for alias in [
        r"\newcommand{\bridge}{\wrapped}",
        r"\newcommand*{\bridge}[0]{\wrapped}",
        r"\def\bridge{\wrapped}",
        r"\gdef\bridge{\wrapped}",
        r"\csdef{bridge}{\wrapped}",
        r"\NewDocumentCommand{\bridge}{}{\wrapped}",
        r"\def\bridge{\aliasstep}\def\aliasstep{\wrapped}",
    ] {
        for (target, arguments) in [
            (r"\newcommand{\wrapped}[3]{\textcolor{accnet}{#3}}", ""),
            (
                r"\newcommand{\wrapped}[4][default]{\textcolor{accnet}{#4}}",
                "[note]",
            ),
            (
                r"\NewDocumentCommand{\wrapped}{s t+ m m m}{\textcolor{accnet}{#5}}",
                "*+",
            ),
            (
                r"\NewDocumentCommand{\wrapped}{d() m m m}{\textcolor{accnet}{#4}}",
                "(note)",
            ),
        ] {
            let source =
                format!("{target}{alias}\\bridge{arguments}\\textcolor{{accnet}}{{MarkerUnique}}");
            let error = diagnostic(&source, &format!("l.1 {source}"), false);
            assert!(
                error.code.is_none() && error.data.is_none(),
                "{source}: {error:?}"
            );
            let (mut workspace, uri) = setup(&source, r"\definecolor{accent}{HTML}{123456}");
            store(&mut workspace, &uri, error.clone());
            assert!(actions(&workspace, &uri, &error).is_empty());
        }
    }
    for prefix in [
        r"\newcommand{\done}{}\newcommand{\bridge}{\done}\bridge",
        r"\def\bridge{\relax}\bridge",
        r"\newcommand{\bridge}[0]{\bfseries}\bridge",
        r"\def\bridge{\textbf}\bridge{Heading}",
        r"\NewDocumentCommand{\done}{s t+ O{default} m}{}\NewDocumentCommand{\bridge}{}{\done}\bridge*+[note]{Unused}",
        r"\NewDocumentCommand{\done}{d() m}{}\def\bridge{\done}\bridge(note){Unused}",
    ] {
        let source = format!("{prefix}\\textcolor{{accnet}}{{MarkerUnique}}");
        let error = diagnostic(&source, &format!("l.1 {source}"), false);
        let start = source.rfind("accnet").unwrap();
        assert_eq!(
            error.range,
            Text::new(source.clone()).range(start, start + 6),
            "{source}"
        );
        let (mut workspace, uri) = setup(&source, r"\definecolor{accent}{HTML}{123456}");
        store(&mut workspace, &uri, error.clone());
        assert_eq!(
            repair(&workspace, &uri, &error, "accent").unwrap(),
            source.replacen("{accnet}", "{accent}", 1)
        );
    }
}

#[test]
fn multiline_consumed_color_operands_require_more_than_the_final_line_echo() {
    for source in [
        "\\textcolor{% before\naccnet% after\n}{MarkerUnique}",
        "\\newcommand{\\wrapped}[3]{\\textcolor{accnet}{#3}}\n\\wrapped\\textcolor{% before\naccnet% after\n}{MarkerUnique}",
        "\\textcolor{accnet}\n{MarkerUnique}",
    ] {
        let line = source.lines().count();
        let echo = source.lines().last().unwrap();
        let error = diagnostic(source, &format!("l.{line} {echo}"), false);
        assert!(error.code.is_none() && error.data.is_none(), "{source}: {error:?}");
        assert_eq!(error.range.start, Position::new(line as u32 - 1, 0));
        let (mut workspace, uri) = setup(source, r"\definecolor{accent}{HTML}{123456}");
        store(&mut workspace, &uri, error.clone());
        assert!(actions(&workspace, &uri, &error).is_empty());
    }
    // A command can start on an earlier line when all consumed operands are
    // still fully present in the echo.
    let source = "\\textcolor\n{accnet}{MarkerUnique}";
    let error = diagnostic(source, "l.2 {accnet}{MarkerUnique}", false);
    let start = source.find("accnet").unwrap();
    assert_eq!(
        error.range,
        Text::new(source.into()).range(start, start + 6)
    );
    assert_eq!(
        error.code,
        Some(NumberOrString::String("undefined-color".into()))
    );
}

#[test]
fn direct_color_errors_select_literal_name_in_full_and_unique_shortened_contexts() {
    for (call, rest) in direct_calls() {
        let source = format!("😀 {call}{rest} % accnet\r\n");
        let text = Text::new(source.clone());
        for numbered in [true, false] {
            for echo in [format!("😀 {call}"), format!("...{call}")] {
                let error = diagnostic(&source, &format!("l.1 {echo}"), numbered);
                let start = source.find("accnet").unwrap();
                assert_eq!(
                    error.range,
                    text.range(start, start + 6),
                    "{call}: {error:?}"
                );
                assert_eq!(
                    error.code,
                    Some(NumberOrString::String("undefined-color".into()))
                );
                assert_eq!(error.data.as_ref().unwrap()["color"], "accnet");
                assert_eq!(error.severity, Some(DiagnosticSeverity::ERROR));
            }
        }
    }
}

#[test]
fn ambiguous_or_expanded_colors_and_delayed_settings_keep_line_diagnostics() {
    for (source, echo) in [
        (r"\color{ accnet }", r"\color{ accnet }"),
        (r"\textcolor{\dynamic}{Text}", r"\textcolor{\dynamic}{Text}"),
        (
            r"\textcolor{accnet}{Text}\wrapper{accnet}",
            r"\textcolor{accnet}{Text}\wrapper{accnet}",
        ),
        (
            r"\newcommand{\paint}[1]{\textcolor{#1}{Text}}\paint{accnet}",
            r"...\textcolor{#1}{Text}}\paint{accnet}",
        ),
        (
            r"\colorlet{alias}{accnet!50!accnet}",
            r"\colorlet{alias}{accnet!50!accnet}",
        ),
        (
            r"\fcolorbox{accnet}{accnet}{Text}",
            r"\fcolorbox{accnet}{accnet}{Text}",
        ),
        (
            r"\blendcolors{!30!accnet}\color{red}",
            r"\blendcolors{!30!accnet}\color{red}",
        ),
        (
            r"\iffalse\color{accnet}\fi\unknowncolor{accnet}",
            r"...color{accnet}",
        ),
        (
            r"\verb|\color{accnet}|\unknowncolor{accnet}",
            r"...color{accnet}",
        ),
        (r"\color{accnet} % color{accnet}", r"...color{accnet}"),
        (
            r"\detokenize{\color{accnet}}",
            r"\detokenize{\color{accnet}}",
        ),
        (
            r"\renewcommand{\color}[1]{}\color{accnet}",
            r"\renewcommand{\color}[1]{}\color{accnet}",
        ),
        (r"\color[HTML]{accnet}", r"\color[HTML]{accnet}"),
        (
            r"\extractcolorspec{accnet}{\out}",
            r"\extractcolorspec{accnet}{\out}",
        ),
    ] {
        let error = diagnostic(source, &format!("l.1 {echo}"), true);
        assert!(
            error.code.is_none() && error.data.is_none(),
            "{source}: {error:?}"
        );
        assert_eq!(error.range.start.character, 0);
    }
    for context in [
        r"l.1 \textcolor{accnet}",
        "<argument> accnet\nl.1 \\color{accnet}",
        "l.2 \\color{accnet}",
        "l.1 \\color{different}",
    ] {
        let error = diagnostic(r"\color{accnet}", context, true);
        assert!(error.code.is_none(), "{context}: {error:?}");
    }
}

#[test]
fn color_spelling_repairs_use_project_definitions_and_preserve_mixes_body_and_comments() {
    let source = "😀 \\textcolor{-red!35!-accnet!50!white}{accnet} % accnet\r\n";
    let error = diagnostic(
        source,
        "l.1 😀 \\textcolor{-red!35!-accnet!50!white}{accnet}",
        true,
    );
    let (mut workspace, uri) = setup(source, r"\definecolor{accent}{HTML}{123456}");
    let unrelated = Url::parse("file:///color-fixes/unrelated.tex").unwrap();
    workspace.documents.insert(
        unrelated.clone(),
        Document::new(&unrelated, r"\definecolor{accnot}{rgb}{1,0,0}".into(), None),
    );
    store(&mut workspace, &uri, error.clone());
    assert_eq!(
        repair(&workspace, &uri, &error, "accent").unwrap(),
        source.replacen("-accnet!", "-accent!", 1)
    );
    assert!(repair(&workspace, &uri, &error, "accnot").is_none());
    let mut malformed = error.clone();
    malformed.code = None;
    store(&mut workspace, &uri, malformed.clone());
    assert!(actions(&workspace, &uri, &malformed).is_empty());
    for definitions in [
        r"\definecolor{accnet}{HTML}{123456}\definecolor{accent}{HTML}{FFFFFF}",
        r"\definecolor{accent}{HTML}{123456}\colorlet{accent}{red}",
    ] {
        let (mut workspace, uri) = setup(source, definitions);
        store(&mut workspace, &uri, error.clone());
        assert!(actions(&workspace, &uri, &error).is_empty());
    }
}

#[test]
fn builtin_color_suggestions_require_consistent_message_data_and_source_ranges() {
    let source = r"\color{rde}";
    let (mut workspace, uri) = setup(source, "");
    let text = &workspace.documents[&uri].text;
    let error = Diagnostic {
        range: text.range(7, 10),
        source: Some("latex-build".into()),
        message: "Package xcolor Error: Undefined color `rde'.".into(),
        code: Some(NumberOrString::String("undefined-color".into())),
        data: Some(serde_json::json!({"color":"rde"})),
        ..Default::default()
    };
    store(&mut workspace, &uri, error.clone());
    assert_eq!(
        repair(&workspace, &uri, &error, "red").unwrap(),
        r"\color{red}"
    );
    let mut guards = vec![];
    let mut e = error.clone();
    e.message = "Package xcolor Error: Undefined color `different'.".into();
    guards.push(e);
    let mut e = error.clone();
    e.data = Some(serde_json::json!({"color":"different"}));
    guards.push(e);
    let mut e = error.clone();
    e.range.start.character = 0;
    guards.push(e);
    let mut e = error.clone();
    e.data = Some(serde_json::json!({"color":"rde", "unresolvedSource":true}));
    guards.push(e);
    for e in guards {
        store(&mut workspace, &uri, e.clone());
        assert!(actions(&workspace, &uri, &e).is_empty(), "{e:?}");
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and xcolor"]
fn real_undefined_color_repairs_compile_all_direct_call_forms() {
    for engine in ["pdflatex", "xelatex"] {
        for (call, rest) in direct_calls() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let main = root.join("main.tex");
            let source = format!("\\documentclass{{article}}\n\\usepackage{{xcolor}}\n\\definecolor{{accent}}{{HTML}}{{123456}}\n\\begin{{document}}\n{call}{rest}\n\\end{{document}}");
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
                Some(NumberOrString::String("undefined-color".into())),
                "{engine}/{call}: {error:?}"
            );
            let mut workspace = Workspace {
                versioned_edits: true,
                code_action_literals: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(uri.clone(), source.clone(), 7);
            store(&mut workspace, &uri, error.clone());
            let updated = repair(&workspace, &uri, &error, "accent")
                .unwrap_or_else(|| panic!("{engine}/{call}: {error:?}"));
            assert_eq!(updated, source.replacen("accnet", "accent", 1));
            std::fs::write(&main, updated).unwrap();
            let after = build::run(&main, &config, &Default::default());
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
            assert!(after.pdf.as_ref().unwrap().is_file());
        }
    }
}

#[test]
fn project_macro_arities_revalidate_cross_file_color_fixes_and_reject_changed_targets() {
    let source = "\\bridge\\textcolor{accnet}{MarkerUnique}";
    let (mut workspace, uri) = setup(
        source,
        "\\definecolor{accent}{HTML}{123456}\\newcommand{\\done}{}\\def\\bridge{\\done}",
    );
    let palette = Url::parse("file:///color-fixes/palette.tex").unwrap();
    let start = source.find("accnet").unwrap();
    let mut error = Diagnostic {
        range: Text::new(source.into()).range(start, start + "accnet".len()),
        source: Some("latex-build".into()),
        message: MESSAGE.into(),
        code: Some(NumberOrString::String("undefined-color".into())),
        data: Some(serde_json::json!({"color":"accnet"})),
        ..Default::default()
    };
    store(&mut workspace, &uri, error.clone());
    workspace
        .build_diagnostic_sources
        .insert(uri.clone(), [uri.clone()].into());
    assert!(actions(&workspace, &uri, &error).is_empty());
    workspace
        .build_diagnostic_sources
        .insert(uri.clone(), [uri.clone(), palette.clone()].into());
    assert_eq!(
        repair(&workspace, &uri, &error, "accent").unwrap(),
        source.replacen("accnet", "accent", 1)
    );

    let later = uri.join("later.sty").unwrap();
    workspace.documents.insert(
        later.clone(),
        Document::new(&later, r"\renewcommand{\bridge}[3]{}".into(), Some(9)),
    );
    workspace
        .recorded_inputs
        .get_mut(&uri)
        .unwrap()
        .insert(later.clone());
    workspace
        .build_diagnostic_sources
        .get_mut(&uri)
        .unwrap()
        .insert(later);
    error.data.as_mut().unwrap()["macroSources"] = serde_json::json!([uri, palette]);
    store(&mut workspace, &uri, error.clone());
    assert!(repair(&workspace, &uri, &error, "accent").is_some());
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
fn a_child_error_retains_color_candidates_from_its_paused_parent() {
    let source = r"\textcolor{accnet}{MarkerUnique}";
    let (mut workspace, root) = setup(r"\definecolor{accent}{HTML}{123456}\input{content}", "");
    let child = root.join("content.tex").unwrap();
    workspace
        .documents
        .insert(child.clone(), Document::new(&child, source.into(), Some(7)));
    workspace
        .recorded_inputs
        .get_mut(&root)
        .unwrap()
        .insert(child.clone());
    workspace
        .build_diagnostic_sources
        .insert(root.clone(), [root.clone(), child.clone()].into());
    let mut error = diagnostic(source, &format!("l.1 {source}"), true);
    error.data.as_mut().unwrap()["macroSources"] = serde_json::json!([child]);
    error.data.as_mut().unwrap()["macroPausedSources"] = serde_json::json!([root]);
    workspace.build_diagnostics.insert(
        root.clone(),
        BTreeMap::from([(child.clone(), vec![error.clone()])]),
    );
    assert_eq!(
        repair(&workspace, &child, &error, "accent").unwrap(),
        source.replace("accnet", "accent")
    );
    for value in [
        serde_json::json!("invalid"),
        serde_json::json!([child]),
        serde_json::json!(["file:///outside-project.tex"]),
    ] {
        let mut malformed = error.clone();
        malformed.data.as_mut().unwrap()["macroPausedSources"] = value;
        workspace
            .build_diagnostics
            .get_mut(&root)
            .unwrap()
            .insert(child.clone(), vec![malformed.clone()]);
        assert!(actions(&workspace, &child, &malformed).is_empty());
    }
    workspace
        .build_diagnostics
        .get_mut(&root)
        .unwrap()
        .insert(child.clone(), vec![error.clone()]);
    workspace.documents.insert(
        root.clone(),
        Document::new(
            &root,
            r"\definecolor{accent}{HTML}{123456}\renewcommand{\textcolor}[2]{}\input{content}"
                .into(),
            Some(8),
        ),
    );
    assert!(actions(&workspace, &child, &error).is_empty());
}
