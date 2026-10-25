use latex_lsp::{compilation::build_log, editing, project::workspace::Workspace, text::Text};
use lsp_types::*;

const ERROR: &str = r"Extra \right.";
const CODE: &str = "unmatched-right-delimiter";
const MISSING_ERROR: &str = r"Missing \right. inserted.";
const MISSING_CODE: &str = "missing-right-delimiter";

fn missing_log(line: u32, prefix: &str, numbered: bool) -> String {
    let header = if numbered {
        format!("./main.tex:{line}: {MISSING_ERROR}")
    } else {
        format!("! {MISSING_ERROR}")
    };
    format!("(./main.tex\n{header}\n<inserted text>\n                \\right .\nl.{line} {prefix}\n\n)\n")
}

#[test]
fn missing_right_repairs_preserve_math_boundaries_comments_and_utf16() {
    for numbered in [false, true] {
        for newline in ["\n", "\r\n"] {
            for (source, line, prefix, closing) in [
                ("😀 $\\left(a$", 1, "😀 $\\left(a$", "$"),
                (r"$$\left(a$$", 1, r"$$\left(a$", "$"),
                (r"\(\left(a\)", 1, r"\(\left(a\)", r"\)"),
                (r"\[\left(a\]", 1, r"\[\left(a\]", r"\]"),
                (
                    r"$\begingroup\left(a\endgroup$",
                    1,
                    r"$\begingroup\left(a\endgroup",
                    r"\endgroup",
                ),
                (
                    r"\begin{equation}\left(a\end{equation}",
                    1,
                    r"\begin{equation}\left(a\end{equation}",
                    r"\end{equation}",
                ),
                (
                    r"\begin{math}\left(a\end{math}",
                    1,
                    r"\begin{math}\left(a\end{math}",
                    r"\end{math}",
                ),
                (
                    r"\begin{displaymath}\left(a\end{displaymath}",
                    1,
                    r"\begin{displaymath}\left(a\end{displaymath}",
                    r"\end{displaymath}",
                ),
                ("$\\left(a% keep\n$", 2, "$", "$"),
                (r"$\left(a\middle|b$", 1, r"$\left(a\middle|b$", "$"),
                (
                    r"\newcommand{\probe}{\left(a} $\probe$",
                    1,
                    r"\newcommand{\probe}{\left(a} $\probe$",
                    "$",
                ),
            ] {
                let source = source.replace('\n', newline);
                let log = missing_log(line, prefix, numbered).replace('\n', newline);
                let (_directory, workspace, root, items) = workspace(&source, &log);
                assert_eq!(items.len(), 1, "{source}: {items:?}");
                let item = &items[0];
                assert_eq!(
                    item.code,
                    Some(NumberOrString::String(MISSING_CODE.into())),
                    "{source}: {item:?}"
                );
                let end = workspace.documents[&root]
                    .text
                    .offset(Position::new(line - 1, 0))
                    .unwrap()
                    + prefix.len();
                let start = end - closing.len();
                assert_eq!(
                    item.range,
                    workspace.documents[&root].text.range(start, end)
                );
                assert_eq!(item.data.as_ref().unwrap()["closingToken"], closing);
                let edits = edit(actions(&workspace, &root, item), &root);
                assert_eq!(edits.len(), 1);
                assert_eq!(
                    edits[0].range,
                    workspace.documents[&root].text.range(start, start)
                );
                let mut expected = source.clone();
                expected.insert_str(start, r"\right.");
                assert_eq!(apply(&source, edits), expected);
            }
        }
    }
    let source = r"$\left(a$ and $\left[b$";
    let first = missing_log(1, r"$\left(a$", false);
    let second = missing_log(1, source, false);
    let log = first.trim_end_matches(")\n").to_owned() + second.trim_start_matches("(./main.tex\n");
    let (_directory, workspace, root, items) = workspace(source, &log);
    assert_eq!(items.len(), 2);
    assert_ne!(items[0].range, items[1].range);
    let edits = items
        .iter()
        .flat_map(|item| edit(actions(&workspace, &root, item), &root))
        .collect();
    assert_eq!(
        apply(source, edits),
        r"$\left(a\right.$ and $\left[b\right.$"
    );
}

