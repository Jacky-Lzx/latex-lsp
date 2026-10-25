use latex_lsp::{
    compilation::build,
    editing::file_operations,
    language::{features, navigation, signature},
    project::workspace::Workspace,
    text::Text,
};
use lsp_types::*;
use std::path::Path;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    Text::new(source.into())
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
fn existence_checks_link_to_files_without_importing_their_symbols_or_static_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "\\documentclass{article}\n\\IfFileExists{checked}{Found.}{Missing.}\n\\begin{document}\\label{main-label}\\ref{checked-label}\\end{document}\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("checked.tex"), r"\label{checked-label}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    let checked = uri(root, "checked.tex");
    let links = navigation::document_links(&workspace, &main);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, Some(checked.clone()));
    assert!(workspace.dependencies(&main).is_empty());
    assert!(!workspace.project_files(&main).contains(&checked));
    let position = workspace.documents[&main]
        .text
        .position(source.find("checked-label").unwrap() + 3);
    assert!(features::definitions(&workspace, &main, position).is_empty());
    let position = workspace.documents[&main]
        .text
        .position(source.find("main-label").unwrap() + 3);
    let changes = features::rename(&workspace, &main, position, "checked-label")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        apply(source, &changes[&main]),
        source.replace("main-label", "checked-label")
    );
}

fn complete(workspace: &mut Workspace, main: &Url, marked: &str, label: &str) -> String {
    let at = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    workspace.open(main.clone(), source.clone(), 1);
    let position = Text::new(source.clone()).position(at);
    let items = features::completion(workspace, main, position);
    let item = items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{marked}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(edit.range.start.line, edit.range.end.line);
    assert!(edit.range.start <= position && position <= edit.range.end);
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    apply(&source, &edits)
}

#[test]
fn file_existence_completions_offer_arbitrary_extensions_extensionless_files_and_directories() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("assets")).unwrap();
    for name in [
        "main.tex",
        "notes.dat",
        "figure.pdf",
        "extensionless",
        "part,one.bin",
        "chapter.ltx",
        "shadow.dat",
        "shadow.dat.tex",
        "assets/数据 file.csv",
    ] {
        std::fs::write(root.join(name), "Not TeX.").unwrap();
    }
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (marked, label, expected) in [
        (
            r"\IfFileExists{not¦es}{Found.}{Missing.}",
            "notes.dat",
            r"\IfFileExists{notes.dat}{Found.}{Missing.}",
        ),
        (
            r"\IfFileExists{fig¦ure}{Found.}{Missing.}",
            "figure.pdf",
            r"\IfFileExists{figure.pdf}{Found.}{Missing.}",
        ),
        (
            r"\IfFileExists{ext¦}{Found.}{Missing.}",
            "extensionless",
            r"\IfFileExists{extensionless}{Found.}{Missing.}",
        ),
        (
            r"\IfFileExists{part¦,one}{Found.}{Missing.}",
            "part,one.bin",
            r"\IfFileExists{part,one.bin}{Found.}{Missing.}",
        ),
        (
            r"\IfFileExists{part,¦",
            "part,one.bin",
            r"\IfFileExists{part,one.bin",
        ),
        (
            r"\IfFileExists{chap¦ter}{Found.}{Missing.}",
            "chapter.ltx",
            r"\IfFileExists{chapter.ltx}{Found.}{Missing.}",
        ),
        (
            r"\IfFileExists{ass¦/数据 file.csv}{Found.}{Missing.}",
            "assets/",
            r"\IfFileExists{assets/数据 file.csv}{Found.}{Missing.}",
        ),
        (
            "😀 \\IfFileExists % keep\r\n{assets/数¦据 file}{Found.}{Missing.}",
            "assets/数据 file.csv",
            "😀 \\IfFileExists % keep\r\n{assets/数据 file.csv}{Found.}{Missing.}",
        ),
        (
            r"\IfFileExists{shad¦ow.dat}{Found.}{Missing.}",
            "shadow.dat.tex",
            r"\IfFileExists{shadow.dat}{Found.}{Missing.}",
        ),
    ] {
        assert_eq!(complete(&mut workspace, &main, marked, label), expected);
    }
    let source = r"\IfFileExists{shad}{Found.}{Missing.}";
    workspace.open(main.clone(), source.into(), 2);
    let items = features::completion(
        &workspace,
        &main,
        Text::new(source.into()).position(source.find("shad").unwrap() + 4),
    );
    assert!(
        !items.iter().any(|item| item.label == "shadow.dat"),
        "{items:?}"
    );
}

