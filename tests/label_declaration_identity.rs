use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;

fn setup(preamble: &str, body: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:label-declaration-identity.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), format!("% 😀\r\n\\documentclass{{article}}\\newcommand\\unicodeword{{😀}}{preamble}\\begin{{document}}{body}\\end{{document}}"), 5);
    (workspace, uri)
}

fn code(diagnostic: &Diagnostic, code: &str) -> bool {
    diagnostic.code == Some(NumberOrString::String(code.into()))
}

fn dynamic_sources() -> [&'static str; 4] {
    [
        r"\documentclass{article}\newcommand\discard[1]{}\def\readername{discard}\expandafter\def\csname\readername\endcsname#1{#1}\begin{document}\section{Heading}\discard{\label{same}}\ref{same}\end{document}",
        r"\documentclass{article}\def\readername{typeout}\let\oldreader\typeout\begin{document}\section{Heading}\expandafter\def\csname\readername\endcsname#1{#1}\typeout{\label{same}}\ref{same}\expandafter\let\csname\readername\endcsname\oldreader\end{document}",
        r"\documentclass{article}\usepackage{etoolbox}\def\readername{message}\let\oldreader\message\begin{document}\section{Heading}\csdef{\readername}#1{#1}\message{\label{same}}\ref{same}\expandafter\let\csname\readername\endcsname\oldreader\end{document}",
        r"\documentclass{article}\def\providername{commands.sty}\begin{document}\section{Heading}\begingroup\input{\providername}\typeout{\label{same}}\endgroup\ref{same}\end{document}",
    ]
}

#[test]
fn dynamic_installation_never_certifies_an_obsolete_discarded_body() {
    for source in dynamic_sources() {
        let directory = tempfile::tempdir().unwrap();
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        std::fs::write(
            directory.path().join("commands.sty"),
            r"\renewcommand\typeout[1]{#1}",
        )
        .unwrap();
        let mut workspace = Workspace::default();
        workspace.config.distribution.enabled = false;
        workspace.open(uri.clone(), source.into(), 5);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("same").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, &uri, at).len(),
            1,
            "{source}"
        );
        assert!(
            !workspace
                .diagnostics(&uri)
                .iter()
                .any(|d| code(d, "undefined-reference")),
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_dynamic_reader_redefinitions_execute_their_label_arguments() {
    use std::process::Command;
    for source in dynamic_sources() {
        let directory = tempfile::tempdir().unwrap();
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        std::fs::write(
            directory.path().join("commands.sty"),
            r"\renewcommand\typeout[1]{#1}",
        )
        .unwrap();
        for _ in 0..3 {
            let output = Command::new("pdflatex")
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
                "{source}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        let aux = std::fs::read_to_string(directory.path().join("main.aux")).unwrap();
        let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
        assert_eq!(aux.matches(r"\newlabel{same}").count(), 1, "{source}");
        assert!(!log.contains("Reference `same' on page 1 undefined"));
        let mut workspace = Workspace::default();
        workspace.config.distribution.enabled = false;
        workspace.open(uri.clone(), source.into(), 5);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("same").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, &uri, at).len(),
            1,
            "{source}"
        );
    }
}

#[test]
fn known_data_declarations_do_not_shadow_real_labels() {
    for (preamble, call) in [
        ("", r"\typeout{\label{same}}"),
        ("", r"\message{\label{same}}"),
        ("", r"\csuse{typeout}{\label{same}}"),
        ("", r"\csname message\endcsname{\label{same}}"),
        (r"\def\logtext{\typeout}", r"\logtext{\label{same}}"),
        (r"\newcommand\discard[1]{}", r"\discard{\label{same}}"),
        (
            r"\newcommand\discard[2][Default]{}",
            r"\discard[\label{same}]{\label{same}}",
        ),
        (r"\def\discard#1;{}", r"\discard\label{same};"),
        (
            r"\newcommand\discard[1]{}\def\forward{\discard}",
            r"\forward{\label{same}}",
        ),
        (
            r"\newcommand\discard[1]{}",
            r"\csuse{discard}{\label{same}}",
        ),
        (
            r"\newcommand\discard[1]{}",
            r"\csname discard\endcsname{\label{same}}",
        ),
    ] {
        let (workspace, uri) = setup(preamble, &format!(r"{call}\label{{same}}\ref{{same}}"));
        let text = &workspace.documents[&uri].text;
        let real = text.source.rfind(r"\label{same}").unwrap() + 7;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, &uri, at),
            vec![Location::new(uri.clone(), text.range(real, real + 4))],
            "{preamble}/{call}"
        );
        let references = features::references(&workspace, &uri, at, true);
        assert_eq!(references.len(), 2, "{preamble}/{call}: {references:?}");
        let data_at = text.position(text.source.find(r"\label{same}").unwrap() + 8);
        assert!(features::definitions(&workspace, &uri, data_at).is_empty());
        assert!(features::references(&workspace, &uri, data_at, true).is_empty());
        assert!(features::hover(&workspace, &uri, data_at).is_none());
        let diagnostics = workspace.diagnostics(&uri);
        assert!(
            !diagnostics
                .iter()
                .any(|d| code(d, "duplicate-label") || code(d, "undefined-reference")),
            "{preamble}/{call}: {diagnostics:?}"
        );
        let hover = features::hover(&workspace, &uri, at).unwrap();
        assert!(format!("{:?}", hover.contents).contains("same"));
        // Read-only filtering must not weaken complete-edit guards.
        assert!(!matches!(
            features::rename(&workspace, &uri, at, "fresh"),
            Ok(Some(_))
        ));
    }
}

