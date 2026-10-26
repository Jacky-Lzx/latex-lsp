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

#[test]
fn nonloading_keys_keep_complete_numeric_references_and_edits() {
    for call in [
        r"\defcitealias{parent}{Name}",
        r"\entrydata{parent}{}",
        r"\addtocategory{group}{parent}",
        r"\csuse{entrydata}{parent}{}",
        r"\csname defcitealias\endcsname{parent}{Name}",
    ] {
        let (_dir, ws, main, default, selected) = setup(
            &format!(r"\cite{{parent}}\newrefsection[chosen]\cite{{parent}}{call}"),
            SELECTED,
        );
        let declaration = key(&ws, &selected, "@book{", "parent");
        let references = features::references(&ws, &selected, declaration.range.start, true);
        assert_eq!(references.len(), 3, "{call}: {references:?}");
        assert!(references.iter().all(|reference| reference.uri != default));
        let edits = features::rename(&ws, &selected, declaration.range.start, "newparent")
            .unwrap()
            .unwrap();
        let changes = edits.changes.unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[&main].len(), 2);
        assert_eq!(changes[&selected].len(), 1);
        assert!(!changes.contains_key(&default));
    }
}
#[test]
fn data_key_consumers_do_not_preload_a_numeric_database() {
    for call in [
        r"\defcitealias{parent}{Name}",
        r"\entrydata{parent}{}",
        r"\addtocategory{group}{parent}",
    ] {
        let (_dir, ws, main, default, _selected) = setup(
            &format!(r"{call}\newrefsection[chosen]\cite{{other}}\newrefsection\cite{{old}}"),
            SELECTED,
        );
        let late = key(&ws, &main, r"\newrefsection\cite{", "old");
        assert_eq!(
            features::definitions(&ws, &main, late.range.start),
            vec![key(&ws, &default, "ids={", "old")],
            "{call}"
        );
    }
}
#[test]
fn new_primary_names_cannot_capture_nonloading_keys_in_their_resource_view() {
    for call in [
        r"\defcitealias{capture}{Name}",
        r"\entrydata{capture}{}",
        r"\addtocategory{group}{capture}",
    ] {
        let (_dir, ws, _main, _default, selected) = setup(
            &format!(r"\cite{{parent}}\newrefsection[chosen]\cite{{parent}}{call}"),
            SELECTED,
        );
        let declaration = key(&ws, &selected, "@book{", "parent");
        assert!(
            features::rename(&ws, &selected, declaration.range.start, "capture").is_err(),
            "{call}"
        );
        assert!(
            features::rename(&ws, &selected, declaration.range.start, "fresh")
                .unwrap()
                .is_some(),
            "{call}"
        );
    }
}
#[test]
fn source_sets_and_executable_conditionals_keep_their_query_protection() {
    for call in [
        r"\ifentrycategory{parent}{group}{\newrefsection}{}",
        r"\ifentryexists{parent}{}{}",
        r"\ifentryinbib{parent}{}{\cite{member}}",
        r"\defbibentryset{parent}{member}\entrydata{parent}{}",
    ] {
        let (_dir, ws, _main, _default, selected) = setup(
            &format!(r"\newrefsection[chosen]\cite{{parent}}{call}"),
            SELECTED,
        );
        let declaration = key(&ws, &selected, "@book{", "parent");
        assert!(features::references(&ws, &selected, declaration.range.start, true).is_empty());
        assert!(
            features::rename(&ws, &selected, declaration.range.start, "fresh").is_err(),
            "{call}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_nonloading_keys_preserve_the_first_database_load_and_complete_renames() {
    let (dir, mut ws, main, default, _selected) = setup(
        r"\defcitealias{parent}{Named}\addtocategory{group}{parent}\newrefsection[chosen]\cite{other}\newrefsection\cite{old}\printbibliography",
        SELECTED,
    );
    let source = ws.documents[&main]
        .text
        .source
        .replace(
            r"\usepackage{biblatex}",
            r"\usepackage[natbib=true]{biblatex}",
        )
        .replace(
            r"\begin{document}",
            r"\DeclareBibliographyCategory{group}\begin{document}",
        );
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    ws.open(main.clone(), source, 6);
    let before = compile(dir.path());
    let last = before
        .split_once(r"\refsection{2}")
        .unwrap()
        .1
        .split_once(r"\endrefsection")
        .unwrap()
        .0;
    assert!(last.contains(r"\field{title}{Default parent}"), "{last}");
    assert!(!last.contains("Selected parent"), "{last}");
    let primary = key(&ws, &default, "@book{", "parent");
    let edits = features::rename(&ws, &default, primary.range.start, "freshparent")
        .unwrap()
        .unwrap();
    let changes = edits.changes.unwrap();
    assert_eq!(changes[&main].len(), 2);
    assert_eq!(changes[&default].len(), 1);
    for (uri, mut edits) in changes {
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
    let after = compile(dir.path());
    assert_eq!(before, after.replace("freshparent", "parent"));
}

#[test]
fn passive_alias_queries_do_not_claim_an_overwritten_cached_owner() {
    let (_dir, ws, main, default, _selected) = setup(
        r"\cite{old}\newrefsection[chosen]\cite{other}\newrefsection\cite{parent}\defcitealias{old}{Name}",
        SELECTED,
    );
    let query = key(&ws, &main, r"\defcitealias{", "old");
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    assert!(features::hover(&ws, &main, query.range.start).is_none());
    let primary = key(&ws, &default, "@book{", "parent");
    assert!(features::rename(&ws, &default, primary.range.start, "fresh").is_err());
}
