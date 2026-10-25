use latex_lsp::{language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    let uri = Url::parse("untitled:copied-reference.tex").unwrap();
    ws.open(uri.clone(), source.into(), 3);
    (ws, uri)
}

fn blocked(source: &str, key: &str) {
    let (ws, uri) = setup(source);
    let position = Text::new(source.into()).position(source.find(key).unwrap() + 1);
    assert!(
        features::rename(&ws, &uri, position, "updated").is_err(),
        "{source}"
    );
}

#[test]
fn captured_reference_calls_block_partial_key_renames() {
    for (declaration, builtin) in [(r"\label{paper}", "ref"), (r"\bibitem{paper}", "cite")] {
        for copy in [
            format!(r"\let\saved\{builtin}"),
            format!(r"\NewCommandCopy{{\saved}}{{\{builtin}}}"),
            format!(r"\csletcs{{saved}}{{{builtin}}}"),
            format!(r"\letcs\saved{{{builtin}}}"),
            format!(r"\cslet{{saved}}{{\{builtin}}}"),
        ] {
            for call in [
                r"\saved{paper}",
                r"\csuse{saved}{paper}",
                r"\csname saved\endcsname{paper}",
            ] {
                blocked(&format!("😀 {declaration}{copy}{call}"), "paper");
            }
        }
    }
}

#[test]
fn captured_chains_redefinitions_and_multiple_possible_kinds_remain_protected() {
    for tail in [
        r"\let\saved\ref\def\ref#1{}\saved{paper}",
        r"\def\link{\ref}\let\saved\link\def\link#1{}\saved{paper}",
        r"\let\saved\ref\def\link{\saved}\link{paper}",
        r"\let\saved\ref\let\next\saved\next{paper}",
        r"\let\saved\ref\let\saved\cite\saved{paper}",
        r"\let\saved\ref\def\cycle{\saved}\def\saved{\cycle}\cycle{paper}",
        r"\let\saved\ref\newcommand\later{\saved{paper}}",
    ] {
        blocked(&format!(r"\label{{paper}}{tail}"), "paper");
    }
    blocked(
        r"\bibitem{paper}\let\saved\ref\let\saved\cite\saved{paper}",
        "paper",
    );
}

#[test]
fn captured_grammars_protect_actual_keys_and_preserve_notes_and_other_namespaces() {
    for (declaration, builtin, arguments) in [
        (r"\label{paper}", "crefrange", "{other}{paper}"),
        (r"\label{paper}", "hyperref", "[paper]{other}"),
        (r"\bibitem{paper}", "cite", "[other][note]{other,paper}"),
        (r"\bibitem{paper}", "cites", "[note]{other}[note]{paper}"),
        (r"\bibitem{paper}", "ifentrycategory", "{paper}{other}{}{}"),
    ] {
        blocked(
            &format!(r"{declaration}\let\saved\{builtin}\saved{arguments}"),
            "paper",
        );
    }
    for source in [
        r"\label{paper}\let\saved\cite\saved{paper}",
        r"\bibitem{paper}\let\saved\ref\saved{paper}",
        r"\bibitem{paper}\let\saved\cite\saved[paper]{other}",
        r"\label{paper}\let\saved\hyperref\saved[other]{paper}",
        r"\bibitem{paper}\let\saved\ifentrycategory\saved{other}{paper}{}{}",
    ] {
        let (ws, uri) = setup(source);
        let position = Text::new(source.into()).position(source.find("paper").unwrap());
        assert_eq!(
            features::rename(&ws, &uri, position, "updated")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            1,
            "{source}"
        );
    }
}

#[test]
fn unused_copies_and_literal_copied_examples_do_not_block_renames() {
    for tail in [
        r"\let\saved\ref",
        r"\let\saved\ref\verb|\saved{paper}|",
        r"\let\saved\ref\detokenize{\saved{paper}}",
        r"\let\saved\ref% \saved{paper}",
    ] {
        let source = format!(r"\label{{paper}}{tail}");
        let (ws, uri) = setup(&source);
        assert_eq!(
            features::rename(&ws, &uri, Position::new(0, 9), "updated")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            1,
            "{source}"
        );
    }
}

