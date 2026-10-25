use latex_lsp::{features, workspace::Workspace};
use lsp_types::*;
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

fn marked(
    body: &str,
    database: &str,
    selected: bool,
) -> (tempfile::TempDir, Workspace, Url, Url, Position) {
    let (dir, mut ws, main, bib) = setup(&body.replace('¦', ""), database, selected);
    let source = ws.documents[&main].text.source.clone();
    let head = source.find(r"\begin{document}").unwrap()
        + r"\begin{document}".len()
        + if selected {
            r"\newrefsection[chosen]".len()
        } else {
            0
        };
    let at = ws.documents[&main]
        .text
        .position(head + body.find('¦').unwrap());
    ws.open(main.clone(), source, 5);
    (dir, ws, main, bib, at)
}
fn candidate(ws: &Workspace, uri: &Url, at: Position, name: &str) -> CompletionItem {
    let items = features::completion(ws, uri, at);
    items
        .iter()
        .find(|item| item.label == name)
        .unwrap_or_else(|| panic!("{name}: {items:?}"))
        .clone()
}
#[test]
fn overridden_owner_aliases_are_citation_candidates_without_stale_book_metadata() {
    for selected in [false, true] {
        for body in [
            r"\defbibentryset{parent}{member}\cite{ol¦BAD}",
            r"\cite{ol¦BAD}\defbibentryset{parent}{member}\cite{parent}",
            r"\cite{parent}\defbibentryset{parent}{member}\csuse{cite}{ol¦BAD}",
        ] {
            let (_dir, ws, main, _bib, at) = marked(body, DATABASE, selected);
            let item = candidate(&ws, &main, at, "old");
            let detail = item.detail.unwrap();
            assert!(detail.contains("Bibliography alias of parent"), "{detail}");
            assert!(!detail.contains("Raw parent"));
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!()
            };
            let doc = &ws.documents[&main];
            let start = doc.text.offset(edit.range.start).unwrap();
            let end = doc.text.offset(edit.range.end).unwrap();
            assert_eq!(&doc.text.source[start..end], "olBAD");
            assert_eq!(edit.new_text, "old");
        }
    }
}
#[test]
fn prospective_alias_members_and_incompatible_relation_candidates_remain_excluded() {
    for selected in [false, true] {
        for body in [
            r"\defbibentryset{parent}{member}\defbibentryset{other}{¦}",
            r"\defbibentryset{other}{¦}\defbibentryset{parent}{member}",
        ] {
            let (_dir, ws, main, _bib, at) = marked(body, DATABASE, selected);
            let items = features::completion(&ws, &main, at);
            assert!(items.iter().any(|item| item.label == "member"), "{items:?}");
            assert!(!items.iter().any(|item| item.label == "old"), "{items:?}");
        }
        for field in ["crossref", "xref", "xdata", "related", "entryset"] {
            let database=format!("{DATABASE} @XDATA{{data,publisher={{Pub}},ids={{data-old}}}} @book{{child,{field}={{}}}}");
            let (_dir, ws, _main, bib) = setup(
                r"\defbibentryset{parent}{member}\cite{parent}",
                &database,
                selected,
            );
            let at = key(&ws, &bib, &format!("{field}={{"), "").range.start;
            let items = features::completion(&ws, &bib, at);
            assert_eq!(
                items.iter().any(|item| item.label == "old"),
                matches!(field, "crossref" | "xref"),
                "{selected} {field}: {items:?}"
            );
            if field == "xdata" {
                assert!(
                    items.iter().any(|item| item.label == "data-old"),
                    "{items:?}"
                );
            }
        }
    }
}
#[test]
fn new_nested_alias_uses_revoke_candidates_and_unsaved_restoration_recovers() {
    let (_dir, mut ws, main, bib, at) = marked(
        r"\defbibentryset{parent}{member}\cite{ol¦}",
        DATABASE,
        false,
    );
    assert_eq!(candidate(&ws, &main, at, "old").label, "old");
    let original = ws.documents[&main].text.source.clone();
    ws.open(
        main.clone(),
        original.replace(
            r"\end{document}",
            r"\defbibentryset{nested}{old}\end{document}",
        ),
        6,
    );
    assert!(features::completion(&ws, &main, at).is_empty());
    ws.open(main.clone(), original, 7);
    assert_eq!(candidate(&ws, &main, at, "old").label, "old");
    ws.open(bib, "@XDATA{parent,ids={old}} @book{member}".into(), 8);
    assert!(features::completion(&ws, &main, at).is_empty());
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_accepted_alias_candidates_materialize_the_correct_source_owner() {
    for (selected, body) in [
        (
            false,
            r"\cite{ol¦}\defbibentryset{parent}{member}\cite{parent}",
        ),
        (true, r"\defbibentryset{parent}{member}\cite{ol¦,parent}"),
    ] {
        let (dir, mut ws, main, _bib, at) = marked(body, DATABASE, selected);
        let item = candidate(&ws, &main, at, "old");
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        let text = ws.documents[&main]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text,
            }])
            .unwrap()
            .source;
        std::fs::write(dir.path().join("main.tex"), &text).unwrap();
        ws.open(main.clone(), text, 6);
        let bbl = compile(dir.path());
        assert!(bbl.contains(r"\keyalias{old}{parent}"), "{bbl}");
        assert!(bbl.contains(r"\entry{parent}{set}"), "{bbl}");
        assert!(bbl.contains(r"\entry{member}{book}"), "{bbl}");
    }
}
