use latex_lsp::{language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;

fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace, Url) {
    let directory = tempfile::tempdir().unwrap();
    for (name, source) in files {
        std::fs::write(directory.path().join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    (directory, workspace, main)
}

fn rewritten(workspace: &Workspace, edit: WorkspaceEdit, uri: &Url) -> String {
    let mut edits = edit.changes.unwrap()[uri].clone();
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
fn imported_string_test_redefinitions_follow_input_order_and_keep_earlier_literals() {
    let source = concat!(
        "\\documentclass{article}\\newbool{draft}\n",
        "\\ifblank{\\booltrue{draft}}{Before}{No}\n",
        "\\input{definitions}\n",
        "\\ifblank{\\boolfalse{draft}}\n",
    );
    let (_directory, workspace, uri) = setup(&[
        ("main.tex", source),
        ("definitions.tex", r"\renewcommand{\ifblank}[1]{#1}"),
    ]);
    let position = Text::new(source.into()).position(source.find("boolfalse{draft}").unwrap() + 11);
    assert_eq!(features::definitions(&workspace, &uri, position).len(), 1);
    assert_eq!(
        features::references(&workspace, &uri, position, true).len(),
        2
    );
    let edit = features::rename(&workspace, &uri, position, "release")
        .unwrap()
        .unwrap();
    assert_eq!(
        rewritten(&workspace, edit, &uri),
        source
            .replacen("\\newbool{draft}", "\\newbool{release}", 1)
            .replace("\\boolfalse{draft}", "\\boolfalse{release}")
    );
}

#[test]
fn local_packages_and_nested_dependencies_propagate_predicate_redefinitions() {
    for load in [r"\input{outer}", r"\usepackage{local}"] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{load}\n\\ifstrempty{{\\booltrue{{draft}}}}\n");
        let (_directory, workspace, uri) = setup(&[
            ("main.tex", &source),
            ("outer.tex", r"\input{definitions}"),
            ("definitions.tex", r"\def\ifstrempty#1{#1}"),
            ("local.sty", r"\RequirePackage{etoolbox}\input{definitions}"),
        ]);
        let position =
            Text::new(source.clone()).position(source.find("booltrue{draft}").unwrap() + 10);
        assert_eq!(
            features::definitions(&workspace, &uri, position).len(),
            1,
            "{load}"
        );
        let edit = features::rename(&workspace, &uri, position, "release")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &uri),
            source.replace("draft", "release")
        );
    }
}

#[test]
fn imported_redefinitions_refresh_on_unsaved_edits() {
    let source = "\\documentclass{article}\\newbool{draft}\\input{definitions}\n\\ifblank{\\booltrue{draft}}{Y}{N}\n";
    let (directory, mut workspace, uri) = setup(&[("main.tex", source), ("definitions.tex", "")]);
    let definitions = Url::from_file_path(directory.path().join("definitions.tex")).unwrap();
    workspace.open(uri.clone(), source.into(), 9);
    let position = Text::new(source.into()).position(source.find("booltrue{draft}").unwrap() + 10);
    assert!(features::definitions(&workspace, &uri, position).is_empty());
    workspace.open(
        definitions.clone(),
        "\\renewcommand{\\ifblank}[1]{#1}".into(),
        1,
    );
    assert_eq!(features::definitions(&workspace, &uri, position).len(), 1);
    assert_eq!(workspace.documents[&uri].version, Some(9));
    workspace.open(definitions.clone(), String::new(), 2);
    assert!(features::definitions(&workspace, &uri, position).is_empty());
}

#[test]
fn quoted_or_deferred_loads_do_not_change_later_string_predicate_meanings() {
    for prefix in [
        r"\ifstrequal{\input{definitions}}{X}{Yes}{No}",
        r"\newcommand{\later}{\input{definitions}}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{prefix}\n\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}\n");
        let (_directory, workspace, uri) = setup(&[
            ("main.tex", &source),
            ("definitions.tex", r"\renewcommand{\ifblank}[1]{#1}"),
        ]);
        let position =
            Text::new(source.clone()).position(source.find("booltrue{draft}").unwrap() + 10);
        assert!(
            features::definitions(&workspace, &uri, position).is_empty(),
            "{prefix}"
        );
    }
}

