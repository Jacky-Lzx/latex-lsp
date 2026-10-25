use latex_lsp::{features, workspace::Workspace};
use lsp_types::*;
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
fn completions(ws: &Workspace, uri: &Url, marker: &str) -> Vec<CompletionItem> {
    let doc = &ws.documents[uri];
    let offset = doc.text.source.find(marker).unwrap() + marker.len();
    features::completion(ws, uri, doc.text.position(offset))
}
fn candidate(ws: &Workspace, uri: &Url, marker: &str, name: &str) -> CompletionItem {
    completions(ws, uri, marker)
        .into_iter()
        .find(|item| item.label == name)
        .unwrap_or_else(|| panic!("missing {name} at {marker}"))
}
#[test]
fn a_source_override_candidate_uses_its_source_metadata_and_replaces_the_complete_key() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,child}",
    );
    let bib = main.join("section.bib").unwrap();
    let item = candidate(&ws, &bib, "crossref={pa", "parent");
    assert!(
        item.detail
            .as_ref()
            .unwrap()
            .contains("Dynamic bibliography set: member"),
        "{item:?}"
    );
    assert!(
        item.detail.as_ref().unwrap().contains(main.path()),
        "{item:?}"
    );
    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
        panic!()
    };
    assert_eq!(edit.range, key(&ws, &bib, "crossref={", "parent").range);
    assert_eq!(edit.new_text, "parent");
}
#[test]
fn an_explicit_early_request_preserves_book_candidate_metadata() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{member}\cite{parent,child}",
    );
    let bib = main.join("section.bib").unwrap();
    let item = candidate(&ws, &bib, "crossref={pa", "parent");
    assert!(
        item.detail.as_ref().unwrap().contains("Selected parent"),
        "{item:?}"
    );
    assert!(!item
        .detail
        .as_ref()
        .unwrap()
        .contains("Dynamic bibliography set"));
}
#[test]
fn distinct_numeric_namespaces_remove_conflicting_candidates_but_keep_common_books() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,child}\newrefsection[chosen]\cite{parent,child}",
    );
    let bib = main.join("section.bib").unwrap();
    let items = completions(&ws, &bib, "crossref={");
    assert!(!items.iter().any(|item| item.label == "parent"));
    assert!(items.iter().any(|item| item.label == "member"));
}
#[test]
fn source_only_named_declarations_are_not_database_parent_candidates() {
    for declaration in [
        r"\csuse{defbibentryset}",
        r"\csname defbibentryset\endcsname",
    ] {
        let (_dir, ws, main) = setup(
            "",
            &format!(
                r"\newrefsection[chosen]{declaration}{{collected}}{{member}}\cite{{collected,child}}"
            ),
        );
        let bib = main.join("section.bib").unwrap();
        let items = completions(&ws, &bib, "crossref={");
        assert!(!items.iter().any(|item| item.label == "collected"));
        assert!(items.iter().any(|item| item.label == "member"));
        let (_dir, ws, main) = setup(
            "",
            &format!(
                r"\newrefsection[chosen]{declaration}{{parent}}{{member}}\cite{{parent,child}}"
            ),
        );
        let item = candidate(
            &ws,
            &main.join("section.bib").unwrap(),
            "crossref={pa",
            "parent",
        );
        assert!(item
            .detail
            .unwrap()
            .contains("Dynamic bibliography set: member"));
    }
}

