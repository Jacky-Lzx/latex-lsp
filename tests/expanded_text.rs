use latex_lsp::{language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;

fn setup(preamble: &str, body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let source = format!("% 😀\r\n\\documentclass{{article}}{preamble}\\begin{{document}}{body}\\label{{safe}}\\ref{{safe}}\\end{{document}}");
    std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source, 5);
    (directory, workspace, uri)
}

fn protected(workspace: &Workspace, uri: &Url, name: &str) {
    let text = &workspace.documents[uri].text;
    let at = text.position(text.source.find(name).unwrap() + 1);
    assert!(
        !matches!(features::prepare_rename(workspace, uri, at), Ok(Some(_))),
        "{}",
        text.source
    );
    assert!(
        !matches!(features::rename(workspace, uri, at, "fresh"), Ok(Some(_))),
        "{}",
        text.source
    );
}

#[test]
fn expanded_messages_do_not_certify_label_declarations_as_executed_code() {
    for (preamble, call) in [
        ("", r"\typeout"),
        ("", r"\message"),
        (r"\def\logtext{\typeout}", r"\logtext"),
        (r"\def\logtext{\message}", r"\logtext"),
        (r"\let\logtext\typeout", r"\logtext"),
        ("", r"\csuse{typeout}"),
        ("", r"\csuse{message}"),
        ("", r"\csname typeout\endcsname"),
        ("", r"\csname message\endcsname"),
    ] {
        let (_directory, workspace, uri) =
            setup(preamble, &format!(r"{call}{{\textbf{{\label{{ghost}}}}}}"));
        protected(&workspace, &uri, "ghost");
        let text = &workspace.documents[&uri].text;
        let edits = features::rename(
            &workspace,
            &uri,
            text.position(text.source.rfind("safe").unwrap() + 1),
            "fresh",
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert_eq!(edits[&uri].len(), 2);
    }
}

#[test]
fn expanded_reference_tokens_do_not_emit_undefined_key_diagnostics() {
    for reader in ["typeout", "message"] {
        let (_directory, workspace, uri) = setup(
            "",
            &format!(r"\{reader}{{\ref{{absentlabel}}\cite{{absentbook}}}}"),
        );
        let diagnostics = workspace.diagnostics(&uri);
        assert!(
            !diagnostics.iter().any(|d| matches!(
                &d.code,
                Some(NumberOrString::String(code))
                    if code == "undefined-reference" || code == "undefined-citation"
            )),
            "{reader}: {diagnostics:?}"
        );
    }
}

#[test]
fn unfinished_message_text_keeps_declarations_protected() {
    for call in [
        r"\typeout{\label{ghost}",
        r"\message{\label{ghost}",
        r"\csuse{typeout}{\label{ghost}",
        r"\csname message\endcsname{\label{ghost}",
    ] {
        let (_directory, mut workspace, uri) = setup("", "");
        // No synthetic closing document group may complete the unfinished text.
        workspace.open(uri.clone(), call.into(), 7);
        protected(&workspace, &uri, "ghost");
    }
}

#[test]
fn unfinished_message_text_cannot_initialize_nested_bibliography_queries() {
    for (preamble, call) in [
        ("", r"\typeout"),
        ("", r"\message"),
        (r"\def\logtext{\typeout}", r"\logtext"),
        (r"\def\logtext{\message}", r"\logtext"),
        ("", r"\csuse{typeout}"),
        ("", r"\csname message\endcsname"),
        (r"\def\ifblank#1#2#3{#2}", r"\typeout"),
        (r"\def\ifblank#1#2#3{#2}\def\logtext{\typeout}", r"\logtext"),
        (r"\def\ifblank#1#2#3{#2}\let\logtext\typeout", r"\logtext"),
    ] {
        let (directory, mut workspace, uri) = setup(
            &format!(r"\usepackage{{biblatex}}\addbibresource{{data.bib}}{preamble}"),
            &format!(
                r"\defbibentryset{{collection}}{{member}}{call}{{\mkbibparens{{\entrydata{{collection}}{{Text}}}}"
            ),
        );
        std::fs::write(
            directory.path().join("data.bib"),
            "@book{collection,title={Raw}} @book{member,title={Member}}",
        )
        .unwrap();
        let source = workspace.documents[&uri]
            .text
            .source
            .split(r"\label{safe}")
            .next()
            .unwrap()
            .to_owned();
        workspace.open(uri.clone(), source.clone(), 6);
        let at = workspace.documents[&uri]
            .text
            .position(source.find(r"\entrydata{collection}").unwrap() + 12);
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{preamble}/{call}"
        );
        assert!(
            !matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
            "{preamble}/{call}"
        );
    }
}

