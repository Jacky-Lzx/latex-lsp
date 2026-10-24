use latex_lsp::{features, workspace::Workspace};
use tower_lsp::lsp_types::*;
fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{refs.bib}}\begin{{document}}{body}\end{{document}}"
    );
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(
        dir.path().join("refs.bib"),
        "@book{parent,title={Raw}} @book{member,title={Member}}",
    )
    .unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = main.join("refs.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, bib)
}
fn location(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let d = &ws.documents[uri];
    let start = d.text.source.find(prefix).unwrap() + prefix.len();
    assert_eq!(&d.text.source[start..start + name.len()], name);
    Location::new(uri.clone(), d.text.range(start, start + name.len()))
}
#[test]
fn late_default_requests_resolve_only_to_the_materialized_source_set() {
    let (_dir, ws, main, _bib) = setup(r"\defbibentryset{parent}{member}\cite{parent}");
    let query = location(&ws, &main, r"\cite{", "parent");
    let target = location(&ws, &main, r"\defbibentryset{", "parent");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![target]
    );
}
#[test]
fn early_default_requests_resolve_only_to_the_database_book() {
    let (_dir, ws, main, bib) = setup(r"\cite{parent}\defbibentryset{parent}{member}\cite{parent}");
    let query = location(&ws, &main, r"\cite{", "parent");
    let target = location(&ws, &bib, "@book{", "parent");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![target]
    );
}
#[test]
fn early_default_source_only_requests_remain_undefined() {
    let (_dir, ws, main, _) =
        setup(r"\cite{collected}\defbibentryset{collected}{member}\cite{collected}");
    let missing = ws
        .diagnostics(&main)
        .into_iter()
        .filter(|d| d.code == Some(NumberOrString::String("undefined-citation".into())))
        .count();
    assert_eq!(missing, 2);
}
#[test]
fn default_candidates_preview_the_identity_created_by_the_first_accepted_request() {
    for (body, wanted, detail) in [
        (
            r"\defbibentryset{parent}{member}\cite{pa}",
            "parent",
            Some("Dynamic bibliography set"),
        ),
        (
            r"\cite{pa}\defbibentryset{parent}{member}",
            "parent",
            Some("Raw"),
        ),
        (
            r"\defbibentryset{collected}{member}\cite{coll}",
            "collected",
            Some("Dynamic bibliography set"),
        ),
        (
            r"\cite{coll}\defbibentryset{collected}{member}",
            "collected",
            None,
        ),
    ] {
        let (_dir, ws, main, _) = setup(body);
        let text = &ws.documents[&main].text;
        let start = text.source.find(r"\cite{").unwrap() + r"\cite{".len();
        let end = start + text.source[start..].find('}').unwrap();
        let items = features::completion(&ws, &main, text.position(end));
        let candidate = items.iter().find(|item| item.label == wanted);
        if let Some(detail) = detail {
            assert!(
                candidate.unwrap().detail.as_ref().unwrap().contains(detail),
                "{items:?}"
            );
        } else {
            assert!(candidate.is_none(), "{items:?}");
        }
    }
}
#[test]
fn default_coupled_parent_edits_require_every_saved_and_unsaved_consumer() {
    for field in ["crossref", "xref"] {
        let (_dir, mut ws, main, bib) =
            setup(r"\defbibentryset{parent}{member}\cite{parent,child}");
        ws.open(
            bib.clone(),
            format!(
                "@book{{parent,title={{Raw}}}} @book{{member}} @book{{child,{field}={{parent}}}}"
            ),
            8,
        );
        let query = location(&ws, &main, r"\defbibentryset{", "parent");
        let edit = features::rename(&ws, &main, query.range.start, "newset")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edit.values().map(Vec::len).sum::<usize>(), 4, "{edit:?}");
        assert_eq!(edit[&main].len(), 2);
        assert_eq!(edit[&bib].len(), 2);
        let raw = location(&ws, &bib, "@book{", "parent");
        assert!(features::prepare_rename(&ws, &bib, raw.range.start).is_err());
        let other = main.join("unsaved.tex").unwrap();
        ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{refs.bib}\begin{document}\cite{parent}\end{document}".into(),9);
        assert!(features::rename(&ws, &main, query.range.start, "newset").is_err());
        ws.documents.remove(&other);
        assert!(features::rename(&ws, &main, query.range.start, "newset")
            .unwrap()
            .is_some());
    }
}
#[test]
fn default_unsafe_source_executions_do_not_fall_back_to_database_unions() {
    for tail in [
        r"\newcommand\later{\cite{absent}}",
        r"\iftrue\cite{parent}\fi",
        r"\bibitem{manual}",
        r"\entrydata{absent}{}",
        r"\defbibentryset{nested}{parent}",
    ] {
        let (_dir, ws, main, _) = setup(&format!(
            r"\defbibentryset{{parent}}{{member}}\cite{{parent,absent}}\ref{{missinglabel}}{tail}"
        ));
        let query = location(&ws, &main, r"\cite{", "parent");
        assert!(
            features::definitions(&ws, &main, query.range.start).is_empty(),
            "{tail}"
        );
        assert!(
            features::rename(&ws, &main, query.range.start, "newset").is_err(),
            "{tail}"
        );
        let diagnostics = ws.diagnostics(&main);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code == Some(NumberOrString::String("undefined-citation".into()))),
            "{tail}"
        );
        assert!(diagnostics
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("undefined-reference".into()))));
    }
}
#[test]
fn default_member_and_citation_diagnostics_follow_unsaved_provider_updates() {
    let (dir, mut ws, main, _) = setup(r"\input{sets}\cite{collectd}");
    let sets = main.join("sets.tex").unwrap();
    let declaration = r"\defbibentryset{collected}{membr}";
    std::fs::write(dir.path().join("sets.tex"), declaration).unwrap();
    ws.open(sets.clone(), declaration.into(), 3);
    let count = |ws: &Workspace, uri: &Url| {
        ws.diagnostics(uri)
            .into_iter()
            .filter(|d| d.code == Some(NumberOrString::String("undefined-citation".into())))
            .count()
    };
    assert_eq!(count(&ws, &main), 1);
    assert_eq!(count(&ws, &sets), 1);
    ws.open(sets.clone(), r"\defbibentryset{collectd}{member}".into(), 4);
    assert_eq!(count(&ws, &main), 0);
    assert_eq!(count(&ws, &sets), 0);
    let query = location(&ws, &main, r"\cite{", "collectd");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![location(&ws, &sets, r"\defbibentryset{", "collectd")]
    );
}
fn compile(dir: &std::path::Path, defined: bool) -> String {
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
    assert_eq!(!log.contains("undefined"), defined, "{log}");
    let blg = std::fs::read_to_string(dir.join("main.blg")).unwrap();
    assert!(!blg.contains("ERROR"), "{blg}");
    assert_eq!(!blg.contains("WARN"), defined, "{blg}");
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_default_source_identity_and_clean_renames_match_the_lsp() {
    for (body, kind, defined) in [
        (r"\defbibentryset{parent}{member}\cite{parent}", "set", true),
        (
            r"\cite{parent}\defbibentryset{parent}{member}\cite{parent}",
            "book",
            true,
        ),
        (
            r"\cite{collected}\defbibentryset{collected}{member}\cite{collected}",
            "missing",
            false,
        ),
    ] {
        let (dir, mut ws, main, bib) = setup(body);
        let before = compile(dir.path(), defined);
        let query = location(
            &ws,
            &main,
            r"\cite{",
            if defined { "parent" } else { "collected" },
        );
        let diagnostics = ws.diagnostics(&main);
        let missing = diagnostics
            .iter()
            .filter(|d| d.code == Some(NumberOrString::String("undefined-citation".into())))
            .count();
        if defined {
            assert!(
                before.contains(&format!(r"\entry{{parent}}{{{kind}}}")),
                "{before}"
            );
            let target = if kind == "set" {
                location(&ws, &main, r"\defbibentryset{", "parent")
            } else {
                location(&ws, &bib, "@book{", "parent")
            };
            assert_eq!(
                features::definitions(&ws, &main, query.range.start),
                vec![target]
            );
            assert_eq!(missing, 0);
        } else {
            assert!(!before.contains(r"\entry{collected}"));
            assert_eq!(missing, 2);
        }
        if kind == "set" {
            let edits = features::rename(&ws, &main, query.range.start, "newset")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(edits.len(), 1);
            let mut changes = edits[&main].clone();
            changes.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let text = ws.documents[&main]
                .text
                .changed(
                    changes
                        .into_iter()
                        .map(|edit| TextDocumentContentChangeEvent {
                            range: Some(edit.range),
                            range_length: None,
                            text: edit.new_text,
                        })
                        .collect(),
                )
                .unwrap()
                .source;
            std::fs::write(dir.path().join("main.tex"), &text).unwrap();
            ws.open(main.clone(), text, 6);
            for extension in ["aux", "bcf", "bbl", "run.xml"] {
                let _ = std::fs::remove_file(dir.path().join(format!("main.{extension}")));
            }
            assert_eq!(
                compile(dir.path(), true),
                before.replace("parent", "newset")
            );
        }
    }
}

#[test]
fn raw_book_renames_cannot_activate_a_previously_suppressed_source_set() {
    for selected in [false, true] {
        let body = if selected {
            r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{member}\cite{parent}"
        } else {
            r"\cite{parent}\defbibentryset{parent}{member}\cite{parent}"
        };
        let (_dir, mut ws, main, bib) = setup(body);
        if selected {
            let text = ws.documents[&main].text.source.replace(
                r"\begin{document}",
                r"\addsectionbib[label=chosen]{refs.bib}\begin{document}",
            );
            ws.open(main.clone(), text, 6);
        }
        let raw = location(&ws, &bib, "@book{", "parent");
        assert!(
            features::prepare_rename(&ws, &bib, raw.range.start).is_err(),
            "selected={selected}"
        );
        assert!(
            features::rename(&ws, &bib, raw.range.start, "newbook").is_err(),
            "selected={selected}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_raw_rename_would_activate_the_suppressed_set_and_member() {
    let (dir, ws, main, bib) = setup(r"\cite{parent}\defbibentryset{parent}{member}\cite{parent}");
    let before = compile(dir.path(), true);
    assert!(before.contains(r"\entry{parent}{book}"));
    assert!(!before.contains(r"\entry{member}"));
    let raw = location(&ws, &bib, "@book{", "parent");
    let query = location(&ws, &main, r"\cite{", "parent");
    for target in [&raw, &query] {
        assert!(features::prepare_rename(&ws, &target.uri, target.range.start).is_err());
        assert!(features::rename(&ws, &target.uri, target.range.start, "newbook").is_err());
    }
    // Reproduce the unsafe partial edit as a backend counterexample.
    let text = ws.documents[&main]
        .text
        .source
        .replace(r"\cite{parent}", r"\cite{newbook}");
    std::fs::write(dir.path().join("main.tex"), text).unwrap();
    std::fs::write(
        dir.path().join("refs.bib"),
        ws.documents[&bib]
            .text
            .source
            .replace("@book{parent,", "@book{newbook,"),
    )
    .unwrap();
    for extension in ["aux", "bcf", "bbl", "run.xml"] {
        let _ = std::fs::remove_file(dir.path().join(format!("main.{extension}")));
    }
    let after = compile(dir.path(), true);
    assert!(after.contains(r"\entry{newbook}{book}"));
    assert!(after.contains(r"\entry{parent}{set}"));
    assert!(after.contains(r"\entry{member}{book}"));
}

#[test]
fn named_source_detection_cannot_fall_back_to_a_database_union() {
    for declaration in [
        r"\csuse{defbibentryset}{parent}{member}",
        r"\csname defbibentryset\endcsname{parent}{member}",
        r"\newcommand\setsuffix{set}\csname defbibentry\setsuffix\endcsname{parent}{member}",
        r"\newcommand\setsuffix{set}\csuse{defbibentry\setsuffix}{parent}{member}",
        r"\newcommand\makeaset[2]{\defbibentryset{#1}{#2}}\makeaset{parent}{member}",
        r"\let\capturedset\defbibentryset\capturedset{parent}{member}",
    ] {
        let (_dir, ws, main, bib) = setup(&format!(r"{declaration}\cite{{parent}}"));
        let query = location(&ws, &main, r"\cite{", "parent");
        let raw = location(&ws, &bib, "@book{", "parent");
        let actual = features::definitions(&ws, &main, query.range.start);
        assert!(!actual.contains(&raw), "{declaration}: {actual:?}");
    }
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_expanded_csuse_sets_keep_an_uncertified_namespace_protected() {
    let (dir, ws, main, _) = setup(
        r"\newcommand\setsuffix{set}\csuse{defbibentry\setsuffix}{parent}{member}\cite{parent}",
    );
    let bbl = compile(dir.path(), true);
    assert!(bbl.contains(r"\entry{parent}{set}"));
    assert!(bbl.contains(r"\entry{member}{book}"));
    let query = location(&ws, &main, r"\cite{", "parent");
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    assert!(features::rename(&ws, &main, query.range.start, "newset").is_err());
}

#[test]
fn static_unrelated_calls_and_literal_dynamic_examples_keep_ordinary_book_edits() {
    for prefix in [
        r"\csuse{relax}",
        r"\verb|\csuse{\unknown}|",
        r"\detokenize{\csuse{\unknown}}",
    ] {
        let (_dir, ws, main, bib) = setup(&format!(r"{prefix}\cite{{parent}}"));
        let query = location(&ws, &main, r"\cite{", "parent");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![location(&ws, &bib, "@book{", "parent")],
            "{prefix}",
        );
        let edit = features::rename(&ws, &main, query.range.start, "newbook")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            edit.values().map(Vec::len).sum::<usize>(),
            2,
            "{prefix}: {edit:?}"
        );
    }
}

#[test]
fn incomplete_default_project_signatures_do_not_expose_a_partial_source_union() {
    let definitions = (0..4097)
        .map(|mut i| {
            let mut name = String::from("command");
            loop {
                name.push((b'a' + (i % 26) as u8) as char);
                i /= 26;
                if i == 0 {
                    break;
                }
            }
            format!(r"\newcommand{{\{name}}}{{text}}")
        })
        .collect::<String>();
    let (_dir, mut ws, main, bib) = setup(&format!(
        r"\newcommand\forwardcite{{\cite}}\defbibentryset{{parent}}{{member}}\cite{{parent,absent}}\forwardcite{{parent}}\csuse{{defbibentryset}}{{other}}{{member}}\csuse{{cite}}{{parent}}\csname cite\endcsname{{parent}}\ref{{missinglabel}}{definitions}"
    ));
    assert_eq!(ws.documents[&main].syntax.definitions.len(), 4098);
    let diagnostics = ws.diagnostics(&main);
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("undefined-citation".into()))),
        "definitions={}, commands={}, diagnostics={diagnostics:?}",
        ws.documents[&main].syntax.definitions.len(),
        ws.documents[&main].syntax.commands.len()
    );
    let citation = location(&ws, &main, r"\cite{", "parent");
    assert!(!features::completion(&ws, &main, citation.range.end)
        .iter()
        .any(|item| matches!(item.label.as_str(), "parent" | "member")));
    for query in [
        location(&ws, &main, r"\defbibentryset{", "parent"),
        location(&ws, &main, r"\cite{", "parent"),
        location(&ws, &bib, "@book{", "parent"),
        location(&ws, &main, r"\csuse{cite}{", "parent"),
        location(&ws, &main, r"\cite{parent,absent}\forwardcite{", "parent"),
        location(&ws, &main, r"\csuse{defbibentryset}{", "other"),
        location(&ws, &main, r"\csname cite\endcsname{", "parent"),
    ] {
        let actual = features::references(&ws, &query.uri, query.range.start, true);
        assert!(actual.is_empty(), "{query:?}: {actual:?}");
        assert!(features::rename(&ws, &query.uri, query.range.start, "newset").is_err());
    }
    ws.open(
        bib.clone(),
        "@book{parent,title={Raw}} @book{member} @book{child,crossref={pa}}".into(),
        7,
    );
    let pending = location(&ws, &bib, "crossref={", "pa");
    assert!(!features::completion(&ws, &bib, pending.range.end)
        .iter()
        .any(|item| matches!(item.label.as_str(), "parent" | "member")));
}

#[test]
fn document_bodies_loaded_from_inputs_activate_the_same_complete_project_model() {
    let (dir, mut ws, main, _) = setup(r"\defbibentryset{parent}{member}\cite{parent}");
    std::fs::write(dir.path().join("begin.tex"), r"\begin{document}").unwrap();
    let source = ws.documents[&main]
        .text
        .source
        .replace(r"\begin{document}", r"\input{begin}");
    ws.open(main.clone(), source, 8);
    let query = location(&ws, &main, r"\cite{", "parent");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![location(&ws, &main, r"\defbibentryset{", "parent")],
    );
}
