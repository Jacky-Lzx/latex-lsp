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
                assert_eq!(
                    navigation::document_links(&workspace, &main).len(),
                    1,
                    "{source}"
                );
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

#[test]
fn collected_verb_bodies_keep_executable_commands_and_file_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), r"\label{chapterlabel}").unwrap();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    for body in [
        r"|\label{kept}\say\input{chapter}|",
        r"|a{b|\label{kept}\say\input{chapter}}d|",
        "|a% ignored |\\label{phantom}\r\n\\label{kept}\\say\\input{chapter}b|",
        r"{|}\label{kept}\say\input{chapter}|",
        r"*[showspaces=true]|\label{kept}\say\input{chapter}|",
    ] {
        for (before, after) in [
            (r"\textbf{", "}"),
            (r"\let\strong\textbf\strong{", "}"),
            (r"\newcommand{\one}[1]{#1}\one{", "}"),
            (r"\def\one#1;{#1}\one", ";"),
            (r"\def\example{", r"}\example"),
        ] {
            let source = format!(
                "\\newcommand{{\\say}}{{Text}}{before}\\Verb{body}{after}\\ref{{kept}}\\say"
            );
            let mut workspace = Workspace {
                versioned_edits: true,
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
            assert_eq!(doc.syntax.labels[0].name, "kept");
            let chapter = Url::from_file_path(root.join("chapter.tex")).unwrap();
            assert_eq!(
                workspace.dependencies(&main),
                vec![chapter.clone()],
                "{source}"
            );
            if before == r"\textbf{" {
                assert_eq!(
                    navigation::document_links(&workspace, &main).len(),
                    1,
                    "{source}"
                );
            }
            let at = doc.text.position(source.rfind("kept").unwrap());
            assert_eq!(features::definitions(&workspace, &main, at).len(), 1);
            for (old, new) in [("kept", "changed"), ("say", "speak")] {
                let at = doc.text.position(source.rfind(old).unwrap());
                let updated = apply(
                    &source,
                    features::rename(&workspace, &main, at, new)
                        .unwrap()
                        .unwrap(),
                    &main,
                );
                assert_eq!(updated, source.replace(old, new), "{source}");
            }
            if before != r"\textbf{" {
                continue;
            }
            let edit = file_operations::will_rename(
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
            assert_eq!(
                apply(&source, edit, &main),
                source.replace("{chapter}", "{renamed}")
            );
        }
    }
}

#[test]
fn scopes_and_partial_macro_operands_leave_verb_body_literal() {
    for (before, after) in [
        ("{", "}"),
        (r"\relax{", "}"),
        (r"\newcommand{\zero}{}\zero{", "}"),
        (r"\newcommand{\one}[1]{#1}\one", ""),
        (r"\newcommand{\two}[2]{#1#2}\two", ""),
        (r"\newcommand{\gobble}[1]{}\gobble\textbf{", "}"),
    ] {
        let source =
            format!("{before}\\Verb|\\label{{phantom}}\\input{{fake}}|{after}\\label{{real}}");
        let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
        assert_eq!(doc.syntax.labels[0].name, "real");
        assert!(
            !doc.syntax
                .commands
                .iter()
                .any(|command| command.name == "input"),
            "{source}"
        );
    }
}

#[test]
fn redefined_fancyverb_commands_keep_their_ordinary_argument_commands() {
    for name in [
        "Verb",
        "SaveVerb",
        "SaveGVerb",
        "UseVerb",
        "UseVerbatim",
        "UseMVerb",
    ] {
        let source =
            format!("\\renewcommand{{\\{name}}}[1]{{#1}}\\{name}{{\\label{{live}}}}\\ref{{live}}");
        let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(doc.syntax.diagnostics.is_empty(), "{source}");
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_collected_verb_executes_body_commands_before_and_after_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for body in [
            r"|\typeout{LIVEBODY}\label{kept}\input{chapter}|",
            r"|a{b|\typeout{LIVEBODY}\label{kept}\input{chapter}}d|",
            "|a% ignored |\\label{phantom}\n\\typeout{LIVEBODY}\\label{kept}\\input{chapter}b|",
            r"{|}\typeout{LIVEBODY}\label{kept}\input{chapter}|",
            r"*[showspaces=true]|\typeout{LIVEBODY}\label{kept}\input{chapter}|",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            std::fs::write(root.join("chapter.tex"), "Chapter text").unwrap();
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let source = format!("\\documentclass{{article}}\n\\usepackage{{fancyvrb}}\n\\begin{{document}}\n\\textbf{{\\Verb{body}}}\\ref{{kept}}\n\\end{{document}}\n");
            let mut workspace = Workspace {
                versioned_edits: true,
                ..Default::default()
            };
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            let at = workspace.documents[&main]
                .text
                .position(source.rfind("kept").unwrap());
            let updated = apply(
                &source,
                features::rename(&workspace, &main, at, "changed")
                    .unwrap()
                    .unwrap(),
                &main,
            );
            assert_eq!(updated, source.replace("kept", "changed"));
            for (content, expected, absent) in
                [(&source, "kept", "changed"), (&updated, "changed", "kept")]
            {
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
                    "{engine}/{body}: {}",
                    outcome.output
                );
                assert!(outcome.output.lines().any(|line| line == "LIVEBODY"));
                let aux = std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                assert!(aux.contains(&format!("newlabel{{{expected}}}")), "{aux}");
                assert!(!aux.contains(&format!("newlabel{{{absent}}}")), "{aux}");
                assert!(!aux.contains("newlabel{phantom}"), "{aux}");
                let fls = std::fs::read_to_string(root.join("build/latex-lsp/main.fls")).unwrap();
                assert!(fls.contains("chapter.tex"), "{fls}");
            }
        }
    }
}

