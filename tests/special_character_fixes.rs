use latex_lsp::{
    build_log, editing,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;
use serde_json::json;

const AMP: &str = "Misplaced alignment tab character &.";
const HASH: &str = "You can't use `macro parameter character #' in horizontal mode.";
fn context(diagnostics: Vec<Diagnostic>) -> CodeActionContext {
    CodeActionContext {
        diagnostics,
        only: None,
        trigger_kind: None,
    }
}
fn setup(source: &str, log: &str) -> (tempfile::TempDir, Workspace, Url, Vec<Diagnostic>) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), source).unwrap();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let parsed = build_log::parse(&root.join("main.tex"), log);
    let diagnostics = parsed.get(&uri).cloned().unwrap_or_default();
    let mut workspace = Workspace {
        code_action_literals: true,
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(uri.clone(), source.into(), 9);
    workspace.build_diagnostics.insert(uri.clone(), parsed);
    (directory, workspace, uri, diagnostics)
}
fn edits(actions: Vec<CodeActionOrCommand>, uri: &Url) -> Vec<TextEdit> {
    actions
        .into_iter()
        .map(|action| {
            let CodeActionOrCommand::CodeAction(action) = action else {
                panic!("expected literal action")
            };
            assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
            let Some(DocumentChanges::Edits(mut documents)) = action.edit.unwrap().document_changes
            else {
                panic!("expected versioned edit")
            };
            assert_eq!(documents.len(), 1);
            let document = documents.remove(0);
            assert_eq!(document.text_document.uri, *uri);
            assert_eq!(document.text_document.version, Some(9));
            assert_eq!(document.edits.len(), 1);
            let OneOf::Left(edit) = document.edits.into_iter().next().unwrap() else {
                panic!()
            };
            edit
        })
        .collect()
}

#[test]
fn direct_special_character_errors_keep_distinct_utf16_ranges_and_escape_actions() {
    let source = "😀 Before & and & after.\r\nBefore # after.\r\n";
    let text = Text::new(source.into());
    for numbered in [false, true] {
        let mut log = "(./main.tex\n".to_string();
        for (line, message, prefix) in [
            (1, AMP, "😀 Before &"),
            (1, AMP, "😀 Before & and &"),
            (2, HASH, "Before #"),
        ] {
            log.push_str(&format!(
                "{}\nl.{line} {prefix}\n                 following source\n\n",
                if numbered {
                    format!("./main.tex:{line}: {message}")
                } else {
                    format!("! {message}")
                }
            ));
        }
        log.push_str(")\n");
        let (_directory, workspace, uri, items) = setup(source, &log);
        assert_eq!(items.len(), 3, "{items:?}");
        let mut ranges: Vec<_> = items.iter().map(|item| item.range).collect();
        ranges.sort_by_key(|range| range.start);
        let expected: Vec<_> = source
            .char_indices()
            .filter(|(_, ch)| matches!(ch, '&' | '#'))
            .map(|(at, _)| text.range(at, at + 1))
            .collect();
        assert_eq!(ranges, expected);
        for item in &items {
            assert_eq!(
                item.code,
                Some(NumberOrString::String("unescaped-special-character".into()))
            );
            assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR));
            let start = text.offset(item.range.start).unwrap();
            let character = &source[start..start + 1];
            assert_eq!(item.data.as_ref().unwrap()["character"], character);
            let fixes = edits(
                editing::code_actions(&workspace, &uri, item.range, &context(vec![item.clone()])),
                &uri,
            );
            assert_eq!(fixes.len(), 1);
            assert_eq!(fixes[0].range, item.range);
            assert_eq!(fixes[0].new_text, format!("\\{character}"));
        }
    }
}

#[test]
fn shortened_special_character_context_must_identify_one_complete_source_suffix() {
    for (source, prefix, precise) in [
        (
            format!("{} unique & tail", "prefix ".repeat(30)),
            "...unique &",
            true,
        ),
        ("unique & middle unique &".into(), "...unique &", false),
        ("prefix & changed".into(), "Wrong &", false),
        (r"prefix \& escaped".into(), r"prefix \&", false),
        (r"prefix \\& literal".into(), r"prefix \\&", true),
        ("% prefix & comment".into(), "% prefix &", false),
        (r"\verb|prefix &|".into(), r"\verb|prefix &", false),
        (r"\def\bad{prefix &}".into(), r"\def\bad{prefix &", false),
    ] {
        let log = format!("(./main.tex\n! {AMP}\nl.1 {prefix}\n\n)\n");
        let (_directory, workspace, uri, items) = setup(&source, &log);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].code.is_some(), precise, "{source}: {:?}", items[0]);
        assert_eq!(
            !editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()))
                .is_empty(),
            precise,
            "{source}"
        );
    }
}

#[test]
fn macro_expansions_unknown_sources_and_invalid_lines_do_not_offer_literal_escapes() {
    let source = "Before & and # after.\n";
    for (location, frame) in [
        ("./main.tex:1", "\\bad ->Before &\n"),
        ("./main.tex:1", "<argument> Before &\n"),
        ("./gone.tex:1", ""),
        ("./main.tex:0", ""),
        ("./main.tex:90", ""),
    ] {
        let line = location.rsplit(':').next().unwrap();
        let log = format!("(./main.tex\n{location}: {AMP}\n{frame}l.{line} Before &\n\n)\n");
        let (_directory, workspace, uri, items) = setup(source, &log);
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none(), "{log}: {items:?}");
        assert!(
            editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()))
                .is_empty()
        );
    }
}

