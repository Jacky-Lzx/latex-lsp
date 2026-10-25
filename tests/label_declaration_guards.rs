use latex_lsp::{features, text::Text, workspace::Workspace};
use lsp_types::*;

fn setup(body: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:label-declaration-guards.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), format!("% 😀\r\n{body}"), 5);
    (workspace, uri)
}

fn contexts() -> [&'static str; 8] {
    [
        r"\newcommand\discard[1]{}\discard{\label{ghost}}",
        r"\newcommand\literal[1]{\detokenize{#1}}\literal{\label{ghost}}",
        r"\renewcommand\footnotemark[1]{\detokenize{#1}}\footnotemark{\label{ghost}}",
        r"\newcommand\literal[1]{\detokenize{#1}}\def\show{\literal}\show{\label{ghost}}",
        r"\newcommand\literal[1]{\detokenize{#1}}\csuse{literal}{\label{ghost}}",
        r"\newcommand\literal[1]{\detokenize{#1}}\csname literal\endcsname{\label{ghost}}",
        r"\def\ifblank#1#2#3{#2}\newcommand\literal[1]{\detokenize{#1}}\literal{\label{ghost}}",
        r"\newcommand\literal[1]{\detokenize{#1}}\literal\label{ghost}",
    ]
}

#[test]
fn labels_inside_parameter_data_never_offer_destructive_key_edits() {
    for context in contexts() {
        let (workspace, uri) = setup(context);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.find("ghost").unwrap() + 1);
        assert!(
            !matches!(features::prepare_rename(&workspace, &uri, at), Ok(Some(_))),
            "{context}"
        );
        assert!(
            !matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
            "{context}"
        );
    }
}

#[test]
fn unfinished_parameter_data_does_not_certify_nested_label_edits() {
    for body in [
        r"\newcommand\literal[1]{\detokenize{#1}}\literal{\label{ghost}",
        r"\def\ifblank#1#2#3{#2}\newcommand\literal[1]{\detokenize{#1}}\literal{\label{ghost}",
        r"\newcommand\literal[2]{\detokenize{#2}}\literal{Text}{\label{ghost}",
        r"\newcommand\literal[1]{\detokenize{#1}}\csuse{literal}{\label{ghost}",
    ] {
        let (workspace, uri) = setup(body);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.find("ghost").unwrap() + 1);
        assert!(
            !matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
            "{body}"
        );
    }
}

#[test]
fn contextual_string_shapes_do_not_certify_user_forwarded_arguments_as_labels() {
    for call in [
        r"\show{\label{ghost}}",
        r"\csuse{show}{\label{ghost}}",
        r"\csname show\endcsname{\label{ghost}}",
    ] {
        let source = format!(
            r"\def\ifblank#1#2#3{{#2}}\newcommand\literal[1]{{\detokenize{{#1}}}}\def\show{{\literal}}{call}"
        );
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.find("ghost").unwrap() + 1);
        assert!(
            !matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
            "{call}"
        );
    }
}

#[test]
fn data_declarations_keep_unrelated_label_renames_complete() {
    for context in contexts() {
        let source = format!(r"{context}\label{{safe}}\ref{{safe}}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("safe").unwrap() + 1);
        let changes = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        let mut edits = changes[&uri].clone();
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
        assert_eq!(changed.source, text.source.replace("{safe}", "{fresh}"));
    }
}

#[test]
fn commented_unicode_and_replaced_declarations_keep_the_same_guard() {
    for body in [
        "\\newcommand\\discard[1]{}\\discard{\\label{% before\r\n章:ghost% after\r\n}}",
        r"\renewcommand\label[1]{#1}\label{ghost}",
        r"\newcommand\discard[1]{}\discard{\hyperdef[ghost]{category}{destination}{Text}}",
    ] {
        let (workspace, uri) = setup(body);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.find("ghost").unwrap() + 1);
        assert!(
            !matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
            "{body}"
        );
    }
}

