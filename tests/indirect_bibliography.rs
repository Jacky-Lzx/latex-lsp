use latex_lsp::{
    features, file_operations, navigation,
    workspace::{Document, DocumentKind, Workspace},
};
use std::path::Path;
use tower_lsp::lsp_types::*;

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
        versioned_edits: true,
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
fn indirect_bibliography_dependencies_keep_data_roles_and_link_each_real_name() {
    for command in ["bibliography", "nobibliography", "bibliographystyle"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let style = command == "bibliographystyle";
            let (argument, files) = if style {
                (
                    "{% head 😀\r\nfirst,part.bst% tail\r\n}",
                    vec![(
                        "first,part.bst",
                        "FUNCTION {fixture} { \"\\newcommand{\\leaked}{Text}\" }",
                    )],
                )
            } else {
                (
                    "{fi rst,% keep 😀\r\nsecond.bib}",
                    vec![
                        (
                            "first.bib",
                            "@book{fixture,title={\\newcommand{\\leaked}{Text}}}",
                        ),
                        ("second.bib", "@book{second,title={Second}}"),
                    ],
                )
            };
            // Bound the undefined probe: its unknown arity must not consume
            // the later citation used to check database completion.
            let source = format!(
                "\\documentclass{{article}}\\def\\loadpkg{{\\{command}}}\n{}{argument}\n{{\\leaked}}",
                call(form, command)
            );
            let mut all = vec![("main.tex", source.as_str())];
            all.extend(files.iter().copied());
            let mut workspace = project(root, &all);
            let main = uri(root, "main.tex");
            let links = navigation::document_links(&workspace, &main);
            assert_eq!(links.len(), files.len(), "{form}/{command}: {links:?}");
            for (name, _) in &files {
                let target = uri(root, name);
                assert!(
                    workspace.dependencies(&main).contains(&target),
                    "{form}/{command}: {target}"
                );
                assert_eq!(
                    workspace.documents[&target].kind,
                    if style {
                        DocumentKind::BibtexStyle
                    } else {
                        DocumentKind::Bibtex
                    }
                );
                let link = links
                    .iter()
                    .find(|link| link.target.as_ref() == Some(&target))
                    .unwrap();
                assert_eq!(
                    features::definitions(&workspace, &main, link.range.start)[0].uri,
                    target
                );
            }
            let at = source.rfind(r"\leaked").unwrap() + 2;
            assert!(features::definitions(
                &workspace,
                &main,
                workspace.documents[&main].text.position(at)
            )
            .is_empty());
            if !style {
                let marked = format!("{source}\n\\cite{{fix|ture}}");
                assert!(
                    items(&mut workspace, &main, &marked)
                        .iter()
                        .any(|item| item.label == "fixture"),
                    "{marked}"
                );
            }
        }
    }
}

#[test]
fn bibliography_file_renames_preserve_indirect_calls_lists_comments_and_explicit_suffixes() {
    for (command, extension) in [
        ("bibliography", "bib"),
        ("nobibliography", "bib"),
        ("bibliographystyle", "bst"),
    ] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            for explicit in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let name = format!(
                    "fixture{}",
                    if explicit {
                        format!(".{extension}")
                    } else {
                        String::new()
                    }
                );
                let argument = if extension == "bib" {
                    format!("{{other,% keep 😀\r\n{name}% tail\r\n}}")
                } else {
                    format!("{{% head 😀\r\n{name}% tail\r\n}}")
                };
                let source = format!(
                    "\\documentclass{{article}}\\RequirePackage{{definitions}}\n{}{argument}",
                    call(form, command)
                );
                let declaration = format!("\\def\\loadpkg{{\\{command}}}");
                let target_name = format!("fixture.{extension}");
                let workspace = project(
                    root,
                    &[
                        ("main.tex", &source),
                        ("definitions.sty", &declaration),
                        (&target_name, ""),
                        ("other.bib", ""),
                    ],
                );
                let main = uri(root, "main.tex");
                let target = uri(root, &target_name);
                let renamed = uri(root, &format!("renamed.{extension}"));
                let edit = file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: target.to_string(),
                        new_uri: renamed.to_string(),
                    }],
                )
                .unwrap()
                .unwrap();
                assert_eq!(
                    apply(&workspace, &edit, &main),
                    source.replace(&name, &name.replace("fixture", "renamed")),
                    "{form}/{command}/{explicit}"
                );
                assert!(file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: target.to_string(),
                        new_uri: uri(root, "renamed.txt").to_string()
                    }]
                )
                .is_err());
                if extension == "bib" {
                    assert!(file_operations::will_rename(
                        &workspace,
                        &[FileRename {
                            old_uri: target.to_string(),
                            new_uri: uri(root, "renamed,part.bib").to_string()
                        }]
                    )
                    .is_err());
                }
            }
        }
    }
}

