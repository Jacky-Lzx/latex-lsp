use latex_lsp::{
    compilation::build,
    language::features,
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;
use std::sync::atomic::AtomicBool;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("file:///saved-text/main.tex").unwrap();
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    workspace.open(uri.clone(), source.into(), 7);
    (workspace, uri)
}

fn updated(source: &str, edit: WorkspaceEdit, uri: &Url) -> String {
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
            OneOf::Left(edit) => edit.clone(),
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
                    text: edit.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
}

#[test]
fn shared_saved_names_and_separate_moving_names_have_independent_navigation() {
    let source = concat!(
        "\\SaveVerb{same}|\\label{same}\\UseVerb{same}|\n",
        "\\UseVerb{same}\\UseVerbatim{same}\\LUseVerbatim{same}\\BUseVerbatim{same}\n",
        "\\SaveGVerb{same}|Moving|\\UseMVerb{same}\\pUseMVerb{same}\n",
        "\\label{same}\\ref{same}\n",
        "\\begin{SaveVerbatim*}[showspaces=true]{block}\n\\UseVerb{block}\n\\end{SaveVerbatim*}\n\\UseVerb{block}\n",
    );
    let (workspace, uri) = setup(source);
    let doc = &workspace.documents[&uri];
    assert!(doc.syntax.diagnostics.is_empty());
    assert_eq!(doc.syntax.saved_text.len(), 2);
    assert_eq!(doc.syntax.saved_text_references.len(), 5);
    assert_eq!(doc.syntax.moving_text.len(), 1);
    assert_eq!(doc.syntax.moving_text_references.len(), 2);
    for (marker, definition, count) in [
        ("\\UseVerb{same}", "\\SaveVerb{same}", 5),
        ("\\LUseVerbatim{same}", "\\SaveVerb{same}", 5),
        ("\\pUseMVerb{same}", "\\SaveGVerb{same}", 3),
        ("\\ref{same}", "\\label{same}\\ref", 2),
        ("\\UseVerb{block}", "[showspaces=true]{block}", 2),
    ] {
        let at = doc
            .text
            .position(source.rfind(marker).unwrap() + marker.find('{').unwrap() + 1);
        let locations = features::definitions(&workspace, &uri, at);
        assert_eq!(locations.len(), 1, "{marker}");
        let expected = source.rfind(definition).unwrap() + definition.find('{').unwrap() + 1;
        assert_eq!(
            locations[0].range.start,
            doc.text.position(expected),
            "{marker}"
        );
        assert_eq!(
            features::references(&workspace, &uri, at, true).len(),
            count,
            "{marker}"
        );
        assert_eq!(
            features::references(&workspace, &uri, at, false).len(),
            count - 1,
            "{marker}"
        );
        assert!(features::hover(&workspace, &uri, at).is_some());
    }
    let inside = doc
        .text
        .position(source.find("\\UseVerb{same}|").unwrap() + 10);
    assert!(features::definitions(&workspace, &uri, inside).is_empty());
    assert!(features::prepare_rename(&workspace, &uri, inside)
        .unwrap()
        .is_none());
}

#[test]
fn bare_saved_arguments_are_grouped_on_rename_and_other_namespaces_stay_intact() {
    let source = concat!(
        "\\begin{SaveVerbatim}x\r\n\\label{x}\\UseVerb x\r\n\\end{SaveVerbatim}\r\n",
        "\\UseVerb x\\UseVerbatim x\\LUseVerbatim x\\BUseVerbatim x\r\n",
        "\\SaveGVerb{x}|Moving|\\UseMVerb{x}\\pUseMVerb{x}\r\n\\label{x}\\ref{x}\r\n",
    );
    for name in ["new name", "comma,key", "中文", " new ", " "] {
        let (workspace, uri) = setup(source);
        let doc = &workspace.documents[&uri];
        let at = doc
            .text
            .position(source.find("\\BUseVerbatim x").unwrap() + "\\BUseVerbatim ".len());
        let prepared = features::prepare_rename(&workspace, &uri, at)
            .unwrap()
            .unwrap();
        assert!(
            matches!(prepared, PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } if placeholder == "x")
        );
        let result = updated(
            source,
            features::rename(&workspace, &uri, at, name)
                .unwrap()
                .unwrap(),
            &uri,
        );
        let expected = source.replacen("SaveVerbatim}x", &format!("SaveVerbatim}}{{{name}}}"), 1).replacen("\\UseVerb x\\UseVerbatim x\\LUseVerbatim x\\BUseVerbatim x", &format!("\\UseVerb {{{name}}}\\UseVerbatim {{{name}}}\\LUseVerbatim {{{name}}}\\BUseVerbatim {{{name}}}"), 1);
        assert_eq!(result, expected);
        let renamed = Document::new(&uri, result, Some(8));
        assert!(renamed.syntax.diagnostics.is_empty());
        assert_eq!(renamed.syntax.saved_text[0].name, name);
        assert!(renamed
            .syntax
            .saved_text_references
            .iter()
            .all(|entry| entry.name == name));
        assert_eq!(renamed.syntax.moving_text[0].name, "x");
        assert_eq!(renamed.syntax.labels[0].name, "x");
    }
}

