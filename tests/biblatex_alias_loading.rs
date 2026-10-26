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
fn missing(ws: &Workspace, uri: &Url) -> Vec<String> {
    ws.diagnostics(uri)
        .into_iter()
        .filter(|item| item.code == Some(NumberOrString::String("undefined-citation".into())))
        .map(|item| item.data.unwrap()["key"].as_str().unwrap().to_owned())
        .collect()
}

const DEFAULT: &str = "@book{parent,title={Default parent},ids={old}}";
const IMPORTED: &str = "@book{member,title={Member}} @book{child,title={Child},crossref={old}}";
const CLOSED:&str="@book{parent,title={Selected parent},ids={old}} @book{member,title={Member}} @book{child,title={Child},crossref={old}}";
#[test]
fn imported_alias_cache_entries_never_produce_missing_guesses_or_partial_alias_edits() {
    for set in ["", r"\defbibentryset{collected}{member}"] {
        for call in [r"\cite", r"\csuse{cite}", r"\csname cite\endcsname"] {
            let (_dir, ws, main, default, _bib) = setup(
                &format!(
                    r"\cite{{old}}\newrefsection[chosen]{set}{call}{{old,child,absent}}\label{{target}}\ref{{target}}"
                ),
                IMPORTED,
            );
            let alias = key(&ws, &default, "ids={", "old");
            assert!(features::references(&ws, &default, alias.range.start, true).is_empty());
            assert!(features::prepare_rename(&ws, &default, alias.range.start).is_err());
            assert!(features::rename(&ws, &default, alias.range.start, "newalias").is_err());
            assert!(missing(&ws, &main).is_empty());
            let label = key(&ws, &main, r"\label{", "target");
            let reference = key(&ws, &main, r"\ref{", "target");
            assert_eq!(
                features::definitions(&ws, &main, reference.range.start),
                vec![label]
            );
            let selection = key(&ws, &main, r"\newrefsection[", "chosen");
            assert_eq!(
                features::definitions(&ws, &main, selection.range.start).len(),
                1
            );
        }
    }
}
#[test]
fn empty_resource_sections_cannot_hide_citations_to_cached_global_aliases() {
    let (dir, mut ws, main, _default, bib) = setup(
        r"\newrefsection[chosen]\cite{old}\newrefsection\cite{old}",
        CLOSED,
    );
    let text = ws.documents[&main]
        .text
        .source
        .replace(r"\addbibresource{default.bib}", "");
    std::fs::write(dir.path().join("main.tex"), &text).unwrap();
    ws.open(main.clone(), text, 6);
    let alias = key(&ws, &bib, "ids={", "old");
    assert!(features::references(&ws, &bib, alias.range.start, true).is_empty());
    assert!(features::rename(&ws, &bib, alias.range.start, "newalias").is_err());
    assert!(missing(&ws, &main).is_empty());
}
#[test]
fn shared_ids_owners_move_together_and_new_alias_capture_is_protected() {
    let (_dir, ws, _main, default, bib) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{old}\newrefsection\cite{capture}",
        CLOSED,
    );
    let alias = key(&ws, &bib, "ids={", "old");
    assert_eq!(
        features::references(&ws, &bib, alias.range.start, true).len(),
        3
    );
    assert!(features::rename(&ws, &bib, alias.range.start, "freshalias")
        .unwrap()
        .is_some());
    assert!(features::rename(&ws, &bib, alias.range.start, "capture").is_err());
    let owner = key(&ws, &default, "@book{", "parent");
    assert!(features::prepare_rename(&ws, &default, owner.range.start)
        .unwrap()
        .is_some());
    let changes = features::rename(&ws, &default, owner.range.start, "newowner")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[&default].len(), 1);
    assert_eq!(changes[&bib].len(), 1);
}
#[test]
fn unsaved_alias_namespace_changes_revoke_complete_edits_and_restoration_recovers() {
    let (_dir, mut ws, main, _default, bib) =
        setup(r"\cite{old}\newrefsection[chosen]\cite{old,child}", CLOSED);
    let alias = key(&ws, &bib, "ids={", "old");
    let expected = features::rename(&ws, &bib, alias.range.start, "newalias")
        .unwrap()
        .unwrap();
    ws.open(bib.clone(), IMPORTED.into(), 8);
    let early = key(&ws, &main, r"\cite{", "old");
    assert!(features::references(&ws, &main, early.range.start, true).is_empty());
    assert!(features::rename(&ws, &main, early.range.start, "newalias").is_err());
    ws.open(bib.clone(), CLOSED.into(), 9);
    assert_eq!(
        features::rename(&ws, &bib, alias.range.start, "newalias")
            .unwrap()
            .unwrap(),
        expected
    );
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_global_alias_cache_loads_a_parent_without_a_local_owner_declaration() {
    for set in ["", r"\defbibentryset{collected}{member}"] {
        let (dir, ws, main, default, _bib) = setup(
            &format!(
                r"\cite{{old}}\newrefsection[chosen]{set}\cite{{old,child}}\printbibliography"
            ),
            IMPORTED,
        );
        let bbl = compile(dir.path());
        let selected = bbl
            .split_once(r"\refsection{1}")
            .unwrap()
            .1
            .split_once(r"\endrefsection")
            .unwrap()
            .0;
        assert!(selected.contains(r"\keyalias{old}{parent}"), "{bbl}");
        assert!(selected.contains(r"\entry{parent}{book}"), "{bbl}");
        let alias = key(&ws, &default, "ids={", "old");
        assert!(features::rename(&ws, &default, alias.range.start, "newalias").is_err());
        assert!(missing(&ws, &main).is_empty());
    }
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_new_aliases_can_capture_later_citations_in_another_resource_section() {
    let (dir, ws, main, _default, bib) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{old}\newrefsection\cite{capture}",
        CLOSED,
    );
    compile_case(dir.path(), true);
    let before = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
    assert!(before.contains("database entry for 'capture'"), "{before}");
    let alias = key(&ws, &bib, "ids={", "old");
    assert!(features::rename(&ws, &bib, alias.range.start, "capture").is_err());
    let changed = ws.documents[&main].text.source.replace(
        r"\newrefsection[chosen]\cite{old}",
        r"\newrefsection[chosen]\cite{capture}",
    );
    std::fs::write(dir.path().join("main.tex"), changed).unwrap();
    std::fs::write(
        dir.path().join("section.bib"),
        CLOSED
            .replace("ids={old}", "ids={capture}")
            .replace("crossref={old}", "crossref={capture}"),
    )
    .unwrap();
    for ext in ["aux", "bcf", "bbl", "run.xml"] {
        let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
    }
    let after = compile(dir.path());
    let body = after
        .split_once(r"\refsection{2}")
        .unwrap()
        .1
        .split_once(r"\endrefsection")
        .unwrap()
        .0;
    assert!(body.contains(r"\keyalias{capture}{parent}"), "{after}");
    assert!(body.contains(r"\entry{parent}{book}"), "{after}");
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_bare_primary_keys_do_not_inherit_global_alias_cache_visibility() {
    let (dir, ws, main, _default, _bib) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{parent}",
        "@book{member,title={Member}}",
    );
    let bbl = compile_case(dir.path(), true);
    let log = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
    assert!(log.contains("database entry for 'parent'"), "{log}");
    assert_eq!(bbl.matches(r"\entry{parent}").count(), 1, "{bbl}");
    assert_eq!(missing(&ws, &main), vec!["parent"]);
}

#[test]
fn tex_requests_source_members_and_database_relations_require_separate_local_alias_proofs() {
    for (body, selected) in [
        (
            r"\cite{old}\newrefsection[chosen]\cite{old}",
            "@book{member}",
        ),
        (
            r"\cite{old}\newrefsection[chosen]\nocite{old}",
            "@book{member}",
        ),
        (
            r"\cite{old}\newrefsection[chosen]\csuse{cite}{old}",
            "@book{member}",
        ),
        (
            r"\cite{old}\newrefsection[chosen]\csname cite\endcsname{old}",
            "@book{member}",
        ),
        (
            r"\cite{old}\newrefsection[chosen]\defbibentryset{collected}{old}\cite{collected}",
            "@book{member}",
        ),
        (
            r"\cite{old}\newrefsection[chosen]\cite{child}",
            "@book{child,crossref={old}}",
        ),
    ] {
        let (_dir, ws, main, default, _bib) = setup(body, selected);
        let alias = key(&ws, &default, "ids={", "old");
        assert!(
            features::references(&ws, &default, alias.range.start, true).is_empty(),
            "{body}"
        );
        assert!(
            features::prepare_rename(&ws, &default, alias.range.start).is_err(),
            "{body}"
        );
        assert!(
            features::rename(&ws, &default, alias.range.start, "freshalias").is_err(),
            "{body}"
        );
        assert!(missing(&ws, &main).is_empty(), "{body}");
    }
}

#[test]
fn new_primary_and_coupled_source_names_cannot_delete_another_sections_global_alias() {
    for set in ["", r"\defbibentryset{member}{child}"] {
        let (_dir, ws, main, _default, bib) = setup(
            &format!(
                r"\cite{{old}}\newrefsection[chosen]{set}\cite{{member}}\newrefsection\cite{{old}}"
            ),
            "@book{member,ids={local}} @book{child}",
        );
        let query = if set.is_empty() {
            key(&ws, &bib, "@book{", "member")
        } else {
            key(&ws, &main, r"\defbibentryset{", "member")
        };
        assert!(
            features::rename(&ws, &query.uri, query.range.start, "freshprimary")
                .unwrap()
                .is_some()
        );
        assert!(features::rename(&ws, &query.uri, query.range.start, "old").is_err());
    }
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_a_new_real_key_can_delete_global_ids_needed_by_later_sections() {
    let (dir, ws, main, _default, bib) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{member}\newrefsection\cite{old}",
        "@book{member,title={Selected member}}",
    );
    compile(dir.path());
    let query = key(&ws, &bib, "@book{", "member");
    assert!(features::rename(&ws, &bib, query.range.start, "old").is_err());
    let changed = ws.documents[&main]
        .text
        .source
        .replace(r"\cite{member}", r"\cite{old}");
    std::fs::write(dir.path().join("main.tex"), changed).unwrap();
    std::fs::write(
        dir.path().join("section.bib"),
        "@book{old,title={Selected member}}",
    )
    .unwrap();
    for ext in ["aux", "bcf", "bbl", "run.xml"] {
        let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
    }
    compile_case(dir.path(), true);
    let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
    assert!(
        blg.contains("Citekey alias 'old' is also a real entry key"),
        "{blg}"
    );
    assert!(blg.contains("database entry for 'old'"), "{blg}");
}