#[test]
fn deferred_definition_bodies_and_native_text_arguments_keep_label_edits() {
    for body in [
        r"\newcommand\later{\label{ghost}}",
        r"\textbf{\label{ghost}}",
        r"\footnotemark{\label{ghost}}",
        r"\footnotetext{\label{ghost}Text}",
        r"\let\saved\footnotemark\renewcommand\footnotemark[1]{}\saved[7]\label{ghost}",
        r"\newcommand\one[1]{#1}\one{\label{ghost}}",
        r"\def\one#1;{#1}\one\label{ghost};",
        r"\newcommand\one[1]{#1}\def\forward{\one}\forward{\label{ghost}}",
        r"\newcommand\one[1]{#1}\csuse{one}{\label{ghost}}",
        r"\newcommand\one[1]{#1}\csname one\endcsname{\label{ghost}}",
    ] {
        let (workspace, uri) = setup(body);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.find("ghost").unwrap() + 1);
        let changes = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(changes[&uri].len(), 1, "{body}");
        assert_eq!(changes[&uri][0].new_text, "fresh");
    }
}

#[test]
fn identity_arguments_need_a_unique_body_and_an_executable_outer_context() {
    for body in [
        r"\newcommand\one[1]{#1}\renewcommand\one[1]{}\one{\label{ghost}}",
        r"\newcommand\one[1]{#1}\newcommand\discard[1]{}\discard{\one{\label{ghost}}}",
        r"\newcommand\one[1]{#1}\detokenize{\one{\label{ghost}}}",
        r"\def\one#1\label{#1}\one Text\label{ghost}",
    ] {
        let (workspace, uri) = setup(body);
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.find("ghost").unwrap() + 1);
        assert!(
            !matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
            "{body}"
        );
    }
}

#[test]
fn unsaved_cross_file_arity_changes_restore_only_proven_independent_groups() {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = uri.join("commands.sty").unwrap();
    let declaration = r"\newcommand\discard[1]{}";
    std::fs::write(provider.to_file_path().unwrap(), declaration).unwrap();
    let source = "% 😀\r\n\\documentclass{article}\\input{commands.sty}\\begin{document}\\discard{\\label{ghost}}\\end{document}";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 5);
    let at = Text::new(source.into()).position(source.find("ghost").unwrap() + 1);
    for (version, body, allowed) in [
        (12, declaration, false),
        (13, r"\newcommand\discard{}", true),
        (14, declaration, false),
        (15, r"\newcommand\discard[1]{#1}", true),
        (16, r"\newcommand\discard[1]{\detokenize{#1}}", false),
    ] {
        workspace.open(provider.clone(), body.into(), version);
        assert_eq!(
            matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
            allowed,
            "{body}"
        );
    }
}

#[test]
fn shared_declarations_require_the_same_argument_meaning_in_every_consumer() {
    let directory = tempfile::tempdir().unwrap();
    let shared = Url::from_file_path(directory.path().join("shared.tex")).unwrap();
    std::fs::write(shared.to_file_path().unwrap(), r"\discard{\label{ghost}}").unwrap();
    for (name, arity) in [("a.tex", ""), ("b.tex", "[1]")] {
        std::fs::write(directory.path().join(name), format!(r"\documentclass{{article}}\newcommand\discard{arity}{{}}\begin{{document}}\input{{shared}}\end{{document}}")).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.add_root(directory.path().into());
    let at = Position::new(0, 18);
    assert!(!matches!(
        features::rename(&workspace, &shared, at, "fresh"),
        Ok(Some(_))
    ));
    let second = shared.join("b.tex").unwrap();
    workspace.open(
        second,
        r"\documentclass{article}\newcommand\discard{}\begin{document}\input{shared}\end{document}"
            .into(),
        12,
    );
    let changes = features::rename(&workspace, &shared, at, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&shared].len(), 1);
}