#[test]
fn macro_expansion_names_remain_editable_inside_messages() {
    let (_directory, workspace, uri) = setup(
        r"\newcommand\word{Text}",
        r"\typeout{\word}\message{\word}\word",
    );
    let text = &workspace.documents[&uri].text;
    let edit = features::rename(
        &workspace,
        &uri,
        text.position(text.source.rfind("word").unwrap() + 1),
        "term",
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(edit[&uri].len(), 4);
}

#[test]
fn expanded_bibliography_queries_do_not_inherit_outer_text_initialization() {
    for reader in ["typeout", "message"] {
        let (directory, mut workspace, uri) = setup(
            r"\usepackage{biblatex}\addbibresource{data.bib}",
            &format!(
                r"\defbibentryset{{collection}}{{member}}\mkbibparens{{\{reader}{{\mkbibparens{{\entrydata{{collection}}{{Text}}}}}}}}\cite{{collection}}"
            ),
        );
        std::fs::write(
            directory.path().join("data.bib"),
            "@book{collection,title={Raw}} @book{member,title={Member}}",
        )
        .unwrap();
        // Reopen after the provider exists to exercise the ordinary project graph.
        let source = workspace.documents[&uri].text.source.clone();
        workspace.open(uri.clone(), source, 6);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.find(r"\entrydata{collection}").unwrap() + 12);
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{reader}"
        );
        assert!(!matches!(
            features::rename(&workspace, &uri, at, "fresh"),
            Ok(Some(_))
        ));
    }
}

#[test]
fn unsaved_message_replacements_restore_only_confirmed_executable_arguments() {
    let (directory, mut workspace, uri) =
        setup(r"\input{commands.sty}", r"\typeout{\label{ghost}}");
    let provider = Url::from_file_path(directory.path().join("commands.sty")).unwrap();
    std::fs::write(provider.to_file_path().unwrap(), "").unwrap();
    let source = workspace.documents[&uri].text.source.clone();
    workspace.open(uri.clone(), source, 6);
    protected(&workspace, &uri, "ghost");
    workspace.open(provider.clone(), r"\renewcommand\typeout[1]{#1}".into(), 12);
    let text = &workspace.documents[&uri].text;
    let at = text.position(text.source.find("ghost").unwrap() + 1);
    let edits = features::rename(&workspace, &uri, at, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits[&uri],
        [TextEdit::new(
            text.range(
                text.source.find("ghost").unwrap(),
                text.source.find("ghost").unwrap() + 5
            ),
            "fresh".into()
        )]
    );
    workspace.open(provider, String::new(), 13);
    protected(&workspace, &uri, "ghost");
}

