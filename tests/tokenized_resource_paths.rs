use latex_lsp::{features, file_operations, text::Text, workspace::Workspace};
use lsp_types::*;

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, String) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(r"\documentclass{{article}}{body}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    for name in [
        "refs.bib",
        "refs%data.bib",
        "refs#data.bib",
        "refs##data.bib",
        "refs^^61data.bib",
        "refs$&^~data.bib",
    ] {
        std::fs::write(dir.path().join(name), "@book{paper,title={Paper}}").unwrap();
    }
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    ws.open(main.clone(), source.clone(), 7);
    (dir, ws, main, source)
}

#[test]
fn tokenized_resource_names_never_link_comment_parameter_or_translated_source_spelling() {
    for command in ["addbibresource", "addglobalbib", "addsectionbib"] {
        for (argument, false_name) in [
            ("refs%data.bib}\n.bib", "refs%data.bib"),
            ("refs##data.bib", "refs##data.bib"),
            ("refs^^61data.bib", "refs^^61data.bib"),
        ] {
            for environment in [false, true] {
                let body = if environment {
                    format!(
                        r"\newenvironment{{future}}{{\{command}{{{argument}}}}}{{}}\begin{{future}}\end{{future}}"
                    )
                } else {
                    format!(r"\def\future{{\{command}{{{argument}}}}}\future")
                };
                let (dir, ws, main, source) = setup(&body);
                let false_uri = Url::from_file_path(dir.path().join(false_name)).unwrap();
                assert!(!ws.project_files(&main).contains(&false_uri), "{source}");
                let links = latex_lsp::navigation::document_links(&ws, &main);
                assert!(
                    links
                        .iter()
                        .all(|link| link.target.as_ref() != Some(&false_uri)),
                    "{source}: {links:?}"
                );
                let at = Text::new(source.clone()).position(source.find("refs").unwrap() + 3);
                assert!(
                    features::completion(&ws, &main, at)
                        .iter()
                        .all(|item| item.kind != Some(CompletionItemKind::FILE)),
                    "{source}"
                );
                let edit = file_operations::will_rename(
                    &ws,
                    &[FileRename {
                        old_uri: false_uri.into(),
                        new_uri: Url::from_file_path(dir.path().join("changed.bib"))
                            .unwrap()
                            .into(),
                    }],
                )
                .unwrap();
                assert!(edit.is_none(), "{source}: {edit:?}");
            }
        }
    }
}

