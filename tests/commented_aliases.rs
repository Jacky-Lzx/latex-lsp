use latex_lsp::{
    language::{features, semantic},
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///commented-aliases/{name}")).unwrap();
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
fn commented_alias_setters_navigate_and_join_citation_renames_without_editing_alias_text() {
    for alias in [
        "\\defcitealias{% before alpha, }\nalpha% tail alpha\n}{The alpha alias}",
        "\\defcitealias{% before alpha\r\nalpha}{The alpha alias}",
        "\\defcitealias{alpha% tail alpha\n}{The alpha alias}",
        "\\defcitealias{% before\n% another\nalpha% tail\n% another\n}{The alpha alias}",
    ] {
        let mut workspace = Workspace::default();
        let bibliography = "@book{alpha,title={Literal alpha}}\n";
        let bib = insert(&mut workspace, "refs.bib", bibliography);
        let aliases = insert(&mut workspace, "aliases.tex", alias);
        let source = "\\citetalias{alpha}\\citep[alpha]{alpha}\\label{alpha}\\verb|\\defcitealias{alpha}{Literal}|";
        let main = insert(&mut workspace, "main.tex", source);
        let offset = alias
            .find("\nalpha")
            .map(|at| at + 1)
            .unwrap_or_else(|| alias.find("{alpha").unwrap() + 1);
        let at = Text::new(alias.into()).position(offset + 2);
        let definitions = features::definitions(&workspace, &aliases, at);
        assert_eq!(definitions.len(), 1, "{alias}");
        assert_eq!(definitions[0].uri, bib);
        assert!(features::hover(&workspace, &aliases, at).is_some());
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, &aliases, at).unwrap()
        else {
            panic!("missing rename target: {alias}")
        };
        assert_eq!(placeholder, "alpha");
        assert_eq!(range, Text::new(alias.into()).range(offset, offset + 5));
        assert_eq!(
            features::references(&workspace, &aliases, at, true).len(),
            4
        );
        let changes = features::rename(&workspace, &aliases, at, "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[&aliases].len(), 1);
        let mut expected = alias.to_string();
        expected.replace_range(offset..offset + 5, "renamed");
        assert_eq!(apply(alias, &changes[&aliases]), expected);
        assert_eq!(
            apply(bibliography, &changes[&bib]),
            "@book{renamed,title={Literal alpha}}\n"
        );
        assert_eq!(
            apply(source, &changes[&main]),
            source
                .replace("\\citetalias{alpha}", "\\citetalias{renamed}")
                .replace("\\citep[alpha]{alpha}", "\\citep[alpha]{renamed}")
        );
        let body_at = Text::new(alias.into()).position(alias.find("The alpha").unwrap() + 6);
        assert!(features::definitions(&workspace, &aliases, body_at).is_empty());
        assert!(features::prepare_rename(&workspace, &aliases, body_at)
            .unwrap()
            .is_none());
    }
}

#[test]
fn commented_alias_completion_preserves_comments_and_body_and_removes_only_old_key_padding() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "refs.bib", "@book{alpha,title={Title}}");
    for (marked, expected) in [
        (
            "\\defcitealias{% alpha, }\r\n al¦BAD % tail alpha\r\n}{Alias alpha}",
            "\\defcitealias{% alpha, }\r\nalpha% tail alpha\r\n}{Alias alpha}",
        ),
        (
            "\\defcitealias{ \n% before\n al¦BAD % tail\n \n}{Alias alpha}",
            "\\defcitealias{% before\nalpha% tail\n}{Alias alpha}",
        ),
        (
            "\\defcitealias{% before\n¦}{Alias alpha}",
            "\\defcitealias{% before\nalpha}{Alias alpha}",
        ),
        (
            "\\defcitealias{¦% after\n}{Alias alpha}",
            "\\defcitealias{alpha% after\n}{Alias alpha}",
        ),
        (
            "\\defcitealias{% before\nal¦",
            "\\defcitealias{% before\nalpha",
        ),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "aliases.tex", &source);
        let item = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        )
        .into_iter()
        .find(|item| item.label == "alpha")
        .unwrap_or_else(|| panic!("missing completion: {marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        assert_eq!(edit.range.start.line, edit.range.end.line);
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        let mut ordered = edits.clone();
        ordered.sort_by_key(|edit| edit.range.start);
        assert!(ordered
            .windows(2)
            .all(|pair| pair[0].range.end <= pair[1].range.start));
        assert_eq!(apply(&source, &edits), expected, "{marked}");
    }
}

