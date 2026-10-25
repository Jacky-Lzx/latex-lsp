use crate::{compilation::auxiliary, project::workspace::Workspace};
use tower_lsp::lsp_types::*;

const CONTEXT: &str = "none/global//global/global/global";
fn record(section: u32, key: &str, number: u32) -> String {
    format!(
        r"\abx@aux@number{{99}}{{{key}}}{{{section}}}{{{CONTEXT}}}{{{number}}}\abx@aux@defaultrefcontext{{{section}}}{{{key}}}{{{CONTEXT}}}"
    )
}
fn records() -> String {
    record(0, "z", 1) + &record(0, "a", 2) + &record(1, "a", 1) + r"\bibcite{a}{99}"
}
fn setup(body: &str, aux: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!("% 😀\r\n\\documentclass{{article}}\\usepackage[style=numeric,sorting=none,defernumbers=true]{{biblatex}}\\addbibresource{{data.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(dir.path().join("data.bib"),"@book{a,title={Alpha},author={Doe, Ann},year={2026}} @book{z,title={Zulu},author={Doe, Zoe},year={2025}}").unwrap();
    std::fs::write(dir.path().join("main.aux"), aux).unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), source, 5);
    (dir, ws, uri)
}
fn all(ws: &Workspace, uri: &Url) -> Vec<InlayHint> {
    auxiliary::hints(
        ws,
        uri,
        Range::new(Position::new(0, 0), Position::new(u32::MAX, 0)),
    )
}
fn labels(hints: Vec<InlayHint>) -> Vec<String> {
    hints
        .into_iter()
        .map(|hint| match hint.label {
            InlayHintLabel::String(value) => value,
            _ => panic!(),
        })
        .collect()
}
#[test]
fn numeric_sections_use_the_third_auxiliary_field_and_exact_printing_positions() {
    let (_dir, ws, uri) = setup(r"\cite{z,a}\newrefsection\cite{a}\nocite{z}", &records());
    let hints = all(&ws, &uri);
    assert_eq!(labels(hints.clone()), ["→ 1", "→ 2", "→ 1"]);
    let doc = &ws.documents[&uri];
    let start = doc.text.source.find(r"\cite{z,a}").unwrap() + 6;
    assert_eq!(hints[0].position, doc.text.position(start + 1));
    assert_eq!(hints[1].position, doc.text.position(start + 3));
    assert!(
        matches!(&hints[0].tooltip,Some(InlayHintTooltip::String(value)) if value.contains("may be stale"))
    );
}
#[test]
fn numeric_records_cannot_guess_author_year_prefixes_or_custom_formats() {
    for edit in [0, 1, 2, 3] {
        let (dir, mut ws, uri) = setup(r"\cite{z,a}\newrefsection\cite{a}", &records());
        let source = &ws.documents[&uri].text.source;
        let changed = match edit {
            0 => source.replace("style=numeric", "style=authoryear"),
            1 => source.replace(
                r"\begin{document}",
                r"\DeclareFieldFormat{labelnumber}{X#1}\begin{document}",
            ),
            2 => source.replace("style=numeric,", ""),
            _ => source.clone(),
        };
        if edit == 3 {
            std::fs::write(
                dir.path().join("main.aux"),
                records().replace(CONTEXT, "none/global/HASH/global/global/global"),
            )
            .unwrap();
        }
        ws.open(uri.clone(), changed, 6);
        assert!(all(&ws, &uri).is_empty(), "edit={edit}");
    }
}
#[test]
fn conflicting_and_unexpanded_auxiliary_records_do_not_retain_older_numbers() {
    for suffix in [
        record(0, "a", 8),
        r"\@input{\dynamic.aux}".into(),
        r"\@input".into(),
        format!(r"\abx@aux@defaultrefcontext{{0}}{{a}}{{{CONTEXT}/other}}"),
        r"\abx@aux@defaultrefcontext{0}{a}{\unknown}".into(),
        format!(r"\abx@aux@number{{100}}{{a}}{{0}}{{{CONTEXT}}}{{\unknown}}"),
    ] {
        let (_dir, ws, uri) = setup(r"\cite{a}", &(records() + &suffix));
        assert!(all(&ws, &uri).is_empty(), "{suffix}");
    }
}
#[test]
fn unproved_refcontext_assignments_and_citation_data_mutators_cannot_use_default_numbers() {
    for body in [
        r"\localrefcontext[sorting=none]\cite{a}",
        r"\assignrefcontextkeys[labelprefix=A]{a}\cite{a}",
        r"\AtEveryCitekey{\clearfield{labelnumber}}\cite{a}",
    ] {
        let (_dir, ws, uri) = setup(body, &records());
        assert!(all(&ws, &uri).is_empty(), "{body}");
    }
    let (_dir, ws, uri) = setup(r"\verb|\newrefcontext[sorting=none]|\cite{a}", &records());
    assert_eq!(labels(all(&ws, &uri)), ["→ 2"]);
}
#[test]
fn repeated_inputs_and_shared_roots_require_every_compiled_number_to_agree() {
    let (dir, mut ws, uri) = setup(r"\input{shared}\newrefsection\input{shared}", &records());
    std::fs::write(dir.path().join("shared.tex"), r"\cite{a}").unwrap();
    ws.refresh_project(&uri);
    let shared = uri.join("shared.tex").unwrap();
    assert!(ws.dependencies(&uri).contains(&shared));
    assert!(all(&ws, &shared).is_empty());
    std::fs::write(
        dir.path().join("main.aux"),
        record(0, "a", 2) + &record(1, "a", 2) + "\n",
    )
    .unwrap();
    assert_eq!(labels(all(&ws, &shared)), ["→ 2"]);
    let other = uri.join("other.tex").unwrap();
    let source = ws.documents[&uri]
        .text
        .source
        .replace(r"\newrefsection\input{shared}", "");
    std::fs::write(dir.path().join("other.aux"), record(0, "a", 7)).unwrap();
    ws.open(other.clone(), source.clone(), 7);
    assert!(all(&ws, &shared).is_empty());
    std::fs::write(dir.path().join("other.aux"), record(0, "a", 2) + "\n").unwrap();
    assert_eq!(labels(all(&ws, &shared)), ["→ 2"]);
    ws.open(
        other,
        source.replace("style=numeric", "style=authoryear"),
        8,
    );
    assert!(all(&ws, &shared).is_empty());
}
#[test]
fn ranges_settings_and_the_combined_response_budget_remain_independent() {
    let (_dir, mut ws, uri) = setup(
        r"\cite{z,a}\newrefsection\cite{a}\label{target}\ref{target}",
        &(records() + r"\newlabel{target}{{4}{9}}"),
    );
    assert_eq!(labels(all(&ws, &uri)), ["→ 1", "→ 2", "→ 1", "→ 4, p. 9"]);
    ws.config.inlay_hints.citations = false;
    assert_eq!(labels(all(&ws, &uri)), ["→ 4, p. 9"]);
    ws.config.inlay_hints.citations = true;
    ws.config.inlay_hints.numbers = false;
    ws.config.inlay_hints.pages = false;
    assert_eq!(labels(all(&ws, &uri)), ["→ 1", "→ 2", "→ 1"]);
    let source = ws.documents[&uri].text.source.replace(
        r"\cite{z,a}\newrefsection\cite{a}",
        &r"\cite{a}".repeat(1100),
    );
    ws.open(uri.clone(), source.clone(), 6);
    assert_eq!(all(&ws, &uri).len(), 1024);
    let doc = &ws.documents[&uri];
    let at = source.rfind(r"\cite{a}").unwrap();
    let hints = auxiliary::hints(
        &ws,
        &uri,
        Range::new(doc.text.position(at), Position::new(u32::MAX, 0)),
    );
    assert_eq!(labels(hints), ["→ 2"]);
    ws.config.inlay_hints.enabled = false;
    assert!(all(&ws, &uri).is_empty());
}
#[test]
fn output_jobs_recursive_auxiliaries_and_incomplete_reads_keep_label_namespaces_independent() {
    let (dir, mut ws, uri) = setup(
        r"\cite{z,a}\newrefsection\cite{a}\label{target}\ref{target}",
        &records(),
    );
    std::fs::create_dir(dir.path().join("build output")).unwrap();
    ws.config.build.output_directory = Some("build output".into());
    ws.config.build.job_name = Some("custom".into());
    let root = dir.path().join("build output/custom.aux");
    let child = dir.path().join("build output/child.aux");
    std::fs::write(
        &root,
        record(0, "z", 1) + r"\@input{child.aux}\newlabel{target}{{4}{9}}",
    )
    .unwrap();
    std::fs::write(&child, records() + r"\@input{custom.aux}").unwrap();
    assert_eq!(labels(all(&ws, &uri)), ["→ 1", "→ 2", "→ 1", "→ 4, p. 9"]);
    std::fs::remove_file(&child).unwrap();
    assert_eq!(labels(all(&ws, &uri)), ["→ 4, p. 9"]);
    let mut source = record(0, "z", 1) + r"\newlabel{target}{{4}{9}}";
    for index in 0..16 {
        source.push_str(&format!(r"\@input{{child{index}.aux}}"));
        std::fs::write(
            dir.path().join(format!("build output/child{index}.aux")),
            r"\relax",
        )
        .unwrap();
    }
    std::fs::write(root, source).unwrap();
    assert_eq!(labels(all(&ws, &uri)), ["→ 4, p. 9"]);
}
#[test]
fn retained_key_conflicts_are_still_observed_after_numeric_metadata_budgets_fill() {
    let mut auxiliary = record(0, "a", 2);
    for index in 0..4095 {
        auxiliary.push_str(&record(0, &format!("filler{index}"), 1));
    }
    let (dir, ws, uri) = setup(r"\cite{a}", &auxiliary);
    assert_eq!(labels(all(&ws, &uri)), ["→ 2"]);
    auxiliary.push_str(&record(0, "overflow", 1));
    auxiliary.push_str(&record(0, "a", 8));
    std::fs::write(dir.path().join("main.aux"), auxiliary).unwrap();
    assert!(all(&ws, &uri).is_empty());
}
#[test]
fn static_named_printing_citations_keep_literal_examples_opaque() {
    let (_dir, mut ws, uri) = setup(
        r"\csuse{cite}{z}\csname cite\endcsname{a}\nocite{z}\verb|\cite{z}|",
        &records(),
    );
    let source = ws.documents[&uri].text.source.clone();
    assert_eq!(labels(all(&ws, &uri)), ["→ 1", "→ 2"]);
    ws.open(
        uri.clone(),
        source.replace(
            r"\begin{document}",
            r"\renewcommand\cite[1]{}\begin{document}",
        ),
        7,
    );
    assert!(all(&ws, &uri).is_empty());
}
#[test]
fn citations_in_unproved_macro_arguments_cannot_certify_execution_sections() {
    let (_dir, mut ws, uri) = setup(r"\cite{z}\take{\cite{a}}", &records());
    let source = ws.documents[&uri].text.source.replace(
        r"\begin{document}",
        r"\newcommand\take[1]{}\begin{document}",
    );
    ws.open(uri.clone(), source, 6);
    // A signature proves token consumption, not the execution of arbitrary
    // macro bodies. The section trace deliberately refuses this project.
    assert_eq!(
        crate::diagnostics::reference_diagnostics::section_hint_occurrences(&ws, &uri).len(),
        1
    );
    assert!(all(&ws, &uri).is_empty());
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_numeric_labels_match_actual_sections_and_reject_author_year_or_reformatted_values() {
    for scenario in 0..4 {
        let (dir, mut ws, uri) = setup(
            r"\cite{z,a}\printbibliography\newrefsection\cite{a}\printbibliography",
            "",
        );
        let mut source = ws.documents[&uri].text.source.clone();
        if scenario == 1 {
            source = source.replace("style=numeric", "style=authoryear");
        }
        if scenario == 2 {
            source = source.replace(
                r"\begin{document}",
                r"\DeclareFieldFormat{labelnumber}{X#1}\begin{document}",
            );
        }
        if scenario == 3 {
            let path = dir.path().join("data.bib");
            let bib = std::fs::read_to_string(&path)
                .unwrap()
                .replace("title={Alpha}", "shorthand={ANN},title={Alpha}");
            std::fs::write(path, bib).unwrap();
        }
        std::fs::write(dir.path().join("main.tex"), &source).unwrap();
        ws.open(uri.clone(), source, 6);
        for (program, args) in [
            (
                "pdflatex",
                vec![
                    "-no-shell-escape",
                    "-interaction=nonstopmode",
                    "-halt-on-error",
                    "main.tex",
                ],
            ),
            ("biber", vec!["main"]),
            (
                "pdflatex",
                vec![
                    "-no-shell-escape",
                    "-interaction=nonstopmode",
                    "-halt-on-error",
                    "main.tex",
                ],
            ),
            (
                "pdflatex",
                vec![
                    "-no-shell-escape",
                    "-interaction=nonstopmode",
                    "-halt-on-error",
                    "main.tex",
                ],
            ),
            (
                "pdflatex",
                vec![
                    "-no-shell-escape",
                    "-interaction=nonstopmode",
                    "-halt-on-error",
                    "main.tex",
                ],
            ),
        ] {
            let output = std::process::Command::new(program)
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{program}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        assert!(!log.contains("undefined"), "{log}");
        let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
        assert!(!blg.contains("WARN") && !blg.contains("ERROR"), "{blg}");
        let expected = match scenario {
            0 => vec!["→ 1", "→ 2", "→ 1"],
            3 => vec!["→ 1"],
            _ => vec![],
        };
        assert_eq!(labels(all(&ws, &uri)), expected, "scenario={scenario}");
    }
}
