use latex_lsp::{editing, features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;
fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:categories.tex").unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), source.into(), 5);
    (ws, uri)
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
fn category_names_navigate_highlight_and_rename_without_changing_citation_keys() {
    let source = "😀 \\label{selected}\\bibitem{selected}\\def\\selected{}\r\n\\DeclareBibliographyCategory{selected}\r\n\\addtocategory{selected}{selected}\r\n\\ifentrycategory{selected}{selected}{}{}\\ifcategory{selected}{}{}\r\n\\printbibliography[category={selected},notcategory=selected,title={selected}]\r\n\\defbibfilter{selected}{category=selected or (type=book and not category=selected)}";
    let (ws, uri) = setup(source);
    let text = Text::new(source.into());
    let position = text.position(source.rfind("selected").unwrap() + 2);
    let definitions = features::definitions(&ws, &uri, position);
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].range.start.line, 1);
    assert!(format!("{:?}", features::hover(&ws, &uri, position)).contains("biblatex category"));
    assert_eq!(features::references(&ws, &uri, position, true).len(), 8);
    assert_eq!(features::references(&ws, &uri, position, false).len(), 7);
    assert_eq!(editing::highlights(&ws, &uri, position).len(), 8);
    assert!(features::prepare_rename(&ws, &uri, position)
        .unwrap()
        .is_some());
    let edit = features::rename(&ws, &uri, position, "updated")
        .unwrap()
        .unwrap();
    let edits = &edit.changes.as_ref().unwrap()[&uri];
    assert_eq!(edits.len(), 8);
    let updated = apply(source, edits.clone());
    assert_eq!(updated.matches("updated").count(), 8);
    for preserved in [
        r"\label{selected}",
        r"\bibitem{selected}",
        r"\def\selected{}",
        r"\addtocategory{updated}{selected}",
        r"\ifentrycategory{selected}{updated}",
        r"title={selected}",
        r"\defbibfilter{selected}",
    ] {
        assert!(updated.contains(preserved), "{updated}");
    }
}
#[test]
fn category_completion_preserves_lists_notes_suffixes_comments_and_unfinished_keys() {
    let prefix = "😀 \\DeclareBibliographyCategory{selected}\r\n";
    for (marked, expected) in [
        (
            r"\addtocategory{se¦BAD}{paper}",
            r"\addtocategory{selected}{paper}",
        ),
        (
            r"\ifentrycategory{paper}{se¦BAD}{}{body}",
            r"\ifentrycategory{paper}{selected}{}{body}",
        ),
        (
            r"\ifcategory{se¦BAD}{}{body}",
            r"\ifcategory{selected}{}{body}",
        ),
        (
            r"\printbibliography[category={se¦BAD},title={Title}]",
            r"\printbibliography[category={selected},title={Title}]",
        ),
        (
            r"\printbibliography[notcategory=se¦BAD,heading=none]",
            r"\printbibliography[notcategory=selected,heading=none]",
        ),
        (
            r"\defbibfilter{mine}{type=book and category=se¦BAD}",
            r"\defbibfilter{mine}{type=book and category=selected}",
        ),
        (r"\addtocategory{se¦", r"\addtocategory{selected"),
        (
            r"\printbibliography[category=se¦",
            r"\printbibliography[category=selected",
        ),
        (
            "\\addtocategory{se¦\n\\end{document}",
            "\\addtocategory{selected\n\\end{document}",
        ),
    ] {
        let at = prefix.len() + marked.find('¦').unwrap();
        let source = format!("{prefix}{}", marked.replace('¦', ""));
        let (ws, uri) = setup(&source);
        let items = features::completion(&ws, &uri, Text::new(source.clone()).position(at));
        let item = items
            .iter()
            .find(|i| i.label == "selected")
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("{item:?}")
        };
        let mut edits = vec![edit.clone()];
        edits.extend(item.additional_text_edits.clone().unwrap_or_default());
        assert_eq!(apply(&source, edits), format!("{prefix}{expected}"));
    }
}
#[test]
fn category_completions_do_not_appear_in_entry_keys_titles_filter_names_or_dynamic_values() {
    for marked in [
        r"\addtocategory{selected}{se¦}",
        r"\ifentrycategory{se¦}{selected}{}{}",
        r"\printbibliography[title={se¦}]",
        r"\defbibfilter{se¦}{category=selected}",
        r"\DeclareBibliographyCategory{se¦}",
        r"\addtocategory{se¦\dynamic}{paper}",
        r"\def\take#1{}\take{\addtocategory{se¦}{paper}}",
        r"\def\addtocategory#1#2{}\addtocategory{se¦}{paper}",
    ] {
        let marked = format!("\\DeclareBibliographyCategory{{selected}}\r\n{marked}");
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (ws, uri) = setup(&source);
        assert!(
            !features::completion(&ws, &uri, Text::new(source).position(at))
                .iter()
                .any(|i| i.label == "selected"),
            "{marked}"
        );
    }
}
#[test]
fn category_names_support_static_aliases_named_calls_and_complete_macro_bodies() {
    for call in [
        r"\csuse{addtocategory}{selected}{paper}",
        r"\csname addtocategory\endcsname{selected}{paper}",
        r"\def\assign{\addtocategory}\assign{selected}{paper}",
        r"\def\assign{\addtocategory}\def\next{\assign}\csuse{next}{selected}{paper}",
        r"\newcommand{\later}{\addtocategory{selected}{paper}}",
    ] {
        let source = format!("\\DeclareBibliographyCategory{{selected}}{call}");
        let (ws, uri) = setup(&source);
        let pos = Text::new(source.clone()).position(source.rfind("selected").unwrap() + 2);
        assert_eq!(features::definitions(&ws, &uri, pos).len(), 1, "{source}");
        let edit = features::rename(&ws, &uri, pos, "updated")
            .unwrap_or_else(|error| panic!("{source}: {error:?}"))
            .unwrap();
        assert_eq!(edit.changes.as_ref().unwrap()[&uri].len(), 2, "{source}");
    }
}
#[test]
fn category_operands_and_dynamic_names_prevent_partial_renames_but_literals_remain_unchanged() {
    for call in [
        r"\noexpand\addtocategory{selected}{paper}",
        r"\def\take#1{}\take{\addtocategory{selected}{paper}}",
        r"\addtocategory{\dynamic}{paper}",
        r"\printbibliography[category=\dynamic]",
        r"\defbibfilter{mine}{category=selected and \dynamic}",
        r"\printbibliography[\dynamic]",
        r"\def\assign{\addtocategory}\def\assign{}\assign{selected}{paper}",
        r"\let\assign=\addtocategory\assign{selected}{paper}",
    ] {
        let source = format!("\\DeclareBibliographyCategory{{selected}}{call}");
        let (ws, uri) = setup(&source);
        assert!(
            features::rename(&ws, &uri, Position::new(0, 32), "updated").is_err(),
            "{source}"
        );
    }
    let source =
        r"\DeclareBibliographyCategory{selected}\detokenize{\addtocategory{selected}{paper}}";
    let (ws, uri) = setup(source);
    let edit = features::rename(&ws, &uri, Position::new(0, 32), "updated")
        .unwrap()
        .unwrap();
    let edits = &edit.changes.as_ref().unwrap()[&uri];
    assert_eq!(edits.len(), 1);
    assert!(apply(source, edits.clone()).ends_with(r"\detokenize{\addtocategory{selected}{paper}}"));
}
#[test]
fn comments_after_protected_option_values_stay_outside_category_edits() {
    let source = "\\DeclareBibliographyCategory{selected}\\printbibliography[category={selected}% keep\n,title={Title}]";
    let (ws, uri) = setup(source);
    let at = source.rfind("selected").unwrap();
    let position = Text::new(source.into()).position(at + 2);
    let edit = features::rename(&ws, &uri, position, "updated")
        .unwrap()
        .unwrap();
    let updated = apply(source, edit.changes.unwrap().remove(&uri).unwrap());
    assert!(
        updated.contains("category={updated}% keep\n,title={Title}"),
        "{updated}"
    );
}

