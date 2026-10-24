use latex_lsp::{editing, workspace::Workspace};
use tower_lsp::lsp_types::*;
fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addsectionbib[label=chosen]{{section.bib}}\begin{{document}}{body}\end{{document}}"
    );
    for (name, text) in [("main.tex",source.as_str()),("default.bib","@book{defaultonly,title={Default}} @book{member,title={Default member}}"),("section.bib","@book{parent,title={Selected parent}} @book{member,title={Selected member}} @book{sectiononly,title={Selected}}")]{std::fs::write(dir.path().join(name),text).unwrap();}
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.code_action_literals = true;
    ws.open(main.clone(), source, 5);
    (dir, ws, main)
}
fn citations(ws: &Workspace, uri: &Url) -> Vec<Diagnostic> {
    ws.diagnostics(uri)
        .into_iter()
        .filter(|diag| diag.code == Some(NumberOrString::String("undefined-citation".into())))
        .collect()
}
fn names(ws: &Workspace, uri: &Url) -> Vec<String> {
    citations(ws, uri)
        .into_iter()
        .map(|diag| diag.data.unwrap()["key"].as_str().unwrap().to_owned())
        .collect()
}
fn actions(ws: &Workspace, uri: &Url, diag: &Diagnostic) -> Vec<CodeAction> {
    editing::code_actions(
        ws,
        uri,
        diag.range,
        &CodeActionContext {
            diagnostics: vec![diag.clone()],
            ..Default::default()
        },
    )
    .into_iter()
    .filter_map(|action| match action {
        CodeActionOrCommand::CodeAction(action) => Some(action),
        _ => None,
    })
    .collect()
}
fn repairs(ws: &Workspace, uri: &Url, diag: &Diagnostic) -> Vec<String> {
    actions(ws, uri, diag)
        .into_iter()
        .map(|a| a.title)
        .filter(|title| title.starts_with("Replace with citation"))
        .collect()
}
#[test]
fn source_sets_and_numeric_database_keys_are_checked_without_union_fallback() {
    let (_dir, ws, main) = setup(
        r"\cite{defaultonly,collected}\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collected,parent,member,sectiononly,defaultonly,absent}\newrefsection[chosen]\cite{collected,parent}",
    );
    assert_eq!(
        names(&ws, &main),
        ["collected", "defaultonly", "absent", "collected"]
    );
}
#[test]
fn an_early_source_only_request_keeps_later_requests_missing_but_a_book_stays_defined() {
    let (_dir, ws, main) = setup(
        r"\newrefsection[chosen]\cite{collected,parent}\defbibentryset{collected}{member}\defbibentryset{parent}{member}\cite{collected,parent}",
    );
    assert_eq!(names(&ws, &main), ["collected", "collected"]);
}
#[test]
fn source_only_spelling_repairs_use_the_accepted_request_position() {
    for (body, wanted) in [
        (
            r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collectd,sectononly,defautonly}",
            true,
        ),
        (
            r"\newrefsection[chosen]\cite{collectd,sectononly,defautonly}\defbibentryset{collected}{member}",
            false,
        ),
    ] {
        let (_dir, ws, main) = setup(body);
        let diagnostics = citations(&ws, &main);
        assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
        assert_eq!(
            repairs(&ws, &main, &diagnostics[0])
                .contains(&"Replace with citation `collected`".into()),
            wanted
        );
        assert!(repairs(&ws, &main, &diagnostics[1])
            .contains(&"Replace with citation `sectiononly`".into()));
        assert!(repairs(&ws, &main, &diagnostics[2]).is_empty());
    }
}
#[test]
fn named_structured_and_nocite_warnings_keep_exact_unicode_ranges() {
    let (_dir,ws,main)=setup("% 😀\r\n\\newrefsection[chosen]\\csuse{defbibentryset}{collected}{member}\\cites[see]{collected}{absent}\\csuse{cite}{absent}\\csname cite\\endcsname{absent}\\nocite{absent}");
    let diagnostics = citations(&ws, &main);
    assert_eq!(diagnostics.len(), 4, "{diagnostics:?}");
    for diag in diagnostics {
        let text = &ws.documents[&main].text;
        assert_eq!(
            &text.source
                [text.offset(diag.range.start).unwrap()..text.offset(diag.range.end).unwrap()],
            "absent"
        );
        assert_eq!(diag.range.start.line, 1);
    }
}
#[test]
fn unsaved_source_providers_refresh_warnings_and_restoring_them_recovers() {
    let (dir, mut ws, main) = setup(r"\newrefsection[chosen]\input{sets}\cite{collected}");
    let sets = main.join("sets.tex").unwrap();
    let declaration = r"\defbibentryset{collected}{member}";
    std::fs::write(dir.path().join("sets.tex"), declaration).unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source, 5);
    assert!(citations(&ws, &main).is_empty());
    ws.open(sets.clone(), r"\defbibentryset{renamed}{member}".into(), 8);
    assert_eq!(names(&ws, &main), ["collected"]);
    assert_eq!(ws.documents[&main].version, Some(5));
    ws.open(sets, declaration.into(), 9);
    assert!(citations(&ws, &main).is_empty());
}
#[test]
fn all_shared_consumers_must_agree_before_reporting_or_repairing() {
    let (dir, mut ws, main) =
        setup(r"\newrefsection[chosen]\defbibentryset{collected}{member}\input{chapter}");
    let child = main.join("chapter.tex").unwrap();
    let source = r"\cite{collectd}\ref{missinglabel}";
    std::fs::write(dir.path().join("chapter.tex"), source).unwrap();
    ws.open(child.clone(), source.into(), 3);
    assert_eq!(names(&ws, &child), ["collectd"]);
    assert_eq!(
        repairs(&ws, &child, &citations(&ws, &child)[0]),
        ["Replace with citation `collected`"]
    );
    let other = main.join("other.tex").unwrap();
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\input{chapter}\end{document}".into(),9);
    assert_eq!(names(&ws, &child), ["collectd"]);
    assert!(repairs(&ws, &child, &citations(&ws, &child)[0]).is_empty());
    ws.open(other.clone(),r"\documentclass{article}\usepackage{biblatex}\addbibresource{section.bib}\begin{document}\defbibentryset{collectd}{member}\input{chapter}\end{document}".into(),10);
    assert!(citations(&ws, &child).is_empty());
    ws.documents.remove(&other);
    assert_eq!(names(&ws, &child), ["collectd"]);
}
#[test]
fn unsafe_source_namespaces_do_not_restore_union_diagnostics_or_fixes() {
    for tail in [
        r"\newcommand\later{\cite{absent}}",
        r"\iftrue\cite{absent}\fi",
        r"\defbibentryset{nested}{collected}",
        r"\bibitem{handwritten}",
        r"\entrydata{absent}{literal}",
    ] {
        let (_dir, ws, main) = setup(&format!(
            r"\newrefsection[chosen]\defbibentryset{{collected}}{{member}}\cite{{absent}}\ref{{missinglabel}}{tail}"
        ));
        assert!(citations(&ws, &main).is_empty(), "{tail}");
        assert!(ws
            .diagnostics(&main)
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("undefined-reference".into()))));
    }
    let (_dir, mut ws, main) =
        setup(r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{absent}");
    for bib in [
        "@book{parent,ids={collected}} @book{member} @book{child,crossref={collected}}",
        "@book{member} @preamble{\"code\"}",
    ] {
        ws.open(main.join("section.bib").unwrap(), bib.into(), 8);
        assert!(citations(&ws, &main).is_empty());
    }
}
#[test]
fn compiler_confirmed_warnings_share_source_repairs_even_when_lint_is_disabled() {
    let (_dir, mut ws, main) =
        setup(r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collectd}");
    let diag = citations(&ws, &main).pop().unwrap();
    let mut compiler = diag.clone();
    compiler.source = Some("latex-build".into());
    compiler.code = None;
    compiler.message =
        "LaTeX Warning: Citation 'collectd' on page 1 undefined on input line 1.".into();
    ws.build_diagnostics.insert(
        main.clone(),
        std::collections::BTreeMap::from([(main.clone(), vec![compiler.clone()])]),
    );
    assert!(citations(&ws, &main).is_empty());
    assert_eq!(
        repairs(&ws, &main, &compiler),
        ["Replace with citation `collected`"]
    );
    ws.config.diagnostics.undefined_citations = false;
    assert_eq!(
        repairs(&ws, &main, &compiler),
        ["Replace with citation `collected`"]
    );
}

