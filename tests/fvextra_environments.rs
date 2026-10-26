use lsp_types::*;
use texquill::{
    editing::{self, file_operations},
    language::{features, navigation, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};

const ENVIRONMENTS: [&str; 2] = ["VerbatimWrite", "VerbatimBuffer"];

fn apply(source: &str, edit: WorkspaceEdit) -> String {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!()
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.version, Some(7));
    let mut changes: Vec<_> = documents[0]
        .edits
        .iter()
        .map(|edit| {
            let OneOf::Left(edit) = edit else { panic!() };
            TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }
        })
        .collect();
    changes.sort_by_key(|change| std::cmp::Reverse(change.range.unwrap().start));
    Text::new(source.into()).changed(changes).unwrap().source
}

#[test]
fn writer_and_buffer_bodies_keep_symbols_paths_and_edits_literal() {
    for environment in ENVIRONMENTS {
        for newline in ["\n", "\r\n"] {
            for options in [
                String::new(),
                format!("[label={{\\detokenize{{\\end{{{environment}}}}}}},frame=single]"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                std::fs::write(root.join("chapter.tex"), "Chapter text").unwrap();
                std::fs::write(root.join("fake.tex"), r"\label{fake}").unwrap();
                let main = Url::from_file_path(root.join("main.tex")).unwrap();
                let marker = format!("\\end{{{environment}}}");
                let body = format!("😀 \\label{{phantom}}\\ref{{kept}}\\say\\input{{chapter}}\\input{{fake}}{newline}\\end{{other}}{marker}\\label{{stillraw}}{newline}{{unclosed literal");
                let before = format!("% 😀{newline}\\usepackage{{fvextra}}\\newcommand{{\\say}}{{Text}}\\label{{kept}}{newline}");
                let after = r"\input{chapter}\say\ref{kept}";
                let literal =
                    format!("\\begin{{{environment}}}{options}{newline}{body}{newline}{marker}");
                let source = format!("{before}{literal}{newline}{after}");
                std::fs::write(root.join("main.tex"), &source).unwrap();
                let mut workspace = Workspace {
                    client: texquill::project::state::ClientCapabilities {
                        versioned_edits: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace.open(main.clone(), source.clone(), 7);
                let doc = &workspace.documents[&main];
                assert!(
                    doc.syntax.diagnostics.is_empty(),
                    "{source}: {:?}",
                    doc.syntax.diagnostics
                );
                assert_eq!(doc.syntax.labels.len(), 1, "{source}");
                assert_eq!(doc.syntax.definitions.len(), 1, "{source}");
                assert_eq!(
                    workspace.dependencies(&main),
                    [Url::from_file_path(root.join("chapter.tex")).unwrap()]
                );
                assert_eq!(navigation::document_links(&workspace, &main).len(), 1);
                for (old, new) in [("kept", "changed"), ("say", "speak")] {
                    let at = doc.text.position(source.rfind(old).unwrap());
                    let updated = apply(
                        &source,
                        features::rename(&workspace, &main, at, new)
                            .unwrap()
                            .unwrap(),
                    );
                    assert_eq!(
                        updated,
                        format!(
                            "{}{literal}{newline}{}",
                            before.replace(old, new),
                            after.replace(old, new)
                        )
                    );
                }
                let edit = file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: Url::from_file_path(root.join("chapter.tex"))
                            .unwrap()
                            .into(),
                        new_uri: Url::from_file_path(root.join("renamed.tex"))
                            .unwrap()
                            .into(),
                    }],
                )
                .unwrap()
                .unwrap();
                let updated = apply(&source, edit);
                assert_eq!(
                    updated,
                    format!(
                        "{before}{literal}{newline}{}",
                        after.replace("chapter", "renamed")
                    )
                );
                let begin = source.find(environment).unwrap();
                let end = source.rfind(&marker).unwrap();
                let linked =
                    editing::linked_editing_ranges(doc, doc.text.position(begin + 1)).unwrap();
                assert_eq!(
                    linked.ranges[1],
                    doc.text.range(end + 5, end + marker.len() - 1)
                );
                assert!(navigation::folding_ranges(doc, None)
                    .iter()
                    .any(|fold| fold.start_line == doc.text.position(begin).line
                        && fold.end_line == doc.text.position(end).line));
                let inside = doc.text.position(source.find("phantom").unwrap());
                assert!(features::completion(&workspace, &main, inside).is_empty());
                assert!(features::definitions(&workspace, &main, inside).is_empty());
                assert!(signature::help(&workspace, &main, inside).is_none());
                let mut position = Position::default();
                let mut literal_token = false;
                for token in semantic::tokens(doc, None).unwrap().data {
                    position = Position::new(
                        position.line + token.delta_line,
                        if token.delta_line == 0 {
                            position.character + token.delta_start
                        } else {
                            token.delta_start
                        },
                    );
                    if position.line == inside.line
                        && position.character <= inside.character
                        && inside.character < position.character + token.length
                    {
                        assert_eq!(token.token_type, 4);
                        literal_token = true;
                    }
                }
                assert!(literal_token);
                let unclosed = Document::new(&main, source[..end].into(), Some(8));
                assert_eq!(unclosed.syntax.diagnostics.len(), 1);
                assert!(unclosed.syntax.diagnostics[0].message.contains(environment));
                assert!(unclosed.syntax.fixes[0].edit.new_text.contains(&marker));
            }
        }
    }
}

