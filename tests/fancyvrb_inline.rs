use latex_lsp::{
    build, features, file_operations, navigation, semantic, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use std::sync::atomic::AtomicBool;
use tower_lsp::lsp_types::*;

fn apply(source: &str, edit: WorkspaceEdit, uri: &Url) -> String {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edits")
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *uri);
    assert_eq!(documents[0].text_document.version, Some(7));
    let mut edits: Vec<_> = documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!("unexpected annotated edit"),
        })
        .collect();
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
fn literal_bodies_do_not_import_symbols_files_or_rename_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), r"\label{chapterlabel}").unwrap();
    std::fs::write(root.join("fake.tex"), r"\label{fakelabel}").unwrap();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let body = concat!(
        r"\label{kept}\label{phantom}\newcommand{\hidden}{Oops}\say",
        r"\input{chapter}\input{fake}\ref{kept}\begin{unmatched}{% 😀",
    );
    for command in ["Verb", "SaveVerb", "SaveGVerb"] {
        for header in [
            "",
            "*",
            " *[showspaces=true]",
            "% header\r\n*[showspaces=true]% name\r\n",
        ] {
            for delimiter in ['|', '!', 'Q', '§', '😀'] {
                // The delimiter itself cannot occur in the literal body.
                let body = body.replace(delimiter, "");
                let name = if command != "Verb" { "{saved}" } else { "" };
                let before = "\\documentclass{article}\\newcommand{\\say}{Text}\\label{kept}\r\n";
                let after = "\\input{chapter}\\say\\ref{kept}\\ref{phantom}\\ref{fakelabel}\r\n";
                let literal = format!("\\{command}{header}{name} {delimiter}{body}{delimiter}");
                let source = format!("{before}{literal}\r\n{after}");
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
                    "{literal}: {:?}",
                    doc.syntax.diagnostics
                );
                assert_eq!(doc.syntax.labels.len(), 1, "{literal}");
                assert_eq!(doc.syntax.definitions.len(), 1, "{literal}");
                let chapter = Url::from_file_path(root.join("chapter.tex")).unwrap();
                assert_eq!(workspace.dependencies(&main), vec![chapter.clone()]);
                assert_eq!(navigation::document_links(&workspace, &main).len(), 1);
                for key in ["phantom", "fakelabel"] {
                    assert!(
                        features::definitions(
                            &workspace,
                            &main,
                            doc.text.position(source.rfind(key).unwrap())
                        )
                        .is_empty(),
                        "{command}/{header}/{delimiter}/{key}"
                    );
                }
                for (old, new) in [("kept", "renamedlabel"), ("say", "renamedcommand")] {
                    let at = doc.text.position(source.rfind(old).unwrap());
                    assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
                    let edit = features::rename(&workspace, &main, at, new)
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        apply(&source, edit, &main),
                        format!(
                            "{}{literal}\r\n{}",
                            before.replace(old, new),
                            after.replace(old, new)
                        )
                    );
                }
                let renamed = file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: chapter.into(),
                        new_uri: Url::from_file_path(root.join("renamed.tex"))
                            .unwrap()
                            .into(),
                    }],
                )
                .unwrap()
                .unwrap();
                let updated = apply(&source, renamed, &main);
                assert!(updated.contains(&literal));
                assert!(updated.ends_with(&after.replace("chapter", "renamed")));
                let inside = doc.text.position(source.find("\\ref{kept}").unwrap() + 7);
                assert!(features::completion(&workspace, &main, inside).is_empty());
                assert!(features::definitions(&workspace, &main, inside).is_empty());
                assert!(signature::help(&workspace, &main, inside).is_none());
            }
        }
    }
}

