use lsp_types::*;
use texquill::{language::features, project::workspace::Workspace, text::Text};

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

fn position(source: &str, name: &str) -> Position {
    Text::new(source.into()).position(source.rfind(name).unwrap() + 1)
}

fn apply(workspace: &Workspace, edit: &WorkspaceEdit, uri: &Url) -> String {
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

const REDEFINITION: &str = r"\renewcommand{\ifblank}[1]{#1}";

#[test]
fn excluded_chapters_do_not_redefine_later_string_data_or_change_its_rename_text() {
    let source = "\\documentclass{article}\\newbool{draft}\\includeonly{kept}\n\\begin{document}\\include{definitions}\\ifblank{\\booltrue{draft}}{Y}{N}\\boolfalse{draft}\\end{document}";
    let (_directory, workspace, main) =
        setup(&[("main.tex", source), ("definitions.tex", REDEFINITION)]);
    let data = Text::new(source.into()).position(source.find("booltrue{draft}").unwrap() + 10);
    assert!(features::definitions(&workspace, &main, data).is_empty());
    let target = position(source, "draft");
    let edit = features::rename(&workspace, &main, target, "release")
        .unwrap()
        .unwrap();
    assert_eq!(
        apply(&workspace, &edit, &main),
        source
            .replacen("newbool{draft}", "newbool{release}", 1)
            .replace("boolfalse{draft}", "boolfalse{release}")
    );
}

#[test]
fn chapter_selection_matches_spelling_comments_empty_lists_and_tex_suffixes() {
    for (selection, file, include, expected) in [
        ("", "definitions.tex", "definitions", true),
        (r"\includeonly{}", "definitions.tex", "definitions", false),
        (
            r"\includeonly{definitions}",
            "definitions.tex",
            "definitions",
            true,
        ),
        (
            r"\includeonly{definitions.tex}",
            "definitions.tex",
            "definitions",
            true,
        ),
        (
            r"\includeonly{definitions}",
            "definitions.tex",
            "definitions.tex",
            true,
        ),
        (
            r"\includeonly{./definitions}",
            "definitions.tex",
            "definitions",
            false,
        ),
        (
            r"\includeonly{definitions}",
            "definitions.tex",
            "./definitions",
            false,
        ),
        (
            r"\includeonly{my definitions.tex}",
            "my definitions.tex",
            "my definitions",
            true,
        ),
        (
            "\\includeonly{% comment 😀, ignored\r\n definitions% tail\r\n,other}",
            "definitions.tex",
            "definitions",
            true,
        ),
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{selection}\\begin{{document}}\\include{{{include}}}\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}\\end{{document}}");
        let (_directory, workspace, main) = setup(&[("main.tex", &source), (file, REDEFINITION)]);
        assert_eq!(
            features::definitions(&workspace, &main, position(&source, "draft")).len(),
            usize::from(expected),
            "{selection}/{include}"
        );
    }
}

#[test]
fn selectors_follow_import_order_and_never_filter_normal_inputs() {
    for (prefix, load, expected) in [
        (r"\input{settings}", r"\include{definitions}", false),
        (
            r"\input{settings}\includeonly{definitions}",
            r"\include{definitions}",
            true,
        ),
        (
            r"\includeonly{definitions}\input{settings}",
            r"\include{definitions}",
            false,
        ),
        (r"\includeonly{}", r"\input{definitions}", true),
        (
            r"\includeonly{other}\includeonly{definitions.tex}",
            r"\include{definitions}",
            true,
        ),
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{prefix}\\begin{{document}}{load}\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}\\end{{document}}");
        let (_directory, workspace, main) = setup(&[
            ("main.tex", &source),
            ("settings.tex", r"\includeonly{}"),
            ("definitions.tex", REDEFINITION),
        ]);
        assert_eq!(
            features::definitions(&workspace, &main, position(&source, "draft")).len(),
            usize::from(expected),
            "{prefix}/{load}"
        );
    }
}

#[test]
fn excluded_dependency_trees_do_not_add_false_contexts_to_executed_shared_sources() {
    let source = r"\documentclass{article}\newbool{draft}\input{definitions}\includeonly{}\begin{document}\include{excluded}\input{shared}\end{document}";
    let shared = r"\ifblank{\booltrue{draft}}";
    let (directory, workspace, main) = setup(&[
        ("main.tex", source),
        ("definitions.tex", REDEFINITION),
        ("excluded.tex", r"\input{shared}"),
        ("shared.tex", shared),
    ]);
    let target = Text::new(source.into()).position(source.find("draft").unwrap());
    let edit = features::rename(&workspace, &main, target, "release")
        .unwrap()
        .unwrap();
    let shared_uri = Url::from_file_path(directory.path().join("shared.tex")).unwrap();
    assert_eq!(
        apply(&workspace, &edit, &shared_uri),
        shared.replace("draft", "release")
    );
}