#[test]
fn resource_moves_reject_comment_parameter_and_character_translation_in_macro_bodies() {
    for command in ["addbibresource", "addglobalbib", "addsectionbib"] {
        for declaration in ["def", "newcommand", "newenvironment"] {
            for call in [
                format!(r"\{command}"),
                format!(r"\csuse{{{command}}}"),
                format!(r"\csname {command}\endcsname"),
                r"\register".into(),
            ] {
                let declaration = if declaration == "newenvironment" {
                    format!(
                        r"\newenvironment{{future}}{{{call}{{refs.bib}}}}{{}}\begin{{future}}\end{{future}}"
                    )
                } else {
                    format!(r"\{declaration}\future{{{call}{{refs.bib}}}}\future")
                };
                let body = format!(r"\def\register{{\{command}}}{declaration}");
                let (dir, ws, main, source) = setup(&body);
                for (name, rejected) in [
                    ("new%data.bib", true),
                    ("new#data.bib", true),
                    ("new^^61data.bib", true),
                    ("new$&^~data.bib", false),
                    ("new 中文.bib", false),
                ] {
                    let result = file_operations::will_rename(
                        &ws,
                        &[FileRename {
                            old_uri: Url::from_file_path(dir.path().join("refs.bib"))
                                .unwrap()
                                .into(),
                            new_uri: Url::from_file_path(dir.path().join(name)).unwrap().into(),
                        }],
                    );
                    if rejected {
                        assert!(result.is_err(), "{body}/{name}: {result:?}");
                    } else {
                        let edit = result.unwrap().unwrap().changes.unwrap();
                        assert_eq!(edit[&main].len(), 1, "{source}/{name}");
                        assert_eq!(edit[&main][0].new_text, name);
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and biblatex"]
fn real_tokenized_resource_moves_preserve_the_native_database_name() {
    for (call, environment) in [
        (r"\addbibresource", false),
        (r"\csuse{addbibresource}", false),
        (r"\csname addbibresource\endcsname", false),
        (r"\register", false),
        (r"\csuse{addbibresource}", true),
    ] {
        let deferred = if environment {
            format!(
                r"\newenvironment{{future}}{{{call}{{refs.bib}}}}{{}}\begin{{future}}\end{{future}}"
            )
        } else {
            format!(r"\def\future{{{call}{{refs.bib}}}}\future")
        };
        let body = format!(
            r"\usepackage{{biblatex}}\def\register{{\addbibresource}}{deferred}\begin{{document}}Text.\end{{document}}"
        );
        let (dir, ws, main, source) = setup(&body);
        for name in ["renamed.bib", "renamed$&^~data.bib"] {
            let result = file_operations::will_rename(
                &ws,
                &[FileRename {
                    old_uri: Url::from_file_path(dir.path().join("refs.bib"))
                        .unwrap()
                        .into(),
                    new_uri: Url::from_file_path(dir.path().join(name)).unwrap().into(),
                }],
            )
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
            let edit = &result[&main][0];
            let changed = Text::new(source.clone())
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text.clone(),
                }])
                .unwrap()
                .source;
            std::fs::write(dir.path().join("main.tex"), changed).unwrap();
            std::fs::copy(dir.path().join("refs.bib"), dir.path().join(name)).unwrap();
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
            let bcf = std::fs::read_to_string(dir.path().join("main.bcf"))
                .unwrap()
                .replace("&amp;", "&");
            assert!(bcf.contains(&format!(">{name}</bcf:datasource>")), "{bcf}");
        }
        let rejected = file_operations::will_rename(
            &ws,
            &[FileRename {
                old_uri: Url::from_file_path(dir.path().join("refs.bib"))
                    .unwrap()
                    .into(),
                new_uri: Url::from_file_path(dir.path().join("bad%data.bib"))
                    .unwrap()
                    .into(),
            }],
        );
        assert!(rejected.is_err());
        std::fs::write(
            dir.path().join("main.tex"),
            source.replace("refs.bib", "bad%data.bib"),
        )
        .unwrap();
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
        assert!(!output.status.success());
    }
}

#[test]
#[ignore = "requires pdflatex and biblatex"]
fn real_precollected_spelling_is_not_the_native_resource_name() {
    for (argument, actual) in [
        ("refs%data.bib}\n.bib", Some("refs.bib")),
        ("refs##data.bib", None),
        ("refs^^61data.bib", Some("refsadata.bib")),
    ] {
        let body = format!(
            r"\usepackage{{biblatex}}\def\future{{\addbibresource{{{argument}}}}}\future\begin{{document}}Text.\end{{document}}"
        );
        let (dir, ws, main, _) = setup(&body);
        let links = latex_lsp::navigation::document_links(&ws, &main);
        assert!(
            links.iter().all(|link| link
                .target
                .as_ref()
                .is_none_or(|target| !target.path().ends_with(".bib"))),
            "{links:?}"
        );
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
        if actual.is_none() {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stdout).contains("Illegal parameter number"));
            continue;
        }
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let bcf = std::fs::read_to_string(dir.path().join("main.bcf")).unwrap();
        assert!(
            bcf.contains(&format!(">{}</bcf:datasource>", actual.unwrap())),
            "{bcf}"
        );
    }
}

#[test]
fn resource_completions_filter_only_names_changed_by_precollected_tokens() {
    for command in ["addbibresource", "addglobalbib", "addsectionbib"] {
        for wrapped in [false, true] {
            for call in [
                format!(r"\{command}"),
                format!(r"\csuse{{{command}}}"),
                format!(r"\csname {command}\endcsname"),
                r"\register".into(),
            ] {
                let call = format!("{call}{{refs.bib}}");
                let body = format!(
                    r"\def\register{{\{command}}}{}",
                    if wrapped {
                        format!(r"\def\future{{{call}}}\future")
                    } else {
                        call
                    }
                );
                let (_dir, ws, main, source) = setup(&body);
                let at = Text::new(source.clone()).position(source.find("refs.bib").unwrap() + 4);
                let items = features::completion(&ws, &main, at);
                for name in [
                    "refs.bib",
                    "refs%data.bib",
                    "refs#data.bib",
                    "refs^^61data.bib",
                    "refs$&^~data.bib",
                ] {
                    let expected = !wrapped || matches!(name, "refs.bib" | "refs$&^~data.bib");
                    assert_eq!(
                        items.iter().any(|item| item.label == name),
                        expected,
                        "{body}/{name}: {items:?}"
                    );
                }
            }
        }
    }
}
