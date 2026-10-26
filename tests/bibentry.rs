use latex_lsp::{
    compilation::build_log,
    editing,
    language::{features, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///bibentry/{name}")).unwrap();
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
fn inline_bibliography_references_join_citation_renames_and_preserve_text_and_labels() {
    for marked in [
        r"\bibentry{al¦pha}",
        "\\bibentry{% before alpha, }\r\nal¦pha% after alpha\r\n}",
        "\\bibentry{al¦pha% after alpha\n}",
    ] {
        let mut workspace = Workspace::default();
        let bibliography = "@book{alpha,title={Literal alpha}}\n";
        let bib = insert(&mut workspace, "refs.bib", bibliography);
        let source = marked.replace('¦', "");
        let inline = insert(&mut workspace, "inline.tex", &source);
        let main_source = "\\citep{alpha}\\defcitealias{alpha}{Alias alpha}\\label{alpha}\\verb|\\bibentry{alpha}|% \\bibentry{alpha}\n";
        let main = insert(&mut workspace, "main.tex", main_source);
        let at = Text::new(source.clone()).position(marked.find('¦').unwrap());
        let definitions = features::definitions(&workspace, &inline, at);
        assert_eq!(definitions.len(), 1, "{marked}");
        assert_eq!(definitions[0].uri, bib);
        assert!(features::hover(&workspace, &inline, at).is_some());
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, &inline, at).unwrap()
        else {
            panic!("missing rename target: {marked}");
        };
        assert_eq!(placeholder, "alpha");
        let text = &workspace.documents[&inline].text;
        assert_eq!(
            &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
            "alpha"
        );
        assert_eq!(features::references(&workspace, &inline, at, true).len(), 4);
        let changes = features::rename(&workspace, &inline, at, "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[&inline].len(), 1);
        let key_start = marked.find('¦').unwrap() - 2;
        let mut expected = source.clone();
        expected.replace_range(key_start..key_start + 5, "renamed");
        assert_eq!(apply(&source, &changes[&inline]), expected);
        assert_eq!(
            apply(bibliography, &changes[&bib]),
            "@book{renamed,title={Literal alpha}}\n"
        );
        assert_eq!(apply(main_source, &changes[&main]), "\\citep{renamed}\\defcitealias{renamed}{Alias alpha}\\label{alpha}\\verb|\\bibentry{alpha}|% \\bibentry{alpha}\n");
    }
}

#[test]
fn inline_bibliography_rejects_lists_dynamic_keys_and_unsupported_call_shapes() {
    let mut workspace = Workspace::default();
    let bib = insert(&mut workspace, "refs.bib", "@book{alpha,title={Title}}");
    for marked in [
        r"\bibentry[Note]{al¦pha}",
        r"\bibentry*{al¦pha}",
        r"\bibentry *{al¦pha}",
        r"\bibentry{al¦pha,beta}",
        r"\bibentry{ al¦pha }",
        r"\bibentry{{al¦pha}}",
        r"\bibentry{al¦pha\suffix}",
        "\\bibentry{al¦% joined\npha}",
        r"\bibentry{alpha} Body al¦pha",
    ] {
        let source = marked.replace('¦', "");
        let uri = insert(&mut workspace, "invalid.tex", &source);
        let at = Text::new(source).position(marked.find('¦').unwrap());
        assert!(
            features::prepare_rename(&workspace, &uri, at)
                .unwrap()
                .is_none(),
            "{marked}"
        );
        let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        if marked.contains("Body") {
            assert_eq!(changes[&uri].len(), 1);
        } else {
            assert!(!changes.contains_key(&uri), "{marked}: {changes:?}");
        }
    }
}

#[test]
fn inline_bibliography_completion_and_hints_use_one_key_and_preserve_comment_boundaries() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "refs.bib", "@book{alpha,title={Title}}");
    for (marked, expected) in [
        (
            "\\bibentry{% alpha, }\r\n al¦BAD % tail alpha\r\n} Body",
            "\\bibentry{% alpha, }\r\nalpha% tail alpha\r\n} Body",
        ),
        (
            "\\bibentry{% before\n¦} Body",
            "\\bibentry{% before\nalpha} Body",
        ),
        ("\\bibentry{% before\nal¦", "\\bibentry{% before\nalpha"),
    ] {
        let source = marked.replace('¦', "");
        let uri = insert(&mut workspace, "inline.tex", &source);
        let at = Text::new(source.clone()).position(marked.find('¦').unwrap());
        let item = features::completion(&workspace, &uri, at)
            .into_iter()
            .find(|item| item.label == "alpha")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit");
        };
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        assert_eq!(apply(&source, &edits), expected);
        let help = signature::help(&workspace, &uri, at).unwrap();
        assert_eq!(help.signatures[0].label, "\\bibentry{key}");
        assert_eq!(help.active_parameter, Some(0));
    }
    for marked in [
        r"\bibentry{alpha} En¦try",
        r"\bibentry[Note]{al¦pha}",
        r"\bibentry*{al¦pha}",
    ] {
        let source = marked.replace('¦', "");
        let uri = insert(&mut workspace, "inline.tex", &source);
        let at = Text::new(source).position(marked.find('¦').unwrap());
        assert!(signature::help(&workspace, &uri, at).is_none(), "{marked}");
    }
}

#[test]
fn inline_bibliography_warnings_use_the_static_single_key_and_offer_a_precise_fix() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "\\bibitem{alpha} Entry.\n\\bibentry{% before😀\r\nalphaa% tail\r\n}\n";
    std::fs::write(&main, source).unwrap();
    let log = format!(
        "{}:4: Package natbib Warning: Citation `alphaa' undefined on input line 4.\n",
        main.display()
    );
    let diagnostics = build_log::parse(&main, &log);
    let diagnostic = &diagnostics[&uri][0];
    assert_eq!(
        diagnostic.range,
        Range::new(Position::new(2, 0), Position::new(2, 6))
    );
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            code_action_literals: true,
            ..Default::default()
        },
        ..Workspace::default()
    };
    workspace.open(uri.clone(), source.into(), 7);
    workspace
        .project
        .build_diagnostics
        .insert(uri.clone(), diagnostics.clone());
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            only: None,
            trigger_kind: None,
        },
    );
    let edit = actions
        .iter()
        .find_map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => action.edit.as_ref()?.changes.as_ref()?
                [&uri]
                .iter()
                .find(|edit| edit.new_text == "alpha"),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{actions:?}"));
    assert_eq!(edit.range, diagnostic.range);
    assert_eq!(
        apply(source, std::slice::from_ref(edit)),
        source.replace("\nalphaa%", "\nalpha%")
    );
    for invalid in [
        r"\bibentry{alpha,alphaa}",
        r"\bibentry{\prefix alphaa}",
        r"\bibentry[Note]{alphaa}",
        r"\defcitealias{alphaa}{Text}",
    ] {
        std::fs::write(&main, invalid).unwrap();
        let log = format!(
            "{}:1: Package natbib Warning: Citation `alphaa' undefined on input line 1.\n",
            main.display()
        );
        let diagnostics = build_log::parse(&main, &log);
        assert_eq!(
            diagnostics[&uri][0].range,
            Range::new(Position::new(0, 0), Position::new(0, invalid.len() as u32)),
            "{invalid}"
        );
    }
}
