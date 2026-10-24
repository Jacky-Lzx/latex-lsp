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
#[test]
fn intersecting_primary_owners_keep_complete_alias_references_and_safe_alias_edits() {
    for selected in [false, true] {
        for phase in ["", r"\cite{old}", r"\cite{parent}"] {
            for field in ["crossref", "xref"] {
                for call in [r"\cite", r"\csuse{cite}", r"\csname cite\endcsname"] {
                    let database =
                        format!("{DATABASE} @book{{child,title={{Child}},{field}={{old}}}}");
                    let (_dir, mut ws, main, bib) = setup(
                        &format!(
                            r"{phase}\defbibentryset{{parent}}{{member}}{call}{{parent,old,child}}"
                        ),
                        &database,
                        selected,
                    );
                    let alias = key(&ws, &bib, "ids={", "old");
                    let source = key(&ws, &main, r"\defbibentryset{", "parent");
                    let refs = features::references(&ws, &bib, alias.range.start, true);
                    assert_eq!(
                        refs.len(),
                        if phase == r"\cite{old}" { 4 } else { 3 },
                        "{phase} {field} {call}: {refs:?}"
                    );
                    assert!(refs.contains(&alias));
                    for location in &refs {
                        let text = &ws.documents[&location.uri].text;
                        let start = text.offset(location.range.start).unwrap();
                        let end = text.offset(location.range.end).unwrap();
                        assert_eq!(&text.source[start..end], "old");
                    }
                    assert_eq!(
                        features::references(&ws, &main, source.range.start, true).len(),
                        if phase == r"\cite{parent}" { 1 } else { 2 }
                    );
                    assert!(features::rename(&ws, &main, source.range.start, "newset").is_err());
                    assert!(features::rename(&ws, &bib, alias.range.start, "parent").is_err());
                    assert!(features::prepare_rename(&ws, &bib, alias.range.start)
                        .unwrap()
                        .is_some());
                    let edits = features::rename(&ws, &bib, alias.range.start, "newalias")
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        edits
                            .changes
                            .as_ref()
                            .unwrap()
                            .values()
                            .map(Vec::len)
                            .sum::<usize>(),
                        refs.len()
                    );
                    apply(&mut ws, edits);
                    let newalias = key(&ws, &bib, "ids={", "newalias");
                    let doc = &ws.documents[&main];
                    let at =
                        doc.text.source.rfind("parent,newalias,child").unwrap() + "parent,".len();
                    assert_eq!(
                        features::definitions(&ws, &main, doc.text.position(at)),
                        vec![newalias]
                    );
                    assert!(missing(&ws, &main).is_empty());
                }
            }
        }
    }
}
#[test]
fn intersecting_owner_graphs_diagnose_real_missing_keys_without_unlocking_nested_sets() {
    let (_dir, ws, main, bib) = setup(
        r"\defbibentryset{parent}{member}\cite{parent,old,absent}",
        DATABASE,
        true,
    );
    assert_eq!(missing(&ws, &main), ["absent"]);
    let alias = key(&ws, &bib, "ids={", "old");
    assert_eq!(
        features::references(&ws, &bib, alias.range.start, true).len(),
        2
    );
    for body in [
        r"\defbibentryset{parent}{member}\defbibentryset{collected}{old}\cite{collected,old}",
        r"\defbibentryset{collected}{old}\cite{collected,old}\defbibentryset{parent}{member}",
    ] {
        let (_dir, ws, main, bib) = setup(body, DATABASE, true);
        let alias = key(&ws, &bib, "ids={", "old");
        assert!(features::references(&ws, &bib, alias.range.start, true).is_empty());
        assert!(features::rename(&ws, &bib, alias.range.start, "newalias").is_err());
        assert!(missing(&ws, &main).is_empty());
    }
}
#[test]
fn an_unsaved_nested_consumer_revokes_shared_alias_edits_and_close_restores_them() {
    let (_dir, mut ws, main, bib) = setup(
        r"\defbibentryset{parent}{member}\cite{old}",
        DATABASE,
        false,
    );
    let alias = key(&ws, &bib, "ids={", "old");
    assert!(features::rename(&ws, &bib, alias.range.start, "newalias")
        .unwrap()
        .is_some());
    let other = main.join("other.tex").unwrap();
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{refs.bib}\begin{document}\defbibentryset{parent}{member}\defbibentryset{nested}{old}\cite{nested}\end{document}".into(),8);
    assert!(features::rename(&ws, &bib, alias.range.start, "newalias").is_err());
    ws.documents.remove(&other);
    assert_eq!(ws.documents[&main].version, Some(5));
    assert!(features::rename(&ws, &bib, alias.range.start, "newalias")
        .unwrap()
        .is_some());
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_alias_only_edits_preserve_overridden_owner_and_member_materialization() {
    for (selected, phase, field) in [
        (false, "", "crossref"),
        (false, r"\cite{parent}", "xref"),
        (true, r"\cite{old}", "crossref"),
        (true, "", "xref"),
    ] {
        let database = format!("{DATABASE} @book{{child,title={{Child}},{field}={{old}}}}");
        let (dir, mut ws, main, bib) = setup(
            &format!(
                r"{phase}\defbibentryset{{parent}}{{member}}\cite{{parent,old,child}}\printbibliography"
            ),
            &database,
            selected,
        );
        let before = compile(dir.path());
        let alias = key(&ws, &bib, "ids={", "old");
        let source = key(&ws, &main, r"\defbibentryset{", "parent");
        assert!(features::rename(&ws, &main, source.range.start, "newset").is_err());
        let edits = features::rename(&ws, &bib, alias.range.start, "newalias")
            .unwrap()
            .unwrap();
        apply(&mut ws, edits);
        for ext in ["aux", "bcf", "bbl", "run.xml"] {
            let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
        }
        assert_eq!(compile(dir.path()), before.replace("old", "newalias"));
        assert!(missing(&ws, &main).is_empty());
    }
}
