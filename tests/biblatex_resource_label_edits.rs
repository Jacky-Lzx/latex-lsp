use latex_lsp::{features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, String, Position) {
    let dir = tempfile::tempdir().unwrap();
    let marked = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addglobalbib{{global.bib}}\addsectionbib[label=ignored,label={{chosen}}]{{section.bib}}\begin{{document}}{body}\end{{document}}"
    );
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    for (name, contents) in [
        ("main.tex", source.as_str()),
        ("section.bib", "@book{paper,title={Selected}}"),
        ("global.bib", "@book{always,title={Global}}"),
        ("ignored.bib", "@book{wrong,title={Literal filename}}"),
    ] {
        std::fs::write(dir.path().join(name), contents).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source.clone(), 5);
    let at = Text::new(source.clone()).position(offset);
    (dir, ws, main, source, at)
}
fn edits(edit: WorkspaceEdit) -> std::collections::BTreeMap<Url, Vec<TextEdit>> {
    if let Some(changes) = edit.changes {
        return changes.into_iter().collect();
    }
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("{edit:?}")
    };
    documents
        .into_iter()
        .map(|doc| {
            (
                doc.text_document.uri,
                doc.edits
                    .into_iter()
                    .map(|edit| {
                        let OneOf::Left(edit) = edit else {
                            panic!("{edit:?}")
                        };
                        edit
                    })
                    .collect(),
            )
        })
        .collect()
}
fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
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
fn resource_label_references_use_a_separate_namespace_and_include_only_the_effective_option() {
    let (_dir, ws, main, source, at) = setup(
        r"\newrefsection[ch¦osen]\begin{refsection}[chosen]\end{refsection}\label{chosen}\ref{chosen}\cite{chosen}\detokenize{\newrefsection[chosen]}",
    );
    let text = Text::new(source.clone());
    let declaration = text.range(
        source.find("{chosen}").unwrap() + 1,
        source.find("{chosen}").unwrap() + 7,
    );
    let uses: Vec<_> = source
        .match_indices("[chosen]")
        .take(2)
        .map(|(start, _)| Location::new(main.clone(), text.range(start + 1, start + 7)))
        .collect();
    assert_eq!(features::references(&ws, &main, at, false), uses);
    let all = features::references(&ws, &main, at, true);
    assert_eq!(all.len(), 3, "{all:?}");
    assert!(all.contains(&Location::new(main.clone(), declaration)));
    assert_eq!(
        features::references(&ws, &main, declaration.start, true),
        all
    );
}

#[test]
fn rename_replaces_inner_option_and_selection_ranges_without_touching_other_namespaces() {
    for versioned in [false, true] {
        let (_dir, mut ws, main, source, at) = setup("😀\r\n\\newrefsection[ch¦osen]\\newrefsection[chosen,global.bib]\\label{chosen}\\ref{chosen}\\detokenize{\\newrefsection[chosen]}");
        ws.versioned_edits = versioned;
        let prepared = features::prepare_rename(&ws, &main, at).unwrap().unwrap();
        let PrepareRenameResponse::RangeWithPlaceholder { range, placeholder } = prepared else {
            panic!("{prepared:?}")
        };
        assert_eq!(placeholder, "chosen");
        assert_eq!(
            &source[Text::new(source.clone()).offset(range.start).unwrap()
                ..Text::new(source.clone()).offset(range.end).unwrap()],
            "chosen"
        );
        let edit = features::rename(&ws, &main, at, "selected")
            .unwrap()
            .unwrap();
        let updated = apply(&source, edits(edit).remove(&main).unwrap());
        let expected = source
            .replacen("label={chosen}", "label={selected}", 1)
            .replacen("[chosen]", "[selected]", 1)
            .replacen("[chosen,", "[selected,", 1);
        assert_eq!(updated, expected);
    }
}

