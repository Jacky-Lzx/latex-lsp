use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn complete(marked: &str, label: &str) -> (CompletionItem, Text, usize) {
    let offset = marked.find('|').unwrap();
    let source = marked.replacen('|', "", 1);
    let uri = Url::parse("file:///operand-completion/main.tex").unwrap();
    let mut workspace = Workspace {
        snippets: true,
        ..Workspace::default()
    };
    // Inspect edits for specific names across the complete command catalog.
    workspace.config.indexing.completion_limit = 10_000;
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
        r"\newcommand{\ord|",
        "\\newcommand{ % comment\n  \\ord|}",
        r"\newcommand*\ord|[1]{#1}",
        r"\NewDocumentCommand{\ord|}{m}{#1}",
        r"\def\ord|#1{#1}",
        r"\futurelet\ord|\relax x",
        r"\NewCommandCopy{\saved}{\ord|}",
        r"\NewCommandCopy{\saved}{\ord|",
        r"\NewCommandCopy{\ord|",
        r"\NewCommandCopy{\ord|}{\relax}",
        r"\letcs{\ord|}{relax}",
        r"\cslet{saved}{\ord|}",
        r"\iffieldequals{title}{\ord|}",
        r"\iflistequals{publisher}{\ord|}",
        r"\ifnameequals{author}{\ord|}",
        "\\iffieldequals{title}% between\r\n{% before\r\n\\ord|% after\r\n}",
        r"\ifnameequals{author}{\ord|",
        r"\iffieldequals{title}\ord|",
        r"\newif\ord|",
        r"\newif\ord|inary",
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
fn empty_operand_prefixes_insert_names_without_arguments_or_fake_definitions() {
    for operand in [
        r"\meaning\|",
        "\\meaning % comment\r\n  \\|",
        r"\meaning\|$",
        r"\let\saved=\|",
        r"\let\|=\relax",
        r"\newcommand{\|}",
        r"\newcommand{ % comment
          \|}",
        r"\newcommand{\|",
        r"\newcommand*\|",
        r"\NewDocumentCommand{\|}{m}{#1}",
        r"\def\|#1{#1}",
        r"\futurelet\|\relax x",
        r"\NewCommandCopy{\saved}{\|}",
        r"\NewCommandCopy{\|}{\relax}",
        r"\letcs{\|}{relax}",
        r"\cslet{saved}{\|}",
        r"\iffieldequals{title}{\|}",
        r"\iflistequals{publisher}{\|}",
        r"\ifnameequals{author}{\|}",
        r"\ifnameequals{author}{\|",
        r"\newif\|",
        "\\newif % condition\r\n\\|",
    ] {
        let marked = format!("\\newcommand{{\\ordinary}}[1]{{#1}}\n中文😀 {operand}");
        for label in ["ordinary", "section"] {
            let (item, text, offset) = complete(&marked, label);
            assert_eq!(
                item.insert_text_format,
                Some(InsertTextFormat::PLAIN_TEXT),
                "{marked}: {label}"
            );
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(edit.range, text.range(offset, offset), "{marked}");
            assert_eq!(edit.new_text, label, "{marked}");
            let uri = Url::parse("file:///operand-completion/main.tex").unwrap();
            let document = Document::new(&uri, text.source, Some(1));
            assert!(
                document
                    .syntax
                    .definitions
                    .iter()
                    .all(|entry| !entry.name.is_empty()),
                "{marked}"
            );
        }
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
        r"\|",
        r"\meaning\meaning\|",
        r"\meaning\string\|",
        r"\meaning\detokenize{\|}",
        r"\meaning{\|",
        r"\meaning*\|",
        r"\let\saved\meaning\|",
        r"\futurelet\saved\|",
        r"\def\meaning{}\meaning\|",
        r"\def\let{}\let\saved\|",
        r"\ord|",
        r"\meaning\meaning\ord|",
        r"\meaning{\ord|}",
        r"\let\saved\meaning\ord|",
        r"\let\saved\newcommand\ord|",
        r"\futurelet\saved\ord|",
        r"\futurelet\saved\relax\ord|",
        r"\def\meaning{}\meaning\ord|",
        r"\def\let{}\let\saved\ord|",
        r"\iffieldequals{title}{\saved}{\ord|}{no}",
        r"\iflistequals{publisher}{\saved}{yes}{\ord|}",
        r"\ifnameequals{author}{\saved}\ord|",
        r"\newcommand\iffieldequals[2]{}\iffieldequals{title}{\ord|}",
        r"\newif\iflistequals\iflistequals{publisher}{\ord|}",
        r"\meaning\ifnameequals{author}{\ord|}",
        r"\def\newif{}\newif\ord|",
        r"\meaning\newif\ord|",
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
    ] {
        std::fs::write(temp.path().join(name), source).unwrap();
    }
    for prefix in ["", "ord"] {
        let source = format!("\\meaning % comment\r\n  \\{prefix}$ suffix");
        std::fs::write(temp.path().join("content.tex"), &source).unwrap();
        let mut workspace = Workspace {
            snippets: true,
            ..Workspace::default()
        };
        workspace.add_root(temp.path().into());
        let uri = Url::from_file_path(temp.path().join("content.tex")).unwrap();
        let text = &workspace.documents[&uri].text;
        let offset = source.find("$ suffix").unwrap();
        let item = features::completion(&workspace, &uri, text.position(offset))
            .into_iter()
            .find(|item| item.label == "ordinary")
            .unwrap();
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.range, text.range(offset - prefix.len(), offset));
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
}

#[test]
fn literal_text_and_escaped_backslashes_do_not_complete_commands() {
    let uri = Url::parse("file:///operand-completion/main.tex").unwrap();
    for marked in [
        "% \\meaning\\|",
        r"\verb!\meaning\|!",
        r"\detokenize{\meaning\| x}",
        r"\detokenize{\ord|}",
        r"\string\|",
        r"\string\ord|",
        r"\meaning\\|",
    ] {
        let offset = marked.find('|').unwrap();
        let source = marked.replacen('|', "", 1);
        let text = Text::new(source.clone());
        let mut workspace = Workspace {
            snippets: true,
            ..Workspace::default()
        };
        workspace
            .documents
            .insert(uri.clone(), Document::new(&uri, source, Some(1)));
        assert!(
            features::completion(&workspace, &uri, text.position(offset)).is_empty(),
            "{marked}"
        );
    }
}
