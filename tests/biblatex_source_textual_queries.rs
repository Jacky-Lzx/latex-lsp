use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;
fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source=format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{biblatex}}\\DeclareBibliographyCategory{{group}}\\addbibresource{{data.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(
        dir.path().join("data.bib"),
        "@book{parent,title={Raw parent}} @book{member,title={Member}} @book{other,title={Other}}",
    )
    .unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = main.join("data.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, bib)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let at = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert_eq!(&doc.text.source[at..at + name.len()], name);
    Location::new(uri.clone(), doc.text.range(at, at + name.len()))
}
#[test]
fn source_predicates_resolve_final_sets_without_becoming_early_book_requests() {
    for predicate in [
        r"\ifentryinbib{parent}{Present}{Missing}",
        r"\ifentrycategory{parent}{group}{Yes}{No}",
    ] {
        let (_dir, ws, main, _bib) = setup(&format!(
            r"{predicate}\defbibentryset{{parent}}{{member}}\cite{{parent}}"
        ));
        let query = key(
            &ws,
            &main,
            &predicate[..predicate.find("parent").unwrap()],
            "parent",
        );
        let declaration = key(&ws, &main, r"\defbibentryset{", "parent");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![declaration.clone()],
            "{predicate}"
        );
        assert_eq!(
            features::references(&ws, &main, declaration.range.start, true).len(),
            3
        );
        let edits = features::rename(&ws, &main, query.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[&main].len(), 3);
    }
}
#[test]
fn source_predicate_queries_keep_early_books_and_new_name_capture_independent() {
    let (_dir, ws, main, bib) = setup(
        r"\cite{parent}\ifentryinbib{parent}{Present}{Missing}\defbibentryset{parent}{member}\cite{parent}",
    );
    let query = key(&ws, &main, r"\ifentryinbib{", "parent");
    let book = key(&ws, &bib, "@book{", "parent");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![book]
    );
    assert_eq!(
        features::references(&ws, &main, query.range.start, true).len(),
        4
    );
    let (_dir, ws, main, _bib) = setup(
        r"\ifentryinbib{capture}{Present}{Missing}\defbibentryset{parent}{member}\cite{parent}",
    );
    let declaration = key(&ws, &main, r"\defbibentryset{", "parent");
    assert!(features::rename(&ws, &main, declaration.range.start, "capture").is_err());
    assert!(
        features::rename(&ws, &main, declaration.range.start, "fresh")
            .unwrap()
            .is_some()
    );
}