#[test]
fn a_new_label_must_not_capture_an_existing_literal_filename_selection() {
    let (_dir, ws, main, _, at) =
        setup(r"\newrefsection[ch¦osen]\cite{paper}\newrefsection[ignored]\cite{wrong}");
    assert!(features::prepare_rename(&ws, &main, at).unwrap().is_some());
    assert!(features::rename(&ws, &main, at, "ignored").is_err());
}

#[test]
fn existing_labels_dynamic_choices_and_unsafe_names_do_not_produce_partial_edits() {
    let (_dir, ws, main, _, at) = setup(r"\newrefsection[ch¦osen]");
    for name in ["", "a,b", "a]b", "a[b", r"\dynamic", "a b"] {
        assert!(features::rename(&ws, &main, at, name).is_err(), "{name}");
    }
    for body in [
        r"\newrefsection[ch¦osen]\newrefsection[\dynamic]",
        r"\newrefsection[ch¦osen]\def\later{\newrefsection[chosen]}",
        r"\newrefsection[ch¦osen]\iftrue\newrefsection[chosen]\fi",
    ] {
        let (_dir, ws, main, _, at) = setup(body);
        assert!(
            !matches!(features::rename(&ws, &main, at, "selected"), Ok(Some(_))),
            "{body}"
        );
    }
}

#[test]
fn all_roots_consuming_a_shared_registration_are_updated_with_document_versions() {
    let (dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]");
    let registration = r"\addsectionbib[label=ignored,label={chosen}]{section.bib}";
    std::fs::write(dir.path().join("resources.tex"), registration).unwrap();
    let source = source.replace(registration, r"\input{resources}");
    ws.open(main.clone(), source.clone(), 5);
    let other = main.join("other.tex").unwrap();
    std::fs::write(dir.path().join("other.tex"), &source).unwrap();
    ws.open(other.clone(), source.clone(), 8);
    let provider = main.join("resources.tex").unwrap();
    ws.open(provider.clone(), registration.into(), 3);
    ws.versioned_edits = true;
    let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
    let refs = features::references(&ws, &main, at, true);
    assert_eq!(refs.len(), 3, "{refs:?}");
    let edit = features::rename(&ws, &main, at, "selected")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(documents)) = &edit.document_changes else {
        panic!("{edit:?}")
    };
    for (uri, version) in [(&main, 5), (&other, 8), (&provider, 3)] {
        let document = documents
            .iter()
            .find(|document| document.text_document.uri == *uri)
            .unwrap();
        assert_eq!(document.text_document.version, Some(version));
        assert_eq!(document.edits.len(), 1);
    }
    assert_eq!(edits(edit).len(), 3);
}

#[test]
fn shared_selection_consumers_must_agree_on_declaration_and_filename_meaning() {
    for literal in [false, true] {
        let (dir, mut ws, main, source, _) = setup(r"\input{shared}¦");
        std::fs::write(dir.path().join("shared.tex"), r"\newrefsection[chosen]").unwrap();
        std::fs::write(
            dir.path().join("chosen.bib"),
            "@book{paper,title={Literal}}",
        )
        .unwrap();
        ws.open(main.clone(), source.clone(), 5);
        let at = Text::new(source.clone()).position(source.find("{chosen}").unwrap() + 2);
        assert!(features::rename(&ws, &main, at, "selected")
            .unwrap()
            .is_some());
        let other = main.join("unsaved-root.tex").unwrap();
        let other_source = if literal {
            source.replace(
                r"\addsectionbib[label=ignored,label={chosen}]{section.bib}",
                "",
            )
        } else {
            source.replace("{section.bib}", "{global.bib}")
        };
        ws.open(other.clone(), other_source, 8);
        assert!(!matches!(
            features::prepare_rename(&ws, &main, at),
            Ok(Some(_))
        ));
        assert!(
            !matches!(features::rename(&ws, &main, at, "selected"), Ok(Some(_))),
            "literal={literal}"
        );
        ws.documents.remove(&other);
        ws.open(main.clone(), source, 5);
        assert!(features::rename(&ws, &main, at, "selected")
            .unwrap()
            .is_some());
    }
}