#[test]
fn environment_completion_and_option_hints_share_header_boundaries() {
    let main = Url::parse("file:///fvextra-environments/main.tex").unwrap();
    for environment in ENVIRONMENTS {
        for header in [
            "[frame=sin¦gle]",
            "[label={A]B},frame=sin¦gle]",
            "% continued\r\n[frame=sin¦gle]",
            "[frame=¦",
        ] {
            let marked = format!("% 😀\r\n\\begin{{{environment}}}{header}");
            let at = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            let mut workspace = Workspace::default();
            workspace.open(main.clone(), source.clone(), 7);
            let help = signature::help(&workspace, &main, Text::new(source).position(at))
                .unwrap_or_else(|| panic!("{marked}"));
            assert_eq!(
                help.signatures[0].label,
                format!("\\begin{{{environment}}}[options]")
            );
            assert_eq!(help.active_parameter, Some(1));
        }
        let source = format!("\\begin{{{}}}", &environment[..environment.len() - 1]);
        let mut workspace = Workspace::default();
        workspace.open(main.clone(), source.clone(), 7);
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(source.len() - 1),
        );
        assert!(items.iter().any(|item| item.label == environment));
        assert!(!items
            .iter()
            .any(|item| item.label == format!("{environment}*")));
        let source = format!("\\begin{{{environment}*}}\\label{{live}}\\end{{{environment}*}}");
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(doc.syntax.diagnostics.is_empty(), "{source}");
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
        // FancyVerb discards the rest of a real closing line.
        let source = format!("\\begin{{{environment}}}\nbody\n\\end{{{environment}}}\\label{{discarded}}\n\\label{{real}}");
        let doc = Document::new(&main, source, Some(7));
        assert_eq!(doc.syntax.labels.len(), 1);
        assert_eq!(doc.syntax.labels[0].name, "real");
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, and fvextra"]
fn real_writes_and_buffers_preserve_body_after_label_rename() {
    use std::sync::atomic::AtomicBool;
    use texquill::{compilation::build, config::BuildConfig};
    for engine in ["pdflatex", "xelatex"] {
        for environment in ENVIRONMENTS {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let first = r"\label{phantom}\ref{kept}\input{fake}";
            let body = format!("{first}\n\\end{{other}}\\end{{{environment}}}");
            let (setup, options, finish) = if environment == "VerbatimWrite" {
                (
                    r"\immediate\openout\codefile=code.txt",
                    r"[writefilehandle=\codefile]",
                    r"\immediate\closeout\codefile",
                )
            } else {
                (
                    "",
                    "[buffername=example,globalbuffer=true]",
                    r"\typeout{BUFFERCOUNT=\examplelength}\typeout{BUFFERLINE=\csname exampleline1\endcsname}",
                )
            };
            let source = format!("\\documentclass{{article}}\n\\usepackage{{fvextra}}\n\\newwrite\\codefile\n\\begin{{document}}\n\\label{{kept}}{setup}\n\\begin{{{environment}}}{options}\n{body}\n\\end{{{environment}}}\n{finish}\nText.\\ref{{kept}}\\label{{real}}\n\\end{{document}}\n");
            let mut workspace = Workspace {
                client: texquill::project::state::ClientCapabilities {
                    versioned_edits: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.open(main.clone(), source.clone(), 7);
            assert_eq!(workspace.documents[&main].syntax.labels.len(), 2);
            assert!(workspace.dependencies(&main).is_empty());
            let at = Text::new(source.clone()).position(source.rfind("kept").unwrap());
            let updated = apply(
                &source,
                features::rename(&workspace, &main, at, "changed")
                    .unwrap()
                    .unwrap(),
            );
            assert!(updated.contains(&body));
            for (content, kept, absent) in
                [(&source, "kept", "changed"), (&updated, "changed", "kept")]
            {
                std::fs::write(root.join("main.tex"), content).unwrap();
                let outcome = build::run(
                    &root.join("main.tex"),
                    &BuildConfig {
                        executable: engine.into(),
                        ..Default::default()
                    },
                    &AtomicBool::new(false),
                );
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{environment}: {}",
                    outcome.output
                );
                let aux = std::fs::read_to_string(root.join("build/texquill/main.aux")).unwrap();
                assert!(
                    aux.contains(&format!("newlabel{{{kept}}}")) && aux.contains("newlabel{real}"),
                    "{aux}"
                );
                assert!(
                    !aux.contains("newlabel{phantom}")
                        && !aux.contains(&format!("newlabel{{{absent}}}")),
                    "{aux}"
                );
                let fls = std::fs::read_to_string(root.join("build/texquill/main.fls")).unwrap();
                assert!(!fls.contains("fake.tex"));
                if environment == "VerbatimWrite" {
                    assert_eq!(
                        std::fs::read_to_string(root.join("build/texquill/code.txt"))
                            .unwrap()
                            .trim_end(),
                        body
                    );
                } else {
                    assert!(
                        outcome.output.lines().any(|line| line == "BUFFERCOUNT=2"),
                        "{}",
                        outcome.output
                    );
                    assert!(
                        outcome.output.contains(&format!("BUFFERLINE={first}")),
                        "{}",
                        outcome.output
                    );
                }
            }
        }
    }
}
