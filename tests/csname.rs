use latex_lsp::{build, editing, features, signature, workspace::Workspace};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn renamed(workspace: &Workspace, document: &Url, position: Position) -> BTreeMap<Url, String> {
    features::rename(workspace, document, position, "greeting")
        .unwrap()
        .unwrap()
        .changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let source = workspace.documents[&uri]
                .text
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
                .source;
            (uri, source)
        })
        .collect()
}

#[test]
fn literal_macro_names_support_navigation_prepared_rename_references_and_highlights() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = "\\newcommand{\\hello}[1]{Hello #1}\n";
    let source = "\\documentclass{article}\r\n\\input{definitions}\r\n\\begin{document}\r\n😀 \\csname hello\\endcsname{World}\r\n\\hello{Direct}\\csname  hello  \\endcsname{Other}\r\n% \\csname hello\\endcsname\r\n\\verb|\\csname hello\\endcsname|\r\n\\end{document}\r\n";
    let workspace = project(
        root,
        &[("main.tex", source), ("definitions.tex", definitions)],
    );
    let main = uri(root, "main.tex");
    let text = &workspace.documents[&main].text;
    for start in [
        source.find("hello").unwrap(),
        source.find("\\hello").unwrap() + 2,
        source.find("  hello").unwrap() + 2,
    ] {
        let position = text.position(start + 2);
        let locations = features::definitions(&workspace, &main, position);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].uri, uri(root, "definitions.tex"));
        assert!(features::hover(&workspace, &main, position).is_some());
        let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
            features::prepare_rename(&workspace, &main, position).unwrap()
        else {
            panic!("missing prepared rename")
        };
        assert_eq!(placeholder, "hello");
        assert_eq!(
            &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
            "hello"
        );
        assert_eq!(
            features::references(&workspace, &main, position, false).len(),
            3
        );
        assert_eq!(
            features::references(&workspace, &main, position, true).len(),
            4
        );
        let highlights = editing::highlights(&workspace, &main, position);
        assert_eq!(highlights.len(), 3);
        assert!(highlights
            .iter()
            .all(|entry| entry.kind == Some(DocumentHighlightKind::READ)));
        let changed = renamed(&workspace, &main, position);
        assert_eq!(
            changed[&main],
            source
                .replace("😀 \\csname hello", "😀 \\csname greeting")
                .replace(
                    "\\hello{Direct}\\csname  hello",
                    "\\greeting{Direct}\\csname  greeting"
                )
        );
        assert_eq!(
            changed[&uri(root, "definitions.tex")],
            definitions.replace("hello", "greeting")
        );
    }
}

