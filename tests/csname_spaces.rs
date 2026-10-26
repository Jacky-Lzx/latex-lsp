use latex_lsp::{
    compilation::build,
    config::BuildConfig,
    language::{features, signature},
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;
use std::sync::atomic::AtomicBool;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("file:///csname-spaces/main.tex").unwrap();
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.open(uri.clone(), source.into(), 7);
    (workspace, uri)
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

fn renamed(source: &str) -> String {
    let (workspace, uri) = setup(source);
    let at = workspace.documents[&uri]
        .text
        .position(source.find(r"\keyname").unwrap() + 2);
    let edit = features::rename(&workspace, &uri, at, "renamedkey")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edits")
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, uri);
    assert_eq!(documents[0].text_document.version, Some(7));
    apply(
        source,
        documents[0]
            .edits
            .iter()
            .map(|edit| match edit {
                OneOf::Left(edit) => edit.clone(),
                _ => panic!("unexpected annotated edit"),
            })
            .collect(),
    )
}

fn completed(marked: &str) -> String {
    let cursor = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let (workspace, uri) = setup(&source);
    let items = features::completion(
        &workspace,
        &uri,
        workspace.documents[&uri].text.position(cursor),
    );
    let item = items
        .iter()
        .find(|item| item.label == "keyname")
        .unwrap_or_else(|| panic!("{marked}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing completion edit")
    };
    assert_eq!(edit.range.start.line, edit.range.end.line);
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    apply(&source, edits)
}

#[test]
fn spaced_constructed_names_do_not_alias_an_ordinary_control_word() {
    for form in [
        r"\csuse{ keyname}",
        r"\csuse{keyname }",
        r"\csuse{ keyname }",
        r"\csname keyname \endcsname",
        "\\csname keyname\r\n\\endcsname",
        r"\ifcsequal{other}{ keyname }{Yes}{No}",
        r"\csappto{keyname }{Code}",
        r"\csundef{ keyname}",
        r"\csletcs{cached}{ keyname}",
        r"\expandafter\letcs\csname cached\endcsname{keyname }",
    ] {
        let source = format!("% 😀\r\n\\newcommand{{\\keyname}}{{Direct}}\\keyname\r\n{form}");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        let definition = doc.text.position(source.find(r"\keyname").unwrap() + 2);
        assert_eq!(
            features::references(&workspace, &uri, definition, true).len(),
            2,
            "{form}"
        );
        let at = doc.text.position(source.rfind("keyname").unwrap() + 2);
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{form}"
        );
        assert!(
            features::prepare_rename(&workspace, &uri, at)
                .unwrap()
                .is_none(),
            "{form}"
        );
        assert_eq!(
            renamed(&source),
            source.replacen(r"\keyname", r"\renamedkey", 2),
            "{form}"
        );
    }
}

#[test]
fn spaced_named_declarations_do_not_invent_unspaced_definitions() {
    for declaration in [
        r"\csdef{ keyname}{Spaced}",
        r"\csgdef{keyname }{Spaced}",
        r"\csedef{ keyname }{Spaced}",
        r"\csxdef{key name}{Spaced}",
        r"\csletcs{ keyname}{source}",
        r"\cslet{keyname }\source",
        r"\expandafter\def\csname keyname \endcsname{Spaced}",
        r"\expandafter\newcommand\csname keyname \endcsname{Spaced}",
    ] {
        let (workspace, uri) = setup(declaration);
        assert!(
            workspace.documents[&uri].syntax.definitions.is_empty(),
            "{declaration}"
        );
        let source = format!("\\newcommand{{\\keyname}}{{Direct}}\n{declaration}\n\\keyname");
        assert_eq!(
            renamed(&source),
            source.replacen(r"\keyname", r"\renamedkey", 2),
            "{declaration}"
        );
    }
}

#[test]
fn choosing_a_named_completion_replaces_whitespace_on_every_affected_line() {
    let definition = "\\newcommand{\\keyname}{Direct}\r\n";
    for (marked, expected) in [
        (r"\csuse{ key¦name }{Keep}", r"\csuse{keyname}{Keep}"),
        (
            "\\csuse{\r\n  key¦name \r\n}{Keep}",
            r"\csuse{keyname}{Keep}",
        ),
        ("\\csuse{  key¦name \r\n}{Keep}", r"\csuse{keyname}{Keep}"),
        (
            r"\ifcsequal{other}{ key¦name }{Yes}{No}",
            r"\ifcsequal{other}{keyname}{Yes}{No}",
        ),
        (
            r"\csletcs{cached}{ key¦name }",
            r"\csletcs{cached}{keyname}",
        ),
        (r"\csuse{ key¦", r"\csuse{keyname"),
    ] {
        assert_eq!(
            completed(&format!("{definition}{marked}")),
            format!("{definition}{expected}")
        );
    }
}

#[test]
fn spaced_named_lookups_do_not_borrow_an_ordinary_macros_signature() {
    for call in [
        r"\csuse{ keyname}{Text}",
        r"\csuse{keyname }{Text}",
        r"\csname keyname \endcsname{Text}",
    ] {
        let source = format!("\\newcommand{{\\keyname}}[1]{{#1}}\n{call}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("Text").unwrap() + 2);
        assert!(signature::help(&workspace, &uri, at).is_none(), "{call}");
    }
    let source = "\\newcommand{\\keyname}[1]{#1}\n\\csname  keyname\\endcsname{Text}";
    let (workspace, uri) = setup(source);
    let at = workspace.documents[&uri]
        .text
        .position(source.rfind("Text").unwrap() + 2);
    assert!(signature::help(&workspace, &uri, at).is_some());
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_renames_preserve_distinct_spaced_commands_and_rendered_text() {
    for engine in ["pdflatex", "xelatex"] {
        for form in [r"\csuse{keyname }", r"\csname keyname \endcsname"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\documentclass{{article}}\n\\usepackage{{etoolbox}}\n\\newcommand{{\\keyname}}{{DirectUnique}}\n\\def\\spacepart{{ }}\\expandafter\\def\\csname keyname\\spacepart\\endcsname{{SpacedUnique}}\n\\begin{{document}}\\typeout{{PLAIN:\\keyname}}\\typeout{{SPACED:{form}}}\\keyname. {form}.\\end{{document}}\n");
            let result = renamed(&source);
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
                for marker in ["PLAIN:DirectUnique", "SPACED:SpacedUnique"] {
                    assert!(
                        outcome.output.contains(marker),
                        "{engine}/{form}: {}",
                        outcome.output
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_named_completion_selects_the_exact_command_without_residual_whitespace() {
    for engine in ["pdflatex", "xelatex"] {
        for call in [r"\csuse{ key¦name }", "\\csuse{\r\n  key¦name \r\n}"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let marked = format!("\\documentclass{{article}}\r\n\\usepackage{{etoolbox}}\r\n\\newcommand{{\\keyname}}{{DirectUnique}}\r\n\\csdef{{ keyname }}{{SpacedUnique}}\r\n\\begin{{document}}{call}.\\end{{document}}\r\n");
            let source = marked.replace('¦', "");
            let result = completed(&marked);
            assert!(result.contains(r"\csuse{keyname}"));
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
                    "{engine}/{call}: {}",
                    outcome.output
                );
            }
        }
    }
}
