use latex_lsp::{features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;
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