#[test]
#[ignore = "requires pdflatex"]
fn native_expanded_label_tokens_keep_their_literal_log_text_after_safe_edits() {
    for (preamble, call) in [
        ("", r"\typeout"),
        ("", r"\message"),
        (r"\def\logtext{\typeout}", r"\logtext"),
        (r"\usepackage{etoolbox}", r"\csuse{typeout}"),
    ] {
        let (directory, workspace, uri) = setup(
            preamble,
            &format!(r"\section{{Heading}}{call}{{LSP-EXPANDED:\label{{ghost}}}}"),
        );
        let source = &workspace.documents[&uri].text.source;
        let compile = |source: &str| {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
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
                    "{}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            assert!(
                !log.contains("Warning") && !log.contains("undefined"),
                "{log}"
            );
            let start = log.find("LSP-EXPANDED:").unwrap();
            let observation = log[start..]
                .split("[1")
                .next()
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (std::fs::read_to_string(aux).unwrap(), observation)
        };
        let (before, literal) = compile(source);
        assert!(!before.contains(r"\newlabel{ghost}") && literal.contains("ghost"));
        let (legacy_aux, legacy_literal) = compile(&source.replace("ghost", "fresh"));
        assert_eq!(legacy_aux, before);
        assert_eq!(legacy_literal, literal.replace("ghost", "fresh"));
        protected(&workspace, &uri, "ghost");
        let text = Text::new(source.clone());
        let edit = features::rename(
            &workspace,
            &uri,
            text.position(source.rfind("safe").unwrap() + 1),
            "fresh",
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert_eq!(edit[&uri].len(), 2);
        let mut edits = edit[&uri].clone();
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let changed = text
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
        let (after, after_literal) = compile(&changed.source);
        assert_eq!(after.replace("{fresh}", "{safe}"), before);
        assert_eq!(after_literal, literal);
    }
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_expanded_entrydata_stays_literal_inside_initialized_bibliography_text() {
    for reader in ["typeout", "message"] {
        let (directory, mut workspace, uri) = setup(
            r"\usepackage{biblatex}\addbibresource{data.bib}",
            &format!(
                r"\defbibentryset{{collection}}{{member}}\mkbibparens{{\{reader}{{LSP-EXPANDED:\mkbibparens{{\entrydata{{collection}}{{Text}}}}}}}}\cite{{collection}}\printbibliography"
            ),
        );
        std::fs::write(
            directory.path().join("data.bib"),
            "@book{collection,title={Raw data}} @book{member,title={Member}}",
        )
        .unwrap();
        let source = workspace.documents[&uri].text.source.clone();
        workspace.open(uri.clone(), source.clone(), 6);
        let compile = |source: &str| {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
            for extension in ["aux", "bcf", "bbl", "run.xml"] {
                let path = directory.path().join(format!("main.{extension}"));
                if path.exists() {
                    std::fs::remove_file(path).unwrap();
                }
            }
            for program in [
                "pdflatex", "biber", "pdflatex", "biber", "pdflatex", "pdflatex",
            ] {
                let args = if program == "biber" {
                    vec!["main"]
                } else {
                    vec![
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ]
                };
                let output = std::process::Command::new(program)
                    .args(args)
                    .current_dir(directory.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{reader}/{program}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            let blg = std::fs::read_to_string(directory.path().join("main.blg")).unwrap();
            assert!(
                !log.contains("Warning") && !log.contains("undefined"),
                "{log}"
            );
            assert!(!blg.contains("WARN") && !blg.contains("ERROR"), "{blg}");
            let start = log.find("LSP-EXPANDED:").unwrap();
            let observation = log[start..]
                .split("[1")
                .next()
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (
                std::fs::read_to_string(directory.path().join("main.aux")).unwrap(),
                std::fs::read_to_string(directory.path().join("main.bbl")).unwrap(),
                observation,
            )
        };
        let (before_aux, before_bbl, before_text) = compile(&source);
        assert!(before_text.contains(r"\entrydata {collection}{Text}"));
        let (_, legacy_bbl, legacy_text) = compile(&source.replace("collection", "fresh"));
        assert_eq!(legacy_bbl.replace("fresh", "collection"), before_bbl);
        assert_eq!(legacy_text, before_text.replace("collection", "fresh"));
        let text = &workspace.documents[&uri].text;
        let at = text.position(source.find(r"\entrydata{collection}").unwrap() + 12);
        assert!(features::definitions(&workspace, &uri, at).is_empty());
        assert!(!matches!(
            features::rename(&workspace, &uri, at, "fresh"),
            Ok(Some(_))
        ));
        let edits = features::rename(
            &workspace,
            &uri,
            text.position(source.rfind("safe").unwrap() + 1),
            "fresh",
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        let mut edits = edits[&uri].clone();
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
        let (after_aux, after_bbl, after_text) = compile(&changed.source);
        assert_eq!(after_aux.replace("{fresh}", "{safe}"), before_aux);
        assert_eq!(after_bbl, before_bbl);
        assert_eq!(after_text, before_text);
    }
}
