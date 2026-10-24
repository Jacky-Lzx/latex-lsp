use latex_lsp::{features, workspace::Workspace};
use tower_lsp::lsp_types::*;
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
fn calls(ws: &Workspace, uri: &Url) -> Vec<Position> {
    let doc = &ws.documents[uri];
    doc.text
        .source
        .match_indices(r"\cite{parent}")
        .map(|(at, _)| doc.text.position(at + r"\cite{".len() + 2))
        .collect()
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
fn a_direct_set_replaces_only_the_selected_numeric_section_database_entry() {
    let (_dir, ws, main) = setup(
        "",
        r"\cite{parent}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
    );
    let default = main.join("default.bib").unwrap();
    let selected = main.join("section.bib").unwrap();
    let points = calls(&ws, &main);
    let declaration = key(&ws, &main, r"\defbibentryset{", "parent");
    check(
        &ws,
        &main,
        points[0],
        &key(&ws, &default, "@book{", "parent"),
    );
    check(&ws, &main, points[1], &declaration);
    check(&ws, &main, declaration.range.start, &declaration);
    check(
        &ws,
        &selected,
        key(&ws, &selected, "@book{", "parent").range.start,
        &key(&ws, &selected, "@book{", "parent"),
    );
}
#[test]
fn identical_resource_sets_do_not_merge_distinct_numeric_section_namespaces() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}\newrefsection[chosen]\cite{parent}",
    );
    let selected = main.join("section.bib").unwrap();
    let points = calls(&ws, &main);
    check(
        &ws,
        &main,
        points[0],
        &key(&ws, &main, r"\defbibentryset{", "parent"),
    );
    check(
        &ws,
        &main,
        points[1],
        &key(&ws, &selected, "@book{", "parent"),
    );
}
#[test]
fn explicit_requests_before_a_set_definition_keep_the_original_database_identity() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{member}\cite{parent}",
    );
    let selected = main.join("section.bib").unwrap();
    let target = key(&ws, &selected, "@book{", "parent");
    for point in calls(&ws, &main) {
        check(&ws, &main, point, &target);
    }
}
#[test]
fn preamble_and_grouped_definitions_persist_in_their_own_numeric_section() {
    let (_dir, ws, main) = setup(
        r"{\defbibentryset{parent}{member}}",
        r"\cite{parent}\newrefsection[chosen]\cite{parent}\endrefsection\cite{parent}",
    );
    let points = calls(&ws, &main);
    let declaration = key(&ws, &main, r"\defbibentryset{", "parent");
    let selected = main.join("section.bib").unwrap();
    for point in [points[0], points[2]] {
        check(&ws, &main, point, &declaration);
    }
    check(
        &ws,
        &main,
        points[1],
        &key(&ws, &selected, "@book{", "parent"),
    );
}
#[test]
fn wildcard_requests_use_the_final_set_definition_without_locking_database_entries() {
    for body in [
        r"\newrefsection[chosen]\nocite{*}\defbibentryset{parent}{member}\cite{parent}",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\nocite{*}\cite{parent}",
    ] {
        let (_dir, ws, main) = setup("", body);
        check(
            &ws,
            &main,
            calls(&ws, &main)[0],
            &key(&ws, &main, r"\defbibentryset{", "parent"),
        );
    }
}
#[test]
fn unknown_set_execution_and_multiple_declarations_do_not_fall_back_to_database_keys() {
    for body in [
        r"\newrefsection[chosen]\newcommand\later{\defbibentryset{parent}{member}}\cite{parent}",
        r"\newrefsection[chosen]\ifunknown\defbibentryset{parent}{member}\fi\cite{parent}",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\defbibentryset{parent}{child}\cite{parent}",
    ] {
        let (_dir, ws, main) = setup("", body);
        let point = calls(&ws, &main)[0];
        assert!(features::definitions(&ws, &main, point).is_empty());
        assert!(features::hover(&ws, &main, point).is_none());
    }
}

