use latex_lsp::{features, workspace::Workspace};
use lsp_types::*;

fn setup(preamble: &str, body: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:common-parameter-grammars.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), format!("% 😀\r\n\\documentclass{{article}}\\newcommand\\unicodeword{{😀}}{preamble}\\begin{{document}}{body}\\end{{document}}"), 5);
    (workspace, uri)
}

fn cases() -> [(&'static str, &'static str); 13] {
    [
        (
            r"\newcommand\one[1]{#1}\renewcommand\one[1]{#1}",
            r"\one{Text}",
        ),
        (
            r"\newcommand\one[2][Default]{#2}\renewcommand\one[2][Other]{#2}",
            r"\one[Note]{Text}",
        ),
        (
            r"\newcommand\one[2][Default]{#2}\renewcommand\one[2][Other]{#2}",
            r"\one{Text}",
        ),
        (r"\def\one#1;{#1}\def\one#1;{#1}", r"\one Text;"),
        (
            r"\def\one#1;{#1}\def\one#1;{}\def\bridge{\one}",
            r"\bridge Text;",
        ),
        (
            r"\NewDocumentCommand\one{m}{#1}\RenewDocumentCommand\one{m}{#1}",
            r"\one{Text}",
        ),
        (
            r"\NewDocumentCommand\one{s O{Default} m}{#3}\RenewDocumentCommand\one{s O{Other} m}{#3}",
            r"\one*[Note]{Text}",
        ),
        (
            r"\newcommand\one[1]{#1}\renewcommand\one[1]{#1}",
            r"\csuse{one}{Text}",
        ),
        (
            r"\newcommand\one[1]{#1}\renewcommand\one[1]{#1}",
            r"\csname one\endcsname{Text}",
        ),
        (
            r"\newcommand\one[1]{#1}\renewcommand\one[1]{}\def\bridge{\one}",
            r"\bridge{Text}",
        ),
        (
            r"\newcommand\one[1]{#1}\renewcommand\one[1]{}\def\bridge{\one}\def\forward{\bridge}",
            r"\csuse{forward}{Text}",
        ),
        (r"\newcommand\one{}\renewcommand\one{Text}", r"\one"),
        (
            r"\NewDocumentCommand\one{d() m}{#2}\RenewDocumentCommand\one{d() m}{#2}",
            r"\one(Note){Text}",
        ),
    ]
}

#[test]
fn unsaved_provider_changes_only_recover_boundaries_shared_by_every_declaration() {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = uri.join("commands.sty").unwrap();
    let source = "% 😀\r\n\\documentclass{article}\\newcommand\\unicodeword{😀}\\newcommand\\one[1]{#1}\\input{commands.sty}\\begin{document}\\label{same}\\one{Text}\\ref{same}\\end{document}";
    let initial = r"\renewcommand\one[1]{#1}";
    std::fs::write(provider.to_file_path().unwrap(), initial).unwrap();
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 5);
    let check = |workspace: &Workspace, active: bool| {
        let text = &workspace.documents[&uri].text;
        let at = text.position(source.rfind("same").unwrap() + 1);
        assert_eq!(
            features::definitions(workspace, &uri, at).len(),
            usize::from(active)
        );
        assert_eq!(
            matches!(features::rename(workspace, &uri, at, "fresh"), Ok(Some(_))),
            active
        );
        assert_eq!(workspace.documents[&uri].version, Some(5));
    };
    check(&workspace, true);
    workspace.open(provider.clone(), r"\renewcommand\one[2]{#1}".into(), 12);
    check(&workspace, false);
    workspace.open(provider, r"\renewcommand\one[1]{}".into(), 13);
    check(&workspace, true);
}

