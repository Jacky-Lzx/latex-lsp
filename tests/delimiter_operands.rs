use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

const ERROR: &str = "Missing delimiter (. inserted).";
const CODE: &str = "missing-delimiter-operand";

fn log(line: u32, prefix: &str, token: &str, numbered: bool) -> String {
    let header = if numbered {
        format!("./main.tex:{line}: {ERROR}")
    } else {
        format!("! {ERROR}")
    };
    format!("(./main.tex\n{header}\n<to be read again>\n                   {token}\nl.{line} {prefix}\n\n)\n")
}

fn workspace(source: &str, log: &str) -> (tempfile::TempDir, Workspace, Url, Vec<Diagnostic>) {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    let root = Url::from_file_path(&main).unwrap();
    let parsed = build_log::parse(&main, log);
    let items = parsed.get(&root).cloned().unwrap_or_default();
    let mut workspace = Workspace {
        code_action_literals: true,
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(directory.path().into());
    workspace.open(root.clone(), source.into(), 9);
    workspace.build_diagnostics.insert(root.clone(), parsed);
    (directory, workspace, root, items)
}

fn actions(workspace: &Workspace, uri: &Url, item: &Diagnostic) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        uri,
        item.range,
        &CodeActionContext {
            diagnostics: vec![item.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
}

fn edit(actions: CodeActionResponse, uri: &Url, command: &str) -> TextEdit {
    assert_eq!(actions.len(), 1, "{actions:?}");
    let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
        panic!("{actions:?}")
    };
    assert_eq!(
        action.title,
        format!("Insert invisible delimiter after '{command}'")
    );
    let Some(DocumentChanges::Edits(documents)) = &action.edit.as_ref().unwrap().document_changes
    else {
        panic!("{action:?}")
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *uri);
    assert_eq!(documents[0].text_document.version, Some(9));
    assert_eq!(documents[0].edits.len(), 1);
    let OneOf::Left(edit) = &documents[0].edits[0] else {
        panic!("{action:?}")
    };
    edit.clone()
}

fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .map(|edit| TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
}

#[test]
fn delimiter_operands_point_to_the_command_and_insert_a_dot_preserving_source() {
    for numbered in [false, true] {
        for newline in ["\n", "\r\n"] {
            for (source, line, prefix, token, command) in [
                ("😀 $\\left a+b\\right)$", 1, "😀 $\\left a", "a", r"\left"),
                (
                    r"$\left(a+b\right x$",
                    1,
                    r"$\left(a+b\right x",
                    "x",
                    r"\right",
                ),
                (
                    r"$\left(a\middle b\right)$",
                    1,
                    r"$\left(a\middle b",
                    "b",
                    r"\middle",
                ),
                (
                    r"$\left\alpha+b\right)$",
                    1,
                    r"$\left\alpha",
                    r"\alpha",
                    r"\left",
                ),
                (r"$\left(a\right_1$", 1, r"$\left(a\right_", "_", r"\right"),
                (r"$\left(a\right$", 1, r"$\left(a\right$", "$", r"\right"),
                (r"$\left{a\right)}$", 1, r"$\left{", "{", r"\left"),
                (r"$\left 1+2\right)$", 1, r"$\left 1", "1", r"\left"),
                (r"$\left\right)$", 1, r"$\left\right", r"\right", r"\left"),
                ("$\\left % keep\n a+b\\right)$", 2, " a", "a", r"\left"),
                (
                    "$\\left(a\\middle% keep\nb\\right)$",
                    2,
                    "b",
                    "b",
                    r"\middle",
                ),
            ] {
                let source = source.replace('\n', newline);
                let log = log(line, prefix, token, numbered).replace('\n', newline);
                let (_directory, workspace, root, items) = workspace(&source, &log);
                assert_eq!(items.len(), 1, "{source}: {items:?}");
                let item = &items[0];
                assert_eq!(
                    item.code,
                    Some(NumberOrString::String(CODE.into())),
                    "{source}: {item:?}"
                );
                let at = source.find(command).unwrap();
                assert_eq!(
                    item.range,
                    workspace.documents[&root]
                        .text
                        .range(at, at + command.len())
                );
                let data = item.data.as_ref().unwrap();
                assert_eq!(data["sourceLine"], line - 1);
                assert_eq!(data["recoveryToken"], token);
                assert_eq!(data["delimiterCommand"], command);
                let edit = edit(actions(&workspace, &root, item), &root, command);
                assert_eq!(edit.range.start, item.range.end);
                assert_eq!(edit.range.end, item.range.end);
                assert_eq!(edit.new_text, ".");
                let mut expected = source.clone();
                expected.insert(at + command.len(), '.');
                assert_eq!(apply(&source, vec![edit]), expected);
            }
        }
    }
    let source = r"$\left a\middle b\right c$";
    let mut value = "(./main.tex\n".to_owned();
    for (prefix, token) in [
        (r"$\left a", "a"),
        (r"$\left a\middle b", "b"),
        (r"$\left a\middle b\right c", "c"),
    ] {
        value.push_str(
            log(1, prefix, token, false)
                .trim_start_matches("(./main.tex\n")
                .trim_end_matches(")\n"),
        );
    }
    value.push_str(")\n");
    let (_directory, workspace, root, items) = workspace(source, &value);
    assert_eq!(items.len(), 3);
    let edits = items
        .iter()
        .map(|item| {
            edit(
                actions(&workspace, &root, item),
                &root,
                item.data.as_ref().unwrap()["delimiterCommand"]
                    .as_str()
                    .unwrap(),
            )
        })
        .collect();
    assert_eq!(apply(source, edits), r"$\left. a\middle. b\right. c$");
}

#[test]
fn delimiter_operand_attribution_requires_exact_tokens_and_executable_primitives() {
    for (source, prefix, token, precise) in [
        (r"$\left a\right)$", r"...\left a", "a", true),
        (
            r"$\left a\right)$ and $\left a\right)$",
            r"...\left a",
            "a",
            false,
        ),
        (r"$\left a\right)$", r"$\left b", "b", false),
        (r"$\left a\right)$", r"$\left a", "b", false),
        (r"$\left\alpha\right)$", r"$\left\alpha", r"\beta", false),
        (r"$\left\alphaX\right)$", r"$\left\alpha", r"\alpha", false),
        (r"$\left\probe\right)$", r"$\left\probe", "a", false),
        (r"$\left\unknown$", r"$\left\unknown$", "$", false),
        (r"% $\left a\right)$", r"% $\left a", "a", false),
        (r"\verb|$\left a\right)$|", r"\verb|$\left a", "a", false),
        (
            r"\newcommand{\probe}{$\left a\right)$}",
            r"\newcommand{\probe}{$\left a",
            "a",
            false,
        ),
        (
            r"\newcommand{\probe}[1]{#1} $\probe{\left a\right)}$",
            r"\newcommand{\probe}[1]{#1} $\probe{\left a",
            "a",
            false,
        ),
        (
            r"\newcommand{\probe}[1]{#1} $\probe\left a\right)$",
            r"\newcommand{\probe}[1]{#1} $\probe\left a",
            "a",
            false,
        ),
        (
            r"\def\left{} $\left a\right)$",
            r"\def\left{} $\left a",
            "a",
            false,
        ),
        (
            r"\def\right{} $\left(a\right x$",
            r"\def\right{} $\left(a\right x",
            "x",
            false,
        ),
        (
            r"\def\middle{} $\left(a\middle b\right)$",
            r"\def\middle{} $\left(a\middle b",
            "b",
            false,
        ),
        (
            r"\def\alpha{} $\left\alpha\right)$",
            r"\def\alpha{} $\left\alpha",
            r"\alpha",
            false,
        ),
        (r"\meaning\left a", r"\meaning\left a", "a", false),
        (r"$\left^^61\right)$", r"$\left^", "^", false),
        ("$\\left 你\\right)$", "$\\left 你", "你", false),
        (r"\big a", r"\big a", "a", false),
        (r"$\left a\right)$", r"$\left a", "ab", false),
    ] {
        let (_directory, workspace, root, items) = workspace(source, &log(1, prefix, token, false));
        assert_eq!(items.len(), 1, "{source}: {items:?}");
        assert_eq!(items[0].code.is_some(), precise, "{source}: {:?}", items[0]);
        assert_eq!(
            !actions(&workspace, &root, &items[0]).is_empty(),
            precise,
            "{source}"
        );
    }
    for frame in [
        "",
        "<inserted text>\n.\n",
        "<to be read again>\n",
        "<to be read again>\na b\n",
        "<to be read again>\na\n\\probe ->a\n",
        "\\probe ->a\n<to be read again>\na\n",
    ] {
        let value = format!("(./main.tex\n! {ERROR}\n{frame}l.1 $\\left a\n\n)\n");
        let (_directory, workspace, root, items) = workspace(r"$\left a\right)$", &value);
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none(), "{frame}: {:?}", items[0]);
        assert!(actions(&workspace, &root, &items[0]).is_empty());
    }
}

