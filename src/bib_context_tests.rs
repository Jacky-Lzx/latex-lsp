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

#[test]
fn strong_entry_assignment_hints_use_each_requests_execution_state() {
    let bbl = compiled(
        list(A, "A", &["a", "z"]) + &list(B, "B", &["a", "z"]) + &list(GLOBAL, "", &["a", "z"]),
    );
    let aux = number("a", A, 3) + &number("a", B, 8) + &number("a", GLOBAL, 5) + &default("a", B);
    for (body, expected) in [
        (
            r"\cite{a}\assignrefcontextentries[labelprefix=A]{a} Text \cite{a}\assignrefcontextentries[labelprefix=B]{a} Text \cite{a}",
            vec!["→ B8", "→ A3", "→ B8"],
        ),
        (
            r"{\assignrefcontextentries[labelprefix=A]{a}} Text \cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=A]{*}\assignrefcontextentries[labelprefix=B]{a} Text \cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=A]{a}\assignrefcontextentries[]{a} Text \cite{a}",
            vec!["→ 5"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=A]{} Text \cite{a}",
            vec!["→ B8"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=A]{a}\begin{refcontext}[labelprefix=B]\cite{a}\end{refcontext}",
            vec!["→ A3"],
        ),
    ] {
        let (_dir, ws, uri) = setup(body, &bbl, &aux);
        assert_eq!(hints(&ws, &uri), expected, "{body}");
    }
}

#[test]
fn assigned_context_hints_never_borrow_the_defaults_number_or_a_different_prefix() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let body = r"\cite{a}\assignrefcontextentries[labelprefix=A]{a} Text \cite{a}";
    for aux in [
        number("a", B, 8) + &default("a", B),
        number("a", B, 8) + &number("a", A, 3) + &number("a", A, 4) + &default("a", B),
    ] {
        let (_dir, ws, uri) = setup(body, &bbl, &aux);
        assert_eq!(hints(&ws, &uri), ["→ B8"]);
    }
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for invalid in [
        compiled(list(A, "Wrong", &["a"]) + &list(B, "B", &["a"])),
        compiled(
            list(A, "A", &["a"])
                + &list("none/global/other/global/global/global", "A", &["a"])
                + &list(B, "B", &["a"]),
        ),
        compiled(list(&A.replace("none/", "nty/"), "A", &["a"]) + &list(B, "B", &["a"])),
    ] {
        let (_dir, ws, uri) = setup(body, &invalid, &aux);
        assert_eq!(hints(&ws, &uri), ["→ B8"]);
    }
}

#[test]
fn unproved_assignment_execution_and_context_forms_keep_numeric_hints_guarded() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for assignment in [
        r"\assignrefcontextentries[labelprefix=\unknown]{a}",
        r"\assignrefcontextentries[name=chosen]{a}",
        r"\assignrefcontextentries[labelprefix=A,labelprefix=B]{a}",
        r"\assignrefcontextentries[labelprefix=A]{\unknown}",
        r"\assignrefcontextentries{a}",
        r"\def\assign{\assignrefcontextentries}\assign[labelprefix=A]{a}",
        r"\newcommand\future{\assignrefcontextentries[labelprefix=A]{a}}",
        r"\newcommand\run[1]{#1}\run{\assignrefcontextentries[labelprefix=A]{a}}",
        r"\assignrefcontextkeyws[labelprefix=A]{\dynamic}",
    ] {
        let (_dir, ws, uri) = setup(&format!("{assignment} Text \\cite{{a}}"), &bbl, &aux);
        assert!(hints(&ws, &uri).is_empty(), "{assignment}");
    }
}

#[test]
fn shared_physical_assignment_hints_require_all_executions_to_select_the_same_label() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    let body = r"\input{chapter}\assignrefcontextentries[labelprefix=A]{a}\input{chapter}";
    let (dir, mut ws, uri) = setup(body, &bbl, &aux);
    std::fs::write(dir.path().join("chapter.tex"), "% 😀\r\n\\cite{a}").unwrap();
    ws.refresh_project(&uri);
    let child = uri.join("chapter.tex").unwrap();
    assert!(hints(&ws, &child).is_empty());
    let source = ws.documents[&uri].text.source.replace(
        body,
        r"\assignrefcontextentries[labelprefix=A]{a}\input{chapter}\input{chapter}",
    );
    ws.open(uri.clone(), source.clone(), 6);
    assert_eq!(hints(&ws, &child), ["→ A3"]);
    let other = uri.join("other.tex").unwrap();
    std::fs::write(dir.path().join("other.aux"), &aux).unwrap();
    std::fs::write(dir.path().join("other.bbl"), &bbl).unwrap();
    ws.open(
        other.clone(),
        source.replace("labelprefix=A", "labelprefix=B"),
        7,
    );
    assert!(hints(&ws, &child).is_empty());
    ws.open(other, source, 8);
    assert_eq!(hints(&ws, &child), ["→ A3"]);
}

