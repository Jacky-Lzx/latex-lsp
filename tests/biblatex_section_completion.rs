use latex_lsp::{features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, String, Position) {
    let dir = tempfile::tempdir().unwrap();
    let marked = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addglobalbib{{global.bib}}\addsectionbib[label=chosen]{{section.bib}}\begin{{document}}{body}"
    );
    let at = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    for (name, content) in [
        ("main.tex", source.as_str()),
        ("default.bib", "@book{collision,title={Default},author={Default Writer},year={1984}}\n@book{default_only,title={Default only}}"),
        ("section.bib", "@book{collision,title={Selected},author={Selected Writer},year={2026}}\n@book{section_only,title={Selected only}}"),
        ("global.bib", "@book{always,title={Global}}"),
    ] { std::fs::write(dir.path().join(name), content).unwrap(); }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source.clone(), 5);
    let at = ws.documents[&main].text.position(at);
    (dir, ws, main, source, at)
}
fn apply(source: &str, item: &CompletionItem) -> String {
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("{item:?}")
    };
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
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
fn section_completion_uses_active_metadata_and_replaces_the_entire_key_suffix() {
    for (body, expected_title, expected_library) in [
        (
            r"\cite{col¦BAD}\newrefsection[chosen]\cite{section_only}",
            "Default",
            "default.bib",
        ),
        (
            r"\newrefsection[chosen]\cite{col¦BAD}",
            "Selected",
            "section.bib",
        ),
        (
            r"\newrefsection[chosen]\endrefsection\cite{col¦BAD}",
            "Default",
            "default.bib",
        ),
        (
            r"\newrefsection[chosen]\newrefsection[]\cite{col¦BAD}",
            "Default",
            "default.bib",
        ),
    ] {
        let (_dir, ws, main, source, at) = setup(body);
        let items = features::completion(&ws, &main, at);
        let item = items
            .iter()
            .find(|item| item.label == "collision")
            .unwrap_or_else(|| panic!("{body}: {items:?}"));
        let detail = item.detail.as_ref().unwrap();
        assert!(
            detail.contains(expected_title) && detail.contains(expected_library),
            "{detail}"
        );
        assert_eq!(apply(&source, item), source.replace("colBAD", "collision"));
    }
}

#[test]
fn empty_and_incomplete_keys_offer_selected_and_global_entries_without_default_entries() {
    for body in [
        r"\newrefsection[chosen]\cite{¦}",
        r"\newrefsection[chosen]\cite{¦",
        r"\newrefsection[chosen]\cite{always, ¦}",
    ] {
        let (_dir, ws, main, _, at) = setup(body);
        let items = features::completion(&ws, &main, at);
        for key in ["collision", "section_only", "always"] {
            assert!(
                items.iter().any(|item| item.label == key),
                "{body}: {items:?}"
            );
        }
        assert!(
            !items.iter().any(|item| item.label == "default_only"),
            "{body}: {items:?}"
        );
    }
}

#[test]
fn section_metadata_search_keeps_titles_authors_years_multiple_words_and_fuzzy_keys() {
    for query in ["Selected", "Writer 2026", "2026", "cllsn", "collison"] {
        let body = format!(r"\newrefsection[chosen]\cite{{{query}¦}}");
        let (_dir, ws, main, source, at) = setup(&body);
        let item = features::completion(&ws, &main, at)
            .into_iter()
            .find(|item| item.label == "collision")
            .unwrap_or_else(|| panic!("{body}"));
        assert!(item.detail.as_ref().unwrap().contains("Selected"));
        assert_eq!(
            apply(&source, &item),
            source.replace(&format!("{{{query}}}"), "{collision}")
        );
    }
}

#[test]
fn structured_single_named_and_unicode_section_completion_preserve_other_fields() {
    for (marked, completed) in [
        (
            r"\cites{always}[see]{col¦BAD}",
            r"\cites{always}[see]{collision}",
        ),
        (r"\volcite{7}{col¦BAD}", r"\volcite{7}{collision}"),
        (
            r"\entrydata*{col¦BAD}{literal}",
            r"\entrydata*{collision}{literal}",
        ),
        (r"\csuse{cite}{col¦BAD}", r"\csuse{cite}{collision}"),
        (
            r"\csname cite\endcsname{col¦BAD}",
            r"\csname cite\endcsname{collision}",
        ),
        (
            "\\entrydata % boundary\r\n*{% before\r\n col¦BAD % after\r\n}{literal}",
            "\\entrydata % boundary\r\n*{% before\r\ncollision% after\r\n}{literal}",
        ),
    ] {
        let prefix = "😀\r\n\\newrefsection[chosen]";
        let body = format!("{prefix}{marked}");
        let (_dir, ws, main, source, at) = setup(&body);
        let item = features::completion(&ws, &main, at)
            .into_iter()
            .find(|item| item.label == "collision")
            .unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(
            apply(&source, &item),
            format!(
                "{}{}",
                &source[..source.len() - marked.replace('¦', "").len()],
                completed
            )
        );
    }
}