#[test]
fn common_parameters_keep_incomplete_and_nested_reference_data_opaque() {
    for body in [
        r"\label{same}\one{\ref{same}}",
        r"\label{same}\one{\ref{same}",
    ] {
        let (workspace, uri) = setup(r"\newcommand\one[1]{#1}\renewcommand\one[1]{}", body);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        assert!(features::definitions(&workspace, &uri, at).is_empty());
        assert!(!matches!(
            features::rename(&workspace, &uri, at, "fresh"),
            Ok(Some(_))
        ));
    }
    let (workspace, uri) = setup(
        r"\def\one#1;{#1}\def\one#1;{}",
        r"\label{same}\one X\ref{same}",
    );
    let text = &workspace.documents[&uri].text;
    let at = text.position(text.source.rfind("same").unwrap() + 1);
    assert!(features::definitions(&workspace, &uri, at).is_empty());
    assert!(!matches!(
        features::rename(&workspace, &uri, at, "fresh"),
        Ok(Some(_))
    ));
}

#[test]
fn repeated_parameter_grammars_preserve_independent_reference_navigation_and_edits() {
    for (preamble, call) in cases() {
        let (workspace, uri) = setup(preamble, &format!(r"\label{{same}}{call}\ref{{same}}"));
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        let start = text.source.find("same").unwrap();
        assert_eq!(
            features::definitions(&workspace, &uri, at),
            vec![Location::new(uri.clone(), text.range(start, start + 4))],
            "{preamble}: {call}"
        );
        let changes = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes[&uri].len(), 2, "{preamble}: {call}");
        assert!(
            latex_lsp::signature::help(
                &workspace,
                &uri,
                text.position(text.source.rfind(call).unwrap() + call.len() - 1)
            )
            .is_none(),
            "{preamble}: {call}"
        );
    }
}

#[test]
fn shared_shape_does_not_certify_the_replacement_body_or_parameter_execution() {
    let (workspace, uri) = setup(
        r"\newcommand\one[1]{#1}\renewcommand\one[1]{#1}",
        r"\one{\label{same}}\ref{same}",
    );
    let text = &workspace.documents[&uri].text;
    let at = text.position(text.source.rfind("same").unwrap() + 1);
    assert_eq!(features::definitions(&workspace, &uri, at).len(), 1);
    assert!(!matches!(
        features::rename(&workspace, &uri, at, "fresh"),
        Ok(Some(_))
    ));
}

#[test]
fn differing_or_unknown_grammars_keep_following_control_tokens_protected() {
    for preamble in [
        r"\newcommand\one[1]{#1}\renewcommand\one[2]{#1}",
        r"\newcommand\one[1]{#1}\let\one\unknown",
        r"\def\one#1;{#1}\def\one#1:{#1}",
    ] {
        let (workspace, uri) = setup(preamble, r"\label{same}\one{Text}\ref{same}");
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{preamble}"
        );
        assert!(!matches!(
            features::rename(&workspace, &uri, at, "fresh"),
            Ok(Some(_))
        ));
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_independent_references_keep_complete_aux_after_real_lsp_edits() {
    for (preamble, call) in cases() {
        let directory = tempfile::tempdir().unwrap();
        let (workspace, uri) = setup(
            &format!(r"\usepackage{{etoolbox}}{preamble}"),
            &format!(r"\section{{Heading}}\label{{same}}{call}\ref{{same}}"),
        );
        let text = &workspace.documents[&uri].text;
        let compile = |source: &str| {
            std::fs::write(directory.path().join("main.tex"), source).unwrap();
            let aux = directory.path().join("main.aux");
            if aux.exists() {
                std::fs::remove_file(&aux).unwrap();
            }
            for _ in 0..2 {
                let output = std::process::Command::new("pdflatex")
                    .args([
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ])
                    .current_dir(directory.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{preamble}/{call}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            assert!(!log.contains("undefined"), "{log}");
            std::fs::read_to_string(aux).unwrap()
        };
        let before = compile(&text.source);
        assert!(before.contains(r"\newlabel{same}"));
        let at = text.position(text.source.rfind("same").unwrap() + 1);
        let mut edits = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&uri]
            .clone();
        assert_eq!(edits.len(), 2);
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let changed = text
            .clone()
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
        assert_eq!(changed.source, text.source.replace("{same}", "{fresh}"));
        let after = compile(&changed.source);
        assert_eq!(after.replace("{fresh}", "{same}"), before);
    }
}
