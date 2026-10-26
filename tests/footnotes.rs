use lsp_types::*;
use texquill::{
    language::{features, signature},
    project::workspace::Workspace,
    text::Text,
};

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url) {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let source =
        format!("% 😀\r\n\\documentclass{{article}}\\begin{{document}}{body}\\end{{document}}");
    std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source, 5);
    (directory, workspace, uri)
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
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
fn standard_footnote_signatures_follow_optional_numbers_and_unfinished_text() {
    for (call, label, active) in [
        (r"\footnotemark[7¦]", r"\footnotemark[number]", 0),
        (r"\footnotemark[7¦", r"\footnotemark[number]", 0),
        (
            r"\footnotetext[7¦]{Note}",
            r"\footnotetext[number]{text}",
            0,
        ),
        (r"\footnotetext{No¦te}", r"\footnotetext[number]{text}", 1),
        (
            r"\footnotetext[7]{No¦te}",
            r"\footnotetext[number]{text}",
            1,
        ),
        (r"\footnotetext[7]{No¦", r"\footnotetext[number]{text}", 1),
        (
            r"\csuse{footnotetext}[7]{No¦te}",
            r"\csuse{footnotetext}[number]{text}",
            1,
        ),
        (
            r"\csname footnotemark\endcsname[7¦]",
            r"\csname footnotemark\endcsname[number]",
            0,
        ),
        (
            r"\def\note{\footnotetext}\note[7]{No¦te}",
            r"\note[number]{text}",
            1,
        ),
        (r"\def\mark{\footnotemark}\mark[7¦]", r"\mark[number]", 0),
        (r"\footnotemark[1[2] {Ne¦xt}", r"", 0),
        (
            r"\footnotetext[1[2]{No¦te}",
            r"\footnotetext[number]{text}",
            1,
        ),
    ] {
        let (_directory, workspace, uri) = setup(call);
        let source = &workspace.documents[&uri].text.source;
        let offset = source.find('¦').unwrap();
        // Keep an unfinished call genuinely unfinished instead of appending the document end.
        let source = if call.ends_with('¦') {
            source[..offset].to_owned()
        } else {
            source.replace('¦', "")
        };
        let mut workspace = workspace;
        workspace.open(uri.clone(), source, 6);
        let help = signature::help(
            &workspace,
            &uri,
            workspace.documents[&uri].text.position(offset),
        );
        if label.is_empty() {
            assert!(help.is_none(), "{call}: {help:?}");
        } else {
            let help = help.unwrap_or_else(|| panic!("{call}"));
            assert_eq!(help.signatures[0].label, label, "{call}");
            assert_eq!(help.active_parameter, Some(active), "{call}");
        }
    }
}

#[test]
fn footnote_completions_negotiate_snippets_and_preserve_existing_arguments() {
    for snippets in [false, true] {
        for (name, tail) in [
            ("footnotemark", ""),
            ("footnotemark", "[7]"),
            ("footnotetext", ""),
            ("footnotetext", "[7]{Existing}"),
        ] {
            let (_directory, mut workspace, uri) = setup(&format!(r"\{name}{tail}"));
            workspace.client.snippets = snippets;
            let text = &workspace.documents[&uri].text;
            let offset = text.source.find(name).unwrap() + name.len();
            let item = features::completion(&workspace, &uri, text.position(offset))
                .into_iter()
                .find(|item| item.label == name)
                .unwrap();
            assert!(item.documentation.is_some());
            assert_eq!(item.detail, Some(format!(r"\{name}")));
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!("missing edit")
            };
            assert_eq!(edit.range, text.range(offset - name.len(), offset));
            let expected = if snippets && tail.is_empty() && name == "footnotetext" {
                "footnotetext{${1:text}}"
            } else {
                name
            };
            assert_eq!(edit.new_text, expected, "{snippets}: {name}{tail}");
            assert!(apply(&text.source, &[edit]).contains(tail));
        }
    }
}

#[test]
fn footnote_text_and_the_independent_group_after_a_marker_keep_exact_label_edits() {
    for (preamble, marker, note) in [
        ("", r"\footnotemark", r"\footnotetext"),
        ("", r"\csuse{footnotemark}", r"\csuse{footnotetext}"),
        (
            "",
            r"\csname footnotemark\endcsname",
            r"\csname footnotetext\endcsname",
        ),
        (
            r"\def\mark{\footnotemark}\def\note{\footnotetext}",
            r"\mark",
            r"\note",
        ),
    ] {
        for number in ["", "[7]"] {
            let body = format!(
                r"{preamble}\section{{Heading}}\label{{safe}}{marker}{number}{note}{number}{{Note \ref{{safe}}}}{marker}{{\label{{after}}}}\ref{{after}}"
            );
            let (_directory, workspace, uri) = setup(&body);
            let text = &workspace.documents[&uri].text;
            for name in ["safe", "after"] {
                let query = text.source.find(&format!(r"\ref{{{name}}}")).unwrap() + 5;
                let declaration = text.source.find(&format!(r"\label{{{name}}}")).unwrap() + 7;
                assert_eq!(
                    features::definitions(&workspace, &uri, text.position(query)),
                    vec![Location::new(
                        uri.clone(),
                        text.range(declaration, declaration + name.len())
                    )],
                    "{marker}{number}: {name}"
                );
                assert_eq!(
                    features::references(&workspace, &uri, text.position(query), true).len(),
                    2
                );
                let changes = features::rename(&workspace, &uri, text.position(query), "fresh")
                    .unwrap()
                    .unwrap()
                    .changes
                    .unwrap();
                assert_eq!(changes.len(), 1);
                assert_eq!(changes[&uri].len(), 2);
                assert_eq!(
                    apply(&text.source, &changes[&uri]),
                    text.source.replace(&format!("{{{name}}}"), "{fresh}")
                );
            }
        }
    }
}