#[test]
fn assigned_numbers_do_not_require_a_default_context_record() {
    let bbl = compiled(list(A, "A", &["a"]));
    let (_dir, ws, uri) = setup(
        r"\assignrefcontextentries[sorting=none,labelprefix=A]{a} Text \cite{a}",
        &bbl,
        &number("a", A, 3),
    );
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

#[test]
fn entry_assignment_storage_uses_the_executing_numeric_section() {
    let bbl = format!(
        r"{PROLOGUE}\refsection{{0}}{}{}\endrefsection\refsection{{1}}{}{}\endrefsection\endinput",
        list(A, "A", &["a"]),
        list(B, "B", &["a"]),
        list(A, "A", &["a"]),
        list(B, "B", &["a"])
    );
    let aux = number("a", A, 3)
        + &number("a", B, 8)
        + &default("a", B)
        + &format!(
            r"\abx@aux@number{{100}}{{a}}{{1}}{{{A}}}{{4}}\abx@aux@number{{101}}{{a}}{{1}}{{{B}}}{{9}}\abx@aux@defaultrefcontext{{1}}{{a}}{{{B}}}"
        );
    let body = r"\assignrefcontextentries[labelprefix=A]{a} Text \cite{a}\begin{refsection}\cite{a}\assignrefcontextentries[labelprefix=A]{a} Text \cite{a}\end{refsection}\cite{a}";
    for body in [body.to_string(), body.replace("entries[", "entries*[")] {
        let (_dir, ws, uri) = setup(&body, &bbl, &aux);
        assert_eq!(hints(&ws, &uri), ["→ A3", "→ B9", "→ A4", "→ A3"], "{body}");
    }
}
#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_strong_assignment_hints_match_actual_labels_and_keep_the_complete_bbl() {
    let cases = [
        (
            r"\assignrefcontextentries[labelprefix=A]{a}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1"],
        ),
        (
            r"\cite{a}\assignrefcontextentries[labelprefix=A]{a}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=A]{a}\cite{a}\assignrefcontextentries[labelprefix=B]{a}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1", "B1"],
        ),
        (
            r"{\assignrefcontextentries[labelprefix=A]{a}}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=A]{*}\assignrefcontextentries[labelprefix=B]{a}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1"],
        ),
        (
            r"\assignrefcontextentries[]{a}\cite{a}\nocite{z,a}\printbibliography\begin{refcontext}[labelprefix={}]\nocite{a}\printbibliography\end{refcontext}",
            vec!["1"],
        ),
        (
            r"\assignrefcontextentries[labelprefix={}]{a}\cite{a}\nocite{z,a}\printbibliography\begin{refcontext}[labelprefix={}]\nocite{a}\printbibliography\end{refcontext}",
            vec!["1"],
        ),
    ];
    native_assignment_cases(&cases);
}

