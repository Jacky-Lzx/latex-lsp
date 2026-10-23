use latex_lsp::{features, signature, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.snippets = true;
    let uri = Url::parse("untitled:bib-sections.tex").unwrap();
    ws.open(uri.clone(), source.into(), 6);
    (ws, uri)
}
fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
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
        .unwrap()
        .source
}

#[test]
fn section_and_segment_boundaries_keep_following_reference_keys_independent() {
    for (command, args) in [
        ("newrefsection", "[refs.bib]"),
        ("newrefsection", ""),
        ("newrefsegment", ""),
        ("endrefsection", ""),
        ("endrefsegment", ""),
    ] {
        for call in [
            format!(r"\{command}"),
            format!(r"\csuse{{{command}}}"),
            format!(r"\csname {command}\endcsname"),
            r"\sectionboundary".into(),
        ] {
            let source = format!(
                r"\label{{target}}\bibitem{{paper}}\def\sectionboundary{{\{command}}}{call}{args}\ref{{target}}\cite{{paper}}"
            );
            let (ws, uri) = setup(&source);
            for key in ["target", "paper"] {
                let at = ws.documents[&uri]
                    .text
                    .position(source.rfind(key).unwrap() + 1);
                assert_eq!(features::definitions(&ws, &uri, at).len(), 1, "{source}");
                assert_eq!(
                    features::rename(&ws, &uri, at, "updated")
                        .unwrap()
                        .unwrap()
                        .changes
                        .unwrap()[&uri]
                        .len(),
                    2,
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn section_signatures_preserve_native_optional_brackets_and_end_before_the_body() {
    for (call, label, active) in [
        (r"\newrefsection", r"\newrefsection[resources]", 0),
        (
            r"\csuse{newrefsection}",
            r"\csuse{newrefsection}[resources]",
            0,
        ),
        (
            r"\csname newrefsection\endcsname",
            r"\csname newrefsection\endcsname[resources]",
            0,
        ),
        (r"\sectionboundary", r"\sectionboundary[resources]", 0),
        (r"\begin{refsection}", r"\begin{refsection}[resources]", 1),
        (
            r"\csuse{begin}{refsection}",
            r"\csuse{begin}{refsection}[resources]",
            1,
        ),
    ] {
        for tail in ["[re¦fs.bib]", "[{protected ] clo¦sing}]", "[unfin¦ished"] {
            let marked = format!(r"\def\sectionboundary{{\newrefsection}}😀 {call}{tail}");
            let (ws, uri) = setup(&marked.replace('¦', ""));
            let at = ws.documents[&uri].text.position(marked.find('¦').unwrap());
            let help = signature::help(&ws, &uri, at).unwrap_or_else(|| panic!("{marked}"));
            assert_eq!(help.signatures[0].label, label, "{marked}");
            assert_eq!(help.active_parameter, Some(active), "{marked}");
        }
        for tail in ["[literal [opening] body¦", "[{protected ] closing}] body¦"] {
            let marked = format!(r"\def\sectionboundary{{\newrefsection}}{call}{tail}");
            let (ws, uri) = setup(&marked.replace('¦', ""));
            let at = ws.documents[&uri].text.position(marked.find('¦').unwrap());
            assert!(signature::help(&ws, &uri, at).is_none(), "{marked}");
        }
    }
}

#[test]
fn resource_selectors_are_data_and_user_redefinitions_keep_their_own_grammar() {
    for call in [
        r"\newrefsection",
        r"\begin{refsection}",
        r"\csuse{begin}{refsection}",
    ] {
        let source = format!(r"\label{{target}}{call}[\ref{{target}}]\ref{{target}}");
        let (ws, uri) = setup(&source);
        let at = ws.documents[&uri]
            .text
            .position(source.find(r"\ref{target}").unwrap() + 6);
        assert!(features::definitions(&ws, &uri, at).is_empty(), "{source}");
        assert!(
            features::rename(&ws, &uri, Position::new(0, 9), "updated").is_err(),
            "{source}"
        );
        let at = ws.documents[&uri]
            .text
            .position(source.rfind("target").unwrap() + 1);
        assert_eq!(features::definitions(&ws, &uri, at).len(), 1, "{source}");
    }
    for declaration in [
        r"\def\newrefsection#1{}\newrefsection{\ref{target}}",
        r"\renewenvironment{refsection}[1]{}{}\begin{refsection}{\ref{target}}",
    ] {
        let source = format!(r"\label{{target}}{declaration}");
        let (ws, uri) = setup(&source);
        let at = ws.documents[&uri]
            .text
            .position(source.rfind("target").unwrap() + 1);
        assert!(features::definitions(&ws, &uri, at).is_empty(), "{source}");
    }
}

#[test]
fn section_command_and_environment_completion_preserves_existing_headers() {
    for name in [
        "newrefsection",
        "newrefsegment",
        "endrefsection",
        "endrefsegment",
    ] {
        let source = format!(r"\{name}");
        let (ws, uri) = setup(&source);
        let at = ws.documents[&uri].text.position(source.len());
        let item = features::completion(&ws, &uri, at)
            .into_iter()
            .find(|item| item.label == name)
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, name);
    }
    let (ws, uri) = setup(r"\begin{ref}");
    let items = features::completion(&ws, &uri, Position::new(0, 10));
    for name in ["refsection", "refsegment"] {
        assert!(items.iter().any(|item| item.label == name));
    }
}

#[test]
fn raw_resource_percent_is_literal_but_precollected_resources_keep_comments() {
    for call in [
        r"\newrefsection",
        r"\refsection",
        r"\begin{refsection}",
        r"\csuse{newrefsection}",
        r"\csname newrefsection\endcsname",
        r"\csuse{begin}{refsection}",
        r"\csname begin\endcsname{refsection}",
        r"\sectionboundary",
    ] {
        let source = format!(
            r"\def\sectionboundary{{\newrefsection}}\label{{target}}{call}[ref%name.bib]\ref{{target}}"
        );
        let (ws, uri) = setup(&source);
        assert!(ws.documents[&uri].syntax.comments.is_empty(), "{source}");
        let at = ws.documents[&uri]
            .text
            .position(source.rfind("target").unwrap() + 1);
        assert_eq!(features::definitions(&ws, &uri, at).len(), 1, "{source}");
        let edits = features::rename(&ws, &uri, at, "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()
            .remove(&uri)
            .unwrap();
        assert_eq!(
            apply(&source, edits),
            source.replace("target", "updated"),
            "{source}"
        );
        let at = ws.documents[&uri]
            .text
            .position(source.find("%name").unwrap() + 3);
        assert!(signature::help(&ws, &uri, at).is_some(), "{source}");
        let at = ws.documents[&uri]
            .text
            .position(source.find(r"]\ref").unwrap() + 1);
        assert!(signature::help(&ws, &uri, at).is_none(), "{source}");
    }
    for prefix in [
        r"\textbf{",
        r"\newcommand\take[1]{#1}\take{",
        r"\newcommand\deferred{",
    ] {
        let source = format!(
            "{prefix}\\newrefsection[ref% comment\r\nname.bib]}}\\label{{target}}\\ref{{target}}"
        );
        let (ws, uri) = setup(&source);
        assert_eq!(ws.documents[&uri].syntax.comments.len(), 1, "{source}");
        let at = ws.documents[&uri]
            .text
            .position(source.rfind("target").unwrap() + 1);
        assert_eq!(features::definitions(&ws, &uri, at).len(), 1, "{source}");
    }
}

#[test]
#[ignore = "requires pdflatex and biblatex"]
fn real_section_resource_percent_and_reference_renames_preserve_execution() {
    for (call, close) in [
        (r"\newrefsection", ""),
        (r"\begin{refsection}", r"\end{refsection}"),
        (r"\csuse{newrefsection}", ""),
        (r"\csname newrefsection\endcsname", ""),
        (r"\sectionboundary", ""),
    ] {
        let source = format!(
            r"\documentclass{{article}}\usepackage[backend=bibtex]{{biblatex}}\def\sectionboundary{{\newrefsection}}\begin{{document}}\section{{Title}}\label{{target}}{call}[ref%name.bib]\ref{{target}}\typeout{{SECTION-AFTER}}{close}\end{{document}}"
        );
        let (ws, uri) = setup(&source);
        let at = ws.documents[&uri]
            .text
            .position(source.rfind("target").unwrap() + 1);
        let edits = features::rename(&ws, &uri, at, "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()
            .remove(&uri)
            .unwrap();
        let updated = apply(&source, edits);
        assert_eq!(updated, source.replace("target", "updated"));
        let dir = tempfile::tempdir().unwrap();
        for text in [&source, &updated] {
            std::fs::write(dir.path().join("main.tex"), text).unwrap();
            for pass in 0..2 {
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
                let log = String::from_utf8_lossy(&output.stdout);
                assert!(output.status.success(), "{call}: {log}");
                assert!(log.contains("SECTION-AFTER"), "{call}: {log}");
                if pass == 1 {
                    assert!(!log.contains("Reference `updated'"), "{log}");
                }
            }
        }
    }
}
