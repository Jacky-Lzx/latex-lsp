use lsp_types::*;
use texquill::{language::features, project::workspace::Workspace};

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}\addglobalbib{{global.bib}}\begin{{document}}{body}\end{{document}}"
    );
    for (name, text) in [
        ("main.tex", source.as_str()),
        ("default.bib", "@book{parent,ids={legacy},title={Default parent}}\n@book{childdefault,crossref={parent},xref={legacy}}"),
        ("section.bib", "@book{parent,ids={legacy},title={Selected parent}}\n@book{childselected,crossref={parent},xref={legacy}}"),
        ("global.bib", "@book{shared,title={Shared}}\n@book{globalchild,related={shared}}"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main)
}

fn at(ws: &Workspace, uri: &Url, marker: &str) -> Position {
    let doc = &ws.documents[uri];
    doc.text
        .position(doc.text.source.find(marker).unwrap() + marker.len())
}
fn locations(ws: &Workspace, uri: &Url, prefix: &str, key: &str) -> Vec<Location> {
    let doc = &ws.documents[uri];
    doc.text
        .source
        .match_indices(prefix)
        .filter_map(|(offset, _)| {
            let start = offset + prefix.len();
            doc.text.source[start..]
                .starts_with(key)
                .then(|| Location::new(uri.clone(), doc.text.range(start, start + key.len())))
        })
        .collect()
}
fn sorted(mut items: Vec<Location>) -> Vec<Location> {
    items.sort_by_key(|item| {
        (
            item.uri.to_string(),
            item.range.start.line,
            item.range.start.character,
            item.range.end.line,
            item.range.end.character,
        )
    });
    items
}
fn check(
    ws: &Workspace,
    uri: &Url,
    position: Position,
    declaration: &Location,
    expected: &[Location],
) {
    assert_eq!(
        sorted(features::references(ws, uri, position, true)),
        sorted(expected.to_vec())
    );
    assert_eq!(
        sorted(features::references(ws, uri, position, false)),
        sorted(
            expected
                .iter()
                .filter(|item| *item != declaration)
                .cloned()
                .collect()
        )
    );
}

#[test]
fn database_and_source_queries_return_complete_separate_default_and_selected_identities() {
    let (_dir, ws, main) =
        setup(r"\cite{parent}\newrefsection[chosen]\cite{parent}\endrefsection\cite{parent}");
    let calls = locations(&ws, &main, r"\cite{", "parent");
    for (name, indexes) in [("default.bib", vec![0, 2]), ("section.bib", vec![1])] {
        let bib = main.join(name).unwrap();
        let declaration = locations(&ws, &bib, "@book{", "parent").remove(0);
        let mut expected = vec![declaration.clone()];
        expected.extend(locations(&ws, &bib, "crossref={", "parent"));
        expected.extend(indexes.iter().map(|index| calls[*index].clone()));
        check(
            &ws,
            &bib,
            at(&ws, &bib, "@book{par"),
            &declaration,
            &expected,
        );
        check(
            &ws,
            &bib,
            at(&ws, &bib, "crossref={par"),
            &declaration,
            &expected,
        );
        check(
            &ws,
            &main,
            calls[indexes[0]].range.start,
            &declaration,
            &expected,
        );
    }
}

#[test]
fn ids_alias_references_remain_independent_of_primary_keys_in_each_section() {
    let (_dir, ws, main) = setup(r"\cite{legacy}\newrefsection[chosen]\cite{legacy}");
    let calls = locations(&ws, &main, r"\cite{", "legacy");
    for (index, name) in ["default.bib", "section.bib"].into_iter().enumerate() {
        let bib = main.join(name).unwrap();
        let declaration = locations(&ws, &bib, "ids={", "legacy").remove(0);
        let mut expected = vec![declaration.clone(), calls[index].clone()];
        expected.extend(locations(&ws, &bib, "xref={", "legacy"));
        check(
            &ws,
            &bib,
            at(&ws, &bib, "ids={leg"),
            &declaration,
            &expected,
        );
        check(
            &ws,
            &main,
            calls[index].range.start,
            &declaration,
            &expected,
        );
    }
}

