use latex_lsp::{
    features, signature,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///bracket-labels/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(3)));
    uri
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
fn hyperref_bracket_labels_use_the_actual_key_and_rename_distinct_keys_independently() {
    let mut workspace = Workspace::default();
    let declarations = "\\label{sec:[balanced]}\n\\label{sec:[balanced}\n\\label{sec:[literal}\n";
    let declaration = insert(&mut workspace, "labels.tex", declarations);
    let source = "😀 \\ref{sec:[balanced]}\n\\hyperref[sec:[balanced]]{Body sec:[balanced]}\n\\hyperref[{sec:[balanced]}]{Protected}\n\\hyperref[sec:[literal]{Literal}\n";
    let main = insert(&mut workspace, "main.tex", source);
    for (marker, key, count) in [
        ("\\ref{sec:[bal", "sec:[balanced]", 2),
        ("\\hyperref[sec:[bal", "sec:[balanced", 1),
        ("\\hyperref[{sec:[bal", "sec:[balanced]", 2),
        ("\\hyperref[sec:[li", "sec:[literal", 1),
    ] {
        let at = Text::new(source.into()).position(source.find(marker).unwrap() + marker.len());
        let definitions = features::definitions(&workspace, &main, at);
        assert_eq!(definitions.len(), 1, "{marker}: {definitions:?}");
        assert_eq!(definitions[0].uri, declaration);
        let Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) =
            features::prepare_rename(&workspace, &main, at).unwrap()
        else {
            panic!("{marker}")
        };
        assert_eq!(placeholder, key, "{marker}");
        assert_eq!(
            features::references(&workspace, &main, at, false).len(),
            count,
            "{marker}"
        );
        let changes = features::rename(&workspace, &main, at, "sec:renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        let expected = match key {
            "sec:[balanced]" => source
                .replace("\\ref{sec:[balanced]}", "\\ref{sec:renamed}")
                .replace("\\hyperref[{sec:[balanced]}]", "\\hyperref[{sec:renamed}]"),
            "sec:[balanced" => {
                source.replace("\\hyperref[sec:[balanced]]", "\\hyperref[sec:renamed]]")
            }
            _ => source.replace("\\hyperref[sec:[literal]", "\\hyperref[sec:renamed]"),
        };
        assert_eq!(apply(source, changes[&main].clone()), expected, "{marker}");
        assert_eq!(
            apply(declarations, changes[&declaration].clone()),
            declarations.replace(&format!("\\label{{{key}}}"), "\\label{sec:renamed}")
        );
    }
}

#[test]
fn hyperref_literal_bracket_key_completion_replaces_the_whole_key_and_protects_delimiters() {
    let mut workspace = Workspace::default();
    insert(
        &mut workspace,
        "labels.tex",
        "\\label{sec:[literal}\n\\label{sec:[balanced]}\n",
    );
    for (marked, key, expected) in [
        (
            r"\hyperref[sec:[li¦BAD]{Body}",
            "sec:[literal",
            r"\hyperref[{sec:[literal}]{Body}",
        ),
        (
            r"\hyperref[sec:¦[literal]{Body}",
            "sec:[literal",
            r"\hyperref[{sec:[literal}]{Body}",
        ),
        (
            r"\hyperref[{sec:¦[balanced]}]{Body}",
            "sec:[balanced]",
            r"\hyperref[{sec:[balanced]}]{Body}",
        ),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let at = Text::new(source.clone()).position(offset);
        let main = insert(&mut workspace, "main.tex", &source);
        let items = features::completion(&workspace, &main, at);
        let item = items
            .iter()
            .find(|item| item.label == key)
            .unwrap_or_else(|| panic!("{marked}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("{marked}")
        };
        let mut edits = item.additional_text_edits.clone().unwrap_or_default();
        edits.push(edit.clone());
        assert_eq!(apply(&source, edits), expected, "{marked}");
    }
    for command in ["hyperref", "hyperdef"] {
        let tail = if command == "hyperref" {
            "{Body}"
        } else {
            "{category}{name}{Text}"
        };
        let marked = format!("\\{command}[sec:[li¦teral]{tail}");
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let help = signature::help(&workspace, &main, Text::new(source).position(offset)).unwrap();
        assert_eq!(help.active_parameter, Some(0));
        let marked = format!("\\{command}[sec:[literal]{{cat¦egory}}{{name}}{{Text}}");
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let help = signature::help(&workspace, &main, Text::new(source).position(offset)).unwrap();
        assert_eq!(help.active_parameter, Some(1));
    }
}

#[test]
fn hyperdef_bracket_labels_follow_tex_without_inventing_shifted_pdf_destinations() {
    let mut workspace = Workspace::default();
    let source = "\\hyperdef[sec:[balanced]]{category}{name}{Text}\n\\hyperdef[sec:[literal]{category}{name}{Text}\n\\ref{sec:[balanced} \\ref{sec:[literal}\n";
    let main = insert(&mut workspace, "main.tex", source);
    let syntax = &workspace.documents[&main].syntax;
    assert_eq!(
        syntax
            .labels
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["sec:[balanced", "sec:[literal"]
    );
    assert_eq!(syntax.anchors.len(), 1);
    assert_eq!(syntax.anchors[0].name, "category.name");
    for key in ["sec:[balanced", "sec:[literal"] {
        let marker = format!("\\ref{{{key}");
        let at = Text::new(source.into()).position(source.find(&marker).unwrap() + marker.len());
        assert_eq!(features::definitions(&workspace, &main, at).len(), 1);
        let changes = features::rename(&workspace, &main, at, "sec:renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(
            apply(source, changes[&main].clone()),
            source.replace(key, "sec:renamed")
        );
    }
}

#[test]
fn hyperref_warnings_select_the_delimited_key_without_borrowing_a_protected_key() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "😀 \\hyperref[sec:[balanced]]{Body} \\hyperref[{sec:[balanced]}]{Protected}\n";
    std::fs::write(&main, source).unwrap();
    let diagnostics = latex_lsp::build_log::parse(&main, "(./main.tex\nLaTeX Warning: Hyper reference `sec:[balanced' on page 1 undefined on input line 1.\n)\n");
    let items = &diagnostics[&uri];
    assert_eq!(items.len(), 1, "{items:?}");
    let start = source.find("sec:[balanced").unwrap();
    assert_eq!(
        items[0].range,
        Text::new(source.into()).range(start, start + "sec:[balanced".len())
    );
}
