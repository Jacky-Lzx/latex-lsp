use lsp_types::*;
use std::collections::BTreeMap;
use texquill::{
    compilation::build_log,
    editing,
    project::workspace::{Document, Workspace},
    text::Text,
};

fn extension(command: &str) -> &'static str {
    match command {
        "input" => "tex",
        "documentclass" | "LoadClass" | "LoadClassWithOptions" => "cls",
        _ => "sty",
    }
}

#[test]
fn file_errors_distinguish_macro_operands_from_completed_calls() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main.tex");
    for command in [
        "input",
        "usepackage",
        "RequirePackage",
        "RequirePackageWithOptions",
        "documentclass",
        "LoadClass",
        "LoadClassWithOptions",
    ] {
        for (declaration, prefix, precise) in [
            (
                r"\newenvironment{wrapped}[3]{}{}",
                r"\csuse{begin}{wrapped}\relax",
                false,
            ),
            (
                r"\newenvironment{wrapped}[3]{}{}",
                r"\csname begin\endcsname{wrapped}\relax",
                false,
            ),
            (
                r"\newenvironment{wrapped}[3]{}{}\def\start{\begin}",
                r"\start{wrapped}\relax",
                false,
            ),
            (
                r"\newenvironment{wrapped}[1]{}{}",
                r"\csuse{begin}{wrapped}{Unused}",
                true,
            ),
            (
                r"\newenvironment{wrapped}[1]{}{}",
                r"\csname begin\endcsname{wrapped}{Unused}",
                true,
            ),
            (
                r"\newenvironment{wrapped}[1]{}{}\def\start{\begin}",
                r"\start{wrapped}{Unused}",
                true,
            ),
            (
                r"\newenvironment{wrapped}{}{}",
                r"\wrapped\endwrapped",
                true,
            ),
            (
                r"\newenvironment{wrapped}{}{}\def\finish{\endwrapped}",
                r"\wrapped\finish",
                true,
            ),
            (
                r"\newenvironment{wrapped}[3]{}{}",
                r"\begin{wrapped}\relax",
                false,
            ),
            (r"\newenvironment{wrapped}[3]{}{}", r"\wrapped\relax", false),
            (
                r"\newenvironment{wrapped}[3]{}{}",
                r"\csuse{wrapped}\relax",
                false,
            ),
            (
                r"\newenvironment{wrapped}[4][default]{}{}",
                r"\begin{wrapped}[note]\relax",
                false,
            ),
            (
                r"\NewDocumentEnvironment{wrapped}{O{default} m m m}{}{}",
                r"\begin{wrapped}[note]\relax",
                false,
            ),
            (r"\newenvironment{wrapped}{}{}", r"\begin{wrapped}", true),
            (
                r"\newenvironment{wrapped}[1]{}{}",
                r"\begin{wrapped}{Unused}",
                true,
            ),
            (
                r"\newenvironment{wrapped}[1][default]{}{}",
                r"\begin{wrapped}",
                true,
            ),
            (
                r"\newcommand{\wrapped}[3]{}",
                r"\csuse{wrapped}\relax",
                false,
            ),
            (
                r"\newcommand{\wrapped}[3]{}",
                r"\csname wrapped\endcsname\relax",
                false,
            ),
            (r"\newcommand{\wrapped}{}", r"\csuse{wrapped}", true),
            (
                r"\newcommand{\wrapped}{}",
                r"\csname wrapped\endcsname",
                true,
            ),
            (
                r"\newcommand{\wrapped}[1]{}",
                r"\csuse{wrapped}{Unused}",
                true,
            ),
            (
                r"\newcommand{\wrapped}[1]{}",
                r"\csname wrapped\endcsname{Unused}",
                true,
            ),
            (r"\newcommand{\wrapped}[2]{}", r"\wrapped", false),
            (r"\newcommand{\wrapped}[3]{}", r"\wrapped{Unused}", false),
            (
                r"\newcommand{\wrapped}[3][default]{}",
                r"\wrapped[note]",
                false,
            ),
            (r"\def\wrapped#1#2{}", r"\wrapped", false),
            (
                r"\NewDocumentCommand{\wrapped}{s t+ O{default} m m}{}",
                r"\wrapped*+[note]",
                false,
            ),
            (
                r"\NewDocumentCommand{\wrapped}{d() m m}{}",
                r"\wrapped(note)",
                false,
            ),
            (
                r"\newcommand{\wrapped}[2]{}\def\bridge{\wrapped}",
                r"\bridge",
                false,
            ),
            (r"\def\bridge{\missingtarget}", r"\bridge", false),
            (r"\def\bridge{\bridge}", r"\bridge", false),
            ("", r"\unknown{Unused}", false),
            (r"\renewcommand{\input}[1]{}", "", false),
            (r"\renewcommand{\usepackage}[1]{}", "", false),
            (r"\newcommand{\done}{}", r"\done", true),
            (r"\newcommand{\done}{}\def\bridge{\done}", r"\bridge", true),
            (r"\newcommand{\done}[1]{}", r"\done{Unused}", true),
            (
                r"\NewDocumentCommand{\done}{s t+ O{default} m}{}",
                r"\done*+[note]{Unused}",
                true,
            ),
            ("", r"\relax\noindent\bfseries", true),
            ("", r"\selectfont", true),
            (r"\def\switch{\selectfont}", r"\switch", true),
            (
                "",
                r"\fontencoding{OT1}\fontfamily{cmr}\fontseries{m}\fontshape{n}\linespread{1.0}\fontsize{10}{12}\selectfont",
                true,
            ),
            ("", r"\usefont{OT1}{cmr}{m}{n}", true),
            ("", r"\fontfamily", false),
            ("", r"\fontsize{10}", false),
            (r"\renewcommand\selectfont[1]{}", r"\selectfont", false),
        ] {
            let call = format!("{prefix}\\{command}{{palete}}\\relax");
            let source = format!("{declaration}\n{call}\n");
            std::fs::write(&root, &source).unwrap();
            let file = format!("palete.{}", extension(command));
            let message = format!("LaTeX Error: File `{file}' not found.");
            let shown = if command == "input" {
                call.strip_suffix(r"\relax").unwrap()
            } else {
                &call
            };
            let log =
                format!("(./main.tex\n! {message}\n! Emergency stop.\n<read *>\nl.2 {shown}\n");
            let result = build_log::parse(&root, &log);
            let error = result[&Url::from_file_path(&root).unwrap()]
                .iter()
                .find(|e| e.message == message)
                .unwrap();
            assert_eq!(
                error.code.is_some(),
                precise,
                "{command}/{declaration}/{prefix}: {error:?}"
            );
            if precise {
                let start = source.rfind("palete").unwrap();
                assert_eq!(error.range, Text::new(source).range(start, start + 6));
                assert_eq!(error.data.as_ref().unwrap()["file"], file);
            } else {
                assert!(error.data.is_none());
            }
        }
    }
}

