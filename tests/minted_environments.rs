use latex_lsp::{
    compilation::build_log,
    editing::{self, file_operations},
    language::{features, navigation, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;
use std::path::Path;

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
fn header_markers_mixed_end_lines_and_closing_suffixes_remain_literal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("chapter.tex"), r"\label{chapterlabel}").unwrap();
    std::fs::write(root.join("fake.tex"), r"\label{fakeinput}").unwrap();
    let main = uri(root, "main.tex");
    for newline in ["\n", "\r\n"] {
        for header in [
            "{latex}".to_owned(),
            format!("[label={{\\detokenize{{\\end{{minted}}}}}}]{newline}{{latex}}"),
            format!("% comment \\end{{minted}}{newline}[linenos]% language{newline}{{latex}}"),
            format!("[breaklines,{newline}label={{A]B}}]{newline}{{latex}}"),
            // Required arguments may also be a single TeX token.
            "p".to_owned(),
        ] {
            let before = format!(
                "\\documentclass{{article}}\\newcommand{{\\say}}{{Text}}\\label{{kept}}{newline}"
            );
            let body = format!("\\end{{other}}\\end{{minted}}\\label{{falseend}}{newline}😀 \\label{{kept}}\\label{{phantom}}\\input{{fake}}\\say\\ref{{kept}} {{{newline}");
            let tail = r"\label{kept}\label{discarded}\input{fake}\say\ref{kept}\end{other} % 😀";
            let after = format!(
                "\\input{{chapter}}\\say\\ref{{kept}}\\ref{{phantom}}\\ref{{discarded}}{newline}"
            );
            let block =
                format!("\\begin{{minted}}{header}{newline}{body}  \\end{{minted}}{tail}{newline}");
            let source = format!("{before}{block}{after}");
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
                1,
                "{source}: {:?}",
                doc.syntax.labels
            );
            assert_eq!(doc.syntax.labels[0].name, "kept");
            assert_eq!(doc.syntax.definitions.len(), 1);
            assert_eq!(
                workspace.dependencies(&main),
                vec![uri(root, "chapter.tex")]
            );
            let links = navigation::document_links(&workspace, &main);
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].target, Some(uri(root, "chapter.tex")));
            for absent in ["phantom", "discarded"] {
                let at = doc.text.position(source.rfind(absent).unwrap());
                assert!(features::definitions(&workspace, &main, at).is_empty());
            }
            for name in ["kept", "say"] {
                let at = doc.text.position(source.rfind(name).unwrap());
                assert_eq!(features::references(&workspace, &main, at, true).len(), 2);
                let edit = features::rename(&workspace, &main, at, "changed")
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    apply(&source, versioned(edit, &main)),
                    format!(
                        "{}{block}{}",
                        before.replace(name, "changed"),
                        after.replace(name, "changed")
                    )
                );
            }
            let file_edit = file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: uri(root, "chapter.tex").into(),
                    new_uri: uri(root, "renamed.tex").into(),
                }],
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                apply(&source, versioned(file_edit, &main)),
                format!("{before}{block}{}", after.replace("chapter", "renamed"))
            );
            let inside = doc.text.position(source.find("\\input{fake}").unwrap() + 4);
            assert!(features::completion(&workspace, &main, inside).is_empty());
            assert!(features::definitions(&workspace, &main, inside).is_empty());
            assert!(features::hover(&workspace, &main, inside).is_none());
            assert!(signature::help(&workspace, &main, inside).is_none());
        }
    }
}

