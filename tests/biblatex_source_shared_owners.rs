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
const DEFAULT: &str = "@book{parent,title={Default parent},ids={old}} @book{child,title={Default child},crossref={parent}} @book{member,title={Member}}";
const SELECTED: &str = "@book{parent,title={Selected parent},ids={old}} @book{child,title={Selected child},crossref={parent}} @book{member,title={Member}}";

#[test]
fn unrelated_source_sets_keep_complete_shared_raw_owner_edits() {
    for body in [
        r"\defbibentryset{collection}{member}\cite{parent,old,child,collection}\newrefsection[chosen]\cite{parent,old,child}",
        r"\defbibentryset{collection}{member}\cite{parent,old,child,collection}\newrefsection[chosen]\defbibentryset{selectedset}{member}\cite{parent,old,child,selectedset}",
        r"\defbibentryset{collection}{parent}\cite{collection,old,child}\newrefsection[chosen]\cite{parent,old,child}",
    ] {
        let (_dir, ws, main, default, selected) = setup(body, SELECTED);
        let primary = key(&ws, &selected, "@book{", "parent");
        let edits = features::rename(&ws, &selected, primary.range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(edits.len(), 3, "{body}: {edits:?}");
        assert_eq!(edits[&main].len(), 2);
        assert_eq!(edits[&default].len(), 2);
        assert_eq!(edits[&selected].len(), 2);
        for (uri, edits) in edits {
            for edit in edits {
                let text = &ws.documents[&uri].text;
                assert_eq!(
                    &text.source[text.offset(edit.range.start).unwrap()
                        ..text.offset(edit.range.end).unwrap()],
                    "parent"
                );
                assert_eq!(edit.new_text, "fresh");
            }
        }
    }
}
#[test]
fn wildcard_loads_keep_unrelated_source_sets_and_shared_raw_owners_separate() {
    let (_dir, ws, _main, default, selected) = setup(
        r"\defbibentryset{collection}{member}\nocite{*}\newrefsection[chosen]\nocite{*}",
        SELECTED,
    );
    let primary = key(&ws, &default, "@book{", "parent");
    let edits = features::rename(&ws, &default, primary.range.start, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[&default].len(), 2);
    assert_eq!(edits[&selected].len(), 2);
}

#[test]
fn all_transitive_shared_owners_join_edits_beside_unrelated_sets() {
    let (dir, mut ws, main, default, selected) = setup(
        r"\defbibentryset{collection}{member}\cite{collection,parent,old}\newrefsection[chosen]\cite{parent,old,other}\newrefsection[third]\cite{parent,other}",
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
    let primary = key(&ws, &default, "@book{", "parent");
    let edits = features::rename(&ws, &default, primary.range.start, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits.len(), 4);
    assert_eq!(edits[&main].len(), 3);
    assert_eq!(edits[&third].len(), 1);
    assert_eq!(edits[&default].len(), 2);
    assert_eq!(edits[&selected].len(), 2);
}
#[test]
fn source_shadow_alias_intersections_and_new_name_capture_stay_protected() {
    for body in [
        r"\defbibentryset{collection}{member}\cite{parent,old}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,old}",
        r"\defbibentryset{old}{member}\cite{parent,old}\newrefsection[chosen]\cite{parent,old}",
        r"\defbibentryset{fresh}{member}\cite{parent,old}\newrefsection[chosen]\cite{parent,old}",
        r"\defbibentryset{collection}{member}\cite{parent,old}\newrefsection[chosen]\cite{parent,old,fresh}",
    ] {
        let (_dir, ws, _main, default, _selected) = setup(body, SELECTED);
        let primary = key(&ws, &default, "@book{", "parent");
        assert!(
            features::rename(&ws, &default, primary.range.start, "fresh").is_err(),
            "{body}"
        );
    }
}
#[test]
fn unsaved_external_consumers_and_library_captures_revoke_the_entire_edit() {
    let (_dir, mut ws, main, default, selected) = setup(
        r"\defbibentryset{collection}{member}\cite{parent,old,collection}\newrefsection[chosen]\cite{parent,old}",
        SELECTED,
    );
    let primary = key(&ws, &selected, "@book{", "parent");
    let rename = |ws: &Workspace| features::rename(ws, &selected, primary.range.start, "fresh");
    assert!(rename(&ws).unwrap().is_some());
    let other = main.join("other.tex").unwrap();
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{default.bib}\begin{document}\iftrue\cite{parent}\fi\end{document}".into(),7);
    assert!(rename(&ws).is_err());
    ws.documents.remove(&other);
    assert!(rename(&ws).unwrap().is_some());
    ws.open(
        default.clone(),
        format!("{DEFAULT} @book{{fresh,title={{Taken}}}}"),
        8,
    );
    assert!(rename(&ws).is_err());
    ws.open(default, DEFAULT.into(), 9);
    assert!(rename(&ws).unwrap().is_some());
    ws.open(
        selected.clone(),
        SELECTED.replace("@book{parent,", "@book{different,"),
        10,
    );
    assert!(rename(&ws).is_err());
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_shared_raw_owner_edits_preserve_unrelated_source_sets_and_lists() {
    for body in [
        r"\defbibentryset{collection}{member}\cite{parent,old,child,collection}\printbibliography\newrefsection[chosen]\cite{parent,old,child}\printbibliography",
        r"\defbibentryset{collection}{member}\cite{parent,old,child,collection}\printbibliography\newrefsection[chosen]\defbibentryset{selectedset}{member}\cite{parent,old,child,selectedset}\printbibliography",
        r"\defbibentryset{collection}{member}\nocite{*}\printbibliography\newrefsection[chosen]\nocite{*}\printbibliography",
        r"\defbibentryset{collection}{parent}\cite{collection,old,child}\printbibliography\newrefsection[chosen]\cite{parent,old,child}\printbibliography",
    ] {
        let (dir, ws, _main, default, _selected) = setup(body, SELECTED);
        let before = compile(dir.path());
        let primary = key(&ws, &default, "@book{", "parent");
        let edits = features::rename(&ws, &default, primary.range.start, "fresh")
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
