use latex_lsp::{
    build, editing, features, file_operations, navigation, semantic, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use std::{path::Path, sync::atomic::AtomicBool};
use tower_lsp::lsp_types::*;

const ENVIRONMENTS: [&str; 10] = [
    "Verbatim",
    "Verbatim*",
    "LVerbatim",
    "LVerbatim*",
    "BVerbatim",
    "BVerbatim*",
    "SaveVerbatim",
    "SaveVerbatim*",
    "VerbatimOut",
    "VerbatimOut*",
];

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn argument(environment: &str) -> &'static str {
    match environment.trim_end_matches('*') {
        "SaveVerbatim" => "{saved}",
        "VerbatimOut" => "{output.txt}",
        _ => "",
    }
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

fn versioned(edit: WorkspaceEdit, main: &Url) -> Vec<TextEdit> {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned document edits")
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *main);
    assert_eq!(documents[0].text_document.version, Some(7));
    documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!("unexpected annotated edit"),
        })
        .collect()
}

#[test]
fn literal_examples_do_not_define_symbols_import_files_or_participate_in_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), r"\label{chapterlabel}").unwrap();
    std::fs::write(root.join("example.tex"), r"\label{examplelabel}").unwrap();
    let main = uri(root, "main.tex");
    for environment in ENVIRONMENTS {
        let body = concat!(
            "\\label{kept}\\label{fake}\\newcommand{\\hidden}{Text}\\say\n",
            "\\newcounter{fakecounter}\\newenvironment{fakepanel}{}{}\n",
            "\\input{chapter}\\input{example}\\ref{kept}\n",
            "\\begin{unmatched}{ % literal unmatched brace and environment\n",
        );
        let before = "\\documentclass{article}\\newcommand{\\say}{Text}\\label{kept}\n";
        let after = "\\input{chapter}\\say\\ref{kept}\\ref{fake}\\ref{examplelabel}\n";
        let source = format!(
            "{before}\\begin{{{environment}}}{}\n{body}\\end{{{environment}}}\n{after}",
            argument(environment)
        );
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let mut workspace = Workspace {
            versioned_edits: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        workspace.open(main.clone(), source.clone(), 7);
        let doc = &workspace.documents[&main];
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{environment}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1);
        assert_eq!(doc.syntax.definitions.len(), 1);
        assert!(doc.syntax.counters.is_empty() && doc.syntax.environments.is_empty());
        assert_eq!(
            workspace.dependencies(&main),
            vec![uri(root, "chapter.tex")]
        );
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1, "{environment}: {links:?}");
        assert_eq!(links[0].target, Some(uri(root, "chapter.tex")));
        for absent in ["fake", "examplelabel"] {
            let at = doc.text.position(source.rfind(absent).unwrap());
            assert!(
                features::definitions(&workspace, &main, at).is_empty(),
                "{environment}/{absent}"
            );
        }
        for (old, new) in [("kept", "renamedlabel"), ("say", "renamedcommand")] {
            let at = doc.text.position(source.rfind(old).unwrap());
            assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
            let edit = features::rename(&workspace, &main, at, new)
                .unwrap()
                .unwrap();
            let updated = apply(&source, versioned(edit, &main));
            assert_eq!(
                updated,
                format!(
                    "{}\\begin{{{environment}}}{}\n{body}\\end{{{environment}}}\n{}",
                    before.replace(old, new),
                    argument(environment),
                    after.replace(old, new)
                )
            );
        }
        let edit = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(root, "chapter.tex").into(),
                new_uri: uri(root, "renamed.tex").into(),
            }],
        )
        .unwrap()
        .unwrap();
        let updated = apply(&source, versioned(edit, &main));
        assert!(updated.contains(body), "{environment}: {updated}");
        assert!(updated.ends_with(&after.replace("chapter", "renamed")));
        let inside = doc.text.position(source.find("\\ref{kept}").unwrap() + 7);
        assert!(features::completion(&workspace, &main, inside).is_empty());
        assert!(features::definitions(&workspace, &main, inside).is_empty());
        assert!(signature::help(&workspace, &main, inside).is_none());
    }
}