#[test]
fn file_callback_boundaries_cover_paths_and_loader_lookahead_on_later_lines() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main.tex");
    for command in ["input", "usepackage"] {
        for precise in [false, true] {
            let declaration = if precise {
                r"\newcommand{\wrapped}{}"
            } else {
                r"\newcommand{\wrapped}[2]{}"
            };
            let source = format!("{declaration}\n\\wrapped\\{command}\n{{palete}}\n\\relax\n");
            std::fs::write(&root, &source).unwrap();
            let file = format!("palete.{}", extension(command));
            let (line, echo) = if command == "input" {
                (3, "{palete}")
            } else {
                (4, r"\relax")
            };
            let message = format!("LaTeX Error: File `{file}' not found.");
            let log =
                format!("(./main.tex\n! {message}\n! Emergency stop.\n<read *>\nl.{line} {echo}\n");
            let result = build_log::parse(&root, &log);
            let error = result[&Url::from_file_path(&root).unwrap()]
                .iter()
                .find(|e| e.message == message)
                .unwrap();
            assert_eq!(
                error.code.is_some(),
                precise,
                "{command}/{precise}: {error:?}"
            );
        }
    }
}

#[test]
fn delimited_tex_macros_consume_file_commands_until_their_actual_delimiter() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main.tex");
    for command in ["usepackage", "RequirePackage", "LoadClass"] {
        for (declaration, prefix, tail, precise) in [
            (
                r"\def\wrapped#1 #2#3#4\stop{}",
                r"\wrapped Unused \relax",
                r"\stop",
                false,
            ),
            (
                "\\def\\wrapped#1 \t #2#3#4\\stop{}",
                "\\wrapped Unused\t\\relax",
                r"\stop",
                false,
            ),
            (
                "\\def\\wrapped#1 % keep\r\n #2#3#4\\stop{}\\def\\bridge{\\wrapped}",
                r"\bridge Unused \relax",
                r"\stop",
                false,
            ),
            (
                r"\def\wrapped#1 #2\stop{}",
                r"\wrapped Unused payload\stop",
                r"\relax",
                true,
            ),
            (
                r"\def\wrapped#1#2#3\stop{}",
                r"\wrapped\relax",
                r"\stop",
                false,
            ),
            (r"\def\wrapped#1\stop{}", r"\wrapped\relax", r"\stop", false),
            (
                r"\def\wrapped#1#2#3\stop{}\def\bridge{\wrapped}",
                r"\bridge\relax",
                r"\stop",
                false,
            ),
            (
                r"\def\wrapped\start#1\mid#2\stop{}",
                r"\wrapped\start\relax\mid",
                r"\stop",
                false,
            ),
            (
                r"\def\wrapped#1#2\stop{}",
                r"\wrapped\relax{Unused}\stop",
                r"\relax",
                true,
            ),
            (
                r"\def\wrapped#1#2\stop{}\def\bridge{\wrapped}",
                r"\bridge\relax{Unused}\stop",
                r"\relax",
                true,
            ),
        ] {
            let call = format!("{prefix}\\{command}{{palete}}{tail}");
            let source = format!("{declaration}\n{call}\n");
            std::fs::write(&root, &source).unwrap();
            let message = format!(
                "LaTeX Error: File `palete.{}' not found.",
                extension(command)
            );
            let line = declaration.lines().count() + 1;
            let log =
                format!("(./main.tex\n! {message}\n! Emergency stop.\n<read *>\nl.{line} {call}\n");
            let result = build_log::parse(&root, &log);
            let error = result[&Url::from_file_path(&root).unwrap()]
                .iter()
                .find(|error| error.message == message)
                .unwrap();
            assert_eq!(
                error.code.is_some(),
                precise,
                "{command}/{prefix}: {error:?}"
            );
            if precise {
                let at = source.rfind("palete").unwrap();
                assert_eq!(error.range, Text::new(source).range(at, at + 6));
            } else {
                assert!(error.data.is_none());
            }
        }
    }
}