#[test]
fn legacy_escape_commands_recheck_versions_context_and_current_diagnostics() {
    let source = "Before & after.\n";
    let log = format!("(./main.tex\n! {AMP}\nl.1 Before &\n)\n");
    let (_directory, mut workspace, uri, items) = setup(source, &log);
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let actions = editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()));
    assert_eq!(actions.len(), 1);
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!()
    };
    assert_eq!(command.command, "latex.applyQuickFix");
    let arguments = command.arguments.as_ref().unwrap()[0].clone();
    let request = || serde_json::from_value::<editing::FixRequest>(arguments.clone()).unwrap();
    assert!(editing::resolve_fix(&workspace, request()).is_some());
    let mut malformed = arguments.clone();
    malformed["replacement"] = json!("injected");
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(malformed).unwrap()).is_none());
    workspace.documents.insert(
        uri.clone(),
        Document::new(&uri, "Changed & after.\n".into(), Some(9)),
    );
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(10)));
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(9)));
    workspace.build_diagnostics.clear();
    assert!(editing::resolve_fix(&workspace, request()).is_none());
}

#[test]
fn special_character_actions_filter_requests_deduplicate_roots_and_recheck_data() {
    let source = "Before & after.\n";
    let log = format!("(./main.tex\n! {AMP}\nl.1 Before &\n)\n");
    let (_directory, mut workspace, uri, items) = setup(source, &log);
    let other = uri.join("other.tex").unwrap();
    workspace
        .build_diagnostics
        .insert(other, workspace.build_diagnostics[&uri].clone());
    let range = items[0].range;
    assert_eq!(
        editing::code_actions(&workspace, &uri, range, &context(vec![])).len(),
        1
    );
    let mut request = context(items.clone());
    request.only = Some(vec![CodeActionKind::REFACTOR]);
    assert!(editing::code_actions(&workspace, &uri, range, &request).is_empty());
    request = context(items.clone());
    request.diagnostics[0].message = "unrelated".into();
    assert!(editing::code_actions(&workspace, &uri, range, &request).is_empty());
    for diagnostics in workspace.build_diagnostics.values_mut() {
        diagnostics.get_mut(&uri).unwrap()[0].data =
            Some(json!({"character":"#","sourceContext":"Before &"}));
    }
    assert!(editing::code_actions(&workspace, &uri, range, &context(vec![])).is_empty());
}

#[test]
fn missing_math_shift_errors_locate_literal_scripts_and_offer_explicit_text_escapes() {
    for numbered in [false, true] {
        for (character, replacement) in [('_', r"\_"), ('^', r"\textasciicircum{}")] {
            let source = format!("😀 Before foo{character}bar.\r\n\\par\r\n");
            let prefix = format!("😀 Before foo{character}");
            let header = if numbered { "./main.tex:1:" } else { "!" };
            let log = format!("(./main.tex\n{header} Missing $ inserted.\n<inserted text> \n                $\nl.1 {prefix}\n                bar.\n\n! Missing $ inserted.\n<inserted text>\n                $\nl.2 \\par\n\n)\n");
            let (_directory, workspace, uri, items) = setup(&source, &log);
            assert_eq!(items.len(), 2, "{items:?}");
            let precise: Vec<_> = items.iter().filter(|item| item.code.is_some()).collect();
            assert_eq!(precise.len(), 1, "{items:?}");
            let item = precise[0];
            let at = source.find(character).unwrap();
            assert_eq!(item.range, Text::new(source.clone()).range(at, at + 1));
            assert_eq!(
                item.code,
                Some(NumberOrString::String("unescaped-special-character".into()))
            );
            let actions =
                editing::code_actions(&workspace, &uri, item.range, &context(vec![item.clone()]));
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!()
            };
            assert_eq!(
                action.title,
                format!("Escape '{character}' as '{replacement}'")
            );
            let fixes = edits(actions, &uri);
            assert_eq!(fixes.len(), 1);
            assert_eq!(fixes[0].new_text, replacement);
        }
    }
}

