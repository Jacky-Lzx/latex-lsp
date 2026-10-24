use crate::{auxiliary, workspace::Workspace};
use tower_lsp::lsp_types::*;
const A: &str = "none/global/7FC56270E7A70FA81A5935B72EACBE29/global/global/global";
const B: &str = "none/global/9D5ED678FE57BCCA610140957AFAB571/global/global/global";
const GLOBAL: &str = "none/global//global/global/global";
const PROLOGUE: &str = r"% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
\begingroup\makeatletter\@ifundefined{ver@biblatex.sty}{\@latex@error{Missing 'biblatex' package}{The bibliography requires the 'biblatex' package.}\aftergroup\endinput}{}\endgroup
";
fn list(context: &str, prefix: &str, keys: &[&str]) -> String {
    format!(
        r"\datalist[entry]{{{context}}}{}\enddatalist",
        keys.iter()
            .map(|key| format!(
                r"\entry{{{key}}}{{book}}{{}}{{}}{}\field{{title}}{{Data}}\endentry",
                if prefix.is_empty() {
                    String::new()
                } else {
                    format!(r"\field{{labelprefix}}{{{prefix}}}")
                }
            ))
            .collect::<String>()
    )
}
fn compiled(lists: String) -> String {
    format!(r"{PROLOGUE}\refsection{{0}}{lists}\endrefsection\endinput")
}
fn number(key: &str, context: &str, value: u32) -> String {
    format!(r"\abx@aux@number{{99}}{{{key}}}{{0}}{{{context}}}{{{value}}}")
}
fn default(key: &str, context: &str) -> String {
    format!(r"\abx@aux@defaultrefcontext{{0}}{{{key}}}{{{context}}}")
}
fn setup(body: &str, bbl: &str, aux: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source=format!("% 😀\r\n\\documentclass{{article}}\\usepackage[style=numeric,sorting=none,defernumbers=true]{{biblatex}}\\addbibresource{{data.bib}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    std::fs::write(
        dir.path().join("data.bib"),
        "@book{a,title={Alpha}} @book{z,title={Zulu}}",
    )
    .unwrap();
    std::fs::write(dir.path().join("main.bbl"), bbl).unwrap();
    std::fs::write(dir.path().join("main.aux"), aux).unwrap();
    let uri = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.open(uri.clone(), source, 5);
    (dir, ws, uri)
}
fn hints(ws: &Workspace, uri: &Url) -> Vec<String> {
    auxiliary::hints(
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
fn prefixed_deferred_labels_join_actual_default_context_and_bbl_fields() {
    let bbl = compiled(list(A, "A", &["z", "a"]) + &list(GLOBAL, "", &["z", "a"]));
    let aux = number("z", A, 1) + &number("a", A, 2) + &default("z", A) + &default("a", A);
    let (_dir, ws, uri) = setup(
        r"\begin{refcontext}[labelprefix=A]\cite{z,a}\printbibliography\end{refcontext}",
        &bbl,
        &aux,
    );
    assert_eq!(hints(&ws, &uri), ["→ A1", "→ A2"]);
}
#[test]
fn two_printing_contexts_use_the_compiled_per_key_default_for_both_citations() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]) + &list(GLOBAL, "", &["a"]));
    let aux = number("a", A, 1) + &number("a", B, 1) + &default("a", B);
    let (_dir, ws, uri) = setup(
        r"\begin{refcontext}[labelprefix=A]\cite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\cite{a}\printbibliography\end{refcontext}",
        &bbl,
        &aux,
    );
    // Native biblatex getrefcontext chooses the recorded default, even inside A.
    assert_eq!(hints(&ws, &uri), ["→ B1", "→ B1"]);
}
#[test]
fn citations_outside_the_printing_context_keep_the_real_prefix() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(GLOBAL, "", &["a"]));
    let aux = number("a", A, 1) + &default("a", A);
    let (_dir, ws, uri) = setup(
        r"\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\cite{a}",
        &bbl,
        &aux,
    );
    assert_eq!(hints(&ws, &uri), ["→ A1", "→ A1"]);
}