#[test]
fn static_named_sets_citations_and_members_have_exact_unicode_crlf_ranges() {
    for declare in [
        r"\csuse{defbibentryset}",
        r"\csname defbibentryset\endcsname",
    ] {
        let body = format!("😀\r\n\\newrefsection[chosen]{declare}{{parent}}{{member}}\\csuse{{cite}}{{parent}}\\csname cite\\endcsname{{parent}}");
        let (_dir, ws, main) = setup("", &body);
        let target = key(&ws, &main, &format!("{declare}{{"), "parent");
        check(&ws, &main, target.range.start, &target);
        for marker in [r"\csuse{cite}{", r"\csname cite\endcsname{"] {
            check(
                &ws,
                &main,
                key(&ws, &main, marker, "parent").range.start,
                &target,
            );
        }
        let member = main.join("section.bib").unwrap();
        check(
            &ws,
            &main,
            key(&ws, &main, &format!("{declare}{{parent}}{{"), "member")
                .range
                .start,
            &key(
                &ws,
                &member,
                "@book{parent,title={Selected parent}}\n@book{",
                "member",
            ),
        );
    }
}

#[test]
fn input_order_and_unsaved_edits_recompute_the_final_identity() {
    let (dir, mut ws, main) = setup("", r"\newrefsection[chosen]\input{sets}\cite{parent}");
    std::fs::write(
        dir.path().join("sets.tex"),
        r"\defbibentryset{parent}{member}",
    )
    .unwrap();
    ws.refresh_project(&main);
    let sets = main.join("sets.tex").unwrap();
    check(
        &ws,
        &main,
        calls(&ws, &main)[0],
        &key(&ws, &sets, r"\defbibentryset{", "parent"),
    );
    ws.open(
        sets.clone(),
        r"\cite{parent}\defbibentryset{parent}{member}".into(),
        8,
    );
    let bib = main.join("section.bib").unwrap();
    check(
        &ws,
        &main,
        calls(&ws, &main)[0],
        &key(&ws, &bib, "@book{", "parent"),
    );
    ws.open(sets.clone(), r"\defbibentryset{parent}{member}".into(), 9);
    check(
        &ws,
        &main,
        calls(&ws, &main)[0],
        &key(&ws, &sets, r"\defbibentryset{", "parent"),
    );
}

#[test]
fn shared_physical_citations_require_agreement_between_numeric_executions_and_consumers() {
    let (dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\input{shared}\newrefsection[chosen]\input{shared}",
    );
    std::fs::write(dir.path().join("shared.tex"), r"\cite{parent}").unwrap();
    ws.refresh_project(&main);
    let shared = main.join("shared.tex").unwrap();
    let point = calls(&ws, &shared)[0];
    assert!(features::definitions(&ws, &shared, point).is_empty());
    assert!(features::hover(&ws, &shared, point).is_none());
    let source = ws.documents[&main]
        .text
        .source
        .replace(r"\newrefsection[chosen]\input{shared}", r"\input{shared}");
    ws.open(main.clone(), source, 7);
    check(
        &ws,
        &shared,
        point,
        &key(&ws, &main, r"\defbibentryset{", "parent"),
    );
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other,r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\input{shared}\end{document}".into(),1);
    assert!(features::definitions(&ws, &shared, point).is_empty());
    assert!(features::hover(&ws, &shared, point).is_none());
}

#[test]
fn unproved_namespaces_and_requests_stay_protected() {
    for body in [
        "\\newrefsection[chosen]\\csname defbib% name\nentryset\\endcsname{parent}{member}\\cite{parent}",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\addtocategory{cat}{parent}\cite{parent}",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\defcitealias{parent}{alias}\cite{parent}",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\entrydata{parent}{}\cite{parent}",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\defbibentryset{member}{child}\cite{parent}",
        r"\newrefsection[chosen]\cite{parent}\nocite{*}\defbibentryset{parent}{member}\cite{parent}",
        r"\newrefsection[chosen]\defbibentryset{parent}{\unknown}\cite{parent}",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\setcounter{refsection}{2}\cite{parent}",
    ] {
        let (_dir, ws, main) = setup("", body);
        for point in calls(&ws, &main) {
            assert!(
                features::definitions(&ws, &main, point).is_empty(),
                "{body}"
            );
            assert!(features::hover(&ws, &main, point).is_none(), "{body}");
        }
    }
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
    );
    let bib = main.join("section.bib").unwrap();
    ws.open(bib, r"@book{parent,ids={legacy}} @book{member}".into(), 8);
    assert_eq!(
        features::definitions(&ws, &main, calls(&ws, &main)[0]),
        vec![key(&ws, &main, r"\defbibentryset{", "parent")]
    );
}

