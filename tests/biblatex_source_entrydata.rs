use latex_lsp::{features, signature, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{biblatex}}\\addbibresource{{data.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(
        dir.path().join("data.bib"),
        "@book{collection,title={Raw data}} @book{member,title={Member}}",
    )
    .unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = main.join("data.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, bib)
}

fn location(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let at = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert!(doc.text.source[at..].starts_with(name));
    Location::new(uri.clone(), doc.text.range(at, at + name.len()))
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
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
fn native_text_wrappers_execute_reference_arguments_and_offer_their_real_signatures() {
    for wrapper in ["mkbibparens", "mkbibbrackets", "parentext", "brackettext"] {
        let (_dir, ws, main, _bib) = setup(&format!(r"\label{{safe}}\{wrapper}{{\ref{{safe}}}}"));
        let query = location(&ws, &main, r"\ref{", "safe");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start).len(),
            1,
            "{wrapper}"
        );
        let edits = features::rename(&ws, &main, query.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits[&main].len(), 2);
        let doc = &ws.documents[&main];
        let at = doc.text.position(doc.text.source.find(r"\ref").unwrap());
        let help = signature::help(&ws, &main, at).expect(wrapper);
        assert_eq!(help.signatures[0].label, format!(r"\{wrapper}{{text}}"));
    }
}

#[test]
fn initialized_passive_entrydata_resolves_final_sets_and_keeps_complete_edits() {
    for wrapper in ["mkbibparens", "mkbibbrackets", "parentext", "brackettext"] {
        for star in ["", "*"] {
            let body = format!(
                r"\{wrapper}{{\entrydata{star}{{collection}}{{Text}}}}\defbibentryset{{collection}}{{member}}\cite{{collection}}"
            );
            let (_dir, ws, main, _bib) = setup(&body);
            let query = location(&ws, &main, &format!(r"\entrydata{star}{{"), "collection");
            let declaration = location(&ws, &main, r"\defbibentryset{", "collection");
            assert_eq!(
                features::definitions(&ws, &main, query.range.start),
                vec![declaration],
                "{wrapper}{star}"
            );
            assert!(features::hover(&ws, &main, query.range.start).is_some());
            assert_eq!(
                features::references(&ws, &main, query.range.start, true).len(),
                3
            );
            assert!(features::prepare_rename(&ws, &main, query.range.start)
                .unwrap()
                .is_some());
            let changes = features::rename(&ws, &main, query.range.start, "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[&main].len(), 3);
            assert_eq!(
                apply(&ws.documents[&main].text.source, &changes[&main]),
                ws.documents[&main]
                    .text
                    .source
                    .replace("collection", "fresh")
            );
        }
    }
}

#[test]
fn initialized_entrydata_completes_the_final_namespace_without_loading_a_book() {
    for star in ["", "*"] {
        for key in ["co¦BAD", "¦"] {
            let body = format!(
                r"\mkbibparens{{\entrydata{star}{{{key}}}{{Text}}}}\defbibentryset{{collection}}{{member}}\cite{{collection}}"
            );
            let marked = body.find('¦').unwrap();
            let (_dir, ws, main, _bib) = setup(&body.replace('¦', ""));
            let doc = &ws.documents[&main];
            let start = doc.text.source.find(r"\mkbibparens").unwrap();
            let at = doc.text.position(start + marked);
            let item = features::completion(&ws, &main, at)
                .into_iter()
                .find(|item| item.label == "collection")
                .expect("final set candidate");
            assert!(item
                .detail
                .as_ref()
                .unwrap()
                .contains("Dynamic bibliography set"));
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!()
            };
            let accepted = apply(&doc.text.source, &[edit]);
            assert!(accepted.contains(&format!(r"\entrydata{star}{{collection}}{{Text}}")));
        }
    }
}

#[test]
fn named_initializers_read_final_sets_and_complete_the_actual_text_argument() {
    for wrapper in ["mkbibparens", "mkbibbrackets", "parentext", "brackettext"] {
        for call in [
            format!(r"\csuse{{{wrapper}}}"),
            format!("\\csname {wrapper}\\endcsname"),
        ] {
            for star in ["", "*"] {
                let body = format!(
                    "{call}% 中文\r\n{{😀 \\entrydata{star}{{collection}}{{Text}}}}\\defbibentryset{{collection}}{{member}}\\cite{{collection}}"
                );
                let (_dir, ws, main, _bib) = setup(&body);
                let prefix = format!(r"\entrydata{star}{{");
                let query = location(&ws, &main, &prefix, "collection");
                assert_eq!(
                    features::definitions(&ws, &main, query.range.start),
                    vec![location(&ws, &main, r"\defbibentryset{", "collection")],
                    "{call}{star}"
                );
                assert!(features::hover(&ws, &main, query.range.start).is_some());
                assert!(features::prepare_rename(&ws, &main, query.range.start)
                    .unwrap()
                    .is_some());
                assert_eq!(
                    features::references(&ws, &main, query.range.start, true).len(),
                    3
                );
                let changes = features::rename(&ws, &main, query.range.start, "fresh")
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(changes.len(), 1);
                assert_eq!(changes[&main].len(), 3);
                assert_eq!(
                    apply(&ws.documents[&main].text.source, &changes[&main]),
                    ws.documents[&main]
                        .text
                        .source
                        .replace("collection", "fresh")
                );

                for key in ["coBAD", ""] {
                    let (_dir, ws, main, _bib) = setup(&body.replacen(
                        &format!("{prefix}collection"),
                        &format!("{prefix}{key}"),
                        1,
                    ));
                    let doc = &ws.documents[&main];
                    let at =
                        doc.text.source.find(&prefix).unwrap() + prefix.len() + key.len().min(2);
                    let item = features::completion(&ws, &main, doc.text.position(at))
                        .into_iter()
                        .find(|item| item.label == "collection")
                        .expect(&call);
                    assert!(item
                        .detail
                        .as_ref()
                        .unwrap()
                        .contains("Dynamic bibliography set"));
                    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                        panic!()
                    };
                    assert!(apply(&doc.text.source, &[edit])
                        .contains(&format!(r"\entrydata{star}{{collection}}{{Text}}")));
                }
            }
        }
    }
}

