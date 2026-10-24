use latex_lsp::{features, workspace::Workspace};
use tower_lsp::lsp_types::*;
fn setup(body: &str, database: &str, selected: bool) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let resource = if selected {
        r"\addsectionbib[label=chosen]{refs.bib}"
    } else {
        r"\addbibresource{refs.bib}"
    };
    let section = if selected {
        r"\newrefsection[chosen]"
    } else {
        ""
    };
    let source = format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{biblatex}}{resource}\\begin{{document}}{section}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(dir.path().join("refs.bib"), database).unwrap();
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = main.join("refs.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, bib)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let start = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert_eq!(&doc.text.source[start..start + name.len()], name);
    Location::new(uri.clone(), doc.text.range(start, start + name.len()))
}
const DATABASE: &str = "@book{parent,title={Raw parent},ids={old}} @book{member,title={Member}}";
#[test]
fn source_overridden_alias_owners_keep_the_alias_and_primary_readonly_identities() {
    for selected in [false, true] {
        for prefix in ["", r"\cite{old}"] {
            let (_dir, ws, main, bib) = setup(
                &format!(r"{prefix}\defbibentryset{{parent}}{{member}}\cite{{parent,old}}"),
                DATABASE,
                selected,
            );
            let source = key(&ws, &main, r"\defbibentryset{", "parent");
            let alias = key(&ws, &bib, "ids={", "old");
            let doc = &ws.documents[&main];
            let at = doc.text.source.rfind(r"\cite{parent,old}").unwrap() + r"\cite{".len();
            assert_eq!(
                features::definitions(&ws, &main, doc.text.position(at)),
                vec![source.clone()]
            );
            assert_eq!(
                features::definitions(&ws, &main, doc.text.position(at + "parent,".len())),
                vec![alias.clone()]
            );
            let hover =
                features::hover(&ws, &main, doc.text.position(at + "parent,".len())).unwrap();
            let HoverContents::Markup(contents) = hover.contents else {
                panic!("plain markup expected")
            };
            assert!(
                contents.value.contains("Bibliography alias of parent"),
                "{contents:?}"
            );
            assert!(!contents.value.contains("Raw parent"), "{contents:?}");
            assert!(features::references(&ws, &main, source.range.start, true).is_empty());
            assert!(features::prepare_rename(&ws, &bib, alias.range.start).is_err());
            assert!(features::rename(&ws, &main, source.range.start, "newset").is_err());
        }
    }
}
#[test]
fn an_early_primary_request_keeps_the_owner_book_without_reclassifying_its_alias() {
    for selected in [false, true] {
        let (_dir, ws, main, bib) = setup(
            r"\cite{parent}\defbibentryset{parent}{member}\cite{old,parent}",
            DATABASE,
            selected,
        );
        assert_eq!(
            features::definitions(&ws, &main, key(&ws, &main, r"\cite{", "parent").range.start),
            vec![key(&ws, &bib, "@book{", "parent")]
        );
        assert_eq!(
            features::definitions(
                &ws,
                &main,
                key(&ws, &main, r"\cite{old,", "parent").range.start
            ),
            vec![key(&ws, &bib, "@book{", "parent")]
        );
        let doc = &ws.documents[&main];
        let at = doc.text.source.find(r"\cite{old,parent}").unwrap() + r"\cite{".len();
        assert_eq!(
            features::definitions(&ws, &main, doc.text.position(at)),
            vec![key(&ws, &bib, "ids={", "old")]
        );
    }
}
#[test]
fn indirect_nested_members_remain_unconfirmed_even_when_the_owner_is_declared_later() {
    for body in [
        r"\defbibentryset{parent}{member}\defbibentryset{collected}{old}\cite{collected,old}",
        r"\defbibentryset{collected}{old}\cite{collected,old}\defbibentryset{parent}{member}",
    ] {
        let (_dir, ws, main, _) = setup(body, DATABASE, true);
        let query = key(&ws, &main, r"\cite{", "collected");
        assert!(features::definitions(&ws, &main, query.range.start).is_empty());
        assert!(features::rename(&ws, &main, query.range.start, "newset").is_err());
    }
}
#[test]
fn invalid_alias_values_and_unsupported_owner_or_alias_name_intersections_stay_guarded() {
    for database in [
        "@book{parent,ids=names} @book{member}",
        "@book{parent,ids={old},IDS={other}} @book{member}",
        "@book{parent,ids={old}} @book{other,ids={old}} @book{member}",
        "@xdata{parent,ids={old}} @book{member}",
        "@set{parent,ids={old}} @book{member}",
        "@book{parent,ids={old,parent}} @book{member}",
    ] {
        let (_dir, ws, main, _) = setup(
            r"\defbibentryset{parent}{member}\cite{parent,old}",
            database,
            true,
        );
        assert!(
            features::definitions(&ws, &main, key(&ws, &main, r"\cite{", "parent").range.start)
                .is_empty(),
            "{database}"
        );
    }
}
#[test]
fn unsaved_alias_name_conflicts_revoke_and_restore_the_same_parent_version() {
    let (_dir, mut ws, main, bib) = setup(
        r"\defbibentryset{parent}{member}\cite{parent,old}",
        DATABASE,
        true,
    );
    let query = key(&ws, &main, r"\cite{", "parent");
    let expected = key(&ws, &main, r"\defbibentryset{", "parent");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![expected.clone()]
    );
    ws.open(
        bib.clone(),
        DATABASE.replace("ids={old}", "ids={parent}"),
        8,
    );
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    ws.documents.remove(&bib);
    ws.reload(&bib);
    assert_eq!(ws.documents[&main].version, Some(5));
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![expected]
    );
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
    assert!(!blg.contains("WARN") && !blg.contains("ERROR"), "{blg}");
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_source_overridden_alias_owners_match_the_navigation_phases() {
    for selected in [false, true] {
        for (body, kind) in [
            (r"\defbibentryset{parent}{member}\cite{old}", "set"),
            (
                r"\cite{old}\defbibentryset{parent}{member}\cite{old}",
                "set",
            ),
            (
                r"\cite{parent}\defbibentryset{parent}{member}\cite{old,parent}",
                "book",
            ),
            (r"\defbibentryset{parent}{member}\cite{child}", "set"),
        ] {
            let database = format!("{DATABASE} @book{{child,title={{Child}},crossref={{old}}}}");
            let (dir, ws, main, bib) =
                setup(&format!(r"{body}\printbibliography"), &database, selected);
            let bbl = compile(dir.path());
            assert!(
                bbl.contains(&format!(r"\entry{{parent}}{{{kind}}}")),
                "{body}: {bbl}"
            );
            assert!(bbl.contains(r"\keyalias{old}{parent}"), "{body}: {bbl}");
            assert_eq!(
                bbl.contains(r"\entry{member}{book}"),
                kind == "set",
                "{body}: {bbl}"
            );
            let source = key(&ws, &main, r"\defbibentryset{", "parent");
            let primary = key(&ws, &bib, "@book{", "parent");
            let query = if kind == "set" {
                source.clone()
            } else {
                key(&ws, &main, r"\cite{", "parent")
            };
            assert_eq!(
                features::definitions(&ws, &main, query.range.start),
                vec![if kind == "set" {
                    source.clone()
                } else {
                    primary
                }]
            );
            if body.contains(r"\cite{old}") {
                let alias = key(&ws, &bib, "ids={", "old");
                let query = key(&ws, &main, r"\cite{", "old");
                assert_eq!(
                    features::definitions(&ws, &main, query.range.start),
                    vec![alias]
                );
            }
        }
    }
}