#[test]
fn shared_files_keep_labels_that_execute_in_any_consumer() {
    let directory = tempfile::tempdir().unwrap();
    let a = Url::from_file_path(directory.path().join("a.tex")).unwrap();
    let b = a.join("b.tex").unwrap();
    let shared = a.join("shared.tex").unwrap();
    let inputs = [
        (
            &a,
            r"\documentclass{article}\begin{document}\input{shared}\label{same}\ref{same}\end{document}",
        ),
        (
            &b,
            r"\documentclass{article}\renewcommand\typeout[1]{#1}\begin{document}\input{shared}\label{same}\ref{same}\end{document}",
        ),
        (&shared, r"\typeout{\label{same}}\ref{same}"),
    ];
    for (uri, source) in &inputs {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    for (uri, source) in inputs {
        workspace.open(uri.clone(), source.into(), 5);
    }
    for (uri, count) in [(&a, 1), (&b, 2), (&shared, 3)] {
        let text = &workspace.documents[uri].text;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, uri, at).len(),
            count,
            "{uri}"
        );
    }
    assert!(!workspace
        .diagnostics(&a)
        .iter()
        .any(|d| code(d, "duplicate-definition")));
    assert!(workspace
        .diagnostics(&b)
        .iter()
        .any(|d| code(d, "duplicate-definition")));
}