#[test]
fn computed_commented_and_unterminated_names_are_not_guessed_or_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\newcommand{\\hello}{}\n\\csname hello world\\endcsname\n\\csname hello%comment\n\\endcsname\n\\csname \\prefix hello\\endcsname\n\\csname hello";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let text = &workspace.documents[&main].text;
    let position = text.position(source.find("hello").unwrap());
    let changed = renamed(&workspace, &main, position);
    assert_eq!(changed[&main], source.replacen("\\hello", "\\greeting", 1));
    for (offset, _) in source.match_indices("hello").skip(1) {
        assert!(
            features::prepare_rename(&workspace, &main, text.position(offset + 2))
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn shared_literal_calls_to_different_macro_definitions_are_ambiguous() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root, &[
        ("a.tex", "\\documentclass{article}\n\\newcommand{\\hello}{A}\n\\begin{document}\\input{shared}\\end{document}"),
        ("b.tex", "\\documentclass{article}\n\\newcommand{\\hello}{B}\n\\begin{document}\\input{shared}\\end{document}"),
        ("shared.tex", "\\csname hello\\endcsname"),
    ]);
    for file in ["a.tex", "shared.tex"] {
        let document = uri(root, file);
        let text = &workspace.documents[&document].text;
        let position = text.position(text.source.find("hello").unwrap() + 2);
        assert!(features::prepare_rename(&workspace, &document, position).is_err());
        assert!(features::rename(&workspace, &document, position, "greeting").is_err());
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn renamed_literal_macro_calls_keep_control_word_syntax_and_compile() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\csname hello\\endcsname{World}\n\\hello{Direct}\n\\end{document}\n";
    let workspace = project(
        root,
        &[
            ("main.tex", source),
            ("definitions.tex", "\\newcommand{\\hello}[1]{Hello #1}\n"),
        ],
    );
    let main = uri(root, "main.tex");
    let position = workspace.documents[&main]
        .text
        .position(source.find("hello").unwrap() + 2);
    for (uri, source) in renamed(&workspace, &main, position) {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    assert_eq!(outcome.pages, Some(1));
}

#[test]
fn expanded_literal_definitions_index_the_resulting_name_and_preserve_declaration_roles() {
    for declaration in [
        r"\expandafter\def\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\gdef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\edef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\xdef\csname hello\endcsname#1{Hello #1}",
        "\\global\\expandafter % comment\n\\def % comment\n\\csname  hello  \\endcsname#1{Hello #1}",
        r"\expandafter\newcommand\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\newrobustcmd\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\renewrobustcmd\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\providerobustcmd\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\NewDocumentCommand\csname hello\endcsname{m}{Hello #1}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main_source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\hello{World} \\csname hello\\endcsname{Other}\n\\end{document}\n";
        let workspace = project(root, &[("main.tex", main_source), ("definitions.tex", declaration)]);
        let definitions = uri(root, "definitions.tex");
        let main = uri(root, "main.tex");
        let document = &workspace.documents[&definitions];
        assert_eq!(document.syntax.definitions.len(), 1, "{declaration}");
        assert_eq!(document.syntax.definitions[0].name, "hello");
        assert_eq!(features::symbols(document)[0].name, "hello");
        let position = document.text.position(declaration.find("hello").unwrap() + 2);
        let references = features::references(&workspace, &definitions, position, false);
        assert_eq!(references.len(), 2, "{declaration}: {references:?}");
        assert!(references.iter().all(|entry| entry.uri == main));
        let highlights = editing::highlights(&workspace, &definitions, position);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].kind, Some(DocumentHighlightKind::WRITE));
        let changed = renamed(&workspace, &definitions, position);
        assert_eq!(changed[&definitions], declaration.replace("hello", "greeting"));
        assert_eq!(changed[&main], main_source.replace("hello", "greeting"));
        let text = &workspace.documents[&main].text;
        let call = text.position(main_source.find("World").unwrap() + 2);
        let help = signature::help(&workspace, &main, call).unwrap();
        assert_eq!(help.signatures[0].label, r"\hello{#1}");
        assert_eq!(help.active_parameter, Some(0));
        let items = features::completion(&workspace, &main, text.position(main_source.find("\\hello").unwrap() + 3));
        assert!(items.iter().any(|item| item.label == "hello" && item.detail.as_deref() == Some(r"\hello{#1}")));
    }
}

#[test]
fn computed_expanded_definitions_do_not_define_the_csname_primitive() {
    for declaration in [
        r"\expandafter\def\csname hello\suffix\endcsname{Hello}",
        r"\expandafter\newcommand\csname hello\suffix\endcsname{Hello}",
        r"\expandafter\newrobustcmd\csname hello\suffix\endcsname{Hello}",
        "\\expandafter\\def\\csname hello% comment\n\\endcsname{Hello}",
        r"\expandafter\def\csname hello world\endcsname{Hello}",
        r"\expandafter\def\csname hello",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(root, &[("main.tex", declaration)]);
        let main = uri(root, "main.tex");
        let document = &workspace.documents[&main];
        assert!(document.syntax.definitions.is_empty(), "{declaration}");
        let position = document
            .text
            .position(declaration.find("csname").unwrap() + 2);
        assert!(features::prepare_rename(&workspace, &main, position)
            .unwrap()
            .is_none());
    }
}