#[test]
fn folding_linked_names_and_highlighting_use_the_actual_end_marker() {
    let main = Url::parse("file:///minted-environments/main.tex").unwrap();
    let source = concat!(
        "\\begin{minted}[label={\\detokenize{\\end{minted}}}]\r\n{latex}\r\n",
        "\\end{other}\\end{minted}\\label{fake}\r\n",
        "😀 \\input{fake} % literal {\r\n",
        "  \\end{minted}\\label{discarded} % still literal\r\n",
        "\\label{real}\r\n",
    );
    let doc = Document::new(&main, source.into(), Some(7));
    assert!(doc.syntax.diagnostics.is_empty());
    let folds = navigation::folding_ranges(&doc, None);
    assert_eq!(folds.len(), 1, "{folds:?}");
    assert_eq!((folds[0].start_line, folds[0].end_line), (0, 4));
    let close = source.rfind("\\end{minted}").unwrap();
    for offset in [source.find("minted").unwrap(), close + 5] {
        let linked = editing::linked_editing_ranges(&doc, doc.text.position(offset + 1)).unwrap();
        assert_eq!(
            linked.ranges,
            vec![doc.text.range(7, 13), doc.text.range(close + 5, close + 11)]
        );
    }
    for offset in [
        source.find("\\end{minted}").unwrap(),
        source.find("\\end{other}").unwrap(),
    ] {
        assert!(editing::linked_editing_ranges(&doc, doc.text.position(offset + 6)).is_none());
    }
    let mut position = Position::new(0, 0);
    let mut body = false;
    let mut suffix = false;
    for token in semantic::tokens(&doc, None).unwrap().data {
        position = Position::new(
            position.line + token.delta_line,
            if token.delta_line == 0 {
                position.character + token.delta_start
            } else {
                token.delta_start
            },
        );
        if position.line == 3 || position.line == 4 && position.character >= 14 {
            assert_eq!(
                semantic::options().legend.token_types[token.token_type as usize],
                SemanticTokenType::STRING
            );
            body |= position.line == 3;
            suffix |= position.line == 4;
        }
    }
    assert!(body && suffix);
    assert!(doc.syntax.comments.is_empty());
}

#[test]
fn false_end_markers_leave_a_single_unclosed_environment_with_a_working_fix() {
    let main = Url::parse("file:///minted-environments/incomplete.tex").unwrap();
    for newline in ["\n", "\r\n"] {
        let source = format!("\\begin{{minted}}[label={{\\detokenize{{\\end{{minted}}}}}}]{newline}{{latex}}{newline}\\end{{other}}\\end{{minted}}{newline}\\label{{fake}}{newline}");
        let doc = Document::new(&main, source.clone(), Some(7));
        assert!(doc.syntax.labels.is_empty());
        assert_eq!(doc.syntax.diagnostics.len(), 1);
        assert_eq!(
            doc.syntax.diagnostics[0].message,
            "Unclosed environment `minted`"
        );
        assert_eq!(doc.syntax.fixes.len(), 1);
        assert_eq!(
            doc.syntax.fixes[0].edit.new_text,
            format!("\\end{{minted}}{newline}")
        );
        assert!(editing::linked_editing_ranges(&doc, Position::new(0, 8)).is_none());
        let updated = apply(&source, vec![doc.syntax.fixes[0].edit.clone()]);
        let repaired = Document::new(&main, updated, Some(8));
        assert!(repaired.syntax.diagnostics.is_empty());
        assert!(repaired.syntax.labels.is_empty());
    }
}

#[test]
fn header_signatures_follow_tex_argument_boundaries_and_unfinished_input() {
    let main = Url::parse("file:///minted-environments/signatures.tex").unwrap();
    let mut workspace = Workspace::default();
    for (marked, active) in [
        (r"\begin{minted}¦", 1),
        (r"\begin{minted} ¦", 1),
        (r"\begin{minted}[break¦lines]{python}", 1),
        (r"\begin{minted}[label={A]B},break¦lines]{python}", 1),
        (r"\begin{minted}[label=[literal]{py¦thon}", 2),
        (
            r"\begin{minted}[label={\detokenize{\end{minted}}}]{la¦tex}",
            2,
        ),
        (r"\begin{minted}[label={\%literal¦}]{python}", 1),
        (r"\begin{minted}[linenos]¦", 2),
        ("\\begin{minted}[linenos]% ] ignored\r\n{py¦thon}", 2),
        ("\\begin{minted}[label={A% ] ignored\r\nB}]{py¦thon}", 2),
        ("\\begin{minted}{py% } ignored\r\nth¦on}", 2),
        (
            "\\begin{minted}% remove endline\r\n[linenos] % language\r\n {py¦thon}",
            2,
        ),
        (r"\begin{minted}{py¦thon}", 2),
        (r"\begin{minted}{\lan¦guage}", 2),
        (r"\begin{minted}\lan¦guage", 2),
        (r"\begin{minted}[break¦", 1),
        (r"\begin{minted}[label={nested[bracket]¦", 1),
        (r"\begin{minted}{py¦", 2),
        ("\\begin{minted}{😀中文¦}", 2),
    ] {
        let marked = format!("% 😀\r\n{marked}");
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        let help = signature::help(&workspace, &main, Text::new(source).position(offset))
            .unwrap_or_else(|| panic!("missing signature: {marked}"));
        assert_eq!(
            help.signatures[0].label, r"\begin{minted}[options]{language}",
            "{marked}"
        );
        assert_eq!(help.active_parameter, Some(active), "{marked}");
        assert_eq!(help.signatures[0].active_parameter, Some(active));
        assert_eq!(help.signatures[0].parameters.as_ref().unwrap().len(), 3);
        assert!(help.signatures[0].documentation.is_some());
    }
}