#[test]
fn repeated_execution_of_one_physical_declaration_is_a_consistent_candidate() {
    let (dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\input{sets}\cite{parent,child}\newrefsection[chosen]\input{sets}\cite{parent,child}",
    );
    std::fs::write(
        dir.path().join("sets.tex"),
        r"\defbibentryset{parent}{member}",
    )
    .unwrap();
    ws.refresh_project(&main);
    let bib = main.join("section.bib").unwrap();
    let item = candidate(&ws, &bib, "crossref={pa", "parent");
    assert!(
        item.detail.as_ref().unwrap().contains("sets.tex"),
        "{item:?}"
    );
}
#[test]
fn unsaved_consumers_cannot_reintroduce_book_metadata_for_a_source_override() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,child}",
    );
    let bib = main.join("section.bib").unwrap();
    ws.open(main.join("unsaved.tex").unwrap(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\cite{parent,child}\end{document}".into(),9);
    let items = completions(&ws, &bib, "crossref={");
    assert!(!items.iter().any(|item| item.label == "parent"));
    assert!(items.iter().any(|item| item.label == "member"));
}
#[test]
fn an_unfinished_relation_value_still_offers_the_certified_source_candidate() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,child}",
    );
    let bib = main.join("section.bib").unwrap();
    ws.open(
        bib.clone(),
        "@book{parent,title={Book}}\n@book{member}\n@inbook{child,crossref={pa".into(),
        9,
    );
    let item = candidate(&ws, &bib, "crossref={pa", "parent");
    assert!(item
        .detail
        .as_ref()
        .unwrap()
        .contains("Dynamic bibliography set: member"));
}
#[test]
fn unproved_source_execution_keeps_relation_candidates_protected() {
    for body in [
        r"\newrefsection[chosen]\ifunknown\defbibentryset{parent}{member}\fi\cite{parent}",
        r"\newrefsection[chosen]\newcommand\later{\defbibentryset{parent}{member}}\cite{parent}",
    ] {
        let (_dir, ws, main) = setup("", body);
        let bib = main.join("section.bib").unwrap();
        assert!(completions(&ws, &bib, "crossref={").is_empty());
    }
}

fn apply(source: &str, item: &CompletionItem) -> String {
    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit.as_ref() else {
        panic!("{item:?}")
    };
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    latex_lsp::text::Text::new(source.into())
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
        .source
}
#[test]
fn relation_edits_preserve_utf16_suffixes_lists_and_only_valid_source_parent_types() {
    for (field, marked, expected, name) in [
        ("crossref", "paBAD", "parent", "parent"),
        ("xref", "paBAD", "parent", "parent"),
        (
            "xdata",
            "child, meBAD,child",
            "child, member,child",
            "member",
        ),
        (
            "related",
            "child, meBAD,child",
            "child, member,child",
            "member",
        ),
        (
            "entryset",
            "child, meBAD,child",
            "child, member,child",
            "member",
        ),
    ] {
        let (_dir, mut ws, main) = setup(
            "",
            r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
        );
        let bib = main.join("section.bib").unwrap();
        let member_type = if field == "xdata" { "xdata" } else { "book" };
        let source = format!(
            "% 😀\r\n@book{{parent}} @{member_type}{{member}}\r\n@book{{child,{field}={{{marked}}}}}"
        );
        ws.open(bib.clone(), source.clone(), 7);
        let marker = if name == "parent" { "paBAD" } else { "meBAD" };
        let prefix = format!("{field}={{{}", marked.split_once(marker).unwrap().0);
        let item = candidate(&ws, &bib, &(prefix + &marker[..2]), name);
        assert_eq!(apply(&source, &item), source.replace(marked, expected));
        assert_eq!(
            item.detail
                .unwrap()
                .contains("Dynamic bibliography set: member"),
            name == "parent"
        );
        if name == "member" {
            let items = completions(&ws, &bib, &format!("{field}={{"));
            assert!(!items.iter().any(|item| item.label == "parent"));
        }
    }
}

#[test]
fn a_quoted_source_parent_key_converts_the_entire_quoted_value_to_braces() {
    let (_dir, mut ws, main) = setup(
        "",
        r#"\newrefsection[chosen]\defbibentryset{s"et}{member}\cite{s"et}"#,
    );
    let bib = main.join("section.bib").unwrap();
    let source = "% 😀\r\n@book{s\"et} @book{member}\r\n@book{child,xref=\"sBAD\"}";
    ws.open(bib.clone(), source.into(), 8);
    let item = candidate(&ws, &bib, "xref=\"s", "s\"et");
    assert_eq!(apply(source, &item), source.replace("\"sBAD\"", "{s\"et}"));
}