fn native_assignment_cases(cases: &[(&str, Vec<&str>)]) {
    native_assignment_files(cases, &[]);
}
fn native_assignment_files(cases: &[(&str, Vec<&str>)], files: &[(&str, &str)]) {
    native_assignment_preamble(cases, files, "");
}
fn native_assignment_preamble(cases: &[(&str, Vec<&str>)], files: &[(&str, &str)], preamble: &str) {
    native_assignment_engine(cases, files, preamble, "pdflatex");
}
fn native_assignment_engine(
    cases: &[(&str, Vec<&str>)],
    files: &[(&str, &str)],
    preamble: &str,
    engine: &str,
) {
    for (body, expected) in cases {
        let (dir, mut ws, uri) = assignment_preamble(preamble, body, "", "");
        ws.config.build.engine = crate::config::Engine::from_program(engine);
        let cache = dir.path().join("tex-cache");
        std::fs::create_dir(&cache).unwrap();
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        ws.refresh_project(&uri);
        let source = ws.documents[&uri].text.source.clone();
        let compile = |source: &str| {
            for extension in ["aux", "bcf", "bbl", "run.xml"] {
                let path = dir.path().join(format!("main.{extension}"));
                if path.exists() {
                    std::fs::remove_file(path).unwrap();
                }
            }
            std::fs::write(dir.path().join("main.tex"), source).unwrap();
            for program in [engine, "biber", engine, engine, engine] {
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
                    .env("TEXMFVAR", &cache)
                    .env("TEXMFCACHE", &cache)
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
        assert_eq!(&actual, expected, "{body}");
        ws.open(uri.clone(), instrumented, 7);
        assert!(hints(&ws, &uri).is_empty());
    }
}

#[test]
fn explicit_empty_assignment_prefix_cannot_borrow_the_unhashed_global_context() {
    let empty = "none/global/D41D8CD98F00B204E9800998ECF8427E/global/global/global";
    let bbl = compiled(list(GLOBAL, "", &["a"]) + &list(empty, "", &["a"]) + &list(B, "B", &["a"]));
    let aux =
        number("a", GLOBAL, 5) + &number("a", empty, 6) + &number("a", B, 8) + &default("a", B);
    for (options, expected) in [("", "→ 5"), ("labelprefix={}", "→ 6")] {
        let (_dir, ws, uri) = setup(
            &format!(r"\assignrefcontextentries[{options}]{{a}} Text \cite{{a}}"),
            &bbl,
            &aux,
        );
        assert_eq!(hints(&ws, &uri), [expected]);
    }
    let (_dir, ws, uri) = setup(
        r"\assignrefcontextentries[]{a} Text \cite{a}",
        &compiled(list(empty, "", &["a"])),
        &number("a", empty, 6),
    );
    assert!(hints(&ws, &uri).is_empty());
}

#[test]
fn weak_entry_assignments_yield_to_explicit_contexts_and_strong_assignments() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for (body, expected) in [
        (
            r"\assignrefcontextentries*[labelprefix=A]{a} Text \cite{a}",
            vec!["→ A3"],
        ),
        (
            r"{\assignrefcontextentries*[labelprefix=A]{a}} Text \cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=B]{a}\assignrefcontextentries*[labelprefix=A]{a} Text \cite{a}",
            vec!["→ B8"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{*}\assignrefcontextentries*[labelprefix=B]{a} Text \cite{a}\assignrefcontextentries[labelprefix=B]{a} Text \cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}",
            vec!["→ A3", "→ B8", "→ B8"],
        ),
        (
            r"\cite{a}\assignrefcontextentries*[labelprefix=A]{a} Text \cite{a}\assignrefcontextentries*[labelprefix=B]{a} Text \cite{a}",
            vec!["→ B8", "→ A3", "→ B8"],
        ),
    ] {
        let (_dir, ws, uri) = setup(body, &bbl, &aux);
        assert_eq!(hints(&ws, &uri), expected, "{body}");
    }
}

#[test]
fn weak_assignment_context_environments_follow_input_order_and_shared_executions() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    let (dir, mut ws, uri) = setup(
        r"\assignrefcontextentries*[labelprefix=A]{a}\input{chapter}\begin{refcontext}[labelprefix=A]\input{chapter}\end{refcontext}",
        &bbl,
        &aux,
    );
    let child = uri.join("chapter.tex").unwrap();
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{a}").unwrap();
    ws.refresh_project(&uri);
    ws.open(child.clone(), r"\cite{a}".into(), 6);
    assert!(hints(&ws, &child).is_empty());
    let source = ws.documents[&uri]
        .text
        .source
        .replace(r"\input{chapter}\begin{refcontext}", r"\begin{refcontext}");
    ws.open(uri.clone(), source.clone(), 7);
    assert_eq!(hints(&ws, &child), ["→ B8"]);
    let other = uri.join("new-root.tex").unwrap();
    std::fs::write(dir.path().join("new-root.aux"), &aux).unwrap();
    std::fs::write(dir.path().join("new-root.bbl"), &bbl).unwrap();
    ws.open(
        other.clone(),
        source
            .replace(r"\begin{refcontext}[labelprefix=A]", "")
            .replace(r"\end{refcontext}", ""),
        8,
    );
    assert!(hints(&ws, &child).is_empty());
    ws.close(&other);
    assert_eq!(hints(&ws, &child), ["→ B8"]);

    let (dir, mut ws, uri) = setup(
        r"\assignrefcontextentries*[labelprefix=A]{a}\begin{refcontext}[labelprefix=A]\input{chapter}\cite{a}",
        &bbl,
        &aux,
    );
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{a}\end{refcontext}").unwrap();
    ws.refresh_project(&uri);
    ws.open(
        uri.join("chapter.tex").unwrap(),
        r"\cite{a}\end{refcontext}".into(),
        6,
    );
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

#[test]
fn weak_assignments_preserve_missing_metadata_and_unproved_scope_guards() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for body in [
        r"\assignrefcontextentries*[labelprefix=A]{a}\end{refcontext}\cite{a}",
        r"\assignrefcontextentries*[labelprefix=A]{a}\begin{refcontext}[labelprefix=A]\cite{a}",
        r"\assignrefcontextentries*[labelprefix=A]{a}\begin{refcontext}[labelprefix=A]\begin{refcontext}[labelprefix=B]\cite{a}\end{refcontext}\end{refcontext}",
    ] {
        let (_dir, ws, uri) = setup(body, &bbl, &aux);
        assert!(hints(&ws, &uri).is_empty(), "{body}");
    }
    let body = r"\assignrefcontextentries*[labelprefix=A]{a}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}";
    let (_dir, ws, uri) = setup(body, &bbl, &(number("a", B, 8) + &default("a", B)));
    assert_eq!(hints(&ws, &uri), ["→ B8"]);
    let (_dir, ws, uri) = setup(body, &bbl, &(aux + &number("a", A, 9)));
    assert_eq!(hints(&ws, &uri), ["→ B8"]);
    // Explicit contexts bypass weak selections even if that would leave no
    // label. They must never borrow the otherwise valid weak assignment.
    let (_dir, ws, uri) = setup(body, &bbl, &number("a", A, 3));
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

#[test]
#[ignore = "requires native pdfLaTeX, Biber, and biblatex"]
fn native_weak_assignment_hints_match_actual_labels_and_keep_the_complete_bbl() {
    let printing = r"\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}";
    let cases = [
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\cite{a}",
            vec!["A1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\cite{a}",
            vec!["B1", "A1"],
        ),
        (
            r"{\assignrefcontextentries*[labelprefix=A]{a}}\cite{a}",
            vec!["A1"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=B]{a}\assignrefcontextentries*[labelprefix=A]{a}\cite{a}",
            vec!["B1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{*}\assignrefcontextentries*[labelprefix=B]{a}\cite{a}\assignrefcontextentries[labelprefix=B]{a}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}",
            vec!["A1", "B1", "B1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}{\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}}\cite{a}",
            vec!["B1", "A1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\begin{quote}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\end{quote}\cite{a}",
            vec!["B1", "A1"],
        ),
    ];
    let cases: Vec<_> = cases
        .into_iter()
        .map(|(body, labels)| (format!("{body}{printing}"), labels))
        .collect();
    let mut borrowed: Vec<_> = cases
        .iter()
        .map(|(body, labels)| (body.as_str(), labels.clone()))
        .collect();
    let section = format!(
        r"\assignrefcontextentries*[labelprefix=A]{{a}}\begin{{refsection}}\cite{{a}}\assignrefcontextentries*[labelprefix=A]{{a}}\cite{{a}}{printing}\end{{refsection}}\cite{{a}}{printing}"
    );
    borrowed.push((&section, vec!["B1", "A1", "A1"]));
    native_assignment_cases(&borrowed);
}

#[test]
fn weak_assignment_toggles_restore_at_native_group_and_environment_boundaries() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for (body, expected) in [
        (
            r"{\refcontext[labelprefix=B]\cite{a}}\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\begin{quote}\refcontext[labelprefix=A]\cite{a}\end{quote}\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\begingroup\refcontext[labelprefix=A]\cite{a}\endgroup\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\bgroup\refcontext[labelprefix=A]\cite{a}\egroup\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\refcontext[labelprefix=A] Text {\endrefcontext\cite{a}}\cite{a}\endrefcontext\cite{a}",
            vec!["→ A3", "→ B8", "→ A3"],
        ),
        (
            r"\begin{refcontext}[labelprefix=A] Text {\endrefcontext\cite{a}}\cite{a}\end{refcontext}\cite{a}",
            vec!["→ A3", "→ B8", "→ A3"],
        ),
        (
            r"\newrefcontext[labelprefix=A]\cite{a}\newrefcontext[labelprefix=B]\cite{a}\endrefcontext\cite{a}",
            vec!["→ B8", "→ B8", "→ A3"],
        ),
        (
            r"$x$\refcontext[labelprefix=A]$x$\cite{a}\endrefcontext$$x$$\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\(x\)\refcontext[labelprefix=A]\[x\]\cite{a}\endrefcontext\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\newcommand\take[1]{}\take{$}\refcontext[labelprefix=A]\cite{a}\endrefcontext\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"{\refcontext[labelprefix=A]\assignrefcontextentries[labelprefix=B]{a}\cite{a}}\cite{a}",
            vec!["→ B8", "→ B8"],
        ),
        (
            r"\verb|$\refcontext[labelprefix=A]{| Text \refcontext[labelprefix=A]\cite{a}\endrefcontext\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\begin{verbatim}
$\refcontext[labelprefix=A]{
\end{verbatim}
\refcontext[labelprefix=A]\cite{a}\endrefcontext\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\newcommand\price{\$}\price\refcontext[labelprefix=A]\cite{a}\endrefcontext\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
    ] {
        let (_dir, ws, uri) = setup(
            &format!(r"\assignrefcontextentries*[labelprefix=A]{{a}}{body}"),
            &bbl,
            &aux,
        );
        assert_eq!(hints(&ws, &uri), expected, "{body}");
    }
}

#[test]
fn weak_assignment_scope_proofs_reject_hidden_controls_and_unbalanced_execution() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for body in [
        r"\def\formula{\(}\formula\refcontext[labelprefix=A]\cite{a}\)\cite{a}",
        r"\def\formula{$}\formula\refcontext[labelprefix=A]\cite{a}\formula\cite{a}",
        r"\newcommand\open{\begingroup}\open\refcontext[labelprefix=A]\cite{a}\endgroup",
        r"\csuse{begingroup}\refcontext[labelprefix=A]\cite{a}\endgroup",
        r"\csname begingroup\endcsname\refcontext[labelprefix=A]\cite{a}\endgroup",
        r"\def\begingroup{}\begingroup\refcontext[labelprefix=A]\cite{a}",
        r"\begingroup\refcontext[labelprefix=A]\cite{a}\egroup",
        r"{\refcontext[labelprefix=A]\cite{a}\endgroup",
        r"$\refcontext[labelprefix=A]\cite{a}$\cite{a}",
        r"\(\refcontext[labelprefix=A]\cite{a}\)\cite{a}",
        r"\newcommand\take[1]{#1}\take{\refcontext[labelprefix=A]\cite{a}}",
        r"\mkbibparens{\begingroup\refcontext[labelprefix=A]\cite{a}\endgroup}\cite{a}",
        r"\mkbibbrackets{\begingroup\refcontext[labelprefix=A]\cite{a}\endgroup}\cite{a}",
        r"\parentext{\begingroup\refcontext[labelprefix=A]\cite{a}\endgroup}\cite{a}",
        r"\brackettext{\begingroup\refcontext[labelprefix=A]\cite{a}\endgroup}\cite{a}",
        r"\refcontext[labelprefix=A]\refcontext[labelprefix=B]\cite{a}",
        r"\begin{quote}\refcontext[labelprefix=A]\cite{a}\end{center}",
    ] {
        let (_dir, ws, uri) = setup(
            &format!(r"\assignrefcontextentries*[labelprefix=A]{{a}}{body}"),
            &bbl,
            &aux,
        );
        assert!(hints(&ws, &uri).is_empty(), "{body}");
    }
}

#[test]
fn weak_assignment_scopes_cross_inputs_without_treating_filenames_as_groups() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    let (dir, mut ws, uri) = setup(
        r"\assignrefcontextentries*[labelprefix=A]{a}{\input{chapter}\cite{a}}\cite{a}",
        &bbl,
        &aux,
    );
    let child = uri.join("chapter.tex").unwrap();
    std::fs::write(
        dir.path().join("chapter.tex"),
        r"\refcontext[labelprefix=A]\cite{a}",
    )
    .unwrap();
    ws.refresh_project(&uri);
    assert_eq!(hints(&ws, &uri), ["→ B8", "→ A3"]);
    assert_eq!(hints(&ws, &child), ["→ B8"]);
    let source = ws.documents[&uri]
        .text
        .source
        .replace(r"{\input{chapter}\cite{a}}", r"\input{chapter}\cite{a}}");
    ws.open(
        child.clone(),
        r"{\refcontext[labelprefix=A]\cite{a}".into(),
        8,
    );
    ws.open(uri.clone(), source, 9);
    assert_eq!(hints(&ws, &uri), ["→ B8", "→ A3"]);
    assert_eq!(hints(&ws, &child), ["→ B8"]);
}

#[test]
#[ignore = "requires native pdfLaTeX, Biber, and biblatex"]
fn native_direct_context_scopes_match_actual_labels_and_keep_the_complete_bbl() {
    let cases = [
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}{\refcontext[labelprefix=B]\cite{a}}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\begin{quote}\refcontext[labelprefix=A]\cite{a}\end{quote}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\refcontext[labelprefix=A] Text {\endrefcontext\cite{a}}\cite{a}\endrefcontext\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1", "B1", "A1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\begin{refcontext}[labelprefix=A] Text {\endrefcontext\cite{a}}\cite{a}\end{refcontext}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1", "B1", "A1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\begingroup\refcontext[labelprefix=A]\cite{a}\endgroup\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\bgroup\refcontext[labelprefix=A]\cite{a}\egroup\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}\newrefcontext[labelprefix=A]\cite{a}\newrefcontext[labelprefix=B]\cite{a}\endrefcontext\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "B1", "A1"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{a}$x$\refcontext[labelprefix=A]$x$\cite{a}\endrefcontext$$x$$\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
    ];
    native_assignment_cases(&cases);
    native_assignment_files(
        &[(
            r"\assignrefcontextentries*[labelprefix=A]{a}{\input{chapter}\cite{a}}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        )],
        &[("chapter.tex", r"\refcontext[labelprefix=A] Text")],
    );
    native_assignment_files(
        &[(
            r"\assignrefcontextentries*[labelprefix=A]{a}{\input{chapter}\cite{a}}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        )],
        &[("chapter.tex", "\\refcontext[labelprefix=A]\\endinput\r\n}{")],
    );
}

#[test]
fn weak_context_scopes_stop_after_the_last_executing_endinput_line() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for newline in ["\n", "\r\n"] {
        let (dir, mut ws, uri) = setup(
            r"\assignrefcontextentries*[labelprefix=A]{a}{\input{chapter}\cite{a}}\cite{a}",
            &bbl,
            &aux,
        );
        let child = uri.join("chapter.tex").unwrap();
        // The ignored close/open pair would restore the caller's toggle while
        // leaving the lexical stack balanced. It must never execute.
        let source = format!(r"\refcontext[labelprefix=A]\endinput{newline}}}{{");
        std::fs::write(dir.path().join("chapter.tex"), &source).unwrap();
        ws.refresh_project(&uri);
        assert_eq!(hints(&ws, &uri), ["→ B8", "→ A3"]);
        let same_line = format!(
            r"\refcontext[labelprefix=A]\endinput Text {{\endrefcontext\cite{{a}}}}{newline}}}{{"
        );
        ws.open(child.clone(), same_line, 8);
        assert_eq!(hints(&ws, &uri), ["→ B8", "→ A3"]);
        assert_eq!(hints(&ws, &child), ["→ A3"]);
    }
}

#[test]
fn weak_context_scope_termination_wrappers_keep_the_trace_guarded() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for source in [
        r"\def\finish{\endinput}\finish",
        r"\csuse{endinput}",
        r"\csname endinput\endcsname",
        r"\def\endinput{}\endinput",
    ] {
        let (dir, mut ws, uri) = setup(
            r"\assignrefcontextentries*[labelprefix=A]{a}{\input{chapter}\cite{a}}\cite{a}",
            &bbl,
            &aux,
        );
        std::fs::write(dir.path().join("chapter.tex"), format!("{source}\n}}{{")).unwrap();
        ws.refresh_project(&uri);
        assert!(hints(&ws, &uri).is_empty(), "{source}");
    }
}

fn assignment_preamble(
    preamble: &str,
    body: &str,
    bbl: &str,
    aux: &str,
) -> (tempfile::TempDir, Workspace, Url) {
    let (dir, mut ws, uri) = setup(body, bbl, aux);
    let source = ws.documents[&uri].text.source.replace(
        r"\begin{document}",
        &format!(r"{preamble}\begin{{document}}"),
    );
    ws.open(uri.clone(), source, 6);
    (dir, ws, uri)
}

#[test]
fn named_and_executed_forwarding_assignments_keep_native_strength_and_context_rules() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for command in [
        r"\csuse{assignrefcontextentries}",
        r"\csname assignrefcontextentries\endcsname",
        r"\assign",
        r"\outerassign",
        r"\csuse{outerassign}",
        r"\csname outerassign\endcsname",
    ] {
        let preamble = r"\def\assign{\assignrefcontextentries}\def\outerassign{\assign}";
        let (_dir, ws, uri) = assignment_preamble(
            preamble,
            &format!(r"{command}[labelprefix=A]{{a}} Text \cite{{a}}"),
            &bbl,
            &aux,
        );
        assert_eq!(hints(&ws, &uri), ["→ A3"], "{command}");
        // The explicit environment precedes the first weak invocation. Scope
        // collection must resolve the invocation before executing the trace.
        let body = format!(
            r"\begin{{refcontext}}[labelprefix=A]{command}*[labelprefix=A]{{a}} Text \cite{{a}}\end{{refcontext}}\cite{{a}}"
        );
        let (_dir, ws, uri) = assignment_preamble(preamble, &body, &bbl, &aux);
        assert_eq!(hints(&ws, &uri), ["→ B8", "→ A3"], "{command}");
    }
}

#[test]
fn forwarding_assignment_aliases_require_a_native_executed_global_preamble() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for (preamble, body) in [
        (
            r"\assign[labelprefix=A]{a}\def\assign{\assignrefcontextentries}",
            r"\cite{a}",
        ),
        (
            r"{\def\assign{\assignrefcontextentries}}",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\begingroup\def\assign{\assignrefcontextentries}\endgroup",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\begin{quote}\def\assign{\assignrefcontextentries}\end{quote}",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\newcommand\later{\def\assign{\assignrefcontextentries}}",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\def\outerassign{\assign}\outerassign[labelprefix=A]{a}\def\assign{\assignrefcontextentries}",
            r"\cite{a}",
        ),
        (
            r"\bgroup\def\assign{\assignrefcontextentries}\egroup",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"$\def\assign{\assignrefcontextentries}$",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\providecommand\textbf{\assignrefcontextentries}",
            r"\textbf[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\ProvideDocumentCommand\textbf{}{\assignrefcontextentries}",
            r"\textbf[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\def\def#1#2{}\def\assign{\assignrefcontextentries}",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\def\newcommand#1#2{}\newcommand\assign{\assignrefcontextentries}",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\def\csname#1\endcsname{}",
            r"\csname assignrefcontextentries\endcsname[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\let\assign\assignrefcontextentries",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"",
            r"\def\assign{\assignrefcontextentries}\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\def\assign{\assignrefcontextentries}\def\assignrefcontextentries[#1]#2{}",
            r"\assign[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"\def\csuse#1{}",
            r"\csuse{assignrefcontextentries}[labelprefix=A]{a}\cite{a}",
        ),
        (
            r"",
            r"\csname assignrefcontext\unknown\endcsname[labelprefix=A]{a}\cite{a}",
        ),
    ] {
        let (_dir, ws, uri) = assignment_preamble(preamble, body, &bbl, &aux);
        assert!(hints(&ws, &uri).is_empty(), "{preamble} {body}");
    }
}

#[test]
fn assignment_alias_declarations_follow_native_unconditional_forms_and_late_binding() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for preamble in [
        r"\def\assign{\assignrefcontextentries}",
        r"\gdef\assign{\assignrefcontextentries}",
        r"\newcommand\assign{\assignrefcontextentries}",
        r"\NewDocumentCommand\assign{}{\assignrefcontextentries}",
        r"\DeclareDocumentCommand\assign{}{\assignrefcontextentries}",
        r"\csdef{assign}{\assignrefcontextentries}",
        r"\csgdef{assign}{\assignrefcontextentries}",
        r"\renewcommand\textbf{\assignrefcontextentries}\def\assign{\textbf}",
        r"\RenewDocumentCommand\textbf{}{\assignrefcontextentries}\def\assign{\textbf}",
        r"\def\assign{\innerassign}\def\innerassign{\assignrefcontextentries}",
    ] {
        let (_dir, ws, uri) = assignment_preamble(
            preamble,
            r"\assign[labelprefix=A]{a} Text \cite{a}",
            &bbl,
            &aux,
        );
        assert_eq!(hints(&ws, &uri), ["→ A3"], "{preamble}");
    }
}

