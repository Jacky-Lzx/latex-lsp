use latex_lsp::{language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;

fn setup(extra: &str, body: &str) -> (tempfile::TempDir, Workspace, Url, String) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}{extra}\begin{{document}}{body}\end{{document}}"
    );
    for (name, text) in [
        ("main.tex", source.as_str()),
        ("default.bib", "@book{collision,title={Default}}"),
        ("section.bib", "@book{collision,title={Selected}}"),
        ("other.bib", "@book{collision,title={Other}}"),
        ("global.bib", "@book{always,title={Global}}"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source.clone(), 5);
    (dir, ws, main, source)
}

fn check(ws: &Workspace, uri: &Url, key: &str, targets: &[&str]) {
    let source = &ws.documents[uri].text.source;
    let positions: Vec<_> = source
        .match_indices(&format!(r"\cite{{{key}}}"))
        .map(|(at, _)| ws.documents[uri].text.position(at + 7))
        .collect();
    assert_eq!(positions.len(), targets.len());
    for (at, expected) in positions.into_iter().zip(targets) {
        let locations = features::definitions(ws, uri, at);
        if expected.is_empty() {
            assert!(locations.is_empty(), "{locations:?}");
            assert!(features::hover(ws, uri, at).is_none());
        } else {
            assert_eq!(locations.len(), 1, "{expected}: {locations:?}");
            assert!(locations[0].uri.path().ends_with(expected), "{locations:?}");
            let hover = features::hover(ws, uri, at).unwrap();
            let HoverContents::Markup(contents) = hover.contents else {
                panic!("{hover:?}")
            };
            assert_eq!(contents.kind, MarkupKind::Markdown);
            assert_eq!(
                contents.value,
                format!(
                    "```bibtex\n{}\n```",
                    ws.documents[&locations[0].uri].text.source.trim()
                )
            );
        }
    }
}

#[test]
fn citations_resolve_and_rename_complete_default_selected_and_global_identities() {
    let (_dir, ws, main, source) = setup(
        r"\addglobalbib{global.bib}",
        r"\cite{collision}\newrefsection[chosen]\cite{collision}\cite{always}\newrefsection[]\cite{collision}\refsection[other]\cite{collision}\endrefsection\cite{collision}",
    );
    check(
        &ws,
        &main,
        "collision",
        &[
            "default.bib",
            "section.bib",
            "default.bib",
            "other.bib",
            "default.bib",
        ],
    );
    check(&ws, &main, "always", &["global.bib"]);
    let at = Text::new(source.clone()).position(source.find(r"\cite{collision}").unwrap() + 7);
    let edit = features::rename(&ws, &main, at, "updated")
        .unwrap()
        .unwrap();
    let changes = edit.changes.unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[&main].len(), 3);
    assert_eq!(changes[&main.join("default.bib").unwrap()].len(), 1);
    for (name, count) in [("default.bib", 3), ("section.bib", 1), ("global.bib", 1)] {
        let bib = main.join(name).unwrap();
        let changes = features::rename(&ws, &bib, Position::new(0, 8), "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes.len(), 2, "{name}");
        assert_eq!(changes[&bib].len(), 1, "{name}");
        assert_eq!(changes[&main].len(), count, "{name}");
    }
}

#[test]
fn sections_survive_tex_groups_and_environment_end_returns_to_default_resources() {
    let (_dir, ws, main, _) = setup(
        "",
        r"{\newrefsection[chosen]}\cite{collision}\begingroup\newrefsection[other.bib]\endgroup\cite{collision}\begin{refsection}[chosen]\cite{collision}\newrefsection[]\cite{collision}\end{refsection}\cite{collision}",
    );
    check(
        &ws,
        &main,
        "collision",
        &[
            "section.bib",
            "other.bib",
            "section.bib",
            "default.bib",
            "default.bib",
        ],
    );
}

#[test]
fn repeated_file_registration_keeps_global_role_independent_of_section_registration() {
    let (_dir, ws, main, _) = setup(
        r"\addsectionbib[label=shared]{global.bib}\addglobalbib{global.bib}",
        r"\cite{always}\newrefsection[chosen]\cite{always}\cite{collision}",
    );
    check(&ws, &main, "always", &["global.bib", "global.bib"]);
    check(&ws, &main, "collision", &["section.bib"]);
}

