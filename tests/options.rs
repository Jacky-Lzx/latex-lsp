use lsp_types::*;
use texquill::{language::features, project::workspace::Workspace, text::Text};
fn complete(marked: &str) -> Vec<CompletionItem> {
    let uri = Url::parse("file:///project/main.tex").unwrap();
    let mut ws = Workspace::default();
    let marker = if marked.contains('¦') { '¦' } else { '|' };
    let text = Text::new(marked.replace(marker, ""));
    let at = text.position(marked.find(marker).unwrap());
    ws.open(uri.clone(), text.source, 1);
    features::completion(&ws, &uri, at)
}
fn inserted(items: &[CompletionItem], label: &str) -> String {
    let item = items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{label}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    edit.new_text.clone()
}
#[test]
fn options_follow_package_class_graphics_and_environment_contexts() {
    for (source, label, replacement) in [
        (r"\usepackage[mar|]{geometry}", "margin", "margin="),
        (r"\documentclass[two|]{article}", "twoside", "twoside"),
        (r"\includegraphics[wi|]{picture}", "width", "width="),
        (
            r"\includegraphics[trim={1,2,3,4},cl|]{picture}",
            "clip",
            "clip=",
        ),
        (r"\hypersetup{colorl|}", "colorlinks", "colorlinks="),
        (r"\geometry{mar|", "margin", "margin="),
        (r"\setkeys{Gin}{ang|}", "angle", "angle="),
        (r"\begin{enumerate}[leftm|]", "leftmargin", "leftmargin="),
    ] {
        assert_eq!(inserted(&complete(source), label), replacement);
    }
    assert!(!complete(r"\includegraphics[width=1cm,wi|]{p}")
        .iter()
        .any(|item| item.label == "width"));
    assert!(complete(r"\newcommand{\geometry}[1]{#1}\geometry{mar|}").is_empty());
    assert!(complete(r"\verb|\includegraphics[wi¦]{p}|").is_empty());
}
#[test]
fn values_and_key_replacement_preserve_existing_values_and_utf16_ranges() {
    assert_eq!(
        inserted(
            &complete(r"\includegraphics[keepaspectratio=t|]{p}"),
            "true"
        ),
        "true"
    );
    assert_eq!(
        inserted(&complete(r"\includegraphics[ang|le=90]{p}"), "angle"),
        "angle"
    );
    let items = complete("😀 \\usepackage[lin|kcolor=blue]{hyperref}");
    let item = items.iter().find(|item| item.label == "linkcolor").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(
        edit.range,
        Range::new(Position::new(0, 15), Position::new(0, 24))
    );
}
