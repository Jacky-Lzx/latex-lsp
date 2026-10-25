use latex_lsp::{
    features, navigation, semantic, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn uri() -> Url {
    Url::parse("file:///minted-tests/main.tex").unwrap()
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
fn inline_code_is_opaque_to_symbols_dependencies_diagnostics_and_edits() {
    for command in ["mintinline", "mint"] {
        for (open, close, body) in [
            (
                "{",
                "}",
                r"\label{phantom}\input{fake}\ref{kept}\say % 😀 {nested}",
            ),
            (
                "|",
                "|",
                r"\label{phantom}\input{fake}\ref{kept}\say }{% 😀",
            ),
            (
                "§",
                "§",
                r"\label{phantom}\input{fake}\ref{kept}\say }{% 😀",
            ),
            (
                "😀",
                "😀",
                r"\label{phantom}\input{fake}\ref{kept}\say }{% 中文",
            ),
        ] {
            let literal = format!("\\{command}[breaklines,formatcom={{\\relax}}]% header\r\n{{latex}} \r\n{open}{body}{close}");
            let source = format!("\\newcommand{{\\say}}{{Text}}\\label{{kept}}\r\n{literal}\r\n\\say\\ref{{kept}}\\ref{{phantom}}\r\n");
            let main = uri();
            let mut workspace = Workspace::default();
            workspace.open(main.clone(), source.clone(), 7);
            let doc = &workspace.documents[&main];
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{source}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(doc.syntax.labels.len(), 1, "{source}");
            assert_eq!(doc.syntax.definitions.len(), 1);
            assert!(!doc.syntax.commands.iter().any(|cmd| cmd.name == "input"));
            assert!(navigation::document_links(&workspace, &main).is_empty());
            for name in ["kept", "say"] {
                let at = doc.text.position(source.rfind(name).unwrap() + 1);
                assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
                let edit = features::rename(&workspace, &main, at, "changed")
                    .unwrap()
                    .unwrap();
                let updated = apply(&source, edit.changes.unwrap()[&main].clone());
                assert!(updated.contains(&literal));
                assert!(updated.ends_with(&format!(
                    "\\{}\\ref{{{}}}\\ref{{phantom}}\r\n",
                    if name == "say" { "changed" } else { "say" },
                    if name == "kept" { "changed" } else { "kept" }
                )));
            }
            let inside = doc.text.position(source.find("\\ref{kept}").unwrap() + 7);
            assert!(features::completion(&workspace, &main, inside).is_empty());
            assert!(features::definitions(&workspace, &main, inside).is_empty());
            assert!(features::hover(&workspace, &main, inside).is_none());
            assert!(signature::help(&workspace, &main, inside).is_none());
            let missing = doc.text.position(source.rfind("phantom").unwrap() + 1);
            assert!(features::definitions(&workspace, &main, missing).is_empty());
        }
    }
}

#[test]
fn inline_aliases_preserve_literal_bodies_and_command_rename_ranges() {
    for target in ["mintinline", "mint"] {
        for declaration in [
            format!("\\let\\code\\{target}"),
            format!("\\NewCommandCopy{{\\code}}{{\\{target}}}"),
            format!("\\newcommand{{\\code}}{{\\{target}}}"),
            format!("\\def\\code{{\\{target}}}"),
            format!("\\let\\forward\\{target}\\let\\code\\forward"),
        ] {
            let body = r"\label{kept}\input{fake}\code {nested}%";
            let source = format!(
                "{declaration}\\label{{kept}}\\code[breaklines]{{latex}}|{body}|\\ref{{kept}}"
            );
            let main = uri();
            let mut workspace = Workspace::default();
            workspace.open(main.clone(), source.clone(), 7);
            let doc = &workspace.documents[&main];
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{source}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(doc.syntax.labels.len(), 1, "{source}");
            assert!(navigation::document_links(&workspace, &main).is_empty());
            let at = doc
                .text
                .position(source.find("\\code[breaklines").unwrap() + 2);
            let edit = features::rename(&workspace, &main, at, "snippet")
                .unwrap()
                .unwrap();
            let updated = apply(&source, edit.changes.unwrap()[&main].clone());
            assert!(updated.contains(body));
            assert_eq!(updated.matches("\\snippet").count(), 2, "{updated}");
            let at = doc.text.position(source.rfind("kept").unwrap());
            assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
        }
    }
}

#[test]
fn raw_braces_and_percent_signs_balance_without_tex_escape_rules() {
    for source in [
        r"\mintinline{tex}{\{\label{phantom}\}}\label{real}",
        "\\mintinline{tex}{% }\\label{real}",
        "\\mint{tex}{line one\r\n{\\label{phantom}}\r\nline two}\\label{real}",
        "\\mintinline{tex}|{\\label{phantom}%|\\label{real}",
        "\\mintinline{tex}%\\label{phantom}%\\label{real}",
        "\\mintinline{tex}#\\label{phantom}#\\label{real}",
        "\\mintinline{tex}|line one\r\n\\label{phantom}\r\nline two|\\label{real}",
    ] {
        let doc = Document::new(&uri(), source.into(), Some(1));
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
            ["real"],
            "{source}"
        );
    }
}

#[test]
fn macro_arguments_preserve_tokenized_comments_escapes_and_delimiters() {
    for command in ["mintinline", "mint"] {
        for body in [
            r"{a\}\label{phantom}\input{fake}b}",
            "{a% ignored }\r\n\\label{phantom}\\input{fake}b}",
            r"|a\|\label{phantom}\input{fake}b|",
            r"|a{b|\label{phantom}\input{fake}}d|",
            "|a% ignored |\r\n\\label{phantom}\\input{fake}b|",
            "% header comment\r\n|\\label{phantom}\\input{fake}|",
        ] {
            for (before, after) in [
                (r"\textbf{", "}"),
                (r"\newcommand{\one}[1]{#1}\one{", "}"),
                (r"\let\strong\textbf\strong{", "}"),
                (r"\def\example{", r"}\example"),
            ] {
                let literal = format!("{before}\\{command}[breaklines]{{latex}}{body}{after}");
                let source = format!("\\label{{kept}}{literal}\\ref{{kept}}");
                let main = uri();
                let mut workspace = Workspace::default();
                workspace.open(main.clone(), source.clone(), 7);
                let doc = &workspace.documents[&main];
                assert!(
                    doc.syntax.diagnostics.is_empty(),
                    "{source}: {:?}",
                    doc.syntax.diagnostics
                );
                assert_eq!(doc.syntax.labels.len(), 1, "{source}");
                assert!(
                    !doc.syntax
                        .commands
                        .iter()
                        .any(|command| command.name == "input"),
                    "{source}"
                );
                assert!(navigation::document_links(&workspace, &main).is_empty());
                let at = doc.text.position(source.rfind("kept").unwrap());
                let edit = features::rename(&workspace, &main, at, "changed")
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    apply(&source, edit.changes.unwrap()[&main].clone()),
                    format!("\\label{{changed}}{literal}\\ref{{changed}}")
                );
                let inside = doc.text.position(source.find("phantom").unwrap());
                assert!(features::completion(&workspace, &main, inside).is_empty());
                assert!(signature::help(&workspace, &main, inside).is_none());
                assert!(features::hover(&workspace, &main, inside).is_none());
            }
        }
    }
}