#[test]
fn comments_completed_headers_raw_code_and_discarded_tails_have_no_header_signature() {
    let main = Url::parse("file:///minted-environments/no-signatures.tex").unwrap();
    let mut workspace = Workspace::default();
    for marked in [
        "\\begin{minted}% comm¦ent\r\n{python}",
        "\\begin{minted}[label={A% comm¦ent\r\nB}]{python}",
        "\\begin{minted}[label={\\\\% comm¦ent\r\nB}]{python}",
        "\\begin{minted}{py% comm¦ent\r\nthon}",
        r"\begin{minted}{python}¦",
        "\\begin{minted}{python} ignored¦\r\ncode\r\n\\end{minted}",
        "\\begin{minted}{python}\r\n\\frac{a}{b¦}\r\n\\end{minted}",
        "\\begin{minted}{python}\r\ncode\r\n\\end{minted}\\frac{a}{b¦}\r\n",
        "\\begin{minted}\r\n\r\n{py¦thon}",
        "\\begin{minted}[linenos]\r\n\r\n{py¦thon}",
        "\\begin{verbatim}\r\n\\begin{minted}[break¦lines]{python}\r\n\\end{verbatim}",
        r"% \begin{minted}[break¦lines]{python}",
        r"\begin{ minted}[break¦lines]{python}",
        r"\begin{minted }[break¦lines]{python}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        assert!(
            signature::help(&workspace, &main, Text::new(source).position(offset)).is_none(),
            "{marked}"
        );
    }
}

#[test]
fn project_overrides_suppress_builtin_headers_and_unsaved_settings_refresh_them() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["one", "two"] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    let main = uri(root, "one/main.tex");
    let settings = uri(root, "one/settings.sty");
    let source = "\\documentclass{article}\\usepackage{settings}\r\n\\begin{minted}[linenos]{python}\r\ncode\r\n\\end{minted}\r\n";
    std::fs::write(root.join("one/main.tex"), source).unwrap();
    std::fs::write(
        root.join("two/main.tex"),
        r"\documentclass{article}\renewenvironment{minted}[1]{}{}",
    )
    .unwrap();
    let at = Text::new(source.into()).position(source.find("python").unwrap() + 2);
    for definition in [
        r"\renewenvironment{minted}[1]{}{}",
        r"\NewDocumentEnvironment{minted}{mm}{}{}",
        r"\RenewEnvironmentCopy{minted}{other}",
        r"\def\minted#1{#1}",
        r"\let\minted\other",
        r"\def\begin#1{}",
        r"\newenvironment{begin}{}{}",
        r"\csundef{minted}",
        r"\csundef{\dynamic}",
    ] {
        std::fs::write(root.join("one/settings.sty"), definition).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.join("one"));
        workspace.add_root(root.join("two"));
        workspace.open(main.clone(), source.into(), 7);
        assert!(
            signature::help(&workspace, &main, at).is_none(),
            "{definition}"
        );
        workspace.open(settings.clone(), String::new(), 8);
        let help = signature::help(&workspace, &main, at)
            .unwrap_or_else(|| panic!("header did not refresh: {definition}"));
        assert_eq!(
            help.signatures[0].label,
            r"\begin{minted}[options]{language}"
        );
        assert_eq!(help.active_parameter, Some(2));
    }
}