#[test]
fn passive_queries_keep_early_books_missing_keys_and_new_name_capture_separate() {
    let (_dir, ws, main, bib) = setup(
        r"\cite{collection}\mkbibparens{\entrydata{collection}{Text}}\defbibentryset{collection}{member}\cite{collection}",
    );
    let query = location(&ws, &main, r"\entrydata{", "collection");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![location(&ws, &bib, "@book{", "collection")]
    );
    let (_dir, ws, main, _bib) = setup(
        r"\mkbibparens{\entrydata{capture}{Text}}\defbibentryset{collection}{member}\cite{collection}",
    );
    let declaration = location(&ws, &main, r"\defbibentryset{", "collection");
    assert!(features::rename(&ws, &main, declaration.range.start, "capture").is_err());
    assert!(
        features::rename(&ws, &main, declaration.range.start, "fresh")
            .unwrap()
            .is_some()
    );
    assert!(ws
        .diagnostics(&main)
        .iter()
        .all(|diagnostic| diagnostic.code
            != Some(NumberOrString::String("undefined-citation".into()))));
}

#[test]
fn uninitialized_or_executable_entrydata_never_allows_a_partial_source_rename() {
    for query in [
        r"\entrydata{collection}{Text}",
        r"\mkbibparens{Text}\entrydata{collection}{Text}",
        r"\unknown{\entrydata{collection}{Text}}",
        r"\mkbibparens*{\entrydata{collection}{Text}}",
        r"\mkbibparens[note]{\entrydata{collection}{Text}}",
        r"\mkbibparens{\entrydata{collection}}{Text}",
        r"\mkbibparens{\entrydata{collection}{\unknown}}",
        r"\mkbibparens{\entrydata{collection}{a_b}}",
        r"\mkbibparens{\entrydata{collection,member}{Text}}",
        r"\def\mkbibparens#1{}\mkbibparens{\entrydata{collection}{Text}}",
        r"\def\entrydata#1#2{}\mkbibparens{\entrydata{collection}{Text}}",
        r"\newcommand{\drop}[1]{}\drop{\mkbibparens{\entrydata{collection}{Text}}}",
        r"\def\dormant{\mkbibparens{\entrydata{collection}{Text}}}",
        r"\let\copied\mkbibparens\copied{\entrydata{collection}{Text}}",
        r"\csuse{mkbibparens}*{\entrydata{collection}{Text}}",
        r"\csname mkbibparens\endcsname[note]{\entrydata{collection}{Text}}",
        r"\csuse{mkbibparens}{\entrydata{collection}}{Text}",
        r"\csname mkbibparens\endcsname{\entrydata{collection}{\unknown}}",
        r"\def\csuse#1#2{}\csuse{mkbibparens}{\entrydata{collection}{Text}}",
        r"\def\mkbibparens#1{}\csuse{mkbibparens}{\entrydata{collection}{Text}}",
        r"\def\mkbibparens#1{}\csname parentext\endcsname{\entrydata{collection}{Text}}",
        r"\newcommand{\drop}[1]{}\drop{\csuse{mkbibparens}{\entrydata{collection}{Text}}}",
        r"\def\dormant{\csname mkbibparens\endcsname{\entrydata{collection}{Text}}}",
        r"\let\copied\mkbibparens\csuse{copied}{\entrydata{collection}{Text}}",
        r"\def\wrap{\mkbibparens}\csuse{wrap}{\entrydata{collection}{Text}}",
    ] {
        let (_dir, ws, main, _bib) = setup(&format!(
            r"{query}\defbibentryset{{collection}}{{member}}\cite{{collection}}"
        ));
        let at = location(&ws, &main, r"\defbibentryset{", "collection")
            .range
            .start;
        assert!(
            !matches!(features::rename(&ws, &main, at, "fresh"), Ok(Some(_))),
            "{query}"
        );
    }
}