fn one() -> (String, String) {
    (
        compiled(list(A, "A", &["a"]) + &list(GLOBAL, "", &["a"])),
        number("a", A, 1) + &default("a", A),
    )
}
#[test]
fn assignments_dynamic_options_redefinitions_and_custom_formats_stay_guarded() {
    let (bbl, aux) = one();
    for extra in [
        r"\assignrefcontextkeys[labelprefix=A]{a}",
        r"\DeclareRefcontext{named}{labelprefix=A}",
        r"\localrefcontext[labelprefix=A]",
        r"\renewcommand\refcontext[1]{}",
        r"\renewenvironment{refcontext}{}{}",
        r"\newcommand\futurecontext{\newrefcontext[labelprefix=A]}",
        r"\DeclareFieldFormat{labelprefix}{#1}",
    ] {
        let (_dir, ws, uri) = setup(
            &format!(
                r"{extra}\begin{{refcontext}}[labelprefix=A]\cite{{a}}\printbibliography\end{{refcontext}}"
            ),
            &bbl,
            &aux,
        );
        assert!(hints(&ws, &uri).is_empty(), "{extra}");
    }
    for options in [
        r"labelprefix=\unknown",
        r"labelprefix={\textbf{A}}",
        r"sorting=\unknown",
        r"unknown=A",
        r"labelprefix=A,labelprefix=B",
    ] {
        let (_dir, ws, uri) = setup(
            &format!(
                r"\begin{{refcontext}}[{options}]\cite{{a}}\printbibliography\end{{refcontext}}"
            ),
            &bbl,
            &aux,
        );
        assert!(hints(&ws, &uri).is_empty(), "{options}");
    }
}
#[test]
fn incomplete_or_unsafe_bbl_prefix_proofs_never_fall_back_to_ranks_or_bibcite() {
    let (bbl, aux) = one();
    for invalid in [
        bbl.replace(r"\endinput", ""),
        bbl.clone() + r"\unknown",
        bbl.replace(
            r"\field{labelprefix}{A}",
            r"\field{labelprefix}{A}\field{labelprefix}{A}",
        ),
        bbl.replace(r"\field{labelprefix}{A}", ""),
        bbl.replace(
            r"\field{labelprefix}{A}",
            r"\field{labelprefix}{\textbf{A}}",
        ),
        bbl.replace(r"\field{labelprefix}{A}", r"\field{labelprefix}{A B}"),
        bbl.replace(
            r"\field{labelprefix}{A}",
            r"\field{title}{\field{labelprefix}{A}}",
        ),
        bbl.replace(A, B),
        bbl.replace("{book}{}{}", "{book}{skiplabel=true}{}"),
        bbl.replace(r"\endentry", r"\field{shorthand}{SHORT}\endentry"),
    ] {
        let (_dir, ws, uri) = setup(
            r"\begin{refcontext}[labelprefix=A]\cite{a}\printbibliography\end{refcontext}",
            &invalid,
            &(aux.clone() + r"\bibcite{a}{99}"),
        );
        assert!(hints(&ws, &uri).is_empty(), "{invalid}");
    }
    let (dir, ws, uri) = setup(
        r"\begin{refcontext}[labelprefix=A]\cite{a}\printbibliography\end{refcontext}",
        &bbl,
        &aux,
    );
    std::fs::remove_file(dir.path().join("main.bbl")).unwrap();
    assert!(hints(&ws, &uri).is_empty());
}
#[test]
fn unprefixed_auxiliary_labels_and_independent_reference_hints_keep_their_proofs() {
    let (bbl, aux) = one();
    let (_dir, mut ws, uri) = setup(
        r"\begin{refcontext}[labelprefix=A]\cite{a,z}\printbibliography\end{refcontext}",
        "invalid",
        &(aux + &number("z", GLOBAL, 8) + &default("z", GLOBAL)),
    );
    assert_eq!(hints(&ws, &uri), ["→ 8"]);
    let source = ws.documents[&uri]
        .text
        .source
        .replace("defernumbers=true", "defernumbers=false");
    ws.open(uri.clone(), source, 6);
    assert!(hints(&ws, &uri).is_empty());
    let (_dir, ws, uri) = setup(
        r"\begin{refcontext}[labelprefix=\unknown]\cite{a}\printbibliography\end{refcontext}\label{target}\ref{target}",
        &bbl,
        &(number("a", A, 1) + &default("a", A) + r"\newlabel{target}{{7}{9}}"),
    );
    assert_eq!(hints(&ws, &uri), ["→ 7, p. 9"]);
}
#[test]
fn prefix_and_list_budgets_reject_incomplete_results_at_the_existing_limits() {
    let (_bbl, aux) = one();
    for size in [255, 256] {
        let prefix = "X".repeat(size);
        let bbl = compiled(list(A, &prefix, &["a"]));
        let (_dir, ws, uri) = setup(
            r"\begin{refcontext}[labelprefix=A]\cite{a}\printbibliography\end{refcontext}",
            &bbl,
            &aux,
        );
        if size == 255 {
            assert_eq!(hints(&ws, &uri), [format!("→ {prefix}1")]);
        } else {
            assert!(hints(&ws, &uri).is_empty());
        }
    }
    for count in [4096, 4097] {
        let mut lists = list(A, "A", &["a"]);
        for index in 1..count {
            lists.push_str(&list(
                &format!("none/global/c{index}/global/global/global"),
                "",
                &[],
            ));
        }
        let bbl = compiled(lists);
        let (_dir, ws, uri) = setup(
            r"\begin{refcontext}[labelprefix=A]\cite{a}\printbibliography\end{refcontext}",
            &bbl,
            &aux,
        );
        if count == 4096 {
            assert_eq!(hints(&ws, &uri), ["→ A1"]);
        } else {
            assert!(hints(&ws, &uri).is_empty());
        }
    }
    let (bbl, aux) = one();
    let (dir, ws, uri) = setup(
        r"\begin{refcontext}[labelprefix=A]\cite{a}\printbibliography\end{refcontext}",
        &bbl,
        &aux,
    );
    std::fs::write(dir.path().join("main.bbl"), " ".repeat(2 * 1024 * 1024 + 1)).unwrap();
    assert!(hints(&ws, &uri).is_empty());
}
#[test]
fn shared_physical_citations_require_matching_complete_prefixed_labels() {
    let (bbl, aux) = one();
    let (dir, mut ws, uri) = setup(
        r"\input{chapter}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}",
        &bbl,
        &aux,
    );
    std::fs::write(dir.path().join("chapter.tex"), "% 😀\r\n\\cite{a}").unwrap();
    let source = ws.documents[&uri].text.source.clone();
    ws.open(uri.clone(), source.clone(), 6);
    let child = uri.join("chapter.tex").unwrap();
    assert_eq!(hints(&ws, &child), ["→ A1"]);
    let other = uri.join("other.tex").unwrap();
    std::fs::write(dir.path().join("other.bbl"), compiled(list(B, "B", &["a"]))).unwrap();
    std::fs::write(
        dir.path().join("other.aux"),
        number("a", B, 1) + &default("a", B),
    )
    .unwrap();
    ws.open(
        other.clone(),
        source.replace("labelprefix=A", "labelprefix=B"),
        7,
    );
    assert!(hints(&ws, &child).is_empty());
    ws.documents.remove(&other);
    assert_eq!(hints(&ws, &child), ["→ A1"]);
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_context_hints_match_actual_default_fields_and_plain_bbl() {
    let cases = [
        (
            r"\newrefcontext[labelprefix=A]\cite{a}\printbibliography",
            vec!["A1"],
        ),
        (
            r"\refcontext[labelprefix=A]\cite{a}\printbibliography\endrefcontext",
            vec!["A1"],
        ),
        (
            r"\begin{refcontext}[labelprefix=A]\cite{z,a}\printbibliography\end{refcontext}",
            vec!["A1", "A2"],
        ),
        (
            r"\begin{refcontext}[labelprefix=A]\cite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\cite{a}\printbibliography\end{refcontext}",
            vec!["B1", "B1"],
        ),
        (
            r"\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\cite{a}",
            vec!["A1", "A1"],
        ),
        (
            r"\begin{refcontext}[sorting=none]\cite{z,a}\printbibliography\end{refcontext}\begin{refcontext}[sorting=nty]\cite{a,z}\printbibliography\end{refcontext}",
            vec!["4", "3", "3", "4"],
        ),
    ];
    for (body, expected) in cases {
        let (dir, mut ws, uri) = setup(body, "", "");
        let source = ws.documents[&uri].text.source.clone();
        let compile = |source: &str| {
            for extension in ["aux", "bcf", "bbl", "run.xml"] {
                let path = dir.path().join(format!("main.{extension}"));
                if path.exists() {
                    std::fs::remove_file(path).unwrap();
                }
            }
            std::fs::write(dir.path().join("main.tex"), source).unwrap();
            for program in ["pdflatex", "biber", "pdflatex", "pdflatex", "pdflatex"] {
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
                    .current_dir(dir.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{program}: {}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
            let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
            assert!(
                !log.contains("undefined") && !log.contains("Warning"),
                "{log}"
            );
            assert!(!blg.contains("WARN") && !blg.contains("ERROR"), "{blg}");
            (
                std::fs::read_to_string(dir.path().join("main.bbl")).unwrap(),
                log,
            )
        };
        let (plain, _) = compile(&source);
        let expected_hints: Vec<_> = expected.iter().map(|label| format!("→ {label}")).collect();
        assert_eq!(hints(&ws, &uri), expected_hints, "{body}");
        let instrumented = source.replace(r"\begin{document}", r"\AtEveryCitekey{\typeout{LSP-LABEL:\thefield{labelprefix}\thefield{labelnumber}}}\begin{document}");
        let (observed, log) = compile(&instrumented);
        assert_eq!(observed, plain);
        let actual: Vec<_> = log
            .lines()
            .filter_map(|line| line.strip_prefix("LSP-LABEL:"))
            .collect();
        assert_eq!(actual, expected, "{body}");
        ws.open(uri.clone(), instrumented, 7);
        assert!(hints(&ws, &uri).is_empty());
    }
}

#[test]
fn direct_literal_context_forms_preserve_the_recorded_unprefixed_default() {
    for body in [
        r"\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\cite{a}",
        r"\newrefcontext[sorting=none]\cite{a}",
        r"\refcontext[labelprefix={A},sorting={none}]\cite{a}\endrefcontext",
        r"\begin{refcontext}\cite{a}\end{refcontext}",
    ] {
        let (_dir, ws, uri) = setup(body, "", &(number("a", GLOBAL, 2) + &default("a", GLOBAL)));
        let (_, projects) = crate::bib_section_keys::section_projects(&ws, &uri).unwrap();
        for project in &projects {
            assert!(
                crate::bib_section_keys::source_trace(
                    &ws,
                    &project.root,
                    &project.docs,
                    &project.signatures
                )
                .is_some(),
                "trace: {body}"
            );
        }
        assert!(
            !crate::reference_diagnostics::section_hint_occurrences(&ws, &uri).is_empty(),
            "printing: {body}"
        );
        let count = body.matches(r"\cite{a}").count();
        assert_eq!(hints(&ws, &uri), vec!["→ 2"; count], "{body}");
    }
    let (bbl, aux) = one();
    for body in [
        r"\csuse{newrefcontext}[labelprefix=A]\cite{a}",
        r"\csname refcontext\endcsname[labelprefix=A]\cite{a}\endrefcontext",
        r"\newcommand\future{\newrefcontext[labelprefix=A]}\cite{a}",
        r"\refcontext[labelprefix=A]{named}\cite{a}\endrefcontext",
        r"\let\savedcontext\refcontext\cite{a}",
    ] {
        let (_dir, ws, uri) = setup(body, &bbl, &aux);
        assert!(hints(&ws, &uri).is_empty(), "{body}");
    }
}
