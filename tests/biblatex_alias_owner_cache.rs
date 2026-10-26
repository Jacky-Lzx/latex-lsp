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
const DEFAULT: &str = "@book{parent,title={Default parent},ids={old}}";
const SELECTED: &str =
    "@book{parent,title={Selected parent},ids={other}} @book{member,title={Member}}";
const RETURN: &str = r"\cite{old}\newrefsection[chosen]\cite{other}\newrefsection\cite{old}";
#[test]
fn repeated_resource_alias_requests_do_not_claim_the_local_owner_after_cache_replacement() {
    let (_dir, ws, main, default, _bib) = setup(RETURN, SELECTED);
    let late = key(&ws, &main, r"\newrefsection\cite{", "old");
    assert!(features::definitions(&ws, &main, late.range.start).is_empty());
    assert!(features::hover(&ws, &main, late.range.start).is_none());
    let primary = key(&ws, &default, "@book{", "parent");
    assert!(features::references(&ws, &default, primary.range.start, true).is_empty());
    assert!(features::prepare_rename(&ws, &default, primary.range.start).is_err());
    assert!(features::rename(&ws, &default, primary.range.start, "newparent").is_err());
}
#[test]
fn numeric_processing_order_distinguishes_returning_to_section_zero_from_a_new_section() {
    let (_dir, ws, main, default, _bib) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{other}\endrefsection\cite{old}",
        SELECTED,
    );
    let late = key(&ws, &main, r"\endrefsection\cite{", "old");
    assert_eq!(
        features::definitions(&ws, &main, late.range.start),
        vec![key(&ws, &default, "ids={", "old")]
    );
    let primary = key(&ws, &default, "@book{", "parent");
    assert!(
        features::rename(&ws, &default, primary.range.start, "newparent")
            .unwrap()
            .is_some()
    );
}
#[test]
fn prospective_alias_completions_and_uncited_real_keys_observe_the_global_cache() {
    let (_dir, ws, main, _default, _bib) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{other}\newrefsection\cite{olBAD}",
        SELECTED,
    );
    let partial = key(&ws, &main, r"\newrefsection\cite{", "olBAD");
    let at = Position::new(partial.range.start.line, partial.range.start.character + 2);
    assert!(features::completion(&ws, &main, at).is_empty());
    let (_dir, ws, main, default, _bib) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{member}\newrefsection\cite{old}",
        "@book{old,title={Uncited collision}} @book{member,title={Member}}",
    );
    let late = key(&ws, &main, r"\newrefsection\cite{", "old");
    assert!(features::definitions(&ws, &main, late.range.start).is_empty());
    let alias = key(&ws, &default, "ids={", "old");
    assert!(features::rename(&ws, &default, alias.range.start, "newalias").is_err());
}
#[test]
fn shared_inputs_and_unsaved_section_order_changes_revoke_and_restore_cache_proofs() {
    let (dir, mut ws, main, default, _bib) = setup(
        r"\input{chapter}\newrefsection[chosen]\cite{other}\newrefsection\input{chapter}",
        SELECTED,
    );
    let chapter = main.join("chapter.tex").unwrap();
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{old}").unwrap();
    ws.open(chapter.clone(), r"\cite{old}".into(), 3);
    let query = key(&ws, &chapter, r"\cite{", "old");
    assert!(features::definitions(&ws, &chapter, query.range.start).is_empty());
    let text = ws.documents[&main].text.source.replace(
        r"\newrefsection\input{chapter}",
        r"\endrefsection\input{chapter}",
    );
    ws.open(main.clone(), text, 6);
    assert_eq!(
        features::definitions(&ws, &chapter, query.range.start),
        vec![key(&ws, &default, "ids={", "old")]
    );
    let primary = key(&ws, &default, "@book{", "parent");
    assert!(
        features::rename(&ws, &default, primary.range.start, "newparent")
            .unwrap()
            .is_some()
    );
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_global_primary_data_differs_from_local_bare_key_lookup_in_a_later_section() {
    for name in ["old", "parent"] {
        let (dir, ws, main, default, _bib) = setup(
            &format!(
                r"\cite{{old}}\newrefsection[chosen]\cite{{other}}\newrefsection\cite{{{name}}}"
            ),
            SELECTED,
        );
        let bbl = compile(dir.path());
        let body = bbl
            .split_once(r"\refsection{2}")
            .unwrap()
            .1
            .split_once(r"\endrefsection")
            .unwrap()
            .0;
        let query = key(&ws, &main, r"\newrefsection\cite{", name);
        if name == "old" {
            assert!(body.contains(r"\field{title}{Selected parent}"), "{bbl}");
            assert!(features::definitions(&ws, &main, query.range.start).is_empty());
        } else {
            assert!(body.contains(r"\field{title}{Default parent}"), "{bbl}");
            assert_eq!(
                features::definitions(&ws, &main, query.range.start),
                vec![key(&ws, &default, "@book{", "parent")]
            );
        }
    }
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_an_uncited_real_key_can_delete_a_cached_alias_before_a_later_section() {
    let (dir, ws, main, default, _bib) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{member}\newrefsection\cite{old}",
        "@book{old,title={Uncited collision}} @book{member,title={Member}}",
    );
    compile_case(dir.path(), true);
    let log = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
    assert!(
        log.contains("Citekey alias 'old' is also a real entry key"),
        "{log}"
    );
    assert!(log.contains("database entry for 'old'"), "{log}");
    let alias = key(&ws, &default, "ids={", "old");
    assert!(features::rename(&ws, &default, alias.range.start, "newalias").is_err());
    let late = key(&ws, &main, r"\newrefsection\cite{", "old");
    assert!(features::definitions(&ws, &main, late.range.start).is_empty());
}

#[test]
fn unused_default_sections_do_not_preload_the_database_or_its_primary_cache() {
    let (_dir, ws, main, _default, _bib) = setup(
        r"\newrefsection[chosen]\cite{other}\newrefsection\cite{parent}\newrefsection[chosen]\cite{other}",
        SELECTED,
    );
    let query = key(
        &ws,
        &main,
        r"\cite{parent}\newrefsection[chosen]\cite{",
        "other",
    );
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    let (_dir, ws, main, default, _bib) = setup(
        r"\newrefsection[chosen]\cite{other}\newrefsection\cite{old}",
        SELECTED,
    );
    let query = key(&ws, &main, r"\newrefsection\cite{", "old");
    assert_eq!(
        features::definitions(&ws, &main, query.range.start),
        vec![key(&ws, &default, "ids={", "old")]
    );
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_only_requested_numeric_sections_parse_their_database_files() {
    let (dir, ws, main, _default, _bib) = setup(
        r"\newrefsection[chosen]\cite{other}\newrefsection\cite{parent}\newrefsection[chosen]\cite{other}",
        SELECTED,
    );
    let bbl = compile(dir.path());
    let body = bbl
        .split_once(r"\refsection{3}")
        .unwrap()
        .1
        .split_once(r"\endrefsection")
        .unwrap()
        .0;
    assert!(body.contains(r"\field{title}{Default parent}"), "{bbl}");
    let query = key(
        &ws,
        &main,
        r"\cite{parent}\newrefsection[chosen]\cite{",
        "other",
    );
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
}

#[test]
fn coexisting_duplicate_primary_providers_do_not_certify_an_alias_owner_by_file_order() {
    let (dir, mut ws, main, _default, _bib) =
        setup(r"\cite{old}\newrefsection[chosen]\cite{other}", SELECTED);
    std::fs::write(
        dir.path().join("a-shared.bib"),
        "@book{parent,title={Competing parent}}",
    )
    .unwrap();
    let text = ws.documents[&main].text.source.replace(
        r"\addsectionbib",
        r"\addglobalbib{a-shared.bib}\addsectionbib",
    );
    ws.open(main.clone(), text, 6);
    let query = key(&ws, &main, r"\cite{", "old");
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    assert!(features::hover(&ws, &main, query.range.start).is_none());
}

#[test]
fn an_unused_coexisting_primary_collision_cannot_become_a_later_owner_proof() {
    let (dir, mut ws, main, default, _bib) = setup(
        r"\newrefsection[chosen,a-shared.bib]\cite{member}\newrefsection\cite{old}",
        SELECTED,
    );
    std::fs::write(
        dir.path().join("a-shared.bib"),
        "@book{parent,title={Competing parent}}",
    )
    .unwrap();
    let text = ws.documents[&main].text.source.replace(
        r"\addsectionbib[label=chosen]{section.bib}",
        r"\addsectionbib[label=chosen]{default.bib}",
    );
    ws.open(default.clone(), format!("{DEFAULT} @book{{member}}"), 7);
    ws.open(main.clone(), text, 6);
    let query = key(&ws, &main, r"\newrefsection\cite{", "old");
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    let alias = key(&ws, &default, "ids={", "old");
    assert!(features::references(&ws, &default, alias.range.start, true).is_empty());
}
