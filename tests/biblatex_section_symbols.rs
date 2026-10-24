use latex_lsp::{features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(extra: &str, body: &str, shared: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}{extra}\begin{{document}}{body}\end{{document}}"
    );
    for (name, text) in [
        ("main.tex", source.as_str()),
        ("default.bib", "@book{parent,title={Default parent},ids={old}}\n@inbook{childdefault,crossref={parent}}"),
        ("section.bib", "@book{parent,title={Selected parent},ids={old}}\n@inbook{childselected,crossref={parent}}"),
        ("shared.bib", shared),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main)
}

fn check(ws: &Workspace, origin: &Url, marker: &str, target: Option<(&Url, &str)>) {
    let doc = &ws.documents[origin];
    let start = doc.text.source.find(marker).unwrap() + marker.len();
    let position = doc.text.position(start);
    let definitions = features::definitions(ws, origin, position);
    let hover = features::hover(ws, origin, position);
    if let Some((target, key)) = target {
        let dest = &ws.documents[target];
        let offset = dest.text.source.find(key).unwrap();
        assert_eq!(
            definitions,
            vec![Location::new(
                target.clone(),
                dest.text.range(offset, offset + key.len())
            )],
            "{origin} {marker}"
        );
        let hover = hover.expect(marker);
        assert_eq!(hover.range, Some(doc.text.range(start, start + key.len())));
        let HoverContents::Markup(contents) = hover.contents else {
            panic!()
        };
        assert!(contents.value.contains(target.path()), "{}", contents.value);
        if target.path().ends_with("section.bib") {
            assert!(
                !contents.value.contains("Default parent"),
                "{}",
                contents.value
            );
        }
    } else {
        assert!(definitions.is_empty(), "{marker}: {definitions:?}");
        assert!(hover.is_none(), "{marker}: {hover:?}");
    }
}

#[test]
fn declarations_and_crossrefs_keep_default_and_selected_database_identities() {
    let (_dir, ws, main) = setup(
        "",
        r"\cite{childdefault}\newrefsection[chosen]\cite{childselected}",
        "",
    );
    for name in ["default.bib", "section.bib"] {
        let bib = main.join(name).unwrap();
        check(&ws, &bib, "@book{", Some((&bib, "parent")));
        check(&ws, &bib, "crossref={", Some((&bib, "parent")));
        check(&ws, &bib, "ids={", Some((&bib, "old")));
        assert!(features::rename(&ws, &bib, Position::new(0, 8), "renamed").is_err());
    }
}

#[test]
fn relation_lists_resolve_each_member_in_the_owners_sections() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let bib = main.join("section.bib").unwrap();
    let source = "% 😀\r\n@book{parent😀,title={Selected parent}}\r\n@book{second}\r\n@book{child,crossref={parent😀},xref=\"parent😀\",xdata={parent😀,second},related={parent😀,second},entryset={parent😀,second}}";
    ws.open(bib.clone(), source.into(), 7);
    for marker in [
        "crossref={",
        "xref=\"",
        "xdata={",
        "related={",
        "entryset={",
    ] {
        check(&ws, &bib, marker, Some((&bib, "parent😀")));
    }
    for marker in [
        "xdata={parent😀,",
        "related={parent😀,",
        "entryset={parent😀,",
    ] {
        check(&ws, &bib, marker, Some((&bib, "second")));
    }
}

#[test]
fn global_relations_require_identical_targets_across_all_resource_views() {
    let (_dir, mut ws, main) = setup(
        r"\addglobalbib{shared.bib}",
        r"\cite{shared}\newrefsection[chosen]\cite{shared}",
        "@book{shared,crossref={parent}}\n@book{stable}\n@book{other,xref={stable}}",
    );
    let shared = main.join("shared.bib").unwrap();
    check(&ws, &shared, "crossref={", None);
    check(&ws, &shared, "xref={", Some((&shared, "stable")));
    let selected = main.join("section.bib").unwrap();
    ws.open(selected, "@book{different}".into(), 9);
    // Absence in one active view is also disagreement.
    check(&ws, &shared, "crossref={", None);
}

