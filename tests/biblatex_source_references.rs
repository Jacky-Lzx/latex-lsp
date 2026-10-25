use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;
fn setup(preamble: &str, body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}{preamble}\begin{{document}}{body}\end{{document}}"
    );
    for(name,text)in[("main.tex",source.as_str()),("default.bib","@book{parent,title={Default parent}}\n@book{member,title={Default member}}\n@inbook{child,crossref={parent}}"),("section.bib","@book{parent,title={Selected parent}}\n@book{member,title={Selected member}}\n@inbook{child,crossref={parent}}")]{std::fs::write(dir.path().join(name),text).unwrap();}
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let start = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert!(doc.text.source[start..].starts_with(name));
    Location::new(uri.clone(), doc.text.range(start, start + name.len()))
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
fn check(ws: &Workspace, query: &Location, declaration: &Location, expected: &[Location]) {
    assert_eq!(
        sorted(features::references(
            ws,
            &query.uri,
            query.range.start,
            true
        )),
        sorted(expected.to_vec())
    );
    assert_eq!(
        sorted(features::references(
            ws,
            &query.uri,
            query.range.start,
            false
        )),
        sorted(
            expected
                .iter()
                .filter(|item| *item != declaration)
                .cloned()
                .collect()
        )
    );
}
fn calls(ws: &Workspace, uri: &Url, name: &str) -> Vec<Location> {
    let doc = &ws.documents[uri];
    let marker = format!(r"\cite{{{name}}}");
    doc.text
        .source
        .match_indices(&marker)
        .map(|(at, _)| Location::new(uri.clone(), doc.text.range(at + 6, at + 6 + name.len())))
        .collect()
}
#[test]
fn source_and_database_keys_have_complete_separate_graphs() {
    let (_dir, ws, main) = setup(
        "",
        r"\cite{parent}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
    );
    let cite = calls(&ws, &main, "parent");
    let source = key(&ws, &main, r"\defbibentryset{", "parent");
    let selected = main.join("section.bib").unwrap();
    let expected = vec![
        source.clone(),
        cite[1].clone(),
        key(&ws, &selected, "crossref={", "parent"),
    ];
    for query in &expected {
        check(&ws, query, &source, &expected);
    }
    let default = main.join("default.bib").unwrap();
    let book = key(&ws, &default, "@book{", "parent");
    let expected = vec![
        book.clone(),
        cite[0].clone(),
        key(&ws, &default, "crossref={", "parent"),
    ];
    for query in &expected {
        check(&ws, query, &book, &expected);
    }
    // Editing also certifies and includes the backend's hidden raw prerequisite.
    let edit = features::rename(&ws, &main, source.range.start, "changed")
        .unwrap()
        .unwrap();
    let actual: Vec<_> = edit
        .changes
        .unwrap()
        .into_iter()
        .flat_map(|(uri, edits)| {
            edits.into_iter().map(move |edit| {
                assert_eq!(edit.new_text, "changed");
                Location::new(uri.clone(), edit.range)
            })
        })
        .collect();
    assert_eq!(
        sorted(actual),
        sorted(vec![
            source,
            cite[1].clone(),
            key(&ws, &selected, "crossref={", "parent"),
            key(&ws, &selected, "@book{", "parent")
        ])
    );
}
#[test]
fn identical_resources_keep_distinct_numeric_reference_graphs() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}\newrefsection[chosen]\cite{parent}",
    );
    let selected = main.join("section.bib").unwrap();
    ws.open(selected.clone(), "@book{parent} @book{member}".into(), 8);
    let cite = calls(&ws, &main, "parent");
    let source = key(&ws, &main, r"\defbibentryset{", "parent");
    let book = key(&ws, &selected, "@book{", "parent");
    check(&ws, &source, &source, &[source.clone(), cite[0].clone()]);
    check(&ws, &book, &book, &[book.clone(), cite[1].clone()]);
}
#[test]
fn early_explicit_requests_leave_the_source_declaration_unused_and_books_complete() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{member}\cite{parent}",
    );
    let source = key(&ws, &main, r"\defbibentryset{", "parent");
    check(&ws, &source, &source, std::slice::from_ref(&source));
    let selected = main.join("section.bib").unwrap();
    let book = key(&ws, &selected, "@book{", "parent");
    let mut expected = vec![book.clone(), key(&ws, &selected, "crossref={", "parent")];
    expected.extend(calls(&ws, &main, "parent"));
    for query in &expected {
        check(&ws, query, &book, &expected);
    }
}
#[test]
fn repeated_source_declarations_and_member_spans_are_deduplicated_across_numeric_sections() {
    let (dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\input{sets}\cite{parent}\newrefsection[chosen]\input{sets}\cite{parent}\cite{member}",
    );
    let sets = main.join("sets.tex").unwrap();
    std::fs::write(
        dir.path().join("sets.tex"),
        r"\defbibentryset{parent}{member}",
    )
    .unwrap();
    ws.refresh_project(&main);
    let source = key(&ws, &sets, r"\defbibentryset{", "parent");
    let mut expected = vec![
        source.clone(),
        key(
            &ws,
            &main.join("section.bib").unwrap(),
            "crossref={",
            "parent",
        ),
    ];
    expected.extend(calls(&ws, &main, "parent"));
    check(&ws, &source, &source, &expected);
    let bib = main.join("section.bib").unwrap();
    let book = key(
        &ws,
        &bib,
        "@book{parent,title={Selected parent}}\n@book{",
        "member",
    );
    let member = key(&ws, &sets, r"\defbibentryset{parent}{", "member");
    let mut expected = vec![book.clone(), member.clone()];
    expected.extend(calls(&ws, &main, "member"));
    check(&ws, &member, &book, &expected);
    check(&ws, &book, &book, &expected);
}
#[test]
fn static_named_source_queries_have_exact_unicode_ranges_and_local_highlights() {
    for declare in [
        r"\csuse{defbibentryset}",
        r"\csname defbibentryset\endcsname",
    ] {
        let (_dir,ws,main)=setup("",&format!("% 😀\r\n\\newrefsection[chosen]{declare}{{parent}}{{member}}\\csuse{{cite}}{{parent}}\\csname cite\\endcsname{{parent}}"));
        let source = key(&ws, &main, &format!("{declare}{{"), "parent");
        let bib = main.join("section.bib").unwrap();
        let mut expected = vec![source.clone(), key(&ws, &bib, "crossref={", "parent")];
        for marker in [r"\csuse{cite}{", r"\csname cite\endcsname{"] {
            expected.push(key(&ws, &main, marker, "parent"));
        }
        for query in &expected {
            check(&ws, query, &source, &expected);
        }
        let highlights = latex_lsp::editing::highlights(&ws, &main, source.range.start);
        assert_eq!(highlights.len(), 3, "{highlights:?}");
        assert!(
            highlights
                .iter()
                .any(|h| h.range == source.range && h.kind == Some(DocumentHighlightKind::WRITE)),
            "{highlights:?}"
        );
    }
}
#[test]
fn ambiguous_shared_relations_keep_the_complete_graph_protected() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}\newrefsection[chosen]\cite{parent}",
    );
    let source = key(&ws, &main, r"\defbibentryset{", "parent");
    for query in [
        source,
        calls(&ws, &main, "parent")[0].clone(),
        key(
            &ws,
            &main.join("section.bib").unwrap(),
            "crossref={",
            "parent",
        ),
    ] {
        assert!(features::references(&ws, &query.uri, query.range.start, true).is_empty());
    }
}
#[test]
fn unsaved_consumers_can_make_shared_source_members_ambiguous_without_partial_results() {
    let (dir, mut ws, main) = setup("", r"\newrefsection[chosen]\input{sets}\cite{member}");
    std::fs::write(
        dir.path().join("sets.tex"),
        r"\defbibentryset{parent}{member}",
    )
    .unwrap();
    ws.refresh_project(&main);
    let sets = main.join("sets.tex").unwrap();
    let member = key(&ws, &sets, r"\defbibentryset{parent}{", "member");
    let selected = main.join("section.bib").unwrap();
    let book = key(
        &ws,
        &selected,
        "@book{parent,title={Selected parent}}\n@book{",
        "member",
    );
    let mut expected = vec![book.clone(), member.clone()];
    expected.extend(calls(&ws, &main, "member"));
    check(&ws, &member, &book, &expected);
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{default.bib}\begin{document}\input{sets}\cite{member}\end{document}".into(),9);
    assert!(features::references(&ws, &member.uri, member.range.start, true).is_empty());
    assert!(features::references(&ws, &book.uri, book.range.start, true).is_empty());
    ws.documents.remove(&other);
    check(&ws, &member, &book, &expected);
}
#[test]
fn unproved_source_requests_aliases_and_database_preambles_remain_protected() {
    for extra in [
        r"\defcitealias{parent}{P}",
        r"\addtocategory{category}{parent}",
        r"\newcommand\later{\cite{parent}}",
    ] {
        let (_dir, ws, main) = setup(
            "",
            &format!(
                r"\newrefsection[chosen]\defbibentryset{{parent}}{{member}}\cite{{parent}}{extra}"
            ),
        );
        assert!(
            features::references(&ws, &main, calls(&ws, &main, "parent")[0].range.start, true)
                .is_empty()
        );
    }
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
    );
    let bib = main.join("section.bib").unwrap();
    for text in [
        "@book{parent,IDS=unknown} @book{member}",
        "@book{parent} @book{member} @preamble{\"code\"}",
    ] {
        ws.open(bib.clone(), text.into(), 9);
        assert!(
            features::references(&ws, &main, calls(&ws, &main, "parent")[0].range.start, true)
                .is_empty()
        );
    }
}