#[test]
fn names_are_data_but_formatting_and_aftersave_commands_are_live() {
    for name in ["{saved\\detokenize{\\end{fake}}}", "x", "\\slot"] {
        let source = format!(
            "\\SaveVerb *[aftersave={{\\label{{active}}\\newcommand{{\\visible}}{{Text}}}}]{name}% delimiter\r\n|\\label{{phantom}}|\\UseVerb[formatcom={{\\visible}}]{name}\\label{{real}}"
        );
        let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(
            doc.syntax
                .labels
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["active", "real"]
        );
        assert_eq!(doc.syntax.definitions.len(), 1);
        assert_eq!(doc.syntax.definitions[0].name, "visible");
        assert_eq!(
            doc.syntax
                .commands
                .iter()
                .filter(|command| command.name == "visible")
                .count(),
            2
        );
        assert!(!doc
            .syntax
            .commands
            .iter()
            .any(|command| matches!(command.name.as_str(), "slot" | "detokenize" | "end")));
        for command in doc
            .syntax
            .commands
            .iter()
            .filter(|command| matches!(command.name.as_str(), "SaveVerb" | "UseVerb"))
        {
            assert_eq!(
                command.argument.as_ref().unwrap().value,
                name.trim_start_matches('{')
                    .strip_suffix('}')
                    .unwrap_or(name)
            );
        }
    }
    let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
    let doc = Document::new(
        &main,
        r"\Verb[formatcom={\label{active}}]|\label{phantom}|\label{real}".into(),
        Some(7),
    );
    assert_eq!(
        doc.syntax
            .labels
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["active", "real"]
    );
}

#[test]
fn global_saved_names_are_data_while_moving_use_options_remain_source() {
    let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
    for used in ["UseMVerb", "pUseMVerb"] {
        for name in ["{saved\\detokenize{\\end{fake}}}", " x", "\\slot"] {
            let body = r"\label{active}\label{phantom}\newcommand{\hidden}{Oops}\input{example}";
            let source = format!("\\SaveGVerb *[showspaces=true]% name\r\n{name}{{|}}{body}|\\{used} *[formatcom={{\\label{{active}}\\newcommand{{\\visible}}{{Text}}}}]{name}\\ref{{active}}");
            let mut workspace = Workspace {
                versioned_edits: true,
                ..Default::default()
            };
            workspace.open(main.clone(), source.clone(), 7);
            let doc = &workspace.documents[&main];
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{source}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(doc.syntax.labels.len(), 1, "{source}");
            assert_eq!(doc.syntax.labels[0].name, "active");
            assert_eq!(doc.syntax.definitions.len(), 1);
            assert_eq!(doc.syntax.definitions[0].name, "visible");
            assert!(workspace.dependencies(&main).is_empty());
            assert!(!doc.syntax.commands.iter().any(|command| matches!(
                command.name.as_str(),
                "slot" | "detokenize" | "end" | "input"
            )));
            let at = doc.text.position(source.rfind("active").unwrap());
            assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
            let updated = apply(
                &source,
                features::rename(&workspace, &main, at, "renamedlabel")
                    .unwrap()
                    .unwrap(),
                &main,
            );
            assert!(updated.contains(body) && updated.contains(name));
            assert_eq!(updated.matches("renamedlabel").count(), 2);
        }
    }
}