#[test]
fn static_named_entrydata_and_nested_initializers_keep_exact_physical_keys() {
    for call in [r"\csuse{entrydata}", r"\csname entrydata\endcsname"] {
        let body = format!(
            r"\mkbibparens{{\mkbibbrackets{{{call}*{{collection}}{{Text}}}}}}\defbibentryset{{collection}}{{member}}\cite{{collection}}"
        );
        let (_dir, ws, main, _bib) = setup(&body);
        let query = location(&ws, &main, &format!("{call}*{{"), "collection");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![location(&ws, &main, r"\defbibentryset{", "collection")]
        );
        let edit = features::rename(&ws, &main, query.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edit[&main].len(), 3);
    }
}

#[test]
fn shared_initialized_queries_require_each_executing_roots_final_identity() {
    let (dir, mut ws, main, _bib) =
        setup(r"\input{query}\defbibentryset{collection}{member}\cite{collection}\input{query}");
    std::fs::write(
        dir.path().join("query.tex"),
        r"\mkbibparens{\entrydata{collection}{Text}}",
    )
    .unwrap();
    ws.open(main.clone(), ws.documents[&main].text.source.clone(), 6);
    let child = main.join("query.tex").unwrap();
    let query = location(&ws, &child, r"\entrydata{", "collection");
    assert_eq!(
        features::definitions(&ws, &child, query.range.start),
        vec![location(&ws, &main, r"\defbibentryset{", "collection")]
    );
    let edit = features::rename(&ws, &child, query.range.start, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edit[&child].len(), 1);
    assert_eq!(edit[&main].len(), 2);
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other, r"\documentclass{article}\usepackage{biblatex}\addbibresource{data.bib}\begin{document}\input{query}\cite{collection}\end{document}".into(), 8);
    assert!(features::definitions(&ws, &child, query.range.start).is_empty());
    assert!(features::rename(&ws, &child, query.range.start, "fresh").is_err());
    assert_eq!(ws.documents[&main].version, Some(6));
}

