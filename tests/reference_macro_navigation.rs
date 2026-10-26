use lsp_types::*;
use std::sync::atomic::AtomicBool;
use texquill::{
    compilation::build, config::BuildConfig, editing, language::features,
    project::workspace::Workspace, text::Text,
};
fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("file:///saved-name-commands/main.tex").unwrap();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            versioned_edits: true,
            ..Default::default()
        },
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

#[test]
fn macros_inside_key_arguments_navigate_and_rename_independently_of_static_keys() {
    for call in [
        r"\ref{\keyname}",
        r"\pageref{\keyname}",
        r"\eqref{\keyname}",
        r"\ref*{\keyname}",
        r"\autoref{\keyname}",
        r"\nameref{\keyname}",
        r"\vref[see]{\keyname}",
        r"\vpageref[a][b]{\keyname}",
        r"\cref{keyname,\keyname}",
        r"\crefrange{\keyname}{keyname}",
        r"\crefrange{keyname}{\keyname}",
        r"\hyperref[\keyname]{Text}",
        r"\cite{\keyname}",
        r"\nocite{\keyname}",
        r"\cite{keyname,\keyname}",
        r"\cite*[see][p. 4]{\keyname}",
        r"\bibentry{\keyname}",
        r"\label{\keyname}",
        r"\bibitem{\keyname}",
        "\\ref{% keep\r\n\\keyname}",
    ] {
        let source = format!("% 😀\r\n\\newcommand{{\\keyname}}{{payload}}\\label{{keyname}}\\bibitem{{keyname}}\\ref{{keyname}}\\cite{{keyname}}\r\n{call}\r\n");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let start = source.rfind(r"\keyname").unwrap();
        let position = text.position(start + 3);
        let definitions = features::definitions(&workspace, &uri, position);
        assert_eq!(definitions.len(), 1, "{call}: {definitions:?}");
        let declaration = source.find(r"\keyname").unwrap();
        assert_eq!(
            definitions[0].range,
            text.range(declaration, declaration + 8),
            "{call}"
        );
        assert_eq!(
            features::hover(&workspace, &uri, position).unwrap().range,
            Some(text.range(start, start + 8)),
            "{call}"
        );
        assert_eq!(
            features::references(&workspace, &uri, position, true).len(),
            2,
            "{call}"
        );
        let highlights = editing::highlights(&workspace, &uri, position);
        assert_eq!(highlights.len(), 2, "{call}");
        assert!(highlights
            .iter()
            .any(|item| item.range == text.range(start, start + 8)
                && item.kind == Some(DocumentHighlightKind::READ)));
        assert_eq!(
            features::prepare_rename(&workspace, &uri, position).unwrap(),
            Some(PrepareRenameResponse::RangeWithPlaceholder {
                range: text.range(start + 1, start + 8),
                placeholder: "keyname".into()
            }),
            "{call}"
        );
        let edit = features::rename(&workspace, &uri, position, "renamedkey")
            .unwrap()
            .unwrap();
        let from_declaration = features::rename(
            &workspace,
            &uri,
            text.position(declaration + 2),
            "renamedkey",
        )
        .unwrap()
        .unwrap();
        assert_eq!(edit, from_declaration, "{call}");
        assert_eq!(
            changed(&source, edit, &uri),
            source.replace(r"\keyname", r"\renamedkey"),
            "{call}"
        );
    }
}

#[test]
fn key_macro_navigation_preserves_opaque_text_and_static_key_namespaces() {
    for opaque in [
        r"\verb|\ref{\keyname}|",
        "% \\ref{\\keyname}",
        r"\detokenize{\ref{\keyname}}",
        r"\string\keyname",
        r"\ref{\detokenize{\keyname}}",
        r"\cite{\string\keyname}",
    ] {
        let source = format!("\\newcommand{{\\keyname}}{{payload}}\n{opaque}");
        let (workspace, uri) = setup(&source);
        let position = workspace.documents[&uri]
            .text
            .position(source.rfind(r"\keyname").unwrap() + 3);
        assert!(
            features::definitions(&workspace, &uri, position).is_empty(),
            "{opaque}"
        );
        assert!(
            features::prepare_rename(&workspace, &uri, position)
                .unwrap()
                .is_none(),
            "{opaque}"
        );
    }
    let source = r"\newcommand{\keyname}{payload}\label{keyname}\bibitem{keyname}\ref{\keyname}\cite{keyname,\keyname}";
    let (workspace, uri) = setup(source);
    for (needle, expected) in [
        (r"\label{keyname}", r"\ref{\keyname}"),
        (r"\bibitem{keyname}", r"\cite{renamedstatic,\keyname}"),
    ] {
        let at = source.find(needle).unwrap() + needle.find('{').unwrap() + 2;
        let position = workspace.documents[&uri].text.position(at);
        let renamed = changed(
            source,
            features::rename(&workspace, &uri, position, "renamedstatic")
                .unwrap()
                .unwrap(),
            &uri,
        );
        assert!(renamed.contains(expected), "{needle}: {renamed}");
        assert!(renamed.contains(r"\newcommand{\keyname}{payload}"));
    }
}

