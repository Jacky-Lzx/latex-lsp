use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;

fn setup(preamble: &str, body: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:label-literal-readers.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), format!("% 😀\r\n\\documentclass{{article}}\\newcommand\\unicodeword{{😀}}{preamble}\\begin{{document}}\\section{{Heading}}{body}\\end{{document}}"), 5);
    (workspace, uri)
}

fn readers() -> [(&'static str, &'static str); 8] {
    [
        (r"\newcommand\literal[1]{\detokenize{#1}}", r"\literal"),
        (r"\def\literal#1{\detokenize{#1}}", r"\literal"),
        (r"\newcommand\literal[1]{\typeout{#1}}", r"\literal"),
        (r"\newcommand\literal[1]{\message{#1}}", r"\literal"),
        (r"\newcommand\literal[1]{\detokenize{#1}}\def\forward{\literal}", r"\forward"),
        (r"\newcommand\literal[1]{\detokenize{#1}}", r"\csuse{literal}"),
        (r"\newcommand\literal[1]{\detokenize{#1}}", r"\csname literal\endcsname"),
        ("\\newcommand\\literal[1]{% before reader\r\n \\detokenize % before argument\r\n {#1}% after reader\r\n}", r"\literal"),
    ]
}

fn code(diagnostic: &Diagnostic, value: &str) -> bool {
    diagnostic.code == Some(NumberOrString::String(value.into()))
}

#[test]
fn complete_native_data_reader_bodies_do_not_define_reference_labels() {
    for (preamble, call) in readers() {
        let (workspace, uri) = setup(
            preamble,
            &format!(r"{call}{{\label{{ghost}}}}\ref{{ghost}}"),
        );
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("ghost").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{preamble}/{call}"
        );
        assert!(features::hover(&workspace, &uri, at).is_none());
        let diagnostics = workspace.diagnostics(&uri);
        assert!(
            diagnostics
                .iter()
                .any(|item| code(item, "undefined-reference")),
            "{preamble}/{call}: {diagnostics:?}"
        );
        assert!(!matches!(
            features::rename(&workspace, &uri, at, "fresh"),
            Ok(Some(_))
        ));
    }
}

#[test]
fn literal_declarations_neither_shadow_real_labels_nor_create_duplicate_diagnostics() {
    for (preamble, call) in readers() {
        let (workspace, uri) = setup(
            preamble,
            &format!(r"{call}{{\label{{same}}}}\label{{same}}\ref{{same}}"),
        );
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        let declaration = text.source.rfind(r"\label{same}").unwrap() + 7;
        assert_eq!(
            features::definitions(&workspace, &uri, at),
            vec![Location::new(
                uri.clone(),
                text.range(declaration, declaration + 4)
            )],
            "{preamble}/{call}"
        );
        assert_eq!(features::references(&workspace, &uri, at, true).len(), 2);
        assert!(!workspace
            .diagnostics(&uri)
            .iter()
            .any(|item| code(item, "duplicate-label")
                || code(item, "duplicate-definition")
                || code(item, "undefined-reference")));
        // Read-only filtering cannot relax the destructive data-key guard.
        assert!(!matches!(
            features::rename(&workspace, &uri, at, "fresh"),
            Ok(Some(_))
        ));
    }
}

#[test]
fn mixed_execution_changed_readers_and_unknown_outer_wrappers_keep_structural_identity() {
    for (preamble, call) in [
        (r"\newcommand\literal[1]{\detokenize{#1}#1}", r"\literal"),
        (
            r"\def\detokenize#1{#1}\newcommand\literal[1]{\detokenize{#1}}",
            r"\literal",
        ),
        (
            r"\renewcommand\typeout[1]{#1}\newcommand\literal[1]{\typeout{#1}}",
            r"\literal",
        ),
        (
            r"\newcommand\literal[1]{\texttt{\detokenize{#1}}}",
            r"\literal",
        ),
    ] {
        let (workspace, uri) = setup(preamble, &format!(r"{call}{{\label{{same}}}}\ref{{same}}"));
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, &uri, at).len(),
            1,
            "{preamble}/{call}"
        );
        assert!(!workspace
            .diagnostics(&uri)
            .iter()
            .any(|item| code(item, "undefined-reference")));
    }
}