#[test]
#[ignore = "requires pdflatex, minted v3, and latexminted on PATH"]
fn real_minted_environment_boundaries_and_versioned_renames_match_both_reader_modes() {
    for raw in [true, false] {
        for discarded_tail in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            std::fs::create_dir(root.join("_minted")).unwrap();
            let main = uri(root, "main.tex");
            let options = if raw {
                "verbatim"
            } else {
                "highlightmode=immediate"
            };
            let tail = if discarded_tail {
                r"\label{kept}\label{discarded}"
            } else {
                ""
            };
            let body = "\\end{other}\\end{minted}\\label{falseend}\n\\label{kept}\\label{phantom} % MINTEDBODY\n";
            let source = format!("\\documentclass{{article}}\n\\usepackage[{options}]{{minted}}\n\\begin{{document}}\n\\label{{kept}}\n\\begin{{minted}}[label={{\\detokenize{{\\end{{minted}}}}}}]\n{{latex}}\n{body}\\end{{minted}}{tail}\nReal.\\ref{{kept}}\n\\end{{document}}\n");
            let mut workspace = Workspace {
                client: latex_lsp::project::state::ClientCapabilities {
                    versioned_edits: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.open(main.clone(), source.clone(), 7);
            for (offset, active) in [
                (source.find("detokenize").unwrap() + 3, 1),
                (source.find("{latex}").unwrap() + 3, 2),
            ] {
                let help = signature::help(
                    &workspace,
                    &main,
                    workspace.documents[&main].text.position(offset),
                )
                .unwrap();
                assert_eq!(
                    help.signatures[0].label,
                    r"\begin{minted}[options]{language}"
                );
                assert_eq!(help.active_parameter, Some(active));
            }
            let at = workspace.documents[&main]
                .text
                .position(source.rfind("kept").unwrap());
            let edit = features::rename(&workspace, &main, at, "renamedlabel")
                .unwrap()
                .unwrap();
            let updated = apply(&source, versioned(edit, &main));
            assert!(updated.contains(body));
            assert!(updated.contains(&format!("\\end{{minted}}{tail}\n")));
            std::fs::write(root.join("main.tex"), &updated).unwrap();
            let output = std::process::Command::new("pdflatex")
                .args([
                    "-interaction=nonstopmode",
                    "-file-line-error",
                    if raw {
                        "-no-shell-escape"
                    } else {
                        "-shell-restricted"
                    },
                    "main.tex",
                ])
                .current_dir(root)
                .output()
                .unwrap();
            let log = std::fs::read_to_string(root.join("main.log")).unwrap();
            assert_eq!(
                output.status.success(),
                !discarded_tail,
                "{raw}/{discarded_tail}: {log}"
            );
            if discarded_tail {
                assert!(log.contains("Extraneous input"), "{log}");
                let diagnostics = build_log::parse(&root.join("main.tex"), &log);
                assert!(
                    diagnostics
                        .get(&main)
                        .is_some_and(|items| items.iter().any(|item| {
                            item.severity == Some(DiagnosticSeverity::ERROR)
                                && item.message.contains("Extraneous input")
                        })),
                    "{diagnostics:?}"
                );
            }
            let aux = std::fs::read_to_string(root.join("main.aux")).unwrap();
            assert!(aux.contains("\\newlabel{renamedlabel}"), "{aux}");
            for label in ["kept", "discarded", "falseend", "phantom"] {
                assert!(!aux.contains(&format!("\\newlabel{{{label}}}")), "{aux}");
            }
            if !raw {
                assert!(std::fs::read_dir(root.join("_minted"))
                    .unwrap()
                    .flatten()
                    .filter(|entry| entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".highlight.minted"))
                    .any(|entry| std::fs::read_to_string(entry.path())
                        .unwrap()
                        .contains("MINTEDBODY")));
            }
        }
    }
}