#[test]
fn literal_aliases_and_nonletter_names_are_indexed_without_invalid_direct_completions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let definitions = r"\expandafter\let\csname hello\endcsname=\textbf
\expandafter\def\csname hello-two\endcsname{Other}";
    let workspace = project(
        root,
        &[("definitions.tex", definitions), ("main.tex", "\\hel")],
    );
    let main = uri(root, "main.tex");
    let definitions_uri = uri(root, "definitions.tex");
    assert_eq!(
        workspace.documents[&definitions_uri]
            .syntax
            .definitions
            .len(),
        2
    );
    let items = features::completion(&workspace, &main, Position::new(0, 4));
    assert!(items.iter().any(|item| item.label == "hello"));
    assert!(!items.iter().any(|item| item.label == "hello-two"));
    let text = &workspace.documents[&definitions_uri].text;
    let position = text.position(definitions.find("hello-two").unwrap() + 2);
    assert_eq!(
        renamed(&workspace, &definitions_uri, position)[&definitions_uri],
        definitions.replace("hello-two", "greeting")
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn renamed_expanded_literal_definitions_and_aliases_still_compile() {
    for declaration in [
        r"\expandafter\def\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\gdef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\edef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\xdef\csname hello\endcsname#1{Hello #1}",
        r"\expandafter\newcommand\csname hello\endcsname[1]{Hello #1}",
        r"\expandafter\NewDocumentCommand\csname hello\endcsname{m}{Hello #1}",
        r"\expandafter\let\csname hello\endcsname=\textbf",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "\\documentclass{article}\n\\input{definitions}\n\\begin{document}\n\\hello{World}\\csname hello\\endcsname{Other}\n\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", declaration)],
        );
        let definitions = uri(root, "definitions.tex");
        let position = workspace.documents[&definitions]
            .text
            .position(declaration.find("hello").unwrap() + 2);
        for (uri, source) in renamed(&workspace, &definitions, position) {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let outcome = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status, "success",
            "{declaration}: {}",
            outcome.output
        );
        assert_eq!(outcome.pages, Some(1));
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn robust_macro_renames_preserve_defaults_literal_calls_and_protected_expansion() {
    for declaration in [
        r"\newrobustcmd{\hello}",
        r"\newrobustcmd*\hello",
        r"\providerobustcmd\hello",
        r"\expandafter\newrobustcmd\csname hello\endcsname",
        r"\expandafter\providerobustcmd\csname hello\endcsname",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let definitions = format!("{declaration}[2][blue]{{\\typeout{{ROBUST:#1:#2}}#2}}\n");
        let source = "\\documentclass{article}\\usepackage{etoolbox}\\input{definitions}\n\\begin{document}\n\\hello{Default}\\hello[red]{Explicit}\\csname hello\\endcsname{Literal}\n\\edef\\protectedcall{\\hello{Protected}}\\typeout{ROBUST:before-call}\\protectedcall\n\\end{document}\n";
        let workspace = project(
            root,
            &[("main.tex", source), ("definitions.tex", &definitions)],
        );
        let main = root.join("main.tex");
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let before = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(before.status, "success", "{declaration}: {}", before.output);
        let document = uri(root, "definitions.tex");
        let text = &workspace.documents[&document].text;
        let position = text.position(definitions.find("hello").unwrap() + 2);
        for (uri, changed) in renamed(&workspace, &document, position) {
            std::fs::write(uri.to_file_path().unwrap(), changed).unwrap();
        }
        let after = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(after.status, "success", "{declaration}: {}", after.output);
        for output in [&before.output, &after.output] {
            for state in [
                "ROBUST:blue:Default",
                "ROBUST:red:Explicit",
                "ROBUST:blue:Literal",
                "ROBUST:blue:Protected",
            ] {
                assert!(output.contains(state), "{declaration}: {state}\n{output}");
            }
            assert!(
                output.find("ROBUST:before-call").unwrap()
                    < output.find("ROBUST:blue:Protected").unwrap()
            );
        }
    }
}