#[test]
fn shared_sources_with_different_predicate_meanings_reject_only_ambiguous_edits() {
    let shared = "\\ifblank{\\booltrue{draft}}{Y}{N}\n\\boolfalse{other}\n";
    let (_directory, workspace, main) = setup(&[
        (
            "main.tex",
            "\\documentclass{article}\\input{booleans}\\input{shared}",
        ),
        (
            "second.tex",
            "\\documentclass{article}\\input{booleans}\\input{definitions}\\input{shared}",
        ),
        ("booleans.tex", "\\newbool{draft}\\newbool{other}"),
        ("definitions.tex", "\\renewcommand{\\ifblank}[1]{#1}"),
        ("shared.tex", shared),
    ]);
    let path = main.to_file_path().unwrap();
    let declaration = Url::from_file_path(path.parent().unwrap().join("booleans.tex")).unwrap();
    let shared_uri = Url::from_file_path(path.parent().unwrap().join("shared.tex")).unwrap();
    assert!(features::rename(&workspace, &declaration, Position::new(0, 10), "release").is_err());
    let ambiguous = Text::new(shared.into()).position(shared.find("draft").unwrap() + 1);
    assert!(features::prepare_rename(&workspace, &shared_uri, ambiguous)
        .unwrap()
        .is_none());
    let other = Text::new(workspace.documents[&declaration].text.source.clone()).position(
        workspace.documents[&declaration]
            .text
            .source
            .find("other")
            .unwrap(),
    );
    assert!(features::rename(&workspace, &declaration, other, "another")
        .unwrap()
        .is_some());
}

#[test]
fn repeated_loads_keep_both_meanings_and_do_not_leak_deferred_or_literal_assignments() {
    let shared = r"\ifblank{\booltrue{draft}}{Y}{N}";
    let (_directory, workspace, main) = setup(&[
        (
            "main.tex",
            r"\documentclass{article}\newbool{draft}\input{shared}\input{definitions}\input{shared}",
        ),
        ("shared.tex", shared),
        ("definitions.tex", r"\renewcommand{\ifblank}[1]{#1}"),
    ]);
    let declaration = Text::new(workspace.documents[&main].text.source.clone()).position(
        workspace.documents[&main]
            .text
            .source
            .find("draft")
            .unwrap(),
    );
    assert!(features::rename(&workspace, &main, declaration, "release").is_err());

    for prefix in [
        r"\newcommand{\later}{\renewcommand{\ifblank}[1]{#1}}",
        r"\ifstrequal{\renewcommand{\ifblank}[1]{#1}}{X}{Y}{N}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{prefix}{shared}");
        let (_directory, workspace, uri) = setup(&[("main.tex", &source)]);
        let position = Text::new(source.clone()).position(source.rfind("draft").unwrap());
        assert!(
            features::definitions(&workspace, &uri, position).is_empty(),
            "{prefix}"
        );
    }
}