#[test]
fn unfinished_data_and_hyperdef_labels_use_the_same_filter() {
    for source in [
        r"\typeout{\label{ghost}",
        r"\message{\label{ghost}",
        r"\newcommand\discard[1]{}\discard{\label{ghost}",
        r"\typeout{\hyperdef[ghost]{category}{destination}{Text}",
    ] {
        let (mut workspace, uri) = setup("", "");
        workspace.open(uri.clone(), source.into(), 6);
        let text = &workspace.documents[&uri].text;
        assert!(
            !workspace.documents[&uri].syntax.labels.is_empty(),
            "{source}"
        );
        let at = text.position(source.find("ghost").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_data_readers_agree_with_aux_labels_and_undefined_warnings() {
    use std::process::Command;
    for (preamble, call, count) in [
        (r"\providecommand\textbf[1]{}", r"\textbf{\label{same}}", 1),
        (
            r"\usepackage{etoolbox}\providerobustcmd\textbf[1]{}",
            r"\textbf{\label{same}}",
            1,
        ),
        ("", r"\typeout{\label{same}}", 0),
        ("", r"\message{\label{same}}", 0),
        (r"\def\logtext{\typeout}", r"\logtext{\label{same}}", 0),
        (r"\newcommand\discard[1]{}", r"\discard{\label{same}}", 0),
        (
            r"\newcommand\discard[1]{}\def\forward{\discard}",
            r"\forward{\label{same}}",
            0,
        ),
        (r"\def\discard#1;{}", r"\discard\label{same};", 0),
        (r"\newcommand\one[1]{#1}", r"\one{\label{same}}", 1),
        (
            r"\newcommand\strong[1]{\textbf{#1}}",
            r"\strong{\label{same}}",
            1,
        ),
    ] {
        for with_real in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
            let real = if with_real { r"\label{same}" } else { "" };
            let source = format!("% 😀\r\n\\documentclass{{article}}{preamble}\\begin{{document}}\\section{{Heading}}{call}{real}\\ref{{same}}\\end{{document}}");
            std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
            for _ in 0..3 {
                let output = Command::new("pdflatex")
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
            let expected = count + usize::from(with_real);
            let aux = std::fs::read_to_string(directory.path().join("main.aux")).unwrap();
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            assert_eq!(aux.matches(r"\newlabel{same}").count(), expected);
            assert_eq!(
                log.contains("Reference `same' on page 1 undefined"),
                expected == 0
            );
            assert_eq!(log.contains("Label `same' multiply defined"), expected > 1);
            let mut workspace = Workspace::default();
            workspace.config.distribution.enabled = false;
            workspace.open(uri.clone(), source.clone(), 5);
            let at = workspace.documents[&uri]
                .text
                .position(source.rfind("same").unwrap() + 1);
            assert_eq!(
                features::definitions(&workspace, &uri, at).len(),
                expected,
                "{preamble}/{call}"
            );
            let diagnostics = workspace.diagnostics(&uri);
            assert_eq!(
                diagnostics.iter().any(|d| code(d, "undefined-reference")),
                expected == 0
            );
            assert_eq!(
                diagnostics.iter().any(|d| code(d, "duplicate-label")),
                expected > 1
            );
        }
    }
}

#[test]
fn data_labels_do_not_satisfy_missing_references_or_supply_completion() {
    for (preamble, call) in [
        ("", r"\typeout{\label{ghost}}"),
        ("", r"\message{\label{ghost}}"),
        (r"\newcommand\discard[1]{}", r"\discard{\label{ghost}}"),
    ] {
        let (workspace, uri) = setup(preamble, &format!(r"{call}\ref{{ghost}}"));
        let text = &workspace.documents[&uri].text;
        let start = text.source.rfind("ghost").unwrap();
        let at = text.position(start + 1);
        assert!(features::definitions(&workspace, &uri, at).is_empty());
        assert!(features::hover(&workspace, &uri, at).is_none());
        let diagnostics = workspace.diagnostics(&uri);
        let missing: Vec<_> = diagnostics
            .iter()
            .filter(|d| code(d, "undefined-reference"))
            .collect();
        assert_eq!(missing.len(), 1, "{call}: {diagnostics:?}");
        assert_eq!(missing[0].range, text.range(start, start + 5));
        assert!(
            !features::completion(&workspace, &uri, text.position(start + 2))
                .iter()
                .any(|item| item.label == "ghost")
        );
    }
}

#[test]
fn unknown_execution_and_unproved_reader_installation_keep_structural_labels() {
    for (preamble, call) in [
        (r"\newcommand\one[1]{#1}", r"\one{\label{same}}"),
        (
            r"\newcommand\strong[1]{\textbf{#1}}",
            r"\strong{\label{same}}",
        ),
        (r"\providecommand\textbf[1]{}", r"\textbf{\label{same}}"),
        (r"\providerobustcmd\textbf[1]{}", r"\textbf{\label{same}}"),
        (
            r"\newcommand\discard[1]{}\def\readername{discard}\expandafter\def\csname\readername\endcsname#1{#1}",
            r"\discard{\label{same}}",
        ),
        (
            r"\def\readername{typeout}",
            r"\expandafter\def\csname\readername\endcsname#1{#1}\typeout{\label{same}}",
        ),
        (
            r"\def\readername{message}",
            r"\csdef{\readername}#1{#1}\message{\label{same}}",
        ),
        (r"\newcommand\later{\label{same}}", ""),
        (
            r"\typeout{\newcommand\discard[1]{}}",
            r"\discard{\label{same}}",
        ),
        (
            r"\newcommand\later{\newcommand\discard[1]{}}",
            r"\discard{\label{same}}",
        ),
        (r"{\newcommand\discard[1]{}}", r"\discard{\label{same}}"),
        (
            r"\iffalse\newcommand\discard[1]{}\fi",
            r"\discard{\label{same}}",
        ),
        (
            r"\newcommand\discard[1]{}\renewcommand\discard[1]{#1}",
            r"\discard{\label{same}}\relax",
        ),
        ("", r"\discard{\label{same}}\newcommand\discard[1]{}"),
        (
            r"\newcommand\discard[1]{}\appto\discard{#1}",
            r"\discard{\label{same}}",
        ),
        (
            r"\newcommand\discard[1]{}\csappto{discard}{#1}",
            r"\discard{\label{same}}",
        ),
        (
            r"\newcommand\discard[1]{}\csuse{csappto}{discard}{#1}",
            r"\discard{\label{same}}",
        ),
        (
            r"\def\strip#1{\expandafter\expandafter\expandafter\identity#1}\def\identity#1{#1}",
            r"\strip{\typeout{\label{same}}}",
        ),
    ] {
        let (workspace, uri) = setup(preamble, &format!(r"{call}\ref{{same}}"));
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, &uri, at).len(),
            1,
            "{preamble}/{call}"
        );
        assert!(
            !workspace
                .diagnostics(&uri)
                .iter()
                .any(|d| code(d, "undefined-reference")),
            "{preamble}/{call}"
        );
    }
}

#[test]
fn unsaved_native_reader_overrides_refresh_without_editing_the_parent() {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = uri.join("commands.sty").unwrap();
    let source = "% 😀\r\n\\documentclass{article}\\input{commands.sty}\\begin{document}\\typeout{\\label{ghost}}\\ref{ghost}\\end{document}";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    std::fs::write(provider.to_file_path().unwrap(), "").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 5);
    let at = workspace.documents[&uri]
        .text
        .position(source.rfind("ghost").unwrap() + 1);
    for (version, replacement, expected) in [
        (12, r"\renewcommand\typeout[1]{#1}", 1),
        (13, "", 0),
        (14, r"\renewcommand\typeout[1]{#1}", 1),
    ] {
        workspace.open(provider.clone(), replacement.into(), version);
        assert_eq!(features::definitions(&workspace, &uri, at).len(), expected);
        assert_eq!(
            workspace
                .diagnostics(&uri)
                .iter()
                .filter(|d| code(d, "undefined-reference"))
                .count(),
            1 - expected
        );
        assert_eq!(workspace.documents[&uri].version, Some(5));
    }
    workspace.documents.remove(&provider);
    workspace.reload(&provider);
    assert!(features::definitions(&workspace, &uri, at).is_empty());
}
