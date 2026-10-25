use latex_lsp::{
    compilation::build_log,
    editing,
    language::{features, semantic, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///entrydata/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(7)));
    uri
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
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
fn data_contexts_edit_one_key_and_nested_citations_preserving_code_and_other_namespaces() {
    for key in ["alpha", "O\"Connor", "book(old)", "中文😀"] {
        for star in ["", "*", "% boundary\r\n *% star\r\n"] {
            let mut workspace = Workspace::default();
            let bibliography = format!("@book{{{key},title={{Literal {key}}}}}");
            let bib = insert(&mut workspace, "refs.bib", &bibliography);
            let source = format!("% 😀\r\n\\entrydata{star}{{% before {key}\r\n{key}% after\r\n}}{{Literal {key} \\cite{{{key}}} \\label{{{key}}}}}\\verb|\\entrydata{{{key}}}{{}}|% \\entrydata{{{key}}}{{}}\r\n");
            let main = insert(&mut workspace, "main.tex", &source);
            let doc = &workspace.documents[&main];
            let start = source.find(&format!("\r\n{key}% after")).unwrap() + 2;
            let at = doc.text.position(start + 1);
            let range = doc.text.range(start, start + key.len());
            let definitions = features::definitions(&workspace, &main, at);
            assert_eq!(definitions.len(), 1, "{source}");
            assert_eq!(definitions[0].uri, bib);
            assert!(features::hover(&workspace, &main, at).is_some());
            assert_eq!(features::references(&workspace, &main, at, true).len(), 3);
            assert_eq!(editing::highlights(&workspace, &main, at).len(), 2);
            let Some(PrepareRenameResponse::RangeWithPlaceholder {
                range: selected,
                placeholder,
            }) = features::prepare_rename(&workspace, &main, at).unwrap()
            else {
                panic!("{source}")
            };
            assert_eq!(placeholder, key);
            assert_eq!(selected, range);
            assert_eq!(doc.syntax.citation_references.len(), 1);
            assert_eq!(
                doc.syntax.citation_references[0].span,
                start..start + key.len()
            );

            let tokens = semantic::tokens(doc, None).unwrap();
            let mut line = 0;
            let mut character = 0;
            let mut key_tokens = Vec::new();
            for token in tokens.data {
                character = if token.delta_line == 0 {
                    character + token.delta_start
                } else {
                    token.delta_start
                };
                line += token.delta_line;
                if token.token_type == 3 && token.token_modifiers_bitset == 0 {
                    key_tokens.push((Position::new(line, character), token.length));
                }
            }
            assert!(key_tokens.contains(&(range.start, key.encode_utf16().count() as u32)));
            let selection = editing::selection_ranges(doc, &[at]).unwrap().remove(0);
            let mut current = Some(&selection);
            let mut ranges = Vec::new();
            while let Some(selection) = current {
                ranges.push(selection.range);
                current = selection.parent.as_deref();
            }
            assert!(ranges.contains(&range));
            let call_start = source.find("\\entrydata").unwrap();
            let call_end = source.find("\\verb").unwrap();
            assert!(
                ranges.contains(&doc.text.range(call_start, call_end)),
                "{ranges:?}"
            );

            let changes = features::rename(&workspace, &main, at, "new\"key😀")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes[&main].len(), 2);
            let expected = source
                .replace(&format!("\r\n{key}% after"), "\r\nnew\"key😀% after")
                .replace(&format!("\\cite{{{key}}}"), "\\cite{new\"key😀}");
            assert_eq!(apply(&source, &changes[&main]), expected);
            assert_eq!(
                apply(&bibliography, &changes[&bib]),
                format!("@book{{new\"key😀,title={{Literal {key}}}}}")
            );
        }
    }
}

#[test]
fn context_key_completion_preserves_comments_and_replaces_the_whole_single_key() {
    for (marked, expected) in [
        (
            "😀 \\entrydata*{al¦BAD}{Literal alpha}",
            "😀 \\entrydata*{alpha}{Literal alpha}",
        ),
        (
            "\\entrydata % star\r\n*{% before\r\n al¦BAD % after\r\n}{alpha}",
            "\\entrydata % star\r\n*{% before\r\nalpha% after\r\n}{alpha}",
        ),
        (
            "\\entrydata{\r\n al¦BAD \r\n}{alpha}",
            "\\entrydata{alpha}{alpha}",
        ),
        (
            "\\entrydata{% before\r\nal¦",
            "\\entrydata{% before\r\nalpha",
        ),
        ("\\entrydata{¦}{alpha}", "\\entrydata{alpha}{alpha}"),
    ] {
        let mut workspace = Workspace::default();
        insert(&mut workspace, "refs.bib", "@book{alpha,title={Alpha}}");
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        let item = features::completion(&workspace, &main, at)
            .into_iter()
            .find(|item| item.label == "alpha")
            .unwrap_or_else(|| panic!("{marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        assert_eq!(edit.range.start.line, at.line);
        assert_eq!(edit.range.end.line, at.line);
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        assert_eq!(apply(&source, &edits), expected, "{marked}");
    }
}

#[test]
fn unsupported_context_keys_never_become_partial_lists_or_targets_inside_code() {
    for marked in [
        "\\entrydata[option]{al¦pha}{code}",
        "\\entrydata**{al¦pha}{code}",
        "\\entrydata{al¦pha,beta}{code}",
        "\\entrydata{al¦pha beta}{code}",
        "\\entrydata{{al¦pha}}{code}",
        "\\entrydata{al¦pha\\suffix}{code}",
        "\\entrydata{al¦% joined\npha}{code}",
        "\\entrydata{beta}{al¦pha}",
        "\\entrydata{beta}{code}{al¦pha}",
        "\\entrydata{beta}{code} text {al¦pha}",
        "\\Entrydata{al¦pha}{code}",
        "\\verb|\\entrydata{al¦pha}{code}|",
        "\\begin{verbatim}\\entrydata{al¦pha}{code}\\end{verbatim}",
    ] {
        let mut workspace = Workspace::default();
        insert(
            &mut workspace,
            "refs.bib",
            "@book{alpha,title={Alpha}} @book{beta,title={Beta}}",
        );
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        assert!(
            features::definitions(&workspace, &main, at).is_empty(),
            "{marked}"
        );
        assert!(
            features::prepare_rename(&workspace, &main, at)
                .unwrap()
                .is_none(),
            "{marked}"
        );
        assert!(
            !features::completion(&workspace, &main, at)
                .iter()
                .any(|item| item.label == "alpha"),
            "{marked}"
        );
        assert!(
            !workspace.documents[&main]
                .syntax
                .citation_references
                .iter()
                .any(|entry| entry.name == "alpha"),
            "{marked}"
        );
    }
}

#[test]
fn context_signatures_track_key_code_and_waiting_parameters_and_respect_user_signatures() {
    for (marked, active) in [
        ("\\entrydata¦", 0),
        ("\\entrydata *% boundary\r\n ¦", 0),
        ("\\entrydata{al¦pha}{code}", 0),
        ("\\entrydata*{alpha} ¦", 1),
        ("\\entrydata{alpha}{co¦de}", 1),
        ("\\entrydata{alpha}{co¦", 1),
    ] {
        let mut workspace = Workspace::default();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let at = workspace.documents[&main]
            .text
            .position(marked.find('¦').unwrap());
        let help = signature::help(&workspace, &main, at).unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(help.active_parameter, Some(active), "{marked}");
        assert_eq!(help.signatures[0].label, "\\entrydata{key}{code}");
    }
    for snippets in [false, true] {
        for tail in ["", "{alpha}{code}", "*{alpha}{code}"] {
            let mut workspace = Workspace {
                snippets,
                ..Default::default()
            };
            let source = format!("\\entrydata{tail}");
            let main = insert(&mut workspace, "main.tex", &source);
            let at = workspace.documents[&main]
                .text
                .position("\\entrydata".len());
            let item = features::completion(&workspace, &main, at)
                .into_iter()
                .find(|item| item.label == "entrydata")
                .unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!()
            };
            assert_eq!(
                edit.new_text,
                if snippets && tail.is_empty() {
                    "entrydata{${1:key}}{${2:code}}"
                } else {
                    "entrydata"
                }
            );
        }
    }
    let mut workspace = Workspace::default();
    let source = "\\newcommand\\entrydata[1]{}\\entrydata{custom}";
    let main = insert(&mut workspace, "main.tex", source);
    let at = workspace.documents[&main]
        .text
        .position(source.find("custom").unwrap() + 2);
    assert_eq!(
        signature::help(&workspace, &main, at).unwrap().signatures[0].label,
        "\\entrydata{#1}"
    );
}

#[test]
fn context_key_renames_respect_project_isolation_aliases_and_shared_meanings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")), format!("\\documentclass{{article}}\\addbibresource{{{name}.bib}}\\entrydata*{{legacy}}{{Literal legacy}}")).unwrap();
        std::fs::write(
            root.join(format!("{name}.bib")),
            "@book{owner,ids={legacy},title={Book}}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a.tex")).unwrap();
    let bib = Url::from_file_path(root.join("a.bib")).unwrap();
    let doc = &workspace.documents[&main];
    let at = doc
        .text
        .position(doc.text.source.find("{legacy}").unwrap() + 2);
    let changes = features::rename(&workspace, &main, at, "new-alias")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(
        apply(&doc.text.source, &changes[&main]),
        doc.text.source.replace("{legacy}", "{new-alias}")
    );
    assert_eq!(
        apply(&workspace.documents[&bib].text.source, &changes[&bib]),
        "@book{owner,ids={new-alias},title={Book}}"
    );
    std::fs::write(root.join("shared.tex"), "\\entrydata{legacy}{code}").unwrap();
    for name in ["a", "b"] {
        std::fs::write(
            root.join(format!("{name}.tex")),
            format!("\\documentclass{{article}}\\addbibresource{{{name}.bib}}\\input{{shared}}"),
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let shared = Url::from_file_path(root.join("shared.tex")).unwrap();
    assert!(features::rename(&workspace, &shared, Position::new(0, 14), "new-alias").is_err());
}

#[test]
fn data_contexts_do_not_claim_ordinary_undefined_citation_warning_positions() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "\\entrydata{missing}{Literal missing \\cite{missing}}\n";
    std::fs::write(&main, source).unwrap();
    let diagnostics = build_log::parse(
        &main,
        "Package biblatex Warning: Citation 'missing' on page 1 undefined on input line 1.\n",
    );
    let start = source.rfind("missing").unwrap();
    assert_eq!(diagnostics[&uri].len(), 1);
    assert_eq!(
        diagnostics[&uri][0].range,
        Text::new(source.into()).range(start, start + 7)
    );
}
