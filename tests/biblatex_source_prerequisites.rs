use latex_lsp::{features, workspace::Workspace};
use tower_lsp::lsp_types::*;
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