#[test]
fn code_notes_parameters_dynamic_and_automatic_sections_do_not_offer_guessed_keys() {
    for body in [
        r"\newrefsection[chosen]\cite[col¦BAD]{always}",
        r"\newrefsection[chosen]\volcite{col¦BAD}{always}",
        r"\newrefsection[chosen]\entrydata{always}{col¦BAD}",
        r"\newcommand\run[1]{#1}\run{\newrefsection[chosen]}\cite{col¦BAD}",
        r"\newrefsection[\dynamic]\cite{col¦BAD}",
        r"\newrefsection[chosen]\iftrue\endrefsection\fi\cite{col¦BAD}",
        r"\ExecuteBibliographyOptions{refsection = section}\newrefsection[chosen]\section{Next}\cite{col¦BAD}",
        r"\newrefsection[chosen]\newcommand\later{\cite{col¦BAD}}",
    ] {
        let (_dir, ws, main, _, at) = setup(body);
        assert!(
            !features::completion(&ws, &main, at)
                .iter()
                .any(|item| item.label == "collision"),
            "{body}"
        );
    }
}

#[test]
fn selection_edits_refresh_completion_metadata_without_changing_child_versions() {
    let (dir, mut ws, main, source, _) = setup(r"\newrefsection[chosen]\input{chapter}%¦");
    let child = Url::from_file_path(dir.path().join("chapter.tex")).unwrap();
    let content = r"\cite{colBAD}";
    std::fs::write(child.to_file_path().unwrap(), content).unwrap();
    ws.open(main.clone(), source.clone(), 5);
    ws.open(child.clone(), content.into(), 3);
    let at = Position::new(0, 9);
    for (stage, selector) in ["chosen", "", "chosen"].into_iter().enumerate() {
        ws.open(
            main.clone(),
            source.replace("[chosen]", &format!("[{selector}]")),
            8 + stage as i32,
        );
        assert_eq!(ws.documents[&child].version, Some(3));
        let item = features::completion(&ws, &child, at)
            .into_iter()
            .find(|item| item.label == "collision")
            .unwrap();
        let expected = if selector.is_empty() {
            "Default"
        } else {
            "Selected"
        };
        assert!(item.detail.as_ref().unwrap().contains(expected), "{item:?}");
        assert_eq!(apply(content, &item), r"\cite{collision}");
    }
}

#[test]
fn differing_shared_consumers_and_duplicate_active_keys_do_not_offer_arbitrary_metadata() {
    let (dir, mut ws, main, source, _) = setup(r"\input{chapter}%¦");
    let child = Url::from_file_path(dir.path().join("chapter.tex")).unwrap();
    std::fs::write(child.to_file_path().unwrap(), r"\cite{colBAD}").unwrap();
    ws.open(main.clone(), source.clone(), 5);
    let other = Url::from_file_path(dir.path().join("unsaved-root.tex")).unwrap();
    ws.open(
        other.clone(),
        source.replace(r"\input{chapter}", r"\newrefsection[chosen]\input{chapter}"),
        8,
    );
    assert!(!features::completion(&ws, &child, Position::new(0, 9))
        .iter()
        .any(|item| item.label == "collision"));
    ws.open(other, source, 9);
    assert!(features::completion(&ws, &child, Position::new(0, 9))
        .iter()
        .any(|item| item.label == "collision"));

    let (_dir, mut ws, main, _, at) = setup(r"\newrefsection[chosen]\cite{¦}");
    let global = main.join("global.bib").unwrap();
    ws.open(
        global,
        "@book{always,title={Global}}\n@book{collision,title={Global collision}}".into(),
        8,
    );
    let items = features::completion(&ws, &main, at);
    assert!(
        !items.iter().any(|item| item.label == "collision"),
        "{items:?}"
    );
    assert!(items.iter().any(|item| item.label == "section_only"));
    assert!(items.iter().any(|item| item.label == "always"));
}

#[test]
#[ignore = "requires pdflatex, biblatex and Biber"]
fn real_selected_metadata_completion_compiles_using_the_selected_database() {
    let (dir, ws, main, source, at) = setup(
        r"\newrefsection[chosen]\cite{Selected 2026¦}\cite{always}\printbibliography\end{document}",
    );
    let item = features::completion(&ws, &main, at)
        .into_iter()
        .find(|item| item.label == "collision")
        .unwrap();
    assert!(item.detail.as_ref().unwrap().contains("Selected"));
    let completed = apply(&source, &item);
    assert_eq!(completed, source.replace("{Selected 2026}", "{collision}"));
    std::fs::write(dir.path().join("main.tex"), completed).unwrap();
    let latex = || {
        let output = std::process::Command::new("pdflatex")
            .args([
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "main.tex",
            ])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    };
    latex();
    let output = std::process::Command::new("biber")
        .arg("main")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    latex();
    latex();
    let bcf = std::fs::read_to_string(dir.path().join("main.bcf")).unwrap();
    let selected = bcf
        .split_once("<bcf:bibdata section=\"1\">")
        .unwrap()
        .1
        .split_once("</bcf:bibdata>")
        .unwrap()
        .0;
    assert!(selected.contains("section.bib") && selected.contains("global.bib"));
    assert!(!selected.contains("default.bib"));
    let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
    assert!(bbl.contains("Selected") && bbl.contains("Global"));
    assert!(!bbl.contains("Default"));
    let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
}

#[test]
fn section_citation_completion_keeps_label_completion_independent() {
    for command in [r"\ref{col¦BAD}", r"\csuse{ref}{col¦BAD}"] {
        let body = format!(r"\newrefsection[chosen]\label{{collision}}{command}");
        let (_dir, ws, main, source, at) = setup(&body);
        let item = features::completion(&ws, &main, at)
            .into_iter()
            .find(|item| item.label == "collision")
            .unwrap();
        assert_eq!(apply(&source, &item), source.replace("colBAD", "collision"));
    }
}
