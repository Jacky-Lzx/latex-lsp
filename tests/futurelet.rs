use latex_lsp::{build, features, signature, text::Text, workspace::Workspace};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn changed_sources(workspace: &Workspace, edit: WorkspaceEdit) -> BTreeMap<Url, String> {
    edit.changes
        .unwrap()
        .into_iter()
        .map(|(uri, mut edits)| {
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let text = workspace.documents[&uri]
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
                .unwrap();
            (uri, text.source)
        })
        .collect()
}

#[test]
fn futurelet_targets_support_cross_file_navigation_and_independent_rename() {
    for target in [
        r"\futurelet\lookahead",
        "\\global\\futurelet % target\n\\lookahead",
        r"\expandafter\futurelet\csname lookahead\endcsname",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let definitions =
            format!("\\def\\inspect#1{{\\ifx\\lookahead A#1\\fi}}\n{target}\\inspect A\n");
        let source = "\\documentclass{article}\\input{definitions}\n\\begin{document}😀 \\lookahead\\csname lookahead\\endcsname\n% \\lookahead\n\\verb|\\lookahead|\\end{document}";
        let workspace = project(
            directory.path(),
            &[("main.tex", source), ("definitions.tex", &definitions)],
        );
        let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let definitions_uri =
            Url::from_file_path(directory.path().join("definitions.tex")).unwrap();
        let doc = &workspace.documents[&main];
        let position = doc.text.position(source.find("lookahead").unwrap() + 2);
        let locations = features::definitions(&workspace, &main, position);
        assert_eq!(locations.len(), 1, "{target}");
        assert_eq!(locations[0].uri, definitions_uri);
        let entry = workspace.documents[&definitions_uri]
            .syntax
            .definitions
            .iter()
            .find(|entry| entry.name == "lookahead")
            .unwrap();
        assert_eq!(entry.detail, "Defined with \\futurelet");
        assert_eq!(
            features::references(&workspace, &main, position, true).len(),
            4
        );
        assert!(features::hover(&workspace, &main, position).is_some());
        assert!(features::completion(&workspace, &main, position)
            .iter()
            .any(|item| item.label == "lookahead"));
        assert!(features::symbols(&workspace.documents[&definitions_uri])
            .iter()
            .any(|symbol| symbol.name == "lookahead"));
        assert!(features::prepare_rename(&workspace, &main, position)
            .unwrap()
            .is_some());
        let changed = changed_sources(
            &workspace,
            features::rename(&workspace, &main, position, "renamedlookahead")
                .unwrap()
                .unwrap(),
        );
        assert_eq!(changed.len(), 2);
        assert_eq!(
            changed[&definitions_uri],
            definitions.replace("lookahead", "renamedlookahead")
        );
        let (body, rest) = source.split_once("\n% ").unwrap();
        assert_eq!(
            changed[&main],
            format!(
                "{}\n% {rest}",
                body.replace("lookahead", "renamedlookahead")
            )
        );
        for conflict in ["inspect", "futurelet", "input"] {
            assert!(features::rename(&workspace, &main, position, conflict).is_err());
        }
        assert!(features::rename(&workspace, &main, position, "前瞻").is_err());
    }
}

#[test]
fn futurelet_static_names_allow_unicode_without_inventing_a_call_signature() {
    let directory = tempfile::tempdir().unwrap();
    let declaration = r"\expandafter\futurelet\csname lookahead\endcsname\inspect\original";
    let source = format!(
        r"\def\inspect{{}}\def\original#1{{#1}}{declaration}\csname lookahead\endcsname{{Value}}"
    );
    let workspace = project(directory.path(), &[("main.tex", &source)]);
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let text = &workspace.documents[&uri].text;
    let position = text.position(source.find("lookahead").unwrap());
    let changed = changed_sources(
        &workspace,
        features::rename(&workspace, &uri, position, "前瞻-符号")
            .unwrap()
            .unwrap(),
    );
    assert_eq!(changed[&uri], source.replace("lookahead", "前瞻-符号"));
    assert!(signature::help(
        &workspace,
        &uri,
        text.position(source.find("Value").unwrap() + 2)
    )
    .is_none());
}