#[test]
fn category_renames_require_unique_declarations_valid_names_and_unoccupied_targets() {
    for suffix in [
        r"\DeclareBibliographyCategory{selected}",
        r"\DeclareBibliographyCategory{updated}",
        r"\addtocategory{updated}{paper}",
    ] {
        let source = format!("\\DeclareBibliographyCategory{{selected}}{suffix}");
        let (ws, uri) = setup(&source);
        assert!(
            features::rename(&ws, &uri, Position::new(0, 32), "updated").is_err(),
            "{source}"
        );
    }
    let (mut ws, uri) =
        setup(r"\DeclareBibliographyCategory{selected}\addtocategory{selected}{paper}");
    for name in ["", "two words", "other,key", "bad]", r"\dynamic"] {
        assert!(
            features::rename(&ws, &uri, Position::new(0, 32), name).is_err(),
            "{name}"
        );
    }
    ws.index_truncated.insert("maxFiles".into());
    assert!(features::rename(&ws, &uri, Position::new(0, 32), "updated")
        .unwrap()
        .is_none());
}
#[test]
fn category_options_preserve_contiguous_commented_names_and_reject_spliced_names() {
    let source="\\DeclareBibliographyCategory{selected}\r\n\\printbibliography[\r\n% comment\r\ncategory = {% before\r\nselected% after\r\n},title={Book}]";
    let (ws, uri) = setup(source);
    let position = Text::new(source.into()).position(source.rfind("selected").unwrap() + 2);
    let edit = features::rename(&ws, &uri, position, "updated")
        .unwrap()
        .unwrap();
    let updated = apply(source, edit.changes.unwrap().remove(&uri).unwrap());
    assert!(updated.contains("updated% after\r\n"));
    assert!(updated.contains("% before\r\n"));
    let source =
        "\\DeclareBibliographyCategory{selected}\\addtocategory{sel% comment\nected}{paper}";
    let (ws, uri) = setup(source);
    assert!(features::rename(&ws, &uri, Position::new(0, 32), "updated").is_err());
}
#[test]
fn shared_categories_refresh_unsaved_meanings_and_keep_other_projects_isolated() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "a.tex",
            r"\documentclass{article}\input{common}\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\def\addtocategory#1#2{}\input{common}\input{shared}",
        ),
        ("common.tex", r"\DeclareBibliographyCategory{selected}"),
        ("shared.tex", r"\addtocategory{selected}{paper}"),
        (
            "independent.tex",
            r"\documentclass{article}\DeclareBibliographyCategory{selected}",
        ),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let common = Url::from_file_path(dir.path().join("common.tex")).unwrap();
    let shared = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    assert!(features::definitions(&ws, &shared, Position::new(0, 18)).is_empty());
    assert!(features::rename(&ws, &common, Position::new(0, 32), "updated").is_err());
    ws.open(
        b,
        r"\documentclass{article}\input{common}\input{shared}".into(),
        2,
    );
    assert_eq!(
        features::definitions(&ws, &shared, Position::new(0, 18)).len(),
        1
    );
    let changes = features::rename(&ws, &common, Position::new(0, 32), "updated")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[&common].len(), 1);
    assert_eq!(changes[&shared].len(), 1);
}
#[test]
fn categories_shared_with_a_missing_or_different_declaration_reject_renames() {
    for declaration in ["", r"\DeclareBibliographyCategory{selected}"] {
        let dir = tempfile::tempdir().unwrap();
        for (name, source) in [
            (
                "a.tex",
                r"\documentclass{article}\DeclareBibliographyCategory{selected}\input{shared}"
                    .to_string(),
            ),
            (
                "b.tex",
                format!("\\documentclass{{article}}{declaration}\\input{{shared}}"),
            ),
            ("shared.tex", r"\addtocategory{selected}{paper}".into()),
        ] {
            std::fs::write(dir.path().join(name), source).unwrap();
        }
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        ws.add_root(dir.path().into());
        let a = Url::from_file_path(dir.path().join("a.tex")).unwrap();
        let source = &ws.documents[&a].text.source;
        let pos = Text::new(source.clone()).position(source.find("selected").unwrap() + 2);
        assert!(features::rename(&ws, &a, pos, "updated").is_err());
    }
}