#[test]
fn checked_file_renames_preserve_both_branches_and_allow_extension_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("assets")).unwrap();
    let source = "😀 \\IfFileExists % keep\r\n{assets/data,part.bin} % branch\r\n{\\typeout{data,part}}{Missing data,part.}\r\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("assets/data,part.bin"), "Data.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    let old = uri(root, "assets/data,part.bin");
    let links = navigation::document_links(&workspace, &main);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, Some(old.clone()));
    let start = source.find("assets/data,part.bin").unwrap();
    assert_eq!(
        links[0].range,
        Text::new(source.into()).range(start, start + 20)
    );
    let changes = file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: old.to_string(),
            new_uri: uri(root, "assets/新 data.csv").to_string(),
        }],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[&main].len(), 1);
    assert_eq!(
        apply(source, &changes[&main]),
        source.replace("assets/data,part.bin", "assets/新 data.csv")
    );
}

#[test]
fn checked_paths_keep_the_intended_file_when_a_suffix_shadow_moves_into_place() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\IfFileExists{data.dat}{Found.}{Missing.}";
    for (name, content) in [
        ("main.tex", source),
        ("data.dat", "Intended."),
        ("decoy.tex", "Decoy."),
    ] {
        std::fs::write(root.join(name), content).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let changes = file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: uri(root, "decoy.tex").to_string(),
            new_uri: uri(root, "data.dat.tex").to_string(),
        }],
    );
    // The same literal filename would now test another file, and adding its
    // original extension still has the same TeX shadow. No safe edit exists.
    assert!(changes.unwrap_err().message.contains("different file"));
    let changes = file_operations::will_rename(
        &workspace,
        &[
            FileRename {
                old_uri: uri(root, "data.dat").to_string(),
                new_uri: uri(root, "moved.dat").to_string(),
            },
            FileRename {
                old_uri: uri(root, "decoy.tex").to_string(),
                new_uri: uri(root, "data.dat.tex").to_string(),
            },
        ],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(
        apply(source, &changes[&uri(root, "main.tex")]),
        r"\IfFileExists{moved.dat}{Found.}{Missing.}"
    );
}

#[test]
fn unsafe_or_nonexistent_checks_and_nonfilename_arguments_do_not_offer_file_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = concat!(
        "% \\IfFileExists{child}{}{}\n",
        "\\verb|\\IfFileExists{child}{}{}|\n",
        "\\IfFileExists{\\dynamic}{}{}\n",
        "\\IfFileExists*{child}{}{}\n",
        "\\IfFileExists[option]{child}{}{}\n",
        "\\IfFileExists{missing}{}{}\n",
    );
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("child.tex"), "Child.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    assert!(navigation::document_links(&workspace, &main).is_empty());
    assert!(file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: uri(root, "child.tex").to_string(),
            new_uri: uri(root, "renamed.tex").to_string(),
        }]
    )
    .unwrap()
    .is_none());
    for marked in [
        r"\IfFileExists*{ch¦ild}{}{}",
        r"\IfFileExists[option]{ch¦ild}{}{}",
        r"\IfFileExists{\dyn¦amic}{}{}",
        r"\IfFileExists{child}{ch¦ild}{}",
        r"\IfFileExists{child}{}{ch¦ild}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 1);
        assert!(
            features::completion(&workspace, &main, Text::new(source).position(offset))
                .iter()
                .all(|item| item.kind != Some(CompletionItemKind::FILE)),
            "{marked}"
        );
    }
}