#[test]
fn delimiter_echoes_and_help_do_not_change_child_parent_or_sibling_sources() {
    let help = "I was expecting to see something like `(' or `\\{' or\n`\\}' here. If you typed, e.g., `{' instead of `\\{', you\nshould probably delete the `{' by typing `1' now, so that\nbraces don't get unbalanced. Otherwise just proceed.\nAcceptable delimiters are characters whose \\delcode is\nnonnegative, or you can use `\\delimiter <delimiter code>'.\n\n";
    for numbered in [false, true] {
        for lua in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let child = directory.path().join("child.tex");
            let sibling = directory.path().join("sibling.tex");
            std::fs::write(&main, "\\input{child}\nParent\n").unwrap();
            std::fs::write(&child, "$\\left a+b\\right)$\n$\\left(a\\right x$\n").unwrap();
            std::fs::write(&sibling, "Sibling\n").unwrap();
            let root = Url::from_file_path(&main).unwrap();
            let child_uri = Url::from_file_path(&child).unwrap();
            let sibling_uri = Url::from_file_path(&sibling).unwrap();
            let mut value = "(./main.tex\n(./child.tex\n".to_owned();
            for (line, token, echo, rest) in [
                (1, "a", r"$\left a", r"+b\right)$"),
                (2, "x", r"$\left(a\right x", "$"),
            ] {
                let header = if numbered {
                    format!("./child.tex:{line}: {ERROR}")
                } else {
                    format!("! {ERROR}")
                };
                let echo = format!("l.{line} {echo}");
                value.push_str(&format!(
                    "{header}\n<to be read again>\n                   {token}\n{echo}\n{}{rest}\n",
                    " ".repeat(echo.len())
                ));
                value.push_str(&if lua {
                    help.replace("proceed.", "proceed")
                } else {
                    help.into()
                });
            }
            value.push_str(")\nLaTeX Warning: Parent sentinel on input line 2.\n(./sibling.tex\nLaTeX Warning: Sibling sentinel on input line 1.\n)\n)\n");
            let items = build_log::parse(&main, &value);
            assert_eq!(items[&child_uri].len(), 2, "{numbered}/{lua}: {items:?}");
            assert!(items[&child_uri]
                .iter()
                .all(|item| item.code == Some(NumberOrString::String(CODE.into()))));
            assert_eq!(items[&root].len(), 1, "{numbered}/{lua}: {items:?}");
            assert!(items[&root][0].message.contains("Parent sentinel"));
            assert_eq!(items[&sibling_uri].len(), 1);
            assert!(items[&sibling_uri][0].message.contains("Sibling sentinel"));
        }
    }
}