fn warnings(ws: &Workspace, uri: &Url) -> Vec<String> {
    ws.diagnostics(uri)
        .into_iter()
        .filter(|diag| diag.code == Some(NumberOrString::String("undefined-citation".into())))
        .map(|diag| diag.data.unwrap()["key"].as_str().unwrap().to_string())
        .collect()
}
#[test]
fn declarations_materialize_sets_without_explicit_citations_and_queries_do_not_warn() {
    let (_dir, ws, main, _bib) = setup(
        r"\ifentryinbib{parent}{存在}{缺失}\ifentrycategory{absent}{group}{}{}\defbibentryset{parent}{member}",
    );
    let query = key(&ws, &main, r"\ifentryinbib{", "parent");
    let declaration = key(&ws, &main, r"\defbibentryset{", "parent");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![declaration]
    );
    assert_eq!(
        features::references(&ws, &main, query.range.start, true).len(),
        2
    );
    assert_eq!(
        features::rename(&ws, &main, query.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&main]
            .len(),
        2
    );
    assert!(warnings(&ws, &main).is_empty());
    let (_dir, ws, main, _bib) =
        setup(r"\ifentryinbib{absent}{}{}\defbibentryset{parent}{missingmember}\cite{missingcite}");
    assert_eq!(warnings(&ws, &main), ["missingmember", "missingcite"]);
}
#[test]
fn early_missing_requests_remain_missing_after_later_source_declarations() {
    let (_dir, ws, main, _bib) = setup(
        r"\cite{collected}\ifentryinbib{collected}{}{}\defbibentryset{collected}{member}\cite{collected}",
    );
    let query = key(&ws, &main, r"\ifentryinbib{", "collected");
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    assert_eq!(warnings(&ws, &main), ["collected", "collected"]);
}
#[test]
fn named_queries_and_comments_keep_exact_key_edits_and_final_completion() {
    for predicate in [
        r"\csuse{ifentryinbib}{parent}{Literal parent}{No parent}",
        r"\csname ifentrycategory\endcsname{parent}{group}{存在}{缺失}",
        "\\ifentryinbib{parent}{% \\newrefsection\nYes}{No}",
    ] {
        let (_dir, ws, main, _bib) = setup(&format!(
            r"{predicate}\defbibentryset{{parent}}{{member}}\cite{{parent}}"
        ));
        let query = key(
            &ws,
            &main,
            &predicate[..predicate.find("parent").unwrap()],
            "parent",
        );
        let declaration = key(&ws, &main, r"\defbibentryset{", "parent");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![declaration.clone()]
        );
        let changes = features::rename(&ws, &main, query.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        let cite = key(&ws, &main, r"\cite{", "parent");
        let mut actual = changes[&main]
            .iter()
            .map(|edit| {
                assert_eq!(edit.new_text, "fresh");
                edit.range
            })
            .collect::<Vec<_>>();
        let mut expected = vec![query.range, declaration.range, cite.range];
        actual.sort_by_key(|range| range.start);
        expected.sort_by_key(|range| range.start);
        assert_eq!(actual, expected);
        assert!(features::completion(&ws, &main, cite.range.start)
            .iter()
            .any(|item| item.label == "parent"));
        assert!(features::completion(&ws, &main, query.range.start)
            .iter()
            .any(|item| item.label == "parent"));
    }
}
#[test]
fn shared_physical_queries_require_each_final_namespace_to_agree() {
    for conflict in [false, true] {
        let (dir, mut ws, main, bib) = setup(&format!(
            r"\defbibentryset{{parent}}{{member}}\input{{query}}\newrefsection{}\input{{query}}",
            if conflict {
                ""
            } else {
                r"\defbibentryset{parent}{member}"
            }
        ));
        std::fs::write(
            dir.path().join("query.tex"),
            r"\ifentryinbib{parent}{Yes}{No}",
        )
        .unwrap();
        let source = ws.documents[&main].text.source.clone();
        ws.open(main.clone(), source, 6);
        let child = main.join("query.tex").unwrap();
        let query = key(&ws, &child, r"\ifentryinbib{", "parent");
        // Even two distinct declarations with equal spelling are different identities.
        assert!(features::definitions(&ws, &child, query.range.start).is_empty());
        assert!(features::rename(&ws, &child, query.range.start, "fresh").is_err());
        assert!(features::references(
            &ws,
            &bib,
            key(&ws, &bib, "@book{", "parent").range.start,
            true
        )
        .is_empty());
    }
    let (dir, mut ws, main, _bib) =
        setup(r"\defbibentryset{parent}{member}\input{query}\input{query}");
    std::fs::write(dir.path().join("query.tex"), r"\ifentryinbib{parent}{}{}").unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 6);
    let child = main.join("query.tex").unwrap();
    let query = key(&ws, &child, r"\ifentryinbib{", "parent");
    assert_eq!(
        features::references(&ws, &child, query.range.start, true).len(),
        2
    );
    assert_eq!(
        features::rename(&ws, &child, query.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&child]
            .len(),
        1
    );
}
#[test]
fn unsaved_query_meanings_and_ids_intersections_keep_full_graphs_protected() {
    for extra in [
        r"\ifentryinbib{parent}{\newrefsection}{}",
        r"\ifentrycategory{parent}{\unknown}{}{}",
        r"\entrydata{parent}{}",
        r"\ifentryexists{parent}{}{}",
    ] {
        let (_dir, ws, main, _bib) = setup(&format!(
            r"\defbibentryset{{parent}}{{member}}\cite{{parent}}{extra}"
        ));
        let declaration = key(&ws, &main, r"\defbibentryset{", "parent");
        assert!(
            features::references(&ws, &main, declaration.range.start, true).is_empty(),
            "{extra}"
        );
        assert!(
            features::rename(&ws, &main, declaration.range.start, "fresh").is_err(),
            "{extra}"
        );
    }
    let (dir, mut ws, main, bib) =
        setup(r"\input{macros}\ifentryinbib{parent}{}{}\defbibentryset{parent}{member}");
    std::fs::write(dir.path().join("macros.tex"), "").unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 6);
    let query = key(&ws, &main, r"\ifentryinbib{", "parent");
    assert_eq!(
        features::references(&ws, &main, query.range.start, true).len(),
        2
    );
    let provider = main.join("macros.tex").unwrap();
    ws.open(
        provider.clone(),
        r"\renewcommand\ifentryinbib[3]{}".into(),
        7,
    );
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    ws.open(provider, String::new(), 8);
    assert_eq!(
        features::references(&ws, &main, query.range.start, true).len(),
        2
    );
    let library = ws.documents[&bib]
        .text
        .source
        .replace("title={Member}", "title={Member},ids={old}");
    ws.open(bib.clone(), library, 8);
    assert!(features::rename(&ws, &main, query.range.start, "fresh").is_err());
    ws.open(
        bib,
        std::fs::read_to_string(dir.path().join("data.bib")).unwrap(),
        9,
    );
    assert_eq!(
        features::references(&ws, &main, query.range.start, true).len(),
        2
    );
}