#[test]
fn exposed_inputs_load_sources_outside_scanned_roots() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let source =
        r"\documentclass{article}\newbool{draft}\input{definitions}\ifblank{\input{../outside}}";
    std::fs::write(project.join("main.tex"), source).unwrap();
    std::fs::write(
        project.join("definitions.tex"),
        r"\renewcommand{\ifblank}[1]{#1}",
    )
    .unwrap();
    let outside_source = r"\booltrue{draft}";
    std::fs::write(directory.path().join("outside.tex"), outside_source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(project);
    let outside = Url::from_file_path(directory.path().join("outside.tex")).unwrap();
    assert!(workspace.documents.contains_key(&outside));
    let position = Text::new(outside_source.into()).position(outside_source.find("draft").unwrap());
    assert_eq!(
        features::definitions(&workspace, &outside, position).len(),
        1
    );
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_imported_predicate_redefinitions_preserve_rename_behavior_in_three_engines() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    let source = concat!(
        "\\documentclass{article}\\usepackage{etoolbox}\\newbool{draft}\n",
        "\\begin{document}\n",
        "\\ifblank{\\boolfalse{draft}}{\\typeout{ORDER:wrong}}{\\typeout{ORDER:literal}}\n",
        "\\input{definitions}\n",
        "\\ifblank{\\booltrue{draft}}\n",
        "\\ifbool{draft}{\\typeout{ORDER:true}}{\\typeout{ORDER:wrong}}\n",
        "Text.\\end{document}\n",
    );
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let (directory, workspace, uri) = setup(&[
            ("main.tex", source),
            ("definitions.tex", r"\renewcommand{\ifblank}[1]{#1}"),
        ]);
        let root = directory.path();
        let wrapper = root.join("engine");
        std::fs::write(&wrapper, format!("#!/bin/sh\nexport TEXMFVAR=\"${{0%/*}}/tex-cache\"\nexport TEXMFCACHE=\"$TEXMFVAR\"\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = BuildConfig {
            executable: wrapper.to_string_lossy().into_owned(),
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
        let before = build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(before.status, "success", "{engine}: {}", before.output);
        let position =
            Text::new(source.into()).position(source.find("newbool{draft}").unwrap() + 8);
        let edit = features::rename(&workspace, &uri, position, "release")
            .unwrap()
            .unwrap();
        let changed = rewritten(&workspace, edit, &uri);
        assert!(changed.contains(r"\ifblank{\boolfalse{draft}}"));
        assert!(changed.contains(r"\ifblank{\booltrue{release}}"));
        std::fs::write(root.join("main.tex"), changed).unwrap();
        let after = build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(after.status, "success", "{engine}: {}", after.output);
        let states = |output: &str| {
            output
                .lines()
                .filter(|line| line.starts_with("ORDER:"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(states(&before.output), ["ORDER:literal", "ORDER:true"]);
        assert_eq!(states(&after.output), states(&before.output));
        assert_eq!(after.pages, Some(1));
    }
}

#[test]
fn literal_verbatim_and_declaration_tokens_cannot_hide_later_imports() {
    for data in [r"\begin{verbatim}", r"\verb|", r"\def\ifblank#1{#1}"] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\ifblank{{{data}}}{{Y}}{{N}}\\input{{definitions}}\\ifblank{{\\booltrue{{draft}}}}");
        let (_directory, workspace, uri) = setup(&[
            ("main.tex", &source),
            ("definitions.tex", r"\renewcommand{\ifblank}[1]{#1}"),
        ]);
        let position = Text::new(source.clone()).position(source.rfind("draft").unwrap());
        assert_eq!(
            features::definitions(&workspace, &uri, position).len(),
            1,
            "{data}"
        );
        let edit = features::rename(&workspace, &uri, position, "release")
            .unwrap()
            .unwrap();
        assert!(rewritten(&workspace, edit, &uri).contains(&format!("\\ifblank{{{data}}}")));
    }
}

#[test]
fn explicit_copies_restore_native_string_data_after_custom_calls() {
    for (save, restore) in [
        (r"\let\saved\ifblank", r"\let\ifblank\saved"),
        (r"\let\saved=\ifblank", r"\let\ifblank = \saved"),
        (r"\cslet{saved}\ifblank", r"\letcs\ifblank{saved}"),
        (r"\csletcs{saved}{ifblank}", r"\csletcs{ifblank}{saved}"),
        (
            r"\NewCommandCopy{\saved}{\ifblank}",
            r"\RenewCommandCopy{\ifblank}{\saved}",
        ),
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{save}\\renewcommand{{\\ifblank}}[1]{{#1}}\\ifblank{{\\booltrue{{draft}}}}{restore}\\ifblank{{\\boolfalse{{draft}}}}{{Y}}{{N}}");
        let (_directory, workspace, main) = setup(&[("main.tex", &source)]);
        let literal = Text::new(source.clone()).position(source.rfind("draft").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &main, literal).is_empty(),
            "{save}/{restore}"
        );
        let declaration = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
        let edit = features::rename(&workspace, &main, declaration, "release")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &main),
            source
                .replacen("newbool{draft}", "newbool{release}", 1)
                .replace("booltrue{draft}", "booltrue{release}"),
            "{save}/{restore}"
        );
    }
}

#[test]
fn copied_bindings_follow_nested_inputs_and_keep_the_meaning_at_copy_time() {
    for middle in [
        r"\let\chain\saved\renewcommand{\saved}[1]{#1}\let\ifblank\chain",
        r"\let\ifblank\saved",
    ] {
        let source = r"\documentclass{article}\newbool{draft}\input{save}\input{override}\ifblank{\booltrue{draft}}\input{restore}\ifblank{\boolfalse{draft}}{Y}{N}";
        let (_directory, workspace, main) = setup(&[
            ("main.tex", source),
            ("save.tex", r"\let\saved\ifblank"),
            ("override.tex", r"\renewcommand{\ifblank}[1]{#1}"),
            ("restore.tex", middle),
        ]);
        let literal = Text::new(source.into()).position(source.rfind("draft").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &main, literal).is_empty(),
            "{middle}"
        );
        let target = Text::new(source.into()).position(source.find("draft").unwrap() + 1);
        let edit = features::rename(&workspace, &main, target, "release")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &main),
            source
                .replacen("newbool{draft}", "newbool{release}", 1)
                .replace("booltrue{draft}", "booltrue{release}")
        );
    }
}