#[test]
fn assignment_forwarding_proof_bounds_executed_chains_to_64_edges() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    for count in [64, 65] {
        let mut preamble = String::new();
        for index in 0..count {
            let name = format!("assign{}", "a".repeat(index));
            let target = if index + 1 == count {
                "assignrefcontextentries".to_string()
            } else {
                format!("assign{}", "a".repeat(index + 1))
            };
            preamble.push_str(&format!(r"\def\{name}{{\{target}}}"));
        }
        let (_dir, ws, uri) = assignment_preamble(
            &preamble,
            r"\assign[labelprefix=A]{a} Text \cite{a}",
            &bbl,
            &aux,
        );
        let expected = if count == 64 {
            vec!["→ A3"]
        } else {
            Vec::new()
        };
        assert_eq!(hints(&ws, &uri), expected, "{count}");
    }
}

#[test]
#[ignore = "requires native pdfLaTeX, Biber, and biblatex"]
fn native_named_assignment_hints_match_actual_labels_and_keep_the_complete_bbl() {
    let printing = r"\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}";
    let mut cases = Vec::new();
    for (call, weak) in [
        (r"\csuse{assignrefcontextentries}", false),
        (r"\csname assignrefcontextentries\endcsname", false),
        (r"\csuse{assignrefcontextentries}", true),
        (r"\csname assignrefcontextentries\endcsname", true),
        (r"\assign", false),
        (r"\assign", true),
        (r"\outerassign", false),
        (r"\csuse{outerassign}", true),
    ] {
        let body = if weak {
            format!(
                r"{call}*[labelprefix=A]{{a}}\begin{{refcontext}}[labelprefix=A]\cite{{a}}\end{{refcontext}}\cite{{a}}{printing}"
            )
        } else {
            format!(r"{call}[labelprefix=A]{{a}}\cite{{a}}{printing}")
        };
        cases.push((body, if weak { vec!["B1", "A1"] } else { vec!["A1"] }));
    }
    for call in [r"\csuse{assignrefcontextentries}", r"\outerassign"] {
        cases.push((format!(r"\begin{{refcontext}}[labelprefix=A]{call}*[labelprefix=A]{{a}}\cite{{a}}\end{{refcontext}}\cite{{a}}{printing}"), vec!["B1", "A1"]));
    }
    let borrowed: Vec<_> = cases
        .iter()
        .map(|(body, labels)| (body.as_str(), labels.clone()))
        .collect();
    native_assignment_preamble(
        &borrowed,
        &[],
        r"\def\outerassign{\assign}\def\assign{\assignrefcontextentries}",
    );
    native_assignment_preamble(
        &borrowed[4..6],
        &[("aliases.tex", r"\def\assign{\assignrefcontextentries}")],
        r"\input{aliases}",
    );
}