#[test]
fn file_repairs_recheck_delimited_tex_macros_and_their_aliases() {
    for command in ["input", "usepackage", "documentclass"] {
        for (declaration, prefix) in [
            (
                r"\newenvironment{wrapped}[3]{}{}\def\bridge{\wrapped}",
                r"\begin{wrapped}\relax",
            ),
            (
                r"\newenvironment{wrapped}[3]{}{}\def\bridge{\wrapped}",
                r"\csuse{bridge}\relax",
            ),
            (
                r"\newcommand{\wrapped}[3]{}\def\bridge{\wrapped}",
                r"\csuse{bridge}\relax",
            ),
            (
                r"\newcommand{\wrapped}[3]{}\def\bridge{\wrapped}",
                r"\csname bridge\endcsname\relax",
            ),
            (
                r"\def\wrapped#1#2#3\stop{}\def\bridge{\wrapped}",
                r"\bridge\relax",
            ),
            (
                r"\def\wrapped#1 #2#3#4\stop{}\def\bridge{\wrapped}",
                r"\bridge Unused \relax",
            ),
        ] {
            let (_temp, mut workspace, uri, definitions, mut error) = fixture(command);
            let source = format!(r"{prefix}\{command}{{palete}}\stop\relax");
            let at = source.find("palete").unwrap();
            let doc = Document::new(&uri, source, Some(8));
            error.range = doc.text.range(at, at + 6);
            workspace.documents.insert(uri.clone(), doc);
            workspace.documents.insert(
                definitions.clone(),
                Document::new(&definitions, declaration.into(), Some(4)),
            );
            store(&mut workspace, &uri, &uri, &error);
            assert!(actions(&workspace, &uri, &error).is_empty(), "{command}");
            workspace.documents.insert(
                definitions.clone(),
                Document::new(
                    &definitions,
                    r"\newcommand{\done}{}\def\bridge{\done}\newenvironment{wrapped}{}{}".into(),
                    Some(5),
                ),
            );
            assert_eq!(actions(&workspace, &uri, &error).len(), 1, "{command}");
        }
    }
}

