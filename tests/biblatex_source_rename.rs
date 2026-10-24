use latex_lsp::{features, workspace::Workspace};
use tower_lsp::lsp_types::*;
fn setup(preamble: &str, body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}{preamble}\begin{{document}}{body}\end{{document}}"
    );
    for(name,text)in[("main.tex",source.as_str()),("default.bib","@book{parent,title={Default parent}}\n@book{member,title={Default member}}\n@inbook{child,crossref={parent}}"),("section.bib","@book{parent,title={Selected parent}}\n@book{member,title={Selected member}}\n@inbook{child,crossref={parent}}")]{std::fs::write(dir.path().join(name),text).unwrap();}
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main)
}
fn key(ws: &Workspace, uri: &Url, prefix: &str, name: &str) -> Location {
    let doc = &ws.documents[uri];
    let start = doc.text.source.find(prefix).unwrap() + prefix.len();
    assert!(doc.text.source[start..].starts_with(name));
    Location::new(uri.clone(), doc.text.range(start, start + name.len()))
}
fn sorted(mut items: Vec<Location>) -> Vec<Location> {
    items.sort_by_key(|item| {
        (
            item.uri.to_string(),
            item.range.start.line,
            item.range.start.character,
            item.range.end.line,
            item.range.end.character,
        )
    });
    items
}
fn check(ws: &Workspace, query: &Location, declaration: &Location, expected: &[Location]) {
    assert_eq!(
        sorted(features::references(
            ws,
            &query.uri,
            query.range.start,
            true
        )),
        sorted(expected.to_vec())
    );
    assert_eq!(
        sorted(features::references(
            ws,
            &query.uri,
            query.range.start,
            false
        )),
        sorted(
            expected
                .iter()
                .filter(|item| *item != declaration)
                .cloned()
                .collect()
        )
    );
}
fn calls(ws: &Workspace, uri: &Url, name: &str) -> Vec<Location> {
    let doc = &ws.documents[uri];
    let marker = format!(r"\cite{{{name}}}");
    doc.text
        .source
        .match_indices(&marker)
        .map(|(at, _)| Location::new(uri.clone(), doc.text.range(at + 6, at + 6 + name.len())))
        .collect()
}
fn edits(edit: WorkspaceEdit) -> std::collections::BTreeMap<Url, Vec<TextEdit>> {
    if let Some(changes) = edit.changes {
        return changes.into_iter().collect();
    }
    let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
        panic!("missing edits")
    };
    changes
        .into_iter()
        .map(|edit| {
            (
                edit.text_document.uri,
                edit.edits
                    .into_iter()
                    .map(|edit| match edit {
                        OneOf::Left(edit) => edit,
                        _ => panic!(),
                    })
                    .collect(),
            )
        })
        .collect()
}
fn apply(ws: &mut Workspace, edit: WorkspaceEdit) {
    for (uri, mut edits) in edits(edit) {
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
        ws.open(uri, source, 20);
    }
}
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
    assert!(
        !blg.contains("ERROR") && !blg.contains("I didn't find"),
        "{blg}"
    );
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}
fn section(bbl: &str, index: u32) -> &str {
    bbl.split_once(&format!("\\refsection{{{index}}}"))
        .unwrap()
        .1
        .split_once(r"\endrefsection")
        .unwrap()
        .0
}
fn entries(section: &str) -> std::collections::BTreeMap<String, String> {
    section
        .split(r"\entry{")
        .skip(1)
        .map(|entry| {
            let (name, rest) = entry.split_once('}').unwrap();
            (name.into(), rest.split_once(r"\endentry").unwrap().0.into())
        })
        .collect()
}
fn clean(body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let (dir, mut ws, main) = setup("", body);
    let bib = main.join("section.bib").unwrap();
    let text = ws.documents[&bib]
        .text
        .source
        .replace(",crossref={parent}", ",title={Standalone child}");
    std::fs::write(dir.path().join("section.bib"), &text).unwrap();
    ws.open(bib, text, 8);
    (dir, ws, main)
}
#[test]
fn independent_source_and_member_renames_preserve_other_sections_and_exact_ranges() {
    for member in [false, true] {
        let (_dir, mut ws, main) = clean(
            r"\cite{parent}\cite{member}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}\cite{member}",
        );
        let bib = main.join("section.bib").unwrap();
        let default = main.join("default.bib").unwrap();
        let before = ws.documents[&default].text.source.clone();
        let query = if member {
            key(&ws, &main, r"\defbibentryset{parent}{", "member")
        } else {
            key(&ws, &main, r"\defbibentryset{", "parent")
        };
        let old = if member { "member" } else { "parent" };
        let new = if member { "newmember" } else { "newset" };
        let prepared = features::prepare_rename(&ws, &query.uri, query.range.start)
            .unwrap()
            .unwrap();
        assert_eq!(
            prepared,
            PrepareRenameResponse::RangeWithPlaceholder {
                range: query.range,
                placeholder: old.into()
            }
        );
        let expected = features::references(&ws, &query.uri, query.range.start, true);
        assert_eq!(expected.len(), if member { 3 } else { 2 });
        let edit = features::rename(&ws, &query.uri, query.range.start, new)
            .unwrap()
            .unwrap();
        let actual = edits(edit.clone());
        let mut ranges = Vec::new();
        for (uri, items) in &actual {
            for item in items {
                assert_eq!(item.new_text, new);
                ranges.push(Location::new(uri.clone(), item.range));
            }
        }
        assert_eq!(sorted(ranges), sorted(expected));
        assert!(!actual.contains_key(&default));
        assert_eq!(actual.contains_key(&bib), member);
        apply(&mut ws, edit);
        assert_eq!(ws.documents[&default].text.source, before);
        let newquery = key(
            &ws,
            &main,
            if member {
                r"\defbibentryset{parent}{"
            } else {
                r"\defbibentryset{"
            },
            new,
        );
        assert_eq!(
            features::references(&ws, &main, newquery.range.start, true).len(),
            if member { 3 } else { 2 }
        );
    }
}
#[test]
fn source_renames_reject_raw_database_relation_dependencies_and_hidden_parent_edits() {
    for field in ["crossref", "xref", "xdata", "related", "entryset"] {
        let (_dir, mut ws, main) =
            clean(r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}");
        let bib = main.join("section.bib").unwrap();
        ws.open(
            bib.clone(),
            format!("@book{{parent}} @book{{member}} @book{{child,{field}={{parent}}}}"),
            8,
        );
        let source = key(&ws, &main, r"\defbibentryset{", "parent");
        let book = key(&ws, &bib, "@book{", "parent");
        for query in [source, book] {
            assert!(
                features::prepare_rename(&ws, &query.uri, query.range.start).is_err(),
                "{field}"
            );
            assert!(
                features::rename(&ws, &query.uri, query.range.start, "new").is_err(),
                "{field}"
            );
        }
    }
}
#[test]
fn unused_early_book_locked_source_declarations_cannot_materialize_an_additional_set() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{member}\cite{parent}",
    );
    let source = key(&ws, &main, r"\defbibentryset{", "parent");
    check(&ws, &source, &source, std::slice::from_ref(&source));
    assert!(features::prepare_rename(&ws, &main, source.range.start).is_err());
    assert!(features::rename(&ws, &main, source.range.start, "unused").is_err());
}
#[test]
fn new_source_names_cannot_capture_requests_before_or_after_the_declaration() {
    for extra in [
        r"\cite{new}",
        r"\nocite{new}",
        r"\defbibentryset{other}{new}",
    ] {
        for before in [false, true] {
            let fragment = r"\defbibentryset{parent}{member}\cite{parent}";
            let body = if before {
                format!(r"\newrefsection[chosen]{extra}{fragment}")
            } else {
                format!(r"\newrefsection[chosen]{fragment}{extra}")
            };
            let (_dir, ws, main) = clean(&body);
            let doc = &ws.documents[&main];
            let start = doc.text.source.find(r"\defbibentryset{parent}").unwrap()
                + r"\defbibentryset{".len();
            let query = Location::new(main.clone(), doc.text.range(start, start + 6));
            assert!(
                features::rename(&ws, &main, query.range.start, "new").is_err(),
                "{body}"
            );
        }
    }
}
#[test]
fn source_and_database_targets_cannot_collide_with_unused_source_or_primary_names() {
    for body in [
        r"\newrefsection[chosen]\cite{taken}\defbibentryset{taken}{member}\defbibentryset{parent}{member}\cite{parent}",
        r"\newrefsection[chosen]\defbibentryset{taken}{member}\defbibentryset{parent}{member}\cite{parent}",
    ] {
        let (_dir, ws, main) = clean(body);
        let source = key(
            &ws,
            &main,
            r"\defbibentryset{taken}{member}\defbibentryset{",
            "parent",
        );
        assert!(features::rename(&ws, &main, source.range.start, "taken").is_err());
        assert!(features::rename(&ws, &main, source.range.start, "member").is_err());
        let member = key(&ws, &main, r"\defbibentryset{taken}{", "member");
        assert!(features::rename(&ws, &main, member.range.start, "taken").is_err());
    }
}
#[test]
fn new_names_in_disjoint_numeric_sections_remain_available() {
    let (_dir, mut ws, main) = clean(
        r"\defbibentryset{newset}{member}\cite{newset}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
    );
    let source = key(
        &ws,
        &main,
        r"\newrefsection[chosen]\defbibentryset{",
        "parent",
    );
    let edit = features::rename(&ws, &main, source.range.start, "newset")
        .unwrap()
        .unwrap();
    assert_eq!(edits(edit.clone())[&main].len(), 2);
    apply(&mut ws, edit);
    let citations = calls(&ws, &main, "newset");
    assert_eq!(citations.len(), 2);
    let first = key(&ws, &main, r"\defbibentryset{", "newset");
    let second = key(
        &ws,
        &main,
        r"\newrefsection[chosen]\defbibentryset{",
        "newset",
    );
    assert_eq!(
        features::definitions(&ws, &main, citations[0].range.start),
        vec![first.clone()]
    );
    assert_eq!(
        features::definitions(&ws, &main, citations[1].range.start),
        vec![second.clone()]
    );
    for (declaration, citation) in [
        (first, citations[0].clone()),
        (second, citations[1].clone()),
    ] {
        check(
            &ws,
            &declaration,
            &declaration,
            &[declaration.clone(), citation],
        );
        assert!(
            features::prepare_rename(&ws, &main, declaration.range.start)
                .unwrap()
                .is_some()
        );
    }
}
#[test]
fn repeated_static_named_declarations_keep_unicode_ranges_and_document_versions() {
    for declare in [
        r"\defbibentryset",
        r"\csuse{defbibentryset}",
        r"\csname defbibentryset\endcsname",
    ] {
        let (dir, mut ws, main) = clean(
            r"\newrefsection[chosen]\input{sets}\cite{parent}\newrefsection[chosen]\input{sets}\cite{parent}",
        );
        let sets = main.join("sets.tex").unwrap();
        let source = format!("% 😀\r\n{declare}{{parent}}{{member}}");
        std::fs::write(dir.path().join("sets.tex"), &source).unwrap();
        ws.open(sets.clone(), source, 9);
        ws.versioned_edits = true;
        let query = key(&ws, &sets, &format!("{declare}{{"), "parent");
        assert_eq!(
            features::prepare_rename(&ws, &sets, query.range.start)
                .unwrap()
                .unwrap(),
            PrepareRenameResponse::RangeWithPlaceholder {
                range: query.range,
                placeholder: "parent".into()
            }
        );
        let edit = features::rename(&ws, &sets, query.range.start, "newset")
            .unwrap()
            .unwrap();
        let Some(DocumentChanges::Edits(changes)) = edit.document_changes else {
            panic!()
        };
        assert_eq!(changes.len(), 2);
        for change in changes {
            assert_eq!(
                change.text_document.version,
                Some(if change.text_document.uri == main {
                    5
                } else {
                    9
                })
            );
            assert_eq!(
                change.edits.len(),
                if change.text_document.uri == main {
                    2
                } else {
                    1
                }
            );
        }
    }
}
#[test]
fn unsaved_new_consumers_reject_capture_and_closing_them_restores_source_rename() {
    let (dir, mut ws, main) = clean(r"\newrefsection[chosen]\input{sets}\cite{parent}");
    let sets = main.join("sets.tex").unwrap();
    std::fs::write(
        dir.path().join("sets.tex"),
        r"\defbibentryset{parent}{member}",
    )
    .unwrap();
    ws.refresh_project(&main);
    let query = key(&ws, &sets, r"\defbibentryset{", "parent");
    assert!(features::rename(&ws, &sets, query.range.start, "newset")
        .unwrap()
        .is_some());
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\input{sets}\cite{newset}\end{document}".into(),9);
    assert!(features::rename(&ws, &sets, query.range.start, "newset").is_err());
    ws.documents.remove(&other);
    assert!(features::rename(&ws, &sets, query.range.start, "newset")
        .unwrap()
        .is_some());
}
#[test]
fn member_renames_merge_quoted_relation_edits_and_reject_new_relation_capture() {
    let (_dir, mut ws, main) =
        clean(r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{member}");
    let bib = main.join("section.bib").unwrap();
    ws.open(
        bib.clone(),
        "@book{member} @book{parent} @book{child,related=\"member,member\"}".into(),
        8,
    );
    let query = key(&ws, &main, r"\defbibentryset{parent}{", "member");
    let edit = features::rename(&ws, &main, query.range.start, "new\"member")
        .unwrap()
        .unwrap();
    assert_eq!(edits(edit.clone())[&bib].len(), 2);
    apply(&mut ws, edit);
    assert!(ws.documents[&bib]
        .text
        .source
        .contains("related={new\"member,new\"member}"));
    let query = key(&ws, &main, r"\defbibentryset{parent}{", "new\"member");
    let text = ws.documents[&bib].text.source.clone() + " @book{capture,related={new}}";
    ws.open(bib, text, 21);
    assert!(features::rename(&ws, &main, query.range.start, "new").is_err());
}
#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn source_and_member_renames_preserve_native_numeric_sections() {
    for member in [false, true] {
        let (dir, mut ws, main) = clean(
            r"\cite{parent}\cite{member}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}\cite{member}\cite{child}\printbibliography",
        );
        if member {
            let bib = main.join("section.bib").unwrap();
            let text = ws.documents[&bib]
                .text
                .source
                .replace("title={Standalone child}", "crossref={parent}");
            std::fs::write(dir.path().join("section.bib"), &text).unwrap();
            ws.open(bib, text, 9);
        }
        let original = compile(dir.path());
        let query = key(
            &ws,
            &main,
            if member {
                r"\defbibentryset{parent}{"
            } else {
                r"\defbibentryset{"
            },
            if member { "member" } else { "parent" },
        );
        let old = if member { "member" } else { "parent" };
        let new = if member { "newmember" } else { "newset" };
        let edit = features::rename(&ws, &main, query.range.start, new)
            .unwrap()
            .unwrap();
        apply(&mut ws, edit);
        let edited = tempfile::tempdir().unwrap();
        for name in ["main.tex", "default.bib", "section.bib"] {
            let uri = main.join(name).unwrap();
            std::fs::write(edited.path().join(name), &ws.documents[&uri].text.source).unwrap();
        }
        let after = compile(edited.path());
        assert_eq!(section(&after, 0), section(&original, 0));
        assert_eq!(
            entries(section(&after, 1)),
            entries(&section(&original, 1).replace(&format!("{{{old}}}"), &format!("{{{new}}}")))
        );
    }
}
#[test]
fn wildcard_source_rename_cannot_expose_a_shadowed_database_entry() {
    let (_dir, mut ws, main) =
        clean(r"\newrefsection[chosen]\defbibentryset{parent}{member}\nocite{*}\printbibliography");
    let query = key(&ws, &main, r"\defbibentryset{", "parent");
    assert!(features::prepare_rename(&ws, &main, query.range.start).is_err());
    assert!(features::rename(&ws, &main, query.range.start, "newset").is_err());
    let bib = main.join("section.bib").unwrap();
    ws.open(bib, "@book{member,title={Member}}".into(), 9);
    assert!(features::rename(&ws, &main, query.range.start, "newset")
        .unwrap()
        .is_some());
}
#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_wildcard_rename_reveals_the_raw_database_entry_and_source_only_sets_remain_editable() {
    let (dir, mut ws, main) =
        clean(r"\newrefsection[chosen]\defbibentryset{parent}{member}\nocite{*}\printbibliography");
    let query = key(&ws, &main, r"\defbibentryset{", "parent");
    let original = compile(dir.path());
    assert!(entries(section(&original, 1))["parent"].starts_with("{set}"));
    let edited = tempfile::tempdir().unwrap();
    for name in ["main.tex", "default.bib", "section.bib"] {
        let uri = main.join(name).unwrap();
        let doc = &ws.documents[&uri];
        let text = if uri == main {
            doc.text
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(query.range),
                    range_length: None,
                    text: "newset".into(),
                }])
                .unwrap()
                .source
        } else {
            doc.text.source.clone()
        };
        std::fs::write(edited.path().join(name), text).unwrap();
    }
    let after = compile(edited.path());
    let result = entries(section(&after, 1));
    assert!(result["parent"].starts_with("{book}"), "{after}");
    assert!(result["newset"].starts_with("{set}"), "{after}");
    assert!(features::rename(&ws, &main, query.range.start, "newset").is_err());
    let bib = main.join("section.bib").unwrap();
    let text = "@book{member,title={Member}}";
    std::fs::write(dir.path().join("section.bib"), text).unwrap();
    ws.open(bib, text.into(), 10);
    let original = compile(dir.path());
    let edit = features::rename(&ws, &main, query.range.start, "newset")
        .unwrap()
        .unwrap();
    apply(&mut ws, edit);
    for name in ["main.tex", "default.bib", "section.bib"] {
        let uri = main.join(name).unwrap();
        std::fs::write(edited.path().join(name), &ws.documents[&uri].text.source).unwrap();
    }
    for name in ["main.aux", "main.bcf", "main.bbl", "main.blg"] {
        let path = edited.path().join(name);
        if path.exists() {
            std::fs::remove_file(path).unwrap();
        }
    }
    let after = compile(edited.path());
    assert_eq!(
        entries(section(&after, 1)),
        entries(&section(&original, 1).replace("{parent}", "{newset}"))
    );
}
#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_unused_source_renames_materialize_an_additional_set_and_remain_protected() {
    let (dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{member}\cite{parent}\cite{child}\printbibliography",
    );
    let original = compile(dir.path());
    assert!(entries(section(&original, 1))["parent"].starts_with("{book}"));
    let query = key(&ws, &main, r"\defbibentryset{", "parent");
    let edit = WorkspaceEdit {
        changes: Some(std::collections::HashMap::from([(
            main.clone(),
            vec![TextEdit {
                range: query.range,
                new_text: "unused".into(),
            }],
        )])),
        ..Default::default()
    };
    assert!(features::prepare_rename(&ws, &main, query.range.start).is_err());
    assert!(features::rename(&ws, &main, query.range.start, "unused").is_err());
    apply(&mut ws, edit);
    let edited = tempfile::tempdir().unwrap();
    for name in ["main.tex", "default.bib", "section.bib"] {
        let uri = main.join(name).unwrap();
        std::fs::write(edited.path().join(name), &ws.documents[&uri].text.source).unwrap();
    }
    let after = compile(edited.path());
    let before = entries(section(&original, 1));
    let after = entries(section(&after, 1));
    assert_eq!(after["parent"], before["parent"]);
    assert!(!before.contains_key("unused") && !before.contains_key("member"));
    assert!(after["unused"].starts_with("{set}"));
    assert!(after["member"].contains(r"\inset{unused}"));
}
