use latex_lsp::{
    features, navigation, semantic, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn uri() -> Url {
    Url::parse("file:///listings-inline/main.tex").unwrap()
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
fn literal_delimiters_and_tex_headers_keep_code_opaque_and_following_source_editable() {
    for literal in [
        r"\lstinline|\label{kept}\input{fake}\style }%|".to_owned(),
        r"\lstinline*\label{kept}\input{fake}\style }%*".into(),
        "\\lstinline§\\label{kept}\\input{fake}\\style }%§".into(),
        "\\lstinline😀\\label{kept}\\input{fake}\\style }%😀".into(),
        r"\lstinline{\label fake {\style}".into(),
        r"\lstinline [title=[literal] |\label{kept}\input{fake}\style }%|".into(),
        r"\lstinline[title={A]B}] |\label{kept}\input{fake}\style }%|".into(),
        "\\lstinline% comment\r\n[language=TeX]% comment\r\n|\\label{kept}\\input{fake}\\style }%|"
            .into(),
        "\\lstinline [basicstyle={\\style}] |\\label{kept}\\input{fake}\\style }%|".into(),
    ] {
        let source=format!("\\newcommand{{\\style}}{{\\ttfamily}}\\label{{kept}}\r\n{literal}\\style\\ref{{kept}}\r\n");
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
        assert_eq!(doc.syntax.labels[0].name, "kept");
        assert_eq!(doc.syntax.definitions.len(), 1);
        assert!(!doc
            .syntax
            .commands
            .iter()
            .any(|command| command.name == "input"));
        assert!(navigation::document_links(&workspace, &main).is_empty());
        assert_eq!(
            features::references(
                &workspace,
                &main,
                doc.text.position(source.rfind("kept").unwrap()),
                true
            )
            .len(),
            2
        );
        let edit = features::rename(
            &workspace,
            &main,
            doc.text.position(source.rfind("kept").unwrap()),
            "changed",
        )
        .unwrap()
        .unwrap();
        let updated = apply(&source, edit.changes.unwrap()[&main].clone());
        assert!(updated.contains(&literal));
        assert!(updated.ends_with("\\style\\ref{changed}\r\n"));
        let count = if literal.contains("basicstyle") { 3 } else { 2 };
        assert_eq!(
            features::references(
                &workspace,
                &main,
                doc.text.position(source.rfind("style").unwrap()),
                true
            )
            .len(),
            count
        );
        let body = source.find("\\label{kept}").unwrap();
        let body = source[body + 1..]
            .find("\\label")
            .map(|at| body + 1 + at)
            .unwrap();
        let inside = doc.text.position(body + 3);
        assert!(features::definitions(&workspace, &main, inside).is_empty());
        assert!(features::completion(&workspace, &main, inside).is_empty());
        assert!(features::hover(&workspace, &main, inside).is_none());
        assert!(signature::help(&workspace, &main, inside).is_none());
    }
}

#[test]
fn braced_code_uses_the_first_literal_closing_brace_and_recovers_at_line_end() {
    for source in [
        r"\lstinline{\label fake}\label{real}",
        r"\lstinline{a{b}\label{real}",
        r"\lstinline{a\}\label{real}",
        r"\lstinline{a%}\label{real}",
        "\\lstinline|\\label{fake} }%\r\n\\label{real}",
        "\\lstinline{\\label fake {\r\n\\label{real}",
    ] {
        let doc = Document::new(&uri(), source.into(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
        assert_eq!(doc.syntax.labels[0].name, "real");
    }
}

#[test]
fn headers_have_signatures_hover_snippets_and_preserve_existing_code() {
    let main = uri();
    let mut workspace = Workspace {
        snippets: true,
        ..Default::default()
    };
    for marked in [
        r"\lstinline¦",
        r"\lstinline[language=Te¦X]|code|",
        r"\lstinline[title={A]B},language=Te¦X]|code|",
        r"\lstinline[title=[lite¦ral]|code|",
        "\\lstinline% continue\r\n [language=Te¦X]|code|",
        "\\lstinline[title={A% ] ignored\r\nB¦}]|code|",
        r"\lstinline[language=Te¦",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        let help = signature::help(&workspace, &main, Text::new(source).position(offset))
            .unwrap_or_else(|| panic!("missing hint: {marked}"));
        assert_eq!(help.signatures[0].label, r"\lstinline[options]");
        assert_eq!(help.active_parameter, Some(0));
    }
    for marked in [
        r"\lstinline[title=[literal]¦",
        r"\lstinline|co¦de|",
        r"\lstinline*co¦de*",
        r"\lstinline{co¦de}",
        r"\lstinline{code} ¦",
        r"\lstinline|code| ¦",
        "\\lstinline[title={A% comm¦ent\r\nB}]|code|",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        assert!(
            signature::help(&workspace, &main, Text::new(source).position(offset)).is_none(),
            "{marked}"
        );
    }
    workspace.open(main.clone(), "\\lsti".into(), 7);
    let items = features::completion(&workspace, &main, Position::new(0, 5));
    let item = items.iter().find(|item| item.label == "lstinline").unwrap();
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(edit.new_text, "lstinline|${1:code}|");
    for marked in [
        r"\lsti¦nline|code|",
        r"\lsti¦nline{code}",
        r"\lsti¦nline*code*",
        r"\lsti¦nline[language=TeX]|code|",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 8);
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        );
        let item = items.iter().find(|item| item.label == "lstinline").unwrap();
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(apply(&source, vec![edit.clone()]), source);
    }
    let source =
        "😀 \\lstinline% header\r\n[basicstyle={\\ttfamily}] |}\\label{fake}%|\\label{real}";
    workspace.open(main.clone(), source.into(), 9);
    let doc = &workspace.documents[&main];
    assert!(features::hover(
        &workspace,
        &main,
        doc.text.position(source.find("lstinline").unwrap() + 2)
    )
    .is_some());
    let mut position = Position::new(0, 0);
    let legend = semantic::options().legend;
    let mut spans = Vec::new();
    for token in semantic::tokens(doc, None).unwrap().data {
        position = Position::new(
            position.line + token.delta_line,
            if token.delta_line == 0 {
                position.character + token.delta_start
            } else {
                token.delta_start
            },
        );
        let end = Position::new(position.line, position.character + token.length);
        spans.push((
            &source[doc.text.offset(position).unwrap()..doc.text.offset(end).unwrap()],
            legend.token_types[token.token_type as usize].as_str(),
        ));
    }
    assert!(spans.contains(&("\\lstinline", "macro")), "{spans:?}");
    assert!(spans.contains(&("\\ttfamily", "macro")), "{spans:?}");
    assert!(
        spans.contains(&("|}\\label{fake}%|", "string")),
        "{spans:?}"
    );
    assert!(spans
        .iter()
        .any(|(value, kind)| *kind == "comment" && value.contains("header")));
}

#[test]
fn local_redefinitions_replace_the_inline_grammar() {
    for declaration in [r"\renewcommand{\lstinline}[1]{#1}", r"\let\lstinline\other"] {
        let source = format!("{declaration}\\lstinline{{\\label{{real}}}}");
        let doc = Document::new(&uri(), source, Some(7));
        assert_eq!(doc.syntax.labels.len(), 1, "{declaration}");
        assert_eq!(doc.syntax.labels[0].name, "real");
        assert!(doc.syntax.diagnostics.is_empty());
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, and listings"]
fn real_inline_forms_preserve_native_labels_inputs_and_header_commands_after_renaming() {
    for engine in ["pdflatex", "xelatex"] {
        for literal in [
            r"\lstinline|\label{kept}\input{fake}\style { }%|",
            r"\lstinline*\label{kept}\input{fake}\style { }%*",
            r"\lstinline{\label kept {\style}",
            r"\lstinline{a\}",
            r"\lstinline [title=[literal] |\label{kept}\input{fake}|",
            r"\lstinline[title={A]B}] |\label{kept}\input{fake}|",
            "\\lstinline% comment\n[language=TeX]% comment\n|\\label{kept}\\input{fake}|",
            r"\lstinline[basicstyle={\style}]|\label{kept}\input{fake}|",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let fake = Url::from_file_path(root.join("fake.tex")).unwrap();
            std::fs::write(root.join("fake.tex"), r"\label{fakeinput}").unwrap();
            let source=format!("\\documentclass{{article}}\n\\usepackage{{listings}}\n\\newcommand{{\\style}}{{\\ttfamily\\typeout{{HEADER-RAN}}}}\n\\begin{{document}}\n\\label{{kept}}\n{literal}\\label{{tail}}\\ref{{kept}}\n\\end{{document}}\n");
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            let doc = &workspace.documents[&main];
            assert!(
                doc.syntax.diagnostics.is_empty(),
                "{literal}: {:?}",
                doc.syntax.diagnostics
            );
            let at = doc.text.position(source.rfind("kept").unwrap());
            assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
            let edit = features::rename(&workspace, &main, at, "renamedlabel")
                .unwrap()
                .unwrap();
            let updated = apply(&source, edit.changes.unwrap()[&main].clone());
            assert!(updated.contains(literal));
            assert!(updated.contains("\\ref{renamedlabel}"));
            std::fs::write(root.join("main.tex"), &updated).unwrap();
            let outcome = latex_lsp::build::run(
                &root.join("main.tex"),
                &latex_lsp::config::BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                },
                &std::sync::atomic::AtomicBool::new(false),
            );
            assert_eq!(
                outcome.status, "success",
                "{engine}/{literal}: {}",
                outcome.output
            );
            if literal.contains("basicstyle") {
                assert!(outcome.output.contains("HEADER-RAN"));
            }
            assert!(!outcome.inputs.unwrap().contains(&fake));
            let aux = std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
            assert!(aux.contains("newlabel{renamedlabel}"));
            assert!(aux.contains("newlabel{tail}"));
            assert!(
                !aux.contains("newlabel{kept}") && !aux.contains("newlabel{fakeinput}"),
                "{aux}"
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, and listings"]
fn real_unfinished_code_recovers_at_eol_without_changing_the_compiler_error_after_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for delimiter in ["|", "{"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let literal = format!("\\lstinline{delimiter} \\label kept \\input fake");
            let source=format!("\\documentclass{{article}}\n\\usepackage{{listings}}\n\\begin{{document}}\n\\label{{kept}}\n{literal}\n\\ref{{kept}}\\label{{after}}\n\\end{{document}}\n");
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            let at = workspace.documents[&main]
                .text
                .position(source.rfind("kept").unwrap());
            let edit = features::rename(&workspace, &main, at, "renamedlabel")
                .unwrap()
                .unwrap();
            let updated = apply(&source, edit.changes.unwrap()[&main].clone());
            assert!(updated.contains(&literal));
            std::fs::write(root.join("main.tex"), updated).unwrap();
            // Continue after the native EOL error to observe the recovered line.
            let output = std::process::Command::new(engine)
                .args(["-interaction=nonstopmode", "-no-shell-escape", "main.tex"])
                .current_dir(root)
                .output()
                .unwrap();
            let log = String::from_utf8_lossy(&output.stdout);
            assert!(!output.status.success(), "{engine}/{delimiter}: {log}");
            assert!(
                log.contains("lstinline ended by EOL"),
                "{engine}/{delimiter}: {log}"
            );
            let aux = std::fs::read_to_string(root.join("main.aux")).unwrap();
            assert!(
                aux.contains("newlabel{renamedlabel}") && aux.contains("newlabel{after}"),
                "{engine}/{delimiter}: {aux}"
            );
            assert!(!aux.contains("newlabel{kept}"));
        }
    }
}
