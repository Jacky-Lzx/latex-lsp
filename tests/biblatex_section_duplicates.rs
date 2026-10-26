use lsp_types::*;
use texquill::project::workspace::Workspace;

fn setup(extra: &str, body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}{extra}\begin{{document}}{body}\end{{document}}"
    );
    for (name, text) in [
        ("main.tex", source.as_str()),
        ("default.bib", "@book{collision,title={Default}}"),
        ("section.bib", "@book{collision,title={Selected}}"),
        ("other.bib", "@book{collision,title={Other}}"),
        ("global.bib", "@book{collision,title={Global}}"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(main.clone(), source, 5);
    (dir, ws, main)
}
fn conflicts(ws: &Workspace, uri: &Url) -> Vec<Diagnostic> {
    ws.diagnostics(uri)
        .into_iter()
        .filter(|item| item.code == Some(NumberOrString::String("duplicate-definition".into())))
        .collect()
}
fn related(ws: &Workspace, main: &Url, file: &str) -> Vec<String> {
    let diagnostics = conflicts(ws, &main.join(file).unwrap());
    if diagnostics.is_empty() {
        return Vec::new();
    }
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    diagnostics[0]
        .related_information
        .as_ref()
        .unwrap()
        .iter()
        .map(|info| {
            info.location
                .uri
                .path_segments()
                .unwrap()
                .next_back()
                .unwrap()
                .to_owned()
        })
        .collect()
}
#[test]
fn independent_sections_do_not_report_cross_file_duplicate_keys() {
    let (_dir, ws, main) = setup(
        "",
        r"\cite{collision}\newrefsection[chosen]\cite{collision}",
    );
    for name in ["default.bib", "section.bib"] {
        assert!(
            conflicts(&ws, &main.join(name).unwrap()).is_empty(),
            "{name}"
        );
    }
}
#[test]
fn globals_conflict_only_with_resources_that_share_a_section() {
    let (_dir, ws, main) = setup(
        r"\addglobalbib{global.bib}",
        r"\cite{collision}\newrefsection[chosen]\cite{collision}",
    );
    assert_eq!(related(&ws, &main, "default.bib"), ["global.bib"]);
    assert_eq!(related(&ws, &main, "section.bib"), ["global.bib"]);
    assert_eq!(
        related(&ws, &main, "global.bib"),
        ["default.bib", "section.bib"]
    );
}
#[test]
fn simultaneously_selected_resources_still_report_duplicates() {
    let (_dir, ws, main) = setup("", r"\newrefsection[chosen,other]\cite{collision}");
    assert!(related(&ws, &main, "default.bib").is_empty());
    assert_eq!(related(&ws, &main, "section.bib"), ["other.bib"]);
    assert_eq!(related(&ws, &main, "other.bib"), ["section.bib"]);
}
#[test]
fn unsaved_registration_changes_refresh_conflicts_without_changing_parent_version() {
    let (dir, mut ws, main) = setup(r"\input{setup}", r"\newrefsection[chosen]\cite{collision}");
    let child = main.join("setup.tex").unwrap();
    std::fs::write(dir.path().join("setup.tex"), r"\addsectionbib{section.bib}").unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 5);
    assert!(related(&ws, &main, "default.bib").is_empty());
    for (version, command) in [(1, "addbibresource"), (2, "addsectionbib")] {
        ws.open(
            child.clone(),
            format!("\\{command}{{section.bib}}"),
            version,
        );
        assert_eq!(ws.documents[&main].version, Some(5));
        assert_eq!(
            related(&ws, &main, "default.bib"),
            if version == 1 {
                vec!["section.bib"]
            } else {
                vec![]
            }
        );
    }
}
#[test]
fn uncertain_sections_suppress_only_cross_library_key_duplicates() {
    for body in [
        r"\iftrue\newrefsection[chosen]\fi",
        r"\newcommand\run[1]{#1}\run{\newrefsection[chosen]}",
        r"\newrefsection[\dynamic]",
        r"\newrefsection[chosen]\input{\dynamic}",
    ] {
        let (dir, mut ws, main) = setup(
            r"\addglobalbib{global.bib}\label{shared}\hypertarget{dest}{}",
            &format!(r"{body}\input{{chapter}}"),
        );
        std::fs::write(
            dir.path().join("chapter.tex"),
            r"\label{shared}\hypertarget{dest}{}",
        )
        .unwrap();
        let source = ws.documents[&main].text.source.clone();
        ws.open(main.clone(), source, 5);
        assert!(related(&ws, &main, "default.bib").is_empty(), "{body}");
        let diagnostics = conflicts(&ws, &main);
        assert_eq!(diagnostics.len(), 2, "{body}: {diagnostics:?}");
        assert!(diagnostics
            .iter()
            .any(|item| item.message.contains("label `shared`")));
        assert!(diagnostics
            .iter()
            .any(|item| item.message.contains("PDF destination `dest`")));
    }
}
#[test]
fn same_library_duplicate_keys_remain_local_errors() {
    let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{collision}");
    let bib = main.join("section.bib").unwrap();
    ws.open(
        bib.clone(),
        "@book{collision,title={One}}\n@book{collision,title={Two}}".into(),
        1,
    );
    let diagnostics = ws.diagnostics(&bib);
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code
                == Some(NumberOrString::String("duplicate-bibliography-key".into()))),
        "{diagnostics:?}"
    );
    assert!(conflicts(&ws, &bib).is_empty());
}
#[test]
fn shared_consumers_contribute_only_their_actual_resource_conflicts() {
    let (dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{collision}");
    let other = main.join("other.tex").unwrap();
    let simultaneous = r"\documentclass{article}\usepackage{biblatex}\addbibresource{default.bib}\addbibresource{section.bib}\begin{document}\cite{collision}\end{document}";
    std::fs::write(dir.path().join("other.tex"), simultaneous).unwrap();
    ws.reload(&other);
    assert!(ws.documents.contains_key(&other));
    assert_eq!(related(&ws, &main, "default.bib"), ["section.bib"]);
    ws.open(
        other,
        simultaneous
            .replace(
                r"\addbibresource{section.bib}",
                r"\addsectionbib[label=chosen]{section.bib}",
            )
            .replace(
                r"\cite{collision}",
                r"\newrefsection[chosen]\cite{collision}",
            ),
        1,
    );
    assert!(related(&ws, &main, "default.bib").is_empty());
    assert!(related(&ws, &main, "section.bib").is_empty());
}
#[test]
fn literal_biblatex_examples_do_not_silence_traditional_duplicate_keys() {
    let (_dir, mut ws, main) = setup("", "");
    ws.open(main.clone(), "\\documentclass{article}\n% \\addbibresource{unused.bib}\n\\verb|\\newrefsection[chosen]|\n\\bibliography{default,section}".into(), 6);
    assert_eq!(related(&ws, &main, "default.bib"), ["section.bib"]);
}

#[test]
fn default_only_biblatex_ignores_stale_recorded_library_dependencies() {
    let (_dir, mut ws, main) = setup("", r"\cite{collision}");
    let selected = main.join("section.bib").unwrap();
    // A previous build may still list a library after its resource role changes.
    ws.project.bibliography_inputs.insert(
        main.clone(),
        std::collections::BTreeSet::from([selected.clone()]),
    );
    assert!(conflicts(&ws, &main.join("default.bib").unwrap()).is_empty());
    assert!(conflicts(&ws, &selected).is_empty());
}
#[test]
fn named_resource_registrations_and_section_calls_keep_native_resource_roles() {
    for (registration, section) in [
        (r"\csuse{addsectionbib}", r"\csuse{newrefsection}"),
        (
            r"\csname addsectionbib\endcsname",
            r"\csname newrefsection\endcsname",
        ),
    ] {
        let (_dir, mut ws, main) = setup("", r"\newrefsection[chosen]\cite{collision}");
        let source = ws.documents[&main]
            .text
            .source
            .replace(r"\addsectionbib", registration)
            .replace(r"\newrefsection", section);
        ws.open(main.clone(), source, 6);
        assert!(related(&ws, &main, "default.bib").is_empty());
        assert!(related(&ws, &main, "section.bib").is_empty());
    }
}

#[test]
#[ignore = "requires pdflatex, biber and biblatex"]
fn real_biber_accepts_separate_keys_and_warns_only_when_resources_coexist() {
    let (dir, mut ws, main) = setup(
        "",
        r"\cite{collision}\nocite{*}\newrefsection[chosen]\cite{collision}\nocite{*}\printbibliography",
    );
    let original = ws.documents[&main].text.source.clone();
    for (stage, simultaneous) in [false, true, false].into_iter().enumerate() {
        let source = if simultaneous {
            original.replace("addsectionbib", "addbibresource")
        } else {
            original.clone()
        };
        std::fs::write(dir.path().join("main.tex"), &source).unwrap();
        ws.open(main.clone(), source, 6 + stage as i32);
        let latex = || {
            let result = std::process::Command::new("pdflatex")
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
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
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
        let log = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
        assert_eq!(log.contains("Duplicate entry key"), simultaneous, "{log}");
        for file in ["default.bib", "section.bib"] {
            assert_eq!(
                !conflicts(&ws, &main.join(file).unwrap()).is_empty(),
                simultaneous,
                "{file}: {log}"
            );
        }
        latex();
        latex();
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        if !simultaneous {
            assert!(!log.contains("undefined"), "stage {stage}: {log}");
            let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
            assert!(bbl.contains("Default") && bbl.contains("Selected"), "{bbl}");
        }
    }
}