#[test]
fn generated_code_revokes_initialized_queries_and_literal_examples_stay_independent() {
    let (_dir, mut ws, main, bib) = setup(
        r"\mkbibparens{\entrydata{collection}{Text}}\defbibentryset{collection}{member}\cite{collection}",
    );
    let query = location(&ws, &main, r"\entrydata{", "collection");
    let plain = ws.documents[&bib].text.source.clone();
    for extra in [
        " @book{trigger,execute={\\unknown}}",
        " @preamble{\"\\unknown\"}",
    ] {
        ws.open(bib.clone(), format!("{plain}{extra}"), 8);
        assert!(features::definitions(&ws, &main, query.range.start).is_empty());
        assert!(!matches!(
            features::rename(&ws, &main, query.range.start, "fresh"),
            Ok(Some(_))
        ));
    }
    ws.open(bib, plain, 9);
    assert_eq!(
        features::definitions(&ws, &main, query.range.start).len(),
        1
    );
    let (_dir, ws, main, _bib) = setup(
        r"\detokenize{\mkbibparens{\entrydata{collection}{Text}}}\defbibentryset{collection}{member}\cite{collection}",
    );
    let declaration = location(&ws, &main, r"\defbibentryset{", "collection");
    let edits = features::rename(&ws, &main, declaration.range.start, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits[&main].len(), 2);
    assert!(apply(&ws.documents[&main].text.source, &edits[&main])
        .contains(r"\entrydata{collection}{Text}"));
}

#[test]
fn native_forwarding_wrappers_revoke_execution_when_their_target_is_redefined() {
    for (wrapper, target, independent) in [
        ("parentext", "mkbibparens", "brackettext"),
        ("brackettext", "mkbibbrackets", "parentext"),
    ] {
        let body = format!(
            r"\renewcommand\{target}[1]{{}}\label{{safe}}\{wrapper}{{\ref{{safe}}\entrydata{{collection}}{{Text}}}}\defbibentryset{{collection}}{{member}}\cite{{collection}}"
        );
        let (_dir, ws, main, _bib) = setup(&body);
        let reference = location(&ws, &main, r"\ref{", "safe");
        assert!(
            features::definitions(&ws, &main, reference.range.start).is_empty(),
            "{wrapper}"
        );
        let doc = &ws.documents[&main];
        let at = doc.text.position(doc.text.source.find(r"\ref").unwrap());
        assert!(signature::help(&ws, &main, at).is_none(), "{wrapper}");
        let declaration = location(&ws, &main, r"\defbibentryset{", "collection");
        assert!(
            !matches!(
                features::rename(&ws, &main, declaration.range.start, "fresh"),
                Ok(Some(_))
            ),
            "{wrapper}"
        );

        // An unrelated native initializer keeps its own actual meaning.
        let (_dir, ws, main, _bib) = setup(&format!(
            r"\renewcommand\{target}[1]{{}}\{independent}{{\entrydata{{collection}}{{Text}}}}\defbibentryset{{collection}}{{member}}\cite{{collection}}"
        ));
        let query = location(&ws, &main, r"\entrydata{", "collection");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![location(&ws, &main, r"\defbibentryset{", "collection")]
        );
    }
}