#[test]
fn missing_right_requires_exact_recovery_and_direct_unconsumed_terminators() {
    for (source, prefix, precise) in [
        (r"$\left(a$", r"...\left(a$", true),
        (r"$\left(a$ and $\left(a$", r"...\left(a$", false),
        (r"$\left(a$", r"$\left(b$", false),
        (r"$\left(a$", r"$\left(a", false),
        (r"$\left(a\$", r"$\left(a\$", false),
        (r"$\left(a\\)", r"$\left(a\\)", false),
        (r"% $\left(a$", r"% $\left(a$", false),
        (r"\verb|$\left(a$|", r"\verb|$\left(a$", false),
        (
            r"\newcommand{\probe}{$\left(a$}",
            r"\newcommand{\probe}{$\left(a$",
            false,
        ),
        (
            r"\newcommand{\probe}[1]{#1} $\left(a\probe$",
            r"\newcommand{\probe}[1]{#1} $\left(a\probe$",
            false,
        ),
        (
            r"\newcommand{\probe}[1]{#1} \probe{$\left(a$}",
            r"\newcommand{\probe}[1]{#1} \probe{$\left(a$",
            false,
        ),
        (r"\def\right{} $\left(a$", r"\def\right{} $\left(a$", false),
        (r"$\left(a\string$", r"$\left(a\string$", false),
        (r"$\left(a\meaning$", r"$\left(a\meaning$", false),
        (r"$\left(a\noexpand$", r"$\left(a\noexpand$", false),
        (r"$\left(a\meaning\)", r"$\left(a\meaning\)", false),
        (
            r"\def\endgroup{} $\left(a\endgroup",
            r"\def\endgroup{} $\left(a\endgroup",
            false,
        ),
        (
            r"\def\end{} \begin{math}\left(a\end{math}",
            r"\def\end{} \begin{math}\left(a\end{math}",
            false,
        ),
        (
            r"\renewenvironment{equation}{}{} \begin{equation}\left(a\end{equation}",
            r"\renewenvironment{equation}{}{} \begin{equation}\left(a\end{equation}",
            false,
        ),
        (
            r"\[\left(a\]\end{document}",
            r"\[\left(a\]\end{document}",
            false,
        ),
        (r"$\left(a\end{custom}", r"$\left(a\end{custom}", false),
    ] {
        let (_directory, workspace, root, items) =
            workspace(source, &missing_log(1, prefix, false));
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
        "<inserted text>\n$\n",
        "<inserted text>\n\\right)\n",
        "<inserted text>\n\\right .\n\\probe ->$\n",
        "\\probe ->$\n<inserted text>\n\\right .\n",
    ] {
        let log = format!("(./main.tex\n! {MISSING_ERROR}\n{frame}l.1 $\\left(a$\n\n)\n");
        let (_directory, workspace, root, items) = workspace(r"$\left(a$", &log);
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none(), "{frame}: {:?}", items[0]);
        assert!(actions(&workspace, &root, &items[0]).is_empty());
    }
}

