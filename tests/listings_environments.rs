use latex_lsp::{
    compilation::build,
    editing::{self, file_operations},
    language::{features, navigation, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;
use std::{path::Path, sync::atomic::AtomicBool};

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
fn environment_option_signatures_respect_unfinished_groups_comments_and_unicode() {
    let main = Url::parse("file:///listings-environments/signatures.tex").unwrap();
    let mut workspace = Workspace::default();
    for marked in [
        r"\begin{lstlisting}¦",
        r"\begin{lstlisting} ¦",
        r"\begin{lstlisting}[frame=sin¦gle]",
        r"\begin{lstlisting}[title={A]B},frame=sin¦gle]",
        r"\begin{lstlisting}[title=[lite¦ral]",
        r"\begin{lstlisting}[title={\detokenize{\end{lstlisting}}},frame=sin¦gle]",
        r"\begin{lstlisting}[title={\%literal¦}]",
        "\\begin{lstlisting}% continue\r\n [frame=sin¦gle]",
        "\\begin{lstlisting}[title={A% ] ignored\r\nB¦}]",
        "\\begin{lstlisting}[title={😀中文¦}]",
        r"\begin{lstlisting}[frame=¦",
        r"\begin{lstlisting}[title={nested[bracket]¦",
    ] {
        let marked = format!("% 😀\r\n{marked}");
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        let help = signature::help(&workspace, &main, Text::new(source).position(offset))
            .unwrap_or_else(|| panic!("missing signature: {marked}"));
        assert_eq!(
            help.signatures[0].label, r"\begin{lstlisting}[options]",
            "{marked}"
        );
        assert_eq!(help.active_parameter, Some(1), "{marked}");
        assert_eq!(help.signatures[0].active_parameter, Some(1));
        assert_eq!(help.signatures[0].parameters.as_ref().unwrap().len(), 2);
        assert!(help.signatures[0].documentation.is_some());
    }
}

#[test]
fn completed_options_names_comments_and_raw_bodies_do_not_continue_header_hints() {
    let main = Url::parse("file:///listings-environments/no-signatures.tex").unwrap();
    let mut workspace = Workspace::default();
    for marked in [
        "\\begin{lstlisting}% comm¦ent\r\n[frame=single]",
        "\\begin{lstlisting}[title={A% comm¦ent\r\nB}]",
        "\\begin{lstlisting}[title={\\\\% comm¦ent\r\nB}]",
        r"\begin{lstlisting}[frame=single]¦",
        r"\begin{lstlisting}[title=[literal]¦",
        r"\begin{lstlisting}{na¦me}",
        r"\begin{lstlisting}[frame=single]{na¦me}",
        r"\begin{lstlisting}[frame=single] ignored¦",
        "\\begin{lstlisting}\r\n[frame=sin¦gle]",
        "\\begin{lstlisting}\r\n{na¦me}",
        "\\begin{lstlisting}\r\n\\frac{a}{b¦}\r\n\\end{lstlisting}",
        "\\begin{lstlisting}[frame=single]\r\n\\frac{a}{b¦}\r\n\\end{lstlisting}",
        "\\begin{verbatim}\r\n\\begin{lstlisting}[frame=sin¦gle]\r\n\\end{verbatim}",
        r"% \begin{lstlisting}[frame=sin¦gle]",
        r"\begin{ lstlisting}[frame=sin¦gle]",
        r"\begin{lstlisting }[frame=sin¦gle]",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        assert!(
            signature::help(&workspace, &main, Text::new(source).position(offset)).is_none(),
            "{marked}"
        );
    }
    // Closing-line commands are ordinary TeX and keep their own signatures.
    let source = "\\begin{lstlisting}\r\ncode\r\n\\end{lstlisting}\\frac{a}{b}";
    workspace.open(main.clone(), source.into(), 7);
    let help = signature::help(
        &workspace,
        &main,
        Text::new(source.into()).position(source.rfind('b').unwrap() + 1),
    )
    .unwrap();
    assert_eq!(help.signatures[0].label, r"\frac{numerator}{denominator}");
    assert_eq!(help.active_parameter, Some(1));
}

#[test]
fn project_overrides_and_unsaved_settings_refresh_listings_header_signatures() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    let settings = uri(root, "settings.sty");
    let unrelated = root.join("other-project");
    std::fs::create_dir(&unrelated).unwrap();
    std::fs::write(
        unrelated.join("main.tex"),
        r"\documentclass{article}\def\begin#1{}",
    )
    .unwrap();
    let source="\\documentclass{article}\\usepackage{settings}\r\n\\begin{lstlisting}[frame=single]\r\ncode\r\n\\end{lstlisting}\r\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    let at = Text::new(source.into()).position(source.find("single").unwrap() + 2);
    for definition in [
        r"\renewenvironment{lstlisting}[1]{}{}",
        r"\NewDocumentEnvironment{lstlisting}{mm}{}{}",
        r"\RenewEnvironmentCopy{lstlisting}{other}",
        r"\def\lstlisting#1{#1}",
        r"\let\lstlisting\other",
        r"\csundef{lstlisting}",
        r"\def\lstlisting@#1{#1}",
        r"\csundef{lstlisting@}",
        r"\lstnewenvironment{lstlisting}[1]{}{}",
        r"\lstnewenvironment{\dynamic}{}{}",
        r"\def\begin#1{}",
        r"\csundef{\dynamic}",
    ] {
        std::fs::write(root.join("settings.sty"), definition).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        workspace.add_root(unrelated.clone());
        workspace.open(main.clone(), source.into(), 7);
        assert!(
            signature::help(&workspace, &main, at).is_none(),
            "{definition}"
        );
        workspace.open(settings.clone(), String::new(), 8);
        assert_eq!(
            signature::help(&workspace, &main, at).unwrap().signatures[0].label,
            r"\begin{lstlisting}[options]"
        );
    }
    for definition in [
        r"\lstnewenvironment{Other}[1]{}{}",
        r"\renewenvironment{lstlisting*}{}{}",
    ] {
        std::fs::write(root.join("settings.sty"), definition).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        workspace.open(main.clone(), source.into(), 7);
        assert!(
            signature::help(&workspace, &main, at).is_some(),
            "{definition}"
        );
    }
}

#[test]
fn header_markers_and_discarded_header_text_preserve_real_source_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    std::fs::write(root.join("chapter.tex"), r"\label{chapterlabel}").unwrap();
    std::fs::write(root.join("fake.tex"), r"\label{fakeinput}").unwrap();
    for newline in ["\n", "\r\n"] {
        for header in [
            "".to_owned(),
            "[title={\\detokenize{\\end{lstlisting}}}]".into(),
            format!("[frame=single,{newline}title={{\\detokenize{{\\end{{lstlisting}}}}}}]"),
            format!("% comment \\end{{lstlisting}}{newline}[frame=single]"),
            "[frame=single]{name\\detokenize{\\end{lstlisting}}}".into(),
            "[frame=single]ignored \\end{lstlisting}".into(),
            "ignored \\end{lstlisting}".into(),
            "[title={A]B}]".into(),
            "[title=[literal]".into(),
            format!("[title={{A% ] \\end{{lstlisting}}{newline}B}}]"),
        ] {
            let before = format!(
                "\\documentclass{{article}}\\newcommand{{\\say}}{{Text}}\\label{{kept}}{newline}"
            );
            let body = format!(
                "😀 \\label{{kept}}\\label{{phantom}}\\input{{fake}}\\say\\ref{{kept}} {{{newline}"
            );
            let block =
                format!("\\begin{{lstlisting}}{header}{newline}{body}  \\end{{lstlisting}}");
            let tail = format!("\\input{{chapter}}\\say\\ref{{kept}}\\ref{{phantom}}{newline}");
            let source = format!("{before}{block}{tail}");
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
                "{header}: {:?}",
                doc.syntax.diagnostics
            );
            assert_eq!(
                doc.syntax.labels.len(),
                1,
                "{header}: {:?}",
                doc.syntax.labels
            );
            assert_eq!(doc.syntax.labels[0].name, "kept");
            assert_eq!(
                workspace.dependencies(&main),
                vec![uri(root, "chapter.tex")]
            );
            let links = navigation::document_links(&workspace, &main);
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].target, Some(uri(root, "chapter.tex")));
            for name in ["kept", "say"] {
                let at = doc.text.position(source.rfind(name).unwrap());
                assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
                let edits = versioned(
                    features::rename(&workspace, &main, at, "changed")
                        .unwrap()
                        .unwrap(),
                    &main,
                );
                assert_eq!(
                    apply(&source, edits),
                    format!(
                        "{}{block}{}",
                        before.replace(name, "changed"),
                        tail.replace(name, "changed")
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
            assert_eq!(
                apply(&source, versioned(edit, &main)),
                format!("{before}{block}{}", tail.replace("chapter", "renamed"))
            );
            let inside = doc.text.position(source.find("\\input{fake}").unwrap() + 3);
            assert!(features::completion(&workspace, &main, inside).is_empty());
            assert!(features::definitions(&workspace, &main, inside).is_empty());
            assert!(features::hover(&workspace, &main, inside).is_none());
            assert!(signature::help(&workspace, &main, inside).is_none());
        }
    }
}