#[test]
fn early_explicit_and_wildcard_mixtures_do_not_offer_an_unproved_identity() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{member}\nocite{*}",
    );
    let bib = main.join("section.bib").unwrap();
    let items = completions(&ws, &bib, "crossref={");
    assert!(!items.iter().any(|item| item.label == "parent"));
    assert!(items.iter().any(|item| item.label == "member"));
}
#[test]
fn a_wildcard_without_an_early_explicit_request_does_not_lock_the_book() {
    let (_dir, ws, main) = setup(
        "",
        r"\newrefsection[chosen]\nocite{*}\defbibentryset{parent}{member}",
    );
    let item = candidate(
        &ws,
        &main.join("section.bib").unwrap(),
        "crossref={pa",
        "parent",
    );
    assert!(item.detail.unwrap().contains("Dynamic bibliography set"));
}
#[test]
fn ids_expressions_preambles_duplicate_fields_and_truncation_keep_their_guards() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,child}",
    );
    let bib = main.join("section.bib").unwrap();
    let base = ws.documents[&bib].text.source.clone();
    for suffix in [
        ",IDS={parent}",
        ",IDS=unknown",
        ",ids={old} # unknown",
        ",CROSSREF={member}",
    ] {
        let invalid = base.replace("crossref={parent}", &format!("crossref={{parent}}{suffix}"));
        ws.open(bib.clone(), invalid, 9);
        assert!(completions(&ws, &bib, "crossref={").is_empty(), "{suffix}");
    }
    ws.open(
        bib.clone(),
        format!("{base}\n@preamble{{\"\\defbibentryset{{other}}{{member}}\"}}"),
        10,
    );
    assert!(completions(&ws, &bib, "crossref={").is_empty());
    ws.open(bib.clone(), base, 11);
    assert!(completions(&ws, &bib, "crossref={pa")
        .iter()
        .any(|item| item.label == "parent"));
    ws.index_truncated.insert("test discovery limit".into());
    assert!(completions(&ws, &bib, "crossref={").is_empty());
}
#[test]
fn distinct_declarations_and_nested_sets_do_not_create_partial_candidate_catalogs() {
    for declarations in [
        r"\defbibentryset{parent}{member}\defbibentryset{parent}{member}",
        r"\defbibentryset{parent}{member}\defbibentryset{outer}{parent}",
    ] {
        let (_dir, ws, main) = setup(
            "",
            &format!(r"\newrefsection[chosen]{declarations}\cite{{parent,child}}"),
        );
        assert!(completions(&ws, &main.join("section.bib").unwrap(), "crossref={").is_empty());
    }
}
#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn accepted_relation_completions_compile_with_the_previewed_set_or_book_identity() {
    for early in [false, true] {
        let body = format!(
            r"\newrefsection[chosen]{}\defbibentryset{{parent}}{{member}}\cite{{parent,child}}\printbibliography",
            if early { r"\cite{parent}" } else { "" }
        );
        let (dir, mut ws, main) = setup("", &body);
        let bib = main.join("section.bib").unwrap();
        let source = ws.documents[&bib]
            .text
            .source
            .replace("crossref={parent}", "crossref={paBAD}");
        ws.open(bib.clone(), source.clone(), 7);
        let item = candidate(&ws, &bib, "crossref={pa", "parent");
        let detail = item.detail.as_ref().unwrap();
        assert_eq!(
            detail.contains("Dynamic bibliography set"),
            !early,
            "{detail}"
        );
        assert_eq!(detail.contains("Selected parent"), early, "{detail}");
        std::fs::write(dir.path().join("section.bib"), apply(&source, &item)).unwrap();
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
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{program}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
        let section = bbl
            .split_once(r"\refsection{1}")
            .unwrap()
            .1
            .split_once(r"\endrefsection")
            .unwrap()
            .0;
        assert!(
            section.contains(&format!(
                "\\entry{{parent}}{{{}}}",
                if early { "book" } else { "set" }
            )),
            "{section}"
        );
        let child = section
            .split_once(r"\entry{child}")
            .unwrap()
            .1
            .split_once(r"\endentry")
            .unwrap()
            .0;
        assert_eq!(
            child.contains(r"\field{booktitle}{Selected parent}"),
            early,
            "{child}"
        );
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        assert!(!log.contains("undefined"), "{log}");
        let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
        assert!(
            !blg.contains("ERROR") && !blg.contains("I didn't find"),
            "{blg}"
        );
    }
}