fn compile_clean(dir: &std::path::Path) -> String {
    for extension in ["aux", "bcf", "bbl", "run.xml"] {
        let path = dir.join(format!("main.{extension}"));
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
    }
    for program in [
        "pdflatex", "biber", "pdflatex", "biber", "pdflatex", "pdflatex",
    ] {
        let args = if program == "biber" {
            vec!["main"]
        } else {
            vec![
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ]
        };
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let log = std::fs::read_to_string(dir.join("main.log")).unwrap();
    assert!(
        !log.contains("Warning") && !log.contains("undefined"),
        "{log}"
    );
    let log = std::fs::read_to_string(dir.join("main.blg")).unwrap();
    assert!(!log.contains("WARN") && !log.contains("ERROR"), "{log}");
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_initialized_queries_preserve_complete_bbl_after_actual_lsp_edits() {
    native_initialized_query_edits(&["mkbibparens", "mkbibbrackets", "parentext", "brackettext"]);
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_named_initialized_queries_preserve_complete_bbl_after_actual_lsp_edits() {
    native_initialized_query_edits(&[
        "csuse{mkbibparens}",
        "csname mkbibbrackets\\endcsname",
        "csuse{parentext}",
        "csname brackettext\\endcsname",
    ]);
}

fn native_initialized_query_edits(wrappers: &[&str]) {
    for wrapper in wrappers {
        for star in ["", "*"] {
            // Query before declaration: entrydata reads the final BBL without
            // manufacturing an early request for the database's original book.
            let body = format!(
                r"\{wrapper}{{\entrydata{star}{{collection}}{{Text}}}}\defbibentryset{{collection}}{{member}}\cite{{collection}}\printbibliography"
            );
            let (dir, ws, main, bib) = setup(&body);
            let before = compile_clean(dir.path());
            assert!(before.contains(r"\entry{collection}{set}"), "{before}");
            assert!(!before.contains(r"\entry{collection}{book}"), "{before}");
            let query = location(&ws, &main, &format!(r"\entrydata{star}{{"), "collection");
            let changes = features::rename(&ws, &main, query.range.start, "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[&main].len(), 3);
            for (uri, edits) in changes {
                std::fs::write(
                    uri.to_file_path().unwrap(),
                    apply(&ws.documents[&uri].text.source, &edits),
                )
                .unwrap();
            }
            assert_eq!(
                std::fs::read_to_string(bib.to_file_path().unwrap()).unwrap(),
                ws.documents[&bib].text.source
            );
            assert_eq!(
                compile_clean(dir.path()).replace("fresh", "collection"),
                before,
                "{wrapper}{star}"
            );

            // Accept the real completion over a complete invalid suffix. It
            // must restore the original BBL rather than preload the old book.
            let incomplete = ws.documents[&main].text.source.replacen(
                &format!(r"\entrydata{star}{{collection}}"),
                &format!(r"\entrydata{star}{{coBAD}}"),
                1,
            );
            let mut completing = Workspace::default();
            completing.config.distribution.enabled = false;
            completing.open(main.clone(), incomplete, 9);
            let doc = &completing.documents[&main];
            let at = doc
                .text
                .position(doc.text.source.find("coBAD").unwrap() + 2);
            let item = features::completion(&completing, &main, at)
                .into_iter()
                .find(|item| item.label == "collection")
                .unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!()
            };
            std::fs::write(
                main.to_file_path().unwrap(),
                apply(&doc.text.source, &[edit]),
            )
            .unwrap();
            assert_eq!(
                compile_clean(dir.path()),
                before,
                "{wrapper}{star} completion"
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_redefined_forward_targets_discard_queries_without_phantom_references() {
    for (wrapper, target) in [
        ("parentext", "mkbibparens"),
        ("brackettext", "mkbibbrackets"),
    ] {
        let body = format!(
            r"\renewcommand\{target}[1]{{}}\label{{safe}}\{wrapper}{{\ref{{safe}}\entrydata{{collection}}{{Text}}}}\defbibentryset{{collection}}{{member}}\cite{{collection}}\printbibliography"
        );
        let (dir, ws, main, _bib) = setup(&body);
        let before = compile_clean(dir.path());
        assert!(before.contains(r"\entry{collection}{set}"));
        let source = &ws.documents[&main].text.source;
        std::fs::write(
            dir.path().join("main.tex"),
            source.replace(r"\ref{safe}", r"\typeout{LSP-EXECUTED:yes}\ref{safe}"),
        )
        .unwrap();
        assert_eq!(compile_clean(dir.path()), before);
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        assert!(!log.contains("LSP-EXECUTED:yes"), "{log}");
        let reference = location(&ws, &main, r"\ref{", "safe");
        assert!(features::definitions(&ws, &main, reference.range.start).is_empty());
        let declaration = location(&ws, &main, r"\defbibentryset{", "collection");
        assert!(!matches!(
            features::rename(&ws, &main, declaration.range.start, "fresh"),
            Ok(Some(_))
        ));
    }
}