#[test]
fn budgets_and_truncated_indexes_never_return_a_partial_source_identity() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
    );
    ws.index_truncated.insert(main.to_string());
    assert!(features::definitions(&ws, &main, calls(&ws, &main)[0]).is_empty());
    assert!(features::hover(&ws, &main, calls(&ws, &main)[0]).is_none());
    let body = format!(
        r"\newrefsection[chosen]\defbibentryset{{parent}}{{{}}}\cite{{parent}}",
        std::iter::repeat_n("member", 100_001)
            .collect::<Vec<_>>()
            .join(",")
    );
    let (_dir, ws, main) = setup("", &body);
    assert!(features::definitions(&ws, &main, calls(&ws, &main)[0]).is_empty());
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_biber_numeric_sections_agree_with_source_set_navigation() {
    let bodies = [
        (r"\cite{parent}", false),
        (r"\defbibentryset{parent}{member}\cite{parent}", true),
        (
            r"\cite{parent}\defbibentryset{parent}{member}\cite{parent}",
            false,
        ),
        (
            r"\nocite{*}\defbibentryset{parent}{member}\cite{parent}",
            true,
        ),
        (r"{\defbibentryset{parent}{member}}\cite{parent}", true),
        (
            r"\nocite{parent}\defbibentryset{parent}{member}\cite{parent}",
            false,
        ),
        (
            r"\citefield{parent}{title}\defbibentryset{parent}{member}\cite{parent}",
            false,
        ),
        (
            r"\cite{child}\defbibentryset{parent}{member}\cite{parent}",
            true,
        ),
        (
            r"\csuse{defbibentryset}{parent}{member}\csuse{cite}{parent}",
            true,
        ),
        (
            r"\csname defbibentryset\endcsname{parent}{member}\csname cite\endcsname{parent}",
            true,
        ),
    ];
    let mut body = String::new();
    for (source, _) in bodies {
        body.push_str(r"\newrefsection[chosen]");
        body.push_str(source);
        body.push_str(r"\printbibliography");
    }
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
        let result = std::process::Command::new(program)
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
    let doc = &ws.documents[&main];
    for (index, (source, is_set)) in bodies.into_iter().enumerate() {
        let contents = bbl
            .split_once(&format!("\\refsection{{{}}}", index + 1))
            .unwrap()
            .1
            .split_once(r"\endrefsection")
            .unwrap()
            .0;
        let kind = if is_set { "set" } else { "book" };
        assert!(
            contents.contains(&format!("\\entry{{parent}}{{{kind}}}")),
            "section {}: {contents}",
            index + 1
        );
        let start = doc.text.source.find(source).unwrap();
        let cite = if source.contains(r"\csuse{cite}") {
            r"\csuse{cite}{parent}"
        } else if source.contains(r"\csname cite") {
            r"\csname cite\endcsname{parent}"
        } else {
            r"\cite{parent}"
        };
        let cite_start = start + source.rfind(cite).unwrap();
        let point = doc.text.position(cite_start + cite.len() - 3);
        let expected = if is_set {
            let key = start + source.find("{parent}{member}").unwrap() + 1;
            Location::new(main.clone(), doc.text.range(key, key + "parent".len()))
        } else {
            key(&ws, &main.join("section.bib").unwrap(), "@book{", "parent")
        };
        check(&ws, &main, point, &expected);
    }
}

#[test]
fn repeated_physical_set_declarations_keep_their_identity_with_bounded_execution_work() {
    for (members, repetitions, certified) in [(1, 3, true), (1_000, 512, false)] {
        let body = format!(
            r"\newrefsection[chosen]{}\cite{{parent}}",
            r"\input{sets}".repeat(repetitions)
        );
        let (dir, mut ws, main) = setup("", &body);
        let source = format!(
            r"\defbibentryset{{parent}}{{{}}}",
            std::iter::repeat_n("member", members)
                .collect::<Vec<_>>()
                .join(",")
        );
        std::fs::write(dir.path().join("sets.tex"), source).unwrap();
        ws.refresh_project(&main);
        let position = calls(&ws, &main)[0];
        if certified {
            check(
                &ws,
                &main,
                position,
                &key(
                    &ws,
                    &main.join("sets.tex").unwrap(),
                    r"\defbibentryset{",
                    "parent",
                ),
            );
        } else {
            assert!(features::definitions(&ws, &main, position).is_empty());
            assert!(features::hover(&ws, &main, position).is_none());
        }
    }
}