#[test]
fn assignment_aliases_from_preamble_inputs_follow_unsaved_meanings_and_close_restore() {
    let bbl = compiled(list(A, "A", &["a"]) + &list(B, "B", &["a"]));
    let aux = number("a", A, 3) + &number("a", B, 8) + &default("a", B);
    let (dir, mut ws, uri) = assignment_preamble(
        r"\input{aliases}",
        r"\assign*[labelprefix=A]{a} Text \cite{a}",
        &bbl,
        &aux,
    );
    let child = uri.join("aliases.tex").unwrap();
    std::fs::write(
        dir.path().join("aliases.tex"),
        r"\def\assign{\assignrefcontextentries}",
    )
    .unwrap();
    ws.refresh_project(&uri);
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
    ws.open(child.clone(), r"\def\assign#1#2{#2}".into(), 7);
    assert_eq!(hints(&ws, &uri), ["→ B8"]);
    ws.close(&child);
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

fn category_setup(body: &str, extra_aux: &str) -> (tempfile::TempDir, Workspace, Url) {
    let bbl = compiled(list(A, "A", &["a", "z"]) + &list(B, "B", &["a", "z"]));
    let aux = number("a", A, 3)
        + &number("a", B, 8)
        + &default("a", B)
        + &number("z", A, 4)
        + &number("z", B, 9)
        + &default("z", B)
        + r"\abx@aux@category{x}{a}\abx@aux@category{y}{a}"
        + extra_aux;
    let (dir, mut ws, uri) = setup(body, &bbl, &aux);
    let source = ws.documents[&uri].text.source.replace(r"\begin{document}",
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}\begin{document}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    ws.open(uri.clone(), source, 5);
    (dir, ws, uri)
}

#[test]
fn automatic_categories_follow_matching_order_strength_and_entry_precedence() {
    for (body, expected) in [
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\cite{a,z}",
            vec!["→ A3", "→ B9"],
        ),
        (
            r"\cite{a}\assignrefcontextcats[labelprefix=A]{x}\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats[labelprefix=B]{y}\cite{a}",
            vec!["→ B8"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats[labelprefix=B]{y}\assignrefcontextcats[labelprefix=A]{x}\cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats*[labelprefix=B]{y}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats*[labelprefix=B]{x}\cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextentries*[labelprefix=B]{a}\cite{a}\assignrefcontextentries[labelprefix=A]{a}\cite{a}",
            vec!["→ B8", "→ A3"],
        ),
        (
            r"{\assignrefcontextcats[labelprefix=A]{x}}\cite{a}",
            vec!["→ A3"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{*}\cite{a}",
            vec!["→ B8"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{absent}\cite{a}",
            vec!["→ B8"],
        ),
        (
            r"\assignrefcontextcats*[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\cite{a}",
            vec!["→ A3", "→ B8", "→ A3"],
        ),
    ] {
        let (_dir, ws, uri) = category_setup(body, "");
        assert_eq!(hints(&ws, &uri), expected, "{body}");
    }
}

#[test]
fn automatic_categories_require_current_members_to_match_the_preloaded_aux() {
    let body = r"\assignrefcontextcats[labelprefix=A]{late}\cite{a}\addtocategory{late}{a}\cite{a}";
    let (_dir, ws, uri) = category_setup(body, r"\abx@aux@category{late}{a}");
    assert_eq!(hints(&ws, &uri), ["→ A3", "→ A3"]);
    let (_dir, ws, uri) = category_setup(body, "");
    assert!(hints(&ws, &uri).is_empty());
    let (_dir, mut ws, uri) =
        category_setup(r"\assignrefcontextcats[labelprefix=A]{x}\cite{a}", "");
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
    let source = ws.documents[&uri].text.source.clone();
    ws.open(
        uri.clone(),
        source.replace(r"\addtocategory{x}{a}", r"\addtocategory{x}{z}"),
        6,
    );
    assert!(hints(&ws, &uri).is_empty());
    ws.open(uri.clone(), source, 7);
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

#[test]
fn automatic_categories_keep_named_calls_and_executed_forwarding_edges() {
    for (preamble, call) in [
        ("", r"\csuse{assignrefcontextcats}"),
        ("", r"\csname assignrefcontextcats\endcsname"),
        (r"\def\assign{\assignrefcontextcats}", r"\assign"),
        (
            r"\newcommand\assign{\innerassign}\def\innerassign{\assignrefcontextcats}",
            r"\assign",
        ),
    ] {
        let (_dir, mut ws, uri) =
            category_setup(&format!(r"{call}*[labelprefix=A]{{x}}\cite{{a}}"), "");
        let source = ws.documents[&uri].text.source.replace(
            r"\begin{document}",
            &format!(r"{preamble}\begin{{document}}"),
        );
        ws.open(uri.clone(), source, 6);
        assert_eq!(hints(&ws, &uri), ["→ A3"], "{preamble} {call}");
    }
}

#[test]
fn automatic_categories_guard_hidden_members_even_when_old_aux_would_match() {
    for extra in [
        r"\newcommand\later{\addtocategory{late}{a}}\later",
        r"\newcommand\later{\addtocategory{late}{a}}",
        r"\AtBeginDocument{\addtocategory{late}{a}}",
        r"\AddToHook{unused}[mine]{\addtocategory{late}{a}}",
        r"\AddToHookNext{unused}{\addtocategory{late}{a}}",
        r"\AddToHookWithArguments{unused}{\addtocategory{late}{a}}",
        r"\AtEndDocument[mine]{\addtocategory{late}{a}}",
        r"\csuse{AddToHook}{unused}{\addtocategory{late}{a}}",
        r"\csname AddToHook\endcsname{unused}{\addtocategory{late}{a}}",
        r"\def\forward{\AddToHook}\forward{unused}{\addtocategory{late}{a}}",
        r"\let\save\addtocategory\save{late}{a}",
        r"\def\addtocategory#1#2{}",
        r"\newcommand\run[1]{#1}\run{\addtocategory{late}{a}}",
        r"\def\DeclareBibliographyCategory#1{}",
    ] {
        let (_dir, mut ws, uri) =
            category_setup(r"\assignrefcontextcats[labelprefix=A]{late}\cite{a}", "");
        let source = ws.documents[&uri]
            .text
            .source
            .replace(r"\begin{document}", &format!(r"{extra}\begin{{document}}"));
        ws.open(uri.clone(), source, 6);
        assert!(hints(&ws, &uri).is_empty(), "{extra}");
    }
    let (_dir, ws, uri) = category_setup(
        r"\detokenize{\addtocategory{late}{a}}\assignrefcontextcats[labelprefix=A]{x}\cite{a}",
        "",
    );
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

#[test]
fn automatic_categories_do_not_borrow_missing_or_mismatched_compiled_records() {
    for extra in [
        r"\abx@aux@category{x}{z}",
        r"\abx@aux@category{x}{\unknown}",
        r"\abx@aux@category{x}",
    ] {
        let (_dir, ws, uri) =
            category_setup(r"\assignrefcontextcats[labelprefix=A]{x}\cite{a}", extra);
        assert!(hints(&ws, &uri).is_empty(), "{extra}");
    }
    let (dir, ws, uri) = category_setup(r"\assignrefcontextcats[labelprefix=A]{x}\cite{a}", "");
    std::fs::write(
        dir.path().join("main.bbl"),
        compiled(list(A, "Wrong", &["a"]) + &list(B, "B", &["a"])),
    )
    .unwrap();
    assert!(hints(&ws, &uri).is_empty());
    std::fs::remove_file(dir.path().join("main.aux")).unwrap();
    assert!(hints(&ws, &uri).is_empty());
}

#[test]
fn automatic_categories_use_global_members_and_global_order_with_section_local_assignments() {
    let bbl = format!(
        r"{PROLOGUE}\refsection{{0}}{}\endrefsection\refsection{{1}}{}\endrefsection\endinput",
        list(A, "A", &["a"]) + &list(B, "B", &["a"]),
        list(A, "A", &["a"]) + &list(B, "B", &["a"])
    );
    let aux = number("a", A, 3)
        + &number("a", B, 8)
        + &default("a", B)
        + &(number("a", A, 4) + &number("a", B, 9) + &default("a", B)).replace("{0}", "{1}")
        + r"\abx@aux@category{x}{a}\abx@aux@category{y}{a}";
    let body = r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats[labelprefix=B]{y}\cite{a}\begin{refsection}\assignrefcontextcats[labelprefix=A]{x}\cite{a}\end{refsection}\cite{a}";
    let (dir, mut ws, uri) = category_setup(body, "");
    std::fs::write(dir.path().join("main.bbl"), bbl).unwrap();
    std::fs::write(dir.path().join("main.aux"), aux).unwrap();
    ws.refresh_project(&uri);
    assert_eq!(hints(&ws, &uri), ["→ B8", "→ A4", "→ A3"]);
}

#[test]
fn automatic_categories_refresh_shared_inputs_without_mutating_parent_versions() {
    let (dir, mut ws, uri) = category_setup(r"\input{assignment}\input{chapter}", "");
    std::fs::write(
        dir.path().join("assignment.tex"),
        r"\assignrefcontextcats[labelprefix=A]{x}",
    )
    .unwrap();
    std::fs::write(dir.path().join("chapter.tex"), "% query\r\n😀 \\cite{a}").unwrap();
    ws.refresh_project(&uri);
    let assignment = uri.join("assignment.tex").unwrap();
    let child = uri.join("chapter.tex").unwrap();
    assert_eq!(hints(&ws, &child), ["→ A3"]);
    ws.open(
        assignment.clone(),
        r"\assignrefcontextcats[labelprefix=B]{x}".into(),
        8,
    );
    assert_eq!(hints(&ws, &child), ["→ B8"]);
    assert_eq!(ws.documents[&uri].version, Some(5));
    let other = uri.join("other.tex").unwrap();
    let source = ws.documents[&uri].text.source.clone();
    std::fs::write(
        dir.path().join("other.aux"),
        std::fs::read(dir.path().join("main.aux")).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("other.bbl"),
        std::fs::read(dir.path().join("main.bbl")).unwrap(),
    )
    .unwrap();
    ws.open(
        other.clone(),
        source.replace(
            r"\input{assignment}",
            r"\assignrefcontextcats[labelprefix=A]{x}",
        ),
        9,
    );
    assert!(hints(&ws, &child).is_empty());
    ws.documents.remove(&other);
    assert_eq!(hints(&ws, &child), ["→ B8"]);
}

#[test]
#[ignore = "requires native pdfLaTeX, Biber, and biblatex"]
fn native_automatic_category_hints_match_actual_labels_and_complete_bbl() {
    let cases = [
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1"],
        ),
        (
            r"\cite{a}\assignrefcontextcats[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats[labelprefix=B]{y}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats[labelprefix=B]{y}\assignrefcontextcats[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats*[labelprefix=B]{y}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats*[labelprefix=B]{x}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextentries*[labelprefix=B]{a}\cite{a}\assignrefcontextentries[labelprefix=A]{a}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{late}\cite{a}\addtocategory{late}{a}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1", "A1"],
        ),
        (
            r"{\assignrefcontextcats[labelprefix=A]{x}}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{*}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1"],
        ),
        (
            r"\assignrefcontextcats*[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1", "B1", "A1"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}\newrefsection\assignrefcontextcats[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["A1", "A1"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x}\assignrefcontextcats[labelprefix=B]{y}\cite{a}\begin{refsection}\assignrefcontextcats[labelprefix=A]{x}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}\end{refsection}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
            vec!["B1", "A1"],
        ),
    ];
    native_assignment_preamble(
        &cases,
        &[],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}",
    );
    native_assignment_preamble(
        &[
            (
                r"\csuse{assignrefcontextcats}[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
                vec!["A1"],
            ),
            (
                r"\csname assignrefcontextcats\endcsname*[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\cite{a}\end{refcontext}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
                vec!["A1", "B1"],
            ),
            (
                r"\assign[labelprefix=A]{x}\cite{a}\begin{refcontext}[labelprefix=A]\nocite{a}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a}\printbibliography\end{refcontext}",
                vec!["A1"],
            ),
        ],
        &[],
        r"\DeclareBibliographyCategory{x}\DeclareBibliographyCategory{y}\DeclareBibliographyCategory{late}\DeclareBibliographyCategory{*}\addtocategory{x}{a}\addtocategory{y}{a}\def\assign{\innerassign}\def\innerassign{\assignrefcontextcats}",
    );
}