#[test]
fn header_groups_and_comment_continuations_do_not_start_literal_body_early() {
    let main = Url::parse("file:///listings-environments/header-tokens.tex").unwrap();
    for newline in ["\n", "\r\n"] {
        for header in [
            format!("{{name{newline}\\detokenize{{\\end{{lstlisting}}}}}}"),
            format!("[frame=single]% continue \\end{{lstlisting}}{newline}{{name\\detokenize{{\\end{{lstlisting}}}}}}"),
            "{name} dropped \\end{lstlisting}".into(),
            "\\relax dropped \\end{lstlisting}".into(),
            "😀 dropped \\end{lstlisting}".into(),
            format!("{newline}[raw"),
            format!("[frame=single]{newline}{{raw"),
        ] {
            let source = format!("\\begin{{lstlisting}}{header}{newline}\\label{{fake}}{newline}\\end{{lstlisting}}\\label{{real}}");
            let doc = Document::new(&main,source,Some(7));
            assert!(doc.syntax.diagnostics.is_empty(),"{header}: {:?}",doc.syntax.diagnostics);
            assert_eq!(doc.syntax.labels.len(),1,"{header}");
            assert_eq!(doc.syntax.labels[0].name,"real");
        }
    }
}

#[test]
fn active_newlines_and_literal_body_markers_use_the_exact_closing_string() {
    let main = Url::parse("file:///listings-environments/body.tex").unwrap();
    for newline in ["\n", "\r\n"] {
        for header in ["", "[frame=single]"] {
            for prefix in ["[raw] ", "{raw} ", "\\end{other}", "\\", "\\en", "% "] {
                let source = format!("\\begin{{lstlisting}}{header}{newline}\\end {{lstlisting}}\\label{{fake}}{newline}{prefix}\\end{{lstlisting}}\\label{{real}}");
                let doc = Document::new(&main, source, Some(7));
                assert!(
                    doc.syntax.diagnostics.is_empty(),
                    "{header}/{prefix}: {:?}",
                    doc.syntax.diagnostics
                );
                assert_eq!(doc.syntax.labels.len(), 1);
                assert_eq!(doc.syntax.labels[0].name, "real");
            }
        }
    }
}

