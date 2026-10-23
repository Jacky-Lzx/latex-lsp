use latex_lsp::{
    build,
    config::BuildConfig,
    editing, features, navigation, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use std::sync::atomic::AtomicBool;
use tower_lsp::lsp_types::*;

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
fn single_code_lines_keep_commands_braces_and_files_literal() {
    for newline in ["\n", "\r\n"] {
        for options in [
            "",
            "[showspaces=true]",
            "[label={\\detokenize{\\end{VerbEnv}}}]",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            std::fs::write(root.join("chapter.tex"), "Chapter text").unwrap();
            std::fs::write(root.join("fake.tex"), r"\label{fake}").unwrap();
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let body = r"😀 \label{phantom}\input{fake}\ref{kept}\say\begin{wrong}{% literal";
            let literal =
                format!("\\begin{{VerbEnv}}{options}{newline}{body}{newline}\t \\end{{VerbEnv}} ");
            let source = format!("\\usepackage{{fvextra}}\\newcommand{{\\say}}{{Text}}\\label{{kept}}{newline}{literal}{newline}\\input{{chapter}}\\say\\ref{{kept}}");
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
                "{source}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(doc.syntax.labels.len(), 1);
            assert_eq!(doc.syntax.definitions.len(), 1);
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
                assert!(updated.contains(&literal));
                assert!(updated.ends_with(&r"\input{chapter}\say\ref{kept}".replace(old, new)));
            }
            let start = source.find("VerbEnv").unwrap();
            let end = source.rfind(r"\end{VerbEnv}").unwrap();
            let linked = editing::linked_editing_ranges(doc, doc.text.position(start + 1)).unwrap();
            assert_eq!(linked.ranges[1], doc.text.range(end + 5, end + 12));
            assert!(navigation::folding_ranges(doc, None)
                .iter()
                .any(|fold| fold.start_line == 1 && fold.end_line == 3));
            let inside = doc.text.position(source.find("phantom").unwrap());
            assert!(features::completion(&workspace, &main, inside).is_empty());
            assert!(signature::help(&workspace, &main, inside).is_none());
        }
    }
}

#[test]
fn malformed_closing_lines_resume_source_and_retain_the_environment_scope() {
    let main = Url::parse("file:///verb-env/main.tex").unwrap();
    for ending in [
        "not a closing line",
        r"X\end{VerbEnv}",
        r"\end{VerbEnv}X",
        "\\end{VerbEnv}\t",
        r"\end{other}\end{VerbEnv}",
    ] {
        let source = format!("\\usepackage{{fvextra}}\\begin{{VerbEnv}}\n\\label{{phantom}}\\input{{fake}}\n{ending}\n\\label{{after}}\\fvset{{extra=false}}\\textbf{{\\Verb|\\label{{inside}}|}}\\end{{VerbEnv}}\n\\textbf{{\\Verb|\\label{{outside}}|}}\\label{{real}}");
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        let labels: Vec<_> = doc
            .syntax
            .labels
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(labels, ["after", "inside", "real"], "{source}");
        assert!(!doc
            .syntax
            .commands
            .iter()
            .any(|command| command.name == "input"));
    }
    // An end marker in the code line stops reading before the closing line.
    let source = "\\begin{VerbEnv}\nexample \\end{VerbEnv}\n\\end{VerbEnv}\n\\label{real}";
    let doc = Document::new(&main, source.into(), Some(7));
    assert!(doc.syntax.diagnostics.is_empty());
    assert_eq!(doc.syntax.labels.len(), 1);
    for source in [
        "\\begin{VerbEnv}",
        "\\begin{VerbEnv}\r",
        "\\begin{VerbEnv}\ncode",
        "\\begin{VerbEnv}\nexample \\end{VerbEnv}",
    ] {
        let doc = Document::new(&main, source.into(), Some(7));
        assert_eq!(doc.syntax.diagnostics.len(), 1, "{source}");
        assert!(doc.syntax.diagnostics[0]
            .message
            .contains("Unclosed environment `VerbEnv`"));
        assert!(
            editing::linked_editing_ranges(&doc, Position::new(0, 8)).is_none(),
            "{source}"
        );
    }
}