#[test]
fn unfinished_macro_arguments_do_not_let_escaped_code_braces_close_the_wrapper() {
    for before in [
        r"\textbf{",
        r"\let\strong\textbf\strong{",
        r"\def\example{",
        r"\newcommand{\example}{",
        r"\def\one#1;{#1}\one{",
    ] {
        let source = format!(r"{before}\mintinline{{latex}}{{a\}}\label{{fake}}b}}");
        let doc = Document::new(&uri(), source.clone(), Some(7));
        assert!(doc.syntax.labels.is_empty(), "{source}");
        assert_eq!(
            doc.syntax.diagnostics.len(),
            1,
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(
            doc.syntax.diagnostics[0].message, "Unclosed brace",
            "{source}"
        );
    }
}

#[test]
fn ordinary_scopes_and_consumed_commands_preserve_raw_minted_catcodes() {
    for before in [
        "{",
        r"\relax{",
        r"\newcommand{\zero}{}\zero{",
        r"\newcommand{\gobble}[1]{}\gobble\textbf{",
        r"\let\zero\relax\zero{",
        r"\unknown{",
    ] {
        let source = format!(r"{before}\mintinline{{latex}}{{a\}}}}\label{{real}}");
        let doc = Document::new(&uri(), source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
    }
}

#[test]
fn incomplete_code_stays_literal_until_its_delimiter_or_end_of_input() {
    for source in [
        "\\mintinline{tex}{\\label{phantom} %\r\n\\input{fake}",
        "\\mint{tex}|\\label{phantom} }%",
    ] {
        let doc = Document::new(&uri(), source.into(), Some(1));
        assert!(doc.syntax.labels.is_empty());
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert!(!doc.syntax.commands.iter().any(|cmd| cmd.name == "input"));
    }
    let source = "\\mintinline{tex}|\\label{phantom} }%\r\ncode|\\label{real}";
    let doc = Document::new(&uri(), source.into(), Some(1));
    assert_eq!(doc.syntax.labels[0].name, "real");
    assert!(doc.syntax.diagnostics.is_empty());
    // A local assignment replaces the built-in argument grammar.
    let source = r"\renewcommand{\mintinline}[1]{#1}\mintinline{\label{real}}";
    let doc = Document::new(&uri(), source.into(), Some(1));
    assert_eq!(doc.syntax.labels[0].name, "real");
}

#[test]
fn headers_have_signatures_and_snippets_and_incremental_edits_refresh_literal_boundaries() {
    let main = uri();
    let mut workspace = Workspace {
        snippets: true,
        ..Default::default()
    };
    for command in ["mintinline", "mint"] {
        for (marked, active) in [
            (format!("\\{command}[break¦lines]{{tex}}{{code}}"), 0),
            (format!("\\{command}[title={{A]B}}]{{te¦x}}{{code}}"), 1),
            (format!("\\{command}{{tex}} ¦"), 2),
        ] {
            let offset = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            workspace.open(main.clone(), source.clone(), 7);
            let help =
                signature::help(&workspace, &main, Text::new(source).position(offset)).unwrap();
            assert_eq!(
                help.signatures[0].label,
                format!("\\{command}[options]{{language}}{{code}}")
            );
            assert_eq!(help.active_parameter, Some(active));
        }
    }
    workspace.open(main.clone(), "\\minti".into(), 8);
    let items = features::completion(&workspace, &main, Position::new(0, 6));
    let item = items
        .iter()
        .find(|item| item.label == "mintinline")
        .unwrap();
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    assert!(item.documentation.is_some());
    let source = "😀 \\mintinline{tex}{\\label{phantom}}\\label{real}";
    workspace.open(main.clone(), source.into(), 9);
    let at = source.find("{\\label").unwrap();
    let text = &workspace.documents[&main].text;
    let range = text.range(at, at + "{\\label{phantom}}".len());
    let updated = text
        .changed(vec![TextDocumentContentChangeEvent {
            range: Some(range),
            range_length: None,
            text: "|}\\label{phantom}%|".into(),
        }])
        .unwrap();
    workspace.open(main.clone(), updated.source, 10);
    assert_eq!(workspace.documents[&main].syntax.labels.len(), 1);
    assert_eq!(workspace.documents[&main].syntax.labels[0].name, "real");
    let doc = &workspace.documents[&main];
    let legend = semantic::options().legend;
    let mut at = Position::new(0, 0);
    let mut kinds = Vec::new();
    for token in semantic::tokens(doc, None).unwrap().data {
        at = Position::new(
            at.line + token.delta_line,
            if token.delta_line == 0 {
                at.character + token.delta_start
            } else {
                token.delta_start
            },
        );
        let end = Position::new(at.line, at.character + token.length);
        kinds.push((
            &doc.text.source[doc.text.offset(at).unwrap()..doc.text.offset(end).unwrap()],
            legend.token_types[token.token_type as usize].as_str(),
        ));
    }
    assert!(kinds.contains(&("\\mintinline", "macro")), "{kinds:?}");
    assert!(
        kinds.contains(&("|}\\label{phantom}%|", "string")),
        "{kinds:?}"
    );
    assert!(!kinds.iter().any(|(_, kind)| *kind == "comment"));
}

#[test]
#[ignore = "requires pdflatex, xelatex and minted v3"]
fn real_tokenized_inline_code_preserves_labels_and_inputs_after_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for command in ["mintinline", "mint"] {
            for body in [
                r"{a\}\typeout{PHANTOM}\label{kept}\input{fake}b}",
                "{a% ignored }\n\\typeout{PHANTOM}\\label{kept}\\input{fake}b}",
                r"|a\|\typeout{PHANTOM}\label{kept}\input{fake}b|",
                r"|a{b|\typeout{PHANTOM}\label{kept}\input{fake}}d|",
                "|a% ignored |\n\\typeout{PHANTOM}\\label{kept}\\input{fake}b|",
                "% header\n|\\typeout{PHANTOM}\\label{kept}\\input{fake}|",
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = Url::from_file_path(root.join("main.tex")).unwrap();
                let literal = format!(
                    "\\let\\code\\{command}\\let\\strong\\textbf\\strong{{\\code{{latex}}{body}}}"
                );
                let source = format!("\\documentclass{{article}}\n\\usepackage[verbatim]{{minted}}\n\\begin{{document}}\n\\label{{kept}}{literal}\\label{{tail}}\\ref{{kept}}\n\\end{{document}}\n");
                let mut workspace = Workspace::default();
                workspace.open(main.clone(), source.clone(), 7);
                let doc = &workspace.documents[&main];
                assert!(
                    doc.syntax.diagnostics.is_empty(),
                    "{source}: {:?}",
                    doc.syntax.diagnostics
                );
                let at = doc.text.position(source.rfind("kept").unwrap());
                let edit = features::rename(&workspace, &main, at, "changed")
                    .unwrap()
                    .unwrap();
                let updated = apply(&source, edit.changes.unwrap()[&main].clone());
                assert!(updated.contains(&literal));
                std::fs::write(root.join("main.tex"), updated).unwrap();
                let output = std::process::Command::new(engine)
                    .args([
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "-no-shell-escape",
                        "main.tex",
                    ])
                    .current_dir(root)
                    .output()
                    .unwrap();
                let log = std::fs::read_to_string(root.join("main.log")).unwrap();
                assert!(output.status.success(), "{engine}/{command}/{body}: {log}");
                assert!(!log.lines().any(|line| line == "PHANTOM"));
                let aux = std::fs::read_to_string(root.join("main.aux")).unwrap();
                assert!(
                    aux.contains("newlabel{changed}") && aux.contains("newlabel{tail}"),
                    "{aux}"
                );
                assert!(!aux.contains("newlabel{kept}"), "{aux}");
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and minted v3"]
fn real_minted_verbatim_mode_confirms_code_boundaries_without_shell_escape() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = concat!(
        "\\documentclass{article}\n\\usepackage[verbatim]{minted}\n\\begin{document}\n",
        "\\mintinline{latex}{\\typeout{PHANTOM}{nested}\\{\\}}\\label{braces}\n",
        "\\mintinline{latex}%\\typeout{PHANTOM}%\\label{percent}\n",
        "\\mintinline{latex}#\\typeout{PHANTOM}#\\label{hash}\n",
        "\\mintinline{latex}|line one\n\\typeout{PHANTOM}\nline two|\\label{multiline}\n",
        "\\mint{latex}{\\typeout{PHANTOM}}\\label{block}\n",
        "\\typeout{AFTER-CODE}\\end{document}\n",
    );
    std::fs::write(root.join("main.tex"), source).unwrap();
    let output = std::process::Command::new("pdflatex")
        .args([
            "-interaction=nonstopmode",
            "-halt-on-error",
            "-no-shell-escape",
            "main.tex",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    let log = std::fs::read_to_string(root.join("main.log")).unwrap();
    assert!(output.status.success(), "{log}");
    assert!(log.lines().any(|line| line == "AFTER-CODE"));
    assert!(!log.lines().any(|line| line == "PHANTOM"));
    let aux = std::fs::read_to_string(root.join("main.aux")).unwrap();
    let doc = Document::new(&uri(), source.into(), Some(1));
    assert!(doc.syntax.diagnostics.is_empty());
    assert_eq!(doc.syntax.labels.len(), 5);
    for label in &doc.syntax.labels {
        assert!(
            aux.contains(&format!("\\newlabel{{{}}}", label.name)),
            "{aux}"
        );
    }
}

#[test]
fn macro_consuming_only_header_leaves_minted_body_catcodes_direct() {
    for command in ["mintinline", "mint"] {
        let source = format!("\\newcommand{{\\three}}[3]{{#1{{#2}}#3}}\\three\\{command}{{latex}}|a\\|\\label{{live}} tail|\\ref{{live}}");
        let main = uri();
        let mut workspace = Workspace::default();
        workspace.open(main.clone(), source.clone(), 7);
        let doc = &workspace.documents[&main];
        assert!(doc.syntax.diagnostics.is_empty(), "{source}");
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
        let at = doc.text.position(source.rfind("live").unwrap());
        let edit = features::rename(&workspace, &main, at, "changed")
            .unwrap()
            .unwrap();
        assert_eq!(
            apply(&source, edit.changes.unwrap()[&main].clone()),
            source.replace("live", "changed")
        );
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, listings, and minted v3"]
fn real_partial_header_arguments_leave_inline_code_readers_in_direct_source() {
    for engine in ["pdflatex", "xelatex"] {
        for code in [
            r"\newcommand{\two}[2]{#1#2}\two\lstinline|a\|\label{live} tail|",
            r"\newcommand{\three}[3]{#1{#2}#3}\three\mintinline{latex}|a\|\label{live} tail|",
            r"\newcommand{\two}[2]{#1#2}\two\Verb|\label{phantom}\input{fake}|\label{live}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\documentclass{{article}}\n\\usepackage{{listings,fancyvrb}}\n\\usepackage[verbatim]{{minted}}\n\\begin{{document}}\n{code}\n\\end{{document}}\n");
            let doc = Document::new(&uri(), source.clone(), Some(7));
            assert!(doc.syntax.diagnostics.is_empty(), "{source}");
            assert_eq!(doc.syntax.labels.len(), 1, "{source}");
            std::fs::write(root.join("main.tex"), source).unwrap();
            let output = std::process::Command::new(engine)
                .args([
                    "-interaction=nonstopmode",
                    "-halt-on-error",
                    "-no-shell-escape",
                    "main.tex",
                ])
                .current_dir(root)
                .output()
                .unwrap();
            let log = std::fs::read_to_string(root.join("main.log")).unwrap();
            assert!(output.status.success(), "{engine}/{code}: {log}");
            let aux = std::fs::read_to_string(root.join("main.aux")).unwrap();
            assert!(aux.contains("newlabel{live}"), "{aux}");
            assert!(!aux.contains("newlabel{phantom}"), "{aux}");
        }
    }
}
