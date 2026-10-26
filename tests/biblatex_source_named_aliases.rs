use lsp_types::*;
use texquill::{language::features, project::workspace::Workspace};
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
const DEFAULT: &str = "@book{parent,title={Default parent},ids={old}} @book{member,title={Member}}";
const SELECTED: &str =
    "@book{parent,title={Selected parent},ids={old}} @book{member,title={Member}}";

#[test]
fn source_named_aliases_have_distinct_late_source_and_early_alias_identities() {
    for (body, source_prefix, early) in [
        (
            r"\newrefsection[chosen]\defbibentryset{old}{member}\cite{old}",
            r"\defbibentryset{",
            false,
        ),
        (
            r"\newrefsection[chosen]\cite{parent}\defbibentryset{old}{member}\cite{old}",
            r"\defbibentryset{",
            false,
        ),
        (
            r"\newrefsection[chosen]\cite{old}\defbibentryset{old}{member}\cite{old}",
            r"\defbibentryset{",
            true,
        ),
        (
            r"\newrefsection[chosen]\csuse{defbibentryset}{old}{member}\csname cite\endcsname{old}",
            r"\csuse{defbibentryset}{",
            false,
        ),
    ] {
        let (_dir, ws, main, _default, selected) = setup(body, SELECTED);
        let set = key(&ws, &main, source_prefix, "old");
        let alias = key(&ws, &selected, "ids={", "old");
        let doc = &ws.documents[&main];
        let start = doc.text.source.rfind("{old}").unwrap() + 1;
        let target = if early { alias.clone() } else { set.clone() };
        assert_eq!(
            features::definitions(&ws, &main, doc.text.position(start)),
            vec![target.clone()],
            "{body}"
        );
        let refs = features::references(&ws, &target.uri, target.range.start, true);
        assert_eq!(refs.len(), if early { 3 } else { 2 }, "{body}: {refs:?}");
        assert!(features::rename(&ws, &main, set.range.start, "freshset").is_err());
        assert!(features::rename(&ws, &selected, alias.range.start, "freshalias").is_err());
    }
}
#[test]
fn relations_and_wildcards_cannot_certify_source_named_aliases_by_request_order() {
    for body in [
        r"\newrefsection[chosen]\defbibentryset{old}{member}\cite{old,child}",
        r"\newrefsection[chosen]\defbibentryset{old}{member}\cite{child,old}",
        r"\newrefsection[chosen]\cite{child}\defbibentryset{old}{member}\cite{old}",
    ] {
        let (_dir, ws, main, _default, _selected) =
            setup(body, &format!("{SELECTED} @book{{child,crossref={{old}}}}"));
        let set = key(&ws, &main, r"\defbibentryset{", "old");
        assert!(features::references(&ws, &main, set.range.start, true).is_empty());
        assert!(features::rename(&ws, &main, set.range.start, "fresh").is_err());
    }
    let (_dir, ws, main, _default, _selected) = setup(
        r"\newrefsection[chosen]\defbibentryset{old}{member}\nocite{*}\cite{old}",
        SELECTED,
    );
    let query = key(&ws, &main, r"\cite{", "old");
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
}
#[test]
fn unsaved_relations_revoke_and_restore_only_certified_read_identities() {
    let (_dir, mut ws, main, _default, selected) = setup(
        r"\newrefsection[chosen]\defbibentryset{old}{member}\cite{old}",
        SELECTED,
    );
    let query = key(&ws, &main, r"\cite{", "old");
    let set = key(&ws, &main, r"\defbibentryset{", "old");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![set.clone()]
    );
    ws.open(
        selected.clone(),
        format!("{SELECTED} @book{{child,crossref={{old}}}}"),
        8,
    );
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    ws.open(selected, SELECTED.into(), 9);
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![set]
    );
}
#[test]
fn dynamic_relation_values_cannot_certify_source_named_aliases() {
    for (field, value) in ["crossref", "xref", "xdata", "related", "entryset"]
        .into_iter()
        .flat_map(|field| ["alias", "{o} # {ld}", "\"o\" # \"ld\""].map(|value| (field, value)))
    {
        let library = format!("@string{{alias=\"old\"}} {SELECTED} @book{{child,{field}={value}}}");
        let (_dir, ws, main, _default, _selected) = setup(
            r"\newrefsection[chosen]\defbibentryset{old}{member}\cite{old,child}",
            &library,
        );
        let query = key(&ws, &main, r"\cite{", "old");
        assert!(
            features::definitions(&ws, &main, query.range.start).is_empty(),
            "{field}: {value}"
        );
        let (_dir, ws, main, _default, _selected) = setup(
            r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collected}",
            &library,
        );
        let query = key(&ws, &main, r"\cite{", "collected");
        assert_eq!(
            features::definitions(&ws, &main, query.range.start),
            vec![key(&ws, &main, r"\defbibentryset{", "collected")]
        );
    }
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_expanded_relations_suppress_the_alias_named_source_set_without_warnings() {
    for value in ["alias", "{o} # {ld}"] {
        let library = format!(
            "@string{{alias=\"old\"}} {SELECTED} @book{{child,title={{Child}},crossref={value}}}"
        );
        let (dir, ws, main, _default, _selected) = setup(
            r"\newrefsection[chosen]\defbibentryset{old}{member}\cite{old,child}\printbibliography",
            &library,
        );
        let bbl = compile(dir.path());
        assert!(bbl.contains(r"\entry{parent}{book}"));
        assert!(bbl.contains(r"\keyalias{old}{parent}"));
        assert!(!bbl.contains(r"\entry{old}{set}"));
        let query = key(&ws, &main, r"\cite{", "old");
        assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    }
}
#[test]
fn shared_alias_and_source_executions_never_return_a_partial_reference_graph() {
    let (dir, mut ws, main, default, selected) = setup(
        r"\input{chapter}\newrefsection[chosen]\defbibentryset{old}{member}\input{chapter}",
        SELECTED,
    );
    let chapter = main.join("chapter.tex").unwrap();
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{old}").unwrap();
    ws.open(chapter.clone(), r"\cite{old}".into(), 3);
    let query = key(&ws, &chapter, r"\cite{", "old");
    assert!(features::definitions(&ws, &chapter, query.range.start).is_empty());
    for library in [default, selected] {
        let alias = key(&ws, &library, "ids={", "old");
        assert!(features::references(&ws, &library, alias.range.start, true).is_empty());
    }
}
#[test]
fn source_named_aliases_restore_independent_missing_diagnostics_without_editing() {
    let (_dir, ws, main, _default, _selected) = setup(
        r"\newrefsection[chosen]\defbibentryset{old}{member}\cite{old,absent}",
        SELECTED,
    );
    let doc = &ws.documents[&main];
    let start = doc.text.source.find("old,absent").unwrap();
    assert_eq!(
        features::definitions(&ws, &main, doc.text.position(start)),
        vec![key(&ws, &main, r"\defbibentryset{", "old")]
    );
    let warnings: Vec<_> = ws
        .diagnostics(&main)
        .into_iter()
        .filter(|d| d.code == Some(NumberOrString::String("undefined-citation".into())))
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("absent"));
    assert!(features::rename(&ws, &main, doc.text.position(start), "fresh").is_err());
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_multiple_source_names_and_other_aliases_keep_backend_identities() {
    for body in [
        r"\defbibentryset{parent}{member}\defbibentryset{old}{leaf}\cite{parent,old}",
        r"\defbibentryset{parent}{member}\defbibentryset{old}{leaf}\cite{old,parent}",
        r"\cite{other}\defbibentryset{old}{leaf}\cite{old}",
        r"\defbibentryset{old}{leaf}\defbibentryset{collected}{parent}\cite{old,collected}",
    ] {
        let body = format!(r"\newrefsection[chosen]{body}\printbibliography");
        let (dir,ws,main,_default,selected)=setup(&body,"@book{parent,title={Raw parent},ids={old,other}} @book{member,title={Member}} @book{leaf,title={Leaf}}");
        let bbl = compile(dir.path());
        assert!(bbl.contains(r"\entry{old}{set}"));
        assert!(!bbl.contains(r"\keyalias{old}{parent}"));
        let doc = &ws.documents[&main];
        let start = doc
            .text
            .source
            .rfind("old}")
            .or_else(|| doc.text.source.rfind("old,"))
            .unwrap();
        let old_start =
            doc.text.source.find(r"\defbibentryset{old}").unwrap() + r"\defbibentryset{".len();
        let old_set = Location::new(main.clone(), doc.text.range(old_start, old_start + 3));
        assert_eq!(
            features::definitions(&ws, &main, doc.text.position(start)),
            vec![old_set.clone()],
            "{body}"
        );
        assert_eq!(
            features::references(&ws, &main, old_set.range.start, true).len(),
            2,
            "{body}"
        );
        assert!(features::rename(&ws, &main, old_set.range.start, "fresh").is_err());
        if body.contains(r"\cite{other}") {
            let alias = key(&ws, &selected, "ids={old,", "other");
            let call = key(&ws, &main, r"\cite{", "other");
            assert_eq!(
                features::definitions(&ws, &main, call.range.start),
                vec![alias]
            );
            assert!(bbl.contains(r"\keyalias{other}{parent}"));
        }
    }
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_source_named_alias_reads_match_direct_and_early_backend_requests() {
    for selected in [false, true] {
        for early in [false, true] {
            let body = format!(
                "{}{}{}",
                if selected {
                    r"\newrefsection[chosen]"
                } else {
                    ""
                },
                if early { r"\cite{old}" } else { "" },
                r"\defbibentryset{old}{member}\cite{old}\printbibliography"
            );
            let (dir, ws, main, default, section) = setup(&body, SELECTED);
            let bbl = compile(dir.path());
            if early {
                assert!(bbl.contains(r"\entry{parent}{book}"));
                assert!(bbl.contains(r"\keyalias{old}{parent}"));
                assert!(!bbl.contains(r"\entry{old}{set}"));
            } else {
                assert!(bbl.contains(r"\entry{old}{set}"));
                assert!(bbl.contains(r"\entry{member}{book}"));
                assert!(!bbl.contains(r"\keyalias{old}{parent}"));
            }
            let doc = &ws.documents[&main];
            let start = doc.text.source.rfind("{old}").unwrap() + 1;
            let target = if early {
                key(
                    &ws,
                    if selected { &section } else { &default },
                    "ids={",
                    "old",
                )
            } else {
                key(&ws, &main, r"\defbibentryset{", "old")
            };
            assert_eq!(
                features::definitions(&ws, &main, doc.text.position(start)),
                vec![target]
            );
        }
    }
}
