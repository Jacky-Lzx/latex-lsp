use lsp_types::*;
use texquill::{
    language::{features, signature},
    project::workspace::Workspace,
    text::Text,
};

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace, Url) {
    let directory = tempfile::tempdir().unwrap();
    for (name, source) in files {
        let path = directory.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    (directory, workspace, main)
}

fn rewritten(workspace: &Workspace, edit: &WorkspaceEdit, uri: &Url) -> String {
    let mut edits = edit.changes.as_ref().unwrap()[uri].clone();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[uri]
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
        .unwrap()
        .source
}

#[test]
fn direct_copies_keep_data_opaque_and_branches_editable_after_the_source_is_redefined() {
    for predicate in ["ifblank", "ifstrempty", "notblank", "ifstrequal"] {
        for copy in [
            format!("\\let\\saved\\{predicate}"),
            format!("\\let\\saved = \\{predicate}"),
            format!("\\cslet{{saved}}\\{predicate}"),
            format!("\\letcs\\saved{{{predicate}}}"),
            format!("\\csletcs{{saved}}{{{predicate}}}"),
            format!("\\NewCommandCopy{{\\saved}}{{\\{predicate}}}"),
            format!("\\DeclareCommandCopy{{\\saved}}{{\\{predicate}}}"),
        ] {
            let extra = if predicate == "ifstrequal" {
                r"{\input{ghost}\newbool{ghost}}"
            } else {
                ""
            };
            let source = format!("\\documentclass{{article}}\\newbool{{draft}}{copy}\\renewcommand{{\\{predicate}}}[1]{{#1}}\\saved{{\\booltrue{{draft}}}}{extra}{{\\boolfalse{{draft}}}}{{\\booltrue{{draft}}}}");
            let (directory, workspace, main) =
                setup(&[("main.tex", &source), ("ghost.tex", r"\newbool{ghost}")]);
            let data =
                Text::new(source.clone()).position(source.find("booltrue{draft}").unwrap() + 10);
            assert!(
                features::definitions(&workspace, &main, data).is_empty(),
                "{copy}"
            );
            let target = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
            let edit = features::rename(&workspace, &main, target, "release")
                .unwrap()
                .unwrap();
            let expected = source
                .replacen("newbool{draft}", "newbool{release}", 1)
                .replace("boolfalse{draft}", "boolfalse{release}");
            let at = expected.rfind("booltrue{draft}").unwrap();
            let mut expected = expected;
            expected.replace_range(at..at + "booltrue{draft}".len(), "booltrue{release}");
            assert_eq!(rewritten(&workspace, &edit, &main), expected, "{copy}");
            let ghost = Url::from_file_path(directory.path().join("ghost.tex")).unwrap();
            assert!(
                !workspace.project_documents(&main).contains(&ghost),
                "{copy}"
            );
        }
    }
}

#[test]
fn captured_data_arity_controls_native_names_and_alias_signature_help() {
    for (copy, name, data, active) in [
        (
            r"\let\saved\ifstrequal",
            "saved",
            r"{A}{\booltrue{draft}}",
            1,
        ),
        (
            r"\let\ifblank\ifstrequal",
            "ifblank",
            r"{A}{\booltrue{draft}}",
            1,
        ),
        (
            r"\let\ifstrequal\ifblank",
            "ifstrequal",
            r"{\booltrue{draft}}",
            0,
        ),
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{copy}\\{name}{data}{{\\boolfalse{{draft}}}}{{N}}");
        let (_directory, workspace, main) = setup(&[("main.tex", &source)]);
        let position =
            Text::new(source.clone()).position(source.find("booltrue{draft}").unwrap() + 10);
        assert!(
            features::definitions(&workspace, &main, position).is_empty(),
            "{copy}"
        );
        assert!(
            features::completion(&workspace, &main, position).is_empty(),
            "{copy}"
        );
        let help = signature::help(&workspace, &main, position).unwrap();
        assert_eq!(help.active_parameter, Some(active), "{copy}");
        assert_eq!(
            help.signatures[0].label,
            format!(
                "\\{name}{}{{then}}{{else}}",
                "{string}".repeat(active as usize + 1)
            ),
            "{copy}"
        );
    }
}

#[test]
fn shared_native_arities_preserve_common_literal_data_and_guard_only_the_extra_operand() {
    let shared = r"\saved{\booltrue{first}}{\booltrue{second}}{Y}{N}\boolfalse{other}";
    let (directory, workspace, _) = setup(&[
        (
            "a.tex",
            r"\documentclass{article}\input{booleans}\let\saved\ifblank\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\input{booleans}\let\saved\ifstrequal\input{shared}",
        ),
        (
            "booleans.tex",
            r"\newbool{first}\newbool{second}\newbool{other}",
        ),
        ("shared.tex", shared),
    ]);
    let declarations = Url::from_file_path(directory.path().join("booleans.tex")).unwrap();
    let text = &workspace.documents[&declarations].text;
    let first = text.position(text.source.find("first").unwrap() + 1);
    let second = text.position(text.source.find("second").unwrap() + 1);
    assert!(
        features::rename(&workspace, &declarations, first, "initial")
            .unwrap()
            .is_some()
    );
    assert!(features::rename(&workspace, &declarations, second, "later").is_err());
    let shared_uri = Url::from_file_path(directory.path().join("shared.tex")).unwrap();
    let first_data = Text::new(shared.into()).position(shared.find("first").unwrap() + 1);
    assert!(features::definitions(&workspace, &shared_uri, first_data).is_empty());
    let other = Text::new(shared.into()).position(shared.rfind("other").unwrap() + 1);
    assert!(features::rename(&workspace, &shared_uri, other, "another")
        .unwrap()
        .is_some());
}

#[test]
fn native_copies_do_not_keep_the_destinations_old_file_declaration_or_verbatim_grammar() {
    for (name, data) in [
        ("input", "ghost"),
        ("label", "ghost"),
        ("newcommand", r"\ghost"),
        ("verb", r"\label{ghost}"),
    ] {
        let source=format!("\\documentclass{{article}}\\let\\{name}\\ifblank\\{name}{{{data}}}{{Y}}{{N}}\\newbool{{real}}");
        let (directory, workspace, main) =
            setup(&[("main.tex", &source), ("ghost.tex", r"\newbool{ghost}")]);
        let parsed = &workspace.documents[&main].syntax;
        assert!(parsed.labels.is_empty(), "{name}: {:?}", parsed.labels);
        assert!(
            !parsed.definitions.iter().any(|entry| entry.name == "ghost"),
            "{name}"
        );
        assert!(
            parsed.conditionals.iter().any(|entry| entry.name == "real"),
            "{name}"
        );
        assert!(
            parsed.diagnostics.is_empty(),
            "{name}: {:?}",
            parsed.diagnostics
        );
        let ghost = Url::from_file_path(directory.path().join("ghost.tex")).unwrap();
        assert!(
            !workspace.project_documents(&main).contains(&ghost),
            "{name}"
        );
    }
}

#[test]
fn shared_native_and_custom_copies_guard_only_names_whose_execution_differs() {
    let (directory, workspace, _) = setup(&[
        (
            "a.tex",
            r"\documentclass{article}\input{flags}\input{save}\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\input{flags}\newcommand{\saved}[1]{#1}\input{shared}",
        ),
        ("flags.tex", r"\newbool{draft}\newbool{other}"),
        (
            "save.tex",
            r"\let\first\ifblank\input{chain}\renewcommand{\first}[1]{#1}",
        ),
        ("chain.tex", r"\let\saved\first"),
        (
            "shared.tex",
            r"\saved{\booltrue{draft}}{Y}{N}\boolfalse{other}",
        ),
    ]);
    let flags = Url::from_file_path(directory.path().join("flags.tex")).unwrap();
    let text = &workspace.documents[&flags].text;
    assert!(features::rename(
        &workspace,
        &flags,
        text.position(text.source.find("draft").unwrap() + 1),
        "release"
    )
    .is_err());
    assert!(features::rename(
        &workspace,
        &flags,
        text.position(text.source.find("other").unwrap() + 1),
        "another"
    )
    .unwrap()
    .is_some());
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_direct_copies_preserve_literal_tokens_and_states_after_macro_and_boolean_renames() {
    use std::os::unix::fs::PermissionsExt;
    use texquill::{compilation::build, config::BuildConfig};
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for (predicate, destination) in [
            ("ifblank", "saved"),
            ("notblank", "saved"),
            ("ifstrempty", "saved"),
            ("ifstrequal", "saved"),
            ("ifstrequal", "ifblank"),
            ("ifblank", "ifstrequal"),
        ] {
            for copy in [
                format!("\\let\\{destination}\\{predicate}"),
                format!("\\csletcs{{{destination}}}{{{predicate}}}"),
                format!("\\DeclareCommandCopy{{\\{destination}}}{{\\{predicate}}}"),
            ] {
                let extra = if predicate == "ifstrequal" {
                    "{Never}"
                } else {
                    ""
                };
                let (yes, no) = if predicate == "notblank" {
                    ("literal", "wrong")
                } else {
                    ("wrong", "literal")
                };
                let source = format!("\\documentclass{{article}}\\usepackage{{etoolbox}}\\newbool{{draft}}{copy}\\renewcommand{{\\{predicate}}}[1]{{#1}}\\begin{{document}}\\{predicate}{{\\booltrue{{draft}}}}\\{destination}{{\\boolfalse{{draft}}}}{extra}{{\\typeout{{COPY:{yes}}}}}{{\\typeout{{COPY:{no}}}}}\\ifbool{{draft}}{{\\typeout{{COPY:true}}}}{{\\typeout{{COPY:wrong}}}}Text.\\end{{document}}");
                let (directory, mut workspace, main) = setup(&[("main.tex", &source)]);
                let wrapper = directory.path().join("engine");
                std::fs::write(&wrapper, format!("#!/bin/sh\nexport TEXMFVAR=\"${{0%/*}}/tex-cache\"\nexport TEXMFCACHE=\"$TEXMFVAR\"\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
                let config = BuildConfig {
                    executable: wrapper.to_string_lossy().into(),
                    arguments: Some(vec![
                        "-interaction=nonstopmode".into(),
                        "-no-shell-escape".into(),
                        "-file-line-error".into(),
                        "-recorder".into(),
                        "-output-directory=%o".into(),
                        "%f".into(),
                    ]),
                    timeout_seconds: 20,
                    ..Default::default()
                };
                let before =
                    build::run(&main.to_file_path().unwrap(), &config, &Default::default());
                assert_eq!(
                    before.status, "success",
                    "{engine}/{copy}: {}",
                    before.output
                );
                if destination == "saved" {
                    let at = Text::new(source.clone()).position(source.find("saved").unwrap() + 1);
                    let edit = features::rename(&workspace, &main, at, "original")
                        .unwrap()
                        .unwrap();
                    let renamed = rewritten(&workspace, &edit, &main);
                    assert_eq!(renamed.matches("original").count(), 2, "{copy}");
                    workspace.open(main.clone(), renamed, 2);
                }
                let text = &workspace.documents[&main].text;
                let at = text.position(text.source.find("newbool{draft}").unwrap() + 8);
                let edit = features::rename(&workspace, &main, at, "release")
                    .unwrap()
                    .unwrap();
                let renamed = rewritten(&workspace, &edit, &main);
                assert!(renamed.contains(r"\boolfalse{draft}"), "{copy}");
                assert!(renamed.contains(r"\booltrue{release}"), "{copy}");
                std::fs::write(main.to_file_path().unwrap(), renamed).unwrap();
                let after = build::run(&main.to_file_path().unwrap(), &config, &Default::default());
                assert_eq!(after.status, "success", "{engine}/{copy}: {}", after.output);
                let states = |output: &str| {
                    output
                        .lines()
                        .filter(|line| line.starts_with("COPY:"))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    states(&before.output),
                    ["COPY:literal", "COPY:true"],
                    "{engine}/{copy}"
                );
                assert_eq!(
                    states(&after.output),
                    states(&before.output),
                    "{engine}/{copy}"
                );
                assert_eq!(before.pages, Some(1));
                assert_eq!(after.pages, before.pages);
            }
        }
    }
}
