use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;
fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source=format!("% 😀\r\n\\documentclass{{article}}\\usepackage{{biblatex}}\\addbibresource{{default.bib}}\\addsectionbib[label=chosen]{{section.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    for (name, title) in [("default.bib", "Default"), ("section.bib", "Selected")] {
        std::fs::write(dir.path().join(name),format!("@book{{parent,title={{{title} parent}},ids={{old}}}} @book{{member,title={{{title} member}}}} @book{{child,title={{{title} child}},crossref={{old}}}} ")).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let default = main.join("default.bib").unwrap();
    let selected = main.join("section.bib").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main, default, selected)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let start = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert_eq!(&doc.text.source[start..start + name.len()], name);
    Location::new(uri.clone(), doc.text.range(start, start + name.len()))
}
fn compile_checked(dir: &std::path::Path, allow_warnings: bool) -> String {
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
    assert!(
        (allow_warnings || !blg.contains("WARN")) && !blg.contains("ERROR"),
        "{blg}"
    );
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}

fn compile(dir: &std::path::Path) -> String {
    compile_checked(dir, false)
}
fn missing(ws: &Workspace, uri: &Url) -> Vec<String> {
    ws.diagnostics(uri)
        .into_iter()
        .filter(|item| item.code == Some(NumberOrString::String("undefined-citation".into())))
        .map(|item| item.data.unwrap()["key"].as_str().unwrap().to_owned())
        .collect()
}
fn apply(ws: &mut Workspace, edits: WorkspaceEdit) {
    for (uri, mut changes) in edits.changes.unwrap() {
        changes.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let text = ws.documents[&uri]
            .text
            .changed(
                changes
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
        ws.open(uri, text, 9);
    }
}

const BODY: &str =
    r"\cite{old}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,old,child}";
#[test]
fn independent_alias_edits_preserve_both_numeric_owner_identities() {
    for selected in [false, true] {
        let (_dir, mut ws, main, default, bib) = setup(BODY);
        let (edited, untouched) = if selected {
            (&bib, &default)
        } else {
            (&default, &bib)
        };
        let original = ws.documents[untouched].text.source.clone();
        let alias = key(&ws, edited, "ids={", "old");
        assert!(features::prepare_rename(&ws, edited, alias.range.start)
            .unwrap()
            .is_some());
        let change = features::rename(&ws, edited, alias.range.start, "freshalias")
            .unwrap()
            .unwrap();
        let edits = change.changes.as_ref().unwrap();
        assert_eq!(edits.values().map(Vec::len).sum::<usize>(), 3, "{edits:?}");
        assert_eq!(edits[edited].len(), 2);
        assert!(!edits.contains_key(untouched));
        apply(&mut ws, change);
        assert_eq!(ws.documents[untouched].text.source, original);
        let newalias = key(&ws, edited, "ids={", "freshalias");
        assert_eq!(
            features::references(&ws, edited, newalias.range.start, true).len(),
            3
        );
        let source = key(&ws, &main, r"\defbibentryset{", "parent");
        let late = key(
            &ws,
            &main,
            r"\defbibentryset{parent}{member}\cite{",
            "parent",
        );
        assert_eq!(
            features::definitions(&ws, &main, late.range.start),
            vec![source.clone()]
        );
        assert!(features::rename(&ws, &main, source.range.start, "newset").is_err());
        assert!(missing(&ws, &main).is_empty());
    }
}
#[test]
fn new_alias_names_cannot_capture_other_numeric_citations_sets_or_relations() {
    for extra in [
        r"\newrefsection\cite{capture}",
        r"\newrefsection\csuse{cite}{capture}",
        r"\newrefsection\defbibentryset{capture}{member}\cite{capture}",
    ] {
        let (_dir, mut ws, _main, default, bib) = setup(&format!("{BODY}{extra}"));
        // A third processed section also scans uncited relations. Keep this
        // capture fixture independent of a cached IDS request in that library.
        let text = ws.documents[&default]
            .text
            .source
            .replace("crossref={old}", "crossref={parent}");
        ws.open(default, text, 8);
        let alias = key(&ws, &bib, "ids={", "old");
        assert!(features::rename(&ws, &bib, alias.range.start, "capture").is_err());
        assert!(features::rename(&ws, &bib, alias.range.start, "freshalias")
            .unwrap()
            .is_some());
    }
    let (_dir, mut ws, _main, default, bib) = setup(BODY);
    let text = ws.documents[&default].text.source.clone() + " @book{other,related={capture}}";
    ws.open(default, text, 8);
    let alias = key(&ws, &bib, "ids={", "old");
    assert!(features::rename(&ws, &bib, alias.range.start, "capture").is_err());
}
#[test]
fn imported_cached_aliases_and_shared_ambiguous_calls_revoke_alias_edits() {
    let (_dir, mut ws, main, _default, bib) = setup(BODY);
    let alias = key(&ws, &bib, "ids={", "old");
    let expected = features::rename(&ws, &bib, alias.range.start, "freshalias")
        .unwrap()
        .unwrap();
    let text = ws.documents[&bib].text.source.clone();
    ws.open(bib.clone(), text.replace(",ids={old}", ""), 8);
    let early = key(&ws, &main, r"\cite{", "old");
    assert!(features::rename(&ws, &main, early.range.start, "freshalias").is_err());
    ws.open(bib.clone(), text, 9);
    assert_eq!(
        features::rename(&ws, &bib, alias.range.start, "freshalias")
            .unwrap()
            .unwrap(),
        expected
    );
    let chapter = main.join("chapter.tex").unwrap();
    ws.open(chapter.clone(), r"\cite{old}".into(), 3);
    let text = ws.documents[&main]
        .text
        .source
        .replace(r"\cite{old}", r"\input{chapter}")
        .replace(
            r"\cite{parent,old,child}",
            r"\input{chapter}\cite{parent,child}",
        );
    ws.open(main, text, 6);
    assert!(features::references(&ws, &bib, alias.range.start, true).is_empty());
    assert!(features::rename(&ws, &bib, alias.range.start, "freshalias").is_err());
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_independent_alias_edits_keep_global_primary_spellings_and_set_loading() {
    for selected in [false, true] {
        let (dir, mut ws, main, default, bib) = setup(&format!(r"{BODY}\printbibliography"));
        let before = compile(dir.path());
        let edited = if selected { &bib } else { &default };
        let alias = key(&ws, edited, "ids={", "old");
        let edit = features::rename(&ws, edited, alias.range.start, "freshalias")
            .unwrap()
            .unwrap();
        apply(&mut ws, edit);
        for ext in ["aux", "bcf", "bbl", "run.xml"] {
            let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
        }
        let after = compile(dir.path());
        let old_line = r"\keyalias{freshalias}{parent}";
        assert_eq!(after.replace(old_line, r"\keyalias{old}{parent}"), before);
        let source = key(&ws, &main, r"\defbibentryset{", "parent");
        assert!(features::rename(&ws, &main, source.range.start, "newset").is_err());
    }
}
