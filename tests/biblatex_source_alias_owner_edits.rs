use lsp_types::*;
use texquill::{language::features, project::workspace::Workspace};
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
fn source_owner_edits_include_the_uncited_raw_primary_and_leave_ids_spelling_alone() {
    for selected in [false, true] {
        for phase in ["", r"\cite{old}"] {
            for call in [
                r"\defbibentryset",
                r"\csuse{defbibentryset}",
                r"\csname defbibentryset\endcsname",
            ] {
                let (_dir, mut ws, main, bib) = setup(
                    &format!(r"{phase}{call}{{parent}}{{member}}\cite{{parent,old}}"),
                    DATABASE,
                    selected,
                );
                let source = key(&ws, &main, &format!("{call}{{"), "parent");
                let primary = key(&ws, &bib, "@book{", "parent");
                let alias = key(&ws, &bib, "ids={", "old");
                let references = features::references(&ws, &main, source.range.start, true);
                assert_eq!(references.len(), 2);
                assert!(!references.contains(&primary));
                assert!(features::prepare_rename(&ws, &main, source.range.start)
                    .unwrap()
                    .is_some());
                assert!(features::rename(&ws, &main, source.range.start, "old").is_err());
                let edit = features::rename(&ws, &main, source.range.start, "newset")
                    .unwrap()
                    .unwrap();
                let changes = edit.changes.as_ref().unwrap();
                assert_eq!(
                    changes.values().map(Vec::len).sum::<usize>(),
                    3,
                    "{changes:?}"
                );
                assert_eq!(
                    changes[&bib],
                    vec![TextEdit {
                        range: primary.range,
                        new_text: "newset".into()
                    }]
                );
                apply(&mut ws, edit);
                let moved = key(&ws, &main, &format!("{call}{{"), "newset");
                let cite = key(&ws, &main, r"\cite{newset,", "old");
                assert_eq!(
                    features::definitions(&ws, &main, cite.range.start),
                    vec![alias]
                );
                assert_eq!(
                    features::references(&ws, &main, moved.range.start, true).len(),
                    2
                );
                assert!(missing(&ws, &main).is_empty());
            }
        }
    }
}
#[test]
fn independent_raw_consumers_and_early_primary_requests_keep_owner_edits_protected() {
    for selected in [false, true] {
        let (_dir, mut ws, main, _bib) = setup(
            r"\defbibentryset{parent}{member}\cite{old}",
            DATABASE,
            selected,
        );
        let source = key(&ws, &main, r"\defbibentryset{", "parent");
        assert!(features::rename(&ws, &main, source.range.start, "newset")
            .unwrap()
            .is_some());
        let other = main.join("other.tex").unwrap();
        ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{refs.bib}\begin{document}\cite{old}\end{document}".into(),3);
        assert!(features::rename(&ws, &main, source.range.start, "newset").is_err());
        ws.documents.remove(&other);
        assert!(features::rename(&ws, &main, source.range.start, "newset")
            .unwrap()
            .is_some());
        let (_dir, ws, main, _bib) = setup(
            r"\cite{parent}\defbibentryset{parent}{member}\cite{old}",
            DATABASE,
            selected,
        );
        let source = key(&ws, &main, r"\defbibentryset{", "parent");
        assert!(features::rename(&ws, &main, source.range.start, "newset").is_err());
    }
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_coupled_ids_owner_edits_preserve_the_set_and_alias_loading_phase() {
    for (selected, phase, relation) in [
        (false, "", ""),
        (false, r"\cite{old}", ""),
        (true, "", ",crossref={old}"),
        (true, r"\cite{old}", ",xref={old}"),
    ] {
        let database = format!("{DATABASE} @book{{child,title={{Child}}{relation}}}");
        let (dir, mut ws, main, _bib) = setup(
            &format!(
                r"{phase}\defbibentryset{{parent}}{{member}}\cite{{parent,old,child}}\printbibliography"
            ),
            &database,
            selected,
        );
        let before = compile(dir.path());
        let source = key(&ws, &main, r"\defbibentryset{", "parent");
        let edit = features::rename(&ws, &main, source.range.start, "newset")
            .unwrap()
            .unwrap();
        apply(&mut ws, edit);
        for ext in ["aux", "bcf", "bbl", "run.xml"] {
            let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
        }
        assert_eq!(compile(dir.path()), before.replace("parent", "newset"));
    }
}
