use lsp_types::*;
use texquill::{language::features, project::workspace::Workspace, text::Text};

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
fn undefinition_operands_do_not_swallow_following_inputs_or_execute_their_old_grammar() {
    for removal in [
        r"\undef\saved",
        r"\gundef{\saved}",
        r"\csundef{saved}",
        r"\csgundef{saved}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\let\\saved\\ifblank{removal}\\input{{custom}}\\saved{{\\booltrue{{draft}}}}\\let\\saved\\ifblank\\saved{{\\boolfalse{{draft}}}}{{Y}}{{N}}");
        let (directory, workspace, main) = setup(&[
            ("main.tex", &source),
            ("custom.tex", r"\newcommand{\saved}[1]{#1}"),
        ]);
        let custom = Url::from_file_path(directory.path().join("custom.tex")).unwrap();
        assert!(
            workspace.project_documents(&main).contains(&custom),
            "{removal}"
        );
        let at = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
        let edit = features::rename(&workspace, &main, at, "release")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &main),
            source
                .replacen("newbool{draft}", "newbool{release}", 1)
                .replace("booltrue{draft}", "booltrue{release}"),
            "{removal}"
        );
    }
    for target in ["input", "label", "newcommand", "verb"] {
        let source =
            format!("\\documentclass{{article}}\\undef\\{target}{{ghost}}\\newbool{{real}}");
        let (directory, workspace, main) =
            setup(&[("main.tex", &source), ("ghost.tex", r"\newbool{ghost}")]);
        let parsed = &workspace.documents[&main].syntax;
        assert!(parsed.labels.is_empty(), "{target}");
        assert!(parsed.definitions.is_empty(), "{target}");
        assert!(
            parsed.conditionals.iter().any(|entry| entry.name == "real"),
            "{target}"
        );
        assert!(
            parsed.diagnostics.is_empty(),
            "{target}: {:?}",
            parsed.diagnostics
        );
        let ghost = Url::from_file_path(directory.path().join("ghost.tex")).unwrap();
        assert!(
            !workspace.project_documents(&main).contains(&ghost),
            "{target}"
        );
    }
}

#[test]
fn removing_string_meanings_invalidates_new_copies_but_preserves_earlier_captures() {
    for removal in [
        r"\undef\ifblank",
        r"\gundef{\ifblank}",
        r"\csundef{ifblank}",
        r"\csgundef{ifblank}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\let\\original\\ifblank{removal}\\let\\saved\\ifblank\\let\\ifblank\\saved\\ifblank{{\\booltrue{{draft}}}}\\input{{after}}\\original{{\\boolfalse{{draft}}}}{{Y}}{{N}}");
        let (directory, workspace, main) =
            setup(&[("main.tex", &source), ("after.tex", r"\newbool{after}")]);
        let at = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
        let edit = features::rename(&workspace, &main, at, "release")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &main),
            source
                .replacen("newbool{draft}", "newbool{release}", 1)
                .replace("booltrue{draft}", "booltrue{release}"),
            "{removal}"
        );
        let after = Url::from_file_path(directory.path().join("after.tex")).unwrap();
        assert!(
            workspace.project_documents(&main).contains(&after),
            "{removal}"
        );
    }
}

#[test]
fn named_undefinitions_without_a_copy_or_redefinition_still_refresh_string_semantics() {
    for removal in [r"\undef\ifblank", r"\csundef{ifblank}"] {
        let source = format!(
            "\\documentclass{{article}}\\newbool{{draft}}{removal}\\ifblank{{\\booltrue{{draft}}}}"
        );
        let (_directory, workspace, main) = setup(&[("main.tex", &source)]);
        let at = Text::new(source.clone()).position(source.rfind("draft").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, &main, at).len(),
            1,
            "{removal}"
        );
    }
}

#[test]
fn removal_targets_remain_macro_references_and_offer_control_operand_signatures() {
    use texquill::language::signature;
    for removal in [
        r"\undef\saved",
        r"\gundef{\saved}",
        r"\csundef{saved}",
        r"\csgundef{saved}",
    ] {
        let source =
            format!("\\documentclass{{article}}\\let\\saved\\ifblank{removal}\\newbool{{real}}");
        let (_directory, workspace, main) = setup(&[("main.tex", &source)]);
        let operand = Text::new(source.clone()).position(source.rfind("saved").unwrap() + 1);
        let edit = features::rename(&workspace, &main, operand, "original")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &main),
            source.replace("saved", "original"),
            "{removal}"
        );
        let at = source.find(removal).unwrap() + removal.find("saved").unwrap() + 2;
        let help =
            signature::help(&workspace, &main, Text::new(source.clone()).position(at)).unwrap();
        let command = removal
            .split(['\\', '{'])
            .find(|part| !part.is_empty())
            .unwrap();
        assert_eq!(
            help.signatures[0].label,
            format!("\\{command}{{command}}"),
            "{removal}"
        );
        assert!(
            !features::definitions(&workspace, &main, Text::new(source.clone()).position(at))
                .is_empty(),
            "{removal}"
        );
    }
}