#[test]
fn quoted_deferred_and_copied_selectors_do_not_exclude_real_includes() {
    for prefix in [
        r"\ifblank{\includeonly{}}{Y}{N}",
        r"\newcommand{\later}{\includeonly{}}",
        r"\let\saved\includeonly{}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{prefix}\\begin{{document}}\\include{{definitions}}\\ifblank{{\\booltrue{{draft}}}}\\end{{document}}");
        let (_directory, workspace, main) =
            setup(&[("main.tex", &source), ("definitions.tex", REDEFINITION)]);
        assert_eq!(
            features::definitions(&workspace, &main, position(&source, "draft")).len(),
            1,
            "{prefix}"
        );
    }
}

#[test]
fn unsaved_selector_edits_update_main_semantics_without_changing_main_version() {
    let source = r"\documentclass{article}\newbool{draft}\input{settings}\begin{document}\include{definitions}\ifblank{\booltrue{draft}}{Y}{N}\end{document}";
    let (directory, mut workspace, main) = setup(&[
        ("main.tex", source),
        ("settings.tex", r"\includeonly{}"),
        ("definitions.tex", REDEFINITION),
    ]);
    let settings = Url::from_file_path(directory.path().join("settings.tex")).unwrap();
    workspace.open(main.clone(), source.into(), 9);
    let target = position(source, "draft");
    assert!(features::definitions(&workspace, &main, target).is_empty());
    workspace.open(settings.clone(), r"\includeonly{definitions}".into(), 1);
    assert_eq!(features::definitions(&workspace, &main, target).len(), 1);
    assert_eq!(workspace.documents[&main].version, Some(9));
    workspace.open(settings, r"\includeonly{}".into(), 2);
    assert!(features::definitions(&workspace, &main, target).is_empty());
}

#[test]
fn shared_sources_keep_each_roots_selection_and_reject_only_ambiguous_edits() {
    let shared = r"\ifblank{\booltrue{draft}}{Y}{N}\boolfalse{other}";
    let (directory, workspace, _) = setup(&[
        (
            "a.tex",
            r"\documentclass{article}\input{booleans}\includeonly{}\begin{document}\include{definitions}\input{shared}\end{document}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\input{booleans}\includeonly{definitions}\begin{document}\include{definitions}\input{shared}\end{document}",
        ),
        ("booleans.tex", r"\newbool{draft}\newbool{other}"),
        ("definitions.tex", REDEFINITION),
        ("shared.tex", shared),
    ]);
    let declarations = Url::from_file_path(directory.path().join("booleans.tex")).unwrap();
    let shared_uri = Url::from_file_path(directory.path().join("shared.tex")).unwrap();
    assert!(features::rename(&workspace, &declarations, Position::new(0, 10), "release").is_err());
    assert!(
        features::prepare_rename(&workspace, &shared_uri, position(shared, "draft"))
            .unwrap()
            .is_none()
    );
    assert!(features::rename(
        &workspace,
        &shared_uri,
        position(shared, "other"),
        "another"
    )
    .unwrap()
    .is_some());
}

#[test]
fn unknown_selection_rejects_only_affected_data_edits_after_an_actual_include() {
    for selector in [
        r"\includeonly{\chapters}",
        r"\def\includeonly#1{}\includeonly{}",
        r"\def\include#1{}\includeonly{definitions}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\newbool{{other}}{selector}\\begin{{document}}\\include{{definitions}}\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}\\boolfalse{{other}}\\end{{document}}");
        let (_directory, workspace, main) =
            setup(&[("main.tex", &source), ("definitions.tex", REDEFINITION)]);
        let declaration = Text::new(source.clone()).position(source.find("draft").unwrap());
        assert!(
            features::rename(&workspace, &main, declaration, "release").is_err(),
            "{selector}"
        );
        assert!(
            features::prepare_rename(&workspace, &main, position(&source, "draft"))
                .unwrap()
                .is_none(),
            "{selector}"
        );
        assert!(
            features::rename(&workspace, &main, position(&source, "other"), "another")
                .unwrap()
                .is_some(),
            "{selector}"
        );
    }
    for selector in [
        r"\includeonly{\chapters}",
        r"\def\includeonly#1{}\includeonly{}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{selector}\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}\\boolfalse{{draft}}");
        let (_directory, workspace, main) =
            setup(&[("main.tex", &source), ("unused.tex", REDEFINITION)]);
        let edit = features::rename(&workspace, &main, position(&source, "draft"), "release")
            .unwrap()
            .unwrap();
        assert!(apply(&workspace, &edit, &main).contains(r"\ifblank{\booltrue{draft}}"));
    }
}