#[test]
fn alias_names_with_literal_spaces_macros_groups_or_comment_splices_are_not_merged() {
    let mut workspace = Workspace::default();
    let bib = insert(&mut workspace, "refs.bib", "@book{alpha,title={Title}}");
    insert(&mut workspace, "main.tex", "\\citep{alpha}");
    for (argument, dynamic) in [
        ("{% before\n alpha}", false),
        ("{alpha % after\n}", false),
        ("{% before\nalpha\n}", false),
        ("{al% middle\npha}", true),
        ("{\\chosen}", true),
        ("{{alpha}}", true),
        ("{alpha,beta}", true),
    ] {
        let source = format!("\\defcitealias{argument}{{The alpha alias}}");
        let aliases = insert(&mut workspace, "aliases.tex", &source);
        let offset = argument
            .find("alpha")
            .or_else(|| argument.find("al%"))
            .or_else(|| argument.find("chosen"))
            .unwrap()
            + "\\defcitealias".len()
            + 1;
        let at = Text::new(source.clone()).position(offset);
        assert!(features::prepare_rename(&workspace, &aliases, at)
            .unwrap()
            .is_none());
        let changes = features::rename(&workspace, &bib, Position::new(0, 8), "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert!(!changes.contains_key(&aliases), "{source}: {changes:?}");
        if dynamic {
            // Literal padding can be corrected by completion, but dynamic
            // expressions and multiple literal pieces cannot be one key edit.
            assert!(!features::completion(&workspace, &aliases, at)
                .iter()
                .any(|item| item.label == "alpha"));
        }
    }
}

#[test]
fn alias_semantic_tokens_use_the_actual_utf16_key_range_and_leave_display_text_plain() {
    let source = "😀 \\defcitealias{% before 键😀\r\n键😀% tail\r\n}{Alias 键😀}\r\n";
    let mut workspace = Workspace::default();
    let main = insert(&mut workspace, "aliases.tex", source);
    let doc = &workspace.documents[&main];
    let legend = semantic::options().legend;
    let mut position = Position::default();
    let mut keys = Vec::new();
    for token in semantic::tokens(doc, None).unwrap().data {
        position = Position::new(
            position.line + token.delta_line,
            if token.delta_line == 0 {
                position.character + token.delta_start
            } else {
                token.delta_start
            },
        );
        if legend.token_types[token.token_type as usize].as_str() == "enumMember" {
            keys.push((position, token.length));
        }
    }
    assert_eq!(keys, [(Position::new(1, 0), 3)]);
}

#[test]
#[ignore = "requires pdflatex"]
fn real_alias_completion_and_three_file_citation_rename_keep_the_alias_working() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let aliases = directory.path().join("aliases.tex");
    let child = directory.path().join("child.tex");
    let main_source = "\\documentclass{article}\n\\usepackage{natbib}\n\\input{aliases}\n\\begin{document}\n\\input{child}\n\\begin{thebibliography}{1}\n\\bibitem[Author(2025)]{alpha} Author, Title, 2025.\n\\end{thebibliography}\n\\end{document}\n";
    let alias_source = "\\defcitealias{% before alpha\n alBAD % tail alpha\n}{The ALIAS alpha}\n";
    let child_source = "See \\citetalias{alpha} and \\citep{alpha}.\n";
    let mut workspace = Workspace::default();
    for (path, source) in [
        (&main, main_source),
        (&aliases, alias_source),
        (&child, child_source),
    ] {
        std::fs::write(path, source).unwrap();
        workspace.open(Url::from_file_path(path).unwrap(), source.into(), 7);
    }
    let alias_uri = Url::from_file_path(&aliases).unwrap();
    let item = features::completion(&workspace, &alias_uri, Position::new(1, 3))
        .into_iter()
        .find(|item| item.label == "alpha")
        .unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
        panic!("missing edit")
    };
    let mut edits = item.additional_text_edits.unwrap_or_default();
    edits.push(edit);
    let completed = apply(alias_source, &edits);
    assert_eq!(completed, alias_source.replace("\n alBAD %", "\nalpha%"));
    std::fs::write(&aliases, &completed).unwrap();
    workspace.open(alias_uri.clone(), completed.clone(), 8);
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let compile = || {
        let result = latex_lsp::compilation::build::run(&main, &config, &Default::default());
        assert_eq!(result.status, "success", "{}", result.output);
        result
    };
    compile();
    assert!(compile().diagnostics.values().all(Vec::is_empty));
    let changes = features::rename(&workspace, &alias_uri, Position::new(1, 2), "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 3);
    for (uri, edits) in changes {
        let source = &workspace.documents[&uri].text.source;
        std::fs::write(uri.to_file_path().unwrap(), apply(source, &edits)).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&aliases).unwrap(),
        completed.replace("\nalpha%", "\nrenamed%")
    );
    compile();
    let result = compile();
    assert!(result.diagnostics.values().all(Vec::is_empty));
    let aux = std::fs::read_to_string(result.pdf.unwrap().with_extension("aux")).unwrap();
    assert!(aux.contains("\\bibcite{renamed}"));
    assert!(!aux.contains("\\bibcite{alpha}"));
}