#[test]
fn quoted_deferred_and_redefined_removal_commands_do_not_erase_captured_meanings() {
    for decoy in [
        r"\ifblank{\undef\saved}{Y}{N}",
        r"\newcommand{\later}{\csundef{saved}}",
        r"\renewcommand{\csundef}[1]{}\csundef{saved}",
        r"\renewcommand{\undef}[1]{}\undef{\saved}",
        r"\renewcommand{\undef}[1]{}\undef\saved",
        r"\let\undef\ifblank\undef{\csundef{saved}}{Y}{N}",
    ] {
        let source = format!("\\documentclass{{article}}\\newbool{{draft}}\\let\\saved\\ifblank{decoy}\\saved{{\\booltrue{{draft}}}}{{Y}}{{N}}");
        let (_directory, workspace, main) = setup(&[("main.tex", &source)]);
        let at = Text::new(source.clone()).position(source.rfind("draft").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &main, at).is_empty(),
            "{decoy}"
        );
        let at = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
        let edit = features::rename(&workspace, &main, at, "release")
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten(&workspace, edit, &main),
            source.replacen("newbool{draft}", "newbool{release}", 1),
            "{decoy}"
        );
    }
}

#[test]
fn dynamic_and_shared_removal_targets_guard_affected_operands_only() {
    let source = r"\documentclass{article}\newbool{draft}\newbool{other}\let\saved\ifblank\csundef{\name}\saved{\booltrue{draft}}{Y}{N}\boolfalse{other}";
    let (_directory, workspace, main) = setup(&[("main.tex", source)]);
    let at = Text::new(source.into()).position(source.find("draft").unwrap() + 1);
    assert!(features::rename(&workspace, &main, at, "release").is_err());
    let at = Text::new(source.into()).position(source.rfind("other").unwrap() + 1);
    assert!(features::rename(&workspace, &main, at, "another")
        .unwrap()
        .is_some());
    let (directory, workspace, _) = setup(&[
        (
            "a.tex",
            r"\documentclass{article}\input{flags}\let\saved\ifblank\input{shared}",
        ),
        (
            "b.tex",
            r"\documentclass{article}\input{flags}\let\saved\ifblank\input{remove}\input{shared}",
        ),
        ("remove.tex", r"\csundef{saved}"),
        ("flags.tex", r"\newbool{draft}\newbool{other}"),
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
fn real_removals_and_redefinitions_preserve_rename_results_in_three_engines() {
    use std::os::unix::fs::PermissionsExt;
    use texquill::{compilation::build, config::BuildConfig};
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for predicate in ["ifblank", "notblank", "ifstrempty", "ifstrequal"] {
            for removal in [
                r"\undef\saved",
                r"\gundef{\saved}",
                r"\csundef{saved}",
                r"\csgundef{saved}",
            ] {
                let extra = if predicate == "ifstrequal" {
                    "{Never}"
                } else {
                    ""
                };
                let branches = |marker: &str| {
                    if predicate == "notblank" {
                        format!("{{\\typeout{{UNDEF:{marker}}}}}{{\\typeout{{UNDEF:wrong}}}}")
                    } else {
                        format!("{{\\typeout{{UNDEF:wrong}}}}{{\\typeout{{UNDEF:{marker}}}}}")
                    }
                };
                let source = format!("\\documentclass{{article}}\\usepackage{{etoolbox}}\\newbool{{draft}}\\let\\original\\{predicate}\\let\\saved\\{predicate}\\begin{{document}}\\original{{\\boolfalse{{draft}}}}{extra}{}{removal}\\input{{custom}}\\saved{{\\booltrue{{draft}}}}\\original{{\\boolfalse{{draft}}}}{extra}{}\\let\\saved\\original\\saved{{\\boolfalse{{draft}}}}{extra}{}\\ifbool{{draft}}{{\\typeout{{UNDEF:true}}}}{{Wrong.}}Text.\\end{{document}}", branches("before"), branches("after"), branches("restored"));
                let (directory, mut workspace, main) = setup(&[
                    ("main.tex", &source),
                    (
                        "custom.tex",
                        r"\newcommand{\saved}[1]{#1\typeout{UNDEF:custom}}",
                    ),
                ]);
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
                    "{engine}/{predicate}/{removal}: {}",
                    before.output
                );
                let at = Text::new(source.clone()).position(source.find("original").unwrap() + 1);
                let edit = features::rename(&workspace, &main, at, "originalTokens")
                    .unwrap()
                    .unwrap();
                let renamed = rewritten(&workspace, edit, &main);
                assert_eq!(renamed.matches("originalTokens").count(), 4);
                workspace.open(main.clone(), renamed.clone(), 2);
                let at = Text::new(renamed.clone()).position(renamed.find("draft").unwrap() + 1);
                let edit = features::rename(&workspace, &main, at, "release")
                    .unwrap()
                    .unwrap();
                let renamed = rewritten(&workspace, edit, &main);
                assert_eq!(
                    renamed.matches(r"\boolfalse{draft}").count(),
                    3,
                    "{removal}"
                );
                assert!(renamed.contains(r"\booltrue{release}"));
                std::fs::write(main.to_file_path().unwrap(), renamed).unwrap();
                let after = build::run(&main.to_file_path().unwrap(), &config, &Default::default());
                assert_eq!(
                    after.status, "success",
                    "{engine}/{predicate}/{removal}: {}",
                    after.output
                );
                let states = |output: &str| {
                    output
                        .lines()
                        .filter(|line| line.starts_with("UNDEF:"))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    states(&before.output),
                    [
                        "UNDEF:before",
                        "UNDEF:custom",
                        "UNDEF:after",
                        "UNDEF:restored",
                        "UNDEF:true"
                    ],
                    "{engine}/{predicate}/{removal}"
                );
                assert_eq!(states(&after.output), states(&before.output));
                assert_eq!(before.pages, Some(1));
                assert_eq!(after.pages, before.pages);
            }
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pdflatex, xelatex, lualatex and etoolbox"]
fn real_undefined_copies_report_the_control_and_leave_following_groups_executable() {
    use std::os::unix::fs::PermissionsExt;
    use texquill::{compilation::build, config::BuildConfig};
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for removal in [r"\undef\saved", r"\csundef{saved}"] {
            let source = format!("\\documentclass{{article}}\\usepackage{{etoolbox}}\\newbool{{draft}}\\let\\saved\\ifblank\\begin{{document}}{removal}\n\\saved{{\\booltrue{{draft}}\\ifbool{{draft}}{{\\typeout{{UNDEF:executed}}}}{{Wrong.}}}}\\ifbool{{draft}}{{Wrong.}}{{\\typeout{{UNDEF:local}}}}Text.\\end{{document}}");
            let (directory, workspace, main) = setup(&[("main.tex", &source)]);
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
            let before = build::run(&main.to_file_path().unwrap(), &config, &Default::default());
            assert_eq!(
                before.status, "error",
                "{engine}/{removal}: {}",
                before.output
            );
            let error = before.diagnostics[&main]
                .iter()
                .find(|error| error.message == "Undefined control sequence.")
                .unwrap();
            let at = source.rfind(r"\saved").unwrap();
            assert_eq!(
                error.range,
                Text::new(source.clone()).range(at, at + 6),
                "{engine}/{removal}: {error:?}"
            );
            for marker in ["UNDEF:executed", "UNDEF:local"] {
                assert!(
                    before.output.lines().any(|line| line == marker),
                    "{engine}/{removal}: {}",
                    before.output
                );
            }
            // Failed builds do not publish successful PDF metadata.
            assert!(before.pdf.is_none());
            assert_eq!(before.pages, None);
            let at = Text::new(source.clone()).position(source.find("draft").unwrap() + 1);
            let edit = features::rename(&workspace, &main, at, "release")
                .unwrap()
                .unwrap();
            let renamed = rewritten(&workspace, edit, &main);
            assert_eq!(renamed, source.replace("draft", "release"));
            std::fs::write(main.to_file_path().unwrap(), renamed).unwrap();
            let after = build::run(&main.to_file_path().unwrap(), &config, &Default::default());
            assert_eq!(after.status, "error");
            for marker in ["UNDEF:executed", "UNDEF:local"] {
                assert!(
                    after.output.lines().any(|line| line == marker),
                    "{engine}/{removal}: {}",
                    after.output
                );
            }
            assert!(after.pdf.is_none());
            assert_eq!(after.pages, None);
        }
    }
}