#[test]
fn copied_reference_meanings_remain_separate_from_confirmed_navigation() {
    let source = r"\label{paper}\let\saved\ref\saved{paper}";
    let (ws, uri) = setup(source);
    let position = Text::new(source.into()).position(source.rfind("paper").unwrap());
    assert!(features::definitions(&ws, &uri, position).is_empty());
    assert!(features::prepare_rename(&ws, &uri, position)
        .unwrap()
        .is_none());
}

#[test]
fn copied_reference_guards_follow_project_scope_and_unsaved_changes() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "main.tex",
            r"\documentclass{article}\input{macros}\input{chapter}",
        ),
        ("macros.tex", r"\let\saved\ref"),
        ("chapter.tex", r"\label{paper}\saved{paper}"),
        (
            "other.tex",
            r"\documentclass{article}\label{paper}\let\saved\cite\saved{paper}",
        ),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let chapter = Url::from_file_path(dir.path().join("chapter.tex")).unwrap();
    let other = Url::from_file_path(dir.path().join("other.tex")).unwrap();
    let macros = Url::from_file_path(dir.path().join("macros.tex")).unwrap();
    assert!(features::rename(&ws, &chapter, Position::new(0, 9), "updated").is_err());
    let source = &ws.documents[&other].text;
    assert_eq!(
        features::rename(
            &ws,
            &other,
            source.position(source.source.find("paper").unwrap()),
            "updated"
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap()[&other]
            .len(),
        1
    );
    ws.open(macros, r"\newcommand\saved[1]{}".into(), 7);
    assert_eq!(
        features::rename(&ws, &chapter, Position::new(0, 9), "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&chapter]
            .len(),
        1
    );
}

#[test]
fn captured_reference_analysis_does_not_return_partial_results_past_its_budget() {
    let source = format!(
        r"\label{{paper}}\label{{other}}\let\saved\ref{}",
        r"\saved{paper}".repeat(20_001)
    );
    let (ws, uri) = setup(&source);
    assert!(features::rename(
        &ws,
        &uri,
        Text::new(source.clone()).position(source.find("other").unwrap()),
        "updated"
    )
    .is_err());
}

#[test]
#[ignore = "requires pdflatex"]
fn real_captured_reference_calls_break_when_only_the_declaration_is_renamed() {
    for (declaration, builtin) in [
        (r"\section{Section}\label{paper}", "ref"),
        (
            r"\begin{thebibliography}{1}\bibitem{paper}A book.\end{thebibliography}",
            "cite",
        ),
    ] {
        let source = format!(
            r"\documentclass{{article}}\let\saved\{builtin}\begin{{document}}{declaration}\saved{{paper}}\end{{document}}"
        );
        blocked(&source, "paper");
        let broken = source.replacen("paper", "changed", 1);
        let dir = tempfile::tempdir().unwrap();
        for (text, has_warning) in [(&source, false), (&broken, true)] {
            std::fs::write(dir.path().join("main.tex"), text).unwrap();
            let mut log = String::new();
            for _ in 0..2 {
                let output = std::process::Command::new("pdflatex")
                    .args([
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ])
                    .current_dir(dir.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stdout)
                );
                log = String::from_utf8(output.stdout).unwrap();
            }
            assert_eq!(log.contains("undefined"), has_warning, "{log}");
        }
    }
}
#[test]
fn captured_citations_protect_database_keys_across_shared_consumer_meanings() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "a.tex",
            r"\documentclass{article}\bibliography{refs}\let\saved\cite\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\bibliography{refs}\newcommand\saved[1]{}\input{shared}",
        ),
        ("shared.tex", r"\saved{paper}"),
        ("refs.bib", "@book{paper,title={Book}}"),
    ] {
        std::fs::write(dir.path().join(name), source).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let refs = Url::from_file_path(dir.path().join("refs.bib")).unwrap();
    let a = Url::from_file_path(dir.path().join("a.tex")).unwrap();
    assert!(features::rename(&ws, &refs, Position::new(0, 8), "updated").is_err());
    ws.open(
        a,
        r"\documentclass{article}\bibliography{refs}\newcommand\saved[1]{}\input{shared}".into(),
        8,
    );
    assert_eq!(
        features::rename(&ws, &refs, Position::new(0, 8), "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&refs]
            .len(),
        1
    );
}
