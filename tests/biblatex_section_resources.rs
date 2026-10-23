use latex_lsp::{
    features, file_operations, navigation, signature, text::Text, workspace::Workspace,
};
use tower_lsp::lsp_types::*;

fn setup(source: &str, filename: &str) -> (tempfile::TempDir, Workspace, Url, Url) {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in [
        ("main.tex", source),
        (
            filename,
            "@book{selected,title={Selected}}\n\\newcommand\\leaked{}",
        ),
        ("default.bib", "@book{paper,title={Default}}"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let bib = Url::from_file_path(dir.path().join(filename)).unwrap();
    (dir, ws, main, bib)
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
fn section_registrations_offer_file_navigation_and_moves_without_loading_entries_or_source() {
    for call in [
        r"\addsectionbib",
        r"\csuse{addsectionbib}",
        r"\csname addsectionbib\endcsname",
        r"\register",
        r"\csuse{register}",
        r"\csname register\endcsname",
    ] {
        let source = format!(
            r"\documentclass{{article}}\def\register{{\addsectionbib}}\addbibresource{{default.bib}}{call}[label=chosen]{{section.bib}}\cite{{paper}}\leaked"
        );
        let (dir, ws, main, bib) = setup(&source, "section.bib");
        assert!(!ws.dependencies(&main).contains(&bib), "{call}");
        assert!(!ws.project_files(&main).contains(&bib), "{call}");
        let links: Vec<_> = navigation::document_links(&ws, &main)
            .into_iter()
            .filter(|link| link.target.as_ref() == Some(&bib))
            .collect();
        assert_eq!(links.len(), 1, "{call}");
        let text = Text::new(source.clone());
        assert_eq!(
            links[0].range,
            text.range(
                source.find("section.bib").unwrap(),
                source.find("section.bib").unwrap() + "section.bib".len()
            )
        );
        let at = text.position(source.find("section.bib").unwrap() + 3);
        assert_eq!(features::definitions(&ws, &main, at)[0].uri, bib, "{call}");
        let paper = text.position(source.find("paper").unwrap() + 1);
        assert_eq!(features::definitions(&ws, &main, paper).len(), 1, "{call}");
        assert!(
            features::definitions(
                &ws,
                &main,
                text.position(source.rfind("leaked").unwrap() + 2)
            )
            .is_empty(),
            "{call}"
        );
        let renamed = Url::from_file_path(dir.path().join("new%data.bib")).unwrap();
        let edit = file_operations::will_rename(
            &ws,
            &[FileRename {
                old_uri: bib.to_string(),
                new_uri: renamed.to_string(),
            }],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            apply(&source, edit.changes.unwrap().remove(&main).unwrap()),
            source.replace("section.bib", "new%data.bib"),
            "{call}"
        );
    }
}

#[test]
fn registration_completion_preserves_labels_literal_filenames_suffixes_and_utf16_ranges() {
    for (call, filename) in [
        (r"\addsectionbib", "refs%中文.bib"),
        (r"\addsectionbib", "refs#&$^~.bib"),
        (r"\csuse{addsectionbib}", "refs,part.bib"),
        (r"\register", "refs file.bib"),
    ] {
        for tail in ["BAD}", ""] {
            let marked = format!("% 😀\r\n\\documentclass{{article}}\\def\\register{{\\addsectionbib}}{call}[label=chosen]{{ref¦{tail}");
            let source = marked.replace('¦', "");
            let (_dir, ws, main, _) = setup(&source, filename);
            let at = Text::new(source.clone()).position(marked.find('¦').unwrap());
            let items = features::completion(&ws, &main, at);
            let item = items
                .into_iter()
                .find(|item| item.label == filename)
                .unwrap_or_else(|| panic!("{marked}"));
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!()
            };
            assert_eq!(
                apply(&source, vec![edit]),
                marked.replace(
                    &format!("ref¦{tail}"),
                    &format!("{filename}{}", if tail.is_empty() { "" } else { "}" })
                ),
                "{marked}"
            );
            let help = signature::help(&ws, &main, at).unwrap();
            assert_eq!(help.active_parameter, Some(1), "{marked}");
        }
    }
}

#[test]
fn raw_direct_registration_percent_does_not_hide_following_source_and_exact_names_keep_their_suffix(
) {
    let source =
        r"\documentclass{article}\addsectionbib[label=chosen]{refs%data.bib}\label{kept}\ref{kept}";
    let (_dir, ws, main, bib) = setup(source, "refs%data.bib");
    assert!(ws.documents[&main].syntax.comments.is_empty());
    assert_eq!(
        navigation::document_links(&ws, &main)[0].target.as_ref(),
        Some(&bib)
    );
    let at = Text::new(source.into()).position(source.rfind("kept").unwrap() + 1);
    assert_eq!(
        features::rename(&ws, &main, at, "updated")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&main]
            .len(),
        2
    );
    let source = r"\documentclass{article}\addsectionbib{refs}";
    let (_dir, ws, main, _bib) = setup(source, "refs.bib");
    assert!(navigation::document_links(&ws, &main).is_empty());
}

#[test]
fn registration_data_overrides_and_unsupported_resource_options_do_not_offer_file_edits() {
    for call in [
        r"\newcommand\discard[1]{}\discard{\addsectionbib{section.bib}}",
        r"\newcommand\addsectionbib[1]{}\addsectionbib{section.bib}",
        r"\let\register\addsectionbib\register{section.bib}",
        r"\addsectionbib[location=remote]{section.bib}",
        r"\addsectionbib*{section.bib}",
        r"\verb|\addsectionbib{section.bib}|",
    ] {
        let source = format!(r"\documentclass{{article}}{call}");
        let (_dir, ws, main, _) = setup(&source, "section.bib");
        assert!(
            navigation::document_links(&ws, &main).is_empty(),
            "{source}"
        );
        let at = Text::new(source.clone()).position(source.find("section.bib").unwrap() + 3);
        assert!(
            !features::completion(&ws, &main, at)
                .iter()
                .any(|item| item.label == "section.bib"),
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex, biblatex and Biber"]
fn real_registered_resource_moves_preserve_explicit_section_selection() {
    let source = r"\documentclass{article}\usepackage{biblatex}\addsectionbib[label=chosen]{section.bib}\begin{document}\newrefsection[chosen]\nocite{selected}\printbibliography\end{document}";
    let (dir, ws, main, bib) = setup(source, "section.bib");
    std::fs::write(
        bib.to_file_path().unwrap(),
        "@book{selected,author={Doe, Jane},title={Selected},year={2026}}",
    )
    .unwrap();
    let renamed = Url::from_file_path(dir.path().join("selected%renamed.bib")).unwrap();
    let edits = file_operations::will_rename(
        &ws,
        &[FileRename {
            old_uri: bib.to_string(),
            new_uri: renamed.to_string(),
        }],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap()
    .remove(&main)
    .unwrap();
    let updated = apply(source, edits);
    assert_eq!(
        updated,
        source.replace("section.bib", "selected%renamed.bib")
    );
    for (text, moved) in [(source, false), (updated.as_str(), true)] {
        if moved {
            std::fs::rename(bib.to_file_path().unwrap(), renamed.to_file_path().unwrap()).unwrap();
        }
        std::fs::write(main.to_file_path().unwrap(), text).unwrap();
        let latex = || {
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
        latex();
        latex();
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        assert!(!log.contains("undefined"), "{log}");
        let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
        assert!(bbl.contains("Selected"), "{bbl}");
        let bcf = std::fs::read_to_string(dir.path().join("main.bcf")).unwrap();
        assert!(
            bcf.contains(if moved {
                ">selected%renamed.bib<"
            } else {
                ">section.bib<"
            }),
            "{bcf}"
        );
    }
}