#[test]
fn included_citations_inherit_each_call_and_require_agreement_between_visits() {
    let (dir, mut ws, main, source) = setup("", r"\newrefsection[chosen]\input{chapter}");
    let chapter = Url::from_file_path(dir.path().join("chapter.tex")).unwrap();
    std::fs::write(chapter.to_file_path().unwrap(), r"\cite{collision}").unwrap();
    ws.open(main.clone(), source.clone(), 5);
    check(&ws, &chapter, "collision", &["section.bib"]);
    let repeated = source.replace(
        r"\end{document}",
        r"\endrefsection\input{chapter}\end{document}",
    );
    ws.open(main.clone(), repeated, 6);
    check(&ws, &chapter, "collision", &[""]);
    ws.open(main, source, 7);
    check(&ws, &chapter, "collision", &["section.bib"]);
}

#[test]
fn unsaved_selection_changes_refresh_child_targets_without_changing_child_versions() {
    let (dir, mut ws, main, source) = setup("", r"\newrefsection[chosen]\input{chapter}");
    let chapter = Url::from_file_path(dir.path().join("chapter.tex")).unwrap();
    std::fs::write(chapter.to_file_path().unwrap(), r"\cite{collision}").unwrap();
    ws.open(main.clone(), source.clone(), 5);
    ws.open(chapter.clone(), r"\cite{collision}".into(), 3);
    for (stage, selection) in ["chosen", "other.bib", "chosen"].into_iter().enumerate() {
        ws.open(
            main.clone(),
            source.replace("[chosen]", &format!("[{selection}]")),
            8 + stage as i32,
        );
        assert_eq!(ws.documents[&chapter].version, Some(3));
        check(
            &ws,
            &chapter,
            "collision",
            &[if stage == 1 {
                "other.bib"
            } else {
                "section.bib"
            }],
        );
    }
}

#[test]
fn uncertain_execution_and_automatic_sections_do_not_certify_a_default_or_selected_target() {
    for (extra, body) in [
        (
            "",
            r"\newcommand\run[1]{#1}\run{\newrefsection[chosen]}\cite{collision}",
        ),
        (
            "",
            r"\newcommand\later{\newrefsection[chosen]}\cite{collision}",
        ),
        ("", r"\newrefsection[\dynamic]\cite{collision}"),
        (
            "",
            r"\newrefsection[chosen]\input{\dynamic}\cite{collision}",
        ),
        (
            "",
            r"\newrefsection[chosen]\iftrue\endrefsection\fi\cite{collision}",
        ),
        (
            r"\ExecuteBibliographyOptions{refsection = section}",
            r"\newrefsection[chosen]\section{Next}\cite{collision}",
        ),
        (
            r"\AtBeginDocument{\newrefsection[chosen]}",
            r"\cite{collision}",
        ),
        ("", r"\newrefsection[missing.bib]\cite{collision}"),
        (
            "",
            r"\newrefsection[chosen]\setcounter{refsection}{0}\cite{collision}",
        ),
        (
            "",
            r"\newrefsection[chosen]\expandafter\endrefsection\relax\cite{collision}",
        ),
        (
            "",
            r"\let\choose\newrefsection\choose[chosen]\cite{collision}",
        ),
    ] {
        let (_dir, ws, main, _) = setup(extra, body);
        check(&ws, &main, "collision", &[""]);
    }
}

#[test]
fn static_named_section_and_citation_calls_preserve_source_ranges() {
    for body in [
        "😀\r\n\\csuse{newrefsection}[chosen]\\csuse{cite}{collision}",
        "😀\r\n\\csname newrefsection\\endcsname[chosen]\\csname cite\\endcsname{collision}",
    ] {
        let (_dir, ws, main, source) = setup("", body);
        let offset = source.rfind("collision").unwrap();
        let at = ws.documents[&main].text.position(offset + 2);
        let locations = features::definitions(&ws, &main, at);
        assert_eq!(locations.len(), 1, "{body}: {locations:?}");
        assert!(locations[0].uri.path().ends_with("section.bib"));
        assert_eq!(
            features::hover(&ws, &main, at).unwrap().range,
            Some(ws.documents[&main].text.range(offset, offset + 9))
        );
    }
}