#[test]
fn legacy_missing_right_fixes_reject_stale_source_metadata_and_replacements() {
    let source = r"$\left(a$";
    let (_directory, mut workspace, root, items) =
        workspace(source, &missing_log(1, source, false));
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
    workspace.open(root.clone(), r"$\left(b$".into(), 9);
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
        .unwrap()["closingToken"] = "WRONG".into();
    assert!(editing::resolve_fix(&workspace, request()).is_none());
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

fn edit(actions: CodeActionResponse, uri: &Url) -> Vec<TextEdit> {
    assert_eq!(actions.len(), 1, "{actions:?}");
    let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
        panic!("{actions:?}")
    };
    assert_eq!(
        action.title,
        if action.diagnostics.as_ref().unwrap()[0].code
            == Some(NumberOrString::String("missing-right-delimiter".into()))
        {
            r"Insert invisible '\right.'"
        } else {
            r"Remove unmatched '\right'"
        }
    );
    let Some(DocumentChanges::Edits(documents)) = &action.edit.as_ref().unwrap().document_changes
    else {
        panic!("{action:?}")
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *uri);
    assert_eq!(documents[0].text_document.version, Some(9));
    documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!("{edit:?}"),
        })
        .collect()
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
fn direct_right_delimiters_keep_symbols_comments_utf16_and_individual_error_positions() {
    for numbered in [false, true] {
        for newline in ["\n", "\r\n"] {
            for (body, line, prefix, expected) in [
                ("😀 $a\\right)$", 1, "😀 $a\\right)", "😀 $a)$"),
                (
                    r"$a\left(b\right)\right]$",
                    1,
                    r"$a\left(b\right)\right]",
                    r"$a\left(b\right)]$",
                ),
                (r"$\alpha\right.b$", 1, r"$\alpha\right.", r"$\alpha{}b$"),
                ("$a\\right % keep\n)$", 2, ")", "$a % keep\n)$"),
                ("$\\alpha\\right% keep\n.b$", 2, ".", "$\\alpha{}% keep\nb$"),
                (r"$a\right (b$", 1, r"$a\right (", "$a (b$"),
                (r"$a\right/b$", 1, r"$a\right/", "$a/b$"),
                (r"$a\right|b$", 1, r"$a\right|", "$a|b$"),
            ] {
                let body = body.replace('\n', newline);
                let expected = expected.replace('\n', newline);
                let header = if numbered {
                    format!("./main.tex:{line}: {ERROR}")
                } else {
                    format!("! {ERROR}")
                };
                let log = format!("(./main.tex{newline}{header}{newline}l.{line} {prefix}{newline}                 following source{newline}{newline}){newline}");
                let (_directory, workspace, root, items) = workspace(&body, &log);
                assert_eq!(items.len(), 1, "{body}: {items:?}");
                let item = &items[0];
                assert_eq!(
                    item.code,
                    Some(NumberOrString::String(CODE.into())),
                    "{body}: {item:?}"
                );
                let at = body.rfind(r"\right").unwrap();
                assert_eq!(
                    item.range,
                    workspace.documents[&root].text.range(at, at + 6)
                );
                assert_eq!(item.data.as_ref().unwrap()["sourceLine"], line - 1);
                assert_eq!(
                    apply(&body, edit(actions(&workspace, &root, item), &root)),
                    expected
                );
            }
        }
    }
    let body = r"$a\right) b\right]$";
    let log = "(./main.tex\n! Extra \\right.\nl.1 $a\\right)\n             b\\right]$\n\n! Extra \\right.\nl.1 $a\\right) b\\right]\n                      $\n\n)\n";
    let (_directory, workspace, root, items) = workspace(body, log);
    assert_eq!(items.len(), 2);
    assert_ne!(items[0].range, items[1].range);
    let edits = items
        .iter()
        .flat_map(|item| edit(actions(&workspace, &root, item), &root))
        .collect();
    assert_eq!(apply(body, edits), "$a) b]$");
}