#[test]
fn missing_math_shift_requires_exact_insertion_frame_and_unambiguous_literal_source() {
    for (source, prefix, frame, precise) in [
        (
            "Before _ after.",
            "Before _",
            "<inserted text>\n                $\n",
            true,
        ),
        ("Before _ after.", "Before _", "", false),
        (
            "Before _ after.",
            "Before _",
            "<inserted text>\n                }\n",
            false,
        ),
        (
            "Before _ after.",
            "Before _",
            "<inserted text>\n                $\n\\bad ->Before _\n",
            false,
        ),
        (
            "Before _ after.",
            "Before _",
            "<argument> _\n<inserted text>\n                $\n",
            false,
        ),
        (
            "Before _ after.",
            "Wrong _",
            "<inserted text>\n                $\n",
            false,
        ),
        (
            r"Before \_ after.",
            r"Before \_",
            "<inserted text>\n                $\n",
            false,
        ),
        (
            r"\verb|Before _|",
            r"\verb|Before _",
            "<inserted text>\n                $\n",
            false,
        ),
        (
            r"\def\bad{Before _}",
            r"\def\bad{Before _",
            "<inserted text>\n                $\n",
            false,
        ),
        (
            "% Before _",
            "% Before _",
            "<inserted text>\n                $\n",
            false,
        ),
        (
            "Before _ middle Before _",
            "...Before _",
            "<inserted text>\n                $\n",
            false,
        ),
        (
            "Long prefix unique _ after.",
            "...unique _",
            "<inserted text>\n                $\n",
            true,
        ),
        (
            "Before $ after.",
            "Before $",
            "<inserted text>\n                $\n",
            false,
        ),
        (
            "Before _ after.",
            "Before",
            "<inserted text>\n                $\n",
            false,
        ),
    ] {
        let log = format!("(./main.tex\n! Missing $ inserted.\n{frame}l.1 {prefix}\n\n)\n");
        let (_directory, workspace, uri, items) = setup(source, &log);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].code.is_some(), precise, "{source}: {items:?}");
        assert_eq!(
            !editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()))
                .is_empty(),
            precise,
            "{log}"
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_special_character_fixes_repair_child_sources_without_changing_valid_tex() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let child = root.join("child.tex");
            let compiler = root.join(engine);
            std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: compiler.to_string_lossy().into(),
                ..Default::default()
            };
            std::fs::write(
                &main,
                "\\documentclass{article}\n\\begin{document}\n\\input{child}\n\\end{document}\n",
            )
            .unwrap();
            let source = concat!(
                "#\\par\n",
                "Before & and & after.\n",
                "Before # after.\n",
                "$Math # after.$\n",
                "\\hbox{#}\n",
                "\\vbox{#}\n",
                "Text foo_bar.\\par\n",
                "Text x^2.\\par\n",
                "Text prefix prefix prefix prefix prefix prefix prefix unique_word.\\par\n",
                "Escaped \\& and \\#.\n",
                "Escaped \\_ and \\textasciicircum{}. $x_1^2$\n",
                "\\begin{tabular}{ll}A & B\\end{tabular}\n",
                "% Comment & #\n",
                "\\verb|Verbatim & #|\n",
            );
            std::fs::write(&child, source).unwrap();
            let outcome = latex_lsp::build::run(&main, &config, &Default::default());
            assert_eq!(
                outcome.status, "error",
                "{engine}/{width}: {}",
                outcome.output
            );
            let uri = Url::from_file_path(&child).unwrap();
            let items: Vec<_> = outcome.diagnostics[&uri]
                .iter()
                .filter(|item| {
                    item.code == Some(NumberOrString::String("unescaped-special-character".into()))
                })
                .cloned()
                .collect();
            assert_eq!(
                items.len(),
                10,
                "{engine}/{width}: {:?}\n{}",
                outcome.diagnostics,
                outcome.output
            );
            let mut workspace = Workspace {
                code_action_literals: true,
                versioned_edits: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(uri.clone(), source.into(), 9);
            workspace
                .build_diagnostics
                .insert(Url::from_file_path(&main).unwrap(), outcome.diagnostics);
            let mut fixes = Vec::new();
            for item in items {
                let actions =
                    editing::code_actions(&workspace, &uri, item.range, &context(vec![item]));
                assert_eq!(actions.len(), 1, "{engine}/{width}");
                fixes.extend(edits(actions, &uri));
            }
            fixes.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let text = Text::new(source.into());
            let mut fixed = source.to_string();
            for edit in fixes {
                fixed.replace_range(
                    text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap(),
                    &edit.new_text,
                );
            }
            assert!(fixed.ends_with("Escaped \\& and \\#.\nEscaped \\_ and \\textasciicircum{}. $x_1^2$\n\\begin{tabular}{ll}A & B\\end{tabular}\n% Comment & #\n\\verb|Verbatim & #|\n"));
            std::fs::write(&child, fixed).unwrap();
            let repaired = latex_lsp::build::run(&main, &config, &Default::default());
            assert_eq!(
                repaired.status, "success",
                "{engine}/{width}: {}",
                repaired.output
            );
            assert!(
                repaired
                    .diagnostics
                    .values()
                    .flatten()
                    .all(|item| item.severity != Some(DiagnosticSeverity::ERROR)),
                "{engine}/{width}: {:?}",
                repaired.diagnostics
            );
        }
    }
}

const EXTRA_ALIGNMENT: &str = r"Extra alignment tab has been changed to \cr.";

#[test]
fn extra_alignment_tabs_keep_distinct_utf16_ranges_and_offer_versioned_text_escapes() {
    let source="😀 \\begin{tabular}{ll}A & B & C\\\\\\end{tabular} \\begin{tabular}{ll}D & E & F\\\\\\end{tabular}\r\n";
    let text = Text::new(source.into());
    for numbered in [false, true] {
        for frame in 0..3 {
            let mut log = "(./main.tex\n".to_string();
            let expected: Vec<_> = source
                .char_indices()
                .filter(|(_, ch)| *ch == '&')
                .enumerate()
                .filter(|(index, _)| index % 2 == 1)
                .map(|(_, pair)| pair)
                .collect();
            for (at, _) in &expected {
                let header = if numbered { "./main.tex:1:" } else { "!" };
                let extra = if frame != 0 {
                    "<template> ...fil }\\hskip \\tabcolsep \\endtemplate\n                      \n"
                } else {
                    ""
                };
                let recent = if frame != 2 {
                    "<recently read> \\endtemplate\n                           \n"
                } else {
                    ""
                };
                log.push_str(&format!("{header} {EXTRA_ALIGNMENT}\n{recent}{extra}l.1 {}\n                  following source\n\n",&source[..at+1]));
            }
            log.push_str(")\n");
            let (_directory, workspace, uri, items) = setup(source, &log);
            assert_eq!(items.len(), 2, "{items:?}");
            for (item, (at, _)) in items.iter().zip(expected) {
                assert_eq!(
                    item.code,
                    Some(NumberOrString::String("extra-alignment-tab".into()))
                );
                assert_eq!(item.range, text.range(at, at + 1));
                let actions = editing::code_actions(
                    &workspace,
                    &uri,
                    item.range,
                    &context(vec![item.clone()]),
                );
                let fixes = edits(actions, &uri);
                assert_eq!(fixes.len(), 1);
                assert_eq!(fixes[0].range, item.range);
                assert_eq!(fixes[0].new_text, r"\&");
            }
        }
    }
}