fn compile(dir: &std::path::Path) -> String {
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
    assert!(!log.contains("undefined"), "{log}");
    let blg = std::fs::read_to_string(dir.join("main.blg")).unwrap();
    assert!(
        !blg.contains("ERROR") && !blg.contains("I didn't find"),
        "{blg}"
    );
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}
fn section(bbl: &str, index: u32) -> &str {
    bbl.split_once(&format!("\\refsection{{{index}}}"))
        .unwrap()
        .1
        .split_once(r"\endrefsection")
        .unwrap()
        .0
}
fn entries(section: &str) -> std::collections::BTreeMap<String, String> {
    section
        .split(r"\entry{")
        .skip(1)
        .map(|entry| {
            let (name, rest) = entry.split_once('}').unwrap();
            (name.into(), rest.split_once(r"\endentry").unwrap().0.into())
        })
        .collect()
}
#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn complete_source_and_member_graph_edits_preserve_other_sections_in_native_biber() {
    for member in [false, true] {
        let (dir, mut ws, main) = setup(
            "",
            r"\cite{parent}\cite{member}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}\cite{member}\cite{child}\printbibliography",
        );
        if !member {
            let bib = main.join("section.bib").unwrap();
            let text = ws.documents[&bib]
                .text
                .source
                .replace(",crossref={parent}", ",title={Standalone child}");
            std::fs::write(dir.path().join("section.bib"), &text).unwrap();
            ws.open(bib, text, 9);
        }
        let original = compile(dir.path());
        let source = key(&ws, &main, r"\defbibentryset{", "parent");
        let bib = main.join("section.bib").unwrap();
        let query = if member {
            key(
                &ws,
                &bib,
                "@book{parent,title={Selected parent}}\n@book{",
                "member",
            )
        } else {
            source
        };
        let old = if member { "member" } else { "parent" };
        let new = if member {
            "renamedmember"
        } else {
            "renamedset"
        };
        let references = features::references(&ws, &query.uri, query.range.start, true);
        assert_eq!(
            references.len(),
            if member { 3 } else { 2 },
            "{references:?}"
        );
        let edited = tempfile::tempdir().unwrap();
        for name in ["main.tex", "default.bib", "section.bib"] {
            let uri = main.join(name).unwrap();
            let doc = &ws.documents[&uri];
            let mut edits: Vec<_> = references
                .iter()
                .filter(|location| location.uri == uri)
                .map(|location| TextDocumentContentChangeEvent {
                    range: Some(location.range),
                    range_length: None,
                    text: new.into(),
                })
                .collect();
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.unwrap().start));
            let text = doc.text.changed(edits).unwrap().source;
            std::fs::write(edited.path().join(name), text).unwrap();
        }
        let after = compile(edited.path());
        assert_eq!(section(&after, 0), section(&original, 0));
        let expected =
            entries(&section(&original, 1).replace(&format!("{{{old}}}"), &format!("{{{new}}}")));
        assert_eq!(entries(section(&after, 1)), expected);
        assert!(section(&after, 1).contains(&format!(
            "\\entry{{{}}}{{set}}",
            if member { "parent" } else { new }
        )));
    }
}