#[test]
fn raw_unicode_crlf_regions_fold_highlight_and_link_only_the_real_delimiters() {
    for environment in ENVIRONMENTS {
        let source = format!(
            "\\begin{{{environment}}}{}\r\n😀 \\label{{fake}} % literal\r\n\\begin{{fakepanel}}{{\r\n\\end{{{environment}}}\r\n\\label{{real}}\r\n",
            argument(environment)
        );
        let main = Url::parse("file:///fancy-environments/main.tex").unwrap();
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{environment}: {:?}",
            doc.syntax.diagnostics
        );
        let folds = navigation::folding_ranges(&doc, None);
        assert_eq!(folds.len(), 1, "{environment}: {folds:?}");
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 3));
        let mut position = Position::new(0, 0);
        let mut string_line = false;
        for token in semantic::tokens(&doc, None).unwrap().data {
            position = Position::new(
                position.line + token.delta_line,
                if token.delta_line == 0 {
                    position.character + token.delta_start
                } else {
                    token.delta_start
                },
            );
            if position.line == 1 {
                assert_eq!(position.character, 0);
                assert_eq!(
                    token.length,
                    "😀 \\label{fake} % literal".encode_utf16().count() as u32
                );
                assert_eq!(
                    semantic::options().legend.token_types[token.token_type as usize],
                    SemanticTokenType::STRING
                );
                string_line = true;
            }
        }
        assert!(string_line);
        for offset in [
            source.find(environment).unwrap(),
            source.rfind(environment).unwrap(),
        ] {
            let linked =
                editing::linked_editing_ranges(&doc, doc.text.position(offset + 1)).unwrap();
            assert_eq!(linked.ranges.len(), 2);
            for range in linked.ranges {
                let span =
                    doc.text.offset(range.start).unwrap()..doc.text.offset(range.end).unwrap();
                assert_eq!(&source[span], environment);
            }
        }
    }
}

#[test]
fn only_the_first_end_marker_on_a_fancyverb_line_can_close_the_environment() {
    let main = Url::parse("file:///fancy-environments/end-markers.tex").unwrap();
    for environment in ENVIRONMENTS {
        for newline in ["\n", "\r\n"] {
            let body = format!(
                "\\end{{other}}\\end{{{environment}}}\\label{{fakeafter}}{newline}\\label{{stillbody}}{newline}"
            );
            let source = format!(
                "\\begin{{{environment}}}{}{newline}{body}  \\end{{{environment}}}{newline}\\label{{real}}",
                argument(environment)
            );
            let doc = Document::new(&main, source.clone(), Some(7));
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{environment}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(
                doc.syntax.labels.len(),
                1,
                "{environment}: {:?}",
                doc.syntax.labels
            );
            assert_eq!(doc.syntax.labels[0].name, "real");
            for key in ["fakeafter", "stillbody"] {
                assert!(doc.syntax.is_ignored(source.find(key).unwrap()));
            }
            let folds = navigation::folding_ranges(&doc, None);
            assert_eq!(folds.len(), 1);
            assert_eq!((folds[0].start_line, folds[0].end_line), (0, 3));
            let at = doc.text.position(source.find(environment).unwrap());
            let linked = editing::linked_editing_ranges(&doc, at).unwrap();
            assert_eq!(linked.ranges.len(), 2);
            assert_eq!(linked.ranges[1].start.line, 3);
            let incomplete = source[..source.rfind("  \\end").unwrap()].to_owned();
            let doc = Document::new(&main, incomplete, Some(7));
            assert!(doc.syntax.labels.is_empty());
            assert_eq!(doc.syntax.diagnostics.len(), 1);
            assert_eq!(
                doc.syntax.fixes[0].edit.new_text,
                format!("\\end{{{environment}}}{newline}")
            );
        }
    }
    // Other packages retain their existing delimiter rules.
    let source = "\\begin{verbatim}\n\\end{other}\\end{verbatim}\\label{real}\n";
    let doc = Document::new(&main, source.into(), Some(7));
    assert!(doc.syntax.diagnostics.is_empty());
    assert_eq!(doc.syntax.labels[0].name, "real");
}

#[test]
fn incomplete_raw_environments_offer_one_versioned_close_fix_without_parsing_the_body() {
    let main = Url::parse("file:///fancy-environments/incomplete.tex").unwrap();
    for environment in ENVIRONMENTS {
        let source = format!(
            "\\begin{{{environment}}}{}\r\n\\label{{fake}}{{\\begin{{fake}}\r\n",
            argument(environment)
        );
        let mut workspace = Workspace {
            versioned_edits: true,
            code_action_literals: true,
            ..Default::default()
        };
        workspace.open(main.clone(), source.clone(), 7);
        let doc = &workspace.documents[&main];
        assert!(doc.syntax.labels.is_empty());
        assert_eq!(
            doc.syntax.diagnostics.len(),
            1,
            "{environment}: {:?}",
            doc.syntax.diagnostics
        );
        let diagnostic = doc.syntax.diagnostics[0].clone();
        assert_eq!(
            diagnostic.message,
            format!("Unclosed environment \u{60}{environment}\u{60}")
        );
        let actions = editing::code_actions(
            &workspace,
            &main,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                ..Default::default()
            },
        );
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(action) = actions.into_iter().next().unwrap() else {
            panic!()
        };
        let updated = apply(&source, versioned(action.edit.unwrap(), &main));
        assert_eq!(updated, format!("{source}\\end{{{environment}}}\r\n"));
        let repaired = Document::new(&main, updated, Some(8));
        assert!(repaired.syntax.diagnostics.is_empty());
        assert!(repaired.syntax.labels.is_empty());
    }
}

