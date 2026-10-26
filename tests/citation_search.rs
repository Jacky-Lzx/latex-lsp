use latex_lsp::{language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;
fn fixture() -> (Workspace, Url) {
    let mut ws = Workspace::default();
    let uri = Url::parse("file:///search/main.tex").unwrap();
    let bib = Url::parse("file:///search/refs.bib").unwrap();
    ws.open(bib,"@book{texbook,title={The {TeX}book},author={Donald Knuth},year={1984}}\n@book{knuth-prefix,title={Other},author={Someone},year={2020}}\n@article{unicode,title={计算与排版},author={张三},year={2026}}".into(),1);
    (ws, uri)
}
fn complete(ws: &mut Workspace, uri: &Url, marked: &str) -> (Text, Vec<CompletionItem>) {
    let text = Text::new(marked.replace('|', ""));
    let at = text.position(marked.find('|').unwrap());
    ws.open(uri.clone(), text.source.clone(), 1);
    let items = features::completion(ws, uri, at);
    (text, items)
}
#[test]
fn citation_search_uses_author_title_year_multiple_words_unicode_and_fuzzy_keys() {
    let (mut ws, uri) = fixture();
    for (query, key) in [
        ("Knuth", "texbook"),
        ("TeXbook", "texbook"),
        ("1984", "texbook"),
        ("Knuth 1984", "texbook"),
        ("txbk", "texbook"),
        ("texbok", "texbook"),
        ("张三", "unicode"),
        ("排版", "unicode"),
    ] {
        let (text, items) = complete(&mut ws, &uri, &format!("😀 \\cite{{{query}|}}"));
        let item = items
            .iter()
            .find(|item| item.label == key)
            .unwrap_or_else(|| panic!("{query}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, key);
        assert_eq!(
            &text.source
                [text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap()],
            query
        );
        assert!(item.filter_text.as_ref().unwrap().starts_with(query));
    }
    let (_, items) = complete(&mut ws, &uri, r"\cite{knuth|}");
    assert_eq!(items[0].label, "knuth-prefix");
    let (_, items) = complete(&mut ws, &uri, r"\cites[see]{1984|}{unicode}");
    assert!(items.iter().any(|item| item.label == "texbook"));
    let (_, items) = complete(&mut ws, &uri, r"\cite{Knuth 1984|");
    assert!(items.iter().any(|item| item.label == "texbook"));
}
#[test]
fn metadata_search_does_not_cross_saved_projects_or_insert_prose() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.tex"), "\\documentclass{article}").unwrap();
    let (mut ws, _) = fixture();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let (_, items) = complete(&mut ws, &uri, "\\documentclass{article}\\cite{Knuth|}");
    assert!(items.is_empty());
}

#[test]
fn full_bibtex_previews_omit_paths_and_keep_encoded_navigation_uris() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("中文目录 with space %20😀");
    std::fs::create_dir(&root).unwrap();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let bib = Url::from_file_path(root.join("ref.bib")).unwrap();
    let record = "@article{Peng18,\n  author = {X. Peng and W. Qian},\n  title = {A {nested} paper},\n  journal = {Journal},\n  year = {2018},\n  volume = {37},\n  number = {12},\n  pages = {3109--3122},\n}";
    std::fs::write(
        root.join("ref.bib"),
        format!("{record}\n@book{{other,title={{Another entry}}}}\n"),
    )
    .unwrap();
    let expected = format!("```bibtex\n{record}\n```");
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    for body in [
        r"\cite{Pe}",
        r"\entrydata{Pe}{Text}",
        r"\newrefsection[ref.bib]\cite{Pe}",
    ] {
        let source = format!(
            r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{ref.bib}}\begin{{document}}{body}\end{{document}}"
        );
        std::fs::write(root.join("main.tex"), &source).unwrap();
        ws.open(main.clone(), source.clone(), 1);
        let text = Text::new(source);
        let at = text.source.find("{Pe}").unwrap() + 3;
        let items = features::completion(&ws, &main, text.position(at));
        let item = items.iter().find(|item| item.label == "Peng18").unwrap();
        let Some(Documentation::MarkupContent(docs)) = &item.documentation else {
            panic!("{body}: {item:?}")
        };
        assert_eq!(docs.kind, MarkupKind::Markdown);
        assert_eq!(docs.value, expected);
        assert!(item.detail.is_none());
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!()
        };
        let accepted = text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        let position = accepted.position(accepted.source.find("{Peng18}").unwrap() + 2);
        ws.open(main.clone(), accepted.source, 2);
        let hover = features::hover(&ws, &main, position).unwrap();
        let HoverContents::Markup(contents) = hover.contents else {
            panic!()
        };
        assert_eq!(contents.kind, MarkupKind::Markdown);
        assert_eq!(contents.value, expected, "{body}");
        let targets = features::definitions(&ws, &main, position);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].uri, bib);
        assert!(targets[0].uri.as_str().contains("%E4%B8%AD"));
    }
}
