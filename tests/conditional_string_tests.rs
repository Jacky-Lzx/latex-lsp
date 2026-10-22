use latex_lsp::{features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

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

fn apply(workspace: &Workspace, edit: WorkspaceEdit, uri: &Url) -> String {
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
fn conditional_pre_input_definitions_reach_children_before_their_calls() {
    let source = concat!(
        "\\documentclass{article}\\newbool{draft}\\newcommand{\\identity}[1]{#1}\n",
        "\\ifblank{\\boolfalse{draft}}{Y}{N}\n",
        "\\InputIfFileExists{./checked}{\\let\\ifblank\\identity}{}\n",
    );
    let child = r"\ifblank{\booltrue{draft}}";
    let (directory, workspace, main) = setup(&[("main.tex", source), ("checked.tex", child)]);
    let checked = Url::from_file_path(directory.path().join("checked.tex")).unwrap();
    let target = position(child, "draft");
    assert_eq!(features::definitions(&workspace, &checked, target).len(), 1);
    let edit = features::rename(&workspace, &checked, target, "release")
        .unwrap()
        .unwrap();
    assert_eq!(
        apply(&workspace, edit.clone(), &checked),
        child.replace("draft", "release")
    );
    assert_eq!(
        apply(&workspace, edit, &main),
        source.replacen("newbool{draft}", "newbool{release}", 1)
    );
}

#[test]
fn only_the_selected_local_file_hook_changes_later_predicate_meanings() {
    for name in ["InputIfFileExists", "IfFileExists"] {
        for present in [false, true] {
            let hook = r"\let\ifblank\identity";
            for redefine_when_found in [false, true] {
                let (found, missing) = if redefine_when_found {
                    (hook, "")
                } else {
                    ("", hook)
                };
                let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\newcommand{{\\identity}}[1]{{#1}}\\{name}{{./checked}}{{{found}}}{{{missing}}}\n\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}\n");
                let mut files = vec![("main.tex", source.as_str())];
                if present {
                    files.push(("checked.tex", ""));
                }
                let (_directory, workspace, main) = setup(&files);
                assert_eq!(
                    features::definitions(&workspace, &main, position(&source, "draft")).len(),
                    usize::from(present == redefine_when_found),
                    "{name}/{present}/{redefine_when_found}"
                );
            }
        }
    }
}

#[test]
fn nested_checks_and_pre_input_loads_propagate_in_execution_order() {
    let source = concat!(
        "\\documentclass{article}\\newbool{draft}\n",
        "\\IfFileExists{./checked}{% found 😀\r\n",
        "  \\InputIfFileExists % file\r\n{./checked}% hook\r\n",
        "  {\\input{definitions}}{}\r\n}{}\n",
    );
    let child = r"\ifblank{\booltrue{draft}}";
    let (directory, workspace, _) = setup(&[
        ("main.tex", source),
        ("checked.tex", child),
        ("definitions.tex", r"\renewcommand{\ifblank}[1]{#1}"),
    ]);
    let checked = Url::from_file_path(directory.path().join("checked.tex")).unwrap();
    assert_eq!(
        features::definitions(&workspace, &checked, position(child, "draft")).len(),
        1
    );
}

#[test]
fn existence_checks_do_not_execute_checked_source_definitions() {
    let source = "\\documentclass{article}\\newbool{draft}\\IfFileExists{./checked}{}{}\n\\ifblank{\\booltrue{draft}}{Y}{N}\n";
    let (_directory, workspace, main) = setup(&[
        ("main.tex", source),
        ("checked.tex", r"\renewcommand{\ifblank}[1]{#1}"),
    ]);
    assert!(features::definitions(&workspace, &main, position(source, "draft")).is_empty());
}

#[test]
fn non_source_file_creation_and_deletion_refresh_the_selected_hook() {
    let source = "\\documentclass{article}\\newbool{draft}\\newcommand{\\identity}[1]{#1}\\IfFileExists{./gate.csv}{\\let\\ifblank\\identity}{}\n\\ifblank{\\booltrue{draft}}{Y}{N}\n";
    let (directory, mut workspace, main) = setup(&[("main.tex", source)]);
    let gate = Url::from_file_path(directory.path().join("gate.csv")).unwrap();
    workspace.open(main.clone(), source.into(), 9);
    let target = position(source, "draft");
    assert!(features::definitions(&workspace, &main, target).is_empty());
    std::fs::write(gate.to_file_path().unwrap(), "Data,not TeX").unwrap();
    latex_lsp::file_operations::FileChanges::new([gate.as_str()])
        .unwrap()
        .refresh(&mut workspace);
    assert_eq!(features::definitions(&workspace, &main, target).len(), 1);
    assert!(!workspace.documents.contains_key(&gate));
    assert_eq!(workspace.documents[&main].version, Some(9));
    std::fs::remove_file(gate.to_file_path().unwrap()).unwrap();
    latex_lsp::file_operations::FileChanges::new([gate.as_str()])
        .unwrap()
        .refresh(&mut workspace);
    assert!(features::definitions(&workspace, &main, target).is_empty());
}

#[test]
fn uncertain_file_conditions_reject_affected_edits_but_preserve_other_symbols() {
    for condition in [
        r"\IfFileExists{unresolved-system-name}{\let\ifblank\identity}{}",
        r"\IfFileExists{\dynamic}{\let\ifblank\identity}{}",
        r"\def\IfFileExists#1#2#3{#2}\IfFileExists{./gate}{\let\ifblank\identity}{}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\newbool{{other}}\\newcommand{{\\identity}}[1]{{#1}}{condition}\n\\ifblank{{\\booltrue{{draft}}}}{{Y}}{{N}}\\boolfalse{{other}}");
        let (_directory, workspace, main) = setup(&[("main.tex", &source)]);
        let draft = Text::new(source.clone()).position(source.find("draft").unwrap());
        assert!(
            features::rename(&workspace, &main, draft, "release").is_err(),
            "{condition}"
        );
        assert!(
            features::rename(&workspace, &main, position(&source, "other"), "another")
                .unwrap()
                .is_some(),
            "{condition}"
        );
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_conditional_hook_renames_preserve_state_and_input_order_in_three_engines() {
    use latex_lsp::{build, config::BuildConfig};
    use std::os::unix::fs::PermissionsExt;

    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for command in ["InputIfFileExists", "IfFileExists"] {
            for present in [false, true] {
                let hook = r"\typeout{HOOK:before}\let\ifblank\identity";
                let wrong = r"\typeout{HOOK:wrong}";
                let (found, missing) = if present {
                    (hook, wrong)
                } else {
                    (wrong, hook)
                };
                let main_call = if command == "InputIfFileExists" && present {
                    ""
                } else {
                    r"\ifblank{\booltrue{draft}}"
                };
                let source = format!(concat!(
                    "\\documentclass{{article}}\\usepackage{{etoolbox}}\\newbool{{draft}}\\newcommand{{\\identity}}[1]{{#1}}\n",
                    "\\begin{{document}}\n",
                    "\\ifblank{{\\boolfalse{{draft}}}}{{\\typeout{{HOOK:wrong}}}}{{\\typeout{{HOOK:native}}}}\n",
                    "\\{command}{{./checked}}{{{found}}}{{{missing}}}\n",
                    "{main_call}\n",
                    "\\ifbool{{draft}}{{\\typeout{{HOOK:true}}}}{{\\typeout{{HOOK:wrong}}}}Text.\\end{{document}}\n"
                ), command=command, found=found, missing=missing, main_call=main_call);
                let child = r"\typeout{HOOK:child}\ifblank{\booltrue{draft}}";
                let mut files = vec![("main.tex", source.as_str())];
                if present {
                    files.push(("checked.tex", child));
                }
                let (directory, workspace, main) = setup(&files);
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
                    "{engine}/{command}/{present}: {}",
                    before.output
                );
                let target =
                    Text::new(source.clone()).position(source.find("newbool{draft}").unwrap() + 8);
                let edit = features::rename(&workspace, &main, target, "release")
                    .unwrap()
                    .unwrap();
                let changed = apply(&workspace, edit.clone(), &main);
                assert!(changed.contains(r"\ifblank{\boolfalse{draft}}"));
                for uri in edit.changes.as_ref().unwrap().keys() {
                    std::fs::write(
                        uri.to_file_path().unwrap(),
                        apply(&workspace, edit.clone(), uri),
                    )
                    .unwrap();
                }
                let after = build::run(&root.join("main.tex"), &config, &Default::default());
                assert_eq!(
                    after.status, "success",
                    "{engine}/{command}/{present}: {}",
                    after.output
                );
                let states = |output: &str| {
                    output
                        .lines()
                        .filter(|line| line.starts_with("HOOK:"))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                let mut expected = vec!["HOOK:native", "HOOK:before"];
                if command == "InputIfFileExists" && present {
                    expected.push("HOOK:child");
                }
                expected.push("HOOK:true");
                assert_eq!(
                    states(&before.output),
                    expected,
                    "{engine}/{command}/{present}"
                );
                assert_eq!(states(&after.output), states(&before.output));
                assert_eq!(after.pages, Some(1));
            }
        }
    }
}

#[test]
fn shared_file_hooks_keep_each_roots_file_condition_and_edit_ambiguity() {
    let source = r"\IfFileExists{./gate.csv}{\let\ifblank\identity}{}\ifblank{\booltrue{draft}}{Y}{N}\boolfalse{other}";
    let (directory, workspace, _) = setup(&[
        (
            "a/main.tex",
            r"\documentclass{article}\input{../booleans}\input{../shared}",
        ),
        (
            "b/main.tex",
            r"\documentclass{article}\input{../booleans}\input{../shared}",
        ),
        (
            "booleans.tex",
            r"\newbool{draft}\newbool{other}\newcommand{\identity}[1]{#1}",
        ),
        ("shared.tex", source),
        ("a/gate.csv", "Data,not TeX"),
    ]);
    let declarations = Url::from_file_path(directory.path().join("booleans.tex")).unwrap();
    let draft = Position::new(0, 10);
    assert!(features::rename(&workspace, &declarations, draft, "release").is_err());
    let shared = Url::from_file_path(directory.path().join("shared.tex")).unwrap();
    assert!(
        features::prepare_rename(&workspace, &shared, position(source, "draft"))
            .unwrap()
            .is_none()
    );
    assert!(
        features::rename(&workspace, &shared, position(source, "other"), "another")
            .unwrap()
            .is_some()
    );
}