#[test]
fn bibliography_path_completion_preserves_indirect_forms_comments_and_style_commas() {
    for (command, extension) in [
        ("bibliography", "bib"),
        ("nobibliography", "bib"),
        ("bibliographystyle", "bst"),
    ] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let name = if extension == "bst" {
                "fixture,part"
            } else {
                "fixture"
            };
            let target = format!("{name}.{extension}");
            let mut workspace =
                project(root, &[("main.tex", ""), (&target, ""), ("other.bib", "")]);
            let main = uri(root, "main.tex");
            let header = format!("\\documentclass{{article}}\\def\\loadpkg{{\\{command}}}\n");
            for argument in [
                format!("{{fix|{}}}", &name[3..]),
                format!("{{% head 😀\r\nfix|{}.{extension}% tail\r\n}}", &name[3..]),
                "{fix|".into(),
            ] {
                let marked = format!("{header}{}{argument}", call(form, command));
                let expected = marked.replace("fix|", "fix");
                let expected = if argument == "{fix|" {
                    format!("{header}{}{{{name}", call(form, command))
                } else {
                    expected
                };
                assert_eq!(
                    complete(&mut workspace, &main, &marked, &target),
                    expected,
                    "{form}/{command}/{argument}"
                );
            }
        }
    }
}

#[test]
fn indirect_bibliography_paths_ignore_macro_operands_and_uncertain_names() {
    for command in ["bibliography", "nobibliography", "bibliographystyle"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let ctrl = call(form, command);
            for body in [
                format!("\\newcommand{{\\hold}}[1]{{}}\\hold{{{ctrl}{{fix|ture}}}}"),
                format!("\\renewcommand{{\\{command}}}[1]{{}}{ctrl}{{fix|ture}}"),
                format!("{ctrl}{{\\filename|}}"),
                format!("\\endinput{ctrl}{{fix|ture}}"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!("\\def\\loadpkg{{\\{command}}}\n{body}");
                let mut workspace = project(
                    root,
                    &[
                        ("main.tex", &source.replace('|', "")),
                        ("fixture.bib", ""),
                        ("fixture.bst", ""),
                    ],
                );
                let main = uri(root, "main.tex");
                assert!(
                    navigation::document_links(&workspace, &main).is_empty(),
                    "{command}/{form}/{body}"
                );
                assert!(
                    workspace.dependencies(&main).is_empty(),
                    "{command}/{form}/{body}"
                );
                assert!(
                    file_items(&items(&mut workspace, &main, &source)).is_empty(),
                    "{command}/{form}/{body}"
                );
            }
        }
    }
}

#[test]
fn deferred_indirect_bibliography_paths_are_editable_without_loading_their_resources() {
    for command in ["bibliography", "nobibliography", "bibliographystyle"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let ctrl = call(form, command);
            let source = format!(
                "\\def\\loadpkg{{\\{command}}}\\newcommand{{\\later}}{{{ctrl}{{fix|ture}}}}"
            );
            let mut ws = project(
                root,
                &[
                    ("main.tex", &source.replace('|', "")),
                    ("fixture.bib", ""),
                    ("fixture.bst", ""),
                ],
            );
            let main = uri(root, "main.tex");
            let target = uri(
                root,
                if command == "bibliographystyle" {
                    "fixture.bst"
                } else {
                    "fixture.bib"
                },
            );
            let links = navigation::document_links(&ws, &main);
            assert_eq!(links.len(), 1, "{source}: {links:?}");
            assert_eq!(links[0].target.as_ref(), Some(&target));
            assert!(ws.dependencies(&main).is_empty(), "{source}");
            assert!(
                !file_items(&items(&mut ws, &main, &source)).is_empty(),
                "{source}"
            );
            let new = uri(
                root,
                if command == "bibliographystyle" {
                    "renamed.bst"
                } else {
                    "renamed.bib"
                },
            );
            let edit = file_operations::will_rename(
                &ws,
                &[FileRename {
                    old_uri: target.into(),
                    new_uri: new.into(),
                }],
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                apply(&ws, &edit, &main),
                source.replace('|', "").replace("{fixture}", "{renamed}")
            );
        }
    }
}