#[test]
fn extra_alignment_tab_fixes_require_the_exact_internal_frame_and_literal_source() {
    for (source, prefix, frame) in [
        ("Before & after.", "Before &", ""),
        ("Before & after.", "Before &", "<recently read> \\unknown\n"),
        (
            "Before & after.",
            "Before &",
            "<recently read> \\endtemplate\n\\bad ->Before &\n",
        ),
        (
            "Before & after.",
            "Before &",
            "<recently read> \\endtemplate\n<template> incomplete\n",
        ),
        (
            "Before & middle Before &",
            "...Before &",
            "<recently read> \\endtemplate\n",
        ),
        (
            r"Before \& escaped",
            r"Before \&",
            "<recently read> \\endtemplate\n",
        ),
        (
            "Before & after.",
            "Different &",
            "<recently read> \\endtemplate\n",
        ),
    ] {
        let log = format!("(./main.tex\n! {EXTRA_ALIGNMENT}\n{frame}l.1 {prefix}\n\n)\n");
        let (_directory, workspace, uri, items) = setup(source, &log);
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none(), "{log}: {items:?}");
        assert!(
            editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()))
                .is_empty()
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_extra_alignment_tabs_repair_tables_arrays_and_raw_alignments() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for verbose in [false, true] {
                for numbered in [false, true] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path();
                    let main = root.join("main.tex");
                    let child = root.join("child.tex");
                    let compiler = root.join(engine);
                    std::fs::write(&compiler,format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    let mut arguments = vec![
                        "-interaction=nonstopmode".into(),
                        "-recorder".into(),
                        "-output-directory=%o".into(),
                        "%f".into(),
                    ];
                    if numbered {
                        arguments.insert(0, "-file-line-error".into());
                    }
                    let config = latex_lsp::config::BuildConfig {
                        executable: compiler.to_string_lossy().into(),
                        arguments: Some(arguments),
                        ..Default::default()
                    };
                    std::fs::write(&main,format!("\\documentclass{{article}}\n{}\\begin{{document}}\n\\input{{child}}\n\\end{{document}}\n",if verbose {"\\errorcontextlines=100\n"}else{""})).unwrap();
                    let source = concat!(
                        "\\begin{tabular}{ll}A & B & C\\\\\\end{tabular}\n",
                        "$\\begin{array}{cc}A & B & C\\\\\\end{array}$\n",
                        "\\halign{#\\hfil & #\\hfil\\cr A & B & C\\cr}\n",
                        "\\begin{tabular}{ll}Legal & Escaped \\&\\\\\\end{tabular}\n",
                    );
                    std::fs::write(&child, source).unwrap();
                    let outcome = latex_lsp::build::run(&main, &config, &Default::default());
                    assert_eq!(
                        outcome.status, "error",
                        "{engine}/{width}/{verbose}/{numbered}: {}",
                        outcome.output
                    );
                    let uri = Url::from_file_path(&child).unwrap();
                    let items: Vec<_> = outcome.diagnostics[&uri]
                        .iter()
                        .filter(|item| {
                            item.code == Some(NumberOrString::String("extra-alignment-tab".into()))
                        })
                        .cloned()
                        .collect();
                    assert_eq!(
                        items.len(),
                        3,
                        "{engine}/{width}/{verbose}/{numbered}: {:?}\n{}",
                        outcome.diagnostics,
                        outcome.output
                    );
                    let mut workspace = Workspace {
                        code_action_literals: true,
                        versioned_edits: true,
                        ..Default::default()
                    };
                    workspace.add_root(root.into());
                    workspace.open(uri.clone(), source.into(), 9);
                    workspace
                        .build_diagnostics
                        .insert(Url::from_file_path(&main).unwrap(), outcome.diagnostics);
                    let mut fixes = Vec::new();
                    for item in items {
                        let actions = editing::code_actions(
                            &workspace,
                            &uri,
                            item.range,
                            &context(vec![item]),
                        );
                        assert_eq!(actions.len(), 1);
                        fixes.extend(edits(actions, &uri));
                    }
                    fixes.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
                    let text = Text::new(source.into());
                    let mut fixed = source.to_string();
                    for edit in fixes {
                        fixed.replace_range(
                            text.offset(edit.range.start).unwrap()
                                ..text.offset(edit.range.end).unwrap(),
                            &edit.new_text,
                        );
                    }
                    assert!(fixed
                        .ends_with("\\begin{tabular}{ll}Legal & Escaped \\&\\\\\\end{tabular}\n"));
                    std::fs::write(&child, fixed).unwrap();
                    let repaired = latex_lsp::build::run(&main, &config, &Default::default());
                    assert_eq!(
                        repaired.status, "success",
                        "{engine}/{width}/{verbose}/{numbered}: {}",
                        repaired.output
                    );
                    assert!(repaired
                        .diagnostics
                        .values()
                        .flatten()
                        .all(|item| item.severity != Some(DiagnosticSeverity::ERROR)));
                    std::fs::write(
                        &child,
                        r"\def\bad#1#2{&}\begin{tabular}{ll}A & B\bad x& C\\\end{tabular}",
                    )
                    .unwrap();
                    let macro_error = latex_lsp::build::run(&main, &config, &Default::default());
                    assert_eq!(macro_error.status, "error");
                    assert!(
                        macro_error
                            .diagnostics
                            .values()
                            .flatten()
                            .all(|item| item.code
                                != Some(NumberOrString::String("extra-alignment-tab".into()))),
                        "{engine}/{width}/{verbose}/{numbered}: {:?}",
                        macro_error.diagnostics
                    );
                }
            }
        }
    }
}