#[test]
fn futurelet_rejects_braced_dynamic_and_unexpanded_csname_targets() {
    for source in [
        r"\futurelet{\lookahead}\inspect A",
        r"\futurelet\csname lookahead\endcsname\inspect A",
        r"\expandafter\futurelet\csname look\dynamic\endcsname\inspect A",
        "% \\futurelet\\lookahead\\inspect A\n\\verb|\\futurelet\\lookahead\\inspect A|",
    ] {
        let parsed = latex_lsp::syntax::Syntax::latex(&Text::new(source.into()));
        assert!(
            parsed
                .definitions
                .iter()
                .all(|entry| entry.name != "lookahead"),
            "{source}"
        );
    }
}

#[test]
fn futurelet_continuations_remain_active_when_the_target_names_a_conversion_primitive() {
    for target in [
        r"\futurelet\lookahead",
        r"\futurelet\string",
        r"\futurelet\detokenize",
        r"\expandafter\futurelet\csname lookahead\endcsname",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!(
            r"\documentclass{{article}}\begin{{document}}{target}\input{{child}}\fromchild\end{{document}}"
        );
        let workspace = project(
            directory.path(),
            &[
                ("main.tex", &source),
                ("child.tex", r"\newcommand{\fromchild}{Loaded}"),
            ],
        );
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let doc = &workspace.documents[&uri];
        assert!(
            doc.syntax
                .commands
                .iter()
                .any(|command| command.name == "input"),
            "{target}"
        );
        let position = doc.text.position(source.find("fromchild").unwrap() + 2);
        let definitions = features::definitions(&workspace, &uri, position);
        assert_eq!(definitions.len(), 1, "{target}");
        assert_eq!(
            definitions[0].uri,
            Url::from_file_path(directory.path().join("child.tex")).unwrap()
        );
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_futurelet_renames_preserve_lookahead_and_execute_the_continuation() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for named in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let target = if named {
                r"\expandafter\futurelet\csname lookahead\endcsname"
            } else {
                r"\futurelet\lookahead"
            };
            let compare = if named {
                r"\expandafter\ifx\csname lookahead\endcsname A"
            } else {
                r"\ifx\lookahead A"
            };
            let definitions = format!("\\def\\inspect#1{{{compare}\\typeout{{FUTURELET:yes}}\\else\\errmessage{{Wrong lookahead}}\\fi #1}}\n{target}\\inspect A\n");
            let source = format!("{compare}\\typeout{{FUTURELET:retained}}\\else\\errmessage{{Lost lookahead}}\\fi\n% \\lookahead\n\\verb|\\lookahead|\n");
            let workspace = project(root, &[
                ("main.tex", "\\documentclass{article}\\begin{document}\\input{definitions}\\input{content}\\end{document}\n"),
                ("definitions.tex", &definitions),
                ("content.tex", &source),
            ]);
            let compiler = root.join("compiler");
            std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: compiler.to_string_lossy().into(),
                arguments: Some(
                    [
                        "-interaction=nonstopmode",
                        "-synctex=1",
                        "-recorder",
                        "-output-directory=%o",
                        "%f",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                ),
                ..Default::default()
            };
            let before = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(
                before.status, "success",
                "{engine}/{named}: {}",
                before.output
            );
            let uri = Url::from_file_path(root.join("definitions.tex")).unwrap();
            let position = workspace.documents[&uri]
                .text
                .position(definitions.rfind("lookahead").unwrap());
            let new_name = if named {
                "前瞻-符号"
            } else {
                "renamedlookahead"
            };
            let changes = changed_sources(
                &workspace,
                features::rename(&workspace, &uri, position, new_name)
                    .unwrap()
                    .unwrap(),
            );
            assert_eq!(changes.len(), 2);
            for (uri, source) in changes {
                std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
            }
            let after = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(
                after.status, "success",
                "{engine}/{named}: {}",
                after.output
            );
            for outcome in [before, after] {
                let observed: Vec<_> = outcome
                    .output
                    .lines()
                    .filter(|line| line.starts_with("FUTURELET:"))
                    .collect();
                assert_eq!(
                    observed,
                    ["FUTURELET:yes", "FUTURELET:retained"],
                    "{engine}/{named}"
                );
            }
        }
    }
}