#[test]
#[ignore = "requires pdflatex, BibTeX and biblatex"]
fn real_biblatex_category_rename_preserves_membership_and_filtered_output() {
    let dir = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let source = "\\documentclass{article}\n\\usepackage[backend=bibtex,style=numeric]{biblatex}\n\\addbibresource{refs.bib}\n\\DeclareBibliographyCategory{selected}\n\\def\\assign{\\addtocategory}\n\\assign{selected}{paper}\n\\defbibfilter{mine}{category=selected}\n\\AtEveryBibitem{\\ifcategory{selected}{\\typeout{CURRENT-CATEGORY-OK}}{\\errmessage{Current category lost}}}\n\\begin{document}\n\\nocite{paper}\n\\ifentrycategory{paper}{selected}{\\typeout{CATEGORY-MEMBERSHIP-OK}}{\\errmessage{Category lost}}\n\\printbibliography[category=selected]\n\\printbibliography[filter=mine]\n\\end{document}\n";
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), source.into(), 1);
    let position = Text::new(source.into()).position(source.rfind("selected").unwrap() + 2);
    let edit = features::rename(&ws, &uri, position, "updated")
        .unwrap()
        .unwrap();
    let updated = apply(source, edit.changes.unwrap().remove(&uri).unwrap());
    assert_eq!(updated.matches("selected").count(), 0, "{updated}");
    std::fs::write(uri.to_file_path().unwrap(), updated).unwrap();
    std::fs::write(
        dir.path().join("refs.bib"),
        "@book{paper,author={Doe, Jane},title={Category book},year={2026},publisher={Example}}",
    )
    .unwrap();
    for program in ["pdflatex", "bibtex", "pdflatex", "pdflatex"] {
        let mut command = std::process::Command::new(program);
        if program == "pdflatex" {
            command.args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ]);
        } else {
            command.arg("main");
        }
        let output = command.current_dir(dir.path()).output().unwrap();
        assert!(
            output.status.success(),
            "{program}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
    assert!(log.contains("CATEGORY-MEMBERSHIP-OK"), "{log}");
    assert!(log.contains("CURRENT-CATEGORY-OK"), "{log}");
    assert!(!log.contains("Empty bibliography"), "{log}");
}

#[test]
fn category_analysis_limits_never_return_partial_renames() {
    let source = format!(
        "\\DeclareBibliographyCategory{{selected}}{}",
        r"\addtocategory{selected}{paper}".repeat(20_001)
    );
    let (ws, uri) = setup(&source);
    assert!(!matches!(
        features::rename(&ws, &uri, Position::new(0, 32), "updated"),
        Ok(Some(_))
    ));
    let source = format!(
        "\\DeclareBibliographyCategory{{selected}}\\defbibfilter{{mine}}{{{}}}",
        "not ".repeat(100_001)
    );
    let (ws, uri) = setup(&source);
    assert!(features::rename(&ws, &uri, Position::new(0, 32), "updated").is_err());
}

#[test]
fn unused_command_copies_do_not_hide_other_category_references() {
    let source = r"\DeclareBibliographyCategory{selected}\let\assign=\addtocategory\addtocategory{selected}{paper}";
    let (ws, uri) = setup(source);
    let edit = features::rename(&ws, &uri, Position::new(0, 32), "updated")
        .unwrap()
        .unwrap();
    assert_eq!(edit.changes.as_ref().unwrap()[&uri].len(), 2);
}
