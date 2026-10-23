use latex_lsp::{editing, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        r"\documentclass{{article}}\usepackage{{biblatex}}\addbibresource{{default.bib}}\addglobalbib{{global.bib}}\addsectionbib[label=chosen]{{section.bib}}\begin{{document}}{body}\end{{document}}"
    );
    for (name, contents) in [
        ("main.tex", source.as_str()),
        (
            "default.bib",
            "@book{defaultonly,title={Default},ids={defaultalias}}",
        ),
        (
            "section.bib",
            "@book{sectiononly,title={Selected},ids={sectionalias}}",
        ),
        ("global.bib", "@book{always,title={Global}}"),
    ] {
        std::fs::write(dir.path().join(name), contents).unwrap();
    }
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
fn fixes(ws: &Workspace, uri: &Url, diagnostic: &Diagnostic) -> Vec<String> {
    editing::code_actions(
        ws,
        uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic.clone()],
            ..Default::default()
        },
    )
    .into_iter()
    .filter_map(|action| match action {
        CodeActionOrCommand::CodeAction(action) => Some(action.title),
        CodeActionOrCommand::Command(_) => None,
    })
    .collect()
}
#[test]
fn missing_keys_are_checked_against_each_active_default_selected_and_global_library() {
    let (_dir, ws, main) = setup(
        r"\cite{defaultonly,defaultalias,always,sectiononly,sectionalias}\newrefsection[chosen]\cite{sectiononly,sectionalias,always,defaultonly,defaultalias}\newrefsection[]\cite{defaultonly,always}\nocite{*}",
    );
    assert_eq!(
        names(&ws, &main),
        ["sectiononly", "sectionalias", "defaultonly", "defaultalias"]
    );
}
#[test]
fn structured_named_and_nocite_warnings_keep_exact_utf16_ranges_and_exclude_queries() {
    let (_dir, ws, main) = setup("😀\r\n\\cites{always}[see]{sectiononly}\\newrefsection[chosen]\\volcite{7}{defaultonly}\\csuse{cite}{defaultonly}\\csname cite\\endcsname{defaultonly}\\nocite{defaultonly,*}\\entrydata{defaultonly}{literal}");
    let diagnostics = citations(&ws, &main);
    assert_eq!(diagnostics.len(), 5, "{diagnostics:?}");
    for diagnostic in diagnostics {
        let doc = &ws.documents[&main];
        let start = doc.text.offset(diagnostic.range.start).unwrap();
        let end = doc.text.offset(diagnostic.range.end).unwrap();
        assert_eq!(
            &doc.text.source[start..end],
            diagnostic.data.as_ref().unwrap()["key"].as_str().unwrap()
        );
        assert_eq!(diagnostic.range.start.line, 1);
    }
}
#[test]
fn spelling_repairs_offer_only_keys_from_the_same_reference_section() {
    let (_dir, ws, main) =
        setup(r"\cite{defautonly}\newrefsection[chosen]\cite{sectononly,defautonly}");
    let diagnostics = citations(&ws, &main);
    assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
    assert!(fixes(&ws, &main, &diagnostics[0])
        .iter()
        .any(|title| title == "Replace with citation `defaultonly`"));
    assert!(fixes(&ws, &main, &diagnostics[1])
        .iter()
        .any(|title| title == "Replace with citation `sectiononly`"));
    assert!(!fixes(&ws, &main, &diagnostics[2])
        .iter()
        .any(|title| title.contains("defaultonly")));
}
#[test]
fn uncertain_execution_suppresses_citation_guesses_but_preserves_label_warnings() {
    for body in [
        r"\iftrue\newrefsection[chosen]\fi\cite{absent}",
        r"\newcommand\run[1]{#1}\run{\newrefsection[chosen]}\cite{absent}",
        r"\newrefsection[\dynamic]\cite{absent}",
        r"\newrefsection[chosen]\newcommand\later{\cite{absent}}",
    ] {
        let (_dir, ws, main) = setup(&format!(r"{body}\ref{{missinglabel}}"));
        assert!(citations(&ws, &main).is_empty(), "{body}");
        assert!(ws.diagnostics(&main).iter().any(|diag| diag.code == Some(NumberOrString::String("undefined-reference".into()))), "{body}");
    }
}
#[test]
fn unsaved_library_edits_refresh_parent_warnings_without_changing_its_version() {
    let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{sectiononly}");
    let bib = main.join("section.bib").unwrap();
    assert!(citations(&ws, &main).is_empty());
    ws.open(bib.clone(), "@book{renamed,title={Selected}}".into(), 1);
    assert_eq!(ws.documents[&main].version, Some(5));
    assert_eq!(names(&ws, &main), ["sectiononly"]);
    ws.open(bib, "@book{sectiononly,title={Selected}}".into(), 2);
    assert!(citations(&ws, &main).is_empty());
}
#[test]
fn repeated_inputs_and_unsaved_shared_roots_require_agreeing_resource_contexts() {
    let (dir, mut ws, main) = setup(r"\newrefsection[chosen]\input{chapter}");
    let child = main.join("chapter.tex").unwrap();
    std::fs::write(
        dir.path().join("chapter.tex"),
        r"\cite{defaultonly}\ref{missinglabel}",
    )
    .unwrap();
    let source = ws.documents[&main].text.source.clone();
    ws.open(main.clone(), source.clone(), 5);
    ws.open(
        child.clone(),
        r"\cite{defaultonly}\ref{missinglabel}".into(),
        3,
    );
    assert_eq!(names(&ws, &child), ["defaultonly"]);
    let other = main.join("new-root.tex").unwrap();
    ws.open(
        other.clone(),
        source.replace(r"\newrefsection[chosen]", ""),
        8,
    );
    assert!(citations(&ws, &child).is_empty());
    ws.open(other.clone(), source.clone(), 9);
    assert_eq!(names(&ws, &child), ["defaultonly"]);
    ws.documents.remove(&other);
    ws.open(
        main.clone(),
        source.replace(
            r"\end{document}",
            r"\endrefsection\input{chapter}\end{document}",
        ),
        6,
    );
    assert!(citations(&ws, &child).is_empty());
    assert_eq!(ws.documents[&child].version, Some(3));
}
#[test]
fn disabled_and_compiler_confirmed_citation_warnings_are_not_duplicated() {
    let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{defaultonly}");
    let diagnostics = citations(&ws, &main);
    assert_eq!(diagnostics.len(), 1);
    let mut compiler = diagnostics[0].clone();
    compiler.source = Some("latex-build".into());
    compiler.code = None;
    compiler.message =
        "LaTeX Warning: Citation 'defaultonly' on page 1 undefined on input line 1.".into();
    ws.build_diagnostics.insert(
        main.clone(),
        std::collections::BTreeMap::from([(main.clone(), vec![compiler])]),
    );
    assert!(citations(&ws, &main).is_empty());
    ws.build_diagnostics.clear();
    ws.config.diagnostics.undefined_citations = false;
    assert!(citations(&ws, &main).is_empty());
}
#[test]
fn compiler_citation_spelling_repairs_also_reject_out_of_section_libraries() {
    let (_dir, mut ws, main) = setup(r"\newrefsection[chosen]\cite{defautonly,sectononly}");
    let source = ws.documents[&main].text.source.clone();
    let text = Text::new(source.clone());
    let diagnostics: Vec<_> = ["defautonly", "sectononly"]
        .into_iter()
        .map(|name| {
            let start = source.find(name).unwrap();
            Diagnostic {
                range: text.range(start, start + name.len()),
                source: Some("latex-build".into()),
                severity: Some(DiagnosticSeverity::WARNING),
                message: format!(
                    "LaTeX Warning: Citation '{name}' on page 1 undefined on input line 1."
                ),
                ..Default::default()
            }
        })
        .collect();
    ws.build_diagnostics.insert(
        main.clone(),
        std::collections::BTreeMap::from([(main.clone(), diagnostics.clone())]),
    );
    assert!(!fixes(&ws, &main, &diagnostics[0])
        .iter()
        .any(|title| title.contains("defaultonly")));
    assert!(fixes(&ws, &main, &diagnostics[1])
        .iter()
        .any(|title| title.contains("sectiononly")));
    ws.config.diagnostics.undefined_citations = false;
    assert!(fixes(&ws, &main, &diagnostics[1])
        .iter()
        .any(|title| title.contains("sectiononly")));
    // An unresolvable section owns these compiler warnings too: no union fallback.
    ws.open(main.clone(), source.replace("[chosen]", "[absent]"), 6);
    ws.build_diagnostics.insert(
        main.clone(),
        std::collections::BTreeMap::from([(main.clone(), diagnostics.clone())]),
    );
    for diagnostic in &diagnostics {
        assert!(!fixes(&ws, &main, diagnostic)
            .iter()
            .any(|title| title.starts_with("Replace with citation")));
    }
}