#[test]
fn unicode_and_quoted_source_keys_keep_complete_ranges() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
    );
    let source = ws.documents[&main]
        .text
        .source
        .replace("parent", "集\"合")
        .replace("member", "成员");
    let bib = main.join("section.bib").unwrap();
    let library = ws.documents[&bib]
        .text
        .source
        .replace("parent", "集\"合")
        .replace("member", "成员");
    ws.open(bib.clone(), library, 8);
    ws.open(main.clone(), source, 9);
    let declaration = key(&ws, &main, r"\defbibentryset{", "集\"合");
    let position = key(&ws, &main, r"\cite{", "集\"合").range.start;
    assert_eq!(
        features::definitions(&ws, &main, position),
        vec![declaration]
    );
    let HoverContents::Markup(contents) = features::hover(&ws, &main, position).unwrap().contents
    else {
        panic!()
    };
    assert!(contents.value.contains("成员"));
    let member = key(&ws, &main, r#"\defbibentryset{集"合}{"#, "成员");
    assert_eq!(
        features::definitions(&ws, &main, member.range.start),
        vec![key(
            &ws,
            &bib,
            "@book{集\"合,title={Selected 集\"合}}\n@book{",
            "成员"
        )]
    );
}

#[test]
fn database_preambles_cannot_inject_untraced_set_declarations() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\cite{parent}\newrefsection[chosen]\defbibentryset{other}{member}\cite{parent,other}",
    );
    let bib = main.join("default.bib").unwrap();
    let source = format!(
        "{}\n@preamble{{\"\\defbibentryset{{parent}}{{member}}\"}}",
        ws.documents[&bib].text.source
    );
    ws.open(bib, source, 8);
    let point = calls(&ws, &main)[0];
    assert!(features::definitions(&ws, &main, point).is_empty());
    assert!(features::hover(&ws, &main, point).is_none());
}

#[test]
fn preamble_generated_sets_also_protect_resource_only_navigation_and_edits() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\cite{parent}\newrefsection[chosen]\cite{parent}\label{safe}\ref{safe}",
    );
    let bib = main.join("default.bib").unwrap();
    let source = format!(
        "{}\n@PrEaMbLe{{\"\\defbibentryset{{parent}}{{member}}\"}}",
        ws.documents[&bib].text.source
    );
    ws.open(bib.clone(), source, 8);
    let point = calls(&ws, &main)[0];
    assert!(features::definitions(&ws, &main, point).is_empty());
    assert!(features::hover(&ws, &main, point).is_none());
    assert!(features::references(
        &ws,
        &bib,
        key(&ws, &bib, "@book{", "parent").range.start,
        true
    )
    .is_empty());
    assert!(
        features::prepare_rename(&ws, &bib, key(&ws, &bib, "@book{", "parent").range.start)
            .is_err()
    );
    assert!(features::rename(
        &ws,
        &bib,
        key(&ws, &bib, "@book{", "parent").range.start,
        "updated"
    )
    .is_err());
    assert_eq!(
        features::definitions(
            &ws,
            &main,
            key(&ws, &main, r"\newrefsection[", "chosen").range.start
        ),
        vec![key(&ws, &main, "label=", "chosen")]
    );
    let label = key(&ws, &main, r"\label{", "safe");
    assert_eq!(
        features::definitions(&ws, &main, key(&ws, &main, r"\ref{", "safe").range.start),
        vec![label]
    );
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_biber_preambles_can_change_identity_after_a_second_backend_round() {
    let (dir, mut ws, main) = setup(
        "",
        r"\cite{parent}\newrefsection[chosen]\defbibentryset{other}{member}\cite{parent,other}\printbibliography",
    );
    let bib = main.join("default.bib").unwrap();
    let source = format!(
        "{}\n@preamble{{\"\\defbibentryset{{parent}}{{member}}\"}}",
        ws.documents[&bib].text.source
    );
    std::fs::write(bib.to_file_path().unwrap(), &source).unwrap();
    ws.open(bib, source, 8);
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
    let args = [
        "-no-shell-escape",
        "-interaction=nonstopmode",
        "-halt-on-error",
        "main.tex",
    ];
    run("pdflatex", &args);
    for expected in ["book", "set"] {
        run("biber", &["main"]);
        run("pdflatex", &args);
        run("pdflatex", &args);
        let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
        let section = bbl
            .split_once(r"\refsection{0}")
            .unwrap()
            .1
            .split_once(r"\endrefsection")
            .unwrap()
            .0;
        assert!(
            section.contains(&format!("\\entry{{parent}}{{{expected}}}")),
            "{section}"
        );
        let point = calls(&ws, &main)[0];
        assert!(features::definitions(&ws, &main, point).is_empty());
        assert!(features::hover(&ws, &main, point).is_none());
    }
}