#[test]
fn overwritten_and_custom_copies_do_not_restore_a_native_predicate() {
    for changes in [
        r"\renewcommand{\saved}[1]{#1}\let\ifblank\saved",
        r"\let\custom\ifblank\let\ifblank\saved\let\ifblank\custom",
        r"\def\saved{\ifblank}\let\ifblank\saved",
        r"\let\saved\undefined\let\ifblank\saved",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\let\\saved\\ifblank\\renewcommand{{\\ifblank}}[1]{{#1}}{changes}\\ifblank{{\\booltrue{{draft}}}}");
        let (_directory, workspace, main) = setup(&[("main.tex", &source)]);
        let target = Text::new(source.clone()).position(source.rfind("draft").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, &main, target).len(),
            1,
            "{changes}"
        );
    }
}

#[test]
fn restoring_a_native_copy_does_not_execute_quoted_or_deferred_assignments() {
    for decoy in [
        r"\newcommand{\later}{\renewcommand{\saved}[1]{#1}}",
        r"\ifblank{\let\saved\undefined}{Y}{N}",
        r"\detokenize{\let\saved\undefined}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\let\\saved\\ifblank{decoy}\\renewcommand{{\\ifblank}}[1]{{#1}}\\let\\ifblank\\saved\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}");
        let (_directory, workspace, main) = setup(&[("main.tex", &source)]);
        let target = Text::new(source.clone()).position(source.rfind("draft").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &main, target).is_empty(),
            "{decoy}"
        );
    }
}

#[test]
fn native_copies_with_different_data_arities_keep_every_actual_data_argument_literal() {
    let source = r"\documentclass{article}\newbool{first}\newbool{second}\newbool{other}\let\ifblank\ifstrequal\ifblank{\booltrue{first}}{\booltrue{second}}{Y}{N}\boolfalse{other}";
    let (_directory, workspace, main) = setup(&[("main.tex", source)]);
    for name in ["first", "second"] {
        let text = Text::new(source.into());
        let declaration = text.position(source.find(name).unwrap() + 1);
        let argument = text.position(source.rfind(name).unwrap() + 1);
        let edit = features::rename(&workspace, &main, declaration, "release")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &main),
            source.replacen(&format!("newbool{{{name}}}"), "newbool{release}", 1),
            "{name}"
        );
        assert!(
            features::definitions(&workspace, &main, argument).is_empty(),
            "{name}"
        );
    }
    let other = Text::new(source.into()).position(source.rfind("other").unwrap() + 1);
    assert!(features::rename(&workspace, &main, other, "another")
        .unwrap()
        .is_some());
}

