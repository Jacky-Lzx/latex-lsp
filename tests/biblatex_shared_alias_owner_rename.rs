use latex_lsp::{features, workspace::Workspace};
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
const DEFAULT: &str = "@book{parent,title={Default parent},ids={old}} @book{child,title={Default child},crossref={parent}}";
const SELECTED: &str = "@book{parent,title={Selected parent},ids={old}} @book{child,title={Selected child},crossref={parent}}";

const BODY: &str = r"\cite{parent,old,child}\printbibliography\newrefsection[chosen]\cite{parent,old,child}\printbibliography";
#[test]
fn shared_ids_owners_move_as_one_complete_primary_spelling() {
    for selected_query in [false, true] {
        let (_dir, ws, main, default, selected) = setup(BODY, SELECTED);
        let primary = key(
            &ws,
            if selected_query { &selected } else { &default },
            "@book{",
            "parent",
        );
        assert_eq!(
            features::references(&ws, &primary.uri, primary.range.start, true).len(),
            3
        );
        assert!(
            features::prepare_rename(&ws, &primary.uri, primary.range.start)
                .unwrap()
                .is_some()
        );
        let edits = features::rename(&ws, &primary.uri, primary.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits.len(), 3);
        assert_eq!(edits[&main].len(), 2);
        assert_eq!(edits[&default].len(), 2);
        assert_eq!(edits[&selected].len(), 2);
        for (uri, edits) in edits {
            for edit in edits {
                assert_eq!(
                    &ws.documents[&uri].text.source[ws.documents[&uri]
                        .text
                        .offset(edit.range.start)
                        .unwrap()
                        ..ws.documents[&uri].text.offset(edit.range.end).unwrap()],
                    "parent"
                );
                assert_eq!(edit.new_text, "fresh");
            }
        }
    }
}
#[test]
fn shared_owner_names_cannot_capture_any_coupled_resource_consumer() {
    let (_dir, mut ws, main, default, selected) = setup(BODY, SELECTED);
    let primary = key(&ws, &selected, "@book{", "parent");
    for tail in [r"\cite{fresh}", r"\defcitealias{fresh}{Name}"] {
        let source = ws.documents[&main].text.source.replace(
            r"\printbibliography\newrefsection",
            &format!(r"{tail}\printbibliography\newrefsection"),
        );
        ws.open(main.clone(), source, 6);
        assert!(features::rename(&ws, &selected, primary.range.start, "fresh").is_err());
    }
    ws.open(
        default,
        format!("{DEFAULT} @book{{fresh,title={{Existing}}}}"),
        7,
    );
    assert!(features::rename(&ws, &selected, primary.range.start, "fresh").is_err());
}
#[test]
fn source_owners_and_ambiguous_shared_occurrences_keep_their_edit_protection() {
    for body in [
        r"\cite{old}\newrefsection[chosen]\defbibentryset{parent}{child}\cite{parent,old}",
        r"\input{chapter}\newrefsection[chosen]\input{chapter}",
    ] {
        let (dir, mut ws, main, _default, selected) = setup(body, SELECTED);
        let chapter = main.join("chapter.tex").unwrap();
        std::fs::write(dir.path().join("chapter.tex"), r"\cite{parent}").unwrap();
        ws.open(chapter, r"\cite{parent}".into(), 3);
        let primary = key(&ws, &selected, "@book{", "parent");
        assert!(features::prepare_rename(&ws, &selected, primary.range.start).is_err());
        assert!(features::rename(&ws, &selected, primary.range.start, "fresh").is_err());
    }
}
#[test]
fn conflicting_unsaved_consumers_revoke_the_entire_owner_edit() {
    let (_dir, mut ws, main, default, selected) = setup(BODY, SELECTED);
    let primary = key(&ws, &selected, "@book{", "parent");
    assert!(
        features::rename(&ws, &selected, primary.range.start, "fresh")
            .unwrap()
            .is_some()
    );
    let other = main.join("other.tex").unwrap();
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{default.bib}\begin{document}\iftrue\cite{parent}\fi\end{document}".into(),9);
    assert!(features::rename(&ws, &selected, primary.range.start, "fresh").is_err());
    ws.documents.remove(&other);
    assert!(
        features::rename(&ws, &selected, primary.range.start, "fresh")
            .unwrap()
            .is_some()
    );
    let alias = key(&ws, &default, "ids={", "old");
    assert_eq!(
        features::references(&ws, &default, alias.range.start, true).len(),
        2
    );
}
#[test]
fn all_transitive_ids_owners_join_the_edit_without_unrelated_roots() {
    let (dir, mut ws, main, default, selected) = setup(
        r"\cite{parent,old}\newrefsection[chosen]\cite{parent,old,other}\newrefsection[third]\cite{parent,other}",
        SELECTED,
    );
    ws.open(
        selected.clone(),
        SELECTED.replace("ids={old}", "ids={old,other}"),
        8,
    );
    let third = main.join("third.bib").unwrap();
    let library = "@book{parent,title={Third},ids={other}}";
    std::fs::write(dir.path().join("third.bib"), library).unwrap();
    ws.open(third.clone(), library.into(), 9);
    let source = ws.documents[&main].text.source.replace(
        r"\begin{document}",
        r"\addsectionbib[label=third]{third.bib}\begin{document}",
    );
    ws.open(main.clone(), source, 6);
    let unrelated = main.join("unrelated.tex").unwrap();
    let unrelated_bib = main.join("unrelated.bib").unwrap();
    std::fs::write(dir.path().join("unrelated.bib"), library).unwrap();
    ws.open(unrelated.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{unrelated.bib}\begin{document}\cite{parent,other}\end{document}".into(),4);
    let primary = key(&ws, &default, "@book{", "parent");
    let edits = features::rename(&ws, &default, primary.range.start, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits.len(), 4);
    assert_eq!(edits[&main].len(), 3);
    assert_eq!(edits[&third].len(), 1);
    assert!(!edits.contains_key(&unrelated));
    assert!(!edits.contains_key(&unrelated_bib));
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_coupled_raw_owners_preserve_every_numeric_bibliography() {
    let (dir, ws, _main, _default, selected) = setup(BODY, SELECTED);
    let before = compile(dir.path());
    let primary = key(&ws, &selected, "@book{", "parent");
    let edits = features::rename(&ws, &selected, primary.range.start, "fresh")
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
    let after = compile(dir.path());
    assert_eq!(before, after.replace("fresh", "parent"));
}