#[test]
fn right_delimiter_attribution_requires_unambiguous_direct_source() {
    for (source, prefix, frame, precise) in [
        (r"$a\right)$", r"$a\right)", "", true),
        (r"$a\right)$", r"...a\right)", "", true),
        (r"$a\right) b\right)$", r"...\right)", "", false),
        (r"$a\right)$", r"$b\right)", "", false),
        (r"$a\right)$", r"$a\right", "", false),
        (
            r"$a\right\rangle$",
            r"$a\right\rangle",
            "\\rangle ->\\delimiter \"526930B\n",
            true,
        ),
        (r"$a\right\rangle$", r"$a\right\rangle", "", false),
        (
            r"$a\right\rangle$",
            r"$a\right\rangle",
            "\\rangle ->\\delimiter \"not-hex\n",
            false,
        ),
        (
            r"$a\right\rangle$",
            r"$a\right\rangle",
            "\\rbrace ->\\delimiter \"5267309\n",
            false,
        ),
        (
            r"$a\right)$",
            r"$a\right)",
            "\\rangle ->\\delimiter \"526930B\n",
            false,
        ),
        (
            r"$a\right\rangle$",
            r"$a\right\rangle",
            "\\rangle ->\\delimiter \"526930B\n\\probe ->\\right \\rangle\n",
            false,
        ),
        (
            r"\def\rangle{} $a\right\rangle$",
            r"\def\rangle{} $a\right\rangle",
            "\\rangle ->\\delimiter \"526930B\n",
            false,
        ),
        (
            r"\newcommand{\probe}{\right)} $a\probe$",
            r"\newcommand{\probe}{\right)} $a\probe",
            "\\probe ->\\right )\n",
            false,
        ),
        (
            r"\newcommand{\probe}[1]{#1} $a\probe{\right)}$",
            r"\newcommand{\probe}[1]{#1} $a\probe{\right)}",
            "<argument> \\right )\n",
            false,
        ),
        (
            r"\newcommand{\probe}[1]{#1} $a\probe\right)$",
            r"\newcommand{\probe}[1]{#1} $a\probe\right)",
            "",
            false,
        ),
        (
            r"\def\right{} $a\right)$",
            r"\def\right{} $a\right)",
            "",
            false,
        ),
        (r"\meaning\right)$", r"\meaning\right)", "", false),
        (
            r"\newcommand{\probe}{$a\right)$}",
            r"\newcommand{\probe}{$a\right)",
            "",
            false,
        ),
        (r"\verb|$a\right)$|", r"\verb|$a\right)", "", false),
        (r"% $a\right)$", r"% $a\right)", "", false),
    ] {
        let log = format!("(./main.tex\n! {ERROR}\n{frame}l.1 {prefix}\n\n)\n");
        let (_directory, workspace, root, items) = workspace(source, &log);
        assert_eq!(items.len(), 1, "{source}: {items:?}");
        assert_eq!(items[0].code.is_some(), precise, "{source}: {:?}", items[0]);
        assert_eq!(
            !actions(&workspace, &root, &items[0]).is_empty(),
            precise,
            "{source}"
        );
    }
    for message in [
        r"Extra \right. Example only.",
        r"Package local Error: Extra \right.",
    ] {
        let (_directory, workspace, root, items) = workspace(
            r"$a\right)$",
            &format!("(./main.tex\n! {message}\nl.1 $a\\right)\n\n)\n"),
        );
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none());
        assert!(actions(&workspace, &root, &items[0]).is_empty());
    }
}

#[test]
fn legacy_right_delimiter_fixes_revalidate_versions_source_and_replacements() {
    let source = r"$\alpha\right.b$";
    let (_directory, mut workspace, root, items) = workspace(
        source,
        "(./main.tex\n! Extra \\right.\nl.1 $\\alpha\\right.\n\n)\n",
    );
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let actions = actions(&workspace, &root, &items[0]);
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!("{actions:?}")
    };
    assert_eq!(command.command, "latex.applyQuickFix");
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let request = || serde_json::from_value::<editing::FixRequest>(value.clone()).unwrap();
    let edit = editing::resolve_fix(&workspace, request()).unwrap();
    let Some(DocumentChanges::Edits(edits)) = edit.document_changes else {
        panic!()
    };
    assert_eq!(edits[0].edits.len(), 2);
    let mut changed = value.clone();
    changed["replacement"] = "WRONG".into();
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(changed).unwrap()).is_none());
    workspace.open(root.clone(), source.into(), 10);
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    workspace.open(root.clone(), r"$\alpha\right)b$".into(), 9);
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    workspace.clear_build_diagnostics(&root);
    assert!(editing::resolve_fix(&workspace, request()).is_none());
}

#[test]
fn math_symbols_do_not_consume_following_right_calls_but_local_redefinitions_can() {
    for symbol in [
        "alpha",
        "beta",
        "varepsilon",
        "varphi",
        "Omega",
        "infty",
        "partial",
        "sum",
        "int",
        "bigcup",
        "sin",
        "lim",
        "log",
        "max",
    ] {
        for (definition, precise) in [
            (String::new(), true),
            (format!("\\def\\{symbol}#1{{#1}}"), false),
        ] {
            let source = format!("{definition}$\\{symbol}\\right)$");
            let prefix = source.trim_end_matches('$');
            let log = format!("(./main.tex\n! {ERROR}\nl.1 {prefix}\n\n)\n");
            let (_directory, workspace, root, items) = workspace(&source, &log);
            assert_eq!(items[0].code.is_some(), precise, "{source}: {:?}", items[0]);
            assert_eq!(!actions(&workspace, &root, &items[0]).is_empty(), precise);
        }
    }
}