#[test]
fn preamble_injected_keys_do_not_receive_false_missing_diagnostics_or_database_completions() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\cite{frompreamble}\newrefsection[chosen]\cite{parent}",
    );
    let bib = main.join("default.bib").unwrap();
    let source = format!(
        "{}\n@preamble{{\"\\defbibentryset{{frompreamble}}{{member}}\"}}",
        ws.documents[&bib].text.source
    );
    ws.open(bib, source, 8);
    assert!(ws
        .diagnostics(&main)
        .iter()
        .all(|diag| diag.code != Some(NumberOrString::String("undefined-citation".into()))));
    assert!(features::completion(
        &ws,
        &main,
        key(&ws, &main, r"\cite{", "frompreamble").range.start
    )
    .is_empty());
}

#[test]
fn source_declaration_queries_keep_certified_references_and_safe_coupled_edits() {
    for declare in [
        r"\defbibentryset",
        r"\csuse{defbibentryset}",
        r"\csname defbibentryset\endcsname",
        r"\bibitem",
    ] {
        let body = format!(r"\newrefsection[chosen]{declare}{{parent}}{{member}}\cite{{parent}}");
        let (_dir, ws, main) = setup("", &body);
        let point = key(&ws, &main, &format!("{declare}{{"), "parent")
            .range
            .start;
        let mut expected = if declare == r"\bibitem" {
            Vec::new()
        } else {
            vec![
                key(&ws, &main, &format!("{declare}{{"), "parent"),
                key(&ws, &main, r"\cite{", "parent"),
                key(
                    &ws,
                    &main.join("section.bib").unwrap(),
                    "crossref={",
                    "parent",
                ),
            ]
        };
        let sorted = |mut locations: Vec<Location>| {
            locations.sort_by_key(|item| {
                (
                    item.uri.to_string(),
                    item.range.start.line,
                    item.range.start.character,
                )
            });
            locations
        };
        assert_eq!(
            sorted(features::references(&ws, &main, point, true)),
            sorted(expected.clone()),
            "{declare}"
        );
        let mut complete_edits = expected.clone();
        expected.retain(|item| item.uri != main || item.range.start != point);
        assert_eq!(
            sorted(features::references(&ws, &main, point, false)),
            sorted(expected),
            "{declare}"
        );
        if declare == r"\bibitem" {
            assert!(features::prepare_rename(&ws, &main, point).is_err());
            assert!(features::rename(&ws, &main, point, "renamed").is_err());
        } else {
            let declaration = key(&ws, &main, &format!("{declare}{{"), "parent");
            assert_eq!(
                features::prepare_rename(&ws, &main, point).unwrap(),
                Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: declaration.range,
                    placeholder: "parent".into()
                })
            );
            let bib = main.join("section.bib").unwrap();
            complete_edits.push(key(&ws, &bib, "@book{", "parent"));
            let edit = features::rename(&ws, &main, point, "renamed")
                .unwrap()
                .unwrap();
            let actual: Vec<_> = edit
                .changes
                .unwrap()
                .into_iter()
                .flat_map(|(uri, edits)| {
                    edits.into_iter().map(move |edit| {
                        assert_eq!(edit.new_text, "renamed");
                        Location::new(uri.clone(), edit.range)
                    })
                })
                .collect();
            assert_eq!(sorted(actual), sorted(complete_edits), "{declare}");
        }
    }
}