#[test]
#[ignore = "requires pdflatex, biber and biblatex"]
fn real_compiler_citation_repairs_use_each_sections_resources_and_clear_the_warnings() {
    let (dir, mut ws, main) =
        setup(r"\cite{defautonly}\newrefsection[chosen]\cite{sectononly}\printbibliography");
    assert_eq!(names(&ws, &main), ["defautonly", "sectononly"]);
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        output_directory: Some(".".into()),
        arguments: Some(
            [
                "-no-shell-escape",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "-recorder",
                "-output-directory=%o",
                "%f",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ),
        ..Default::default()
    };
    let first = latex_lsp::build::run(&dir.path().join("main.tex"), &config, &Default::default());
    assert_eq!(first.status, "success", "{}", first.output);
    let warnings: Vec<_> = first.diagnostics[&main]
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("Citation '"))
        .cloned()
        .collect();
    assert_eq!(warnings.len(), 2, "{:?}", first.diagnostics);
    ws.build_diagnostics.insert(main.clone(), first.diagnostics);
    assert!(citations(&ws, &main).is_empty());
    let mut edits = Vec::new();
    for diagnostic in warnings {
        let name = if diagnostic.message.contains("defautonly") {
            "defaultonly"
        } else {
            "sectiononly"
        };
        let action = editing::code_actions(
            &ws,
            &main,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                ..Default::default()
            },
        )
        .into_iter()
        .find_map(|action| match action {
            CodeActionOrCommand::CodeAction(action)
                if action.title == format!("Replace with citation `{name}`") =>
            {
                Some(action)
            }
            _ => None,
        })
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
    ws.build_diagnostics.clear();
    assert!(citations(&ws, &main).is_empty());
    let result = latex_lsp::build::run(&dir.path().join("main.tex"), &config, &Default::default());
    assert_eq!(result.status, "success", "{}", result.output);
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
    let biber = std::fs::read_to_string(dir.path().join("main.blg")).unwrap();
    assert!(!biber.contains("WARN"), "{biber}");
    for pass in 0..2 {
        let result =
            latex_lsp::build::run(&dir.path().join("main.tex"), &config, &Default::default());
        assert_eq!(result.status, "success", "{}", result.output);
        if pass == 1 {
            assert!(
                !result
                    .diagnostics
                    .values()
                    .flatten()
                    .any(|diagnostic| diagnostic.message.contains("undefined")),
                "{:?}",
                result.diagnostics
            );
        }
    }
    let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
    assert!(bbl.contains("Default") && bbl.contains("Selected"), "{bbl}");
}
