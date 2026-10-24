use latex_lsp::{features, workspace::Workspace};
use tower_lsp::lsp_types::*;
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
fn distinct_resource_allocations_keep_same_spelling_ids_declarations_and_references_separate() {
    let (_dir, ws, main, default, bib) = setup(BODY);
    let alias = key(&ws, &bib, "ids={", "old");
    let early = key(&ws, &main, r"\cite{", "old");
    let late = key(&ws, &main, r"\cite{parent,", "old");
    assert_eq!(
        features::definitions(&ws, &main, early.range.start),
        vec![key(&ws, &default, "ids={", "old")]
    );
    assert_eq!(
        features::definitions(&ws, &main, late.range.start),
        vec![alias.clone()]
    );
    let set = key(&ws, &main, r"\defbibentryset{", "parent");
    for (query, count) in [(alias, 3), (set, 2)] {
        let references = features::references(&ws, &query.uri, query.range.start, true);
        assert_eq!(references.len(), count, "{references:?}");
        assert!(references.iter().all(|loc| loc.uri != default));
        let edit = features::rename(&ws, &query.uri, query.range.start, "newname");
        if query.uri == bib {
            assert!(edit.unwrap().is_some());
        } else {
            assert!(edit.is_err());
        }
    }
    assert!(missing(&ws, &main).is_empty());
}
#[test]
fn numeric_source_candidates_do_not_merge_disjoint_alias_owners_or_book_metadata() {
    let (_dir, ws, main, _default, bib) =
        setup(r"\cite{old}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{olBAD}");
    let doc = &ws.documents[&main];
    let at = doc.text.source.find("olBAD").unwrap() + 2;
    let items = features::completion(&ws, &main, doc.text.position(at));
    let item = items
        .iter()
        .find(|item| item.label == "old")
        .unwrap_or_else(|| panic!("{items:?}"));
    assert!(item
        .detail
        .as_ref()
        .unwrap()
        .contains("Bibliography alias of parent"));
    assert!(!item.detail.as_ref().unwrap().contains("Default parent"));
    let relation = key(&ws, &bib, "crossref={", "old");
    let items = features::completion(&ws, &bib, relation.range.end);
    assert!(items.iter().any(|item| item.label == "old"), "{items:?}");
}
#[test]
fn shared_alias_occurrences_and_conflicts_in_one_numeric_view_do_not_produce_partial_edits() {
    let (dir, mut ws, main, _default, bib) = setup(
        r"\input{chapter}\newrefsection[chosen]\defbibentryset{parent}{member}\input{chapter}",
    );
    let chapter = main.join("chapter.tex").unwrap();
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{old}").unwrap();
    ws.open(chapter.clone(), r"\cite{old}".into(), 3);
    let alias = key(&ws, &bib, "ids={", "old");
    let query = key(&ws, &chapter, r"\cite{", "old");
    assert!(features::definitions(&ws, &chapter, query.range.start).is_empty());
    assert!(features::references(&ws, &bib, alias.range.start, true).is_empty());
    assert!(features::rename(&ws, &bib, alias.range.start, "newalias").is_err());
    let (_dir, mut ws, main, _default, bib) = setup(BODY);
    let alias = key(&ws, &bib, "ids={", "old");
    assert_eq!(
        features::references(&ws, &bib, alias.range.start, true).len(),
        3
    );
    let text = ws.documents[&bib].text.source.clone();
    ws.open(
        bib.clone(),
        format!("{text} @book{{conflict,ids={{old}}}}"),
        8,
    );
    assert!(features::references(&ws, &bib, alias.range.start, true).is_empty());
    assert!(missing(&ws, &main).is_empty());
    ws.open(bib.clone(), text, 9);
    assert_eq!(
        features::references(&ws, &bib, alias.range.start, true).len(),
        3
    );
}
#[test]
fn contradictory_global_alias_owner_spellings_remain_unproved_across_numeric_sections() {
    let (_dir, mut ws, main, _default, bib) = setup(BODY);
    let query = key(&ws, &main, r"\cite{parent,", "old");
    let text = ws.documents[&bib]
        .text
        .source
        .replace("@book{parent,", "@book{other,");
    ws.open(bib.clone(), text, 8);
    assert!(features::definitions(&ws, &main, query.range.start).is_empty());
    let alias = key(&ws, &bib, "ids={", "old");
    assert!(features::references(&ws, &bib, alias.range.start, true).is_empty());
    assert!(features::completion(&ws, &main, query.range.end).is_empty());
    assert!(missing(&ws, &main).is_empty());
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_numeric_owner_identities_match_the_readonly_graph_and_candidate_loading() {
    let (dir, mut ws, main, _default, bib) = setup(&format!(r"{BODY}\printbibliography"));
    let before = compile(dir.path());
    let default = before
        .split_once(r"\refsection{0}")
        .unwrap()
        .1
        .split_once(r"\endrefsection")
        .unwrap()
        .0;
    let selected = before
        .split_once(r"\refsection{1}")
        .unwrap()
        .1
        .split_once(r"\endrefsection")
        .unwrap()
        .0;
    assert!(default.contains(r"\entry{parent}{book}"));
    assert!(!default.contains(r"\entry{parent}{set}"));
    assert!(selected.contains(r"\entry{parent}{set}"));
    assert!(selected.contains(r"\entry{member}{book}"));
    assert!(
        default.contains(r"\keyalias{old}{parent}") && selected.contains(r"\keyalias{old}{parent}")
    );
    let text = ws.documents[&main]
        .text
        .source
        .replace("parent,old,child", "parent,ol,child");
    std::fs::write(dir.path().join("main.tex"), &text).unwrap();
    ws.open(main.clone(), text, 6);
    let query = key(&ws, &main, r"\cite{parent,", "ol");
    let items = features::completion(&ws, &main, query.range.end);
    let item = items.iter().find(|item| item.label == "old").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    apply(
        &mut ws,
        WorkspaceEdit {
            changes: Some(std::collections::HashMap::from([(
                main.clone(),
                vec![edit.clone()],
            )])),
            ..Default::default()
        },
    );
    for ext in ["aux", "bcf", "bbl", "run.xml"] {
        let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
    }
    assert_eq!(compile(dir.path()), before);
    let source = key(&ws, &main, r"\defbibentryset{", "parent");
    assert!(features::rename(&ws, &main, source.range.start, "newset").is_err());
    let alias = key(&ws, &bib, "ids={", "old");
    assert!(features::rename(&ws, &bib, alias.range.start, "newalias")
        .unwrap()
        .is_some());
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_uncoupled_global_alias_mappings_explain_numeric_owner_edit_protection() {
    let (dir, ws, main, _default, bib) = setup(&format!(r"{BODY}\printbibliography"));
    let before = compile(dir.path());
    let source = key(&ws, &main, r"\defbibentryset{", "parent");
    assert!(features::rename(&ws, &main, source.range.start, "newset").is_err());
    let text = ws.documents[&main]
        .text
        .source
        .replace("{parent}", "{newset}")
        .replace("{parent,old,child}", "{newset,old,child}");
    std::fs::write(dir.path().join("main.tex"), text).unwrap();
    let text = ws.documents[&bib]
        .text
        .source
        .replacen("@book{parent,", "@book{newset,", 1);
    std::fs::write(dir.path().join("section.bib"), text).unwrap();
    for ext in ["aux", "bcf", "bbl", "run.xml"] {
        let _ = std::fs::remove_file(dir.path().join(format!("main.{ext}")));
    }
    let after = compile_checked(dir.path(), true);
    let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
    assert!(
        blg.contains("Entry alias 'old'") && blg.contains("skipping"),
        "{blg}"
    );
    assert_ne!(before, after);
}