#[test]
fn category_aux_errors_do_not_disable_independent_entry_context_labels() {
    let (_dir, ws, uri) = category_setup(
        r"\assignrefcontextentries[labelprefix=A]{a}\cite{a}",
        r"\abx@aux@category{x}{\unknown}",
    );
    assert_eq!(hints(&ws, &uri), ["→ A3"]);
}

#[test]
fn automatic_categories_keep_literal_stars_and_every_physical_execution() {
    let (_dir, ws, uri) = category_setup(
        r"\addtocategory{*}{a}\assignrefcontextcats[labelprefix=A]{*}\cite{a,z}",
        r"\abx@aux@category{*}{a}",
    );
    assert_eq!(hints(&ws, &uri), ["→ A3", "→ B9"]);
    let (dir, mut ws, uri) = category_setup(
        r"\input{chapter}\assignrefcontextcats[labelprefix=A]{x}\input{chapter}",
        "",
    );
    std::fs::write(dir.path().join("chapter.tex"), r"\cite{a}").unwrap();
    ws.refresh_project(&uri);
    let child = uri.join("chapter.tex").unwrap();
    assert!(hints(&ws, &child).is_empty());
    let source = ws.documents[&uri].text.source.replace(
        r"\input{chapter}\assignrefcontextcats[labelprefix=A]{x}",
        r"\assignrefcontextcats[labelprefix=A]{x}\input{chapter}",
    );
    ws.open(uri.clone(), source, 6);
    assert_eq!(hints(&ws, &child), ["→ A3"]);
}