#[test]
fn source_only_parent_keys_keep_tex_navigation_without_database_relation_previews() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collected,child}",
    );
    let bib = main.join("section.bib").unwrap();
    let source = "@book{member,title={Member}}\n@book{child,crossref={collected}}";
    ws.open(bib.clone(), source.into(), 9);
    let relation = key(&ws, &bib, "crossref={", "collected");
    assert!(completions(&ws, &bib, "crossref={coll").is_empty());
    assert!(features::definitions(&ws, &bib, relation.range.start).is_empty());
    assert!(features::hover(&ws, &bib, relation.range.start).is_none());
    let declaration = key(&ws, &main, r"\defbibentryset{", "collected");
    let citation = key(&ws, &main, r"\cite{", "collected");
    assert_eq!(
        features::definitions(&ws, &main, citation.range.start),
        vec![declaration.clone()]
    );
    ws.open(
        bib.clone(),
        format!("@book{{collected,title={{Underlying parent}}}}\n{source}"),
        10,
    );
    assert!(candidate(&ws, &bib, "crossref={coll", "collected")
        .detail
        .unwrap()
        .contains("Dynamic bibliography set"));
    assert_eq!(
        features::definitions(
            &ws,
            &bib,
            key(&ws, &bib, "crossref={", "collected").range.start
        ),
        vec![declaration]
    );
}

fn native_relation_fixture(dir: &std::path::Path, cyclic_set: bool) -> (String, String) {
    for (stage, (program, args)) in [
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
    ]
    .into_iter()
    .enumerate()
    {
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        if cyclic_set && stage >= 2 {
            assert!(
                !output.status.success(),
                "nested source set unexpectedly compiled"
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("TeX capacity exceeded"));
            break;
        }
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    (
        std::fs::read_to_string(dir.join("main.bbl")).unwrap(),
        std::fs::read_to_string(dir.join("main.blg")).unwrap(),
    )
}
#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_biber_source_parents_need_database_entries_and_supported_relation_types() {
    for field in ["crossref", "xref"] {
        for database in [false, true] {
            let (dir, mut ws, main) = setup(
                "",
                r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collected,child}\printbibliography",
            );
            let bib = main.join("section.bib").unwrap();
            let base=format!("@book{{member,title={{Member}}}}\n@book{{child,title={{Child}},{field}={{collBAD}}}}");
            let library = if database {
                format!("@book{{collected,title={{Database parent}}}}\n{base}")
            } else {
                base
            };
            ws.open(bib.clone(), library.clone(), 8);
            let candidate = completions(&ws, &bib, &format!("{field}={{coll"))
                .into_iter()
                .find(|item| item.label == "collected");
            assert_eq!(candidate.is_some(), database, "{field}");
            let completed = if let Some(item) = candidate {
                apply(&library, &item)
            } else {
                library.replace("collBAD", "collected")
            };
            std::fs::write(dir.path().join("section.bib"), completed).unwrap();
            let (bbl, blg) = native_relation_fixture(dir.path(), false);
            assert!(bbl.contains(r"\entry{collected}{set}"), "{bbl}");
            assert_eq!(
                blg.contains(&format!(
                    "I didn't find a database entry for {field} 'collected'"
                )),
                !database,
                "{blg}"
            );
            assert_eq!(
                bbl.contains(&format!("\\strng{{{field}}}{{collected}}")),
                database,
                "{bbl}"
            );
        }
    }
    for (field, warning) in [
        ("xdata", "not an XDATA entry"),
        ("related", "has no entryset field"),
        ("entryset", "no longer needed in set member entries"),
    ] {
        let (dir, mut ws, main) = setup(
            "",
            r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent,child}\printbibliography",
        );
        let bib = main.join("section.bib").unwrap();
        let entry = if field == "entryset" { "set" } else { "book" };
        let library=format!("@book{{parent,title={{Database parent}}}}\n@book{{member,title={{Member}}}}\n@{entry}{{child,title={{Child}},{field}={{parent}}}}");
        ws.open(bib.clone(), library.clone(), 8);
        assert!(!completions(&ws, &bib, &format!("{field}={{"))
            .iter()
            .any(|item| item.label == "parent"));
        std::fs::write(dir.path().join("section.bib"), library).unwrap();
        let (_, blg) = native_relation_fixture(dir.path(), field == "entryset");
        assert!(blg.contains(warning), "{blg}");
    }
}
