use latex_lsp::{
    build,
    config::BuildConfig,
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use std::{collections::BTreeSet, sync::atomic::AtomicBool};
use tower_lsp::lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("file:///saved-name-commands/main.tex").unwrap();
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    workspace.open(uri.clone(), source.into(), 7);
    (workspace, uri)
}

fn changed(source: &str, edit: WorkspaceEdit, uri: &Url) -> String {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edits")
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *uri);
    assert_eq!(documents[0].text_document.version, Some(7));
    let mut edits: Vec<_> = documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit,
            _ => panic!("unexpected annotated edit"),
        })
        .collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .map(|edit| TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text.clone(),
                })
                .collect(),
        )
        .unwrap()
        .source
}

fn forms() -> Vec<String> {
    [
        r"\SaveVerb{\keyname}|MarkerUnique|\UseVerb{example}",
        r"\SaveVerb{example}|MarkerUnique|\UseVerb{\keyname}",
        r"\SaveVerb{example}|MarkerUnique|\UseVerb \keyname",
        r"\SaveVerb \keyname|MarkerUnique|\UseVerb{example}",
        "\\begin{SaveVerbatim}{\\keyname}\r\nMarkerUnique\r\n\\end{SaveVerbatim}\r\n\\UseVerbatim{example}",
        "\\begin{SaveVerbatim*}{\\keyname}\r\nMarkerUnique\r\n\\end{SaveVerbatim*}\r\n\\UseVerbatim{example}",
        "\\begin{SaveVerbatim}{example}\r\nMarkerUnique\r\n\\end{SaveVerbatim}\r\n\\UseVerbatim{\\keyname}",
        r"\SaveVerb{example}|MarkerUnique|\LUseVerbatim{\keyname}",
        r"\SaveVerb{example}|MarkerUnique|\BUseVerbatim{\keyname}",
        r"\SaveGVerb{\keyname}|MarkerUnique|\UseMVerb{example}",
        r"\SaveGVerb{example}|MarkerUnique|\UseMVerb{\keyname}",
        r"\SaveGVerb{example}|MarkerUnique|\pUseMVerb{\keyname}",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

#[test]
fn expanded_name_commands_support_navigation_and_versioned_rename_in_every_form() {
    for form in forms() {
        let source = format!("% 😀\r\n\\newcommand{{\\keyname}}{{example}}\r\n{form}");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        assert_eq!(doc.syntax.name_command_references.len(), 1, "{form}");
        let entry = &doc.syntax.name_command_references[0];
        assert_eq!(entry.name, "keyname");
        assert_eq!(&source[entry.span.clone()], r"\keyname");
        assert!(!doc
            .syntax
            .commands
            .iter()
            .any(|command| command.span == entry.span));
        let at = doc.text.position(entry.span.start + 2);
        assert_eq!(
            features::definitions(&workspace, &uri, at),
            vec![Location {
                uri: uri.clone(),
                range: doc.text.range(
                    doc.syntax.definitions[0].span.start,
                    doc.syntax.definitions[0].span.end,
                ),
            }],
            "{form}"
        );
        assert_eq!(features::references(&workspace, &uri, at, true).len(), 2);
        assert_eq!(features::references(&workspace, &uri, at, false).len(), 1);
        assert!(features::hover(&workspace, &uri, at).is_some());
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, &uri, at).unwrap()
        else {
            panic!("missing prepare rename: {form}")
        };
        assert_eq!(placeholder, "keyname");
        assert_eq!(range, doc.text.range(entry.span.start + 1, entry.span.end));
        let result = changed(
            &source,
            features::rename(&workspace, &uri, at, r"\renamedkey")
                .unwrap()
                .unwrap(),
            &uri,
        );
        assert_eq!(result, source.replace(r"\keyname", r"\renamedkey"));
        assert!(features::rename(&workspace, &uri, at, "bad-name").is_err());
    }
}

#[test]
fn literal_csname_and_csuse_references_edit_names_without_inserting_a_slash() {
    for argument in [r"\csname keyname\endcsname", r"\csuse{keyname}"] {
        let source = format!("\\newcommand{{\\keyname}}{{example}}\n\\UseVerb{{{argument}}}");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        let start = source.rfind("keyname").unwrap();
        let at = doc.text.position(start + 2);
        assert_eq!(features::definitions(&workspace, &uri, at).len(), 1);
        assert_eq!(features::references(&workspace, &uri, at, true).len(), 2);
        assert!(matches!(
            features::prepare_rename(&workspace, &uri, at).unwrap(),
            Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder })
                if range == doc.text.range(start, start + 7) && placeholder == "keyname"
        ));
        let result = changed(
            &source,
            features::rename(&workspace, &uri, at, "renamedkey")
                .unwrap()
                .unwrap(),
            &uri,
        );
        assert_eq!(result, source.replace("keyname", "renamedkey"));
    }
}