#[test]
fn extra_alignment_tabs_consumed_by_macro_parameters_are_not_treated_as_row_separators() {
    for source in [
        r"\def\bad#1{&}\begin{tabular}{ll}A & B\bad&",
        r"\def\bad#1#2{&}\begin{tabular}{ll}A & B\bad x&",
        r"\def\bad#1&{&}\begin{tabular}{ll}A & B\bad x&",
        r"\begin{tabular}{ll}A & B\unknown&",
    ] {
        let log=format!("(./main.tex\n! {EXTRA_ALIGNMENT}\n<recently read> \\endtemplate\n\nl.1 {source}\n\n)\n");
        let (_directory, workspace, uri, items) = setup(source, &log);
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none(), "{source}: {items:?}");
        assert!(
            editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()))
                .is_empty()
        );
    }
    let source = r"\begin{tabular}{ll}A & B\hfil&";
    let log = format!(
        "(./main.tex\n! {EXTRA_ALIGNMENT}\n<recently read> \\endtemplate\n\nl.1 {source}\n\n)\n"
    );
    let (_directory, workspace, uri, items) = setup(source, &log);
    assert_eq!(
        items[0].code,
        Some(NumberOrString::String("extra-alignment-tab".into()))
    );
    assert_eq!(
        editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone())).len(),
        1
    );
}

const EXTRA_BRACE: &str = "Too many }'s.";
const BRACE_CODE: &str = "extra-closing-brace";

#[test]
fn extra_closing_braces_keep_utf16_occurrences_and_offer_versioned_deletions() {
    let source = "😀 Before } and } after.\r\n$a}b$\r\n\\begingroup text }\\endgroup\r\n$\\left(a}b\\right)$\r\n";
    let text = Text::new(source.into());
    for numbered in [false, true] {
        let mut log = "(./main.tex\n".to_string();
        for (line, message, prefix) in [
            (1, EXTRA_BRACE, "😀 Before }"),
            (1, EXTRA_BRACE, "😀 Before } and }"),
            (2, "Extra }, or forgotten $.", "$a}"),
            (
                3,
                r"Extra }, or forgotten \endgroup.",
                r"\begingroup text }",
            ),
            (4, r"Extra }, or forgotten \right.", r"$\left(a}"),
        ] {
            let header = if numbered {
                format!("./main.tex:{line}:")
            } else {
                "!".into()
            };
            log.push_str(&format!(
                "{header} {message}\nl.{line} {prefix}\n                 following source\n\n"
            ));
        }
        log.push_str(")\n");
        let (_directory, workspace, uri, items) = setup(source, &log);
        let expected: Vec<_> = source
            .match_indices('}')
            .map(|(at, _)| text.range(at, at + 1))
            .collect();
        assert_eq!(items.len(), expected.len(), "{items:?}");
        for (item, expected) in items.iter().zip(expected) {
            assert_eq!(item.code, Some(NumberOrString::String(BRACE_CODE.into())));
            assert_eq!(item.range, expected);
            assert_eq!(item.data.as_ref().unwrap()["character"], "}");
            let actions =
                editing::code_actions(&workspace, &uri, item.range, &context(vec![item.clone()]));
            assert_eq!(actions.len(), 1);
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!()
            };
            assert_eq!(action.title, "Remove extra '}'");
            let changes = edits(actions, &uri);
            assert_eq!(changes[0].range, expected);
            assert_eq!(changes[0].new_text, "");
        }
    }
}

#[test]
fn extra_closing_braces_require_direct_unambiguous_literal_source() {
    for (source, prefix, frame, location, precise) in [
        ("Before } after.", "Before }", "", "./main.tex:1", true),
        (
            "Long prefix unique } after.",
            "...unique }",
            "",
            "./main.tex:1",
            true,
        ),
        (
            "Before } and Before }",
            "...Before }",
            "",
            "./main.tex:1",
            false,
        ),
        ("Before } after.", "Wrong }", "", "./main.tex:1", false),
        (r"Before \} after.", r"Before \}", "", "./main.tex:1", false),
        (
            r"Before \\} after.",
            r"Before \\}",
            "",
            "./main.tex:1",
            true,
        ),
        ("% Before }", "% Before }", "", "./main.tex:1", false),
        (
            r"\verb|Before }|",
            r"\verb|Before }",
            "",
            "./main.tex:1",
            false,
        ),
        (
            r"\def\bad{{Before } inside}",
            r"\def\bad{{Before }",
            "",
            "./main.tex:1",
            false,
        ),
        (
            "Before } after.",
            "Before }",
            "\\bad ->Before }\n",
            "./main.tex:1",
            false,
        ),
        (
            "Before } after.",
            "Before }",
            "<argument> Before }\n",
            "./main.tex:1",
            false,
        ),
        (
            "Before } after.",
            "Before }",
            "<recently read> }\n",
            "./main.tex:1",
            false,
        ),
        ("Before } after.", "Before }", "", "./gone.tex:1", false),
        ("Before } after.", "Before }", "", "./main.tex:0", false),
        ("Before } after.", "Before }", "", "./main.tex:99", false),
    ] {
        let number = location.rsplit(':').next().unwrap();
        let log =
            format!("(./main.tex\n{location}: {EXTRA_BRACE}\n{frame}l.{number} {prefix}\n\n)\n");
        let (_directory, workspace, uri, items) = setup(source, &log);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].code.is_some(), precise, "{log}: {items:?}");
        assert_eq!(
            !editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()))
                .is_empty(),
            precise,
            "{log}"
        );
    }
    for message in [
        "Missing } inserted.",
        "Extra }, or forgotten something.",
        "Too many }'s. additional text",
    ] {
        let (_directory, workspace, uri, items) = setup(
            "Before } after.",
            &format!("(./main.tex\n! {message}\nl.1 Before }}\n)\n"),
        );
        assert_eq!(items.len(), 1);
        assert!(items[0].code.is_none());
        assert!(
            editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()))
                .is_empty()
        );
    }
}