#[test]
fn context_assignment_lists_do_not_strip_literal_trailing_spaces() {
    for (body, expected) in [
        (
            r"\assignrefcontextentries[labelprefix=A]{a }\cite{a,z}",
            vec!["→ B8", "→ B9"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=A]{* }\cite{a,z}",
            vec!["→ B8", "→ B9"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=A]{a , z}\cite{a,z}",
            vec!["→ B8", "→ A4"],
        ),
        (
            r"\assignrefcontextentries*[labelprefix=A]{ a }\cite{a,z}",
            vec!["→ B8", "→ B9"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{x }\cite{a,z}",
            vec!["→ B8", "→ B9"],
        ),
        (
            r"\assignrefcontextcats*[labelprefix=A]{ x }\cite{a,z}",
            vec!["→ B8", "→ B9"],
        ),
        (
            r"\assignrefcontextentries[labelprefix=A]{ a}\cite{a,z}",
            vec!["→ A3", "→ B9"],
        ),
        (
            r"\assignrefcontextcats[labelprefix=A]{ x}\cite{a,z}",
            vec!["→ A3", "→ B9"],
        ),
    ] {
        let (_dir, ws, uri) = category_setup(body, "");
        assert_eq!(hints(&ws, &uri), expected, "{body}");
    }
}

#[test]
#[ignore = "requires pdflatex and biber with biblatex"]
fn native_context_assignment_lists_keep_literal_trailing_spaces() {
    let printing = r"\begin{refcontext}[labelprefix=A]\nocite{a,z}\printbibliography\end{refcontext}\begin{refcontext}[labelprefix=B]\nocite{a,z}\printbibliography\end{refcontext}";
    let bodies: Vec<_> = [
        r"\assignrefcontextentries[labelprefix=A]{a }\cite{a,z}",
        r"\assignrefcontextentries[labelprefix=A]{* }\cite{a,z}",
        r"\assignrefcontextentries[labelprefix=A]{a , z}\cite{a,z}",
        r"\assignrefcontextcats[labelprefix=A]{x }\cite{a,z}",
        r"\assignrefcontextentries[labelprefix=A]{ a}\cite{a,z}",
        r"\assignrefcontextcats[labelprefix=A]{ x}\cite{a,z}",
    ]
    .iter()
    .map(|body| format!("{body}{printing}"))
    .collect();
    let cases: Vec<_> = bodies
        .iter()
        .zip([
            vec!["B1", "B2"],
            vec!["B1", "B2"],
            vec!["B1", "A2"],
            vec!["B1", "B2"],
            vec!["A1", "B2"],
            vec!["A1", "B2"],
        ])
        .map(|(body, expected)| (body.as_str(), expected))
        .collect();
    native_assignment_preamble(
        &cases,
        &[],
        r"\DeclareBibliographyCategory{x}\addtocategory{x}{a}",
    );
}

#[path = "bib_context_keyword_tests.rs"]
mod keywords;