#[test]
fn footnotes_load_sources_from_text_and_from_independent_marker_groups() {
    for command in ["footnotemark", "footnotetext"] {
        for number in ["", "[7]"] {
            let (_directory, mut workspace, uri) = setup(&format!(
                r"\{command}{number}{{\input{{labels.tex}}}}\ref{{inside}}"
            ));
            let child = uri.join("labels.tex").unwrap();
            let labels = "% 😀\r\n\\label{inside}";
            std::fs::write(child.to_file_path().unwrap(), labels).unwrap();
            workspace.refresh_project(&uri);
            let text = &workspace.documents[&uri].text;
            let at = text.source.find(r"\ref{inside}").unwrap() + 5;
            let label = labels.find("inside").unwrap();
            assert_eq!(
                features::definitions(&workspace, &uri, text.position(at)),
                vec![Location::new(
                    child.clone(),
                    Text::new(labels.into()).range(label, label + 6)
                )],
                "{command}{number}"
            );
            let changes = features::rename(&workspace, &uri, text.position(at), "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 2);
            assert_eq!(changes[&uri].len(), 1);
            assert_eq!(changes[&child].len(), 1);
            assert_eq!(
                apply(labels, &changes[&child]),
                labels.replace("{inside}", "{fresh}")
            );
        }
    }
}

#[test]
fn source_footnote_overrides_take_precedence_and_removal_restores_native_grammar() {
    let (_directory, mut workspace, uri) = setup(r"\footnotemark{\label{after}}\ref{after}");
    let provider = uri.join("commands.sty").unwrap();
    std::fs::write(provider.to_file_path().unwrap(), "").unwrap();
    let original = workspace.documents[&uri].text.source.clone();
    let source = original.replace(r"\begin{document}", r"\input{commands.sty}\begin{document}");
    workspace.open(uri.clone(), source.clone(), 6);
    for (name, declaration, call) in [
        (
            "footnotemark",
            r"\renewcommand\footnotemark[1]{}",
            r"\footnotemark{Te¦xt}",
        ),
        (
            "footnotetext",
            r"\renewcommand\footnotetext[2]{}",
            r"\footnotetext{First}{Sec¦ond}",
        ),
    ] {
        workspace.open(provider.clone(), declaration.into(), 10);
        let marked = source.replace(r"\end{document}", &format!(r"{call}\end{{document}}"));
        let offset = marked.find('¦').unwrap();
        workspace.open(uri.clone(), marked.replace('¦', ""), 7);
        let help = signature::help(
            &workspace,
            &uri,
            workspace.documents[&uri].text.position(offset),
        )
        .unwrap_or_else(|| panic!("missing override signature: {name}: {marked}"));
        assert_eq!(
            help.signatures[0].label,
            if name == "footnotemark" {
                r"\footnotemark{#1}"
            } else {
                r"\footnotetext{#1}{#2}"
            }
        );
        workspace.open(provider.clone(), String::new(), 11);
    }
    workspace.open(uri.clone(), original, 8);
    let text = &workspace.documents[&uri].text;
    let query = text.source.find(r"\ref{after}").unwrap() + 5;
    assert_eq!(
        features::definitions(&workspace, &uri, text.position(query)).len(),
        1
    );
}

#[test]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn native_footnote_label_renames_preserve_complete_auxiliary_data() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for number in ["", "[7]"] {
            let body = format!(
                r"\section{{Heading}}\label{{safe}}\footnotemark{number}\footnotetext{number}{{Note \ref{{safe}}}}\footnotemark{{\label{{after}}}}\ref{{after}}"
            );
            let (directory, workspace, uri) = setup(&body);
            let cache = directory.path().join("tex-cache");
            std::fs::create_dir(&cache).unwrap();
            let compile = || {
                let aux = directory.path().join("main.aux");
                if aux.exists() {
                    std::fs::remove_file(&aux).unwrap();
                }
                for _ in 0..2 {
                    let output = std::process::Command::new(engine)
                        .args([
                            "-no-shell-escape",
                            "-interaction=nonstopmode",
                            "-halt-on-error",
                            "main.tex",
                        ])
                        .current_dir(directory.path())
                        .env("TEXMFVAR", &cache)
                        .env("TEXMFCACHE", &cache)
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "{engine}: {}",
                        String::from_utf8_lossy(&output.stdout)
                    );
                }
                let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
                assert!(
                    !log.contains("Warning") && !log.contains("undefined"),
                    "{engine}: {log}"
                );
                std::fs::read_to_string(aux).unwrap()
            };
            let before = compile();
            let text = &workspace.documents[&uri].text;
            let query = text.source.find(r"\ref{after}").unwrap() + 5;
            let changes = features::rename(&workspace, &uri, text.position(query), "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes[&uri].len(), 2);
            std::fs::write(
                uri.to_file_path().unwrap(),
                apply(&text.source, &changes[&uri]),
            )
            .unwrap();
            let after = compile();
            assert!(before.contains(r"\newlabel{after}") && after.contains(r"\newlabel{fresh}"));
            assert_eq!(
                before,
                after.replace("{fresh}", "{after}"),
                "{engine}{number}"
            );
        }
    }
}