#[test]
fn exact_spaces_commas_and_unicode_are_names_and_completions_replace_the_whole_key() {
    let definitions = "\\SaveVerb{example}|A|\\SaveVerb{ example }|B|\\SaveVerb{a,b}|C|\\SaveVerb{中文}|D|\\SaveGVerb{moving}|E|\n";
    for name in ["example", " example ", "a,b", "中文"] {
        let source = format!("{definitions}\\UseVerb{{{name}}}");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        let at = doc.text.position(source.rfind(name).unwrap());
        assert_eq!(
            features::definitions(&workspace, &uri, at).len(),
            1,
            "{name}"
        );
        assert_eq!(features::references(&workspace, &uri, at, true).len(), 2);
        let result = updated(
            &source,
            features::rename(&workspace, &uri, at, "renamed 名称,key")
                .unwrap()
                .unwrap(),
            &uri,
        );
        assert_eq!(result.matches("renamed 名称,key").count(), 2);
    }
    for (call, expected) in [
        ("\\UseVerb{ex¦ample}", vec!["example"]),
        ("\\UseVerb{¦}", vec![" example ", "a,b", "example", "中文"]),
        ("\\UseMVerb{¦}", vec!["moving"]),
        ("\\pUseMVerb{mo¦ving}", vec!["moving"]),
        ("\\UseVerb{中¦", vec!["中文"]),
    ] {
        let marked = format!("{definitions}{call}");
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let (workspace, uri) = setup(&source);
        let items = features::completion(
            &workspace,
            &uri,
            workspace.documents[&uri].text.position(offset),
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            expected,
            "{call}"
        );
        if call.starts_with("\\UseVerb{ex") {
            let Some(CompletionTextEdit::Edit(edit)) = &items[0].text_edit else {
                panic!("missing edit")
            };
            assert_eq!(
                workspace.documents[&uri].text.range(
                    source.rfind("example").unwrap(),
                    source.rfind("example").unwrap() + 7
                ),
                edit.range
            );
        }
    }
}