#[test]
fn shared_global_identity_survives_repeated_inputs_with_different_resource_sets() {
    let (dir, mut ws, main) = setup(r"\input{chapter}\newrefsection[chosen]\input{chapter}");
    let chapter = main.join("chapter.tex").unwrap();
    std::fs::write(dir.path().join("chapter.tex"), "% 😀\r\n\\cite{shared}").unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 6);
    let global = main.join("global.bib").unwrap();
    let declaration = locations(&ws, &global, "@book{", "shared").remove(0);
    let mut expected = vec![declaration.clone()];
    expected.extend(locations(&ws, &global, "related={", "shared"));
    expected.extend(locations(&ws, &chapter, r"\cite{", "shared"));
    check(
        &ws,
        &chapter,
        at(&ws, &chapter, r"\cite{sha"),
        &declaration,
        &expected,
    );
    check(
        &ws,
        &global,
        at(&ws, &global, "@book{sha"),
        &declaration,
        &expected,
    );
}

#[test]
fn a_shared_source_with_different_bindings_never_returns_a_partial_reference_graph() {
    let (dir, mut ws, main) = setup(r"\input{chapter}\newrefsection[chosen]\input{chapter}");
    let chapter = main.join("chapter.tex").unwrap();
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{parent}").unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 6);
    for name in ["default.bib", "section.bib"] {
        let bib = main.join(name).unwrap();
        assert!(features::references(&ws, &bib, at(&ws, &bib, "@book{par"), true).is_empty());
    }
    assert!(features::references(&ws, &chapter, at(&ws, &chapter, r"\cite{par"), true).is_empty());
}

#[test]
fn all_open_consumers_of_a_shared_global_declaration_contribute_and_unrelated_roots_do_not() {
    let (_dir, mut ws, main) = setup(r"\cite{shared}\newrefsection[chosen]\cite{shared}");
    let global = main.join("global.bib").unwrap();
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other.clone(), r"\documentclass{article}\usepackage{biblatex}\addbibresource{global.bib}\begin{document}\cite{shared}\end{document}".into(), 8);
    let independent = main.join("independent.tex").unwrap();
    ws.open(independent.clone(), r"\documentclass{article}\begin{document}\begin{thebibliography}{1}\bibitem{shared}Independent\end{thebibliography}\cite{shared}\end{document}".into(), 9);
    let declaration = locations(&ws, &global, "@book{", "shared").remove(0);
    let mut expected = vec![declaration.clone()];
    expected.extend(locations(&ws, &global, "related={", "shared"));
    expected.extend(locations(&ws, &main, r"\cite{", "shared"));
    expected.extend(locations(&ws, &other, r"\cite{", "shared"));
    check(
        &ws,
        &global,
        at(&ws, &global, "@book{sha"),
        &declaration,
        &expected,
    );
    assert!(expected.iter().all(|location| location.uri != independent));
}

#[test]
fn unproved_deferred_citations_prevent_a_complete_graph_but_literal_examples_do_not() {
    let (_dir, mut ws, main) = setup(r"\verb|\cite{parent}|\newrefsection[chosen]\cite{parent}");
    let bib = main.join("section.bib").unwrap();
    let declaration = locations(&ws, &bib, "@book{", "parent").remove(0);
    let mut expected = vec![declaration.clone()];
    expected.extend(locations(&ws, &bib, "crossref={", "parent"));
    let source = &ws.documents[&main].text.source;
    let actual = source.rfind(r"\cite{parent}").unwrap() + r"\cite{".len();
    expected.push(Location::new(
        main.clone(),
        ws.documents[&main].text.range(actual, actual + 6),
    ));
    check(
        &ws,
        &bib,
        at(&ws, &bib, "@book{par"),
        &declaration,
        &expected,
    );
    let updated = source.replace(
        r"\begin{document}",
        r"\newcommand\later{\cite{parent}}\begin{document}",
    );
    ws.open(main, updated, 8);
    assert!(features::references(&ws, &bib, at(&ws, &bib, "@book{par"), true).is_empty());
}