#[test]
fn shared_bibliography_calls_require_consistent_aliases_and_allow_explicit_root_selection() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = r"\documentclass{article}\def\loadpkg{\bibliography}\input{shared}";
    let mut workspace = project(
        root,
        &[
            ("one.tex", first),
            ("two.tex", first),
            ("shared.tex", r"\loadpkg{fixture}"),
            ("fixture.bib", "@book{fixture,title={Fixture}}"),
            ("fixture.bst", ""),
        ],
    );
    let shared = uri(root, "shared.tex");
    assert_eq!(
        complete(
            &mut workspace,
            &shared,
            r"\loadpkg{fix|ture}",
            "fixture.bib"
        ),
        r"\loadpkg{fixture}"
    );
    let two = uri(root, "two.tex");
    for declaration in [
        r"\def\loadpkg{\bibliographystyle}",
        r"\newcommand{\loadpkg}[1]{Text}",
    ] {
        workspace.documents.insert(
            two.clone(),
            Document::new(
                &two,
                format!("\\documentclass{{article}}{declaration}\\input{{shared}}"),
                Some(8),
            ),
        );
        assert!(file_items(&items(&mut workspace, &shared, r"\loadpkg{fix|ture}")).is_empty());
        assert!(navigation::document_links(&workspace, &shared).is_empty());
        assert!(file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(root, "fixture.bib").to_string(),
                new_uri: uri(root, "renamed.bib").to_string()
            }]
        )
        .is_err());
        assert_eq!(
            complete(
                &mut workspace,
                &shared,
                "% !TeX root = one.tex\r\n\\loadpkg{fix|ture}",
                "fixture.bib"
            ),
            "% !TeX root = one.tex\r\n\\loadpkg{fixture}"
        );
    }
}

#[test]
fn bibliography_features_refresh_closed_alias_definitions_without_parent_version_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let definitions = directory.path().join("defs.sty");
    std::fs::write(&definitions, r"\def\loadpkg{\bibliography}").unwrap();
    let marked = r"\documentclass{article}\RequirePackage{../defs}\loadpkg{fix|ture}";
    let mut workspace = project(
        &root,
        &[
            ("main.tex", &marked.replace('|', "")),
            ("fixture.bib", "@book{fixture,title={Fixture}}"),
        ],
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
        complete(&mut workspace, &main, marked, "fixture.bib"),
        marked.replace('|', "")
    );
    let fixture = uri(&root, "fixture.bib");
    assert!(workspace.dependencies(&main).contains(&fixture));
    let version = workspace.documents[&main].version;
    std::fs::write(&definitions, r"\newcommand{\loadpkg}[1]{Text}").unwrap();
    workspace.refresh_project(&main);
    assert_eq!(workspace.documents[&main].version, version);
    assert!(!workspace.dependencies(&main).contains(&fixture));
    assert!(navigation::document_links(&workspace, &main)
        .iter()
        .all(|link| link.target.as_ref() != Some(&fixture)));
    let position = workspace.documents[&main]
        .text
        .position(marked.find('|').unwrap());
    assert!(file_items(&features::completion(&workspace, &main, position)).is_empty());
    std::fs::write(&definitions, r"\def\loadpkg{\more}\def\more{\bibliography}").unwrap();
    workspace.refresh_project(&main);
    assert_eq!(
        complete(&mut workspace, &main, marked, "fixture.bib"),
        marked.replace('|', "")
    );
}

#[test]
fn directory_moves_update_indirect_database_and_style_paths_together() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\documentclass{article}\def\loaddata{\bibliography}\def\loadstyle{\bibliographystyle}\csuse{loaddata}{old/refs.bib}\csname loadstyle\endcsname{old/local,part.bst}";
    let workspace = project(
        root,
        &[
            ("main.tex", source),
            ("old/refs.bib", "@book{fixture,title={Fixture}}"),
            ("old/local,part.bst", ""),
        ],
    );
    let main = uri(root, "main.tex");
    let edit = file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: uri(root, "old").to_string(),
            new_uri: uri(root, "new").to_string(),
        }],
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        apply(&workspace, &edit, &main),
        source.replace("{old/", "{new/")
    );
}