#[test]
fn open_unsaved_consumers_participate_and_independent_roots_do_not() {
    let (dir, mut ws, main) = setup(
        r"\addglobalbib{shared.bib}",
        r"\newrefsection[chosen]\cite{shared}",
        "@book{shared,crossref={parent}}",
    );
    let shared = main.join("shared.bib").unwrap();
    // This global resource is present in the default view even without cites.
    check(&ws, &shared, "crossref={", None);
    let default = main.join("default.bib").unwrap();
    ws.open(default, "@book{defaultonly}".into(), 7);
    let source = r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\addbibresource{shared.bib}\begin{document}\cite{shared}\end{document}";
    ws.open(main.clone(), source.into(), 8);
    check(
        &ws,
        &shared,
        "crossref={",
        Some((&main.join("section.bib").unwrap(), "parent")),
    );
    let other = Url::from_file_path(dir.path().join("unsaved.tex")).unwrap();
    ws.open(other.clone(), r"\documentclass{article}\usepackage{biblatex}\addbibresource{default.bib}\addglobalbib{shared.bib}\begin{document}\newrefsection[]\cite{shared}\end{document}".into(), 9);
    // Empty selections use ordinary resource semantics, so another selected root
    // ensures that relation provenance is evaluated rather than a union.
    let selecting = source.replace(
        r"\cite{shared}",
        r"\newrefsection[section.bib,shared.bib]\cite{shared}",
    );
    ws.open(main.clone(), selecting, 10);
    check(&ws, &shared, "crossref={", None);
    ws.open(
        other,
        r"\documentclass{article}\begin{document}Independent\end{document}".into(),
        11,
    );
    check(
        &ws,
        &shared,
        "crossref={",
        Some((&main.join("section.bib").unwrap(), "parent")),
    );
}

#[test]
fn unknown_execution_does_not_fall_back_to_union_targets() {
    let (_dir, ws, main) = setup(
        "",
        r"\ifunknown\newrefsection[chosen]\fi\cite{childselected}",
        "",
    );
    check(&ws, &main.join("section.bib").unwrap(), "crossref={", None);
}

