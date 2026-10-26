#[path = "support/completion.rs"]
mod completion;
use lsp_types::*;
use texquill::{language::features, project::workspace::Workspace, text::Text};
fn setup(marked: &str) -> (tempfile::TempDir, Workspace, Url, Position) {
    let dir = tempfile::tempdir().unwrap();
    let marked = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}\begin{{document}}{marked}"
    );
    let at = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    for (name, text) in [
        ("main.tex", source.as_str()),
        (
            "default.bib",
            "@book{parent,title={Default parent}} @book{member,title={Default member}}",
        ),
        (
            "section.bib",
            "@book{parent,title={Selected parent}} @book{member,title={Selected member}}",
        ),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    let at = ws.documents[&main].text.position(at);
    (dir, ws, main, at)
}
fn item(ws: &Workspace, uri: &Url, at: Position, name: &str) -> CompletionItem {
    let items = features::completion(ws, uri, at);
    items
        .iter()
        .find(|item| item.label == name)
        .unwrap_or_else(|| panic!("{name}: {items:?}"))
        .clone()
}
fn applied(source: &str, item: &CompletionItem) -> String {
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
fn source_candidates_preview_numeric_identity_and_replace_the_full_key_suffix() {
    for (body, detail) in [
        (
            r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{pa¦BAD}",
            "Dynamic bibliography set: member",
        ),
        (
            r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}\newrefsection[chosen]\cite{pa¦BAD}",
            "Selected parent",
        ),
        (
            r"\cite{pa¦BAD}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{parent}",
            "Default parent",
        ),
    ] {
        let (_dir, ws, main, at) = setup(body);
        let candidate = item(&ws, &main, at, "parent");
        assert!(
            completion::preview(&candidate).contains(detail),
            "{candidate:?}"
        );
        let text = &ws.documents[&main].text.source;
        assert_eq!(applied(text, &candidate), text.replace("paBAD", "parent"));
    }
}
#[test]
fn accepting_an_early_partial_request_previews_the_book_it_will_lock() {
    for body in [
        r"\newrefsection[chosen]\cite{pa¦}\defbibentryset{parent}{member}\cite{parent}",
        r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{member}\cite{pa¦}",
    ] {
        let (_dir, ws, main, at) = setup(body);
        let candidate = item(&ws, &main, at, "parent");
        assert!(
            completion::preview(&candidate).contains("Selected parent"),
            "{candidate:?}"
        );
        assert!(!completion::preview(&candidate).contains("Dynamic"));
    }
}
#[test]
fn source_only_sets_are_valid_tex_candidates_without_database_parent_entries() {
    let (_dir, ws, main, at) =
        setup(r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{col¦BAD}");
    let candidate = item(&ws, &main, at, "collected");
    assert!(completion::preview(&candidate).contains("Dynamic bibliography set: member"));
}
#[test]
fn source_member_completion_uses_its_numeric_database_and_excludes_nested_sets() {
    let (_dir, ws, main, at) = setup(
        r"\newrefsection[chosen]\defbibentryset{other}{parent}\defbibentryset{collected}{¦}\cite{collected}",
    );
    let candidate = item(&ws, &main, at, "member");
    assert!(completion::preview(&candidate).contains("Selected member"));
    let items = features::completion(&ws, &main, at);
    for blocked in ["other", "collected"] {
        assert!(!items.iter().any(|item| item.label == blocked), "{items:?}");
    }
}
#[test]
fn static_named_and_structured_source_citation_completions_keep_unicode_ranges() {
    for call in [
        r"\cite*[see][p.~2]{pa¦BAD}",
        r"\csuse{cite}{pa¦BAD}",
        r"\csname cite\endcsname{pa¦BAD}",
        r"\cites[see]{member}[also][p.~2]{pa¦BAD}",
    ] {
        let body = format!(
            "% 😀\r\n\\newrefsection[chosen]\\csuse{{defbibentryset}}{{parent}}{{member}}{call}"
        );
        let (_dir, ws, main, at) = setup(&body);
        let candidate = item(&ws, &main, at, "parent");
        let text = &ws.documents[&main].text.source;
        assert_eq!(applied(text, &candidate), text.replace("paBAD", "parent"));
        assert!(completion::preview(&candidate).contains("Dynamic bibliography set"));
    }
}
#[test]
fn source_completion_retains_metadata_search_and_incomplete_literal_fields() {
    for query in ["Dynamic", "member", ""] {
        let body =
            format!(r"\newrefsection[chosen]\defbibentryset{{parent}}{{member}}\cite{{{query}¦}}");
        let (_dir, ws, main, at) = setup(&body);
        let candidate = item(&ws, &main, at, "parent");
        let text = &ws.documents[&main].text.source;
        assert_eq!(
            applied(text, &candidate),
            text.replace(&format!(r"\cite{{{query}}}"), r"\cite{parent}")
        );
    }
    let (_dir, ws, main, at) =
        setup(r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{¦");
    assert_eq!(item(&ws, &main, at, "parent").label, "parent");
}
#[test]
fn shared_unsaved_consumers_remove_only_conflicting_candidates_and_close_restores_them() {
    let (dir, mut ws, main, _at) =
        setup(r"\newrefsection[chosen]\defbibentryset{parent}{member}\input{chapter}¦");
    let chapter = main.join("chapter.tex").unwrap();
    let text = r"\cite{}";
    std::fs::write(dir.path().join("chapter.tex"), text).unwrap();
    ws.open(chapter.clone(), text.into(), 3);
    let at = Position::new(0, 6);
    assert!(item(&ws, &chapter, at, "parent")
        .detail
        .unwrap()
        .contains("Dynamic"));
    let other = main.join("unsaved.tex").unwrap();
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\input{chapter}\end{document}".into(),9);
    let items = features::completion(&ws, &chapter, at);
    assert!(
        !items.iter().any(|item| item.label == "parent"),
        "{items:?}"
    );
    assert!(items.iter().any(|item| item.label == "member"), "{items:?}");
    ws.documents.remove(&other);
    assert!(item(&ws, &chapter, at, "parent")
        .detail
        .unwrap()
        .contains("Dynamic"));
}
#[test]
fn source_completion_keeps_ids_preambles_dynamic_execution_and_truncation_guards() {
    for tail in [
        r"\newcommand\later{\cite{parent}}",
        r"\iftrue\cite{parent}\fi",
    ] {
        let body =
            format!(r"\newrefsection[chosen]\defbibentryset{{parent}}{{member}}\cite{{pa¦}}{tail}");
        let (_dir, ws, main, at) = setup(&body);
        assert!(features::completion(&ws, &main, at).is_empty());
    }
    let (_dir, mut ws, main, at) =
        setup(r"\newrefsection[chosen]\defbibentryset{parent}{member}\cite{pa¦}");
    for text in [
        "@book{other,ids={parent}} @book{member}",
        "@book{parent} @book{member} @preamble{\"code\"}",
    ] {
        ws.open(main.join("section.bib").unwrap(), text.into(), 8);
        assert!(features::completion(&ws, &main, at).is_empty());
    }
    ws.open(
        main.join("section.bib").unwrap(),
        "@book{parent,title={Parent}} @book{member,title={Member}}".into(),
        9,
    );
    ws.index_truncated.insert("maxFiles".into());
    assert!(features::completion(&ws, &main, at).is_empty());
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
#[test]
fn an_accepted_first_query_locks_every_repeated_execution_in_the_same_numeric_section() {
    let (dir, mut ws, main, _) = setup(
        r"\newrefsection[chosen]\input{chapter}\defbibentryset{parent}{member}\input{chapter}¦",
    );
    let chapter = main.join("chapter.tex").unwrap();
    let text = r"\cite{paBAD}";
    std::fs::write(dir.path().join("chapter.tex"), text).unwrap();
    ws.open(chapter.clone(), text.into(), 3);
    let candidate = item(&ws, &chapter, Position::new(0, 8), "parent");
    assert!(completion::preview(&candidate).contains("Selected parent"));
}
#[test]
fn an_early_completion_with_wildcard_loading_does_not_certify_a_source_or_book_mix() {
    let (_dir, ws, main, at) =
        setup(r"\newrefsection[chosen]\cite{¦}\defbibentryset{parent}{member}\nocite{*}");
    let items = features::completion(&ws, &main, at);
    assert!(
        !items.iter().any(|item| item.label == "parent"),
        "{items:?}"
    );
    assert!(items.iter().any(|item| item.label == "member"), "{items:?}");
}
#[test]
fn source_only_keys_before_their_declaration_are_not_previewed_as_later_sets() {
    let (_dir, ws, main, at) =
        setup(r"\newrefsection[chosen]\cite{col¦}\defbibentryset{collected}{member}");
    assert!(!features::completion(&ws, &main, at)
        .iter()
        .any(|item| item.label == "collected"));
}
#[test]
fn numeric_probe_catalogs_share_large_resource_scans_within_the_budget() {
    let body = format!(
        "{}¦",
        r"\newrefsection[chosen]\input{sets}\input{chapter}".repeat(350)
    );
    let (dir, mut ws, main, _) = setup(&body);
    let chapter = main.join("chapter.tex").unwrap();
    let sets = main.join("sets.tex").unwrap();
    for (name, text) in [
        ("sets.tex", r"\defbibentryset{parent}{member}"),
        ("chapter.tex", r"\cite{}"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    ws.open(sets, r"\defbibentryset{parent}{member}".into(), 3);
    ws.open(chapter.clone(), r"\cite{}".into(), 4);
    let mut library = "@book{parent,title={Parent}} @book{member,title={Member}}\n".to_owned();
    for index in 0..1500 {
        library.push_str(&format!("@book{{child{index},title={{Child {index}}}}}\n"));
    }
    ws.open(main.join("section.bib").unwrap(), library, 9);
    ws.config.indexing.completion_limit = 2000;
    let items = features::completion(&ws, &chapter, Position::new(0, 6));
    assert_eq!(items.len(), 1502);
    assert!(items
        .iter()
        .find(|item| item.label == "parent")
        .unwrap()
        .detail
        .as_ref()
        .unwrap()
        .contains("Dynamic bibliography set"));
}
#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn accepted_source_citation_and_member_completions_match_native_first_request_identities() {
    for (body, key_name, kind) in [
        (
            r"\cite{parent}\newrefsection[chosen]\cite{child}\defbibentryset{parent}{member}\cite{pa¦BAD}",
            "parent",
            "set",
        ),
        (
            r"\cite{parent}\newrefsection[chosen]\defbibentryset{parent}{member}\cite{pa¦BAD}",
            "parent",
            "set",
        ),
        (
            r"\cite{parent}\newrefsection[chosen]\cite{pa¦BAD}\defbibentryset{parent}{member}\cite{parent}",
            "parent",
            "book",
        ),
        (
            r"\cite{parent}\newrefsection[chosen]\defbibentryset{collected}{member}\cite{col¦BAD}",
            "collected",
            "set",
        ),
        (
            r"\cite{parent}\newrefsection[chosen]\defbibentryset{collected}{me¦BAD}\cite{collected}",
            "member",
            "book",
        ),
    ] {
        let (dir, mut ws, main, at) = setup(body);
        if body.contains(r"\cite{child}") {
            let bib = main.join("section.bib").unwrap();
            let library = ws.documents[&bib].text.source.clone()
                + " @inbook{child,title={Child},crossref={parent}}";
            std::fs::write(dir.path().join("section.bib"), &library).unwrap();
            ws.open(bib, library, 9);
        }
        let candidate = item(&ws, &main, at, key_name);
        let source = format!(
            "{}\\printbibliography\\end{{document}}",
            applied(&ws.documents[&main].text.source, &candidate)
        );
        std::fs::write(dir.path().join("main.tex"), source).unwrap();
        let bbl = compile(dir.path());
        assert!(section(&bbl, 0).contains(r"\entry{parent}{book}"), "{bbl}");
        assert!(
            entries(section(&bbl, 1))[key_name].starts_with(&format!("{{{kind}}}")),
            "{bbl}"
        );
        if key_name == "member" {
            assert!(section(&bbl, 1).contains(r"\set{member}"), "{bbl}");
        }
    }
}

#[test]
fn preamble_member_queries_keep_the_default_resource_namespace() {
    let (_dir, mut ws, main, _) = setup(r"\newrefsection[chosen]\cite{member}¦");
    let source = ws.documents[&main].text.source.replace(
        r"\begin{document}",
        r"\defbibentryset{collected}{memBAD}\begin{document}",
    );
    let at = Text::new(source.clone()).position(source.find("memBAD").unwrap() + 3);
    ws.open(main.clone(), source, 6);
    let candidate = item(&ws, &main, at, "member");
    assert!(completion::preview(&candidate).contains("Default member"));
}