#[test]
fn shared_sources_resolve_only_when_every_root_selects_the_same_resources() {
    let (dir, mut ws, main, source) = setup("", r"\newrefsection[chosen]\input{shared}");
    let shared = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    std::fs::write(shared.to_file_path().unwrap(), r"\cite{collision}").unwrap();
    ws.open(main, source.clone(), 5);
    let other = Url::from_file_path(dir.path().join("other.tex")).unwrap();
    ws.open(other.clone(), source.replace("[chosen]", "[other.bib]"), 8);
    check(&ws, &shared, "collision", &[""]);
    ws.open(other, source, 9);
    check(&ws, &shared, "collision", &["section.bib"]);
}

#[test]
#[ignore = "requires pdflatex, biblatex and Biber"]
fn real_static_section_targets_agree_with_bcf_and_compiled_bibliographies() {
    let (dir, ws, main, _) = setup(
        r"\addglobalbib{global.bib}",
        r"\cite{collision}\cite{always}\printbibliography\newrefsection[chosen]\cite{collision}\cite{always}\printbibliography\newrefsection[other.bib]\cite{collision}\cite{always}\printbibliography\endrefsection\cite{collision}",
    );
    check(
        &ws,
        &main,
        "collision",
        &["default.bib", "section.bib", "other.bib", "default.bib"],
    );
    check(
        &ws,
        &main,
        "always",
        &["global.bib", "global.bib", "global.bib"],
    );
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
    let bcf = std::fs::read_to_string(dir.path().join("main.bcf")).unwrap();
    for (section, expected) in [(0, "default.bib"), (1, "section.bib"), (2, "other.bib")] {
        let marker = format!("<bcf:bibdata section=\"{section}\">");
        let resources = bcf
            .split_once(&marker)
            .unwrap()
            .1
            .split_once("</bcf:bibdata>")
            .unwrap()
            .0;
        for filename in ["default.bib", "section.bib", "other.bib", "global.bib"] {
            assert_eq!(
                resources.contains(filename),
                filename == expected || filename == "global.bib",
                "{resources}"
            );
        }
    }
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
    assert!(!log.contains("undefined"), "{log}");
    let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
    for title in ["Default", "Selected", "Other", "Global"] {
        assert!(bbl.contains(title), "{bbl}");
    }
}

#[test]
fn unrelated_macro_code_preserves_active_targets_without_certifying_deferred_citations() {
    let (_dir, ws, main, _) = setup(
        r"\newcommand\display[1]{\textbf{#1}}\newcommand\later{\cite{collision}}",
        r"\display{text}\newrefsection[chosen]\cite{collision}",
    );
    check(&ws, &main, "collision", &["", "section.bib"]);
}

#[test]
fn input_relative_path_fallback_does_not_certify_a_native_bibliography_target() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("subdir")).unwrap();
    let source = r"\documentclass{article}\usepackage{biblatex}\input{subdir/setup}\begin{document}\newrefsection[chosen]\cite{collision}\end{document}";
    for (name, content) in [
        ("main.tex", source),
        (
            "subdir/setup.tex",
            r"\addsectionbib[label=chosen]{section.bib}",
        ),
        ("subdir/section.bib", "@book{collision,title={Child}}"),
    ] {
        std::fs::write(dir.path().join(name), content).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let provider = Url::from_file_path(dir.path().join("subdir/setup.tex")).unwrap();
    let library = Url::from_file_path(dir.path().join("subdir/section.bib")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source.into(), 5);
    assert!(ws.documents.contains_key(&library));
    check(&ws, &main, "collision", &[""]);
    ws.open(
        provider,
        r"\addsectionbib[label=chosen]{subdir/section.bib}".into(),
        8,
    );
    assert_eq!(ws.documents[&main].version, Some(5));
    check(&ws, &main, "collision", &["subdir/section.bib"]);
}

#[test]
fn unsaved_selected_root_prevents_falling_back_to_a_saved_default_only_consumer() {
    let (dir, mut ws, main, source) = setup("", r"\input{shared}");
    let shared = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    std::fs::write(shared.to_file_path().unwrap(), r"\cite{collision}").unwrap();
    ws.open(main, source.clone(), 5);
    check(&ws, &shared, "collision", &["default.bib"]);
    let other = Url::from_file_path(dir.path().join("new-root.tex")).unwrap();
    ws.open(
        other.clone(),
        source.replace(r"\input{shared}", r"\newrefsection[chosen]\input{shared}"),
        8,
    );
    check(&ws, &shared, "collision", &[""]);
    ws.open(other, source, 9);
    check(&ws, &shared, "collision", &["default.bib"]);
}