#[test]
fn declaration_navigation_remains_lexically_exact_with_same_file_duplicates() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let bib = main.join("section.bib").unwrap();
    let source = "@book{parent,title={First}}\n@book{parent,title={Second}}";
    ws.open(bib.clone(), source.into(), 8);
    let at = source.rfind("parent").unwrap();
    assert_eq!(
        features::definitions(&ws, &bib, Text::new(source.into()).position(at)),
        vec![Location::new(
            bib.clone(),
            Text::new(source.into()).range(at, at + 6)
        )]
    );
    let HoverContents::Markup(hover) =
        features::hover(&ws, &bib, Text::new(source.into()).position(at))
            .unwrap()
            .contents
    else {
        panic!()
    };
    assert!(hover.value.contains("Second"), "{}", hover.value);
    assert!(!hover.value.contains("First"), "{}", hover.value);
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_biber_resolves_relation_and_ids_targets_in_each_reference_section() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\nocite{parent,child,childdata,childxref,childrelated,childset}\printbibliography\newrefsection[chosen]\nocite{parent,child,childdata,childxref,childrelated,childset}\printbibliography",
        "",
    );
    let dir = main.to_file_path().unwrap().parent().unwrap().to_path_buf();
    for (name, title) in [
        ("default.bib", "Default parent"),
        ("section.bib", "Selected parent"),
    ] {
        let source = format!("@book{{parent,ids={{parentalias}},title={{{title}}},author={{Parent, Pat}},year={{2001}}}}\n@xdata{{data,ids={{dataalias}},publisher={{{title} publisher}}}}\n@book{{member,ids={{memberalias}},title={{{title} member}},author={{Member, May}},year={{2002}}}}\n@inbook{{child,crossref={{parentalias}},title={{Child}},pages={{1--2}}}}\n@book{{childdata,xdata={{dataalias}},title={{Data child}},author={{Child, Chris}},year={{2003}}}}\n@book{{childxref,xref={{parentalias}},title={{Xref child}},author={{Child, Chris}},year={{2003}}}}\n@book{{childrelated,related={{parentalias}},title={{Related child}},author={{Child, Chris}},year={{2003}}}}\n@set{{childset,entryset={{memberalias}}}}");
        let bib = main.join(name).unwrap();
        let pairs = [
            ("crossref", "parentalias"),
            ("xref", "parentalias"),
            ("xdata", "dataalias"),
            ("related", "parentalias"),
            ("entryset", "memberalias"),
        ];
        let mut incomplete = source.clone();
        for (field, key) in pairs {
            incomplete = incomplete.replace(
                &format!("{field}={{{key}}}"),
                &format!("{field}={{{}BAD}}", &key[..2]),
            );
        }
        ws.open(bib.clone(), incomplete, 8);
        for (stage, (field, key)) in pairs.into_iter().enumerate() {
            let (_, updated) = completed(&ws, &bib, &format!("{field}={{{}", &key[..2]), key);
            ws.open(bib.clone(), updated, 9 + stage as i32);
            check(&ws, &bib, &format!("{field}={{"), Some((&bib, key)));
        }
        assert_eq!(ws.documents[&bib].text.source, source);
        std::fs::write(bib.to_file_path().unwrap(), &ws.documents[&bib].text.source).unwrap();
    }
    let run = |program: &str, args: &[&str]| {
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let latex = || {
        run(
            "pdflatex",
            &[
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ],
        )
    };
    latex();
    run("biber", &["main"]);
    latex();
    latex();
    let bbl = std::fs::read_to_string(dir.join("main.bbl")).unwrap();
    for (section, own, other) in [
        (0, "Default parent", "Selected parent"),
        (1, "Selected parent", "Default parent"),
    ] {
        let marker = format!("\\refsection{{{section}}}");
        let body = bbl
            .split_once(&marker)
            .unwrap()
            .1
            .split_once("\\endrefsection")
            .unwrap()
            .0;
        assert!(
            body.contains(&format!("\\field{{booktitle}}{{{own}}}")),
            "{body}"
        );
        assert!(body.contains(&format!("{{{own} publisher}}")), "{body}");
        assert!(body.contains(&format!("{{{own} member}}")), "{body}");
        assert!(!body.contains(other), "{body}");
        for (alias, primary) in [
            ("parentalias", "parent"),
            ("dataalias", "data"),
            ("memberalias", "member"),
        ] {
            assert!(
                body.contains(&format!("\\keyalias{{{alias}}}{{{primary}}}")),
                "{body}"
            );
        }
    }
    let log = std::fs::read_to_string(dir.join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
}

#[test]
fn active_duplicate_targets_are_reported_together_without_an_arbitrary_hover() {
    let (_dir, ws, main) = setup(
        r"\addglobalbib{shared.bib}",
        r"\newrefsection[chosen]\cite{childselected}",
        "@book{parent,title={Global parent}}",
    );
    let bib = main.join("section.bib").unwrap();
    let doc = &ws.documents[&bib];
    let position = doc
        .text
        .position(doc.text.source.find("crossref={").unwrap() + "crossref={".len());
    let mut actual: Vec<_> = features::definitions(&ws, &bib, position)
        .into_iter()
        .map(|item| item.uri)
        .collect();
    actual.sort();
    let mut expected = vec![bib.clone(), main.join("shared.bib").unwrap()];
    expected.sort();
    assert_eq!(actual, expected);
    assert!(features::hover(&ws, &bib, position).is_none());
}

#[test]
fn duplicate_relation_fields_are_not_certified_as_effective_relations() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let bib = main.join("section.bib").unwrap();
    ws.open(
        bib.clone(),
        "@book{parent}\n@book{child,crossref={parent},CROSSREF={parent}}".into(),
        8,
    );
    check(&ws, &bib, "crossref={", None);
    check(&ws, &bib, "CROSSREF={", None);
}

#[test]
fn ids_relations_keep_alias_identity_separate_from_the_primary_key() {
    let (_dir, mut ws, main) = setup(
        "",
        r"\cite{childdefault}\newrefsection[chosen]\cite{childselected}",
        "",
    );
    for name in ["default.bib", "section.bib"] {
        let bib = main.join(name).unwrap();
        let source = ws.documents[&bib]
            .text
            .source
            .replace("crossref={parent}", "crossref={old}");
        ws.open(bib.clone(), source, 8);
        check(&ws, &bib, "crossref={", Some((&bib, "old")));
        check(&ws, &bib, "@book{", Some((&bib, "parent")));
    }
}

#[test]
fn truncated_indexes_suppress_relation_fallback_and_restore_after_reindexing() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let bib = main.join("section.bib").unwrap();
    check(&ws, &bib, "crossref={", Some((&bib, "parent")));
    ws.index_truncated.insert("test discovery limit".into());
    check(&ws, &bib, "crossref={", None);
    ws.index_truncated.clear();
    check(&ws, &bib, "crossref={", Some((&bib, "parent")));
}

#[test]
fn section_database_references_do_not_fall_back_to_a_partial_project_union() {
    let (_dir, ws, main) = setup(
        "",
        r"\cite{childdefault}\newrefsection[chosen]\cite{childselected}",
        "",
    );
    for name in ["default.bib", "section.bib"] {
        let bib = main.join(name).unwrap();
        let doc = &ws.documents[&bib];
        for marker in ["@book{", "crossref={"] {
            let at = doc
                .text
                .position(doc.text.source.find(marker).unwrap() + marker.len());
            for include in [false, true] {
                assert!(features::references(&ws, &bib, at, include).is_empty());
            }
        }
    }
}

