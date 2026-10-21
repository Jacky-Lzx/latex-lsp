use latex_lsp::{
    features, file_operations, navigation,
    workspace::{Document, Workspace},
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
fn indirect_conditionals_link_files_but_only_conditional_inputs_import_declarations() {
    for loader in ["InputIfFileExists", "IfFileExists"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("\\documentclass{{article}}\\RequirePackage{{definitions}}{}{{% file 😀\r\nchapter.tex% tail\r\n}}{{Before}}{{Missing}}\\leaked", call(form, loader));
            let workspace = project(
                root,
                &[
                    ("main.tex", &source),
                    ("definitions.sty", &format!("\\def\\loadpkg{{\\{loader}}}")),
                    ("chapter.tex", r"\newcommand{\leaked}{Chapter}"),
                ],
            );
            let main = uri(root, "main.tex");
            let target = uri(root, "chapter.tex");
            assert_eq!(
                workspace.dependencies(&main).contains(&target),
                loader == "InputIfFileExists",
                "{loader}/{form}"
            );
            let links = navigation::document_links(&workspace, &main);
            let matching: Vec<_> = links
                .iter()
                .filter(|link| link.target.as_ref() == Some(&target))
                .collect();
            assert_eq!(matching.len(), 1, "{loader}/{form}: {links:?}");
            let link = matching[0];
            let text = &workspace.documents[&main].text;
            assert_eq!(
                &source
                    [text.offset(link.range.start).unwrap()..text.offset(link.range.end).unwrap()],
                "chapter.tex"
            );
            assert_eq!(
                features::definitions(&workspace, &main, link.range.start)[0].uri,
                target
            );
            let at = text.position(source.rfind(r"\leaked").unwrap() + 2);
            let definitions = features::definitions(&workspace, &main, at);
            assert_eq!(
                definitions.len(),
                usize::from(loader == "InputIfFileExists"),
                "{loader}/{form}: {definitions:?}"
            );
        }
    }
}

#[test]
fn conditional_completion_preserves_comments_suffixes_and_both_branch_bodies() {
    for loader in ["InputIfFileExists", "IfFileExists"] {
        for form in [
            "direct",
            "named",
            "literal",
            "alias",
            "named-alias",
            "literal-alias",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let mut workspace = project(root, &[("main.tex", ""), ("fixture.tex", "")]);
            let main = uri(root, "main.tex");
            let header = format!("\\documentclass{{article}}\\def\\loadpkg{{\\{loader}}}");
            for (argument, expected) in [
                ("{fix|ture}{Before}{Missing}", "{fixture}{Before}{Missing}"),
                (
                    "{% file 😀\r\nfix|ture.tex% tail\r\n}{Before}{Missing}",
                    "{% file 😀\r\nfixture.tex% tail\r\n}{Before}{Missing}",
                ),
                ("{fix|", "{fixture"),
            ] {
                let marked = format!("{header}{}{argument}", call(form, loader));
                assert_eq!(
                    complete(&mut workspace, &main, &marked, "fixture.tex"),
                    format!("{header}{}{expected}", call(form, loader)),
                    "{loader}/{form}/{argument}"
                );
            }
            for branch in ["{fixture}{Be|fore}{Missing}", "{fixture}{Before}{Mis|sing}"] {
                let marked = format!("{header}{}{branch}", call(form, loader));
                assert!(
                    file_items(&items(&mut workspace, &main, &marked)).is_empty(),
                    "{loader}/{form}/{branch}"
                );
            }
        }
    }
}

#[test]
fn indirect_conditional_file_and_directory_renames_preserve_branches_and_literal_names() {
    for loader in ["InputIfFileExists", "IfFileExists"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            for filename in [
                "fixture",
                "fixture.tex",
                "my chapter.tex",
                "章节.tex",
                "first,second.tex",
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!("\\documentclass{{article}}\\def\\loadpkg{{\\{loader}}}{}{{% file 😀\r\nold/{filename}% tail\r\n}}{{Before}}{{Missing}}", call(form, loader));
                let physical = if filename.ends_with(".tex") {
                    filename.into()
                } else {
                    format!("{filename}.tex")
                };
                let workspace = project(
                    root,
                    &[
                        ("main.tex", &source),
                        (&format!("old/{physical}"), "Chapter"),
                    ],
                );
                let main = uri(root, "main.tex");
                for (old, new, expected) in [
                    (
                        format!("old/{physical}"),
                        "old/renamed.tex".to_owned(),
                        source.replace(
                            &format!("old/{filename}"),
                            if filename.ends_with(".tex") {
                                "old/renamed.tex"
                            } else {
                                "old/renamed"
                            },
                        ),
                    ),
                    (
                        "old".into(),
                        "new".to_owned(),
                        source.replace("old/", "new/"),
                    ),
                ] {
                    let edit = file_operations::will_rename(
                        &workspace,
                        &[FileRename {
                            old_uri: uri(root, &old).to_string(),
                            new_uri: uri(root, &new).to_string(),
                        }],
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!(
                        apply(&workspace, &edit, &main),
                        expected,
                        "{loader}/{form}/{filename}"
                    );
                }
            }
        }
    }
}

