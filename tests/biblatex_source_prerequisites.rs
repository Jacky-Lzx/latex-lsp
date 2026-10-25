use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;
fn setup(body: &str, selected_text: &str) -> (tempfile::TempDir, Workspace, Url, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source=format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{biblatex}}\\addbibresource{{default.bib}}\\addsectionbib[label=chosen]{{section.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(dir.path().join("default.bib"), DEFAULT).unwrap();
    std::fs::write(dir.path().join("section.bib"), selected_text).unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let default = main.join("default.bib").unwrap();
    let bib = main.join("section.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, default, bib)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let start = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert_eq!(&doc.text.source[start..start + name.len()], name);
    Location::new(uri.clone(), doc.text.range(start, start + name.len()))
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
const DEFAULT: &str = "@book{parent,title={Default parent},ids={old}} @book{child,title={Default child},crossref={old}} @book{member,title={Member}}";
const SELECTED: &str = "@book{parent,title={Selected parent},ids={old}} @book{child,title={Selected child},crossref={old}} @book{member,title={Member}}";

fn setup_shared(body: &str) -> (tempfile::TempDir, Workspace, Url, Url, Url, Url) {
    let (dir, mut ws, main, default, selected) = setup(body, SELECTED);
    let provider = main.join("sets.tex").unwrap();
    std::fs::write(
        dir.path().join("sets.tex"),
        "% 😀\r\n\\defbibentryset{parent}{member}",
    )
    .unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 6);
    (dir, ws, main, default, selected, provider)
}
const SHARED_BODY: &str =
    r"\input{sets}\cite{parent,old}\newrefsection[chosen]\input{sets}\cite{parent,old}";
#[test]
fn one_physical_source_declaration_moves_all_raw_prerequisites_atomically() {
    for early in [false, true] {
        let body = if early {
            SHARED_BODY.replace(r"\input{sets}", r"\cite{old}\input{sets}")
        } else {
            SHARED_BODY.into()
        };
        let (_dir, ws, main, default, selected, provider) = setup_shared(&body);
        let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
        assert_eq!(
            features::references(&ws, &provider, declaration.range.start, true).len(),
            3
        );
        let edits = features::rename(&ws, &provider, declaration.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits.len(), 4);
        assert_eq!(edits[&main].len(), 2);
        assert_eq!(edits[&provider].len(), 1);
        assert_eq!(edits[&default].len(), 1);
        assert_eq!(edits[&selected].len(), 1);
    }
}

#[test]
fn independent_raw_consumers_and_distinct_source_identities_stay_protected() {
    for body in [
        r"\cite{parent}\input{sets}\cite{parent,old}\newrefsection[chosen]\input{sets}\cite{parent,old}",
        r"\input{sets}\nocite{*}\newrefsection[chosen]\input{sets}\cite{parent,old}",
        r"\input{sets}\cite{parent,old}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,old}",
    ] {
        let (_dir, ws, _main, _default, _selected, provider) = setup_shared(body);
        let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
        assert!(
            features::rename(&ws, &provider, declaration.range.start, "fresh").is_err(),
            "{body}"
        );
    }
    let (_dir, mut ws, main, _default, _selected, provider) = setup_shared(SHARED_BODY);
    let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
    let other = main.join("other.tex").unwrap();
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{default.bib}\begin{document}\cite{old}\end{document}".into(),9);
    assert!(features::rename(&ws, &provider, declaration.range.start, "fresh").is_err());
    ws.documents.remove(&other);
    assert!(
        features::rename(&ws, &provider, declaration.range.start, "fresh")
            .unwrap()
            .is_some()
    );
}
#[test]
fn coupled_source_new_names_cannot_capture_other_numeric_sections() {
    for extra in [
        r"\cite{fresh}",
        r"\defbibentryset{fresh}{member}\cite{fresh}",
        r"\cite{foreign}",
    ] {
        let (dir, mut ws, main, _default, _selected, provider) =
            setup_shared(&format!(r"{SHARED_BODY}\newrefsection[third]{extra}"));
        let library = if extra == r"\cite{foreign}" {
            "@book{member} @book{foreign,ids={other}} @book{child,crossref={fresh}}"
        } else {
            "@book{member} @book{foreign,ids={other}}"
        };
        std::fs::write(dir.path().join("third.bib"), library).unwrap();
        let source = ws.documents[&main].text.source.replace(
            r"\begin{document}",
            r"\addsectionbib[label=third]{third.bib}\begin{document}",
        );
        ws.open(main, source, 7);
        let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
        assert!(
            features::rename(&ws, &provider, declaration.range.start, "fresh").is_err(),
            "{extra}"
        );
    }
}
#[test]
fn incomplete_alias_owner_closures_and_non_ids_prerequisites_remain_protected() {
    let (_dir, mut ws, _main, _default, selected, provider) = setup_shared(SHARED_BODY);
    let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
    for library in [
        SELECTED.replace(",ids={old}", ""),
        format!("{SELECTED} @book{{different,ids={{old}}}}"),
        SELECTED.replace("ids={old}", "ids=unexpanded"),
    ] {
        ws.open(selected.clone(), library, 9);
        assert!(features::rename(&ws, &provider, declaration.range.start, "fresh").is_err());
    }
    ws.open(selected, SELECTED.into(), 10);
    assert!(
        features::rename(&ws, &provider, declaration.range.start, "fresh")
            .unwrap()
            .is_some()
    );
}
#[test]
fn applying_the_complete_edit_preserves_source_identity_and_alias_names() {
    let (_dir, mut ws, main, _default, _selected, provider) = setup_shared(SHARED_BODY);
    let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
    let edits = features::rename(&ws, &provider, declaration.range.start, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    for (uri, mut edits) in edits {
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let text = ws.documents[&uri]
            .text
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
            .source;
        std::fs::write(uri.to_file_path().unwrap(), &text).unwrap();
        ws.open(uri, text, 10);
    }
    let query = key(&ws, &main, r"\cite{", "fresh");
    let moved = key(&ws, &provider, r"\defbibentryset{", "fresh");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![moved]
    );
    assert_eq!(
        features::references(&ws, &main, query.range.start, true).len(),
        3
    );
    assert!(ws.documents[&main]
        .text
        .source
        .contains(r"\cite{fresh,old}"));
    assert!(ws
        .diagnostics(&main)
        .iter()
        .all(|diag| diag.code != Some(NumberOrString::String("undefined-citation".into()))));
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_multi_prerequisite_source_edits_preserve_every_compiled_record() {
    for body in [
        r"\input{sets}\cite{parent,old}\printbibliography\newrefsection[chosen]\input{sets}\cite{parent,old}\printbibliography",
        r"\cite{old}\input{sets}\cite{parent,old}\printbibliography\newrefsection[chosen]\input{sets}\cite{parent,old}\printbibliography",
        r"\cite{old}\input{sets}\cite{parent,old}\printbibliography\newrefsection[chosen]\cite{old}\input{sets}\cite{parent,old}\printbibliography",
        r"\input{sets}\cite{parent,old,child}\printbibliography\newrefsection[chosen]\input{sets}\cite{parent,old,child}\printbibliography",
    ] {
        let (dir, ws, _main, _default, _selected, provider) = setup_shared(body);
        let before = compile(dir.path());
        assert_eq!(before.matches(r"\entry{parent}{set}").count(), 2);
        let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
        let edits = features::rename(&ws, &provider, declaration.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        for (uri, mut edits) in edits {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let text = ws.documents[&uri]
                .text
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
                .source;
            std::fs::write(uri.to_file_path().unwrap(), text).unwrap();
        }
        for ext in ["aux", "bcf", "bbl", "run.xml"] {
            let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
        }
        assert_eq!(
            compile(dir.path()).replace("fresh", "parent"),
            before,
            "{body}"
        );
    }
}

#[test]
fn source_prerequisite_limit_accepts_a_complete_boundary_and_refuses_partial_edits() {
    for count in [64, 65] {
        let dir = tempfile::tempdir().unwrap();
        let mut preamble = String::new();
        let mut body = String::new();
        for index in 0..count {
            preamble.push_str(&format!(
                r"\addsectionbib[label=s{index}]{{lib{index}.bib}}"
            ));
            body.push_str(&format!(
                r"\newrefsection[s{index}]\input{{sets}}\cite{{parent,old}}"
            ));
            std::fs::write(
                dir.path().join(format!("lib{index}.bib")),
                "@book{parent,ids={old}} @book{member}",
            )
            .unwrap();
        }
        std::fs::write(
            dir.path().join("sets.tex"),
            r"\defbibentryset{parent}{member}",
        )
        .unwrap();
        let source = format!(
            r"\documentclass{{article}}\usepackage{{biblatex}}{preamble}\begin{{document}}{body}\end{{document}}"
        );
        std::fs::write(dir.path().join("main.tex"), &source).unwrap();
        let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
        let provider = main.join("sets.tex").unwrap();
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        ws.open(main, source, 6);
        let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
        let result = features::rename(&ws, &provider, declaration.range.start, "fresh");
        if count == 64 {
            let edits = result.unwrap().unwrap().changes.unwrap();
            assert_eq!(edits.len(), 66);
            assert_eq!(edits.values().map(Vec::len).sum::<usize>(), 129);
        } else {
            assert!(result.is_err());
        }
    }
}

fn setup_non_ids_prerequisites(
    default_ids: bool,
    selected_ids: bool,
    relations: bool,
    repeat: bool,
) -> (tempfile::TempDir, Workspace, Url, Url, Url, Url) {
    let keys = |ids| {
        format!(
            "parent{}{}",
            if ids { ",old" } else { "" },
            if relations { ",child" } else { "" }
        )
    };
    let first = format!(
        r"\input{{sets}}\cite{{{}}}\printbibliography",
        keys(default_ids)
    );
    let second = format!(
        r"\newrefsection[chosen]\input{{sets}}\cite{{{}}}\printbibliography",
        keys(selected_ids)
    );
    let body = format!(
        "{first}{second}{}",
        if repeat {
            format!(r"\newrefsection{first}")
        } else {
            String::new()
        }
    );
    let (dir, mut ws, main, default, selected, provider) = setup_shared(&body);
    for (uri, title, year, ids) in [
        (&default, "Default data", 2001, default_ids),
        (&selected, "Selected data", 2002, selected_ids),
    ] {
        let library = format!("@book{{parent,title={{{title}}},year={{{year}}}{}}} @book{{member,title={{Member}}}}{}", if ids { ",ids={old}" } else { "" }, if relations { " @book{child,title={Child},crossref={parent}}" } else { "" });
        std::fs::write(uri.to_file_path().unwrap(), &library).unwrap();
        ws.open(uri.clone(), library, 7);
    }
    (dir, ws, main, default, selected, provider)
}
const NON_IDS_CASES: &[(bool, bool, bool, bool)] = &[
    (true, false, false, false),
    (false, true, false, false),
    (true, false, true, false),
    (false, false, true, false),
    (true, false, true, true),
];

#[test]
fn mixed_and_non_ids_source_prerequisites_move_every_header_and_relation_atomically() {
    for &(default_ids, selected_ids, relations, repeat) in NON_IDS_CASES {
        let (_dir, ws, main, default, selected, provider) =
            setup_non_ids_prerequisites(default_ids, selected_ids, relations, repeat);
        let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
        let edits = features::rename(&ws, &provider, declaration.range.start, "fresh")
            .unwrap_or_else(|error| {
                panic!("{default_ids} {selected_ids} {relations} {repeat}: {error}")
            })
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits.len(), 4);
        assert_eq!(edits[&main].len(), if repeat { 3 } else { 2 });
        assert_eq!(edits[&provider].len(), 1);
        for uri in [&default, &selected] {
            assert_eq!(edits[uri].len(), if relations { 2 } else { 1 });
            let doc = &ws.documents[uri];
            for edit in &edits[uri] {
                let start = doc.text.offset(edit.range.start).unwrap();
                let end = doc.text.offset(edit.range.end).unwrap();
                assert_eq!(&doc.text.source[start..end], "parent");
                assert_eq!(edit.new_text, "fresh");
            }
        }
    }
}

#[test]
fn non_ids_source_prerequisites_keep_independent_consumers_and_missing_headers_protected() {
    let (_dir, mut ws, main, _default, selected, provider) =
        setup_non_ids_prerequisites(true, false, true, false);
    let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
    let other = main.join("other.tex").unwrap();
    ws.open(other.clone(), r"\documentclass{article}\usepackage{biblatex}\addbibresource{default.bib}\begin{document}\cite{parent}\end{document}".into(), 8);
    assert!(features::rename(&ws, &provider, declaration.range.start, "fresh").is_err());
    ws.documents.remove(&other);
    assert!(
        features::rename(&ws, &provider, declaration.range.start, "fresh")
            .unwrap()
            .is_some()
    );
    let library = ws.documents[&selected].text.source.clone();
    ws.open(
        selected.clone(),
        library.replace("@book{parent,", "@book{different,"),
        9,
    );
    assert!(features::rename(&ws, &provider, declaration.range.start, "fresh").is_err());
    ws.open(selected.clone(), format!("{library} @book{{fresh}}"), 10);
    assert!(features::rename(&ws, &provider, declaration.range.start, "fresh").is_err());
    ws.open(selected, library, 11);
    assert!(
        features::rename(&ws, &provider, declaration.range.start, "fresh")
            .unwrap()
            .is_some()
    );
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_non_ids_source_prerequisite_edits_preserve_inheritance_sets_and_complete_bbl() {
    for &(default_ids, selected_ids, relations, repeat) in NON_IDS_CASES {
        let (dir, ws, _main, _default, _selected, provider) =
            setup_non_ids_prerequisites(default_ids, selected_ids, relations, repeat);
        let before = compile(dir.path());
        assert_eq!(
            before.matches(r"\entry{parent}{set}").count(),
            if repeat { 3 } else { 2 }
        );
        let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
        let edits = features::rename(&ws, &provider, declaration.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        for (uri, mut edits) in edits {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let source = ws.documents[&uri]
                .text
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
                .source;
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        for ext in ["aux", "bcf", "bbl", "run.xml"] {
            let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
        }
        assert_eq!(
            compile(dir.path()).replace("fresh", "parent"),
            before,
            "{default_ids} {selected_ids} {relations} {repeat}"
        );
    }
}

#[test]
fn cached_source_returns_require_the_same_final_physical_set_in_every_consumer() {
    for variant in ["early-primary", "no-source", "wildcard", "different-source"] {
        let (dir, mut ws, main, _default, selected, provider) =
            setup_non_ids_prerequisites(true, false, true, true);
        let original = ws.documents[&main].text.source.clone();
        let replacement = match variant {
            "early-primary" => r"\newrefsection\cite{parent}\input{sets}",
            "no-source" => r"\newrefsection",
            "wildcard" => r"\newrefsection\input{sets}\nocite{*}",
            _ => r"\newrefsection\input{sets}",
        };
        let source = if variant == "different-source" {
            std::fs::write(
                dir.path().join("sets-other.tex"),
                r"\defbibentryset{parent}{othermember}",
            )
            .unwrap();
            let library = format!(
                "{} @book{{othermember,title={{Other member}}}}",
                ws.documents[&selected].text.source
            );
            std::fs::write(selected.to_file_path().unwrap(), &library).unwrap();
            ws.open(selected.clone(), library, 8);
            original.replace(
                r"\newrefsection[chosen]\input{sets}",
                r"\newrefsection[chosen]\input{sets-other}",
            )
        } else {
            original.replace(r"\newrefsection\input{sets}", replacement)
        };
        ws.open(main.clone(), source, 9);
        let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
        if variant == "different-source" {
            let other_provider = main.join("sets-other.tex").unwrap();
            let other_declaration = key(&ws, &other_provider, r"\defbibentryset{", "parent");
            let selected_query = key(
                &ws,
                &main,
                r"\newrefsection[chosen]\input{sets-other}\cite{",
                "parent",
            );
            assert_eq!(
                features::definitions(&ws, &main, selected_query.range.start),
                vec![other_declaration]
            );
            let returned_query = key(&ws, &main, r"\newrefsection\input{sets}\cite{", "parent");
            assert_eq!(
                features::definitions(&ws, &main, returned_query.range.start),
                vec![declaration.clone()]
            );
        }
        assert!(
            features::rename(&ws, &provider, declaration.range.start, "fresh").is_err(),
            "{variant}"
        );
    }
}

#[test]
fn cached_source_return_edits_keep_alias_names_and_restore_navigation_after_application() {
    for early_alias in [false, true] {
        let (_dir, mut ws, main, _default, _selected, provider) =
            setup_non_ids_prerequisites(true, false, true, true);
        if early_alias {
            let source = ws.documents[&main].text.source.replace(
                r"\newrefsection\input{sets}",
                r"\newrefsection\cite{old}\input{sets}",
            );
            ws.open(main.clone(), source, 8);
        }
        let declaration = key(&ws, &provider, r"\defbibentryset{", "parent");
        let edits = features::rename(&ws, &provider, declaration.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits.len(), 4);
        assert_eq!(edits.values().map(Vec::len).sum::<usize>(), 8);
        for (uri, mut edits) in edits {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let source = ws.documents[&uri]
                .text
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
                .source;
            ws.open(uri, source, 10);
        }
        let moved = key(&ws, &provider, r"\defbibentryset{", "fresh");
        let query = key(&ws, &main, r"\cite{", "fresh");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![moved]
        );
        assert_eq!(
            features::references(&ws, &provider, declaration.range.start, true).len(),
            6
        );
        assert!(ws.documents[&main]
            .text
            .source
            .contains(r"\cite{fresh,old,child}"));
    }
}