fn compile(dir: &std::path::Path) -> String {
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
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let log = std::fs::read_to_string(dir.join("main.log")).unwrap();
    assert!(!log.contains("undefined"), "{log}");
    let blg = std::fs::read_to_string(dir.join("main.blg")).unwrap();
    assert!(
        !blg.contains("ERROR") && !blg.contains("I didn't find"),
        "{blg}"
    );
    std::fs::read_to_string(dir.join("main.bbl")).unwrap()
}
fn section(bbl: &str, index: u32) -> &str {
    bbl.split_once(&format!("\\refsection{{{index}}}"))
        .unwrap()
        .1
        .split_once(r"\endrefsection")
        .unwrap()
        .0
}
fn entries(section: &str) -> std::collections::BTreeMap<String, String> {
    section
        .split(r"\entry{")
        .skip(1)
        .map(|entry| {
            let (name, rest) = entry.split_once('}').unwrap();
            (name.into(), rest.split_once(r"\endentry").unwrap().0.into())
        })
        .collect()
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn accepted_source_and_database_repairs_compile_with_separate_numeric_keys() {
    let (dir, mut ws, main) = setup(
        r"\cite{defautonly}\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collectd,sectononly}\printbibliography\newrefsection[chosen]\cite{parnt}\printbibliography",
    );
    let diagnostics = citations(&ws, &main);
    assert_eq!(diagnostics.len(), 4);
    let mut edits = Vec::new();
    for (diag, name) in
        diagnostics
            .iter()
            .zip(["defaultonly", "collected", "sectiononly", "parent"])
    {
        let action = actions(&ws, &main, diag)
            .into_iter()
            .find(|a| a.title == format!("Replace with citation `{name}`"))
            .unwrap();
        edits.extend(action.edit.unwrap().changes.unwrap().remove(&main).unwrap());
    }
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let updated = ws.documents[&main]
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
        .unwrap();
    std::fs::write(dir.path().join("main.tex"), &updated.source).unwrap();
    ws.open(main.clone(), updated.source, 6);
    assert!(citations(&ws, &main).is_empty());
    let bbl = compile(dir.path());
    let first = entries(section(&bbl, 1));
    let second = entries(section(&bbl, 2));
    assert!(first["collected"].starts_with("{set}"), "{bbl}");
    assert!(first.contains_key("member") && first.contains_key("sectiononly"));
    assert!(second["parent"].starts_with("{book}"), "{bbl}");
    assert!(!second.contains_key("collected"));
}

#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn native_early_cite_and_nocite_requests_confirm_missing_sets_and_locked_books() {
    for request in ["cite", "nocite"] {
        for raw in [false, true] {
            let members = if raw { "absent" } else { "member" };
            let body = format!(
                r"\newrefsection[chosen]\{request}{{collected}}\defbibentryset{{collected}}{{{members}}}\cite{{collected}}\printbibliography"
            );
            let (dir, mut ws, main) = setup(&body);
            let bib = if raw {
                "@book{collected,title={Raw parent}} @book{member,title={Member}}"
            } else {
                "@book{member,title={Member}}"
            };
            std::fs::write(dir.path().join("section.bib"), bib).unwrap();
            ws.open(main.join("section.bib").unwrap(), bib.into(), 8);
            assert_eq!(names(&ws, &main).len(), if raw { 0 } else { 2 });
            if raw {
                let bbl = compile(dir.path());
                assert!(entries(section(&bbl, 1))["collected"].starts_with("{book}"));
            } else {
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
                        "{}",
                        String::from_utf8_lossy(&output.stdout)
                    );
                }
                let blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
                let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
                let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
                assert!(
                    blg.contains("I didn't find") && log.contains("undefined"),
                    "{blg}\n{log}"
                );
                assert!(!bbl.contains(r"\entry{collected}"));
            }
        }
    }
}

