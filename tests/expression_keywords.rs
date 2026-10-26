use latex_lsp::{
    compilation::build_log, editing, language::features, project::workspace::Workspace, text::Text,
};
use lsp_types::*;
use std::collections::BTreeMap;

fn setup(root: &std::path::Path, source: &str) -> (Workspace, Url, Url) {
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(
        root.join("definitions.tex"),
        "\\newbool{draft}\\newtoggle{draft}\n",
    )
    .unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    (
        workspace,
        Url::from_file_path(root.join("main.tex")).unwrap(),
        Url::from_file_path(root.join("definitions.tex")).unwrap(),
    )
}

fn changes(workspace: &Workspace, edit: WorkspaceEdit) -> BTreeMap<Url, String> {
    edit.changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let text = workspace.documents[&uri]
                .text
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
                .unwrap();
            (uri, text.source)
        })
        .collect()
}

#[test]
fn commented_keywords_keep_boolean_toggle_navigation_and_rename_namespaces_separate() {
    let directory = tempfile::tempdir().unwrap();
    let source = concat!(
        "\\documentclass{article}\\input{definitions}\r\n",
        "😀 \\ifboolexpr{bo% bool\r\n  ol{draft} a% and\r\n nd no% not\r\n t to% toggle\r\n gl{draft}}{Yes}{No}\r\n",
        "\\ifboolexpe{n% negate\n otbo% boolean\n ol{draft} o% or\n r to% toggle\n gl{draft}}{Yes}{No}\n",
        "\\whileboolexpr{bo% boolean\n ol{draft} and to% toggle\n gl{draft}}{Body}\n",
        "\\unlessboolexpr{bo% boolean\n ol{draft} or to% toggle\n gl{draft}}{Body}\n",
        "\\ifboolexpr{te% test\n st{\\ifstrequal{bool{draft}}{togl{draft}}}or bool{true}}{Text}{No}\n",
    );
    let (workspace, uri, declaration) = setup(directory.path(), source);
    for (marker, replacement, declaration_text) in [
        (
            " ol{draft}",
            " ol{release}",
            "\\newbool{release}\\newtoggle{draft}\n",
        ),
        (
            " gl{draft}",
            " gl{release}",
            "\\newbool{draft}\\newtoggle{release}\n",
        ),
    ] {
        let spans: Vec<_> = source
            .match_indices(marker)
            .map(|(start, _)| start + 4)
            .collect();
        assert_eq!(spans.len(), 4);
        for start in spans {
            let position = Text::new(source.into()).position(start);
            let targets = features::definitions(&workspace, &uri, position);
            assert_eq!(targets.len(), 1, "{marker}: {targets:?}");
            assert_eq!(targets[0].uri, declaration);
            assert!(features::hover(&workspace, &uri, position).is_some());
            assert!(
                matches!(features::prepare_rename(&workspace, &uri, position).unwrap(), Some(PrepareRenameResponse::RangeWithPlaceholder {placeholder, ..}) if placeholder == "draft")
            );
            assert_eq!(
                features::references(&workspace, &uri, position, true).len(),
                5
            );
            let renamed = changes(
                &workspace,
                features::rename(&workspace, &uri, position, "release")
                    .unwrap()
                    .unwrap(),
            );
            assert_eq!(renamed[&uri], source.replace(marker, replacement));
            assert_eq!(renamed[&declaration], declaration_text);
        }
    }
}

#[test]
fn commented_keywords_complete_whole_names_in_complete_and_unfinished_expressions() {
    for (marked, label) in [
        ("😀 \\ifboolexpr{bo% note\r\n  ol{dra¦ft}}{Y}{N}", "draft"),
        (
            "\\ifboolexpe{no% a\n % b\n tbo% c\n ol{dra¦ft}}{Y}{N}",
            "draft",
        ),
        ("\\whileboolexpr{to% note\n gl{dra¦", "draft"),
        ("\\unlessboolexpr{bo% note\n ol{tr¦ue}}{Body}", "true"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let (workspace, uri, _) = setup(directory.path(), &source);
        let items = features::completion(
            &workspace,
            &uri,
            Text::new(source.clone()).position(marked.find('¦').unwrap()),
        );
        let item = items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing edit")
        };
        let changed = Text::new(source)
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        assert_eq!(
            changed.source,
            marked
                .replace("dra¦ft", "draft")
                .replace("dra¦", "draft")
                .replace("tr¦ue", "true")
        );
    }
}