#[test]
fn rename_rejects_ambiguous_dynamic_or_customized_names_and_keeps_namespaces_separate() {
    let base = "\\SaveVerb{old}|A|\\UseVerb{old}";
    for extra in [
        "\\SaveVerb{old}|B|",
        "\\begin{SaveVerbatim}{old}\nB\n\\end{SaveVerbatim}\n",
        "\\SaveVerb{taken}|B|",
        "\\UseVerb{\\dynamic}",
        "\\SaveVerb{\\dynamic}|B|",
        "\\SaveMVerb{moving}|B|",
        "\\renewcommand{\\UseVerb}[1]{}",
        "\\renewenvironment{SaveVerbatim}{}{}",
        "\\FV@SV@old",
        "\\CustomVerbatimCommand{\\custom}{SaveVerb}{}",
    ] {
        let source = format!("{base}\n{extra}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(base.rfind("old").unwrap());
        assert!(
            features::rename(&workspace, &uri, at, "taken").is_err(),
            "{extra}"
        );
    }
    for extra in [
        "\\SaveGVerb{taken}|B|\\UseMVerb{taken}",
        "\\SaveGVerb{\\dynamic}|B|",
        "\\label{taken}\\ref{taken}",
        "\\newcommand{\\taken}{}",
    ] {
        let source = format!("{base}\n{extra}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(base.rfind("old").unwrap());
        assert!(
            features::rename(&workspace, &uri, at, "taken")
                .unwrap()
                .is_some(),
            "{extra}"
        );
    }
    let (workspace, uri) = setup(base);
    let at = workspace.documents[&uri]
        .text
        .position(base.rfind("old").unwrap());
    for invalid in [
        "",
        "\\dynamic",
        "bad%name",
        "bad\nname",
        "{name}",
        "bad~name",
    ] {
        assert!(
            features::rename(&workspace, &uri, at, invalid).is_err(),
            "{invalid:?}"
        );
    }
}

#[test]
fn completing_a_bare_name_groups_the_replacement() {
    let source = r"\SaveVerb{long name}|Text|\UseVerb l";
    let (workspace, uri) = setup(source);
    let items = features::completion(
        &workspace,
        &uri,
        workspace.documents[&uri].text.position(source.len()),
    );
    assert_eq!(items.len(), 1);
    let Some(CompletionTextEdit::Edit(edit)) = items[0].text_edit.clone() else {
        panic!("missing edit")
    };
    let result = Text::new(source.into())
        .changed(vec![TextDocumentContentChangeEvent {
            range: Some(edit.range),
            range_length: None,
            text: edit.new_text,
        }])
        .unwrap();
    assert!(result.source.ends_with(r"\UseVerb {long name}"));
}

#[test]
fn block_header_comments_and_options_remain_unchanged_when_saved_names_are_renamed() {
    let header = "% header\r\n[label={\\detokenize{\\end{SaveVerbatim*}}}]% name\r\n";
    let body = "\\UseVerb{old}\\label{old}\r\n";
    let source = format!("\\begin{{SaveVerbatim*}}{header}{{old}}\r\n{body}\\end{{SaveVerbatim*}}\r\n\\UseVerbatim *[formatcom={{\\small}}]{{old}}");
    let (workspace, uri) = setup(&source);
    let doc = &workspace.documents[&uri];
    assert!(doc.syntax.diagnostics.is_empty());
    let at = doc.text.position(source.rfind("old").unwrap());
    assert_eq!(features::definitions(&workspace, &uri, at).len(), 1);
    assert_eq!(features::references(&workspace, &uri, at, true).len(), 2);
    let result = updated(
        &source,
        features::rename(&workspace, &uri, at, "new name")
            .unwrap()
            .unwrap(),
        &uri,
    );
    assert!(result.contains(header) && result.contains(body));
    assert_eq!(result.matches("new name").count(), 2);
}

#[test]
fn executable_catcodes_and_direct_namespace_mutations_block_saved_text_rename() {
    let base = "\\SaveVerb{old}|Literal|\\UseVerb{old}";
    for extra in [
        r"\fvset{commandchars=abc}",
        r"\Verb[commandchars=abc]|Text|",
        r"\SaveGVerb[codes={\relax}]{moving}|Text|",
        r"\csname FV@SV@old\endcsname",
        r"\csuse{UseVerb}{old}",
        r"\csname UseVerb\endcsname{old}",
        r"\setkeys{FV}{commandchars=abc}",
        r"\define@key{FV}{custom}{}",
        "\\begin{Verbatim}[commandchars=abc]\nLiteral\n\\end{Verbatim}\n",
    ] {
        let source = format!("{base}\n{extra}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(base.rfind("old").unwrap());
        assert!(
            features::rename(&workspace, &uri, at, "new name").is_err(),
            "{extra}"
        );
    }
    for extra in [
        r"\fvset{commandchars=none}",
        r"\fvset{commandchars={none},formatcom=\small}",
        r"\Verb[codes={},commandchars=none]|Text|",
    ] {
        let source = format!("{base}\n{extra}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(base.rfind("old").unwrap());
        assert!(
            features::rename(&workspace, &uri, at, "new name")
                .unwrap()
                .is_some(),
            "{extra}"
        );
    }
}

#[test]
fn shared_projects_expand_saved_text_edits_and_check_other_consumers() {
    use std::collections::BTreeSet;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let paper = "\\documentclass{article}\\usepackage{common}\\begin{document}\\UseVerb{shared}\\end{document}";
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")), paper).unwrap();
    }
    std::fs::write(root.join("common.sty"), r"\SaveVerb{shared}|Literal|").unwrap();
    std::fs::write(
        root.join("other.tex"),
        r"\documentclass{article}\SaveVerb{shared}|Other|\UseVerb{shared}",
    )
    .unwrap();
    let main = Url::from_file_path(root.join("a.tex")).unwrap();
    let b = Url::from_file_path(root.join("b.tex")).unwrap();
    let shared = Url::from_file_path(root.join("common.sty")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let at = workspace.documents[&main]
        .text
        .position(paper.rfind("shared").unwrap());
    let changes = features::rename(&workspace, &main, at, "renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        changes.keys().cloned().collect::<BTreeSet<_>>(),
        [main.clone(), b.clone(), shared.clone()].into()
    );
    assert_eq!(features::references(&workspace, &main, at, true).len(), 3);
    let definition = workspace.documents[&shared]
        .text
        .position("\\SaveVerb{".len());
    assert_eq!(
        features::rename(&workspace, &shared, definition, "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()
            .len(),
        3
    );
    for extra in [
        r"\SaveVerb{renamed}|Taken|",
        r"\SaveVerb{shared}|Override|",
        r"\UseVerb{\dynamic}",
    ] {
        workspace.documents.insert(
            b.clone(),
            Document::new(&b, format!("{paper}{extra}"), None),
        );
        assert!(
            features::rename(&workspace, &main, at, "renamed").is_err(),
            "{extra}"
        );
    }
    let shared_ref = Url::from_file_path(root.join("shared.tex")).unwrap();
    std::fs::write(root.join("shared.tex"), r"\UseVerb{shared}").unwrap();
    for name in ["a", "b"] {
        std::fs::write(
            root.join(format!("{name}.tex")),
            r"\documentclass{article}\SaveVerb{shared}|Own|\input{shared}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let at = workspace.documents[&main]
        .text
        .position("\\documentclass{article}\\SaveVerb{".len());
    assert!(features::rename(&workspace, &main, at, "renamed").is_err());
    let at = workspace.documents[&shared_ref]
        .text
        .position("\\UseVerb{".len());
    assert!(features::prepare_rename(&workspace, &shared_ref, at).is_err());
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_saved_key_renames_keep_compilation_and_literal_bodies_intact() {
    for engine in ["pdflatex", "xelatex"] {
        for form in ["inline", "block", "moving"] {
            for name in ["x", "example", "ex ample", "a,b", "中文"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let uri = Url::from_file_path(root.join("main.tex")).unwrap();
                let argument = if name == "x" {
                    " x".into()
                } else {
                    format!("{{{name}}}")
                };
                let body = r"LiteralUnique \label{phantom}\input{fake}\ref{kept}";
                let saved = match form {
                    "inline" => format!("\\SaveVerb{argument}|{body}|\\UseVerb{argument}"),
                    "block" => format!("\\begin{{SaveVerbatim}}{argument}\r\n{body}\r\n\\end{{SaveVerbatim}}\r\n\\UseVerbatim{argument}\\LUseVerbatim{argument}\\BUseVerbatim{argument}"),
                    _ => format!("\\SaveGVerb{argument}|{body}|\\UseMVerb{argument}\\pUseMVerb{argument}"),
                };
                let source = format!("\\documentclass{{article}}\r\n\\usepackage{{fancyvrb}}\r\n\\begin{{document}}\\label{{kept}}\r\n{saved}\r\nReal.\\ref{{kept}}\\end{{document}}\r\n");
                let mut workspace = Workspace {
                    versioned_edits: true,
                    ..Default::default()
                };
                workspace.add_root(root.into());
                workspace.open(uri.clone(), source.clone(), 7);
                let doc = &workspace.documents[&uri];
                let at = doc.text.position(source.rfind(name).unwrap());
                let result = updated(
                    &source,
                    features::rename(&workspace, &uri, at, " renamed 名称,key ")
                        .unwrap()
                        .unwrap(),
                    &uri,
                );
                assert!(result.contains(body));
                for content in [&source, &result] {
                    std::fs::write(root.join("main.tex"), content).unwrap();
                    let outcome = build::run(
                        &root.join("main.tex"),
                        &latex_lsp::config::BuildConfig {
                            executable: engine.into(),
                            ..Default::default()
                        },
                        &AtomicBool::new(false),
                    );
                    assert_eq!(
                        outcome.status, "success",
                        "{engine}/{form}/{name}: {}",
                        outcome.output
                    );
                    let aux =
                        std::fs::read_to_string(root.join("build/latex-lsp/main.aux")).unwrap();
                    assert_eq!(aux.matches("newlabel{kept}").count(), 1, "{aux}");
                    assert!(!aux.contains("newlabel{phantom}"), "{aux}");
                }
            }
        }
    }
}