#[test]
fn grouped_user_macro_arguments_are_not_file_error_sites() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main.tex");
    for command in ["usepackage", "RequirePackage", "LoadClass"] {
        for (declaration, prefix, tail, precise) in [
            (r"\newcommand{\wrapped}[1]{}", r"\wrapped{", "}", false),
            (
                r"\newcommand{\wrapped}[1][default]{}",
                r"\wrapped[",
                "]",
                false,
            ),
            (
                r"\NewDocumentCommand{\wrapped}{m}{}",
                r"\wrapped{",
                "}",
                false,
            ),
            (
                r"\NewDocumentCommand{\wrapped}{O{default}}{}",
                r"\wrapped[",
                "]",
                false,
            ),
            (
                r"\NewDocumentCommand{\wrapped}{d()}{}",
                r"\wrapped(",
                ")",
                false,
            ),
            (
                r"\newcommand{\wrapped}[1]{}\def\bridge{\wrapped}",
                r"\bridge{",
                "}",
                false,
            ),
            ("", r"\unknown{", "}", false),
            (
                r"\newcommand{\wrapped}[1]{}\renewcommand{\wrapped}[2]{}",
                r"\wrapped{",
                "}",
                false,
            ),
            (
                r"\newcommand{\wrapped}[1]{}",
                r"\wrapped{Unused}",
                r"\relax",
                true,
            ),
            (
                r"\NewDocumentCommand{\wrapped}{d()}{}",
                r"\wrapped(note)",
                r"\relax",
                true,
            ),
        ] {
            let call = format!("{prefix}\\{command}{{palete}}{tail}");
            let source = format!("{declaration}\n{call}\n");
            std::fs::write(&root, &source).unwrap();
            let message = format!(
                "LaTeX Error: File `palete.{}' not found.",
                extension(command)
            );
            let log =
                format!("(./main.tex\n! {message}\n! Emergency stop.\n<read *>\nl.2 {call}\n");
            let result = build_log::parse(&root, &log);
            let error = result[&Url::from_file_path(&root).unwrap()]
                .iter()
                .find(|error| error.message == message)
                .unwrap();
            assert_eq!(
                error.code.is_some(),
                precise,
                "{command}/{prefix}: {error:?}"
            );
            if !precise {
                assert!(error.data.is_none());
            }
        }
    }
}

fn fixture(command: &str) -> (tempfile::TempDir, Workspace, Url, Url, Diagnostic) {
    let temp = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(temp.path().join("main.tex")).unwrap();
    let definitions = Url::from_file_path(temp.path().join("macros.sty")).unwrap();
    let source = format!("\\bridge\\{command}{{palete}}\\relax");
    std::fs::write(
        temp.path().join(format!("palette.{}", extension(command))),
        "",
    )
    .unwrap();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            versioned_edits: true,
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.clone(), Some(7)));
    workspace.documents.insert(
        definitions.clone(),
        Document::new(
            &definitions,
            r"\newcommand{\done}{}\def\bridge{\done}".into(),
            Some(3),
        ),
    );
    workspace
        .project
        .recorded_inputs
        .insert(uri.clone(), [uri.clone(), definitions.clone()].into());
    let start = source.find("palete").unwrap();
    let code = match extension(command) {
        "tex" => "missing-input",
        "sty" => "missing-package",
        _ => "missing-class",
    };
    let mut data = serde_json::json!({"file":format!("palete.{}", extension(command))});
    if command != "input" {
        data["loaderCommand"] = command.into();
    }
    let error = Diagnostic {
        range: workspace.documents[&uri].text.range(start, start + 6),
        source: Some("latex-build".into()),
        message: format!(
            "LaTeX Error: File `palete.{}' not found.",
            extension(command)
        ),
        code: Some(NumberOrString::String(code.into())),
        data: Some(data),
        ..Default::default()
    };
    store(&mut workspace, &uri, &uri, &error);
    (temp, workspace, uri, definitions, error)
}

fn store(workspace: &mut Workspace, root: &Url, uri: &Url, error: &Diagnostic) {
    workspace.project.build_diagnostics.insert(
        root.clone(),
        BTreeMap::from([(uri.clone(), vec![error.clone()])]),
    );
}