#[test]
fn conditional_hooks_and_their_loaded_sources_replace_selection_in_execution_order() {
    for (prefix, selected) in [
        (
            r"\IfFileExists{./settings}{\includeonly{}}{\includeonly{definitions}}",
            false,
        ),
        (
            r"\IfFileExists{./missing}{\includeonly{definitions}}{\includeonly{}}",
            false,
        ),
        (r"\InputIfFileExists{./settings}{\includeonly{}}{}", true),
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}{prefix}\\begin{{document}}\\include{{definitions}}\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}\\end{{document}}");
        let (_directory, workspace, main) = setup(&[
            ("main.tex", &source),
            ("settings.tex", r"\includeonly{definitions}"),
            ("definitions.tex", REDEFINITION),
        ]);
        assert_eq!(
            features::definitions(&workspace, &main, position(&source, "draft")).len(),
            usize::from(selected),
            "{prefix}"
        );
    }
}

#[test]
fn selection_removes_only_one_tex_suffix_and_preserves_distinct_chapter_names() {
    for (selection, include, selected) in [
        ("definitions.tex.tex", "definitions.tex.tex", true),
        ("definitions.tex.tex", "definitions.tex", false),
        ("definitions.tex", "definitions.tex.tex", false),
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\includeonly{{{selection}}}\\begin{{document}}\\include{{{include}}}\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}\\end{{document}}");
        let (_directory, workspace, main) = setup(&[
            ("main.tex", &source),
            ("definitions.tex", REDEFINITION),
            ("definitions.tex.tex", REDEFINITION),
        ]);
        assert_eq!(
            features::definitions(&workspace, &main, position(&source, "draft")).len(),
            usize::from(selected),
            "{selection}/{include}"
        );
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_selected_chapters_preserve_boolean_states_and_renames_in_three_engines() {
    use std::os::unix::fs::PermissionsExt;
    use texquill::{compilation::build, config::BuildConfig};

    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for (selection, selected) in [
            ("", true),
            (r"\includeonly{}", false),
            (r"\includeonly{definitions.tex}", true),
            (r"\includeonly{./definitions}", false),
            (r"\includeonly{definitions}\input{settings}", false),
            (r"\input{settings}\includeonly{definitions}", true),
        ] {
            let source = format!(concat!(
                "\\documentclass{{article}}\\usepackage{{etoolbox}}\\newbool{{draft}}\n",
                "{selection}\n\\begin{{document}}\n",
                "\\ifblank{{\\booltrue{{draft}}}}{{}}{{\\typeout{{SELECT:native}}}}\n",
                "\\include{{definitions}}\n",
                "\\ifblank{{\\booltrue{{draft}}}}{{}}{{}}\n",
                "\\ifbool{{draft}}{{\\typeout{{SELECT:true}}}}{{\\typeout{{SELECT:false}}}}Text.\\end{{document}}\n"
            ), selection=selection);
            let child = format!("\\typeout{{SELECT:child}}{REDEFINITION}");
            let (directory, workspace, main) = setup(&[
                ("main.tex", &source),
                ("settings.tex", r"\includeonly{}"),
                ("definitions.tex", &child),
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
            assert_eq!(
                before.status, "success",
                "{engine}/{selection}: {}",
                before.output
            );
            let target =
                Text::new(source.clone()).position(source.find("newbool{draft}").unwrap() + 8);
            let edit = features::rename(&workspace, &main, target, "release")
                .unwrap()
                .unwrap();
            let changed = apply(&workspace, &edit, &main);
            // The first call always contains literal tokens. Only a selected
            // redefinition makes the second call's boolean executable.
            assert_eq!(
                changed.matches(r"\booltrue{release}").count(),
                usize::from(selected)
            );
            assert!(changed.contains("\\ifblank{\\booltrue{draft}}{}{"));
            for uri in edit.changes.as_ref().unwrap().keys() {
                std::fs::write(uri.to_file_path().unwrap(), apply(&workspace, &edit, uri)).unwrap();
            }
            let after = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(
                after.status, "success",
                "{engine}/{selection}: {}",
                after.output
            );
            let states = |output: &str| {
                output
                    .lines()
                    .filter(|line| line.starts_with("SELECT:"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            let expected = if selected {
                vec!["SELECT:native", "SELECT:child", "SELECT:true"]
            } else {
                vec!["SELECT:native", "SELECT:false"]
            };
            assert_eq!(states(&before.output), expected, "{engine}/{selection}");
            assert_eq!(states(&after.output), states(&before.output));
            assert_eq!(before.pages, Some(1));
            assert_eq!(after.pages, before.pages);
        }
    }
}