#[test]
fn source_only_database_relations_do_not_certify_a_partial_graph() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collected}",
    );
    let bib = main.join("section.bib").unwrap();
    ws.open(
        bib.clone(),
        "@book{member} @book{child,crossref={collected}}".into(),
        8,
    );
    let declaration = key(&ws, &main, r"\defbibentryset{", "collected");
    assert_eq!(
        features::definitions(&ws, &main, calls(&ws, &main, "collected")[0].range.start),
        vec![declaration.clone()]
    );
    assert!(features::references(&ws, &main, declaration.range.start, true).is_empty());
}
#[test]
fn shared_numeric_resources_keep_large_relation_graphs_complete_within_the_budget() {
    let body = r"\newrefsection[chosen]\input{sets}\cite{parent}".repeat(350);
    let (dir, mut ws, main) = setup("", &body);
    let sets = main.join("sets.tex").unwrap();
    std::fs::write(
        dir.path().join("sets.tex"),
        r"\defbibentryset{parent}{member}",
    )
    .unwrap();
    let mut library = "@book{parent} @book{member}\n".to_owned();
    for index in 0..1500 {
        library.push_str(&format!("@book{{child{index},crossref={{parent}}}}\n"));
    }
    let bib = main.join("section.bib").unwrap();
    ws.open(bib.clone(), library, 8);
    ws.refresh_project(&main);
    let declaration = key(&ws, &sets, r"\defbibentryset{", "parent");
    let mut expected = vec![declaration.clone()];
    expected.extend(calls(&ws, &main, "parent"));
    let doc = &ws.documents[&bib];
    for (at, _) in doc.text.source.match_indices("crossref={parent}") {
        expected.push(Location::new(bib.clone(), doc.text.range(at + 10, at + 16)));
    }
    assert_eq!(expected.len(), 1851);
    check(&ws, &declaration, &declaration, &expected);
}