#[test]
fn uncertain_or_deferred_conditionals_do_not_offer_static_path_edits() {
    for loader in ["InputIfFileExists", "IfFileExists"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let ctrl = call(form, loader);
            for body in [
                format!("\\newcommand{{\\hold}}[1]{{}}\\hold{{{ctrl}{{fix|ture}}{{}}{{}}}}"),
                format!("\\newcommand{{\\later}}{{{ctrl}{{fix|ture}}{{}}{{}}}}"),
                format!("\\renewcommand{{\\{loader}}}[3]{{}}{ctrl}{{fix|ture}}{{}}{{}}"),
                format!("{ctrl}{{\\dynamic|}}{{}}{{}}"),
                format!("{ctrl}*{{fix|ture}}{{}}{{}}"),
                format!("{ctrl}[note]{{fix|ture}}{{}}{{}}"),
                format!("\\endinput\n{ctrl}{{fix|ture}}{{}}{{}}"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let marked = format!("\\def\\loadpkg{{\\{loader}}}\n{body}");
                let mut workspace = project(
                    root,
                    &[("main.tex", &marked.replace('|', "")), ("fixture.tex", "")],
                );
                let main = uri(root, "main.tex");
                assert!(
                    navigation::document_links(&workspace, &main).is_empty(),
                    "{loader}/{form}/{body}"
                );
                assert!(
                    file_items(&items(&mut workspace, &main, &marked)).is_empty(),
                    "{loader}/{form}/{body}"
                );
            }
        }
    }
}

#[test]
fn shared_conditional_paths_require_consistent_roots_and_honor_an_explicit_root_directive() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\documentclass{article}\def\loadpkg{\InputIfFileExists}\input{shared}";
    let mut workspace = project(
        root,
        &[
            ("one.tex", source),
            ("two.tex", source),
            ("shared.tex", r"\loadpkg{fixture}{}{}"),
            ("fixture.tex", ""),
        ],
    );
    let shared = uri(root, "shared.tex");
    assert_eq!(
        complete(
            &mut workspace,
            &shared,
            r"\loadpkg{fix|ture}{}{}",
            "fixture.tex"
        ),
        r"\loadpkg{fixture}{}{}"
    );
    let two = uri(root, "two.tex");
    workspace.documents.insert(
        two.clone(),
        Document::new(
            &two,
            r"\documentclass{article}\newcommand{\loadpkg}[1]{Text}\input{shared}".into(),
            Some(8),
        ),
    );
    assert!(file_items(&items(&mut workspace, &shared, r"\loadpkg{fix|ture}{}{}")).is_empty());
    assert!(navigation::document_links(&workspace, &shared).is_empty());
    assert!(file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: uri(root, "fixture.tex").to_string(),
            new_uri: uri(root, "renamed.tex").to_string()
        }]
    )
    .is_err());
    let marked = "% !TeX root = one.tex\r\n\\loadpkg{fix|ture}{}{}";
    assert_eq!(
        complete(&mut workspace, &shared, marked, "fixture.tex"),
        marked.replace('|', "")
    );
}

#[test]
fn closed_external_conditional_aliases_refresh_without_changing_the_parent_version() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let provider = directory.path().join("defs.sty");
    std::fs::write(&provider, r"\def\loadpkg{\InputIfFileExists}").unwrap();
    let marked = r"\documentclass{article}\RequirePackage{../defs}\loadpkg{fix|ture}{}{}";
    let mut workspace = project(
        &root,
        &[("main.tex", &marked.replace('|', "")), ("fixture.tex", "")],
    );
    let main = uri(&root, "main.tex");
    workspace.load_dependencies(&main, 0);
    let provider_uri = Url::from_file_path(&provider).unwrap();
    workspace.documents.get_mut(&provider_uri).unwrap().version = None;
    assert_eq!(
        complete(&mut workspace, &main, marked, "fixture.tex"),
        marked.replace('|', "")
    );
    let version = workspace.documents[&main].version;
    std::fs::write(&provider, r"\newcommand{\loadpkg}[1]{Text}").unwrap();
    workspace.refresh_project(&main);
    assert_eq!(workspace.documents[&main].version, version);
    assert!(navigation::document_links(&workspace, &main)
        .iter()
        .all(|link| link.target.as_ref() != Some(&uri(&root, "fixture.tex"))));
    let position = workspace.documents[&main]
        .text
        .position(marked.find('|').unwrap());
    assert!(file_items(&features::completion(&workspace, &main, position)).is_empty());
    std::fs::write(
        &provider,
        r"\def\loadpkg{\next}\def\next{\InputIfFileExists}",
    )
    .unwrap();
    workspace.refresh_project(&main);
    assert_eq!(
        complete(&mut workspace, &main, marked, "fixture.tex"),
        marked.replace('|', "")
    );
}