#[test]
fn real_delimiters_control_folding_linked_editing_highlighting_and_close_fixes() {
    let main = Url::parse("file:///listings-environments/editor.tex").unwrap();
    let source = concat!(
        "% 😀\r\n",
        "\\begin{lstlisting}[title={\\detokenize{\\end{lstlisting}}}]\r\n",
        "😀 \\label{fake} }%\r\n",
        "\\end{other}\\end{lstlisting}\\label{real}\r\n",
    );
    let doc = Document::new(&main, source.into(), Some(7));
    let close = source.rfind("\\end{lstlisting}").unwrap();
    let open = source.find("lstlisting").unwrap();
    assert!(doc.syntax.diagnostics.is_empty());
    let folds = navigation::folding_ranges_for_client(
        &doc,
        &FoldingRangeClientCapabilities {
            line_folding_only: Some(false),
            ..Default::default()
        },
    );
    assert_eq!(folds.len(), 1, "{folds:?}");
    assert_eq!((folds[0].start_line, folds[0].end_line), (1, 3));
    assert_eq!(
        folds[0].end_character,
        Some(
            doc.text
                .position(close + "\\end{lstlisting}".len())
                .character
        )
    );
    for at in [open, close + 5] {
        assert_eq!(
            editing::linked_editing_ranges(&doc, doc.text.position(at + 1))
                .unwrap()
                .ranges,
            vec![
                doc.text.range(open, open + 10),
                doc.text.range(close + 5, close + 15)
            ]
        );
    }
    let fake = source.find("\\end{lstlisting}").unwrap();
    assert!(editing::linked_editing_ranges(&doc, doc.text.position(fake + 6)).is_none());
    let mut position = Position::new(0, 0);
    let mut literal_body = false;
    let mut real_label = false;
    for token in semantic::tokens(&doc, None).unwrap().data {
        position = Position::new(
            position.line + token.delta_line,
            if token.delta_line == 0 {
                position.character + token.delta_start
            } else {
                token.delta_start
            },
        );
        let kind = &semantic::options().legend.token_types[token.token_type as usize];
        if position.line == 2 {
            assert_eq!(*kind, SemanticTokenType::STRING);
            literal_body = true;
        }
        if position.line == 3
            && position.character
                >= doc
                    .text
                    .position(close + "\\end{lstlisting}".len())
                    .character
        {
            assert_ne!(*kind, SemanticTokenType::STRING);
            real_label = true;
        }
    }
    assert!(literal_body && real_label);
    for newline in ["\n", "\r\n"] {
        for header in [
            "[title={\\detokenize{\\end{lstlisting}}}]",
            "ignored \\end{lstlisting}",
        ] {
            let source = format!("\\begin{{lstlisting}}{header}{newline}\\label{{fake}}{newline}");
            let doc = Document::new(&main, source.clone(), Some(7));
            assert_eq!(doc.syntax.diagnostics.len(), 1);
            assert_eq!(
                doc.syntax.diagnostics[0].message,
                "Unclosed environment `lstlisting`"
            );
            assert!(doc.syntax.labels.is_empty());
            assert_eq!(doc.syntax.fixes.len(), 1);
            let fixed = apply(&source, vec![doc.syntax.fixes[0].edit.clone()]);
            let repaired = Document::new(&main, fixed, Some(8));
            assert!(repaired.syntax.diagnostics.is_empty());
            assert!(repaired.syntax.labels.is_empty());
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex, and listings"]
fn real_headers_and_body_markers_preserve_native_labels_after_versioned_renames() {
    for engine in ["pdflatex", "xelatex"] {
        let headers = [
            "",
            "[frame=single]",
            "\n[raw",
            "[frame=single]\n{raw",
            "[title={\\detokenize{\\end{lstlisting}}}]",
            "[frame=single,\ntitle={\\detokenize{\\end{lstlisting}}}]",
            "% comment \\end{lstlisting}\n[frame=single]",
            "[frame=single]{name\\detokenize{\\end{lstlisting}}}",
            "[frame=single]ignored \\end{lstlisting}",
            "ignored \\end{lstlisting}",
            "[title={A]B}]",
            "[title=[literal]",
            "[title={A% ] \\end{lstlisting}\nB}]",
            "[frame=single]% continuation\n{name\\detokenize{\\end{lstlisting}}}",
        ];
        let cases = headers.into_iter().map(|header| (header, "")).chain(
            ["[raw] ", "{raw} ", "\\end{other}", "\\", "\\en", "% "]
                .into_iter()
                .map(|prefix| ("[frame=single]", prefix)),
        );
        for (header, prefix) in cases {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = uri(root, "main.tex");
            let fake = uri(root, "fake.tex");
            std::fs::write(root.join("fake.tex"), r"\label{fakeinput}").unwrap();
            let body = "\\label{kept}\\label{fake}\\input{fake} {";
            let source=format!("\\documentclass{{article}}\n\\usepackage{{listings}}\n\\begin{{document}}\n\\label{{kept}}\n\\begin{{lstlisting}}{header}\n{body}\n{prefix}\\end{{lstlisting}}\\ref{{kept}}\n\\end{{document}}\n");
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
                "{engine}/{header}/{prefix}: {:?}",
                doc.syntax.diagnostics
            );
            if let Some(offset) = source
                .find("frame=single")
                .or_else(|| source.find("title="))
            {
                let help =
                    signature::help(&workspace, &main, doc.text.position(offset + 2)).unwrap();
                assert_eq!(help.signatures[0].label, r"\begin{lstlisting}[options]");
                assert_eq!(help.active_parameter, Some(1));
            }
            assert!(signature::help(
                &workspace,
                &main,
                doc.text.position(source.find(body).unwrap() + 9)
            )
            .is_none());
            let at = doc.text.position(source.rfind("kept").unwrap());
            assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
            let updated = apply(
                &source,
                versioned(
                    features::rename(&workspace, &main, at, "renamedlabel")
                        .unwrap()
                        .unwrap(),
                    &main,
                ),
            );
            assert!(updated.contains(header) && updated.contains(body));
            assert!(updated.contains("\\ref{renamedlabel}"));
            std::fs::write(root.join("main.tex"), &updated).unwrap();
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
                "{engine}/{header}/{prefix}: {}",
                outcome.output
            );
            assert!(!outcome.inputs.unwrap().contains(&fake));
            let aux = std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
            assert!(aux.contains("newlabel{renamedlabel}"), "{aux}");
            for key in ["kept", "fake", "fakeinput"] {
                assert!(!aux.contains(&format!("newlabel{{{key}}}")), "{aux}");
            }
        }
    }
}