#[test]
fn missing_key_proofs_share_large_databases_across_many_numeric_sections() {
    let (dir, mut ws, main) = setup("");
    let mut library = String::from("@book{member,title={Member}}");
    for index in 0..1500 {
        library.push_str(&format!(
            " @book{{parent{index},crossref={{member}},title={{Book {index}}}}}"
        ));
    }
    std::fs::write(dir.path().join("section.bib"), &library).unwrap();
    ws.open(main.join("section.bib").unwrap(), library, 8);
    let mut source = ws.documents[&main]
        .text
        .source
        .replace(r"\end{document}", "");
    for _ in 0..350 {
        source.push_str(
            r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collected,absent}",
        );
    }
    source.push_str(r"\end{document}");
    ws.open(main.clone(), source, 6);
    let diagnostics = citations(&ws, &main);
    assert_eq!(diagnostics.len(), 350, "{diagnostics:?}");
    assert!(diagnostics
        .iter()
        .all(|diag| diag.data.as_ref().unwrap()["key"] == "absent"));
    ws.index_truncated.insert("maxFiles".into());
    assert!(citations(&ws, &main).is_empty());
}

#[test]
fn ambiguous_set_requests_keep_their_guard_without_hiding_independent_missing_keys() {
    let (_dir, ws, main) = setup(
        r"\newrefsection[chosen]\cite{collected}\defbibentryset{collected}{member}\nocite{*}\cite{collected,absent}",
    );
    assert_eq!(names(&ws, &main), ["absent"]);
}
#[test]
fn duplicate_candidates_do_not_become_repairs_and_completion_limits_do_not_hide_repairs() {
    let (_dir, mut ws, main) =
        setup(r"\newrefsection[chosen]\defbibentryset{collected}{member}\cite{collectd,parnt}");
    ws.config.indexing.completion_limit = 1;
    assert_eq!(
        repairs(&ws, &main, &citations(&ws, &main)[0]),
        ["Replace with citation `collected`"]
    );
    ws.open(
        main.join("section.bib").unwrap(),
        "@book{member} @book{parent,title={First}} @book{parent,title={Second}}".into(),
        8,
    );
    let diagnostics = citations(&ws, &main);
    assert_eq!(diagnostics.len(), 2);
    assert!(repairs(&ws, &main, &diagnostics[1]).is_empty());
}

