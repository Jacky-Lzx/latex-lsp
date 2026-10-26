use latex_lsp::{
    language::{features, navigation, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

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
fn escaped_code_uses_ordinary_balanced_tokens_without_executing_its_commands() {
    for (header, body) in [
        ("", r"\label{phantom}\input{fake}\ref{kept}\say"),
        ("*", r"a\}\label{phantom}{nested}\input{fake}\ref{kept}\say"),
        (
            "[showspaces=true]",
            "a% }\\label{phantom}\r\n\\ref{kept}\\say\\input{fake}",
        ),
        (
            "*[label={A]B},formatcom={\\label{active}}]",
            r"\ref{kept}\say\input{fake}",
        ),
        ("[label=[literal]", r"\ref{kept}\say\input{fake}"),
    ] {
        for (before, after) in [("", ""), (r"\textbf{", "}"), (r"\def\example{", "}")] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            std::fs::write(root.join("chapter.tex"), "Chapter text").unwrap();
            std::fs::write(root.join("fake.tex"), r"\label{fake}").unwrap();
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let literal = format!("\\EscVerb{header}{{{body}}}");
            let source = format!("% 😀\r\n\\usepackage{{fvextra}}\\fvset{{extra=false}}\\label{{kept}}\\newcommand{{\\say}}{{Text}}{before}{literal}{after}\\input{{chapter}}\\say\\ref{{kept}}");
            std::fs::write(root.join("main.tex"), &source).unwrap();
            let mut workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
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
            assert_eq!(
                doc.syntax.labels.len(),
                if header.contains("active") { 2 } else { 1 },
                "{source}"
            );
            assert_eq!(
                workspace.dependencies(&main),
                [Url::from_file_path(root.join("chapter.tex")).unwrap()],
                "{source}"
            );
            assert_eq!(
                navigation::document_links(&workspace, &main).len(),
                1,
                "{source}"
            );
            for (old, new) in [("kept", "changed"), ("say", "speak")] {
                let at = doc.text.position(source.rfind(old).unwrap());
                let updated = apply(
                    &source,
                    features::rename(&workspace, &main, at, new)
                        .unwrap()
                        .unwrap(),
                );
                assert!(updated.contains(&literal), "{updated}");
                assert!(
                    updated.ends_with(&r"\input{chapter}\say\ref{kept}".replace(old, new)),
                    "{updated}"
                );
            }
            let inside = doc
                .text
                .position(source.find(body).unwrap() + body.find("kept").unwrap());
            assert!(features::completion(&workspace, &main, inside).is_empty());
            assert!(features::definitions(&workspace, &main, inside).is_empty());
            assert!(features::hover(&workspace, &main, inside).is_none());
            assert!(signature::help(&workspace, &main, inside).is_none());
            let mut line = 0;
            let mut character = 0;
            let mut covered = false;
            for token in semantic::tokens(doc, None).unwrap().data {
                line += token.delta_line;
                character = if token.delta_line == 0 {
                    character + token.delta_start
                } else {
                    token.delta_start
                };
                if line == inside.line
                    && character <= inside.character
                    && inside.character < character + token.length
                {
                    assert_eq!(token.token_type, 4, "{source}");
                    covered = true;
                }
            }
            assert!(covered, "{source}");
        }
    }
}

#[test]
fn header_comments_follow_fvextra_peeking_and_pretokenized_arguments() {
    let main = Url::parse("file:///escaped-inline/main.tex").unwrap();
    for (code, exposed) in [
        ("\\EscVerb% header\r\n{\\label{example}}", true),
        ("\\EscVerb*% header\r\n{\\label{example}}", true),
        (
            "\\EscVerb[showspaces=true]% header\r\n{\\label{example}}",
            false,
        ),
        ("\\textbf{\\EscVerb% header\r\n{\\label{example}}}", false),
        (
            "\\newcommand{\\two}[2]{#1#2}\\two\\EscVerb% header\r\n{\\label{example}}",
            false,
        ),
        (r"\EscVerb|\label{example}|", true),
    ] {
        let source = format!("\\usepackage{{fvextra}}{code}\\label{{real}}");
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(doc.syntax.diagnostics.is_empty(), "{source}");
        assert_eq!(
            doc.syntax.labels.len(),
            1 + usize::from(exposed),
            "{source}"
        );
    }
}

#[test]
fn escaped_reader_copies_and_forwarders_keep_their_own_rename_identity() {
    let main = Url::parse("file:///escaped-inline/main.tex").unwrap();
    for declaration in [
        r"\let\code\EscVerb",
        r"\NewCommandCopy{\code}{\EscVerb}",
        r"\newcommand{\code}{\EscVerb}",
        r"\let\first\EscVerb\let\code\first",
    ] {
        let source = format!(
            "{declaration}\\textbf{{\\code{{\\label{{phantom}}\\input{{fake}}}}}}\\label{{real}}"
        );
        let mut workspace = Workspace {
            client: latex_lsp::project::state::ClientCapabilities {
                versioned_edits: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.open(main.clone(), source.clone(), 7);
        let doc = &workspace.documents[&main];
        assert!(doc.syntax.diagnostics.is_empty(), "{source}");
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
        assert_eq!(doc.syntax.labels[0].name, "real");
        let at = doc.text.position(source.rfind("code").unwrap());
        let updated = apply(
            &source,
            features::rename(&workspace, &main, at, "example")
                .unwrap()
                .unwrap(),
        );
        assert!(
            updated.contains(r"\example{\label{phantom}\input{fake}}"),
            "{updated}"
        );
        assert!(!updated.contains(r"\code"));
    }
    for name in ["input", "begin", "lstinline", "mintinline", "Verb"] {
        let source = format!(
            "\\let\\{name}\\EscVerb\\{name}{{\\label{{phantom}}\\input{{fake}}}}\\label{{real}}"
        );
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
fn unfinished_code_retains_brace_errors_while_redefinitions_use_ordinary_arguments() {
    let main = Url::parse("file:///escaped-inline/main.tex").unwrap();
    for code in [
        r"\EscVerb{\label{phantom}",
        "\\EscVerb{a% }\r\n\\input{fake}",
    ] {
        let doc = Document::new(&main, code.into(), Some(7));
        assert!(
            doc.syntax.labels.is_empty()
                && doc
                    .syntax
                    .commands
                    .iter()
                    .all(|command| command.name == "EscVerb")
        );
        assert_eq!(doc.syntax.diagnostics.len(), 1, "{code}");
        assert_eq!(doc.syntax.diagnostics[0].message, "Unclosed brace");
    }
    for declaration in [
        r"\newcommand{\EscVerb}[1]{#1}",
        r"\newcommand{\EscVerb}[1]{\EscVerb{#1}}",
        r"\let\EscVerb\unknown",
    ] {
        let source = format!("{declaration}\\EscVerb{{\\label{{live}}}}\\ref{{live}}");
        let doc = Document::new(&main, source.clone(), Some(7));
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
    }
}

#[test]
fn escaped_command_completion_hover_and_option_hints_preserve_code() {
    let main = Url::parse("file:///escaped-inline/main.tex").unwrap();
    for (marked, snippet) in [
        (r"\EscV¦", true),
        (r"\EscV¦erb{a\}b}", false),
        (r"\EscV¦erb*[showspaces=true]{code}", false),
    ] {
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let mut workspace = Workspace {
            client: latex_lsp::project::state::ClientCapabilities {
                snippets: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.open(main.clone(), source.clone(), 7);
        let items = features::completion(&workspace, &main, Text::new(source).position(at));
        let item = items.iter().find(|item| item.label == "EscVerb").unwrap();
        assert_eq!(
            item.insert_text_format,
            Some(if snippet {
                InsertTextFormat::SNIPPET
            } else {
                InsertTextFormat::PLAIN_TEXT
            })
        );
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(
            edit.new_text,
            if snippet {
                "EscVerb{${1:code}}"
            } else {
                "EscVerb"
            }
        );
    }
    let source = r"\EscVerb*[label={A]B},showspaces=true]{code}";
    let mut workspace = Workspace::default();
    workspace.open(main.clone(), source.into(), 7);
    let text = Text::new(source.into());
    let help = signature::help(
        &workspace,
        &main,
        text.position(source.find("true").unwrap()),
    )
    .unwrap();
    assert_eq!(help.signatures[0].label, r"\EscVerb[options]{code}");
    assert_eq!(help.active_parameter, Some(0));
    assert!(
        serde_json::to_string(&features::hover(&workspace, &main, text.position(3)).unwrap())
            .unwrap()
            .contains("fvextra")
    );
}

#[test]
#[ignore = "requires pdflatex, xelatex, and fvextra"]
fn real_escaped_code_never_executes_commands_even_when_escapes_are_invalid() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;
    for engine in ["pdflatex", "xelatex"] {
        for declaration in ["", r"\let\code\EscVerb", r"\newcommand{\code}{\EscVerb}"] {
            for (body, valid) in [
                (r"a\\label\{phantom\}\\input\{fake\}\% {nested}", true),
                (r"\label{phantom}\typeout{PHANTOM}\input{fake}", false),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let caller = if declaration.is_empty() {
                    "EscVerb"
                } else {
                    "code"
                };
                let call = format!("\\{caller}*[showspaces=true]{{{body}}}");
                let call = if declaration.is_empty() {
                    call
                } else {
                    format!("\\textbf{{{call}}}")
                };
                let source = format!("\\documentclass{{article}}\n\\usepackage{{fvextra}}\n\\fvset{{extra=false}}\n{declaration}\n\\begin{{document}}\n\\label{{kept}}{call}\\ref{{kept}}\\label{{real}}\n\\end{{document}}\n");
                let main = Url::from_file_path(root.join("main.tex")).unwrap();
                let mut workspace = Workspace {
                    client: latex_lsp::project::state::ClientCapabilities {
                        versioned_edits: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                workspace.open(main.clone(), source.clone(), 7);
                assert_eq!(workspace.documents[&main].syntax.labels.len(), 2);
                let at = Text::new(source.clone()).position(source.rfind("kept").unwrap());
                let updated = apply(
                    &source,
                    features::rename(&workspace, &main, at, "changed")
                        .unwrap()
                        .unwrap(),
                );
                assert!(updated.contains(body));
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
                        outcome.status == "success",
                        valid,
                        "{engine}: {}",
                        outcome.output
                    );
                    assert_eq!(
                        outcome.output.contains("Invalid backslash escape"),
                        !valid,
                        "{}",
                        outcome.output
                    );
                    assert!(!outcome.output.lines().any(|line| line == "PHANTOM"));
                    let aux =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                    assert!(
                        aux.contains(&format!("newlabel{{{kept}}}"))
                            && aux.contains("newlabel{real}"),
                        "{aux}"
                    );
                    assert!(
                        !aux.contains("newlabel{phantom}")
                            && !aux.contains(&format!("newlabel{{{absent}}}")),
                        "{aux}"
                    );
                    let fls =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.fls")).unwrap();
                    assert!(!fls.contains("fake.tex"));
                }
            }
        }
    }
}