#[test]
fn repeatedly_loaded_sources_keep_custom_and_restored_contexts_separate() {
    let source = r"\documentclass{article}\newbool{draft}\newbool{other}\let\saved\ifblank\renewcommand{\ifblank}[1]{#1}\input{shared}\let\ifblank\saved\input{shared}";
    let shared = r"\ifblank{\booltrue{draft}}{Y}{N}\boolfalse{other}";
    let (directory, workspace, main) = setup(&[("main.tex", source), ("shared.tex", shared)]);
    let target = Text::new(source.into()).position(source.find("draft").unwrap() + 1);
    assert!(features::rename(&workspace, &main, target, "release").is_err());
    let uri = Url::from_file_path(directory.path().join("shared.tex")).unwrap();
    let other = Text::new(shared.into()).position(shared.rfind("other").unwrap() + 1);
    assert!(features::rename(&workspace, &uri, other, "another")
        .unwrap()
        .is_some());
}

#[test]
fn captured_parameter_grammars_keep_the_following_restore_input_executable() {
    for (definition, call) in [
        (
            r"\renewcommand{\ifblank}[1]{#1}",
            r"\ifblank{\booltrue{draft}}",
        ),
        (
            r"\renewcommand{\ifblank}[2][unused]{#2}",
            r"\ifblank[option]{\booltrue{draft}}",
        ),
        (
            r"\RenewDocumentCommand{\ifblank}{s O{} m}{#3}",
            r"\ifblank*[option]{\booltrue{draft}}",
        ),
        (r"\def\ifblank#1;{#1}", r"\ifblank{\booltrue{draft}};"),
        (r"\csdef{ifblank}#1;{#1}", r"\ifblank{\booltrue{draft}};"),
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\let\\saved\\ifblank\\input{{override}}{call}\\input{{restore}}\\ifblank{{\\boolfalse{{draft}}}}{{Y}}{{N}}");
        let (directory, workspace, main) = setup(&[
            ("main.tex", &source),
            ("override.tex", definition),
            ("restore.tex", r"\let\ifblank\saved"),
        ]);
        let literal = Text::new(source.clone()).position(source.rfind("draft").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &main, literal).is_empty(),
            "{definition}"
        );
        let restore = Url::from_file_path(directory.path().join("restore.tex")).unwrap();
        assert!(
            workspace.project_documents(&main).contains(&restore),
            "{definition}"
        );
        let target = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
        let edit = features::rename(&workspace, &main, target, "release")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &main),
            source
                .replacen("newbool{draft}", "newbool{release}", 1)
                .replace("booltrue{draft}", "booltrue{release}"),
            "{definition}"
        );
    }
}

