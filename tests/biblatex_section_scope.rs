use latex_lsp::{editing, features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(header: &str) -> (tempfile::TempDir, Workspace, Url, Url, String) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}\begin{{document}}{header}\label{{collision}}\ref{{collision}}\cite{{collision,absent}}\end{{document}}"
    );
    for (name, text) in [
        ("main.tex", source.as_str()),
        ("default.bib", "@book{collision,title={Default}}"),
        ("section.bib", "@book{collision,title={Selected}}"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = Url::from_file_path(dir.path().join("default.bib")).unwrap();
    ws.open(main.clone(), source.clone(), 5);
    (dir, ws, main, bib, source)
}

#[test]
fn selected_sections_do_not_bind_keys_to_the_default_database_or_offer_partial_edits() {
    for header in [
        r"\newrefsection[chosen]",
        r"\refsection[section.bib]",
        r"\begin{refsection}[chosen]",
        r"\csuse{newrefsection}[chosen]",
        r"\csname newrefsection\endcsname[chosen]",
        r"\csuse{begin}{refsection}[chosen]",
        r"\csname begin\endcsname{refsection}[chosen]",
        r"\def\choose{\newrefsection}\choose[chosen]",
        r"\let\choose\newrefsection\choose[chosen]",
        r"\let\choose\newrefsection\def\newrefsection#1{}\choose[chosen]",
        r"\let\saved\newrefsection\def\choose{\saved}\choose[chosen]",
        r"\newrefsection[ref%name.bib]",
        r"\newrefsection[\dynamic]",
        r"\newcommand\later{\newrefsection[chosen]}",
    ] {
        let (_dir, ws, main, bib, source) = setup(header);
        let text = Text::new(source.clone());
        let at = text.position(source.find(r"\cite{collision").unwrap() + 8);
        assert!(features::definitions(&ws, &main, at).is_empty(), "{header}");
        assert!(
            features::prepare_rename(&ws, &main, at).unwrap().is_none(),
            "{header}"
        );
        assert!(features::hover(&ws, &main, at).is_none(), "{header}");
        assert!(editing::highlights(&ws, &main, at).is_empty(), "{header}");
        assert!(
            !features::completion(&ws, &main, at)
                .iter()
                .any(|item| item.label == "collision"),
            "{header}"
        );
        assert!(
            features::rename(&ws, &bib, Position::new(0, 8), "updated").is_err(),
            "{header}"
        );
        assert_eq!(
            features::references(&ws, &bib, Position::new(0, 8), true).len(),
            1,
            "{header}"
        );
        assert!(
            !ws.diagnostics(&main)
                .iter()
                .any(|diagnostic| diagnostic.message == "Undefined citation `absent`"),
            "{header}"
        );
        let at = text.position(source.find(r"\ref{collision").unwrap() + 7);
        assert_eq!(features::definitions(&ws, &main, at).len(), 1, "{header}");
        assert_eq!(
            features::rename(&ws, &main, at, "updated")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&main]
                .len(),
            2,
            "{header}"
        );
    }
}

#[test]
fn default_resources_empty_lists_data_and_redefined_readers_keep_existing_interactions() {
    for header in [
        "",
        r"\newrefsection",
        r"\newrefsection[]",
        r"\newrefsection[  ]",
        r"\begin{refsection}[]",
        r"\begin{refsegment}",
        r"\newrefsegment",
        r"\newcommand\discard[1]{}\discard{\newrefsection[chosen]}",
        r"\verb|\newrefsection[chosen]|",
        r"\detokenize{\newrefsection[chosen]}",
        r"\newcommand\newrefsection[1][]{}\newrefsection[chosen]",
        r"\newenvironment{refsection}[1][]{}{}\begin{refsection}[chosen]",
        "\\textbf{\\newrefsection[% comment\r\n ]}",
    ] {
        let (_dir, ws, main, bib, source) = setup(header);
        let at = Text::new(source.clone()).position(source.find(r"\cite{collision").unwrap() + 8);
        assert_eq!(features::definitions(&ws, &main, at).len(), 1, "{header}");
        assert!(
            features::rename(&ws, &bib, Position::new(0, 8), "updated").is_ok(),
            "{header}"
        );
        assert!(
            ws.diagnostics(&main)
                .iter()
                .any(|diagnostic| diagnostic.message == "Undefined citation `absent`"),
            "{header}"
        );
    }
}