#[test]
fn spaces_in_constructed_command_names_do_not_match_an_ordinary_control_word() {
    for argument in [r"\csname keyname \endcsname", r"\csuse{ keyname }"] {
        let source = format!("\\newcommand{{\\keyname}}{{example}}\n\\UseVerb{{{argument}}}");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        let at = doc.text.position(source.find(r"\keyname").unwrap() + 2);
        assert_eq!(features::references(&workspace, &uri, at, true).len(), 1);
        assert!(features::definitions(
            &workspace,
            &uri,
            doc.text.position(source.rfind("keyname").unwrap() + 2)
        )
        .is_empty());
    }
}

#[test]
fn literal_conversion_comments_and_bodies_remain_opaque() {
    let source = concat!(
        "\\newcommand{\\keyname}{example}\r\n",
        "\\SaveVerb{\\detokenize{\\keyname}}|\\keyname|\r\n",
        "\\UseVerb{\\string\\keyname\\space}\r\n",
        "\\UseVerb{% 😀 \\keyname\r\n\\keyname}\r\n",
        "\\begin{SaveVerbatim}{example}\r\n\\keyname\r\n\\end{SaveVerbatim}\r\n",
    );
    let (workspace, uri) = setup(source);
    let doc = &workspace.documents[&uri];
    let at = doc.text.position(source.find(r"\keyname").unwrap() + 2);
    assert_eq!(features::references(&workspace, &uri, at, true).len(), 2);
    let result = changed(
        source,
        features::rename(&workspace, &uri, at, "renamedkey")
            .unwrap()
            .unwrap(),
        &uri,
    );
    let expected = source
        .replacen(r"\newcommand{\keyname}", r"\newcommand{\renamedkey}", 1)
        .replacen("\r\n\\keyname}", "\r\n\\renamedkey}", 1);
    assert_eq!(result, expected);
    for marker in [
        r"\detokenize{\keyname",
        r"|\keyname",
        r"\string\keyname",
        "😀 \\keyname",
        "\r\n\\keyname\r\n",
    ] {
        let offset = source.find(marker).unwrap() + marker.find(r"\keyname").unwrap() + 2;
        let at = doc.text.position(offset);
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{marker}"
        );
        assert!(
            features::prepare_rename(&workspace, &uri, at)
                .unwrap()
                .is_none(),
            "{marker}"
        );
        assert!(
            features::completion(&workspace, &uri, at).is_empty(),
            "{marker}"
        );
    }
}

#[test]
fn string_skips_one_token_and_detokenize_skips_a_balanced_group() {
    for (argument, count) in [
        (r"\string\keyname", 0),
        (r"\string{\keyname}", 1),
        (r"\string\%\keyname", 1),
        (
            r"\string% \keyname hidden
\keyname",
            0,
        ),
        (r"\detokenize{{\keyname}\keyname}\keyname", 1),
        (
            r"\detokenize% \keyname hidden
{\keyname}\keyname",
            1,
        ),
        (r"\detokenize\keyname", 1),
    ] {
        let source = format!("\\newcommand{{\\keyname}}{{example}}\n\\UseVerb{{{argument}}}");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        let at = doc.text.position(source.find(r"\keyname").unwrap() + 2);
        assert_eq!(
            features::references(&workspace, &uri, at, false).len(),
            count,
            "{argument}"
        );
    }
}

#[test]
fn name_data_does_not_create_latex_declarations_labels_or_dependencies() {
    let source = r"\UseVerb{\newcommand{\phantom}{Text}\label{fake}\input{missing}\begin{fake}}";
    let (workspace, uri) = setup(source);
    let doc = &workspace.documents[&uri];
    assert!(doc.syntax.definitions.is_empty());
    assert!(doc.syntax.labels.is_empty());
    assert!(workspace.dependencies(&uri).is_empty());
    assert!(doc.syntax.environments.is_empty());
    assert_eq!(doc.syntax.commands.len(), 1);
    let symbols = latex_lsp::navigation::workspace_symbols(&workspace, "");
    assert!(symbols.is_empty());
}