fn compile_case(dir: &std::path::Path, allow_missing: bool) -> String {
    for (program, args) in [
        (
            "pdflatex",
            vec![
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ],
        ),
        ("biber", vec!["main"]),
        (
            "pdflatex",
            vec![
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ],
        ),
        (
            "pdflatex",
            vec![
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ],
        ),
    ] {
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
    assert!(allow_missing || !log.contains("undefined"), "{log}");
    let blg = std::fs::read_to_string(dir.join("main.blg")).unwrap();
    assert!(
        (allow_missing || !blg.contains("WARN")) && !blg.contains("ERROR"),
        "{blg}"
    );
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}

fn compile(dir: &std::path::Path) -> String {
    compile_case(dir, false)
}

fn clean_artifacts(dir: &std::path::Path) {
    for extension in ["aux", "bcf", "bbl", "run.xml"] {
        let path = dir.join(format!("main.{extension}"));
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
    }
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_source_predicates_preserve_loaded_lists_and_complete_set_renames() {
    for early in [false, true] {
        for explicit in [false, true] {
            let predicate =
                r"\ifentryinbib{parent}{Present}{Missing}\ifentrycategory{parent}{group}{Yes}{No}";
            let body = format!(
                r"{}{predicate}\defbibentryset{{parent}}{{member}}{}\printbibliography",
                if early { r"\cite{parent}" } else { "" },
                if explicit { r"\cite{parent}" } else { "" }
            );
            let (dir, ws, main, _bib) = setup(&body);
            let before = compile(dir.path());
            let query = key(&ws, &main, r"\ifentryinbib{", "parent");
            if early {
                // Renaming the locked book could activate the suppressed set.
                assert!(features::rename(&ws, &main, query.range.start, "fresh").is_err());
                let source = ws.documents[&main].text.source.replace(predicate, "");
                std::fs::write(dir.path().join("main.tex"), source).unwrap();
                clean_artifacts(dir.path());
                assert_eq!(compile(dir.path()), before);
                continue;
            }
            let changes = features::rename(&ws, &main, query.range.start, "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            for (uri, edits) in changes {
                let doc = &ws.documents[&uri];
                let mut source = doc.text.source.clone();
                let mut spans = edits
                    .into_iter()
                    .map(|edit| {
                        (
                            doc.text.offset(edit.range.start).unwrap(),
                            doc.text.offset(edit.range.end).unwrap(),
                            edit.new_text,
                        )
                    })
                    .collect::<Vec<_>>();
                spans.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
                for (start, end, text) in spans {
                    source.replace_range(start..end, &text);
                }
                std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
            }
            clean_artifacts(dir.path());
            assert_eq!(compile(dir.path()).replace("fresh", "parent"), before);
            // Removing both pure-text queries must leave every backend record unchanged.
            let source = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
            std::fs::write(
                dir.path().join("main.tex"),
                source.replace(&predicate.replace("parent", "fresh"), ""),
            )
            .unwrap();
            clean_artifacts(dir.path());
            assert_eq!(compile(dir.path()).replace("fresh", "parent"), before);
        }
    }
}
