use lsp_types::*;
use texquill::{
    language::{features, signature},
    project::workspace::Workspace,
    text::Text,
};

fn open(workspace: &mut Workspace, source: &str) -> Url {
    let uri = Url::parse("file:///entry-type-tests/local.bbx").unwrap();
    workspace.open(uri.clone(), source.into(), 4);
    uri
}

#[test]
fn entry_type_conditions_offer_output_types_and_share_database_descriptions() {
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            snippets: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let bib = Url::parse("file:///entry-type-tests/refs.bib").unwrap();
    let bibliography = "@book{book,title={Example}} @custom{custom,title={Custom}}";
    workspace.open_with_language(bib.clone(), bibliography.into(), 1, "bibtex");
    let source = "\\ifentrytype{}{yes}{no}";
    let uri = open(&mut workspace, source);
    let at = Text::new(source.into()).position(source.find("{}").unwrap() + 1);
    let items = features::completion(&workspace, &uri, at);
    assert!(items
        .iter()
        .all(|item| item.kind == Some(CompletionItemKind::CLASS)));
    assert_eq!(items.len(), 29);
    for name in ["article", "book", "online", "report", "thesis", "set"] {
        let item = items.iter().find(|item| item.label == name).unwrap();
        assert_ne!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
        assert!(item.documentation.is_some());
    }
    for name in [
        "title",
        "author",
        "publisher",
        "custom",
        "comment",
        "string",
        "preamble",
        "xdata",
        "conference",
        "mastersthesis",
        "phdthesis",
        "techreport",
    ] {
        assert!(!items.iter().any(|item| item.label == name), "{name}");
    }
    let source = "😀 \\ifentrytype{book}{True book}{False book}\\label{book}";
    open(&mut workspace, source);
    let text = &workspace.documents[&uri].text;
    let start = source.find("{book}").unwrap() + 1;
    let at = text.position(start + 2);
    let hover = features::hover(&workspace, &uri, at).unwrap();
    assert_eq!(hover.range, Some(text.range(start, start + 4)));
    let bib_hover = features::hover(&workspace, &bib, Position::new(0, 3)).unwrap();
    assert_eq!(hover.contents, bib_hover.contents);
    assert!(features::definitions(&workspace, &uri, at).is_empty());
    assert!(features::references(&workspace, &uri, at, true).is_empty());
    assert!(features::prepare_rename(&workspace, &uri, at)
        .unwrap()
        .is_none());
    let help = signature::help(&workspace, &uri, at).unwrap();
    assert_eq!(
        help.signatures[0].label,
        "\\ifentrytype{entry type}{true}{false}"
    );
    assert_eq!(help.active_parameter, Some(0));
    for (marker, parameter) in [("True book", 1), ("False book", 2)] {
        let at = text.position(source.find(marker).unwrap() + marker.len() - 2);
        assert_eq!(
            signature::help(&workspace, &uri, at)
                .unwrap()
                .active_parameter,
            Some(parameter)
        );
        assert!(features::hover(&workspace, &uri, at).is_none());
        assert!(!features::completion(&workspace, &uri, at)
            .iter()
            .any(|item| item.kind == Some(CompletionItemKind::CLASS)));
    }
}