#[test]
fn unclosed_bodies_recover_at_line_end_or_eof_with_valid_utf16_highlighting() {
    for command in ["Verb", "SaveVerb{saved}", "SaveGVerb{saved}"] {
        for newline in ["\n", "\r\n"] {
            for delimiter in ['|', '§', '😀'] {
                for closed in [false, true] {
                    let tail = if closed {
                        delimiter.to_string()
                    } else {
                        String::new()
                    };
                    let raw = format!("{delimiter}中 \\label{{phantom}} % }}{tail}");
                    let source = format!("\\{command}{raw}{newline}\\label{{real}}");
                    let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
                    let doc = Document::new(&main, source.clone(), Some(7));
                    assert!(
                        doc.syntax.diagnostics.is_empty(),
                        "{source}: {:?}",
                        doc.syntax.diagnostics
                    );
                    assert_eq!(doc.syntax.labels.len(), 1);
                    assert_eq!(doc.syntax.labels[0].name, "real");
                    let mut at = Position::new(0, 0);
                    let mut found = false;
                    for token in semantic::tokens(&doc, None).unwrap().data {
                        at = Position::new(
                            at.line + token.delta_line,
                            if token.delta_line == 0 {
                                at.character + token.delta_start
                            } else {
                                token.delta_start
                            },
                        );
                        if at.line == 0
                            && at.character
                                == doc.text.position(source.find(delimiter).unwrap()).character
                        {
                            assert_eq!(token.length, raw.encode_utf16().count() as u32);
                            assert_eq!(
                                semantic::options().legend.token_types[token.token_type as usize],
                                SemanticTokenType::STRING
                            );
                            found = true;
                        }
                    }
                    assert!(found, "{source}");
                    let eof = Document::new(&main, format!("\\{command}{raw}"), Some(7));
                    assert!(eof.syntax.labels.is_empty() && eof.syntax.diagnostics.is_empty());
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_global_saved_bodies_and_moving_use_options_survive_versioned_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for used in ["UseMVerb", "pUseMVerb"] {
            for form in [
                "basic",
                "star",
                "comment",
                "token",
                "grouped-delimiter",
                "saved-name",
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = Url::from_file_path(root.join("main.tex")).unwrap();
                let header = match form {
                    "star" => " *",
                    "comment" => "% header\r\n*[showspaces=true]% name\r\n",
                    _ => "",
                };
                let name = match form {
                    "token" => " x",
                    "saved-name" => "{saved\\detokenize{\\end{fake}}}",
                    _ => "{saved}",
                };
                let delimiter = if form == "grouped-delimiter" {
                    "{|}"
                } else {
                    "|"
                };
                let body = r"\label{kept}\label{phantom}\newcommand{\hidden}{Oops}\input{example}";
                let source = format!("\\documentclass{{article}}\r\n\\usepackage{{fancyvrb}}\r\n\\begin{{document}}\r\n\\label{{kept}}\\SaveGVerb{header}{name}{delimiter}{body}|\\{used} *[formatcom={{\\label{{active}}}}]{name}\r\nReal.\\ref{{kept}}\\end{{document}}\r\n");
                let mut workspace = Workspace {
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace.open(main.clone(), source.clone(), 7);
                let doc = &workspace.documents[&main];
                assert!(
                    doc.syntax.diagnostics.is_empty(),
                    "{used}/{form}: {:?}",
                    doc.syntax.diagnostics
                );
                assert_eq!(doc.syntax.labels.len(), 2);
                assert!(
                    doc.syntax.definitions.is_empty() && workspace.dependencies(&main).is_empty()
                );
                let at = doc.text.position(source.rfind("kept").unwrap());
                let updated = apply(
                    &source,
                    features::rename(&workspace, &main, at, "renamedlabel")
                        .unwrap()
                        .unwrap(),
                    &main,
                );
                assert!(updated.contains(body) && updated.contains(name));
                for (content, expected, absent) in [
                    (&source, "kept", "renamedlabel"),
                    (&updated, "renamedlabel", "kept"),
                ] {
                    std::fs::write(root.join("main.tex"), content).unwrap();
                    let outcome = build::run(
                        &root.join("main.tex"),
                        &latex_lsp::config::BuildConfig {
                            executable: engine.into(),
                            ..Default::default()
                        },
                        &AtomicBool::new(false),
                    );
                    assert_eq!(
                        outcome.status, "success",
                        "{engine}/{used}/{form}: {}",
                        outcome.output
                    );
                    let aux =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                    assert!(
                        aux.contains(&format!("newlabel{{{expected}}}"))
                            && aux.contains("newlabel{active}"),
                        "{aux}"
                    );
                    for key in [absent, "phantom"] {
                        assert!(!aux.contains(&format!("newlabel{{{key}}}")), "{aux}");
                    }
                }
            }
        }
    }
}

#[test]
fn ordinary_and_grouped_delimiters_end_at_the_first_literal_character() {
    let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
    for command in ["Verb", "SaveVerb{saved}", "SaveGVerb{saved}"] {
        for (opening, closing) in [
            ("|", "|"),
            ("{|}", "|"),
            ("#", "#"),
            ("Q", "Q"),
            ("{§}", "§"),
        ] {
            let source =
                format!("\\{command} {opening}\\label{{phantom}}\\{closing}\\label{{real}}");
            let doc = Document::new(&main, source.clone(), Some(7));
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{source}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(doc.syntax.labels.len(), 1, "{source}");
            assert_eq!(doc.syntax.labels[0].name, "real");
        }
    }
    // Only one options argument is consumed. The second `[` is the delimiter.
    let doc = Document::new(
        &main,
        r"\Verb[showspaces=true][\label{phantom}[\label{real}".into(),
        Some(7),
    );
    assert!(doc.syntax.diagnostics.is_empty());
    assert_eq!(doc.syntax.labels.len(), 1);
    assert_eq!(doc.syntax.labels[0].name, "real");
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_inline_bodies_preserve_literal_text_and_executable_options_after_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for command in ["Verb", "SaveVerb"] {
            for form in [
                "basic",
                "star",
                "options",
                "comment",
                "token",
                "grouped-delimiter",
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = Url::from_file_path(root.join("main.tex")).unwrap();
                let key = if command == "SaveVerb" {
                    "aftersave"
                } else {
                    "formatcom"
                };
                let options = format!("[{key}={{\\label{{active}}}}]");
                let header = match form {
                    "star" => " *".into(),
                    "options" => options.clone(),
                    "comment" => format!("% header\r\n*{options}% argument\r\n"),
                    _ => String::new(),
                };
                let name = if command == "SaveVerb" {
                    if form == "token" {
                        " x"
                    } else {
                        "{saved}"
                    }
                } else {
                    ""
                };
                let used = if command == "SaveVerb" {
                    if form == "token" {
                        "\\UseVerb x"
                    } else {
                        "\\UseVerb{saved}"
                    }
                } else {
                    ""
                };
                let body = r"\label{kept}\label{phantom}\newcommand{\hidden}{Oops}\input{fake}";
                let delimiter = if form == "grouped-delimiter" {
                    "{|}"
                } else {
                    "|"
                };
                let source = format!("\\documentclass{{article}}\r\n\\usepackage{{fancyvrb}}\r\n\\begin{{document}}\r\n\\label{{kept}}\\{command}{header}{name}{delimiter}{body}|{used}\r\nReal.\\ref{{kept}}\\end{{document}}\r\n");
                let mut workspace = Workspace {
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace.open(main.clone(), source.clone(), 7);
                let doc = &workspace.documents[&main];
                assert!(doc.syntax.diagnostics.is_empty(), "{command}/{form}");
                assert_eq!(
                    doc.syntax.labels.len(),
                    if matches!(form, "options" | "comment") {
                        2
                    } else {
                        1
                    }
                );
                let at = doc.text.position(source.rfind("kept").unwrap());
                let updated = apply(
                    &source,
                    features::rename(&workspace, &main, at, "renamedlabel")
                        .unwrap()
                        .unwrap(),
                    &main,
                );
                assert!(updated.contains(body));
                for (content, expected, absent) in [
                    (&source, "kept", "renamedlabel"),
                    (&updated, "renamedlabel", "kept"),
                ] {
                    std::fs::write(root.join("main.tex"), content).unwrap();
                    let outcome = build::run(
                        &root.join("main.tex"),
                        &latex_lsp::config::BuildConfig {
                            executable: engine.into(),
                            ..Default::default()
                        },
                        &AtomicBool::new(false),
                    );
                    assert_eq!(
                        outcome.status, "success",
                        "{engine}/{command}/{form}: {}",
                        outcome.output
                    );
                    let aux =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                    assert!(aux.contains(&format!("newlabel{{{expected}}}")), "{aux}");
                    for key in [absent, "phantom"] {
                        assert!(!aux.contains(&format!("newlabel{{{key}}}")), "{aux}");
                    }
                    assert_eq!(
                        aux.contains("newlabel{active}"),
                        matches!(form, "options" | "comment"),
                        "{aux}"
                    );
                }
            }
        }
    }
}