#[test]
fn every_native_string_predicate_can_be_explicitly_restored_through_a_renamed_copy() {
    for predicate in ["ifblank", "notblank", "ifstrempty", "ifstrequal"] {
        let extra = if predicate == "ifstrequal" {
            "{Never}"
        } else {
            ""
        };
        let source=format!("\\documentclass{{article}}\\newbool{{draft}}\\let\\saved\\{predicate}\\renewcommand{{\\{predicate}}}[1]{{#1}}\\{predicate}{{\\booltrue{{draft}}}}\\let\\{predicate}\\saved\\{predicate}{{\\boolfalse{{draft}}}}{extra}{{Y}}{{N}}");
        let (_directory, mut workspace, main) = setup(&[("main.tex", &source)]);
        let alias = Text::new(source.clone()).position(source.find("saved").unwrap() + 1);
        let alias_edit = features::rename(&workspace, &main, alias, "original")
            .unwrap()
            .unwrap();
        let renamed = rewritten(&workspace, alias_edit, &main);
        workspace.open(main.clone(), renamed.clone(), 2);
        let literal = Text::new(renamed.clone()).position(renamed.rfind("draft").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &main, literal).is_empty(),
            "{predicate}"
        );
        let target = Text::new(renamed.clone()).position(renamed.find("draft").unwrap() + 1);
        let edit = features::rename(&workspace, &main, target, "release")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &main),
            renamed
                .replacen("newbool{draft}", "newbool{release}", 1)
                .replace("booltrue{draft}", "booltrue{release}"),
            "{predicate}"
        );
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_restored_predicates_preserve_literal_tokens_and_boolean_states_in_three_engines() {
    use latex_lsp::{compilation::build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for predicate in ["ifblank", "notblank", "ifstrempty", "ifstrequal"] {
            for (save, restore) in [
                (
                    format!("\\let\\saved\\{predicate}"),
                    format!("\\let\\{predicate}\\saved"),
                ),
                (
                    format!("\\csletcs{{saved}}{{{predicate}}}"),
                    format!("\\csletcs{{{predicate}}}{{saved}}"),
                ),
                (
                    format!("\\NewCommandCopy{{\\saved}}{{\\{predicate}}}"),
                    format!("\\RenewCommandCopy{{\\{predicate}}}{{\\saved}}"),
                ),
            ] {
                let extra = if predicate == "ifstrequal" {
                    "{Never}"
                } else {
                    ""
                };
                let branches = |marker: &str| {
                    if predicate == "notblank" {
                        format!("{{\\typeout{{RESTORE:{marker}}}}}{{\\typeout{{RESTORE:wrong}}}}")
                    } else {
                        format!("{{\\typeout{{RESTORE:wrong}}}}{{\\typeout{{RESTORE:{marker}}}}}")
                    }
                };
                let source=format!("\\documentclass{{article}}\\usepackage{{etoolbox}}\\newbool{{draft}}{save}\n\\begin{{document}}\\{predicate}{{\\boolfalse{{draft}}}}{extra}{}\\input{{override}}\\{predicate}{{\\booltrue{{draft}}}}\\input{{restore}}\\{predicate}{{\\boolfalse{{draft}}}}{extra}{}\\ifbool{{draft}}{{\\typeout{{RESTORE:true}}}}{{\\typeout{{RESTORE:wrong}}}}Text.\\end{{document}}",branches("before"),branches("after"));
                let definition =
                    format!("\\renewcommand{{\\{predicate}}}[1]{{#1\\typeout{{RESTORE:custom}}}}");
                let (directory, workspace, main) = setup(&[
                    ("main.tex", &source),
                    ("override.tex", &definition),
                    ("restore.tex", &restore),
                ]);
                let root = directory.path();
                let wrapper = root.join("engine");
                std::fs::write(&wrapper,format!("#!/bin/sh\nexport TEXMFVAR=\"${{0%/*}}/tex-cache\"\nexport TEXMFCACHE=\"$TEXMFVAR\"\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
                let config = BuildConfig {
                    executable: wrapper.to_string_lossy().into_owned(),
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
                let before = build::run(&root.join("main.tex"), &config, &Default::default());
                assert_eq!(
                    before.status, "success",
                    "{engine}/{predicate}/{save}: {}",
                    before.output
                );
                let target =
                    Text::new(source.clone()).position(source.find("newbool{draft}").unwrap() + 8);
                let edit = features::rename(&workspace, &main, target, "release")
                    .unwrap()
                    .unwrap();
                let renamed = rewritten(&workspace, edit.clone(), &main);
                assert_eq!(
                    renamed.matches(r"\boolfalse{draft}").count(),
                    2,
                    "{engine}/{predicate}/{save}"
                );
                assert!(renamed.contains(r"\booltrue{release}"));
                for uri in edit.changes.as_ref().unwrap().keys() {
                    std::fs::write(
                        uri.to_file_path().unwrap(),
                        rewritten(&workspace, edit.clone(), uri),
                    )
                    .unwrap();
                }
                let after = build::run(&root.join("main.tex"), &config, &Default::default());
                assert_eq!(
                    after.status, "success",
                    "{engine}/{predicate}/{save}: {}",
                    after.output
                );
                let states = |output: &str| {
                    output
                        .lines()
                        .filter(|line| line.starts_with("RESTORE:"))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    states(&before.output),
                    vec![
                        "RESTORE:before",
                        "RESTORE:custom",
                        "RESTORE:after",
                        "RESTORE:true"
                    ],
                    "{engine}/{predicate}/{save}"
                );
                assert_eq!(states(&after.output), states(&before.output));
                assert_eq!(before.pages, Some(1));
                assert_eq!(after.pages, before.pages);
            }
        }
    }
}