#[test]
fn imported_unsaved_section_readers_refresh_key_protection_and_preserve_independent_projects() {
    let (dir, mut ws, main, bib, source) = setup(r"\input{definitions}\choose[chosen]");
    let provider = Url::from_file_path(dir.path().join("definitions.tex")).unwrap();
    std::fs::write(
        provider.to_file_path().unwrap(),
        r"\newcommand\choose[1][]{}",
    )
    .unwrap();
    ws.open(provider.clone(), r"\def\choose{\newrefsection}".into(), 8);
    assert!(features::rename(&ws, &bib, Position::new(0, 8), "updated").is_err());
    let independent = Url::from_file_path(dir.path().join("independent.tex")).unwrap();
    let independent_source = r"\documentclass{article}\bibitem{other}\cite{other}";
    ws.open(independent.clone(), independent_source.into(), 2);
    let at = Text::new(independent_source.into())
        .position(independent_source.rfind("other").unwrap() + 1);
    assert_eq!(
        features::rename(&ws, &independent, at, "changed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&independent]
            .len(),
        2
    );
    ws.open(provider, r"\newcommand\choose[1][]{}".into(), 9);
    assert!(features::rename(&ws, &bib, Position::new(0, 8), "updated").is_ok());
    let at = Text::new(source.clone()).position(source.find(r"\cite{collision").unwrap() + 8);
    assert_eq!(features::definitions(&ws, &main, at).len(), 1);
}

#[test]
#[ignore = "requires pdflatex, biblatex and Biber"]
fn real_section_resources_reject_the_partial_rename_that_breaks_biber_resolution() {
    let (dir, ws, main, bib, source) = setup(r"\newrefsection[chosen]");
    assert!(features::rename(&ws, &bib, Position::new(0, 8), "updated").is_err());
    let source = source
        .replace(",absent", "")
        .replace(r"\end{document}", r"\printbibliography\end{document}");
    let broken = source.replace(r"\cite{collision}", r"\cite{updated}");
    for (text, renamed_default, broken) in [(&source, false, false), (&broken, true, true)] {
        std::fs::write(dir.path().join("main.tex"), text).unwrap();
        if renamed_default {
            std::fs::write(
                bib.to_file_path().unwrap(),
                "@book{updated,title={Default}}",
            )
            .unwrap();
        }
        let latex = || {
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
        };
        latex();
        let output = std::process::Command::new("biber")
            .arg("main")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        latex();
        latex();
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        assert_eq!(log.contains("undefined"), broken, "{log}");
        if !broken {
            let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
            assert!(bbl.contains("Selected"));
            assert!(!bbl.contains("Default"));
        }
    }
    let at = Text::new(source.clone()).position(source.find(r"\cite{collision").unwrap() + 8);
    assert!(features::definitions(&ws, &main, at).is_empty());
}

#[test]
fn shared_citations_require_agreement_between_default_and_selected_resource_consumers() {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in [
        (
            "a.tex",
            r"\documentclass{article}\addbibresource{default.bib}\begin{document}\newrefsection[section.bib]\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\addbibresource{default.bib}\begin{document}\input{shared}\cite{collision}",
        ),
        ("shared.tex", r"\cite{collision}"),
        ("default.bib", "@book{collision,title={Default}}"),
        ("section.bib", "@book{collision,title={Selected}}"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let shared = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let default = Url::from_file_path(dir.path().join("default.bib")).unwrap();
    let b = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    assert!(features::definitions(&ws, &shared, Position::new(0, 8)).is_empty());
    assert!(features::rename(&ws, &default, Position::new(0, 8), "updated").is_err());
    let source = &ws.documents[&b].text.source;
    let at = ws.documents[&b]
        .text
        .position(source.rfind("collision").unwrap() + 1);
    assert_eq!(features::definitions(&ws, &b, at).len(), 1);
}