#[test]
fn legacy_extra_closing_brace_deletions_recheck_diagnostics_source_and_version() {
    let source = "Before } after.\n";
    let log = format!("(./main.tex\n! {EXTRA_BRACE}\nl.1 Before }}\n)\n");
    let (_directory, mut workspace, uri, items) = setup(source, &log);
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let actions = editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()));
    assert_eq!(actions.len(), 1);
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!()
    };
    let argument = command.arguments.as_ref().unwrap()[0].clone();
    let request = || serde_json::from_value::<editing::FixRequest>(argument.clone()).unwrap();
    assert!(editing::resolve_fix(&workspace, request()).is_some());
    let mut forged = argument.clone();
    forged["replacement"] = json!("\\}");
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none());
    workspace.documents.insert(
        uri.clone(),
        Document::new(&uri, "Changed } after.\n".into(), Some(9)),
    );
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(10)));
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(9)));
    workspace.build_diagnostics.clear();
    assert!(editing::resolve_fix(&workspace, request()).is_none());
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_extra_closing_brace_deletions_repair_text_math_and_explicit_groups() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                for verbose in [false, true] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path();
                    let main = root.join("main.tex");
                    let child = root.join("child.tex");
                    let compiler = root.join(engine);
                    std::fs::write(&compiler,format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    let mut arguments = vec![
                        "-interaction=nonstopmode".into(),
                        "-recorder".into(),
                        "-output-directory=%o".into(),
                        "%f".into(),
                    ];
                    if numbered {
                        arguments.insert(0, "-file-line-error".into());
                    }
                    let config = latex_lsp::config::BuildConfig {
                        executable: compiler.to_string_lossy().into(),
                        arguments: Some(arguments),
                        ..Default::default()
                    };
                    std::fs::write(&main,format!("\\documentclass{{article}}\n{}\\begin{{document}}\n\\input{{child}}\n\\end{{document}}\n",if verbose {"\\errorcontextlines=100\n"}else{""})).unwrap();
                    let legal = concat!(
                        "{Legal {nested} group} and escaped \\}.\n",
                        "\\verb|literal }| % comment }\n",
                        "\\def\\balanced{nested {group}}\\balanced\n",
                    );
                    let source = format!("{{Valid}} {}First extra}} then Second extra}} tail.\n$a extra}} b$\n\\begingroup text extra}}\\endgroup\n$\\left(a extra}} b\\right)$\n{legal}","long prefix ".repeat(20));
                    std::fs::write(&child, &source).unwrap();
                    let outcome = latex_lsp::build::run(&main, &config, &Default::default());
                    assert_eq!(
                        outcome.status, "error",
                        "{engine}/{width}/{numbered}/{verbose}: {}",
                        outcome.output
                    );
                    let uri = Url::from_file_path(&child).unwrap();
                    let items: Vec<_> = outcome.diagnostics[&uri]
                        .iter()
                        .filter(|item| item.code == Some(NumberOrString::String(BRACE_CODE.into())))
                        .cloned()
                        .collect();
                    assert_eq!(
                        items.len(),
                        5,
                        "{engine}/{width}/{numbered}/{verbose}: {:?}\n{}",
                        outcome.diagnostics,
                        outcome.output
                    );
                    let text = Text::new(source.clone());
                    let expected: Vec<_> = source
                        .match_indices("extra}")
                        .map(|(at, _)| text.range(at + 5, at + 6))
                        .collect();
                    assert_eq!(
                        items.iter().map(|item| item.range).collect::<Vec<_>>(),
                        expected
                    );
                    let mut workspace = Workspace {
                        code_action_literals: true,
                        versioned_edits: true,
                        ..Default::default()
                    };
                    workspace.add_root(root.into());
                    workspace.open(uri.clone(), source.clone(), 9);
                    workspace
                        .build_diagnostics
                        .insert(Url::from_file_path(&main).unwrap(), outcome.diagnostics);
                    let mut changes = Vec::new();
                    for item in items {
                        let actions = editing::code_actions(
                            &workspace,
                            &uri,
                            item.range,
                            &context(vec![item]),
                        );
                        assert_eq!(actions.len(), 1);
                        changes.extend(edits(actions, &uri));
                    }
                    changes.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
                    let mut fixed = source.clone();
                    for edit in changes {
                        assert!(edit.new_text.is_empty());
                        fixed.replace_range(
                            text.offset(edit.range.start).unwrap()
                                ..text.offset(edit.range.end).unwrap(),
                            &edit.new_text,
                        );
                    }
                    assert!(fixed.ends_with(legal));
                    std::fs::write(&child, &fixed).unwrap();
                    let repaired = latex_lsp::build::run(&main, &config, &Default::default());
                    assert_eq!(
                        repaired.status, "success",
                        "{engine}/{width}/{numbered}/{verbose}: {}",
                        repaired.output
                    );
                    assert!(repaired
                        .diagnostics
                        .values()
                        .flatten()
                        .all(|item| item.severity != Some(DiagnosticSeverity::ERROR)));
                    std::fs::write(&child, r"\def\bad{\egroup}\bad").unwrap();
                    let macro_error = latex_lsp::build::run(&main, &config, &Default::default());
                    assert_eq!(macro_error.status, "error");
                    assert!(
                        macro_error.diagnostics.values().flatten().all(
                            |item| item.code != Some(NumberOrString::String(BRACE_CODE.into()))
                        ),
                        "{engine}/{width}/{numbered}/{verbose}: {:?}",
                        macro_error.diagnostics
                    );
                }
            }
        }
    }
}

