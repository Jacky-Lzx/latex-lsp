use lsp_types::*;
use std::path::Path;
use texquill::{
    compilation::build_log,
    editing::{self, file_operations},
    language::{features, navigation, signature},
    project::workspace::Workspace,
    text::Text,
};

const INPUTS: [&str; 3] = ["VerbatimInput", "LVerbatimInput", "BVerbatimInput"];

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
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

fn edits(edit: &WorkspaceEdit, main: &Url) -> Vec<TextEdit> {
    let Some(DocumentChanges::Edits(documents)) = &edit.document_changes else {
        panic!()
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *main);
    assert_eq!(documents[0].text_document.version, Some(7));
    documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect()
}

#[test]
fn variants_complete_navigate_and_rename_wrapped_root_paths_with_versions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("code")).unwrap();
    std::fs::create_dir(root.join("chapters")).unwrap();
    std::fs::create_dir(root.join("chapters/code")).unwrap();
    std::fs::write(
        root.join("main.tex"),
        r"\documentclass{article}\input{chapters/child}",
    )
    .unwrap();
    for name in [
        "sample.rs",
        "sample.unknown",
        "README",
        "sample one.rs",
        "样例 one.rs",
    ] {
        std::fs::write(root.join("code").join(name), "Code.").unwrap();
        std::fs::write(root.join("chapters/code").join(name), "Decoy.").unwrap();
    }
    let child = uri(root, "chapters/child.tex");
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    for command in INPUTS {
        for (name, value) in [
            ("sample.rs", "code/sample.rs"),
            ("sample.unknown", "code/sample.unknown"),
            ("README", "code/README"),
            ("sample one.rs", "\"code/sample one.rs\""),
            ("样例 one.rs", "\\detokenize{\"code/样例 one.rs\"}"),
        ] {
            let source = format!(
                "% 😀\r\n\\{command} *[frame=single,label={{A,B [ C]}}] % keep\r\n{{{value}}}\r\n"
            );
            std::fs::write(child.to_file_path().unwrap(), &source).unwrap();
            workspace.open(child.clone(), source.clone(), 7);
            let links = navigation::document_links(&workspace, &child);
            assert_eq!(links.len(), 1, "{source}: {links:?}");
            assert_eq!(links[0].target, Some(uri(root, &format!("code/{name}"))));
            let from = source.find("code/").unwrap() + "code/".len();
            let at = from + source[from..].chars().next().unwrap().len_utf8();
            let position = Text::new(source.clone()).position(at);
            assert_eq!(
                features::definitions(&workspace, &child, position)[0].uri,
                uri(root, &format!("code/{name}"))
            );
            let help = signature::help(&workspace, &child, position).unwrap();
            assert_eq!(
                help.signatures[0].label,
                format!("\\{command}[options]{{file}}")
            );
            assert_eq!(help.active_parameter, Some(1));
            let items = features::completion(&workspace, &child, position);
            let completion = items
                .iter()
                .find(|item| item.label == format!("code/{name}"))
                .unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &completion.text_edit else {
                panic!()
            };
            let mut completion_edits = completion.additional_text_edits.clone().unwrap_or_default();
            completion_edits.push(edit.clone());
            let completed = apply(&source, completion_edits);
            assert!(completed.contains(&format!("code/{name}")));
            assert!(completed.contains("*[frame=single,label={A,B [ C]}] % keep\r\n"));
            for new in ["new dir", "代码"] {
                let edit = file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: uri(root, "code").into(),
                        new_uri: uri(root, new).into(),
                    }],
                )
                .unwrap()
                .unwrap();
                let updated = apply(&source, edits(&edit, &child));
                assert!(updated.contains(&format!("{new}/{name}")), "{updated}");
                assert!(updated.contains("% keep\r\n"));
                assert!(updated.contains('"'));
                if !new.is_ascii() || !name.is_ascii() {
                    assert!(updated.contains("\\detokenize"), "{updated}");
                }
            }
        }
    }
}