#[test]
#[ignore = "requires pdflatex"]
fn native_displayed_declaration_keys_stay_literal_and_receive_no_lsp_edits() {
    for declaration in [
        r"\newcommand\literal[1]{\typeout{LSP-LITERAL:\detokenize{#1}}\texttt{\detokenize{#1}}}",
        r"\renewcommand\footnotemark[1]{\typeout{LSP-LITERAL:\detokenize{#1}}\texttt{\detokenize{#1}}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let call = if declaration.contains("footnotemark") {
            "footnotemark"
        } else {
            "literal"
        };
        let source = format!("% 😀\r\n\\documentclass{{article}}{declaration}\\begin{{document}}\\section{{Heading}}\\{call}{{\\label{{ghost}}}}\\label{{safe}}\\ref{{safe}}\\end{{document}}");
        let compile = |source: &str| {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
            let auxiliary = directory.path().join("main.aux");
            if auxiliary.exists() {
                std::fs::remove_file(&auxiliary).unwrap();
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
                    "{}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            assert!(
                !log.contains("Warning") && !log.contains("undefined"),
                "{log}"
            );
            let literal = log
                .lines()
                .filter(|line| line.starts_with("LSP-LITERAL:"))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            (std::fs::read_to_string(auxiliary).unwrap(), literal)
        };
        let (before, literal) = compile(&source);
        assert!(!before.contains(r"\newlabel{ghost}"));
        assert_eq!(literal, [r"LSP-LITERAL:\label {ghost}"]);
        let (legacy_auxiliary, legacy_literal) = compile(&source.replace("{ghost}", "{fresh}"));
        assert_eq!(before, legacy_auxiliary);
        assert_eq!(legacy_literal, [r"LSP-LITERAL:\label {fresh}"]);
        std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
        let mut workspace = Workspace::default();
        workspace.config.distribution.enabled = false;
        workspace.open(uri.clone(), source.clone(), 5);
        let at = Text::new(source)
            .position(workspace.documents[&uri].text.source.find("ghost").unwrap() + 1);
        assert!(!matches!(
            features::rename(&workspace, &uri, at, "fresh"),
            Ok(Some(_))
        ));
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn native_captured_marker_keeps_its_optional_boundary_after_source_redefinition() {
    native_label_rename_keeps_auxiliary(
        r"\let\saved\footnotemark\renewcommand\footnotemark[1]{}\saved[7]\label{ghost}",
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn native_single_argument_identity_macros_execute_their_label_arguments() {
    for body in [
        r"\newcommand\one[1]{#1}\one{\label{ghost}}",
        r"\def\one#1;{#1}\one\label{ghost};",
        r"\newcommand\one[1]{#1}\def\forward{\one}\forward{\label{ghost}}",
        r"\newcommand\one[1]{#1}\csname one\endcsname{\label{ghost}}",
    ] {
        native_label_rename_keeps_auxiliary(body);
    }
}

fn native_label_rename_keeps_auxiliary(body: &str) {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let source = format!(
        r"\documentclass{{article}}\begin{{document}}\section{{Heading}}{body}\ref{{ghost}}\end{{document}}"
    );
    let compile = |source: &str| {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
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
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
        assert!(
            !log.contains("Warning") && !log.contains("undefined"),
            "{log}"
        );
        std::fs::read_to_string(aux).unwrap()
    };
    let before = compile(&source);
    std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.clone(), 5);
    let text = &workspace.documents[&uri].text;
    let at = text.position(source.find("ghost").unwrap() + 1);
    let changes = features::rename(&workspace, &uri, at, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes[&uri].len(), 2);
    let mut edits = changes[&uri].clone();
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
    let after = compile(&changed.source);
    assert!(before.contains(r"\newlabel{ghost}") && after.contains(r"\newlabel{fresh}"));
    assert_eq!(before, after.replace("{fresh}", "{ghost}"));
}
