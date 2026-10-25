use super::*;
use crate::project::workspace::Workspace;
use tower_lsp::lsp_types::*;
const PROLOGUE: &str = r"% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
\begingroup
\makeatletter
\@ifundefined{ver@biblatex.sty}
  {\@latex@error{Missing 'biblatex' package}{The bibliography requires the 'biblatex' package.}\aftergroup\endinput}
  {}
\endgroup
";
const CONTEXT: &str = "none/global//global/global/global";
fn entry(key: &str) -> String {
    format!(r"\entry{{{key}}}{{book}}{{}}{{}}\field{{title}}{{Data}}\endentry")
}
fn list(section: u32, keys: &[&str]) -> String {
    format!(
        r"\refsection{{{section}}}\datalist[entry]{{{CONTEXT}}}{}\enddatalist\endrefsection",
        keys.iter().map(|key| entry(key)).collect::<String>()
    )
}
fn bbl() -> String {
    PROLOGUE.to_string() + &list(0, &["z", "a"]) + &list(1, &["a"]) + r"\endinput"
}
fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source=format!("% 😀\r\n\\documentclass{{article}}\\usepackage[style=numeric,sorting=none]{{biblatex}}\\addbibresource{{data.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(dir.path().join("data.bib"),"@book{a,title={Alpha},author={Doe, Ann},year={2026}} @book{z,title={Zulu},author={Doe, Zoe},year={2025}}").unwrap();
    std::fs::write(dir.path().join("main.aux"), r"\bibcite{a}{99}").unwrap();
    std::fs::write(dir.path().join("main.bbl"), bbl()).unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), source, 5);
    (dir, ws, uri)
}
fn hints(ws: &Workspace, uri: &Url) -> Vec<String> {
    crate::compilation::auxiliary::hints(
        ws,
        uri,
        Range::new(Position::new(0, 0), Position::new(u32::MAX, 0)),
    )
    .into_iter()
    .map(|hint| match hint.label {
        InlayHintLabel::String(label) => label,
        _ => panic!(),
    })
    .collect()
}
#[test]
fn loaded_bbl_lists_supply_numbers_without_auxiliary_number_records() {
    let (_dir, mut ws, uri) = setup(r"\cite{z,a}\newrefsection\cite{a}\nocite{z}");
    assert_eq!(hints(&ws, &uri), ["→ 1", "→ 2", "→ 1"]);
    let source = ws.documents[&uri]
        .text
        .source
        .replace("style=numeric", "style=numeric,defernumbers=false");
    ws.open(uri.clone(), source, 6);
    assert_eq!(hints(&ws, &uri), ["→ 1", "→ 2", "→ 1"]);
    let source = ws.documents[&uri]
        .text
        .source
        .replace("defernumbers=false", "defernumbers=true");
    ws.open(uri.clone(), source, 7);
    assert!(hints(&ws, &uri).is_empty());
}
#[test]
fn nested_field_data_cannot_invent_entries_and_incomplete_records_cannot_return_partial_labels() {
    let source = bbl().replace(
        r"\field{title}{Data}",
        r"\field{title}{Literal \entry{fake}{book}{}{} \endentry}",
    );
    let labels = parse(&source).unwrap();
    assert_eq!(labels.len(), 3);
    assert_eq!(labels[&(0, "a".into())], Some("2".into()));
    assert_eq!(parse(&source.replace('\n', "\r\n")), Some(labels));
    for source in [
        bbl().replace(r"\endinput", ""),
        bbl() + r"\newrefsection",
        bbl().replace(r"\endentry", r"\unknown\endentry"),
        bbl().replace("ver@biblatex.sty", "wrong"),
        bbl().replace("version 3.3", "version 3.2"),
        bbl().replace(r"\endrefsection", r"\endrefsection\refsection{0}"),
    ] {
        assert!(parse(&source).is_none(), "{source}");
    }
}
#[test]
fn unsupported_label_semantics_cannot_be_approximated_by_bbl_order() {
    let base = bbl();
    for changed in [
        base.replace(r"\entry{a}{book}{}{}", r"\entry{a}{set}{}{}"),
        base.replace(r"\entry{a}{book}{}{}", r"\entry{a}{book}{skiplab=true}{}"),
        base.replace(r"\field{title}{Data}", r"\field{shorthand}{ANN}"),
        base.replace(r"\field{title}{Data}", r"\field{localnumber}{}"),
        base.replace(r"\field{title}{Data}", r"\field{labelprefix}{A}"),
        base.replace(r"\field{title}{Data}", r"\verb{url}\verb value\endverb"),
        base.replace(r"\enddatalist", r"\enddatalist\keyalias{old}{z}"),
        base.replace(CONTEXT, "none/global/HASH/global/global/global"),
        base.replace(
            r"\enddatalist\endrefsection",
            &format!(r"\enddatalist\datalist[entry]{{{CONTEXT}}}\enddatalist\endrefsection"),
        ),
        base.replace(r"\entry{a}{book}{}{}", r"\entry{z}{book}{}{}"),
    ] {
        assert!(parse(&changed).is_none(), "{changed}");
    }
    let many = PROLOGUE.to_string()
        + &list(
            0,
            &(0..4097)
                .map(|i| format!("key{i}"))
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
        + r"\endinput";
    assert!(parse(&many).is_none());
}
#[test]
fn output_artifacts_mode_switches_and_shared_occurrences_keep_their_proofs() {
    let (dir, mut ws, uri) = setup(r"\input{shared}\newrefsection\input{shared}");
    std::fs::write(dir.path().join("shared.tex"), r"\cite{a}").unwrap();
    ws.refresh_project(&uri);
    let shared = uri.join("shared.tex").unwrap();
    assert!(hints(&ws, &shared).is_empty());
    let consistent =
        PROLOGUE.to_string() + &list(0, &["z", "a"]) + &list(1, &["z", "a"]) + r"\endinput";
    std::fs::write(dir.path().join("main.bbl"), &consistent).unwrap();
    assert_eq!(hints(&ws, &shared), ["→ 2"]);
    std::fs::create_dir(dir.path().join("build output")).unwrap();
    ws.config.build.output_directory = Some("build output".into());
    ws.config.build.job_name = Some("custom".into());
    let path = dir.path().join("build output/custom.bbl");
    std::fs::write(
        &path,
        PROLOGUE.to_string() + &list(0, &["a", "z"]) + &list(1, &["a"]) + r"\endinput",
    )
    .unwrap();
    assert_eq!(hints(&ws, &shared), ["→ 1"]);
    std::fs::write(path, consistent + r"\unknown").unwrap();
    assert!(hints(&ws, &shared).is_empty());
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_loaded_bbl_numbers_match_actual_fields_and_reject_sets_shorthand_and_options() {
    for scenario in 0..6 {
        let (dir, mut ws, uri) =
            setup(r"\cite{z,a}\printbibliography\newrefsection\cite{a}\printbibliography");
        let mut source = ws.documents[&uri].text.source.clone();
        if scenario == 1 {
            source = source.replace("sorting=none", "sorting=nty");
        }
        if scenario == 2 {
            source = source.replace(r"\printbibliography", "");
        }
        let mut bib = std::fs::read_to_string(dir.path().join("data.bib")).unwrap();
        if scenario == 3 {
            bib = bib.replace("title={Alpha}", "shorthand={ANN},title={Alpha}");
        }
        if scenario == 4 {
            bib = bib.replace("title={Alpha}", "options={skiplab=true},title={Alpha}");
        }
        if scenario == 5 {
            bib.push_str(" @set{pair,entryset={z,a}}");
            source = source.replace(r"\cite{z,a}", r"\cite{pair,a}");
        }
        std::fs::write(dir.path().join("data.bib"), bib).unwrap();
        std::fs::write(dir.path().join("main.tex"), &source).unwrap();
        ws.open(uri.clone(), source.clone(), 6);
        let compile = |source: &str| {
            for extension in ["aux", "bcf", "bbl", "run.xml"] {
                let path = dir.path().join(format!("main.{extension}"));
                if path.exists() {
                    std::fs::remove_file(path).unwrap();
                }
            }
            std::fs::write(dir.path().join("main.tex"), source).unwrap();
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
            std::fs::read_to_string(dir.path().join("main.bbl")).unwrap()
        };
        let plain = compile(&source);
        let expected = match scenario {
            0 | 2 => vec!["→ 1", "→ 2", "→ 1"],
            1 => vec!["→ 2", "→ 1", "→ 1"],
            _ => vec![],
        };
        assert_eq!(hints(&ws, &uri), expected, "scenario={scenario}");
        if scenario <= 2 {
            let instrumented=source.replace(r"\begin{document}",r"\AtEveryCitekey{\typeout{LSP-LABEL:\therefsection:\thefield{entrykey}:\thefield{labelnumber}}}\begin{document}");
            assert_eq!(compile(&instrumented), plain);
            let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
            let expected = if scenario == 1 {
                ["LSP-LABEL:0:z:2", "LSP-LABEL:0:a:1", "LSP-LABEL:1:a:1"]
            } else {
                ["LSP-LABEL:0:z:1", "LSP-LABEL:0:a:2", "LSP-LABEL:1:a:1"]
            };
            for value in expected {
                assert!(log.contains(value), "{log}");
            }
        }
    }
}