#[test]
fn environment_names_require_exact_case_spaces_and_one_star_and_completion_reserves_them() {
    let main = Url::parse("file:///fancy-environments/names.tex").unwrap();
    for name in [
        " Verbatim",
        "Verbatim ",
        "lVerbatim",
        "LVerbatim**",
        "BVerbatimX",
    ] {
        let source = format!("\\begin{{{name}}}\n\\label{{visible}}\n\\end{{{name}}}\n");
        let doc = Document::new(&main, source, Some(1));
        assert_eq!(doc.syntax.labels.len(), 1, "{name}");
    }
    let mut workspace = Workspace::default();
    for environment in ENVIRONMENTS {
        let source = format!("\\begin{{{environment}}}");
        workspace.open(main.clone(), source.clone(), 1);
        let at = workspace.documents[&main].text.position(source.len() - 1);
        assert!(
            features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.label == environment),
            "{environment}"
        );
    }
    let source = "\\newenvironment{panel}{}{}\\begin{panel}Text.\\end{panel}";
    workspace.open(main.clone(), source.into(), 2);
    let at = workspace.documents[&main]
        .text
        .position(source.find("panel").unwrap());
    for environment in ENVIRONMENTS {
        assert!(
            features::rename(&workspace, &main, at, environment).is_err(),
            "{environment}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_fancyverb_environments_keep_example_labels_literal_before_and_after_renaming() {
    for engine in ["pdflatex", "xelatex"] {
        for environment in ENVIRONMENTS {
            for foreign_end in [false, true] {
                let prefix = if foreign_end {
                    format!("\\end{{other}}\\end{{{environment}}}\\label{{fakeafter}}\n")
                } else {
                    String::new()
                };
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = uri(root, "main.tex");
                let use_saved = if environment.starts_with("SaveVerbatim") {
                    "\\UseVerbatim{saved}\n"
                } else {
                    ""
                };
                let source = format!(
                "\\documentclass{{article}}\n\\usepackage{{fancyvrb}}\n\\begin{{document}}\n\\label{{kept}}\n\\begin{{{environment}}}{}\n{prefix}\\label{{kept}}\\label{{fake}}\\newcommand{{\\hidden}}{{Text}}\n\\end{{{environment}}}\n{use_saved}\\ref{{kept}}\n\\end{{document}}\n",
                argument(environment)
            );
                std::fs::write(root.join("main.tex"), &source).unwrap();
                let mut workspace = Workspace::default();
                workspace.add_root(root.into());
                workspace.config.build.executable = engine.into();
                let position = workspace.documents[&main]
                    .text
                    .position(source.rfind("kept").unwrap());
                let changes = features::rename(&workspace, &main, position, "renamedlabel")
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                let updated = apply(&source, changes[&main].clone());
                assert!(updated.contains("\\label{kept}\\label{fake}\\newcommand{\\hidden}{Text}"));
                for (contents, key) in [(&source, "kept"), (&updated, "renamedlabel")] {
                    std::fs::write(root.join("main.tex"), contents).unwrap();
                    workspace.open(main.clone(), contents.clone(), 1);
                    let outcome = build::run(
                        &root.join("main.tex"),
                        &workspace.config.build,
                        &AtomicBool::new(false),
                    );
                    assert_eq!(
                        outcome.status, "success",
                        "{engine}/{environment}: {}",
                        outcome.output
                    );
                    assert!(outcome
                        .diagnostics
                        .values()
                        .flatten()
                        .all(|diagnostic| diagnostic.severity != Some(DiagnosticSeverity::ERROR)));
                    let aux =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                    assert!(aux.contains(&format!("newlabel{{{key}}}")), "{aux}");
                    assert!(!aux.contains("newlabel{fake}"), "{aux}");
                    assert!(!aux.contains("newlabel{fakeafter}"), "{aux}");
                    if key == "renamedlabel" {
                        assert!(!aux.contains("newlabel{kept}"), "{aux}");
                    }
                    if environment.starts_with("VerbatimOut") {
                        let output =
                            std::fs::read_to_string(root.join("build/latex-lsp/output.txt"))
                                .unwrap();
                        assert!(output.contains("\\label{kept}\\label{fake}"), "{output}");
                    }
                }
            }
        }
    }
}