#[test]
fn double_scripts_keep_distinct_utf16_ranges_and_offer_explicit_empty_bases() {
    let source = "😀 $x_1_2$ and $y^1^2$ and $z_{ab}_{cd}$ and $w^{ab}^{cd}$\r\n";
    let text = Text::new(source.into());
    for numbered in [false, true] {
        let mut log = "(./main.tex\n".to_string();
        let expected: Vec<_> = [
            ("_2", '_', "Double subscript.", "double-subscript"),
            ("^2", '^', "Double superscript.", "double-superscript"),
            ("_{cd}", '_', "Double subscript.", "double-subscript"),
            ("^{cd}", '^', "Double superscript.", "double-superscript"),
        ]
        .into_iter()
        .map(|(suffix, ch, message, code)| {
            let at = source.find(suffix).unwrap();
            let header = if numbered { "./main.tex:1:" } else { "!" };
            log.push_str(&format!(
                "{header} {message}\nl.1 {}\n                 following source\n\n",
                &source[..at + 1]
            ));
            (at, ch, code)
        })
        .collect();
        log.push_str(")\n");
        let (_directory, workspace, uri, items) = setup(source, &log);
        assert_eq!(items.len(), expected.len(), "{items:?}");
        for (item, (at, ch, code)) in items.iter().zip(expected) {
            assert_eq!(item.code, Some(NumberOrString::String(code.into())));
            assert_eq!(item.range, text.range(at, at + 1));
            assert_eq!(item.data.as_ref().unwrap()["character"], ch.to_string());
            let actions =
                editing::code_actions(&workspace, &uri, item.range, &context(vec![item.clone()]));
            assert_eq!(actions.len(), 1);
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!()
            };
            assert_eq!(action.title, format!("Insert empty base before '{ch}'"));
            let changes = edits(actions, &uri);
            assert_eq!(changes[0].range, item.range);
            assert_eq!(changes[0].new_text, format!("{{}}{ch}"));
        }
    }
}

#[test]
fn double_scripts_require_matching_symbols_direct_context_and_unconsumed_tokens() {
    for (character, message) in [('_', "Double subscript."), ('^', "Double superscript.")] {
        let direct = format!("$x{character}1{character}2$");
        let positive = format!("$x{character}1\\relax{character}2$");
        let unknown = format!("$x{character}1\\unknown{character}2$");
        let parameter = format!("\\def\\bad#1{{{character}}}$x{character}1\\bad{character}2$");
        let delimited =
            format!("\\def\\bad#1{character}{{{character}}}$x{character}1\\bad x{character}2$");
        for (source, frame, precise) in [
            (direct.clone(), "", true),
            (positive, "", true),
            (unknown, "", false),
            (parameter, "", false),
            (delimited, "", false),
            (direct.clone(), "\\bad ->_\n", false),
            (direct.clone(), "<argument> _\n", false),
            (direct.clone(), "<recently read> _\n", false),
            (format!("% {direct}"), "", false),
            (format!("\\verb|{direct}|"), "", false),
            (format!("\\def\\bad{{{direct}}}"), "", false),
            (format!("$x{character}1\\{character}2$"), "", false),
        ] {
            let at = source.rfind(character).unwrap();
            let prefix = &source[..at + 1];
            let log = format!("(./main.tex\n! {message}\n{frame}l.1 {prefix}\n\n)\n");
            let (_directory, workspace, uri, items) = setup(&source, &log);
            assert_eq!(items.len(), 1, "{log}: {items:?}");
            assert_eq!(items[0].code.is_some(), precise, "{log}: {items:?}");
            assert_eq!(
                !editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()))
                    .is_empty(),
                precise
            );
        }
        for (source, prefix, location) in [
            (direct.as_str(), "Wrong _", "./main.tex:1"),
            (direct.as_str(), "$x_1^", "./main.tex:1"),
            (direct.as_str(), "$x^1_", "./main.tex:1"),
            (direct.as_str(), "$x_1_", "./gone.tex:1"),
            (direct.as_str(), "$x_1_", "./main.tex:0"),
            (direct.as_str(), "$x_1_", "./main.tex:99"),
            (
                "Repeated _ middle Repeated _",
                "...Repeated _",
                "./main.tex:1",
            ),
        ] {
            let number = location.rsplit(':').next().unwrap();
            let log = format!("(./main.tex\n{location}: {message}\nl.{number} {prefix}\n\n)\n");
            let (_directory, workspace, uri, items) = setup(source, &log);
            assert_eq!(items.len(), 1);
            assert!(items[0].code.is_none(), "{log}: {items:?}");
            assert!(editing::code_actions(
                &workspace,
                &uri,
                items[0].range,
                &context(items.clone())
            )
            .is_empty());
        }
    }
}