#[test]
fn direct_command_completion_replaces_the_word_inside_saved_name_arguments() {
    for form in [
        r"\UseVerb{\keyn¦ame}",
        r"\SaveVerb{\keyn¦ame}|Body|",
        r"\begin{SaveVerbatim}{\keyn¦ame}
Body
\end{SaveVerbatim}",
        r"\UseVerb{\keyn¦",
    ] {
        let marked = format!("\\newcommand{{\\keyname}}{{example}}\r\n{form}");
        let cursor = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        let items = features::completion(&workspace, &uri, doc.text.position(cursor));
        let item = items
            .iter()
            .find(|item| item.label == "keyname")
            .unwrap_or_else(|| panic!("{form}: {items:?}"));
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing completion edit")
        };
        let start = source.rfind(r"\keyn").unwrap() + 1;
        assert_eq!(
            edit.range,
            doc.text
                .range(start, cursor + if form.contains("¦ame") { 3 } else { 0 })
        );
        assert_eq!(edit.new_text, "keyname");
    }
}

#[test]
fn shared_macro_references_in_names_expand_scope_and_check_each_consumers_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let paper = "\\documentclass{article}\\usepackage{common}\\begin{document}\\UseVerb{\\keyname}\\end{document}";
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")), paper).unwrap();
    }
    std::fs::write(root.join("common.sty"), r"\newcommand{\keyname}{example}").unwrap();
    let main = Url::from_file_path(root.join("a.tex")).unwrap();
    let other = Url::from_file_path(root.join("b.tex")).unwrap();
    let shared = Url::from_file_path(root.join("common.sty")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let at = workspace.documents[&main]
        .text
        .position(paper.find(r"\keyname").unwrap() + 2);
    assert_eq!(features::references(&workspace, &main, at, true).len(), 3);
    let edits = features::rename(&workspace, &main, at, "renamedkey")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits.keys().cloned().collect::<BTreeSet<_>>(),
        [main.clone(), other.clone(), shared].into()
    );
    workspace.documents.insert(
        other.clone(),
        Document::new(
            &other,
            format!("{paper}\\newcommand{{\\renamedkey}}{{Taken}}"),
            None,
        ),
    );
    assert!(features::rename(&workspace, &main, at, "renamedkey").is_err());
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_macro_renames_in_expanded_names_preserve_successful_compilation() {
    let mut cases = forms();
    for argument in [
        r"\csname keyname\endcsname",
        r"\csuse{keyname}",
        r"\string\keyname\space",
    ] {
        let name = if argument.starts_with(r"\string") {
            r"\detokenize{\keyname}"
        } else {
            "example"
        };
        cases.push(format!(
            "\\SaveVerb{{{name}}}|MarkerUnique|\\UseVerb{{{argument}}}"
        ));
    }
    for engine in ["pdflatex", "xelatex"] {
        for form in &cases {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\documentclass{{article}}\r\n\\usepackage{{fancyvrb}}\r\n\\usepackage{{etoolbox}}\r\n\\newcommand{{\\keyname}}{{example}}\r\n\\begin{{document}}\r\n{form}\r\n\\end{{document}}\r\n");
            let (workspace, uri) = setup(&source);
            let at = workspace.documents[&uri]
                .text
                .position(source.find(r"\keyname").unwrap() + 2);
            let result = changed(
                &source,
                features::rename(&workspace, &uri, at, "renamedkey")
                    .unwrap()
                    .unwrap(),
                &uri,
            );
            for content in [&source, &result] {
                std::fs::write(root.join("main.tex"), content).unwrap();
                let outcome = build::run(
                    &root.join("main.tex"),
                    &BuildConfig {
                        executable: engine.into(),
                        ..Default::default()
                    },
                    &AtomicBool::new(false),
                );
                assert_eq!(
                    outcome.status, "success",
                    "{engine}/{form}: {}",
                    outcome.output
                );
                assert!(root.join("build/latex-lsp/main.pdf").is_file());
            }
        }
    }
}
