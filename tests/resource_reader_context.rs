use lsp_types::*;
use texquill::{
    language::{features, navigation},
    project::workspace::Workspace,
    text::Text,
};

fn setup(provider: &str, body: &str) -> (tempfile::TempDir, Workspace, Url, String) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!("\\documentclass{{article}}\\usepackage{{provider}}{body}");
    for (name, text) in [
        ("main.tex", source.as_str()),
        ("provider.sty", provider),
        ("refs%data.bib", "@book{paper,title={Paper}}"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    ws.open(main.clone(), source.clone(), 5);
    (dir, ws, main, source)
}
fn position(source: &str, key: &str) -> Position {
    Text::new(source.into()).position(source.rfind(key).unwrap() + 1)
}

#[test]
fn imported_section_and_file_forwarders_preserve_literal_percent_and_following_keys() {
    for (provider, body) in [
        (
            r"\def\choose{\newrefsection}",
            r"\choose[refs%data.bib]\label{kept}\ref{kept}",
        ),
        (
            r"\def\choose{\newrefsection}",
            r"\csuse{choose}[refs%data.bib]\label{kept}\ref{kept}",
        ),
        (
            r"\def\choose{\newrefsection}",
            r"\csname choose\endcsname[refs%data.bib]\label{kept}\ref{kept}",
        ),
        (
            r"\def\register{\addbibresource}",
            r"\register{refs%data.bib}\label{kept}\ref{kept}",
        ),
        (
            r"\def\register{\addbibresource}",
            r"\csuse{register}{refs%data.bib}\label{kept}\ref{kept}",
        ),
    ] {
        let (dir, ws, main, source) = setup(provider, body);
        assert_eq!(
            features::definitions(&ws, &main, position(&source, "kept")).len(),
            1,
            "{body}"
        );
        let bib = Url::from_file_path(dir.path().join("refs%data.bib")).unwrap();
        assert!(ws.project_files(&main).contains(&bib), "{body}");
        assert!(
            navigation::document_links(&ws, &main)
                .iter()
                .any(|link| link.target.as_ref() == Some(&bib)),
            "{body}"
        );
    }
}
#[test]
fn imported_overrides_restore_comments_and_do_not_certify_phantom_labels() {
    for (provider, body) in [
        (
            r"\renewcommand\newrefsection[1][]{}",
            "\\newrefsection[refs% ignored]\\label{fake}\n]\\label{kept}\\ref{fake}\\ref{kept}",
        ),
        (
            r"\renewcommand\addbibresource[1]{}",
            "\\addbibresource{refs% ignored}\\label{fake}\n}\\label{kept}\\ref{fake}\\ref{kept}",
        ),
    ] {
        let (_dir, ws, main, source) = setup(provider, body);
        assert!(
            features::definitions(&ws, &main, position(&source, "fake")).is_empty(),
            "{body}"
        );
        assert_eq!(
            features::definitions(&ws, &main, position(&source, "kept")).len(),
            1,
            "{body}"
        );
        assert!(
            ws.diagnostics(&main)
                .iter()
                .any(|d| d.message == "Undefined reference `fake`"),
            "{body}"
        );
    }
}
#[test]
fn imported_outer_arguments_keep_precollected_resource_comments() {
    let provider = r"\newcommand\discard[1]{}";
    let body="\\discard{\\newrefsection[refs% ignored]\\label{fake}\n]}\\label{kept}\\ref{fake}\\ref{kept}";
    let (_dir, ws, main, source) = setup(provider, body);
    assert!(features::definitions(&ws, &main, position(&source, "fake")).is_empty());
    assert_eq!(
        features::definitions(&ws, &main, position(&source, "kept")).len(),
        1
    );
}
#[test]
fn later_imports_do_not_rewrite_the_reader_meaning_before_the_load() {
    let body="\\newrefsection[refs%data.bib]\\label{before}\\ref{before}\\usepackage{override}\\newrefsection[refs% hidden]\\label{fake}\n]\\ref{fake}";
    let (dir, mut ws, main, source) = setup("", body);
    std::fs::write(
        dir.path().join("override.sty"),
        r"\renewcommand\newrefsection[1][]{}",
    )
    .unwrap();
    ws.open(main.clone(), source.clone(), 6);
    assert_eq!(
        features::definitions(&ws, &main, position(&source, "before")).len(),
        1
    );
    assert!(features::definitions(&ws, &main, position(&source, "fake")).is_empty());
}

#[test]
fn shared_reader_disagreement_and_unsaved_provider_changes_refresh_catcodes() {
    let dir = tempfile::tempdir().unwrap();
    let a = r"\documentclass{article}\def\choose{\newrefsection}\input{shared}";
    let b = r"\documentclass{article}\newcommand\choose[1][]{}\input{shared}";
    let shared = "\\choose[refs%data.bib]\\label{maybe}\n]\\label{common}\\ref{maybe}\\ref{common}";
    for (name, text) in [
        ("a.tex", a),
        ("b.tex", b),
        ("shared.tex", shared),
        ("refs%data.bib", "@book{paper,title={Paper}}"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let shared_uri = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let b_uri = Url::from_file_path(dir.path().join("b.tex")).unwrap();
    assert!(features::definitions(&ws, &shared_uri, position(shared, "maybe")).is_empty());
    assert_eq!(
        features::definitions(&ws, &shared_uri, position(shared, "common")).len(),
        1
    );
    ws.open(b_uri.clone(), a.into(), 7);
    assert_eq!(
        features::definitions(&ws, &shared_uri, position(shared, "maybe")).len(),
        1
    );
    ws.open(b_uri, b.into(), 8);
    assert!(features::definitions(&ws, &shared_uri, position(shared, "maybe")).is_empty());
    assert_eq!(
        features::definitions(&ws, &shared_uri, position(shared, "common")).len(),
        1
    );
}
#[test]
fn exposed_string_test_calls_keep_their_imported_parameter_meanings() {
    let body = r"\choose[refs%data.bib]\ifstrequal{\label{kept}}{}\ref{kept}";
    let (_dir, ws, main, source) = setup(
        r"\def\choose{\newrefsection}\renewcommand\ifstrequal[2]{#1#2}",
        body,
    );
    assert_eq!(
        features::definitions(&ws, &main, position(&source, "kept")).len(),
        1
    );
    let (_dir, ws, main, source) = setup(
        r"\def\choose{\newrefsection}",
        r"\choose[refs%data.bib]\ifstrequal{\label{fake}}{}{}{}\ref{fake}",
    );
    assert!(features::definitions(&ws, &main, position(&source, "fake")).is_empty());
}
#[test]
fn grouped_alias_declarations_do_not_certify_a_reader_outside_their_scope() {
    let (_dir, ws, main, source) = setup(
        r"{\def\choose{\newrefsection}}",
        "\\choose[refs% hidden]\\label{fake}\n]\\ref{fake}",
    );
    assert!(features::definitions(&ws, &main, position(&source, "fake")).is_empty());
}
#[test]
fn unfinished_raw_group_readers_keep_missing_brace_diagnostics() {
    let (_dir, ws, main, _source) = setup(
        r"\def\register{\addbibresource}",
        r"\register{refs%data.bib",
    );
    assert!(ws
        .diagnostics(&main)
        .iter()
        .any(|diagnostic| diagnostic.message == "Unclosed brace"));
}

#[test]
fn an_oversized_project_does_not_change_an_independent_reader_context() {
    let (_dir, mut ws, main, source) = setup(
        r"\def\choose{\newrefsection}",
        r"\choose[refs%data.bib]\label{kept}\ref{kept}",
    );
    let huge = Url::parse("untitled:oversized-resource-project").unwrap();
    let huge_source = format!(
        "\\documentclass{{article}}{}\\newrefsection[refs% hidden]\\label{{fake}}\n]\\ref{{fake}}",
        " ".repeat(2 * 1024 * 1024)
    );
    ws.open(huge.clone(), huge_source, 1);
    assert!(ws.documents[&huge]
        .syntax
        .labels
        .iter()
        .all(|entry| entry.name != "fake"));
    assert_eq!(
        features::definitions(&ws, &main, position(&source, "kept")).len(),
        1
    );
}

#[test]
fn exposed_source_inputs_are_loaded_from_the_calling_root_directory() {
    let (dir, mut ws, main, _) = setup(r"\def\choose{\newrefsection}", "");
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(
        dir.path().join("sub/chapter.tex"),
        r"\choose[refs%data.bib]\input{definitions.data}",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("definitions.data"),
        r"\newcommand\loadedmarker{}",
    )
    .unwrap();
    let source = r"\documentclass{article}\usepackage{provider}\input{sub/chapter}\loadedmarker";
    ws.open(main.clone(), source.into(), 9);
    let data = Url::from_file_path(dir.path().join("definitions.data")).unwrap();
    assert!(ws.documents.contains_key(&data));
    assert_eq!(
        features::definitions(&ws, &main, position(source, "loadedmarker")).len(),
        1
    );
}

#[test]
fn mixed_group_readers_do_not_certify_argument_labels_or_partial_macro_renames() {
    let dir = tempfile::tempdir().unwrap();
    let a = r"\documentclass{article}\def\register{\addbibresource}\input{shared}";
    let b = r"\documentclass{article}\newcommand\register{}\input{shared}";
    let shared =
        "\\newcommand\\pathpart{}\\register{refs\\pathpart\\label{fake}% ignored}\n}\\ref{fake}";
    for (name, text) in [("a.tex", a), ("b.tex", b), ("shared.tex", shared)] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let shared_uri = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    assert!(features::definitions(&ws, &shared_uri, position(shared, "fake")).is_empty());
    let at = Text::new(shared.into()).position(shared.find("pathpart").unwrap() + 2);
    assert!(features::rename(&ws, &shared_uri, at, "updated").is_err());
}

#[test]
fn named_reader_maps_are_revoked_when_the_only_imported_alias_is_removed() {
    let body="\\csname choose\\endcsname[refs%data.bib]\\label{maybe}\n]\\label{common}\\ref{maybe}\\ref{common}";
    let (dir, mut ws, main, source) = setup(r"\def\choose{\newrefsection}", body);
    assert_eq!(
        features::definitions(&ws, &main, position(&source, "maybe")).len(),
        1
    );
    let provider = Url::from_file_path(dir.path().join("provider.sty")).unwrap();
    ws.open(provider, r"\newcommand\choose[1][]{}".into(), 8);
    assert!(features::definitions(&ws, &main, position(&source, "maybe")).is_empty());
    assert_eq!(
        features::definitions(&ws, &main, position(&source, "common")).len(),
        1
    );
}

#[test]
fn opaque_resource_arguments_also_prevent_partial_category_renames() {
    let dir = tempfile::tempdir().unwrap();
    let a = r"\documentclass{article}\def\register{\addbibresource}\input{shared}";
    let b = r"\documentclass{article}\newcommand\register{}\input{shared}";
    let shared = "\\DeclareBibliographyCategory{selected}\\register{refs\\addtocategory{selected}{paper}% ignored}\n}";
    for (name, text) in [("a.tex", a), ("b.tex", b), ("shared.tex", shared)] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let uri = Url::from_file_path(dir.path().join("shared.tex")).unwrap();
    let at = Text::new(shared.into()).position(shared.find("selected").unwrap() + 2);
    assert!(features::rename(&ws, &uri, at, "updated").is_err());
}
#[test]
#[ignore = "requires pdflatex and biblatex"]
fn real_imported_resource_readers_and_overrides_preserve_label_renames() {
    for (provider, body, fake) in [
        (
            r"\def\choose{\newrefsection}",
            r"\choose[refs%data.bib]\label{kept}\ref{kept}",
            false,
        ),
        (
            r"\renewcommand\newrefsection[1][]{}",
            "\\newrefsection[refs% ignored]\\label{fake}\n]\\label{kept}\\ref{fake}\\ref{kept}",
            true,
        ),
    ] {
        let (dir, mut ws, main, source) = setup(provider, body);
        let source = source.replace(
            r"\usepackage{provider}",
            r"\usepackage{biblatex}\usepackage{provider}\begin{document}",
        ) + r"\end{document}";
        ws.open(main.clone(), source.clone(), 7);
        let edit = features::rename(&ws, &main, position(&source, "kept"), "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()
            .remove(&main)
            .unwrap();
        assert_eq!(edit.len(), 2);
        let mut edit = edit;
        edit.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let updated = Text::new(source.clone())
            .changed(
                edit.into_iter()
                    .map(|edit| TextDocumentContentChangeEvent {
                        range: Some(edit.range),
                        range_length: None,
                        text: edit.new_text,
                    })
                    .collect(),
            )
            .unwrap()
            .source;
        for text in [source.as_str(), updated.as_str()] {
            std::fs::write(main.to_file_path().unwrap(), text).unwrap();
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
            }
            let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
            assert_eq!(log.contains("Reference `fake'"), fake, "{log}");
            assert!(!log.contains("Reference `updated'"), "{log}");
            assert!(!log.contains("Reference `kept'"), "{log}");
        }
        let aux = std::fs::read_to_string(dir.path().join("main.aux")).unwrap();
        assert!(aux.contains(r"\newlabel{updated}"));
        assert!(!aux.contains(r"\newlabel{kept}"));
    }
}