#[test]
fn static_named_quoted_source_repairs_replace_only_the_literal_key() {
    let (_dir, ws, main) = setup("% 😀\r\n\\newrefsection[chosen]\\csname defbibentryset\\endcsname{O\"Connor}{member}\\csuse{cite}{O\"Conor}");
    let diagnostics = citations(&ws, &main);
    assert_eq!(diagnostics.len(), 1);
    let action = actions(&ws, &main, &diagnostics[0])
        .into_iter()
        .find(|a| a.title == "Replace with citation `O\"Connor`")
        .unwrap();
    let edits = action.edit.unwrap().changes.unwrap().remove(&main).unwrap();
    assert_eq!(
        edits,
        [TextEdit {
            range: diagnostics[0].range,
            new_text: "O\"Connor".into()
        }]
    );
}

#[test]
fn active_source_members_report_missing_keys_and_offer_only_valid_database_repairs() {
    for request in ["", r"\cite{collected}"] {
        let (_dir, ws, main) = setup(&format!(
            r"\newrefsection[chosen]\defbibentryset{{collected}}{{membr,defaultonly}}{request}"
        ));
        let diagnostics = citations(&ws, &main);
        assert_eq!(names(&ws, &main), ["membr", "defaultonly"]);
        assert_eq!(
            repairs(&ws, &main, &diagnostics[0]),
            ["Replace with citation `member`"]
        );
        assert!(repairs(&ws, &main, &diagnostics[1]).is_empty());
    }
    let (_dir, ws, main) =
        setup(r"\defbibentryset{collected}{defautonly}\newrefsection[chosen]\cite{parent}");
    let diagnostics = citations(&ws, &main);
    assert_eq!(names(&ws, &main), ["defautonly"]);
    assert_eq!(
        repairs(&ws, &main, &diagnostics[0]),
        ["Replace with citation `defaultonly`"]
    );
}
#[test]
fn an_early_locked_book_does_not_report_members_of_the_suppressed_source_set() {
    let (_dir, ws, main) = setup(
        r"\newrefsection[chosen]\cite{parent}\defbibentryset{parent}{absent}\cite{parent,othermissing}",
    );
    assert_eq!(names(&ws, &main), ["othermissing"]);
}
#[test]
#[ignore = "requires pdflatex, biblatex and biber"]
fn accepted_member_repairs_remove_native_biber_missing_member_warnings() {
    for request in ["", r"\cite{collected}"] {
        let (dir, mut ws, main) = setup(&format!(
            r"\newrefsection[chosen]\defbibentryset{{collected}}{{membr}}{request}\printbibliography"
        ));
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
        ] {
            let output = std::process::Command::new(program)
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        let original_blg = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
        assert!(
            original_blg.contains("database entry for dynamic set member 'membr'"),
            "{original_blg}"
        );
        let diagnostic = citations(&ws, &main).pop().unwrap();
        let action = actions(&ws, &main, &diagnostic)
            .into_iter()
            .find(|a| a.title == "Replace with citation `member`")
            .unwrap();
        let edits = action.edit.unwrap().changes.unwrap().remove(&main).unwrap();
        let updated = ws.documents[&main]
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
            .unwrap();
        std::fs::write(dir.path().join("main.tex"), &updated.source).unwrap();
        ws.open(main.clone(), updated.source, 6);
        assert!(citations(&ws, &main).is_empty());
        let bbl = compile(dir.path());
        let set = entries(section(&bbl, 1));
        assert!(set["collected"].contains(r"\set{member}"));
        assert!(set.contains_key("member"));
    }
}
