use lsp_types::*;
use std::path::Path;
use texquill::{
    editing::file_operations,
    language::{features, navigation},
    project::workspace::{Document, Workspace},
};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, source) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    for doc in workspace.documents.values_mut() {
        doc.version = Some(7);
    }
    workspace
}
fn call(form: &str, loader: &str) -> String {
    match form {
        "direct" => format!("\\{loader}"),
        "named" => format!("\\csuse{{{loader}}}"),
        "literal" => format!("\\csname {loader}\\endcsname"),
        "alias" => r"\loadpkg".into(),
        "named-alias" => r"\csuse{loadpkg}".into(),
        "literal-alias" => r"\csname loadpkg\endcsname".into(),
        _ => panic!(),
    }
}
fn apply(workspace: &Workspace, edit: &WorkspaceEdit, target: &Url) -> String {
    let Some(DocumentChanges::Edits(documents)) = &edit.document_changes else {
        panic!("expected versioned edits")
    };
    let change = documents
        .iter()
        .find(|doc| &doc.text_document.uri == target)
        .unwrap();
    assert_eq!(
        change.text_document.version,
        workspace.documents[target].version
    );
    let mut edits: Vec<_> = change
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[target]
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
fn items(workspace: &mut Workspace, target: &Url, marked: &str) -> Vec<CompletionItem> {
    let at = marked.find('|').unwrap();
    workspace.documents.insert(
        target.clone(),
        Document::new(target, marked.replace('|', ""), Some(7)),
    );
    features::completion(
        workspace,
        target,
        workspace.documents[target].text.position(at),
    )
}
fn complete(workspace: &mut Workspace, target: &Url, marked: &str, label: &str) -> String {
    let candidates = items(workspace, target, marked);
    let item = candidates
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{marked}: missing {label}: {candidates:?}"));
    let Some(CompletionTextEdit::Edit(main)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(main.range.start.line, main.range.end.line);
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(main.clone());
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    for pair in edits.windows(2) {
        assert!(pair[1].range.end <= pair[0].range.start);
    }
    workspace.documents[target]
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
fn file_items(items: &[CompletionItem]) -> Vec<&CompletionItem> {
    items
        .iter()
        .filter(|item| {
            matches!(
                item.kind,
                Some(CompletionItemKind::FILE | CompletionItemKind::FOLDER)
            )
        })
        .collect()
}

#[test]
fn indirect_forwarding_links_the_second_argument_without_importing_the_target() {
    for (command, extension) in [
        ("PassOptionsToPackage", "sty"),
        ("PassOptionsToClass", "cls"),
    ] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\def\\loadpkg{{\\{command}}}\n{} % options 😀\r\n{{draft,key={{local}},code=\\custom}} % target\r\n{{settings/local}}\n", call(form, command));
            let target_name = format!("settings/local.{extension}");
            let workspace = project(
                root,
                &[
                    ("main.tex", &source),
                    (&target_name, r"\newcommand{\hidden}{Hidden}"),
                ],
            );
            let main = uri(root, "main.tex");
            let target = uri(root, &target_name);
            let links = navigation::document_links(&workspace, &main);
            assert_eq!(links.len(), 1, "{form}/{command}: {links:?}");
            assert_eq!(links[0].target.as_ref(), Some(&target));
            let start = source.find("settings/local").unwrap();
            assert_eq!(
                links[0].range,
                workspace.documents[&main].text.range(start, start + 14)
            );
            assert!(workspace.dependencies(&main).is_empty());
            assert!(!workspace.project_files(&main).contains(&target));
            let definitions = features::definitions(
                &workspace,
                &main,
                workspace.documents[&main].text.position(start + 2),
            );
            assert_eq!(definitions.len(), 1, "{form}/{command}: {definitions:?}");
            assert_eq!(definitions[0].uri, target);
            let rename = file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: target.to_string(),
                    new_uri: uri(root, &format!("settings/renamed.{extension}")).to_string(),
                }],
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                apply(&workspace, &rename, &main),
                source.replace("{settings/local}", "{settings/renamed}")
            );
        }
    }
}