#[cfg(unix)]
#[test]
fn loaded_macro_parameters_are_used_and_rechecked_before_right_delimiter_edits() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;
    for definition in [
        r"\def\consume{}",
        r"\def\consume#1{#1}",
        r"\def\consume{}\def\right{}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let macros = directory.path().join("macros.tex");
        let source = "\\input{macros}\n$a\\consume\\right)$\n";
        std::fs::write(&main, source).unwrap();
        std::fs::write(&macros, definition).unwrap();
        let compiler = directory.path().join("compiler");
        std::fs::write(&compiler, "#!/bin/sh\ncat <<'LOG' > \"$1/main.log\"\n(./main.tex\n(./macros.tex)\n! Extra \\right.\nl.2 $a\\consume\\right)\n\n)\nLOG\nexit 1\n").unwrap();
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
        assert_eq!(
            !actions(&workspace, &root, item).is_empty(),
            precise,
            "{definition}: {item:?}"
        );
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
fn missing_right_repairs_recheck_loaded_parameter_and_primitive_definitions() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;
    for definition in [
        r"\def\consume{}",
        r"\def\consume#1{#1}",
        r"\def\consume{}\def\right{}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let macros = directory.path().join("macros.tex");
        let source = "\\input{macros}\n$\\left(a\\consume$\n";
        std::fs::write(&main, source).unwrap();
        std::fs::write(&macros, definition).unwrap();
        let compiler = directory.path().join("compiler");
        std::fs::write(&compiler, "#!/bin/sh\ncat <<'LOG' > \"$1/main.log\"\n(./main.tex\n(./macros.tex)\n! Missing \\right. inserted.\n<inserted text>\n                \\right .\nl.2 $\\left(a\\consume$\n\n)\nLOG\nexit 1\n").unwrap();
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
            std::fs::write(&macros, r"\def\consume{}\def\right{}").unwrap();
            workspace.reload(&Url::from_file_path(&macros).unwrap());
            assert!(actions(&workspace, &root, item).is_empty());
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX and LuaLaTeX"]
fn real_missing_right_repairs_compile_in_all_engines_and_recover_nested_groups() {
    use latex_lsp::{compilation::build, config::BuildConfig};
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
                let source = "$\\left(a$\n$$\\left(a$$\n\\(\\left(a\\)\n\\[\\left(a\\]\n$\\begingroup\\left(a\\endgroup$\n\\begin{equation}\\left(a\\end{equation}\n\\begin{math}\\left(a\\end{math}\n\\begin{displaymath}\\left(a\\end{displaymath}\n$\\left(a% keep\n$\n\\newcommand{\\probe}{\\left(a} $\\probe$\n$\\left(a+\\left[b$\n";
                std::fs::write(&main, "\\documentclass{article}\n\\usepackage{amsmath}\n\\begin{document}\n\\input{child}\n\\end{document}\n").unwrap();
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
                let mut repaired = source.to_owned();
                let mut repairs = 0;
                for round in 0..=12 {
                    let outcome = build::run(&main, &config, &Default::default());
                    let mut workspace = Workspace {
                        code_action_literals: true,
                        versioned_edits: true,
                        ..Default::default()
                    };
                    workspace.add_root(directory.path().into());
                    workspace.open(uri.clone(), repaired.clone(), 9);
                    workspace
                        .build_diagnostics
                        .insert(root.clone(), outcome.diagnostics.clone());
                    let items = outcome
                        .diagnostics
                        .get(&uri)
                        .into_iter()
                        .flatten()
                        .filter(|item| item.message == MISSING_ERROR)
                        .collect::<Vec<_>>();
                    if items.is_empty() {
                        assert_eq!(
                            outcome.status, "success",
                            "{engine}/{width}/{numbered}: {}",
                            outcome.output
                        );
                        assert_eq!(outcome.pages, Some(1));
                        assert!(outcome.pdf.as_ref().unwrap().is_file());
                        assert!(outcome
                            .diagnostics
                            .values()
                            .flatten()
                            .all(|item| item.severity != Some(DiagnosticSeverity::ERROR)));
                        assert!(repaired.contains("% keep\n\\right.$"));
                        assert!(repaired.contains(r"$\left(a+\left[b\right.\right.$"));
                        assert_eq!(repairs, 12, "{engine}/{width}/{numbered}");
                        break;
                    }
                    // Some LuaTeX versions repeat the first recovery until
                    // reaching the error limit. Rebuild after each set of
                    // distinct repairs instead of assuming every error ran.
                    assert!(
                        round < 12 && items.len() <= 11,
                        "{engine}/{width}/{numbered}/{round}: {:?}",
                        outcome.diagnostics
                    );
                    repairs += items.len();
                    let mut edits = vec![];
                    for item in items {
                        assert_eq!(
                            item.code,
                            Some(NumberOrString::String(MISSING_CODE.into())),
                            "{engine}/{width}/{numbered}: {item:?}"
                        );
                        edits.extend(edit(actions(&workspace, &uri, item), &uri));
                    }
                    repaired = apply(&repaired, edits);
                    std::fs::write(&child, &repaired).unwrap();
                }
            }
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX and LuaLaTeX"]
fn real_right_delimiter_fixes_preserve_symbols_and_compile_in_all_engines() {
    use latex_lsp::{compilation::build, config::BuildConfig};
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
                let mut source = "$a\\right)$ and $a\\left(b\\right)\\right]$\n$\\alpha\\right.b$\n$a\\right % keep\n|b$\n$\\alpha\\right% keep dot\n.b$\n".to_owned();
                let mut expected = "$a)$ and $a\\left(b\\right)]$\n$\\alpha{}b$\n$a % keep\n|b$\n$\\alpha{}% keep dot\nb$\n".to_owned();
                let names = [
                    "rangle",
                    "langle",
                    "rbrace",
                    "lbrace",
                    "lfloor",
                    "rfloor",
                    "lceil",
                    "rceil",
                    "vert",
                    "Vert",
                    "backslash",
                    "uparrow",
                    "downarrow",
                    "updownarrow",
                    "Uparrow",
                    "Downarrow",
                    "Updownarrow",
                    "lgroup",
                    "rgroup",
                    "lmoustache",
                    "rmoustache",
                ];
                for name in names {
                    source.push_str(&format!("$\\sin\\right\\{name}$\n"));
                    expected.push_str(&format!("$\\sin\\{name}$\n"));
                }
                std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\input{child}\n\\end{document}\n").unwrap();
                std::fs::write(&child, &source).unwrap();
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
                assert_eq!(
                    outcome.status, "error",
                    "{engine}/{width}/{numbered}: {}",
                    outcome.output
                );
                let items = &outcome.diagnostics[&uri];
                let errors = items
                    .iter()
                    .filter(|item| item.message == ERROR)
                    .collect::<Vec<_>>();
                assert_eq!(
                    errors.len(),
                    5 + names.len(),
                    "{engine}/{width}/{numbered}: {:?}",
                    outcome.diagnostics
                );
                let mut workspace = Workspace {
                    code_action_literals: true,
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.add_root(directory.path().into());
                workspace.open(uri.clone(), source.clone(), 9);
                workspace
                    .build_diagnostics
                    .insert(root.clone(), outcome.diagnostics.clone());
                let mut edits = vec![];
                for item in errors {
                    assert_eq!(
                        item.code,
                        Some(NumberOrString::String(CODE.into())),
                        "{engine}/{width}/{numbered}: {item:?}"
                    );
                    edits.extend(edit(actions(&workspace, &uri, item), &uri));
                }
                let repaired = apply(&source, edits);
                assert_eq!(repaired, expected);
                std::fs::write(&child, &repaired).unwrap();
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
                    Range::new(Position::new(0, 0), Position::new(9, 0)),
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