#[test]
fn every_relation_member_and_named_citation_uses_exact_unicode_ranges_and_highlight_roles() {
    let (_dir, mut ws, main) =
        setup(r"\csuse{cite}{parent😀}\newrefsection[chosen]\csname cite\endcsname{parent😀}");
    for name in ["default.bib", "section.bib"] {
        let bib = main.join(name).unwrap();
        let source = "% 😀\r\n@book{parent😀,ids={alias😀},title={First},title={Second}}\r\n@book{other}\r\n@book{child,crossref={parent😀},xref=\"parent😀\",xdata={other, parent😀 ,other},related={parent😀,other},entryset={other,parent😀}}";
        ws.open(bib.clone(), source.into(), 8);
    }
    let main_doc = &ws.documents[&main];
    let source_calls: Vec<_> = main_doc
        .text
        .source
        .match_indices("{parent😀}")
        .map(|(at, _)| {
            Location::new(
                main.clone(),
                main_doc.text.range(at + 1, at + 1 + "parent😀".len()),
            )
        })
        .collect();
    for (index, name) in ["default.bib", "section.bib"].into_iter().enumerate() {
        let bib = main.join(name).unwrap();
        let declaration = locations(&ws, &bib, "@book{", "parent😀").remove(0);
        let doc = &ws.documents[&bib];
        let mut expected = vec![declaration.clone(), source_calls[index].clone()];
        for prefix in [
            "crossref={",
            "xref=\"",
            "xdata={other, ",
            "related={",
            "entryset={other,",
        ] {
            expected.extend(locations(&ws, &bib, prefix, "parent😀"));
        }
        check(&ws, &bib, declaration.range.start, &declaration, &expected);
        for query in &expected {
            check(&ws, &query.uri, query.range.start, &declaration, &expected);
        }
        let highlights = texquill::editing::highlights(&ws, &bib, at(&ws, &bib, "xref=\"par"));
        assert_eq!(highlights.len(), 6);
        assert_eq!(
            highlights
                .iter()
                .filter(|h| h.kind == Some(DocumentHighlightKind::WRITE))
                .map(|h| h.range)
                .collect::<Vec<_>>(),
            vec![declaration.range]
        );
        assert!(highlights
            .iter()
            .all(|h| h.range == declaration.range || h.kind == Some(DocumentHighlightKind::READ)));
        let highlights = texquill::editing::highlights(&ws, &main, source_calls[index].range.start);
        assert_eq!(
            highlights,
            vec![DocumentHighlight {
                range: source_calls[index].range,
                kind: Some(DocumentHighlightKind::READ)
            }]
        );
        assert!(doc.text.source.contains("\r\n"));
    }
}

#[test]
fn shared_relation_owners_require_all_consumers_and_restore_after_unsaved_changes() {
    let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{parent}");
    let global = main.join("global.bib").unwrap();
    // The global relation appears in both sections, whose targets disagree.
    ws.open(
        global.clone(),
        "@book{globalchild,crossref={parent}}".into(),
        7,
    );
    for name in ["default.bib", "section.bib"] {
        let bib = main.join(name).unwrap();
        assert!(features::references(&ws, &bib, at(&ws, &bib, "@book{par"), true).is_empty());
    }
    // A shared owner loaded by a second root is checked even though it has no citations.
    ws.open(
        global.clone(),
        "@book{shared}\n@book{globalchild,crossref={shared}}".into(),
        8,
    );
    let declaration = locations(&ws, &global, "@book{", "shared").remove(0);
    let mut expected = vec![declaration.clone()];
    expected.extend(locations(&ws, &global, "crossref={", "shared"));
    check(
        &ws,
        &global,
        declaration.range.start,
        &declaration,
        &expected,
    );
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other.clone(), r"\documentclass{article}\usepackage{biblatex}\addbibresource{global.bib}\addbibresource{section.bib}\begin{document}\end{document}".into(), 9);
    let selected = main.join("section.bib").unwrap();
    ws.open(selected.clone(), "@book{shared}".into(), 10);
    assert!(features::references(&ws, &global, declaration.range.start, true).is_empty());
    ws.open(selected, "@book{different}".into(), 11);
    check(
        &ws,
        &global,
        declaration.range.start,
        &declaration,
        &expected,
    );
    assert_eq!(ws.documents[&main].version, Some(5));
}