#[test]
fn fvextra_and_minted_loaders_keep_inline_code_opaque_with_scoped_extra_settings() {
    let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
    for package in [
        r"\usepackage{fvextra}",
        r"\RequirePackage{fvextra}",
        r"\usepackage[verbatim]{minted}",
        r"\usepackage{fancyvrb,fvextra}",
        "\\usepackage{fancyvrb,% package\r\nfvextra}",
    ] {
        for code in [
            r"\Verb{\label{phantom}\input{fake}}",
            r"\textbf{\Verb|a{b|\label{phantom}\input{fake}}d|}",
            r"\textbf{\Verb{a\}\label{phantom}\input{fake}b}}",
            "\\Verb|a\r\n\\label{phantom}\\input{fake}|",
            r"\SaveVerb{saved}{\label{phantom}\input{fake}}\UseVerb{saved}",
        ] {
            let source = format!("{package}{code}\\label{{real}}");
            let doc = Document::new(&main, source.clone(), Some(7));
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{source}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(doc.syntax.labels.len(), 1, "{source}");
            assert_eq!(doc.syntax.labels[0].name, "real");
        }
        for (settings, expected) in [
            ("", vec![]),
            (r"\fvset{extra=false}", vec!["first", "second"]),
            (r"\fvinlineset{extra=false}", vec!["first", "second"]),
            (r"\fvset{extra=false}\fvinlineset{extra=true}", vec![]),
            (
                "\\fvset{extra=false}\\fvinlineset{extra% key\r\n=true}",
                vec![],
            ),
        ] {
            let source = format!("{package}{settings}\\textbf{{\\Verb|\\label{{first}}|}}\\textbf{{\\Verb|\\label{{second}}|}}");
            let doc = Document::new(&main, source.clone(), Some(7));
            assert!(doc.syntax.diagnostics.is_empty(), "{source}");
            assert_eq!(
                doc.syntax
                    .labels
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{source}"
            );
        }
        for scoped in [
            r"{\fvset{extra=false}\textbf{\Verb|\label{live}|}}",
            r"\begingroup\fvset{extra=false}\textbf{\Verb|\label{live}|}\endgroup",
            r"{\fvinlineset{extra=false}\textbf{\Verb|\label{live}|}}",
            r"\textbf{\Verb[extra={false}]|\label{live}|}",
        ] {
            let source =
                format!("{package}{scoped}\\textbf{{\\Verb|\\label{{phantom}}|}}\\label{{real}}");
            let doc = Document::new(&main, source.clone(), Some(7));
            assert!(doc.syntax.diagnostics.is_empty(), "{source}");
            assert_eq!(
                doc.syntax
                    .labels
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                ["live", "real"],
                "{source}"
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, fvextra, and minted v3"]
fn real_fvextra_verb_defaults_and_extra_false_preserve_native_body_execution() {
    for engine in ["pdflatex", "xelatex"] {
        for package in ["fvextra", "minted"] {
            for (settings, options, live) in [
                ("", "", false),
                ("", "[extra=false]", true),
                (r"\fvset{extra=false}", "", true),
                (r"\fvinlineset{extra=false}", "", true),
                (r"\fvset{extra=false}\fvinlineset{extra=true}", "", false),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!("\\documentclass{{article}}\n\\usepackage{}{{{package}}}\n{settings}\n\\begin{{document}}\n\\textbf{{\\Verb{options}|a{{b|\\typeout{{LIVEBODY}}\\label{{inside}}}}d|}}\\label{{real}}\n\\end{{document}}\n", if package == "minted" { "[verbatim]" } else { "" });
                let main = Url::from_file_path(root.join("main.tex")).unwrap();
                let doc = Document::new(&main, source.clone(), Some(7));
                assert!(doc.syntax.diagnostics.is_empty(), "{source}");
                assert_eq!(doc.syntax.labels.len(), 1 + usize::from(live), "{source}");
                std::fs::write(root.join("main.tex"), source).unwrap();
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
                    "{engine}/{package}/{settings}/{options}: {}",
                    outcome.output
                );
                assert_eq!(
                    outcome.output.lines().any(|line| line == "LIVEBODY"),
                    live,
                    "{}",
                    outcome.output
                );
                let aux = std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                assert_eq!(aux.contains("newlabel{inside}"), live, "{aux}");
                assert!(aux.contains("newlabel{real}"), "{aux}");
            }
        }
    }
}

#[test]
fn static_verb_aliases_preserve_reader_modes_and_editable_command_identity() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), "Chapter text").unwrap();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    for (package, live) in [
        (r"\usepackage{fancyvrb}", true),
        (r"\usepackage{fvextra}", false),
        (r"\usepackage{fvextra}\fvset{extra=false}", true),
    ] {
        for declaration in [
            r"\let\code\Verb",
            r"\let\first\Verb\let\code\first",
            r"\NewCommandCopy{\code}{\Verb}",
            r"\DeclareCommandCopy{\code}{\Verb}",
            r"\newcommand{\code}{\Verb}",
            r"\def\code{\Verb}",
            r"\newcommand{\forward}{\Verb}\let\code\forward",
            r"\newcommand{\code}{\forward}\newcommand{\forward}{\Verb}",
        ] {
            let before = format!(
                "{package}\\newcommand{{\\say}}{{Text}}{declaration}\\code[formatcom={{\\say}}]"
            );
            let literal = r"|\label{phantom}\input{fake}\code\say|";
            let middle = r"\textbf{\code";
            let body = r"|a{b|\label{live}\say\input{chapter}}d|";
            let after = r"}\ref{live}\code|tail|";
            let source = format!("{before}{literal}{middle}{body}{after}");
            let mut workspace = Workspace {
                versioned_edits: true,
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
            assert_eq!(doc.syntax.labels.len(), usize::from(live), "{source}");
            let dependencies = workspace.dependencies(&main);
            assert_eq!(
                dependencies,
                if live {
                    vec![Url::from_file_path(root.join("chapter.tex")).unwrap()]
                } else {
                    vec![]
                },
                "{source}"
            );
            let at = doc.text.position(source.rfind("code").unwrap() + 1);
            assert_eq!(
                features::definitions(&workspace, &main, at).len(),
                1,
                "{source}"
            );
            let updated = apply(
                &source,
                features::rename(&workspace, &main, at, "alias")
                    .unwrap()
                    .unwrap(),
                &main,
            );
            assert_eq!(
                updated,
                format!(
                    "{}{literal}{}{body}{}",
                    before.replace("code", "alias"),
                    middle.replace("code", "alias"),
                    after.replace("code", "alias")
                ),
                "{source}"
            );
            let at = doc.text.position(source.find("say").unwrap());
            let updated = apply(
                &source,
                features::rename(&workspace, &main, at, "speak")
                    .unwrap()
                    .unwrap(),
                &main,
            );
            assert_eq!(
                updated,
                format!(
                    "{}{literal}{middle}{}{after}",
                    before.replace("say", "speak"),
                    if live {
                        body.replace("say", "speak")
                    } else {
                        body.into()
                    }
                ),
                "{source}"
            );
            if live {
                let at = doc.text.position(source.rfind("live").unwrap());
                let updated = apply(
                    &source,
                    features::rename(&workspace, &main, at, "changed")
                        .unwrap()
                        .unwrap(),
                    &main,
                );
                assert_eq!(updated, source.replace("live", "changed"));
            }
        }
    }
}

#[test]
fn ambiguous_parameterized_and_consumed_verb_alias_declarations_keep_ordinary_grammar() {
    let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
    for declaration in [
        r"\newcommand{\code}[1]{#1}",
        r"\newcommand{\code}[1]{\Verb}",
        r"\newcommand{\code}{\unknown}",
        r"\newcommand{\code}{\forward}\newcommand{\forward}{\code}",
        r"\let\code\unknown",
        r"\let\code\later\newcommand{\later}{\Verb}",
        r"\let\code\Verb\renewcommand{\code}[1]{#1}",
        r"\newcommand{\gobble}[1]{}\gobble\let\code\Verb",
    ] {
        let source = format!("{declaration}\\code{{\\label{{live}}}}\\ref{{live}}");
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, and fvextra"]
fn real_static_verb_aliases_keep_literal_names_and_native_body_execution_after_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for (package, settings, live) in [
            ("fancyvrb", "", true),
            ("fvextra", "", false),
            ("fvextra", r"\fvset{extra=false}", true),
        ] {
            for declaration in [
                r"\let\code\Verb",
                r"\newcommand{\code}{\Verb}",
                r"\NewCommandCopy{\code}{\Verb}",
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                std::fs::write(root.join("chapter.tex"), "Chapter text").unwrap();
                let main = Url::from_file_path(root.join("main.tex")).unwrap();
                let literal = r"|\label{phantom}\input{fake}\code|";
                let source = format!("\\documentclass{{article}}\n\\usepackage{{{package}}}\n{settings}{declaration}\n\\begin{{document}}\n\\code{literal}\n\\textbf{{\\code|a{{b|\\typeout{{LIVEBODY}}\\label{{live}}\\input{{chapter}}}}d|}}\\label{{real}}\\code|tail|\n\\end{{document}}\n");
                let mut workspace = Workspace {
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace.open(main.clone(), source.clone(), 7);
                let doc = &workspace.documents[&main];
                assert!(doc.syntax.diagnostics.is_empty(), "{source}");
                let at = doc.text.position(source.rfind("code").unwrap());
                let updated = apply(
                    &source,
                    features::rename(&workspace, &main, at, "snippet")
                        .unwrap()
                        .unwrap(),
                    &main,
                );
                assert!(updated.contains(literal), "{updated}");
                assert_eq!(updated.matches("\\snippet").count(), 4, "{updated}");
                for content in [&source, &updated] {
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
                        "{engine}/{package}/{settings}/{declaration}: {}",
                        outcome.output
                    );
                    assert_eq!(
                        outcome.output.lines().any(|line| line == "LIVEBODY"),
                        live,
                        "{}",
                        outcome.output
                    );
                    let aux =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                    assert_eq!(aux.contains("newlabel{live}"), live, "{aux}");
                    assert!(aux.contains("newlabel{real}"), "{aux}");
                    assert!(!aux.contains("newlabel{phantom}"), "{aux}");
                    let fls =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.fls")).unwrap();
                    assert_eq!(fls.contains("chapter.tex"), live, "{fls}");
                }
            }
        }
    }
}

#[test]
fn verb_aliases_replace_reserved_command_headers_and_symbol_operations() {
    let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
    for name in ["lstinline", "mintinline", "section", "input"] {
        let source = format!(
            "\\let\\{name}\\Verb\\{name}{{|}}\\label{{phantom}}\\input{{fake}}|\\label{{real}}"
        );
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
        assert_eq!(doc.syntax.labels[0].name, "real");
        assert!(
            doc.syntax
                .commands
                .iter()
                .filter(|command| command.name == name)
                .all(|command| command.argument.is_none()),
            "{source}"
        );
    }
}

#[test]
fn fvextra_raw_header_peeks_preserve_special_delimiters_and_following_source() {
    let main = Url::parse("file:///fancy-inline/main.tex").unwrap();
    for (settings, options) in [
        ("", ""),
        ("", "*[showspaces=true]"),
        (r"\fvset{extra=false}", ""),
        (r"\fvset{extra=false}", "*"),
        (r"\fvset{extra=false}", "[extra=true]"),
    ] {
        for body in [
            r"%\label{phantom}\input{fake}%",
            " v%line one\r\n[extra=false]\\label{phantom}v",
            r"\label{phantom}\ ",
        ] {
            if body.contains('\n') && !settings.is_empty() && options != "[extra=true]" {
                continue;
            }
            for declaration in ["", r"\let\code\Verb", r"\newcommand{\code}{\Verb}"] {
                // A literal % within a v-delimited body cannot alter the
                // actual header options or expose the following code tokens.
                let command = if declaration.is_empty() {
                    "Verb"
                } else {
                    "code"
                };
                let source = format!("\\usepackage{{fvextra}}{settings}{declaration}\\{command}{options}{body}\\label{{real}}");
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
    }
    let source =
        "\\usepackage{fvextra}\\Verb%line one\r\n[extra=false]\\label{phantom}%\\label{real}"
            .to_owned();
    let doc = Document::new(&main, source.clone(), Some(7));
    assert!(doc.syntax.diagnostics.is_empty(), "{source}");
    assert_eq!(doc.syntax.labels.len(), 1, "{source}");
    assert_eq!(doc.syntax.labels[0].name, "real");
}

#[test]
#[ignore = "requires pdflatex, xelatex, and fvextra"]
fn real_fvextra_special_delimiters_keep_following_labels_editable_after_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for (settings, options) in [
            ("", ""),
            ("", "*[showspaces=true]"),
            (r"\fvset{extra=false}", ""),
            (r"\fvset{extra=false}", "*"),
            (r"\fvset{extra=false}", "[extra=true]"),
        ] {
            for body in [r"%\label{phantom}\input{fake}%", r"\label{phantom}\ "] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = Url::from_file_path(root.join("main.tex")).unwrap();
                let source = format!("\\documentclass{{article}}\n\\usepackage{{fvextra}}\n{settings}\\let\\code\\Verb\n\\begin{{document}}\n\\code{options}{body}\\label{{real}}\n\\end{{document}}\n");
                let mut workspace = Workspace {
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.open(main.clone(), source.clone(), 7);
                let doc = &workspace.documents[&main];
                assert!(doc.syntax.diagnostics.is_empty(), "{source}");
                let at = doc.text.position(source.rfind("real").unwrap());
                let updated = apply(
                    &source,
                    features::rename(&workspace, &main, at, "changed")
                        .unwrap()
                        .unwrap(),
                    &main,
                );
                assert!(updated.contains(body));
                std::fs::write(root.join("main.tex"), updated).unwrap();
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
                    "{engine}/{settings}/{options}/{body}: {}",
                    outcome.output
                );
                let aux = std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                assert!(aux.contains("newlabel{changed}"), "{aux}");
                assert!(
                    !aux.contains("newlabel{real}") && !aux.contains("newlabel{phantom}"),
                    "{aux}"
                );
            }
        }
    }
}