#[test]
fn variant_directory_completions_keep_suffixes_and_single_line_main_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("目录")).unwrap();
    std::fs::write(root.join("目录/sample.rs"), "Code.").unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for command in INPUTS {
        let source = format!("% 😀\r\n\\{command}{{\r\n \\detokenize{{\"目/sample.rs\"}}\r\n}}");
        let at = source.find("目/").unwrap() + "目".len();
        workspace.open(main.clone(), source.clone(), 7);
        let cursor = Text::new(source.clone()).position(at);
        let items = features::completion(&workspace, &main, cursor);
        let item = items.iter().find(|item| item.label == "目录/").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.range.start.line, cursor.line);
        assert_eq!(edit.range.end.line, cursor.line);
        let mut edits = item.additional_text_edits.clone().unwrap_or_default();
        edits.push(edit.clone());
        let updated = apply(&source, edits);
        assert_eq!(
            updated,
            format!("% 😀\r\n\\{command}{{\\detokenize{{\"目录/sample.rs\"}}}}")
        );
    }
}

#[test]
fn mixed_display_and_execution_retains_source_labels_without_importing_unrelated_display_data() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    let code = uri(root, "commands.tex");
    let data = uri(root, "display.tex");
    std::fs::write(root.join("commands.tex"), r"\label{executedlabel}").unwrap();
    std::fs::write(root.join("display.tex"), r"\label{displayonly}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for display in INPUTS
        .into_iter()
        .chain(["verbatiminput", "lstinputlisting"])
    {
        for execute in INPUTS {
            let source = format!(
                r"\documentclass{{article}}\{display}{{commands.tex}}\{display}{{display.tex}}\{execute}[commandchars=abc]{{commands.tex}}\ref{{executedlabel}}\ref{{displayonly}}"
            );
            std::fs::write(root.join("main.tex"), &source).unwrap();
            workspace.open(main.clone(), source.clone(), 7);
            workspace
                .project
                .recorded_inputs
                .insert(main.clone(), [code.clone(), data.clone()].into());
            assert!(
                workspace.project_documents(&main).contains(&code),
                "{display}/{execute}"
            );
            assert!(
                !workspace.project_documents(&main).contains(&data),
                "{display}/{execute}"
            );
            let text = Text::new(source.clone());
            let definitions = features::definitions(
                &workspace,
                &main,
                text.position(source.find("executedlabel").unwrap()),
            );
            assert_eq!(definitions.len(), 1, "{display}/{execute}: {definitions:?}");
            assert_eq!(definitions[0].uri, code);
            assert!(features::definitions(
                &workspace,
                &main,
                text.position(source.find("displayonly").unwrap())
            )
            .is_empty());
        }
    }
}

#[test]
fn family_customizations_do_not_guess_paths_or_exclude_recorded_executable_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    let code = uri(root, "sample.rs");
    std::fs::write(root.join("sample.rs"), r"\label{executed}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for command in INPUTS {
        for settings in [
            r"\fvset{commandchars=abc}",
            r"\def\FV@@Input{}",
            r"\FV@AddToHook\FV@CatCodesHook{}",
            r"\def\LVerbatimInput#1{}",
            r"\let\BVerbatimInput\custom",
        ] {
            let source = format!(r"{settings}\{command}{{sample.rs}}");
            workspace.open(main.clone(), source, 7);
            workspace
                .project
                .recorded_inputs
                .insert(main.clone(), [code.clone()].into());
            assert!(
                navigation::document_links(&workspace, &main).is_empty(),
                "{command}/{settings}"
            );
            assert!(
                workspace.project_documents(&main).contains(&code),
                "{command}/{settings}"
            );
        }
    }
}