#[test]
fn indirect_forwarding_and_actual_loads_rename_together_with_loaded_alias_definitions() {
    for (command, loader, extension) in [
        ("PassOptionsToPackage", "usepackage", "sty"),
        ("PassOptionsToClass", "documentclass", "cls"),
    ] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\RequirePackage{{definitions}}\n{}{{draft,key={{local}}}}{{local}}\n\\{loader}{{local}}\n", call(form, command));
            let declaration = format!("\\def\\loadpkg{{\\{command}}}");
            let target_name = format!("local.{extension}");
            let workspace = project(
                root,
                &[
                    ("main.tex", &source),
                    ("definitions.sty", &declaration),
                    (&target_name, ""),
                ],
            );
            let main = uri(root, "main.tex");
            let target = uri(root, &target_name);
            let rename = file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: target.to_string(),
                    new_uri: uri(root, &format!("renamed.{extension}")).to_string(),
                }],
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                apply(&workspace, &rename, &main),
                source.replace("}{local}", "}{renamed}").replace(
                    &format!("\\{loader}{{local}}"),
                    &format!("\\{loader}{{renamed}}")
                )
            );
            assert!(file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: target.to_string(),
                    new_uri: uri(root, "renamed.txt").to_string()
                }]
            )
            .is_err());
        }
    }
}

#[test]
fn indirect_forwarding_completes_only_the_filename_preserving_options_comments_and_suffixes() {
    for (command, extension) in [
        ("PassOptionsToPackage", "sty"),
        ("PassOptionsToClass", "cls"),
    ] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let header = format!("\\def\\loadpkg{{\\{command}}}\n");
            let name = format!("local.{extension}");
            let double = format!("local.{extension}.{extension}");
            let mut workspace = project(
                root,
                &[
                    ("main.tex", ""),
                    (&name, ""),
                    (&double, ""),
                    (&format!("unusable space.{extension}"), ""),
                ],
            );
            let main = uri(root, "main.tex");
            for (marked, expected, label) in [
                (
                    format!("{}{{draft,key={{local}}}}{{lo|cal}}", call(form, command)),
                    format!("{}{{draft,key={{local}}}}{{local}}", call(form, command)),
                    name.clone(),
                ),
                (
                    format!(
                        "{} % options 😀\r\n{{draft}} % target\r\n{{lo|cal.{extension}}}",
                        call(form, command)
                    ),
                    format!(
                        "{} % options 😀\r\n{{draft}} % target\r\n{{local.{extension}}}",
                        call(form, command)
                    ),
                    double.clone(),
                ),
                (
                    format!("{}{{draft}}{{lo|", call(form, command)),
                    format!("{}{{draft}}{{local", call(form, command)),
                    name.clone(),
                ),
            ] {
                assert_eq!(
                    complete(&mut workspace, &main, &format!("{header}{marked}"), &label),
                    format!("{header}{expected}")
                );
            }
            let options = format!("{header}{}{{lo|cal}}{{local}}", call(form, command));
            assert!(file_items(&items(&mut workspace, &main, &options)).is_empty());
        }
    }
}

#[test]
fn indirect_forwarding_rejects_macro_operands_deferred_calls_and_uncertain_keys() {
    for command in ["PassOptionsToPackage", "PassOptionsToClass"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            for case in [
                "operand",
                "deferred",
                "redefined",
                "dynamic",
                "leading-space",
                "trailing-space",
                "internal-space",
                "list",
                "after-endinput",
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let ctrl = call(form, command);
                let body = match case {
                    "operand" => {
                        format!("\\newcommand{{\\hold}}[1]{{}}\\hold{{{ctrl}{{draft}}{{lo|cal}}}}")
                    }
                    "deferred" => format!("\\newcommand{{\\later}}{{{ctrl}{{draft}}{{lo|cal}}}}"),
                    "redefined" => {
                        format!("\\renewcommand{{\\{command}}}[2]{{}}{ctrl}{{draft}}{{lo|cal}}")
                    }
                    "dynamic" => format!("{ctrl}{{draft}}{{\\name|}}"),
                    "leading-space" => format!("{ctrl}{{draft}}{{ lo|cal}}"),
                    "trailing-space" => format!("{ctrl}{{draft}}{{lo|cal }}"),
                    "internal-space" => format!("{ctrl}{{draft}}{{lo |cal}}"),
                    "list" => format!("{ctrl}{{draft}}{{local,lo|cal}}"),
                    "after-endinput" => format!("\\endinput{ctrl}{{draft}}{{lo|cal}}"),
                    _ => panic!(),
                };
                let source = format!("\\def\\loadpkg{{\\{command}}}\n{body}");
                let mut workspace = project(
                    root,
                    &[
                        ("main.tex", &source.replace('|', "")),
                        ("local.sty", ""),
                        ("local.cls", ""),
                    ],
                );
                let main = uri(root, "main.tex");
                assert!(
                    navigation::document_links(&workspace, &main).is_empty(),
                    "{form}/{command}/{case}"
                );
                assert!(
                    file_items(&items(&mut workspace, &main, &source)).is_empty(),
                    "{form}/{command}/{case}"
                );
                assert!(
                    file_operations::will_rename(
                        &workspace,
                        &[FileRename {
                            old_uri: uri(root, "local.sty").to_string(),
                            new_uri: uri(root, "new.sty").to_string()
                        }]
                    )
                    .unwrap()
                    .is_none(),
                    "{form}/{command}/{case}"
                );
            }
        }
    }
}