#[test]
fn type_name_edits_preserve_comments_branches_and_unfinished_groups() {
    for (marked, expected) in [
        (
            "\\ifentrytype{BO¦BAD}{True book}{False book}",
            "\\ifentrytype{book}{True book}{False book}",
        ),
        (
            "% 😀\r\n\\ifentrytype{% before\r\n BO¦BAD % after\r\n}{yes}{no}",
            "% 😀\r\n\\ifentrytype{% before\r\nbook% after\r\n}{yes}{no}",
        ),
        ("\\ifentrytype{BO¦", "\\ifentrytype{book"),
        (
            "\\ifentrytype{BO¦% after\r\n}",
            "\\ifentrytype{book% after\r\n}",
        ),
        (
            "\\ifentrytype{article}{\\ifentrytype{BO¦BAD}{yes}{no}}{no}",
            "\\ifentrytype{article}{\\ifentrytype{book}{yes}{no}}{no}",
        ),
    ] {
        let source = marked.replace('¦', "");
        let mut workspace = Workspace::default();
        let uri = open(&mut workspace, &source);
        let text = &workspace.documents[&uri].text;
        let at = text.position(marked.find('¦').unwrap());
        let item = features::completion(&workspace, &uri, at)
            .into_iter()
            .find(|item| item.label == "book")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        assert_eq!(edit.range.start.line, at.line);
        assert_eq!(edit.range.end.line, at.line);
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let completed = text
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
            .unwrap();
        assert_eq!(completed.source, expected, "{marked}");
    }
}

#[test]
fn invalid_calls_dynamic_types_and_user_overrides_do_not_get_builtin_type_hints() {
    for marked in [
        "\\ifentrytype*{bo¦ok}{yes}{no}",
        "\\ifentrytype[fmt]{bo¦ok}{yes}{no}",
        "\\ifentrytype\n\n{bo¦ok}{yes}{no}",
        "\\ifentrytype{{bo¦ok}}{yes}{no}",
        "\\ifentrytype{bo¦ok,article}{yes}{no}",
        "\\ifentrytype{bo¦ok\\suffix}{yes}{no}",
        "\\ifentrytype{bo¦% joined\r\nok}{yes}{no}",
        "\\meaning\\ifentrytype{bo¦ok}{yes}{no}",
        "% \\ifentrytype{bo¦ok}{yes}{no}\r\n",
        "\\verb|\\ifentrytype{bo¦ok}{yes}{no}|",
        "\\newcommand\\ifentrytype[1]{}\\ifentrytype{bo¦ok}",
        "\\newcommand\\custom[1]{}\\let\\ifentrytype\\custom\\ifentrytype{bo¦ok}",
        "\\newif\\ifentrytype\\ifentrytype{bo¦ok}",
    ] {
        let source = marked.replace('¦', "");
        let mut workspace = Workspace::default();
        let uri = open(&mut workspace, &source);
        let at = Text::new(source).position(marked.find('¦').unwrap());
        assert!(
            !features::completion(&workspace, &uri, at)
                .iter()
                .any(|item| item.kind == Some(CompletionItemKind::CLASS)),
            "{marked}"
        );
        assert!(features::hover(&workspace, &uri, at).is_none(), "{marked}");
    }
}

#[test]
fn type_hints_restore_after_buffer_changes_and_stay_independent_of_other_projects() {
    let mut workspace = Workspace::default();
    for source in [
        "\\newif\\ifentrytype\\ifentrytype{book}",
        "\\newcommand\\ifentrytype[1]{}\\ifentrytype{book}",
        "\\ifentrytype{book}{yes}{no}",
    ] {
        let uri = open(&mut workspace, source);
        let at = Text::new(source.into()).position(source.rfind("book").unwrap() + 2);
        let native = source.starts_with("\\ifentrytype{");
        assert_eq!(features::hover(&workspace, &uri, at).is_some(), native);
        assert_eq!(
            features::completion(&workspace, &uri, at)
                .iter()
                .any(|item| item.kind == Some(CompletionItemKind::CLASS)),
            native
        );
    }
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("a.tex"),
        "\\documentclass{article}\\newif\\ifentrytype",
    )
    .unwrap();
    let source = "\\documentclass{article}\\ifentrytype{book}{yes}{no}";
    std::fs::write(directory.path().join("b.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let uri = Url::from_file_path(directory.path().join("b.tex")).unwrap();
    let at = Text::new(source.into()).position(source.find("book").unwrap() + 2);
    assert!(features::completion(&workspace, &uri, at)
        .iter()
        .any(|item| item.label == "book"));
    assert!(features::hover(&workspace, &uri, at).is_some());
}