#[test]
fn variant_missing_diagnostics_and_fixes_keep_the_actual_command_and_wrappers() {
    for command in INPUTS {
        for (wrong, right, value, expected) in [
            ("smaple.rs", "sample.rs", "smaple.rs", "sample.rs"),
            ("my oen.rs", "my one.rs", "\"my oen.rs\"", "\"my one.rs\""),
            (
                "样例 oen.rs",
                "样例 one.rs",
                "\\detokenize{\"样例 oen.rs\"}",
                "\\detokenize{\"样例 one.rs\"}",
            ),
            (
                "sample.rs",
                "samplé.rs",
                "sample.rs",
                "\\detokenize{\"samplé.rs\"}",
            ),
            ("sample.rs", "sampl e.rs", "sample.rs", "\"sampl e.rs\""),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = uri(root, "main.tex");
            let source = format!("% 😀\r\n\\{command} *[frame=single] % keep\r\n{{{value}}}\r\n");
            std::fs::write(root.join("main.tex"), &source).unwrap();
            std::fs::write(root.join(right), "Code.").unwrap();
            let report = if value.contains('"') {
                format!("\"{wrong}\"")
            } else {
                wrong.into()
            };
            let echo = source.lines().nth(2).unwrap();
            let log = format!("(./main.tex\n./main.tex:3: FancyVerb Error:\n  No verbatim file {report}\n.\n\\FV@Error ...ncyVerb Error:^^J\\space \\space #1^^J}}\n\nl.3 {echo}\n)\n");
            let items = build_log::parse(&root.join("main.tex"), &log);
            let item = items[&main]
                .iter()
                .find(|item| {
                    item.code == Some(NumberOrString::String("missing-fancyverb-input".into()))
                })
                .unwrap_or_else(|| panic!("{source}: {items:?}"));
            let text = Text::new(source.clone());
            let start = source.find(wrong).unwrap();
            assert_eq!(item.range, text.range(start, start + wrong.len()));
            let mut workspace = Workspace {
                client: texquill::project::state::ClientCapabilities {
                    code_action_literals: true,
                    versioned_edits: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            workspace
                .project
                .build_diagnostics
                .insert(main.clone(), items.clone());
            let fixes = editing::code_actions(
                &workspace,
                &main,
                item.range,
                &CodeActionContext {
                    diagnostics: vec![item.clone()],
                    ..Default::default()
                },
            );
            assert_eq!(fixes.len(), 1, "{command}/{wrong}: {fixes:?}");
            let CodeActionOrCommand::CodeAction(fix) = &fixes[0] else {
                panic!()
            };
            assert_eq!(
                apply(&source, edits(fix.edit.as_ref().unwrap(), &main)),
                source.replace(value, expected)
            );
        }
    }
}

#[test]
fn shortened_echoes_must_identify_one_call_across_all_fancyverb_layouts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let name = format!("{}.rs", "long.part.".repeat(9));
    let source = INPUTS
        .map(|command| format!(r"\{command}{{{name}}}"))
        .join("");
    std::fs::write(&main, &source).unwrap();
    let suffix = &name[name.len() - 30..];
    let log = format!("(./main.tex\n./main.tex:1: FancyVerb Error:\n  No verbatim file {name}\n.\n\\FV@Error ...ncyVerb Error:^^J\\space \\space #1^^J}}\nl.1 ...{suffix}}}\n)\n");
    let items = build_log::parse(&main, &log);
    assert!(items.values().flatten().all(|item| item.code != Some(NumberOrString::String("missing-fancyverb-input".into()))), "{items:?}");
}

#[test]
#[ignore = "requires pdflatex, xelatex and fancyvrb"]
fn real_variant_missing_fixes_compile_wrapped_paths_with_both_engines() {
    for engine in ["pdflatex", "xelatex"] {
        for command in ["LVerbatimInput", "BVerbatimInput"] {
            for (wrong, right, value) in [
                ("smaple.rs", "sample.rs", "smaple.rs"),
                ("smaple.unknown", "sample.unknown", "smaple.unknown"),
                ("CODEDOCUMETN", "CODEDOCUMENT", "CODEDOCUMETN"),
                ("my oen.rs", "my one.rs", "\"my oen.rs\""),
                (
                    "样例 oen.rs",
                    "样例 one.rs",
                    "\\detokenize{\"样例 oen.rs\"}",
                ),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = uri(root, "main.tex");
                let source = format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{fancyvrb}}\\begin{{document}}\r\n\\{command} *[frame=single] % keep\r\n{{{value}}}\r\n\\end{{document}}\r\n");
                std::fs::write(root.join("main.tex"), &source).unwrap();
                std::fs::write(root.join(right), "Original code.\n").unwrap();
                let config = texquill::config::BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                };
                let first = texquill::compilation::build::run(
                    &root.join("main.tex"),
                    &config,
                    &Default::default(),
                );
                assert_eq!(
                    first.status, "error",
                    "{engine}/{command}/{wrong}: {}",
                    first.output
                );
                let item = first.diagnostics[&main]
                    .iter()
                    .find(|item| {
                        item.code == Some(NumberOrString::String("missing-fancyverb-input".into()))
                    })
                    .unwrap_or_else(|| panic!("{engine}/{command}: {:?}", first.diagnostics))
                    .clone();
                let text = Text::new(source.clone());
                let start = source.find(wrong).unwrap();
                assert_eq!(item.range, text.range(start, start + wrong.len()));
                let mut workspace = Workspace {
                    client: texquill::project::state::ClientCapabilities {
                        code_action_literals: true,
                        versioned_edits: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace.open(main.clone(), source.clone(), 7);
                workspace
                    .project
                    .build_diagnostics
                    .insert(main.clone(), first.diagnostics);
                let fixes = editing::code_actions(
                    &workspace,
                    &main,
                    item.range,
                    &CodeActionContext {
                        diagnostics: vec![item],
                        ..Default::default()
                    },
                );
                assert_eq!(fixes.len(), 1, "{engine}/{command}/{wrong}: {fixes:?}");
                let CodeActionOrCommand::CodeAction(fix) = &fixes[0] else {
                    panic!()
                };
                let updated = apply(&source, edits(fix.edit.as_ref().unwrap(), &main));
                std::fs::write(root.join("main.tex"), updated).unwrap();
                let fixed = texquill::compilation::build::run(
                    &root.join("main.tex"),
                    &config,
                    &Default::default(),
                );
                assert_eq!(
                    fixed.status, "success",
                    "{engine}/{command}/{right}: {}",
                    fixed.output
                );
                assert!(fixed.pdf.is_some());
                let recorder =
                    std::fs::read_to_string(root.join("build/texquill/main.fls")).unwrap();
                assert!(recorder
                    .lines()
                    .any(|line| line.strip_prefix("INPUT ").is_some_and(|path| root
                        .join(path.trim_matches('"'))
                        .canonicalize()
                        .ok()
                        .as_ref()
                        == Some(&root.join(right)))));
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, fancyvrb, verbatim and listings"]
fn real_mixed_execution_keeps_labels_and_excludes_other_displayed_sources() {
    let commandchars = r"\\\{\}";
    for engine in ["pdflatex", "xelatex"] {
        for display in INPUTS
            .into_iter()
            .chain(["verbatiminput", "lstinputlisting"])
        {
            for execute in INPUTS {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = uri(root, "main.tex");
                let code = uri(root, "commands.tex");
                let data = uri(root, "display.tex");
                std::fs::write(root.join("commands.tex"), r"Executed.\label{executedlabel}")
                    .unwrap();
                std::fs::write(root.join("display.tex"), r"Displayed.\label{displayonly}").unwrap();
                let source = format!(
                    r"\documentclass{{article}}\usepackage{{fancyvrb,verbatim,listings}}\renewcommand{{\familydefault}}{{\ttdefault}}\begin{{document}}\{display}{{commands.tex}}\{display}{{display.tex}}\{execute}[commandchars={commandchars}]{{commands.tex}}\ref{{executedlabel}}\end{{document}}"
                );
                std::fs::write(root.join("main.tex"), &source).unwrap();
                let outcome = texquill::compilation::build::run(
                    &root.join("main.tex"),
                    &texquill::config::BuildConfig {
                        executable: engine.into(),
                        ..Default::default()
                    },
                    &Default::default(),
                );
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{display}/{execute}: {}",
                    outcome.output
                );
                assert!(
                    std::fs::read_to_string(root.join("build/texquill/main.aux"))
                        .unwrap()
                        .contains("newlabel{executedlabel}")
                );
                let mut workspace = Workspace::default();
                workspace.add_root(root.into());
                workspace.open(main.clone(), source.clone(), 7);
                workspace
                    .project
                    .recorded_inputs
                    .insert(main.clone(), outcome.inputs.unwrap());
                let sources = workspace.project_documents(&main);
                assert!(sources.contains(&code), "{engine}/{display}/{execute}");
                assert!(!sources.contains(&data), "{engine}/{display}/{execute}");
                let text = Text::new(source.clone());
                let definitions = features::definitions(
                    &workspace,
                    &main,
                    text.position(source.rfind("executedlabel").unwrap()),
                );
                assert_eq!(
                    definitions.len(),
                    1,
                    "{engine}/{display}/{execute}: {definitions:?}"
                );
                assert_eq!(definitions[0].uri, code);
            }
        }
    }
}