#[test]
fn shared_key_macro_invocations_keep_existing_project_ambiguity_checks() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")), format!("\\documentclass{{article}}\\newcommand{{\\keyname}}{{{name}}}\\begin{{document}}\\input{{shared}}\\end{{document}}")).unwrap();
    }
    std::fs::write(root.join("shared.tex"), r"\ref{\keyname}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let uri = Url::from_file_path(root.join("shared.tex")).unwrap();
    let position = workspace.documents[&uri].text.position(8);
    assert_eq!(features::definitions(&workspace, &uri, position).len(), 2);
    assert!(features::rename(&workspace, &uri, position, "renamedkey").is_err());
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_key_macro_renames_preserve_compilation() {
    for engine in ["pdflatex", "xelatex"] {
        for call in [
            r"\ref{\keyname}",
            r"\pageref{\keyname}",
            r"\eqref{\keyname}",
            r"\cite{\keyname}",
            r"\nocite{\keyname}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let uri = Url::from_file_path(&main).unwrap();
            let source = format!("\\documentclass{{article}}\r\n\\usepackage{{amsmath}}\r\n\\newcommand{{\\keyname}}{{payload}}\r\n\\begin{{document}}\r\n\\section{{MarkerUnique}}\\label{{payload}}\r\n\\begin{{thebibliography}}{{1}}\\bibitem{{payload}}Entry\\end{{thebibliography}}\r\n{call}\r\n\\end{{document}}\r\n");
            std::fs::write(&main, &source).unwrap();
            let config = BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let before = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(
                before.status, "success",
                "{engine}/{call}: {}",
                before.output
            );
            let mut workspace = Workspace {
                client: texquill::project::state::ClientCapabilities {
                    versioned_edits: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            workspace.open(uri.clone(), source.clone(), 7);
            let position = workspace.documents[&uri]
                .text
                .position(source.rfind(r"\keyname").unwrap() + 3);
            let edit = features::rename(&workspace, &uri, position, "renamedkey")
                .unwrap()
                .unwrap();
            let result = changed(&source, edit, &uri);
            assert_eq!(result, source.replace(r"\keyname", r"\renamedkey"));
            std::fs::write(&main, result).unwrap();
            let after = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(after.status, "success", "{engine}/{call}: {}", after.output);
        }
    }
}

#[test]
fn literal_conversions_consume_data_but_declaration_operands_stay_control_tokens() {
    for declaration in [
        r"\newcommand{\string}[1]{#1}",
        r"\renewcommand\string[1]{#1}",
        r"\def\string#1{#1}",
        r"\let\alias=\string",
        r"\let\string\relax",
        r"\NewCommandCopy{\alias}{\string}",
        r"\NewCommandCopy\alias\string",
        r"\cslet{alias}\string",
        r"\cslet\dynamic\string",
    ] {
        let source = format!("\\newcommand{{\\keyname}}{{payload}}{declaration}\\keyname");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        assert!(
            doc.syntax.diagnostics.is_empty(),
            "{declaration}: {:?}",
            doc.syntax.diagnostics
        );
        assert_eq!(
            features::definitions(
                &workspace,
                &uri,
                doc.text.position(source.rfind(r"\keyname").unwrap() + 2)
            )
            .len(),
            1,
            "{declaration}"
        );
    }
    for conversion in [
        r"\detokenize{\label{ghost}\keyname}",
        r"\string\keyname",
        "\\string % keep\r\n\\keyname",
    ] {
        let source = format!("\\newcommand{{\\keyname}}{{payload}}{conversion}\\ref{{\\keyname}}");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        assert!(doc.syntax.labels.is_empty(), "{conversion}");
        let position = doc.text.position(source.rfind(r"\keyname").unwrap() + 2);
        assert_eq!(
            features::references(&workspace, &uri, position, true).len(),
            2,
            "{conversion}"
        );
        let result = changed(
            &source,
            features::rename(&workspace, &uri, position, "renamedkey")
                .unwrap()
                .unwrap(),
            &uri,
        );
        assert!(result.contains(conversion), "{conversion}: {result}");
    }
    for conversion in [r"\string*", r"\string\%", r"\string😀"] {
        let source = format!("\\newcommand{{\\keyname}}{{payload}}{conversion}\\ref{{\\keyname}}");
        let (workspace, uri) = setup(&source);
        let doc = &workspace.documents[&uri];
        assert_eq!(
            features::definitions(
                &workspace,
                &uri,
                doc.text.position(source.rfind(r"\keyname").unwrap() + 2)
            )
            .len(),
            1,
            "{conversion}"
        );
    }
}