#[test]
fn independent_resource_labels_with_the_same_spelling_remain_separate_symbols() {
    let (dir, mut ws, main, source, at) = setup(r"\newrefsection[ch¦osen]");
    let other = main.join("other.tex").unwrap();
    std::fs::write(dir.path().join("other.tex"), &source).unwrap();
    ws.open(other, source.clone(), 8);
    let edit = features::rename(&ws, &main, at, "selected")
        .unwrap()
        .unwrap();
    let changes = edits(edit);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[&main].len(), 2);
}

#[test]
fn effective_existing_resource_labels_block_rename_collisions() {
    let (_dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]");
    let source = source.replace(
        r"\begin{document}",
        r"\addbibresource[label=taken]{global.bib}\begin{document}",
    );
    ws.open(main.clone(), source.clone(), 6);
    let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
    assert!(features::rename(&ws, &main, at, "taken").is_err());
}

#[test]
fn imported_named_and_forwarded_resource_uses_are_complete_rename_occurrences() {
    for call in [
        r"\choose",
        r"\csuse{choose}",
        r"\csname choose\endcsname",
        r"\begin{refsection}",
        r"\csuse{begin}{refsection}",
    ] {
        let body = format!(r"\input{{reader}}{call}[ch¦osen]");
        let (dir, mut ws, main, source, _) = setup(&body);
        let reader = main.join("reader.tex").unwrap();
        std::fs::write(
            dir.path().join("reader.tex"),
            r"\def\choose{\newrefsection}",
        )
        .unwrap();
        ws.open(main.clone(), source.clone(), 5);
        let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
        let edit = features::rename(&ws, &main, at, "selected")
            .unwrap()
            .unwrap();
        let changes = edits(edit);
        assert_eq!(changes[&main].len(), 2, "{call}");
        ws.open(reader, r"\def\choose#1{}".into(), 3);
        if call.contains("choose") {
            assert_eq!(ws.documents[&main].version, Some(5));
            assert!(
                !matches!(features::rename(&ws, &main, at, "selected"), Ok(Some(_))),
                "{call}"
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex, biber and biblatex"]
fn real_label_renames_preserve_resources_and_reject_a_capture_that_breaks_citations() {
    fn compile(root: &std::path::Path, source: &str) -> (Vec<String>, String, String) {
        std::fs::write(root.join("main.tex"), source).unwrap();
        let latex = || {
            let result = std::process::Command::new("pdflatex")
                .args([
                    "-no-shell-escape",
                    "-interaction=nonstopmode",
                    "-halt-on-error",
                    "main.tex",
                ])
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
        };
        latex();
        let bcf = std::fs::read_to_string(root.join("main.bcf")).unwrap();
        let resources = bcf
            .split("<bcf:bibdata ")
            .skip(1)
            .map(|part| part.split_once("</bcf:bibdata>").unwrap().0.to_string())
            .collect();
        let biber = std::process::Command::new("biber")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            biber.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&biber.stdout),
            String::from_utf8_lossy(&biber.stderr)
        );
        latex();
        latex();
        (
            resources,
            std::fs::read_to_string(root.join("main.bbl")).unwrap(),
            std::fs::read_to_string(root.join("main.log")).unwrap(),
        )
    }
    let (dir, ws, main, source, at) = setup(
        r"\newrefsection[ch¦osen]\cite{paper,always}\printbibliography\newrefsection[ignored]\cite{wrong,always}\printbibliography",
    );
    let (original, bbl, log) = compile(dir.path(), &source);
    assert!(!log.contains("undefined"), "{log}");
    assert!(
        bbl.contains("Selected") && bbl.contains("Literal filename") && bbl.contains("Global"),
        "{bbl}"
    );
    assert!(features::rename(&ws, &main, at, "ignored").is_err());
    let naive = source
        .replacen("label={chosen}", "label={ignored}", 1)
        .replacen("[chosen]", "[ignored]", 1);
    let (captured, _, log) = compile(dir.path(), &naive);
    assert_ne!(original, captured);
    assert!(log.contains("undefined"), "{log}");
    let edit = features::rename(&ws, &main, at, "selected")
        .unwrap()
        .unwrap();
    let accepted = apply(&source, edits(edit).remove(&main).unwrap());
    let (renamed, bbl, log) = compile(dir.path(), &accepted);
    assert_eq!(original, renamed);
    assert!(!log.contains("undefined"), "{log}");
    assert!(
        bbl.contains("Selected") && bbl.contains("Literal filename") && bbl.contains("Global"),
        "{bbl}"
    );
}

#[test]
fn capture_in_another_consumer_and_shared_repeated_uses_are_checked_as_one_symbol() {
    let (dir, mut ws, main, source, _) = setup(r"\input{shared}\input{shared}¦");
    let registration = r"\addsectionbib[label=ignored,label={chosen}]{section.bib}";
    std::fs::write(dir.path().join("resources.tex"), registration).unwrap();
    std::fs::write(dir.path().join("shared.tex"), r"\newrefsection[chosen]").unwrap();
    let source = source.replace(registration, r"\input{resources}");
    ws.open(main.clone(), source.clone(), 5);
    let provider = main.join("resources.tex").unwrap();
    let at = ws.documents[&provider]
        .text
        .position(registration.find("chosen").unwrap() + 2);
    assert_eq!(features::references(&ws, &provider, at, true).len(), 2);
    let other = main.join("other.tex").unwrap();
    let other_source = source.replace(r"\end{document}", r"\newrefsection[ignored]\end{document}");
    std::fs::write(dir.path().join("other.tex"), &other_source).unwrap();
    ws.open(other, other_source, 8);
    assert!(features::rename(&ws, &provider, at, "ignored").is_err());
    let edit = features::rename(&ws, &provider, at, "selected")
        .unwrap()
        .unwrap();
    let changes = edits(edit);
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[&provider].len(), 1);
    assert_eq!(changes[&main.join("shared.tex").unwrap()].len(), 1);
}

#[test]
fn different_resource_labels_of_one_database_keep_distinct_declaration_identities() {
    let (_dir, mut ws, main, source, _) = setup(r"\newrefsection[ch¦osen]\newrefsection[other]");
    let source = source.replace(
        r"\begin{document}",
        r"\addsectionbib[label=other]{section.bib}\begin{document}",
    );
    ws.open(main.clone(), source.clone(), 6);
    let at = Text::new(source.clone()).position(source.rfind("chosen]").unwrap() + 2);
    let edit = features::rename(&ws, &main, at, "selected")
        .unwrap()
        .unwrap();
    let updated = apply(&source, edits(edit).remove(&main).unwrap());
    assert_eq!(
        updated,
        source
            .replacen("{chosen}", "{selected}", 1)
            .replacen("[chosen]", "[selected]", 1)
    );
    assert!(features::rename(&ws, &main, at, "other").is_err());
}

#[test]
fn resource_label_highlights_distinguish_the_declaration_from_uses_and_other_namespaces() {
    let (_dir, ws, main, source, at) =
        setup(r"\newrefsection[ch¦osen]\newrefsection[chosen]\label{chosen}\ref{chosen}");
    let start = source.find("{chosen}").unwrap() + 1;
    let declaration = Text::new(source).range(start, start + 6);
    let highlights = latex_lsp::editing::highlights(&ws, &main, at);
    assert_eq!(highlights.len(), 3);
    assert_eq!(
        highlights
            .iter()
            .filter(|highlight| highlight.kind == Some(DocumentHighlightKind::WRITE))
            .count(),
        1
    );
    assert!(highlights
        .iter()
        .any(|highlight| highlight.range == declaration
            && highlight.kind == Some(DocumentHighlightKind::WRITE)));
    assert_eq!(
        highlights
            .iter()
            .filter(|highlight| highlight.kind == Some(DocumentHighlightKind::READ))
            .count(),
        2
    );
}
