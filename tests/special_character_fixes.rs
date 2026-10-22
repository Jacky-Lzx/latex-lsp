use latex_lsp::{
    build_log, editing,
    text::Text,
    workspace::{Document, Workspace},
};
use serde_json::json;
use tower_lsp::lsp_types::*;

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