#[test]
fn ordinary_header_comments_groups_and_options_precede_raw_lines() {
    let main = Url::parse("file:///verb-env/main.tex").unwrap();
    for header in [
        "[showspaces=true]% comment\n\\label{header}",
        " {discarded\n\\label{header}}",
        "\\% discarded",
        "% continue\n[showspaces=true]",
    ] {
        let source = format!(
            "\\begin{{VerbEnv}}{header}\n\\label{{phantom}}\n\\end{{VerbEnv}}\n\\label{{real}}"
        );
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{source}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(doc.syntax.labels.len(), 1, "{source}");
        assert_eq!(doc.syntax.labels[0].name, "real");
    }
    let marked = "\\begin{VerbEnv}[label={A]B},showspaces=tr¦ue]";
    let at = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let mut workspace = Workspace::default();
    workspace.open(main.clone(), source.clone(), 7);
    let help = signature::help(&workspace, &main, Text::new(source).position(at)).unwrap();
    assert_eq!(help.signatures[0].label, r"\begin{VerbEnv}[options]");
    assert_eq!(help.active_parameter, Some(1));
    let source = r"\begin{VerbEn}";
    workspace.open(main.clone(), source.into(), 8);
    assert!(features::completion(
        &workspace,
        &main,
        Text::new(source.into()).position(source.len() - 1)
    )
    .iter()
    .any(|item| item.label == "VerbEnv"));
    for source in ["\\begin{VerbEnv}\r\n😀 \\label{phantom}\r\n\\end{VerbEnv}", "\\begin{VerbEnv}[label={A% comment\r\nB}]% header\r\n{discarded\r\ntext}\r\ncode\r\n\\end{VerbEnv}\r\n"] {
        for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
            let doc = Document::new(&main, source[..end].into(), Some(8));
            assert!(doc.syntax.ignored.iter().all(|span| span.start <= span.end && span.end <= end && source.is_char_boundary(span.start) && source.is_char_boundary(span.end)));
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, and fvextra"]
fn real_single_line_code_and_recovery_preserve_aux_labels_after_rename() {
    for engine in ["pdflatex", "xelatex"] {
        for (body, ending, repair, valid) in [
            (
                r"\label{phantom}\input{fake}",
                r"\end{VerbEnv}",
                false,
                true,
            ),
            (
                r"\label{phantom}\input{fake}",
                "\t \\end{VerbEnv} ",
                false,
                true,
            ),
            (
                r"\label{phantom}\input{fake}",
                r"\end{VerbEnv}X",
                true,
                false,
            ),
            (
                r"\label{phantom}\input{fake}",
                "not a closing line",
                true,
                false,
            ),
            (r"example \end{VerbEnv}", r"\end{VerbEnv}", false, false),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = Url::from_file_path(root.join("main.tex")).unwrap();
            let source = format!("\\documentclass{{article}}\n\\usepackage{{fvextra}}\n\\begin{{document}}\n\\label{{kept}}\\begin{{VerbEnv}}[showspaces=true]\n{body}\n{ending}\n\\label{{after}}\\fvset{{extra=false}}\\textbf{{\\Verb|\\label{{inside}}|}}{}\\textbf{{\\Verb|\\label{{outside}}|}}Text.\\ref{{kept}}\\label{{real}}\n\\end{{document}}\n", if repair { "\\end{VerbEnv}\n" } else { "" });
            let mut workspace = Workspace {
                versioned_edits: true,
                ..Default::default()
            };
            workspace.open(main.clone(), source.clone(), 7);
            assert!(
                workspace.documents[&main].syntax.diagnostics.is_empty(),
                "{source}"
            );
            assert_eq!(
                workspace.documents[&main].syntax.labels.len(),
                if repair { 4 } else { 5 },
                "{source}"
            );
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
                if !valid {
                    assert!(
                        outcome.output.contains("fvextra Error"),
                        "{}",
                        outcome.output
                    );
                }
                let aux = std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                for name in [kept, "after", "inside", "real"] {
                    assert!(aux.contains(&format!("newlabel{{{name}}}")), "{aux}");
                }
                assert_eq!(aux.contains("newlabel{outside}"), !repair, "{aux}");
                for name in [absent, "phantom"] {
                    assert!(!aux.contains(&format!("newlabel{{{name}}}")), "{aux}");
                }
                let fls = std::fs::read_to_string(root.join("build/latex-lsp/main.fls")).unwrap();
                assert!(!fls.contains("fake.tex"));
            }
        }
    }
}