#[test]
fn incomplete_or_ambiguous_database_and_execution_never_return_partial_graphs() {
    for invalid in [
        "@book{parent}\n@book{parent}",
        "@book{parent}\n@book{other,ids={parent}}",
        "@book{parent}\n@book{child,crossref={parent},CROSSREF={parent}}",
        "@book{parent}\n@book{child,crossref=unknown}",
        "@book{parent}\n@book{child,crossref={parent}",
        "@book{parent,ids={legacy},ids={legacy2}}",
    ] {
        let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{parent}");
        let bib = main.join("section.bib").unwrap();
        ws.open(bib.clone(), invalid.into(), 8);
        assert!(
            features::references(&ws, &bib, at(&ws, &bib, "@book{par"), true).is_empty(),
            "{invalid}"
        );
        assert!(
            features::references(&ws, &main, at(&ws, &main, r"\cite{par"), true).is_empty(),
            "{invalid}"
        );
    }
    for body in [
        r"\ifunknown\newrefsection[chosen]\fi\cite{parent}",
        r"\newrefsection[chosen]\newcommand\later{\cite{parent}}\cite{parent}",
        r"\newrefsection[chosen]\newcommand\pass[1]{#1}\pass{\cite{parent}}",
    ] {
        let (_dir, ws, main) = setup(body);
        let bib = main.join("section.bib").unwrap();
        assert!(
            features::references(&ws, &bib, at(&ws, &bib, "@book{par"), true).is_empty(),
            "{body}"
        );
    }
    let (_dir, ws, main) =
        setup(r"\newrefsection[chosen]\defbibentryset{parent}{childselected}\cite{parent}");
    let bib = main.join("section.bib").unwrap();
    let position = at(&ws, &bib, "@book{par");
    assert_eq!(features::references(&ws, &bib, position, true).len(), 1);
    assert!(features::prepare_rename(&ws, &bib, position).is_err());

    let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{parent}");
    let bib = main.join("section.bib").unwrap();
    let position = at(&ws, &bib, "@book{par");
    assert!(!features::references(&ws, &bib, position, true).is_empty());
    ws.index_truncated.insert("review limit".into());
    assert!(features::references(&ws, &bib, position, true).is_empty());
    assert!(features::references(&ws, &main, at(&ws, &main, r"\cite{par"), true).is_empty());
    ws.index_truncated.clear();
    assert!(!features::references(&ws, &bib, position, true).is_empty());
}