#[test]
fn legacy_operand_fixes_reject_tampered_replacements_metadata_and_stale_sources() {
    let source = r"$\left a\right)$";
    let (_directory, mut workspace, root, items) =
        workspace(source, &log(1, r"$\left a", "a", false));
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let actions = actions(&workspace, &root, &items[0]);
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!("{actions:?}")
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let request = || serde_json::from_value::<editing::FixRequest>(value.clone()).unwrap();
    assert!(editing::resolve_fix(&workspace, request()).is_some());
    let mut forged = value.clone();
    forged["replacement"] = "WRONG".into();
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none());
    workspace.open(root.clone(), source.into(), 10);
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    workspace.open(root.clone(), r"$\left b\right)$".into(), 9);
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    workspace.open(root.clone(), source.into(), 9);
    workspace
        .build_diagnostics
        .get_mut(&root)
        .unwrap()
        .get_mut(&root)
        .unwrap()[0]
        .data
        .as_mut()
        .unwrap()["delimiterCommand"] = "WRONG".into();
    assert!(editing::resolve_fix(&workspace, request()).is_none());
}

#[cfg(unix)]
#[test]
fn loaded_operand_parameters_and_primitive_redefinitions_are_rechecked() {
    use latex_lsp::{build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;
    for definition in [
        r"\def\consume{}",
        r"\def\consume#1{#1}",
        r"\def\consume{}\def\left{}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let macros = directory.path().join("macros.tex");
        let source = "\\input{macros}\n$\\consume\\left a\\right)$\n";
        std::fs::write(&main, source).unwrap();
        std::fs::write(&macros, definition).unwrap();
        let compiler = directory.path().join("compiler");
        std::fs::write(&compiler, "#!/bin/sh\ncat <<'LOG' > \"$1/main.log\"\n(./main.tex\n(./macros.tex)\n! Missing delimiter (. inserted).\n<to be read again>\n                   a\nl.2 $\\consume\\left a\n\n)\nLOG\nexit 1\n").unwrap();
        std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = BuildConfig {
            executable: compiler.to_string_lossy().into(),
            arguments: Some(vec!["%o".into(), "%f".into()]),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &Default::default());
        let root = Url::from_file_path(&main).unwrap();
        let item = &outcome.diagnostics[&root][0];
        let precise = definition == r"\def\consume{}";
        assert_eq!(item.code.is_some(), precise, "{definition}: {item:?}");
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(directory.path().into());
        workspace
            .build_diagnostics
            .insert(root.clone(), outcome.diagnostics.clone());
        assert_eq!(!actions(&workspace, &root, item).is_empty(), precise);
        if precise {
            assert!(
                item.data.as_ref().unwrap()["macroSources"]
                    .as_array()
                    .unwrap()
                    .len()
                    >= 2
            );
            std::fs::write(&macros, r"\def\consume#1{#1}").unwrap();
            workspace.reload(&Url::from_file_path(&macros).unwrap());
            assert!(actions(&workspace, &root, item).is_empty());
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX and LuaLaTeX"]
fn real_delimiter_operand_repairs_generate_pdf_and_clear_all_errors() {
    use latex_lsp::{build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;
    let cache = tempfile::tempdir().unwrap();
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let child = directory.path().join("child.tex");
                let root = Url::from_file_path(&main).unwrap();
                let uri = Url::from_file_path(&child).unwrap();
                let source = "$\\left a+b\\right)$\n$\\left(a+b\\right x$\n$\\left(a\\middle b\\right)$\n$\\left\\alpha+b\\right)$\n$\\left(a\\right_1$\n$\\left(a\\right$\n$\\left 1+2\\right)$\n$\\left\\right)$\n$\\left % keep left\n a+b\\right)$\n$\\left(a\\middle% keep middle\nb\\right)$\n$\\left a\\middle b\\right c$\n";
                std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\input{child}\n\\typeout{LaTeX Warning: Parent sentinel on input line 4.}\n\\end{document}\n").unwrap();
                std::fs::write(&child, source).unwrap();
                let wrapper = directory.path().join("engine");
                std::fs::write(&wrapper, format!("#!/bin/sh\nexport max_print_line={width}\nexport TEXMFVAR=\"$1\"\nexport TEXMFCACHE=\"$1\"\nshift\nexec {engine} \"$@\"\n")).unwrap();
                std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
                let mut arguments = vec![
                    cache.path().to_string_lossy().into_owned(),
                    "-interaction=nonstopmode".into(),
                    "-no-shell-escape".into(),
                    "-recorder".into(),
                ];
                if numbered {
                    arguments.push("-file-line-error".into());
                }
                arguments.push("%f".into());
                let config = BuildConfig {
                    executable: wrapper.to_string_lossy().into(),
                    output_directory: Some(".".into()),
                    arguments: Some(arguments),
                    ..Default::default()
                };
                let outcome = build::run(&main, &config, &Default::default());
                assert!(
                    outcome.diagnostics.get(&root).is_some_and(|items| items
                        .iter()
                        .any(|item| item.message.contains("Parent sentinel"))),
                    "{engine}/{width}/{numbered}: {:?}",
                    outcome.diagnostics
                );
                let items = outcome
                    .diagnostics
                    .get(&uri)
                    .into_iter()
                    .flatten()
                    .filter(|item| item.message == ERROR)
                    .collect::<Vec<_>>();
                assert_eq!(
                    items.len(),
                    13,
                    "{engine}/{width}/{numbered}: {:?}\n{}",
                    outcome.diagnostics,
                    outcome.output
                );
                let mut workspace = Workspace {
                    code_action_literals: true,
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.add_root(directory.path().into());
                workspace.open(uri.clone(), source.into(), 9);
                workspace
                    .build_diagnostics
                    .insert(root.clone(), outcome.diagnostics.clone());
                let mut edits = vec![];
                for item in items {
                    assert_eq!(
                        item.code,
                        Some(NumberOrString::String(CODE.into())),
                        "{engine}/{width}/{numbered}: {item:?}"
                    );
                    edits.push(edit(
                        actions(&workspace, &uri, item),
                        &uri,
                        item.data.as_ref().unwrap()["delimiterCommand"]
                            .as_str()
                            .unwrap(),
                    ));
                }
                let repaired = apply(source, edits);
                std::fs::write(&child, &repaired).unwrap();
                assert!(repaired.contains("\\left. % keep left\n a"));
                assert!(repaired.contains("\\middle.% keep middle\nb"));
                let outcome = build::run(&main, &config, &Default::default());
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{width}/{numbered}: {}",
                    outcome.output
                );
                assert_eq!(outcome.pages, Some(1));
                assert!(outcome.pdf.as_ref().unwrap().is_file());
                assert!(
                    outcome
                        .diagnostics
                        .values()
                        .flatten()
                        .all(|item| item.severity != Some(DiagnosticSeverity::ERROR)),
                    "{engine}/{width}/{numbered}: {:?}",
                    outcome.diagnostics
                );
                workspace.open(uri.clone(), repaired, 10);
                workspace
                    .build_diagnostics
                    .insert(root, outcome.diagnostics);
                assert!(editing::code_actions(
                    &workspace,
                    &uri,
                    Range::new(Position::new(0, 0), Position::new(20, 0)),
                    &CodeActionContext {
                        diagnostics: vec![],
                        only: Some(vec![CodeActionKind::QUICKFIX]),
                        trigger_kind: None
                    }
                )
                .is_empty());
            }
        }
    }
}