fn actions(workspace: &Workspace, uri: &Url, error: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        uri,
        error.range,
        &CodeActionContext {
            diagnostics: vec![error.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
}

#[test]
fn file_repairs_recheck_loaded_macro_sources_and_ignore_later_redefinitions() {
    for command in ["input", "usepackage", "documentclass"] {
        let (_temp, mut workspace, uri, definitions, mut error) = fixture(command);
        workspace
            .project
            .build_diagnostic_sources
            .insert(uri.clone(), [uri.clone()].into());
        assert!(actions(&workspace, &uri, &error).is_empty());
        workspace
            .project
            .build_diagnostic_sources
            .insert(uri.clone(), [uri.clone(), definitions.clone()].into());
        let fixes = actions(&workspace, &uri, &error);
        assert_eq!(fixes.len(), 1, "{command}: {fixes:?}");
        let CodeActionOrCommand::CodeAction(action) = &fixes[0] else {
            panic!()
        };
        let Some(DocumentChanges::Edits(changes)) =
            action.edit.as_ref().unwrap().document_changes.as_ref()
        else {
            panic!()
        };
        assert_eq!(changes[0].text_document.version, Some(7));
        let later = uri.join("later.sty").unwrap();
        workspace.documents.insert(
            later.clone(),
            Document::new(&later, r"\renewcommand{\done}[2]{}".into(), None),
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
        error.data.as_mut().unwrap()["macroSources"] = serde_json::json!([uri, definitions]);
        store(&mut workspace, &uri, &uri, &error);
        assert_eq!(actions(&workspace, &uri, &error).len(), 1);
        workspace.documents.insert(
            definitions.clone(),
            Document::new(
                &definitions,
                r"\newcommand{\done}[2]{}\def\bridge{\done}".into(),
                Some(4),
            ),
        );
        assert!(actions(&workspace, &uri, &error).is_empty());
    }
}

#[test]
fn file_repairs_validate_paused_parent_metadata_and_current_signatures() {
    for command in ["input", "usepackage", "documentclass"] {
        let (_temp, mut workspace, root, definitions, mut error) = fixture(command);
        let child = root.join("content.tex").unwrap();
        let source = workspace.documents[&root].text.source.clone();
        workspace
            .documents
            .insert(child.clone(), Document::new(&child, source, Some(3)));
        workspace.documents.insert(
            root.clone(),
            Document::new(&root, r"\input{content}".into(), Some(7)),
        );
        workspace
            .project
            .recorded_inputs
            .get_mut(&root)
            .unwrap()
            .insert(child.clone());
        workspace.project.build_diagnostic_sources.insert(
            root.clone(),
            [root.clone(), child.clone(), definitions.clone()].into(),
        );
        error.data.as_mut().unwrap()["macroSources"] = serde_json::json!([child, definitions]);
        error.data.as_mut().unwrap()["macroPausedSources"] = serde_json::json!([root]);
        store(&mut workspace, &root, &child, &error);
        assert_eq!(actions(&workspace, &child, &error).len(), 1, "{command}");
        for value in [
            serde_json::json!("invalid"),
            serde_json::json!([child]),
            serde_json::json!(["file:///outside.tex"]),
        ] {
            let mut invalid = error.clone();
            invalid.data.as_mut().unwrap()["macroPausedSources"] = value;
            store(&mut workspace, &root, &child, &invalid);
            assert!(actions(&workspace, &child, &invalid).is_empty());
        }
        store(&mut workspace, &root, &child, &error);
        for declaration in [
            r"\newenvironment{done}[2]{}{}",
            r"\newenvironment{bridge}[2]{}{}",
            r"\renewcommand{\done}[2]{}",
            r"\renewcommand{\bridge}[2]{}",
            r"\renewcommand{\input}[1]{}",
            r"\renewcommand{\usepackage}[1]{}",
            r"\renewcommand{\documentclass}[1]{}",
        ] {
            workspace.documents.insert(
                root.clone(),
                Document::new(&root, format!("{declaration}\\input{{content}}"), Some(10)),
            );
            assert!(
                actions(&workspace, &child, &error).is_empty(),
                "{command}: {declaration}"
            );
        }
    }
}