#[test]
fn unsaved_consumers_with_conflicting_lexical_meanings_protect_shared_citations() {
    let (dir, mut ws, main) = setup(r"\input{chapter}\newrefsection[chosen]\input{chapter}");
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{shared}").unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 6);
    let global = main.join("global.bib").unwrap();
    let position = at(&ws, &global, "@book{sha");
    assert!(!features::references(&ws, &global, position, true).is_empty());
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other.clone(), r"\documentclass{article}\usepackage{biblatex}\addbibresource{global.bib}\renewcommand\cite[1]{#1}\begin{document}\input{chapter}\end{document}".into(), 8);
    assert!(features::references(&ws, &global, position, true).is_empty());
    ws.open(other, r"\documentclass{article}\usepackage{biblatex}\addbibresource{global.bib}\begin{document}\input{chapter}\end{document}".into(), 9);
    assert!(!features::references(&ws, &global, position, true).is_empty());
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_biber_accepts_complete_selected_key_and_alias_reference_edits() {
    let (dir, mut ws, main) = setup(
        r"\cite{legacy}\nocite{parent,child,aliaschild,xrefchild,relatedchild}\printbibliography\newrefsection[chosen]\cite{legacy}\nocite{parent,child,aliaschild,xrefchild,relatedchild}\printbibliography",
    );
    for (name, title) in [
        ("default.bib", "Default parent"),
        ("section.bib", "Selected parent"),
    ] {
        let source = format!("@book{{parent,ids={{legacy}},title={{{title}}},author={{Parent, Pat}},year={{2001}}}}\n@inbook{{child,crossref={{parent}},title={{Child}}}}\n@inbook{{aliaschild,crossref={{legacy}},title={{Alias child}}}}\n@book{{xrefchild,xref={{parent}},title={{Xref child}},author={{Child, Chris}},year={{2002}}}}\n@book{{relatedchild,related={{parent}},title={{Related child}},author={{Child, Chris}},year={{2003}}}}");
        let bib = main.join(name).unwrap();
        std::fs::write(bib.to_file_path().unwrap(), &source).unwrap();
        ws.open(bib, source, 7);
    }
    let run = |program: &str, args: &[&str]| {
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let compile = || {
        for extension in ["aux", "bcf", "bbl", "run.xml"] {
            let _ = std::fs::remove_file(dir.path().join(format!("main.{extension}")));
        }
        let args = [
            "-no-shell-escape",
            "-interaction=nonstopmode",
            "-halt-on-error",
            "main.tex",
        ];
        run("pdflatex", &args);
        run("biber", &["main"]);
        run("pdflatex", &args);
        run("pdflatex", &args);
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        assert!(!log.contains("undefined"), "{log}");
        let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
        assert!(!blg.contains("WARN") && !blg.contains("ERROR"), "{blg}");
        std::fs::read_to_string(dir.path().join("main.bbl")).unwrap()
    };
    compile();
    let selected = main.join("section.bib").unwrap();
    let default = main.join("default.bib").unwrap();
    let default_source = ws.documents[&default].text.source.clone();
    for (prefix, key, renamed, count) in [
        // Rename the alias before its owner: equal IDS spellings share
        // Biber's global cache even in otherwise independent sections.
        ("ids={", "legacy", "selectedlegacy", 3),
        ("@book{", "parent", "selectedparent", 5),
    ] {
        let declaration = locations(&ws, &selected, prefix, key).remove(0);
        let refs = features::references(&ws, &selected, declaration.range.start, true);
        assert_eq!(refs.len(), count, "{refs:?}");
        assert!(refs
            .iter()
            .all(|location| location.uri == main || location.uri == selected));
        for uri in [main.clone(), selected.clone()] {
            let doc = &ws.documents[&uri];
            let mut spans: Vec<_> = refs
                .iter()
                .filter(|location| location.uri == uri)
                .map(|location| {
                    doc.text.offset(location.range.start).unwrap()
                        ..doc.text.offset(location.range.end).unwrap()
                })
                .collect();
            spans.sort_by_key(|span| std::cmp::Reverse(span.start));
            let mut source = doc.text.source.clone();
            for span in spans {
                assert_eq!(&source[span.clone()], key);
                source.replace_range(span, renamed);
            }
            std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
            ws.open(uri, source, 9);
        }
        compile();
    }
    assert_eq!(ws.documents[&default].text.source, default_source);
    let bbl = compile();
    for (section, key, alias, title, other) in [
        (0, "parent", "legacy", "Default parent", "Selected parent"),
        (
            1,
            "selectedparent",
            "selectedlegacy",
            "Selected parent",
            "Default parent",
        ),
    ] {
        let body = bbl
            .split_once(&format!("\\refsection{{{section}}}"))
            .unwrap()
            .1
            .split_once("\\endrefsection")
            .unwrap()
            .0;
        assert!(body.contains(&format!("\\entry{{{key}}}")), "{body}");
        assert!(
            body.contains(&format!("\\keyalias{{{alias}}}{{{key}}}")),
            "{body}"
        );
        assert!(
            body.contains(&format!("\\field{{booktitle}}{{{title}}}")),
            "{body}"
        );
        assert!(!body.contains(other), "{body}");
    }
}

#[test]
fn a_configured_root_does_not_certify_a_database_it_never_loads() {
    let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{parent}");
    let inactive = main.join("inactive.bib").unwrap();
    ws.open(
        inactive.clone(),
        "@book{orphan}\n@book{child,crossref={orphan}}".into(),
        8,
    );
    ws.config.build.root_file = Some("main.tex".into());
    assert!(features::references(&ws, &inactive, at(&ws, &inactive, "@book{or"), true).is_empty());
    assert!(
        features::references(&ws, &inactive, at(&ws, &inactive, "crossref={or"), true).is_empty()
    );
}