#[test]
fn source_key_declarations_protect_database_targets_without_disabling_resource_labels() {
    for declaration in [
        r"\defbibentryset{parent}{member}",
        r"\csuse{defbibentryset}{parent}{member}",
        r"\csname defbibentryset\endcsname{parent}{member}",
        r"\newcommand\makeSet{\defbibentryset}\makeSet{parent}{member}",
        r"\LetLtxMacro\makeSet\defbibentryset\makeSet{parent}{member}",
        r"\begin{thebibliography}{9}\bibitem{parent}Replacement\end{thebibliography}",
    ] {
        let (_dir, ws, main) = setup(
            "",
            &format!(r"\newrefsection[chosen]{declaration}\cite{{parent}}"),
            "",
        );
        let doc = &ws.documents[&main];
        let position = doc
            .text
            .position(doc.text.source.rfind(r"\cite{parent}").unwrap() + 8);
        assert!(features::definitions(&ws, &main, position).is_empty());
        assert!(features::hover(&ws, &main, position).is_none());
        assert!(features::completion(&ws, &main, position).is_empty());
        for name in ["default.bib", "section.bib"] {
            let bib = main.join(name).unwrap();
            check(&ws, &bib, "crossref={", None);
            check(&ws, &bib, "@book{", Some((&bib, "parent")));
        }
        let selection = doc
            .text
            .position(doc.text.source.find("[chosen]").unwrap() + 3);
        let definitions = features::definitions(&ws, &main, selection);
        assert_eq!(definitions.len(), 1, "{declaration}: {definitions:?}");
        assert_eq!(definitions[0].uri, main);
    }
}

#[test]
fn literal_source_key_examples_preserve_confirmed_database_navigation() {
    for literal in [
        r"\verb|\defbibentryset{parent}{member}|",
        r"\detokenize{\defbibentryset{parent}{member}}",
        r"\begin{verbatim}\defbibentryset{parent}{member}\end{verbatim}",
    ] {
        let (_dir, ws, main) = setup(
            "",
            &format!(r"\newrefsection[chosen]{literal}\cite{{parent}}"),
            "",
        );
        let bib = main.join("section.bib").unwrap();
        check(&ws, &bib, "crossref={", Some((&bib, "parent")));
        let doc = &ws.documents[&main];
        let at = doc
            .text
            .position(doc.text.source.rfind(r"\cite{parent}").unwrap() + 8);
        let definitions = features::definitions(&ws, &main, at);
        assert_eq!(definitions.len(), 1, "{literal}: {definitions:?}");
        assert_eq!(definitions[0].uri, bib);
    }
}

fn completed(ws: &Workspace, uri: &Url, marker: &str, key: &str) -> (CompletionItem, String) {
    let doc = &ws.documents[uri];
    let offset = doc.text.source.find(marker).unwrap() + marker.len();
    let item = features::completion(ws, uri, doc.text.position(offset))
        .into_iter()
        .find(|item| item.label == key)
        .unwrap_or_else(|| panic!("missing {key} at {marker}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("{item:?}")
    };
    let mut edits = vec![edit.clone()];
    edits.extend(item.additional_text_edits.iter().flatten().cloned());
    edits.sort_by_key(|edit| edit.range.start);
    let mut source = doc.text.source.clone();
    for edit in edits.into_iter().rev() {
        source.replace_range(
            doc.text.offset(edit.range.start).unwrap()..doc.text.offset(edit.range.end).unwrap(),
            &edit.new_text,
        );
    }
    (item, source)
}

#[test]
fn selected_relation_completion_rejects_default_only_keys_and_uses_selected_metadata() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let default = main.join("default.bib").unwrap();
    ws.open(
        default,
        "@book{defaultonly}\n@book{parent,title={Default parent}}".into(),
        8,
    );
    let bib = main.join("section.bib").unwrap();
    ws.open(
        bib.clone(),
        "@book{parent,title={Selected parent}}\n@book{childselected,crossref={parBAD},xref={def}}"
            .into(),
        9,
    );
    let (item, updated) = completed(&ws, &bib, "crossref={par", "parent");
    assert!(item.detail.unwrap().contains("Selected parent"));
    assert!(
        updated.contains("crossref={parent},xref={def}"),
        "{updated}"
    );
    let doc = &ws.documents[&bib];
    let at = doc
        .text
        .position(doc.text.source.find("xref={def").unwrap() + "xref={def".len());
    assert!(features::completion(&ws, &bib, at)
        .iter()
        .all(|item| item.label != "defaultonly"));
}