#[test]
fn existence_check_command_snippets_and_signatures_show_the_two_branches() {
    let main = Url::parse("file:///file-exists/main.tex").unwrap();
    let mut workspace = Workspace {
        snippets: true,
        ..Default::default()
    };
    workspace.open(main.clone(), r"\IfFileEx".into(), 1);
    let items = features::completion(&workspace, &main, Position::new(0, 9));
    let item = items
        .iter()
        .find(|item| item.label == "IfFileExists")
        .unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(
        edit.new_text,
        "IfFileExists{${1:file}}{${2:if exists}}{${3:if missing}}"
    );
    for (marked, active) in [
        (r"\IfFileExists{fi¦le}{Found.}{Missing.}", 0),
        (r"\IfFileExists{file}{Fo¦und.}{Missing.}", 1),
        (r"\IfFileExists{file}{Found.}{Mis¦sing.}", 2),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 2);
        let help = signature::help(&workspace, &main, Text::new(source).position(offset)).unwrap();
        assert_eq!(
            help.signatures[0].label,
            r"\IfFileExists{file}{if exists}{if missing}"
        );
        assert_eq!(help.active_parameter, Some(active));
    }
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn real_file_checks_match_recorder_inputs_preserve_branch_behavior_and_never_execute_checked_text()
{
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for name in [
            "data.dat",
            "data.dat.tex",
            "picture.pdf",
            "picture.pdf.tex",
            "settings.sty",
            "settings.sty.tex",
            "extensionless",
            "part,one.bin",
        ] {
            std::fs::write(root.join(name), "\\undefinedcheckedcommand\n").unwrap();
        }
        let mut source = "\\documentclass{article}\\begin{document}\n".to_owned();
        for name in [
            "data.dat",
            "picture.pdf",
            "settings.sty",
            "extensionless",
            "part,one.bin",
        ] {
            source.push_str(&format!("\\IfFileExists{{{name}}}{{\\typeout{{CHECK-FOUND:{name}}}}}{{\\typeout{{CHECK-WRONG-BRANCH}}}}\n"));
        }
        source.push_str("\\IfFileExists{missing.xyz}{\\typeout{CHECK-WRONG-BRANCH}}{\\typeout{CHECK-MISSING}}\nHello.\\end{document}\n");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let config = latex_lsp::config::BuildConfig {
            executable: engine.into(),
            ..Default::default()
        };
        for renamed in [false, true] {
            let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(outcome.status, "success", "{}", outcome.output);
            assert!(outcome.output.contains("CHECK-MISSING"));
            assert!(!outcome.output.contains("CHECK-WRONG-BRANCH"));
            assert!(
                outcome.diagnostics.values().all(Vec::is_empty),
                "{:?}",
                outcome.diagnostics
            );
            // The server's source/image dependency set deliberately filters
            // arbitrary data files. Inspect TeX's actual open events here.
            let recorder =
                std::fs::read_to_string(outcome.log.unwrap().with_extension("fls")).unwrap();
            let inputs: std::collections::BTreeSet<_> = recorder
                .lines()
                .filter_map(|line| {
                    let path = root
                        .join(line.strip_prefix("INPUT ")?.trim_matches('"'))
                        .canonicalize()
                        .ok()?;
                    Url::from_file_path(path).ok()
                })
                .collect();
            for name in [
                if renamed {
                    "renamed data.csv"
                } else {
                    "data.dat.tex"
                },
                "picture.pdf.tex",
                "settings.sty",
                "extensionless",
                "part,one.bin",
            ] {
                assert!(inputs.contains(&uri(root, name)), "{name}: {inputs:?}");
            }
            if !renamed {
                let mut workspace = Workspace::default();
                workspace.add_root(root.into());
                let main = uri(root, "main.tex");
                assert!(workspace.dependencies(&main).is_empty());
                let links = navigation::document_links(&workspace, &main);
                assert_eq!(links.len(), 5, "{links:?}");
                for link in links {
                    assert!(inputs.contains(link.target.as_ref().unwrap()));
                }
                let changes = file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: uri(root, "data.dat.tex").to_string(),
                        new_uri: uri(root, "renamed data.csv").to_string(),
                    }],
                )
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
                let updated = apply(&source, &changes[&main]);
                assert_eq!(
                    updated,
                    source.replacen("{data.dat}", "{renamed data.csv}", 1)
                );
                std::fs::write(root.join("main.tex"), updated).unwrap();
                std::fs::rename(root.join("data.dat.tex"), root.join("renamed data.csv")).unwrap();
            }
        }
    }
}