#[test]
fn unsaved_reader_dependencies_restore_structural_labels_without_parent_edits() {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = uri.join("commands.sty").unwrap();
    let source = "% 😀\r\n\\documentclass{article}\\newcommand\\unicodeword{😀}\\input{commands.sty}\\newcommand\\literal[1]{\\detokenize{#1}}\\begin{document}\\literal{\\label{same}}\\ref{same}\\end{document}";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    std::fs::write(provider.to_file_path().unwrap(), "").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 5);
    let at = workspace.documents[&uri]
        .text
        .position(source.rfind("same").unwrap() + 1);
    let check = |workspace: &Workspace, count| {
        assert_eq!(features::definitions(workspace, &uri, at).len(), count);
        assert_eq!(
            workspace
                .diagnostics(&uri)
                .iter()
                .filter(|item| code(item, "undefined-reference"))
                .count(),
            1 - count
        );
        assert_eq!(workspace.documents[&uri].version, Some(5));
    };
    check(&workspace, 0);
    for (version, replacement, count) in [
        (12, r"\def\detokenize#1{#1}", 1),
        (13, "", 0),
        (14, r"\renewcommand\literal[1]{#1}", 1),
    ] {
        workspace.open(provider.clone(), replacement.into(), version);
        check(&workspace, count);
    }
    workspace.documents.remove(&provider);
    workspace.reload(&provider);
    check(&workspace, 0);
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_readers_and_deferred_execution_agree_with_aux_after_safe_label_edits() {
    let deferred = [
        (
            r"\def\dropfirst#1{}\def\later{\typeout{\label{same}}}",
            r"\expandafter\dropfirst\later",
        ),
        (
            r"\def\dropfirst#1{}\newcommand\discard[1]{}\def\later{\discard{\label{same}}}",
            r"\expandafter\dropfirst\later",
        ),
        (
            r"\def\dropfirst#1{}\newcommand\literal[1]{\detokenize{#1}}\def\later{\literal{\label{same}}}",
            r"\expandafter\dropfirst\later",
        ),
        (
            r"\def\dropfirst#1{}\newcommand\later[1][\typeout{\label{same}}]{\dropfirst#1}",
            r"\later",
        ),
    ];
    let cases = readers()
        .into_iter()
        .map(|(preamble, call)| (preamble, format!(r"{call}{{\label{{same}}}}"), 0))
        .chain(
            deferred
                .into_iter()
                .map(|(preamble, call)| (preamble, call.into(), 1)),
        );
    for (preamble, call, count) in cases {
        let directory = tempfile::tempdir().unwrap();
        let (workspace, uri) = setup(
            &format!(r"\usepackage{{etoolbox}}{preamble}"),
            &format!(r"{call}\ref{{same}}\label{{safe}}\ref{{safe}}"),
        );
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        assert_eq!(features::definitions(&workspace, &uri, at).len(), count);
        let compile = |source: &str| {
            std::fs::write(directory.path().join("main.tex"), source).unwrap();
            let aux = directory.path().join("main.aux");
            if aux.exists() {
                std::fs::remove_file(&aux).unwrap();
            }
            for _ in 0..2 {
                let output = std::process::Command::new("pdflatex")
                    .args([
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ])
                    .current_dir(directory.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{preamble}/{call}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let aux = std::fs::read_to_string(aux).unwrap();
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            assert_eq!(aux.matches(r"\newlabel{same}").count(), count);
            assert_eq!(
                log.contains("There were undefined references"),
                count == 0,
                "{log}"
            );
            assert!(!log.contains("multiply defined"), "{log}");
            aux
        };
        let before = compile(&text.source);
        let at = text.position(text.source.rfind("safe").unwrap() + 1);
        let mut edits = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&uri]
            .clone();
        assert_eq!(edits.len(), 2);
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let changed = text
            .clone()
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
            .unwrap();
        assert_eq!(changed.source, text.source.replace("{safe}", "{fresh}"));
        let after = compile(&changed.source);
        assert_eq!(after.replace("{fresh}", "{safe}"), before);
    }
}

#[test]
fn deferred_readers_cannot_hide_labels_executed_after_removing_the_reader_token() {
    for preamble in [
        r"\def\dropfirst#1{}\def\later{\typeout{\label{same}}}",
        r"\def\dropfirst#1{}\newcommand\discard[1]{}\def\later{\discard{\label{same}}}",
        r"\def\dropfirst#1{}\newcommand\literal[1]{\detokenize{#1}}\def\later{\literal{\label{same}}}",
    ] {
        let (workspace, uri) = setup(preamble, r"\expandafter\dropfirst\later\ref{same}");
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, &uri, at).len(),
            1,
            "{preamble}"
        );
        assert!(!workspace
            .diagnostics(&uri)
            .iter()
            .any(|item| code(item, "undefined-reference")));
    }
    let (workspace, uri) = setup(
        r"\def\dropfirst#1{}\newcommand\later[1][\typeout{\label{same}}]{\dropfirst#1}",
        r"\later\ref{same}",
    );
    let text = &workspace.documents[&uri].text;
    let at = text.position(text.source.rfind("same").unwrap() + 1);
    assert_eq!(features::definitions(&workspace, &uri, at).len(), 1);
    assert!(!workspace
        .diagnostics(&uri)
        .iter()
        .any(|item| code(item, "undefined-reference")));
    let (workspace, uri) = setup(
        r"\def\dropfirst#1{}\newcommand\unknown[1]{\dropfirst#1}\newcommand\literal[1]{\detokenize{#1}}",
        r"\unknown{\literal{\label{same}}}\ref{same}",
    );
    let text = &workspace.documents[&uri].text;
    let at = text.position(text.source.rfind("same").unwrap() + 1);
    assert_eq!(features::definitions(&workspace, &uri, at).len(), 1);
}