#[test]
fn legacy_double_script_repairs_reject_escapes_forged_codes_and_outdated_sources() {
    for (character, message) in [('_', "Double subscript."), ('^', "Double superscript.")] {
        let source = format!("$x{character}1{character}2$\n");
        let prefix = source[..source.rfind(character).unwrap() + 1].to_string();
        let log = format!("(./main.tex\n! {message}\nl.1 {prefix}\n)\n");
        let (_directory, mut workspace, uri, items) = setup(&source, &log);
        workspace.code_action_literals = false;
        workspace.apply_edits = true;
        let actions =
            editing::code_actions(&workspace, &uri, items[0].range, &context(items.clone()));
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::Command(command) = &actions[0] else {
            panic!()
        };
        let argument = command.arguments.as_ref().unwrap()[0].clone();
        let request = || serde_json::from_value::<editing::FixRequest>(argument.clone()).unwrap();
        assert!(editing::resolve_fix(&workspace, request()).is_some());
        let mut forged = argument.clone();
        forged["replacement"] = json!(format!("\\{character}"));
        assert!(
            editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none()
        );
        workspace.documents.insert(
            uri.clone(),
            Document::new(&uri, source.replace("x", "y"), Some(9)),
        );
        assert!(editing::resolve_fix(&workspace, request()).is_none());
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(10)));
        assert!(editing::resolve_fix(&workspace, request()).is_none());
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source.clone(), Some(9)));
        for diagnostic in workspace
            .build_diagnostics
            .values_mut()
            .flat_map(|files| files.values_mut())
            .flatten()
        {
            diagnostic.code = Some(NumberOrString::String("unescaped-special-character".into()));
        }
        assert!(editing::resolve_fix(&workspace, request()).is_none());
        assert!(
            editing::code_actions(&workspace, &uri, items[0].range, &context(vec![])).is_empty()
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_double_script_empty_bases_repair_inline_display_and_grouped_scripts() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                for verbose in [false, true] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path();
                    let main = root.join("main.tex");
                    let child = root.join("child.tex");
                    let compiler = root.join(engine);
                    std::fs::write(&compiler,format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    let mut arguments = vec![
                        "-interaction=nonstopmode".into(),
                        "-recorder".into(),
                        "-output-directory=%o".into(),
                        "%f".into(),
                    ];
                    if numbered {
                        arguments.insert(0, "-file-line-error".into());
                    }
                    let config = latex_lsp::config::BuildConfig {
                        executable: compiler.to_string_lossy().into(),
                        arguments: Some(arguments),
                        ..Default::default()
                    };
                    std::fs::write(&main,format!("\\documentclass{{article}}\n{}\\begin{{document}}\n\\input{{child}}\n\\end{{document}}\n",if verbose {"\\errorcontextlines=100\n"} else {""})).unwrap();
                    let legal = concat!(
                        "$x_1^2$ and $x_{a_b}^{c^d}$ and escaped \\_ \\textasciicircum{}.\n",
                        "\\verb|x_1_2 and y^1^2| % x_1_2\n",
                    );
                    let source=format!("{}$x_1_2$ and $y^1^2$ and $z_{{ab}}_{{cd}}$ and $w^{{ab}}^{{cd}}$ and $a_1_2_3$\n\\[d_1_2\\]\n$q_1% preserve comment\n _2$\n{legal}","long prefix ".repeat(20));
                    std::fs::write(&child, &source).unwrap();
                    let outcome = latex_lsp::build::run(&main, &config, &Default::default());
                    assert_eq!(
                        outcome.status, "error",
                        "{engine}/{width}/{numbered}/{verbose}: {}",
                        outcome.output
                    );
                    let uri = Url::from_file_path(&child).unwrap();
                    let items:Vec<_>=outcome.diagnostics[&uri].iter().filter(|item|matches!(item.code.as_ref(),Some(NumberOrString::String(code)) if matches!(code.as_str(),"double-subscript"|"double-superscript"))).cloned().collect();
                    assert_eq!(
                        items.len(),
                        8,
                        "{engine}/{width}/{numbered}/{verbose}: {:?}\n{}",
                        outcome.diagnostics,
                        outcome.output
                    );
                    let mut workspace = Workspace {
                        code_action_literals: true,
                        versioned_edits: true,
                        ..Default::default()
                    };
                    workspace.add_root(root.into());
                    workspace.open(uri.clone(), source.clone(), 9);
                    workspace
                        .build_diagnostics
                        .insert(Url::from_file_path(&main).unwrap(), outcome.diagnostics);
                    let text = Text::new(source.clone());
                    let mut changes = Vec::new();
                    for item in items {
                        let offset = text.offset(item.range.start).unwrap();
                        assert!(matches!(&source[offset..offset + 1], "_" | "^"));
                        let actions = editing::code_actions(
                            &workspace,
                            &uri,
                            item.range,
                            &context(vec![item]),
                        );
                        assert_eq!(actions.len(), 1);
                        changes.extend(edits(actions, &uri));
                    }
                    changes.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
                    let mut fixed = source.clone();
                    for edit in changes {
                        fixed.replace_range(
                            text.offset(edit.range.start).unwrap()
                                ..text.offset(edit.range.end).unwrap(),
                            &edit.new_text,
                        );
                    }
                    assert!(fixed.contains("$x_1{}_2$ and $y^1{}^2$"));
                    assert!(fixed.contains("$q_1% preserve comment\n {}_2$"));
                    assert!(fixed.ends_with(legal));
                    std::fs::write(&child, fixed).unwrap();
                    let repaired = latex_lsp::build::run(&main, &config, &Default::default());
                    assert_eq!(
                        repaired.status, "success",
                        "{engine}/{width}/{numbered}/{verbose}: {}",
                        repaired.output
                    );
                    assert!(repaired
                        .diagnostics
                        .values()
                        .flatten()
                        .all(|item| item.severity != Some(DiagnosticSeverity::ERROR)));
                    for source in [
                        r"\errorcontextlines=-1\def\bad#1{_}$x_1\bad_2$",
                        r"\errorcontextlines=-1\def\bad#1^{^}$x^1\bad x^2$",
                    ] {
                        std::fs::write(&child, source).unwrap();
                        let macro_error =
                            latex_lsp::build::run(&main, &config, &Default::default());
                        assert_eq!(macro_error.status, "error");
                        assert!(macro_error.diagnostics.values().flatten().all(|item|!matches!(item.code.as_ref(),Some(NumberOrString::String(code)) if matches!(code.as_str(),"double-subscript"|"double-superscript"))),"{engine}/{width}/{numbered}/{verbose}: {:?}",macro_error.diagnostics);
                    }
                }
            }
        }
    }
}