#[test]
fn ordinary_spaces_blank_lines_escaped_tokens_and_test_literals_do_not_form_keywords() {
    for marked in [
        "\\ifboolexpr{bo ol{dra¦ft}}{Y}{N}",
        "\\ifboolexpr{bo% note\n\n ol{dra¦ft}}{Y}{N}",
        r"\ifboolexpr{bo\%ol{dra¦ft}}{Y}{N}",
        "\\ifboolexpr{mybo% note\n ol{dra¦ft}}{Y}{N}",
        "\\ifboolexpr{te% note\n st{\\ifstrequal{bo% note\n ol{dra¦ft}}{X}}}{Y}{N}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = marked.replace('¦', "");
        let (workspace, uri, declaration) = setup(directory.path(), &source);
        let position = Text::new(source.clone()).position(marked.find('¦').unwrap());
        assert!(
            features::definitions(&workspace, &uri, position).is_empty(),
            "{marked}"
        );
        assert!(
            features::completion(&workspace, &uri, position).is_empty(),
            "{marked}"
        );
        let renamed = changes(
            &workspace,
            features::rename(&workspace, &declaration, Position::new(0, 10), "release")
                .unwrap()
                .unwrap(),
        );
        assert!(!renamed.contains_key(&uri), "{marked}: {renamed:?}");
    }
}

