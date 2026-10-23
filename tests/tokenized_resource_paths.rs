use latex_lsp::{features, file_operations, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(body: &str) -> (tempfile::TempDir, Workspace, Url, String) {
    let dir = tempfile::tempdir().unwrap();
    let source = format!(r"\documentclass{{article}}{body}");
    std::fs::write(dir.path().join("main.tex"), &source).unwrap();
    for name in [
        "refs.bib",
        "refs%data.bib",
        "refs#data.bib",
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
fn resource_moves_reject_comment_parameter_and_character_translation_in_macro_bodies() {
    for command in ["addbibresource", "addglobalbib", "addsectionbib"] {
        for declaration in ["def", "newcommand"] {
            let body = format!(r"\{declaration}\future{{\{command}{{refs.bib}}}}\future");
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

#[test]
#[ignore = "requires pdflatex and biblatex"]
fn real_tokenized_resource_moves_preserve_the_native_database_name() {
    let body = r"\usepackage{biblatex}\def\future{\addbibresource{refs.bib}}\future\begin{document}Text.\end{document}";
    let (dir, ws, main, source) = setup(body);
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

#[test]
fn resource_completions_filter_only_names_changed_by_precollected_tokens() {
    for command in ["addbibresource", "addglobalbib", "addsectionbib"] {
        for wrapped in [false, true] {
            for call in [
                format!(r"\{command}"),
                format!(r"\csuse{{{command}}}"),
                r"\register".into(),
            ] {
                if wrapped && call != format!(r"\{command}") {
                    continue;
                }
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
