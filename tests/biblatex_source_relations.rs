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
fn check(ws: &Workspace, uri: &Url, position: Position, expected: &Location) {
    assert_eq!(
        features::definitions(ws, uri, position),
        vec![expected.clone()]
    );
    let hover = features::hover(ws, uri, position).expect("certified hover");
    let HoverContents::Markup(contents) = hover.contents else {
        panic!()
    };
    assert!(
        contents.value.contains(expected.uri.path()),
        "{}",
        contents.value
    );
    if expected.uri.path().ends_with("main.tex") {
        assert!(contents.value.contains("member"), "{}", contents.value);
    }
}
#[test]
fn selected_database_relations_point_to_the_source_set_that_replaces_the_parent() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{child,parent}",
    );
    let bib = main.join("section.bib").unwrap();
    let pos = key(&ws, &bib, "crossref={", "parent").range.start;
    check(
        &ws,
        &bib,
        pos,
        &key(&ws, &main, r"\defbibentryset{", "parent"),
    );
}
#[test]
fn an_early_explicit_request_keeps_database_relation_targets_book_bound() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{member}\cite{child,parent}",
    );
    let bib = main.join("section.bib").unwrap();
    check(
        &ws,
        &bib,
        key(&ws, &bib, "crossref={", "parent").range.start,
        &key(&ws, &bib, "@book{", "parent"),
    );
}
#[test]
fn one_database_relation_cannot_choose_between_two_numeric_section_identities() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,child}\newrefsection[chosen]\cite{parent,child}",
    );
    let bib = main.join("section.bib").unwrap();
    let pos = key(&ws, &bib, "crossref={", "parent").range.start;
    assert!(features::definitions(&ws, &bib, pos).is_empty());
    assert!(features::hover(&ws, &bib, pos).is_none());
}
#[test]
fn an_unsaved_consumer_can_make_an_otherwise_source_bound_relation_ambiguous() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{child,parent}",
    );
    let bib = main.join("section.bib").unwrap();
    let pos = key(&ws, &bib, "crossref={", "parent").range.start;
    ws.open(main.join("unsaved.tex").unwrap(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\cite{parent,child}\end{document}".into(),9);
    assert!(features::definitions(&ws, &bib, pos).is_empty());
    assert!(features::hover(&ws, &bib, pos).is_none());
}
#[test]
fn relations_with_invalid_fields_do_not_use_the_source_set_preview() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{child,parent}",
    );
    let bib = main.join("section.bib").unwrap();
    ws.open(
        bib.clone(),
        "@book{parent} @book{member} @inbook{child,crossref={parent},CROSSREF={member}}".into(),
        9,
    );
    let pos = key(&ws, &bib, "crossref={", "parent").range.start;
    assert!(features::definitions(&ws, &bib, pos).is_empty());
    assert!(features::hover(&ws, &bib, pos).is_none());
}

#[test]
fn all_five_static_relation_fields_use_the_final_source_identity_and_exact_utf16_ranges() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
    );
    let bib = main.join("section.bib").unwrap();
    let source="% 😀\r\n@book{parent} @book{member}\r\n@book{a,crossref={parent}}\r\n@book{b,xref=\"parent\"}\r\n@book{c,xdata={parent}}\r\n@book{d,related={parent,parent}}\r\n@set{e,entryset={parent}}";
    ws.open(bib.clone(), source.into(), 8);
    let target = key(&ws, &main, r"\defbibentryset{", "parent");
    for (at, _) in source.match_indices("parent") {
        if at < source.find("@book{a").unwrap() {
            continue;
        }
        let point = ws.documents[&bib].text.position(at + 2);
        check(&ws, &bib, point, &target);
        assert_eq!(
            features::hover(&ws, &bib, point).unwrap().range,
            Some(ws.documents[&bib].text.range(at, at + 6))
        );
    }
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_biber_crossref_identity_agrees_with_source_or_first_requested_parent() {
    for early in [false, true] {
        let body = format!(
            r"\newrefsection[chosen]{}\defbibentryset{{parent}}{{member}}\cite{{parent,child}}\printbibliography",
            if early { r"\cite{parent}" } else { "" }
        );
        let (dir, ws, main) = setup("", &body);
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
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{program}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
        let section = bbl
            .split_once(r"\refsection{1}")
            .unwrap()
            .1
            .split_once(r"\endrefsection")
            .unwrap()
            .0;
        assert!(
            section.contains(&format!(
                "\\entry{{parent}}{{{}}}",
                if early { "book" } else { "set" }
            )),
            "{section}"
        );
        let bib = main.join("section.bib").unwrap();
        let expected = if early {
            key(&ws, &bib, "@book{", "parent")
        } else {
            key(&ws, &main, r"\defbibentryset{", "parent")
        };
        check(
            &ws,
            &bib,
            key(&ws, &bib, "crossref={", "parent").range.start,
            &expected,
        );
        let child = section
            .split_once(r"\entry{child}")
            .unwrap()
            .1
            .split_once(r"\endentry")
            .unwrap()
            .0;
        assert_eq!(
            child.contains(r"\field{booktitle}{Selected parent}"),
            early,
            "{child}"
        );
    }
}