#[test]
fn commented_expression_keywords_keep_exact_compiler_ranges_and_spelling_repairs() {
    for (command, token, kind) in [
        ("ifboolexpr", "bo% operand\r\n ol", "boolean"),
        ("ifboolexpe", "to% operand\n gl", "toggle"),
        ("whileboolexpr", "bo% operand\n ol", "boolean"),
        ("unlessboolexpr", "to% operand\n gl", "toggle"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let call = format!("\\{command}{{no% negate\n t({token}{{darft}}) o% or\n r te% test\n st{{\\ifstrequal{{\\name}}{{A}}}}}}{{Body}}");
        let source = format!("\\documentclass{{article}}\\input{{definitions}}\n{call}\n");
        let (mut workspace, uri, _) = setup(directory.path(), &source);
        let echo = source.lines().last().unwrap();
        let namespace = if kind == "boolean" {
            "Boolean"
        } else {
            "Toggle"
        };
        let log = format!("(./main.tex\n! Package etoolbox Error: Invalid boolean expression.\n\nSee the etoolbox package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\nl.{} {echo}\n    \n{namespace} 'darft' undefined.\n\n)\n", source.lines().count());
        // Conditionals consume only the expression, while loops consume the body.
        let log = if command.starts_with("if") {
            log.replace("{Body}\n", "\n")
        } else {
            log
        };
        let diagnostics = build_log::parse(&directory.path().join("main.tex"), &log);
        let error = diagnostics[&uri][0].clone();
        assert_eq!(
            error.code,
            Some(NumberOrString::String(format!("undefined-{kind}"))),
            "{command}: {error:?}"
        );
        let start = source.find("darft").unwrap();
        assert_eq!(
            error.range,
            Text::new(source.clone()).range(start, start + 5)
        );
        workspace.client.code_action_literals = true;
        workspace
            .project
            .build_diagnostics
            .insert(uri.clone(), diagnostics);
        let actions = editing::code_actions(
            &workspace,
            &uri,
            error.range,
            &CodeActionContext {
                diagnostics: vec![error],
                only: Some(vec![CodeActionKind::QUICKFIX]),
                trigger_kind: None,
            },
        );
        let edit = actions
            .into_iter()
            .find_map(|item| match item {
                CodeActionOrCommand::CodeAction(action)
                    if action.title == format!("Replace with {kind} `draft`") =>
                {
                    action.edit
                }
                _ => None,
            })
            .expect("missing repair");
        assert_eq!(
            changes(&workspace, edit)[&uri],
            source.replace("darft", "draft")
        );
    }
}

#[test]
fn comment_spelling_in_macro_tests_and_malformed_keywords_stays_unlocated() {
    for expression in [
        "bo ol{darft}",
        "bo% comment\n\n ol{darft}",
        r"bo\%ol{darft}",
        "bo% comment\n ol{darft} a nd bool{true}",
        "te% comment\n st{\\probe} or bo% comment\n ol{darft}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!("\\documentclass{{article}}\\input{{definitions}}\n\\ifboolexpr{{{expression}}}{{Y}}{{N}}\n");
        let (_, uri, _) = setup(directory.path(), &source);
        let echo = source
            .lines()
            .last()
            .unwrap()
            .strip_suffix("{Y}{N}")
            .unwrap();
        let output = format!("(./main.tex\n! Package etoolbox Error: Invalid boolean expression.\n\nSee the etoolbox package documentation for explanation.\nType  H <return>  for immediate help.\n ...\n\nl.{} {echo}\n    {{Y}}{{N}}\nBoolean 'darft' undefined.\n\n)\n", source.lines().count());
        let diagnostics = build_log::parse(&directory.path().join("main.tex"), &output);
        assert!(
            diagnostics[&uri][0].code.is_none(),
            "{expression}: {:?}",
            diagnostics[&uri]
        );
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_commented_keywords_preserve_cross_file_renames_and_compile_repairs_in_three_engines() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    let original = concat!(
        "\\documentclass{article}\\usepackage{etoolbox}\\input{definitions}\n",
        "\\begin{document}\\newcounter{steps}\\booltrue{draft}\\togglefalse{draft}\n",
        "\\ifboolexpr{bo% bool\r\n  ol{draft} a% and\r\n nd no% not\r\n t to% toggle\r\n gl{draft}}{\\typeout{KEYWORDS:mixed}}{Wrong.}\r\n",
        "\\ifboolexpr{te% test\n st{\\ifstrequal{bo% literal\n ol{draft}}{bo% literal\n ol{draft}}} o% or\n r to% toggle\n gl{draft}}{\\typeout{KEYWORDS:literal}}{Wrong.}\n",
        "\\ifboolexpe{no% not\n tbo% bool\n ol{draft}}{Wrong.}{\\typeout{KEYWORDS:expanded}}\n",
        "\\whileboolexpr{bo% loop\n ol{draft}}{\\boolfalse{draft}\\stepcounter{steps}}\n",
        "\\unlessboolexpr{to% loop\n gl{draft}}{\\toggletrue{draft}\\stepcounter{steps}}\n",
        "\\typeout{KEYWORDS:steps:\\arabic{steps}}Text.\\end{document}\n",
    );
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for numbered in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let (mut workspace, uri, _) = setup(root, original);
            let main = root.join("main.tex");
            let wrapper = root.join("engine");
            std::fs::write(&wrapper, format!("#!/bin/sh\nexport TEXMFVAR=\"${{0%/*}}/tex-cache\"\nexport TEXMFCACHE=\"$TEXMFVAR\"\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = BuildConfig {
                executable: wrapper.to_string_lossy().into_owned(),
                arguments: Some(vec![
                    "-interaction=nonstopmode".into(),
                    "-no-shell-escape".into(),
                    if numbered {
                        "-file-line-error"
                    } else {
                        "-no-file-line-error"
                    }
                    .into(),
                    "-recorder".into(),
                    "-output-directory=%o".into(),
                    "%f".into(),
                ]),
                timeout_seconds: 20,
                ..Default::default()
            };
            let before = build::run(&main, &config, &Default::default());
            assert_eq!(
                before.status, "success",
                "{engine}/{numbered}: {}",
                before.output
            );
            let states = |output: &str| {
                output
                    .lines()
                    .filter(|line| line.starts_with("KEYWORDS:"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            let expected = vec![
                "KEYWORDS:mixed",
                "KEYWORDS:literal",
                "KEYWORDS:expanded",
                "KEYWORDS:steps:2",
            ];
            assert_eq!(states(&before.output), expected);
            for (marker, replacement) in [("  ol{draft}", "release"), (" gl{draft}", "production")]
            {
                let text = &workspace.documents[&uri].text;
                let position = text
                    .position(text.source.find(marker).unwrap() + marker.find("draft").unwrap());
                let edited = changes(
                    &workspace,
                    features::rename(&workspace, &uri, position, replacement)
                        .unwrap()
                        .unwrap(),
                );
                for (document, source) in edited {
                    std::fs::write(document.to_file_path().unwrap(), source).unwrap();
                }
                workspace = Workspace::default();
                workspace.add_root(root.into());
                let after = build::run(&main, &config, &Default::default());
                assert_eq!(
                    after.status, "success",
                    "{engine}/{numbered}/{replacement}: {}",
                    after.output
                );
                assert_eq!(states(&after.output), expected);
                assert_eq!(after.pages, Some(1));
            }
            let renamed = std::fs::read_to_string(&main).unwrap();
            assert_eq!(renamed.matches("bo% literal\n ol{draft}").count(), 2);
            let broken = renamed.replacen("ol{release}", "ol{relesae}", 1).replacen(
                "gl{production}",
                "gl{prodution}",
                1,
            );
            std::fs::write(&main, &broken).unwrap();
            let failed = build::run(&main, &config, &Default::default());
            assert_eq!(failed.status, "error", "{}", failed.output);
            let errors = failed.diagnostics[&uri]
                .iter()
                .filter(|error| {
                    error
                        .message
                        .starts_with("Package etoolbox Error: Invalid boolean expression.")
                })
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(errors.len(), 2, "{}", failed.output);
            workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
                    code_action_literals: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace
                .project
                .build_diagnostics
                .insert(uri.clone(), failed.diagnostics);
            workspace
                .project
                .build_diagnostic_sources
                .insert(uri.clone(), failed.diagnostic_sources);
            let mut edits = vec![];
            for (error, (kind, typo, name)) in errors.into_iter().zip([
                ("boolean", "relesae", "release"),
                ("toggle", "prodution", "production"),
            ]) {
                let start = broken.find(typo).unwrap();
                assert_eq!(
                    error.range,
                    Text::new(broken.clone()).range(start, start + typo.len()),
                    "{engine}/{numbered}: {error:?}"
                );
                assert_eq!(
                    error.code,
                    Some(NumberOrString::String(format!("undefined-{kind}")))
                );
                let actions = editing::code_actions(
                    &workspace,
                    &uri,
                    error.range,
                    &CodeActionContext {
                        diagnostics: vec![error],
                        only: Some(vec![CodeActionKind::QUICKFIX]),
                        trigger_kind: None,
                    },
                );
                let edit = actions
                    .into_iter()
                    .find_map(|item| match item {
                        CodeActionOrCommand::CodeAction(action)
                            if action.title == format!("Replace with {kind} `{name}`") =>
                        {
                            action.edit
                        }
                        _ => None,
                    })
                    .expect("missing real repair");
                edits.extend(edit.changes.unwrap()[&uri].clone());
            }
            let repaired = changes(
                &workspace,
                WorkspaceEdit {
                    changes: Some([(uri.clone(), edits)].into()),
                    ..Default::default()
                },
            );
            assert_eq!(repaired[&uri], renamed);
            std::fs::write(&main, &repaired[&uri]).unwrap();
            let after = build::run(&main, &config, &Default::default());
            assert_eq!(
                after.status, "success",
                "{engine}/{numbered}: {}",
                after.output
            );
            assert_eq!(states(&after.output), expected);
        }
    }
}
