use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn complete(marked: &str, label: &str) -> (CompletionItem, Text, usize) {
    let offset = marked.find('|').unwrap();
    let source = marked.replacen('|', "", 1);
    let uri = Url::parse("file:///operand-completion/main.tex").unwrap();
    let mut workspace = Workspace {
        snippets: true,
        ..Workspace::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.clone(), Some(1)));
    let text = Text::new(source);
    let item = features::completion(&workspace, &uri, text.position(offset))
        .into_iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("missing {label} in {marked}"));
    (item, text, offset)
}

#[test]
fn command_operands_replace_only_the_name() {
    for operand in [
        r"\meaning\ord|",
        "\\meaning % comment\n  \\ord|",
        r"\meaning\ord|inary",
        r"\let\saved=\ord|",
        r"\let\ord|=\relax",
        r"\newcommand{\ord|}[1]{#1}",
        r"\newcommand*\ord|[1]{#1}",
        r"\NewDocumentCommand{\ord|}{m}{#1}",
        r"\def\ord|#1{#1}",
        r"\futurelet\ord|\relax x",
        r"\NewCommandCopy{\saved}{\ord|}",
        r"\NewCommandCopy{\ord|}{\relax}",
        r"\letcs{\ord|}{relax}",
        r"\cslet{saved}{\ord|}",
    ] {
        let marked = format!("\\newcommand{{\\ordinary}}[1]{{#1}}\n中文😀 {operand}");
        let (item, text, offset) = complete(&marked, "ordinary");
        assert_eq!(
            item.insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT),
            "{marked}"
        );
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.new_text, "ordinary", "{marked}");
        let start = text.source[..offset].rfind('\\').unwrap() + 1;
        let end = offset
            + text.source[offset..]
                .bytes()
                .take_while(u8::is_ascii_alphabetic)
                .count();
        assert_eq!(edit.range, text.range(start, end), "{marked}");
        let changed = text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text,
            }])
            .unwrap();
        assert_eq!(
            changed.source,
            format!("{}ordinary{}", &text.source[..start], &text.source[end..]),
            "{marked}"
        );
    }
}

#[test]
fn builtin_operands_do_not_insert_call_templates() {
    for marked in [
        r"\meaning\sec|",
        r"\let\saved\sec|",
        r"\futurelet\sec|\relax x",
    ] {
        let (item, _, _) = complete(marked, "section");
        assert_eq!(
            item.insert_text_format,
            Some(InsertTextFormat::PLAIN_TEXT),
            "{marked}"
        );
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.new_text, "section", "{marked}");
    }
}

#[test]
fn executed_commands_keep_argument_snippets() {
    for call in [
        r"\ord|",
        r"\meaning\meaning\ord|",
        r"\meaning{\ord|}",
        r"\let\saved\meaning\ord|",
        r"\let\saved\newcommand\ord|",
        r"\futurelet\saved\ord|",
        r"\futurelet\saved\relax\ord|",
        r"\def\meaning{}\meaning\ord|",
        r"\def\let{}\let\saved\ord|",
    ] {
        let marked = format!("\\newcommand{{\\ordinary}}[1]{{#1}}\n{call}");
        let (item, _, _) = complete(&marked, "ordinary");
        assert_eq!(
            item.insert_text_format,
            Some(InsertTextFormat::SNIPPET),
            "{marked}"
        );
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.new_text, "ordinary{${1:arg1}}", "{marked}");
    }
}

#[test]
fn operand_completions_use_cross_file_definitions_and_preserve_following_text() {
    let temp = tempfile::tempdir().unwrap();
    for (name, source) in [
        ("main.tex", r"\input{definitions}\input{content}"),
        ("definitions.tex", r"\newcommand{\ordinary}[1]{#1}"),
        ("content.tex", "\\meaning % comment\r\n  \\ord$ suffix"),
    ] {
        std::fs::write(temp.path().join(name), source).unwrap();
    }
    let mut workspace = Workspace {
        snippets: true,
        ..Workspace::default()
    };
    workspace.add_root(temp.path().into());
    let uri = Url::from_file_path(temp.path().join("content.tex")).unwrap();
    let text = &workspace.documents[&uri].text;
    let offset = text.source.find("ord").unwrap() + 3;
    let item = features::completion(&workspace, &uri, text.position(offset))
        .into_iter()
        .find(|item| item.label == "ordinary")
        .unwrap();
    assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
        panic!("missing edit")
    };
    assert_eq!(edit.range, text.range(offset - 3, offset));
    let changed = text
        .changed(vec![TextDocumentContentChangeEvent {
            range: Some(edit.range),
            range_length: None,
            text: edit.new_text,
        }])
        .unwrap();
    assert_eq!(
        changed.source,
        "\\meaning % comment\r\n  \\ordinary$ suffix"
    );
}