#[test]
fn relation_completion_preserves_lists_and_full_utf16_suffixes_in_all_five_fields() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let bib = main.join("section.bib").unwrap();
    for field in ["crossref", "xref", "xdata", "related", "entryset"] {
        let tail = if matches!(field, "xdata" | "related" | "entryset") {
            ",second"
        } else {
            ""
        };
        let source = format!("% 😀\r\n@book{{parent😀,title={{Selected}}}}\r\n@book{{second}}\r\n@book{{childselected,{field}={{ parBAD{tail} }}}}");
        ws.open(bib.clone(), source.clone(), 8);
        let (_, updated) = completed(&ws, &bib, &format!("{field}={{ par"), "parent😀");
        assert_eq!(updated, source.replace("parBAD", "parent😀"));
    }
}

#[test]
fn relation_completion_supports_unfinished_entries_and_values_without_union_fallback() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let bib = main.join("section.bib").unwrap();
    for value in ["{par", "\"par", "{"] {
        let source = format!(
            "@book{{parent,title={{Selected parent}}}}\n@book{{childselected,crossref={value}"
        );
        ws.open(bib.clone(), source.clone(), 8);
        let (_, updated) = completed(&ws, &bib, &format!("crossref={value}"), "parent");
        assert_eq!(
            updated,
            format!(
                "@book{{parent,title={{Selected parent}}}}\n@book{{childselected,crossref={}parent",
                &value[..1]
            )
        );
    }
}

#[test]
fn global_relation_completion_requires_one_identity_across_views_but_keeps_shared_keys() {
    let (_dir, ws, main) = setup(
        r"\addglobalbib{shared.bib}",
        r"\newrefsection[chosen]\cite{shared}",
        "@book{stable}\n@book{shared,crossref={}}\n@book{other,xref={}}",
    );
    let shared = main.join("shared.bib").unwrap();
    let doc = &ws.documents[&shared];
    let at = doc
        .text
        .position(doc.text.source.find("crossref={").unwrap() + "crossref={".len());
    let items = features::completion(&ws, &shared, at);
    assert!(items.iter().any(|item| item.label == "stable"), "{items:?}");
    assert!(
        !items.iter().any(|item| item.label == "parent"),
        "{items:?}"
    );
}

#[test]
fn selected_quote_key_completion_converts_the_complete_multiline_value_to_braces() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let bib = main.join("section.bib").unwrap();
    let source = "@book{par\"😀,title={Selected parent}}\r\n@book{second}\r\n@book{childselected,related=\"second,\r\n parBAD \"}";
    ws.open(bib.clone(), source.into(), 8);
    let (_, updated) = completed(&ws, &bib, "related=\"second,\r\n par", "par\"😀");
    assert_eq!(updated, "@book{par\"😀,title={Selected parent}}\r\n@book{second}\r\n@book{childselected,related={second,\r\n par\"😀 }}");
}

#[test]
fn unknown_and_truncated_relation_contexts_never_offer_union_completions() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let bib = main.join("section.bib").unwrap();
    let source = "@book{parent,title={Selected parent}}\n@book{childselected,crossref={par}}";
    ws.open(bib.clone(), source.into(), 8);
    ws.index_truncated.insert("test limit".into());
    let position = ws.documents[&bib]
        .text
        .position(source.rfind("par").unwrap() + 3);
    assert!(features::completion(&ws, &bib, position).is_empty());
    ws.index_truncated.clear();
    assert!(features::completion(&ws, &bib, position)
        .iter()
        .any(|item| item.label == "parent"));
    let original = ws.documents[&main].text.source.clone();
    ws.open(
        main,
        original.replace(
            r"\newrefsection[chosen]",
            r"\ifunknown\newrefsection[chosen]\fi",
        ),
        9,
    );
    assert!(features::completion(&ws, &bib, position).is_empty());
}

#[test]
fn duplicated_relation_fields_do_not_offer_effective_parent_completions() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{childselected}", "");
    let bib = main.join("section.bib").unwrap();
    let source = "@book{parent}\n@book{childselected,crossref={par},CROSSREF={parent}}";
    ws.open(bib.clone(), source.into(), 8);
    let at = ws.documents[&bib]
        .text
        .position(source.find("crossref={par").unwrap() + "crossref={par".len());
    assert!(features::completion(&ws, &bib, at).is_empty());
}