#[test]
fn shared_forwarding_aliases_require_consistent_meaning_and_respect_explicit_roots() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = r"\documentclass{article}\def\loadpkg{\PassOptionsToPackage}\input{shared}";
    let mut workspace = project(
        root,
        &[
            ("one.tex", first),
            ("two.tex", first),
            ("shared.tex", r"\loadpkg{draft}{local}"),
            ("local.sty", ""),
            ("local.cls", ""),
        ],
    );
    let shared = uri(root, "shared.tex");
    assert_eq!(
        complete(
            &mut workspace,
            &shared,
            r"\loadpkg{draft}{lo|cal}",
            "local.sty"
        ),
        r"\loadpkg{draft}{local}"
    );
    let two = uri(root, "two.tex");
    for declaration in [
        r"\def\loadpkg{\PassOptionsToClass}",
        r"\newcommand{\loadpkg}[2]{Text}",
    ] {
        workspace.documents.insert(
            two.clone(),
            Document::new(
                &two,
                format!("\\documentclass{{article}}{declaration}\\input{{shared}}"),
                Some(8),
            ),
        );
        assert!(file_items(&items(&mut workspace, &shared, r"\loadpkg{draft}{lo|cal}")).is_empty());
        assert!(navigation::document_links(&workspace, &shared).is_empty());
        assert!(file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(root, "local.sty").to_string(),
                new_uri: uri(root, "renamed.sty").to_string()
            }]
        )
        .is_err());
        assert_eq!(
            complete(
                &mut workspace,
                &shared,
                "% !TeX root = one.tex\r\n\\loadpkg{draft}{lo|cal}",
                "local.sty"
            ),
            "% !TeX root = one.tex\r\n\\loadpkg{draft}{local}"
        );
    }
}

#[test]
fn forwarding_path_actions_refresh_closed_alias_sources_without_parent_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let definitions = directory.path().join("defs.sty");
    std::fs::write(&definitions, r"\def\loadpkg{\PassOptionsToPackage}").unwrap();
    let marked = r"\documentclass{article}\RequirePackage{../defs}\loadpkg{draft}{lo|cal}";
    let mut workspace = project(
        &root,
        &[("main.tex", &marked.replace('|', "")), ("local.sty", "")],
    );
    let main = uri(&root, "main.tex");
    workspace.load_dependencies(&main, 0);
    let definitions_uri = Url::from_file_path(&definitions).unwrap();
    workspace
        .documents
        .get_mut(&definitions_uri)
        .unwrap()
        .version = None;
    assert_eq!(
        complete(&mut workspace, &main, marked, "local.sty"),
        marked.replace('|', "")
    );
    let target = uri(&root, "local.sty");
    assert!(navigation::document_links(&workspace, &main)
        .iter()
        .any(|link| link.target.as_ref() == Some(&target)));
    let version = workspace.documents[&main].version;
    std::fs::write(&definitions, r"\newcommand{\loadpkg}[2]{Text}").unwrap();
    workspace.refresh_project(&main);
    assert_eq!(workspace.documents[&main].version, version);
    assert!(navigation::document_links(&workspace, &main)
        .iter()
        .all(|link| link.target.as_ref() != Some(&target)));
    let position = workspace.documents[&main]
        .text
        .position(marked.find('|').unwrap());
    assert!(file_items(&features::completion(&workspace, &main, position)).is_empty());
    std::fs::write(
        &definitions,
        r"\def\loadpkg{\sendopts}\def\sendopts{\PassOptionsToPackage}",
    )
    .unwrap();
    workspace.refresh_project(&main);
    assert_eq!(
        complete(&mut workspace, &main, marked, "local.sty"),
        marked.replace('|', "")
    );
}
