use latex_lsp::{
    language::{features, navigation, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

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
fn macro_arguments_keep_balanced_tokens_comments_and_multiline_code_opaque() {
    for literal in [
        r"\textbf{\lstinline{a{b}c}}",
        "\\textbf{\\lstinline{a% ignored }\r\nb}}",
        r"\textbf{\lstinline{a\}b}}",
        r"\textbf{\lstinline|a{b|c}d|}",
        "\\textbf{\\lstinline|a% ignored |\r\nb|}",
        "\\textbf{\\lstinline|a\r\nb|}",
        "\\textbf{\\lstinline{a\r\nb}}",
        r"\textbf{\lstinline|a\|b|}",
        r"\textcolor{blue}{\lstinline{a{b}c}}",
        r"\newcommand{\one}[1]{#1}\one{\lstinline{a{b}c}}",
        r"\def\one#1{#1}\one{\lstinline{a{b}c}}",
        r"\def\one#1;{#1}\one\lstinline|a{b|c}d|;",
        r"\newcommand{\strong}{\textbf}\strong{\lstinline{a{b}c}}",
        r"\newcommand{\one}[1]{#1}\newcommand{\strong}{\one}\strong{\lstinline{a{b}c}}",
        r"\def\example{\lstinline{a{b}c}}\example",
        r"\newcommand{\example}{\lstinline{a{b}c}}\example",
        r"\NewDocumentCommand{\one}{m}{#1}\one{\lstinline{a{b}c}}",
        "\\textbf{\\lstinline|a{b|c}d|\\lstinline{a\r\nb}}",
        r"\textbf{\lstinline{a{\label{fake}\input{fake}}c}}",
        r"\let\strong\textbf\strong{\lstinline{a{b}c}}",
        r"\NewCommandCopy{\strong}{\textbf}\strong{\lstinline{a{b}c}}",
        r"\newcommand{\one}[1]{#1}\let\strong\one\strong{\lstinline{a{b}c}}",
        r"\def\one#1;{#1}\let\strong\one\strong\lstinline|a{b|c}d|;",
        "\\let\\strong\\textbf\\let\\stronger\\strong\\stronger{\\lstinline{a\r\nb}}",
        r"\let\strong\textbf\newcommand{\stronger}{\strong}\stronger{\lstinline{a{b}c}}",
        r"\newcommand{\one}[1]{#1}\newcommand{\forward}{\one}\let\strong\forward\strong{\lstinline{a{b}c}}",
    ] {
        let source = format!("{literal}\\label{{real}}\\ref{{real}}");
        let mut workspace = Workspace::default();
        let main = uri();
        workspace.open(main.clone(), source.clone(), 7);
        let doc = &workspace.documents[&main];
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
        let at = doc.text.position(source.rfind("real").unwrap());
        let edit = features::rename(&workspace, &main, at, "changed")
            .unwrap()
            .unwrap();
        let updated = apply(&source, edit.changes.unwrap()[&main].clone());
        assert_eq!(
            updated,
            format!("{literal}\\label{{changed}}\\ref{{changed}}")
        );
        let code = source
            .find("a{")
            .or_else(|| source.find("a%"))
            .or_else(|| source.find("a\\"))
            .or_else(|| source.find("a\r"))
            .unwrap();
        let inside = doc.text.position(code + 1);
        assert!(
            features::completion(&workspace, &main, inside).is_empty(),
            "{source}"
        );
        assert!(
            signature::help(&workspace, &main, inside).is_none(),
            "{source}"
        );
    }
}

#[test]
fn unfinished_known_arguments_and_definitions_keep_their_outer_brace_diagnostic() {
    for before in [
        r"\textbf{",
        r"\newcommand{\one}[1]{#1}\one{",
        r"\let\strong\textbf\strong{",
        r"\def\example{",
        r"\newcommand{\example}{",
        r"\NewDocumentCommand{\example}{}{",
        r"\newenvironment{example}{",
        r"\newenvironment{example}{}{",
        r"\NewDocumentEnvironment{example}{}{",
        r"\def\one#1;{#1}\one{",
        r"\NewDocumentCommand{\one}{d<>}{#1}\one<{",
    ] {
        let source = format!("{before}\\lstinline{{a{{\\label{{fake}}}}c}}");
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
    for source in [
        r"{\lstinline{a{b}c}",
        r"\relax{\lstinline{a{b}c}",
        r"\newcommand{\zero}{}\zero{\lstinline{a{b}c}",
        r"\newcommand{\gobble}[1]{}\newcommand{\example}{}\gobble\def\example{\lstinline{a{b}c}",
    ] {
        let doc = Document::new(&uri(), source.into(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
    }
}

#[test]
fn ordinary_groups_and_consumed_formatter_controls_keep_source_catcodes() {
    for prefix in [
        "{",
        r"\relax{",
        r"\iftrue{",
        r"\newcommand{\zero}{}\zero{",
        r"\renewcommand{\textbf}{}\textbf{",
        r"\newcommand{\one}[1]{#1}\one\lstinline{",
        r"\newcommand{\gobble}[1]{}\gobble\textbf{",
        r"\let\unused\textbf{",
        r"\unknown{",
        r"{\textbf{\lstinline{abc}}",
        r"\let\zero\relax\zero{",
        r"\newcommand{\none}{}\let\zero\none\zero{",
        r"\let\strong\textbf\newcommand{\gobble}[1]{}\gobble\strong{",
        r"\let\unknown\undefined\unknown{",
        r"\let\unknown\later\newcommand{\later}[1]{#1}\unknown{",
        r"\let\zero\textbf\let\zero\relax\zero{",
        r"\newcommand{\one}[1]{#1}\renewcommand{\one}{}\let\zero\one\zero{",
    ] {
        let inline = if prefix.ends_with(r"\lstinline{") {
            "a{b}"
        } else {
            r"\lstinline{a{b}c}"
        };
        let suffix = if prefix.starts_with(r"\iftrue") {
            r"\fi"
        } else {
            ""
        };
        let source = format!("{prefix}{inline}{suffix}\\label{{real}}");
        let doc = Document::new(&uri(), source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
        assert_eq!(doc.syntax.labels[0].name, "real");
    }
    let source =
        "\\def\\one#1;{#1}{\\one\\lstinline|a\nb|;\\lstinline|a\n\\label{real}|}\\label{tail}";
    let doc = Document::new(&uri(), source.into(), Some(7));
    assert!(
        doc.syntax.diagnostics.is_empty(),
        "{:?}",
        doc.syntax.diagnostics
    );
    assert_eq!(
        doc.syntax
            .labels
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["real", "tail"]
    );
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
fn static_inline_aliases_keep_their_names_editable_and_their_code_opaque() {
    for declaration in [
        r"\let\code\lstinline",
        r"\NewCommandCopy{\code}{\lstinline}",
        r"\newcommand{\code}{\lstinline}",
        r"\def\code{\lstinline}",
        r"\let\forward\lstinline\let\code\forward",
        r"\newcommand{\forward}{\lstinline}\newcommand{\code}{\forward}",
    ] {
        let body = r"\label{kept}\input{fake}\code }%";
        let source =
            format!("{declaration}\\label{{kept}}\\code[language=TeX]|{body}|\\ref{{kept}}");
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
            .position(source.find("\\code[language").unwrap() + 2);
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

#[test]
fn local_redefinitions_replace_the_inline_grammar() {
    for declaration in [r"\renewcommand{\lstinline}[1]{#1}", r"\let\lstinline\other"] {
        let source = format!("{declaration}\\lstinline{{\\label{{real}}}}");
        let doc = Document::new(&uri(), source, Some(7));
        assert_eq!(doc.syntax.labels.len(), 1, "{declaration}");
        assert_eq!(doc.syntax.labels[0].name, "real");
        assert!(doc.syntax.diagnostics.is_empty());
    }
    for declaration in [
        r"\newcommand{\code}[1]{#1}",
        r"\newcommand{\code}[1]{\lstinline}",
        r"\newcommand{\code}{\unknown}",
        r"\newcommand{\code}{\forward}\newcommand{\forward}{\code}",
        r"\let\code\unknown",
        r"\let\code\later\newcommand{\later}{\lstinline}",
        r"\let\code\lstinline\renewcommand{\code}[1]{#1}",
        r"\newcommand{\gobble}[1]{}\gobble\let\code\lstinline",
    ] {
        let source = format!("{declaration}\\code{{\\label{{real}}}}");
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
            r"\textbf{\lstinline{a{b}c}}",
            "\\textbf{\\lstinline{a% ignored }\nb}}",
            r"\textbf{\lstinline{a\}b}}",
            r"\textbf{\lstinline|a{b|c}d|}",
            "\\textbf{\\lstinline|a% ignored |\nb|}",
            "\\textbf{\\lstinline|a\nb|}",
            "\\textbf{\\lstinline{a\nb}}",
            r"\textbf{\lstinline|a\|b|}",
            r"\newcommand{\one}[1]{#1}\one{\lstinline{a{b}c}}",
            r"\def\example{\lstinline{a{b}c}}\example",
            r"\relax{\lstinline{a{b}c}",
            r"\newcommand{\zero}{}\zero{\lstinline{a{b}c}",
            r"\let\strong\textbf\strong{\lstinline{a{b}c}}",
            r"\NewCommandCopy{\strong}{\textbf}\strong{\lstinline{a{b}c}}",
            r"\newcommand{\one}[1]{#1}\let\strong\one\strong{\lstinline{a{b}c}}",
            r"\def\one#1;{#1}\let\strong\one\strong\lstinline|a{b|c}d|;",
            "\\let\\strong\\textbf\\let\\stronger\\strong\\stronger{\\lstinline{a\nb}}",
            r"\let\strong\textbf\newcommand{\stronger}{\strong}\stronger{\lstinline{a{b}c}}",
            r"\let\zero\relax\zero{\lstinline{a{b}c}",
            r"\let\code\lstinline\code|\label{kept}\input{fake}|",
            r"\newcommand{\code}{\lstinline}\code|\label{kept}\input{fake}|",
            r"\let\forward\lstinline\let\code\forward\code|\label{kept}\input{fake}|",
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
            let outcome = latex_lsp::compilation::build::run(
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

#[test]
fn macro_consuming_only_command_and_delimiter_keeps_body_catcodes_direct() {
    let source = r"\newcommand{\two}[2]{#1#2}\two\lstinline|a\|\label{live} tail|\ref{live}";
    let main = uri();
    let mut workspace = Workspace::default();
    workspace.open(main.clone(), source.into(), 7);
    let doc = &workspace.documents[&main];
    assert!(doc.syntax.diagnostics.is_empty());
    assert_eq!(doc.syntax.labels.len(), 1);
    let at = doc.text.position(source.rfind("live").unwrap());
    let edit = features::rename(&workspace, &main, at, "changed")
        .unwrap()
        .unwrap();
    assert_eq!(
        apply(source, edit.changes.unwrap()[&main].clone()),
        source.replace("live", "changed")
    );
}
