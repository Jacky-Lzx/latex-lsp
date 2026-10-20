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
fn indirect_package_lists_link_and_jump_to_each_literal_file_before_compiling() {
    for loader in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            for (argument, first) in [
                ("{first,second}", "first"),
                ("{% head 😀\r\nfirst% keep\r\n,second% tail\r\n}", "first"),
                ("{f i r s t,second}", "f i r s t"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = uri(root, "main.tex");
                let options = if loader == "RequirePackageWithOptions" {
                    ""
                } else {
                    "[draft]"
                };
                let source=format!("\\documentclass{{article}}\\def\\loadpkg{{\\{loader}}}\n{}{options}{argument}[2000/01/01]\\relax",call(form,loader));
                let workspace = project(
                    root,
                    &[
                        ("main.tex", &source),
                        ("first.sty", "First"),
                        ("second.sty", "Second"),
                    ],
                );
                let links = navigation::document_links(&workspace, &main);
                for (name, spelling) in [("first.sty", first), ("second.sty", "second")] {
                    let target = uri(root, name);
                    let found: Vec<_> = links
                        .iter()
                        .filter(|link| link.target.as_ref() == Some(&target))
                        .collect();
                    assert_eq!(found.len(), 1, "{loader}/{form}: {links:?}");
                    let text = &workspace.documents[&main].text;
                    let range = found[0].range;
                    assert_eq!(
                        &source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
                        spelling
                    );
                    assert_eq!(
                        features::definitions(&workspace, &main, range.start)[0].uri,
                        target
                    );
                }
            }
        }
    }
}

#[test]
fn package_file_rename_keeps_indirect_call_options_lists_versions_and_extension_rules() {
    for loader in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
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
            let main = uri(root, "main.tex");
            let options = if loader == "RequirePackageWithOptions" {
                ""
            } else {
                "[draft]"
            };
            let source=format!("\\documentclass{{article}}\\def\\loadpkg{{\\{loader}}}\n{}{options}{{% head 😀\r\nfirst% keep\r\n,second}}[2000/01/01]\\relax",call(form,loader));
            let workspace = project(
                root,
                &[
                    ("main.tex", &source),
                    ("first.sty", "First"),
                    ("second.sty", "Second"),
                ],
            );
            let rename = |new: &str| FileRename {
                old_uri: uri(root, "first.sty").into(),
                new_uri: uri(root, new).into(),
            };
            let edit = file_operations::will_rename(&workspace, &[rename("third.sty")])
                .unwrap()
                .unwrap();
            assert_eq!(
                apply(&workspace, &edit, &main),
                source.replace("first%", "third%")
            );
            for invalid in ["third.cls", "new file.sty", "new,file.sty"] {
                assert!(
                    file_operations::will_rename(&workspace, &[rename(invalid)]).is_err(),
                    "{loader}/{form}/{invalid}"
                );
            }
        }
    }
    // Classes take a single filename, so literal commas remain valid there.
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    let workspace = project(
        root,
        &[
            ("main.tex", r"\documentclass{first}"),
            ("first.cls", "Class"),
        ],
    );
    let rename = FileRename {
        old_uri: uri(root, "first.cls").into(),
        new_uri: uri(root, "new,file.cls").into(),
    };
    let edit = file_operations::will_rename(&workspace, &[rename])
        .unwrap()
        .unwrap();
    assert_eq!(apply(&workspace, &edit, &main), r"\documentclass{new,file}");
}

#[test]
fn indirect_package_completion_preserves_comments_lists_options_and_directory_suffixes() {
    for loader in ["usepackage", "RequirePackage", "RequirePackageWithOptions"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = uri(root, "main.tex");
            let mut workspace = project(
                root,
                &[
                    ("main.tex", ""),
                    ("fixture.sty", "Fixture"),
                    ("styles/local.sty", "Local"),
                ],
            );
            let options = if loader == "RequirePackageWithOptions" {
                ""
            } else {
                "[draft]"
            };
            let prefix = format!(
                "\\documentclass{{article}}\\def\\loadpkg{{\\{loader}}}\n{}{options}",
                call(form, loader)
            );
            for (tail, label, expected) in [
                ("{fix|ture}", "fixture.sty", "{fixture}"),
                ("{fix|", "fixture.sty", "{fixture"),
                (
                    "{other,fix|ture}[2000/01/01]",
                    "fixture.sty",
                    "{other,fixture}[2000/01/01]",
                ),
                (
                    "{% head 😀\r\nother,fix|ture% keep\r\n}",
                    "fixture.sty",
                    "{% head 😀\r\nother,fixture% keep\r\n}",
                ),
                ("{sty|/local}", "styles/", "{styles/local}"),
                ("{styles/loc|al}", "styles/local.sty", "{styles/local}"),
            ] {
                assert_eq!(
                    complete(&mut workspace, &main, &format!("{prefix}{tail}"), label),
                    format!("{prefix}{expected}"),
                    "{loader}/{form}/{tail}"
                );
            }
        }
    }
}

#[test]
fn shared_package_aliases_require_consistent_meaning_and_honor_root_selection_for_completion() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = r"\documentclass{article}\def\loadpkg{\usepackage}\input{shared}";
    let mut workspace = project(
        root,
        &[
            ("one.tex", first),
            ("two.tex", first),
            ("shared.tex", r"\loadpkg{fixture}"),
            ("fixture.sty", "Fixture"),
        ],
    );
    let shared = uri(root, "shared.tex");
    assert_eq!(
        complete(&mut workspace, &shared, r"\loadpkg{fix|}", "fixture.sty"),
        r"\loadpkg{fixture}"
    );
    let two = uri(root, "two.tex");
    for definition in [
        r"\def\loadpkg{\RequirePackage}",
        r"\newcommand{\loadpkg}[1]{Text}",
    ] {
        workspace.documents.insert(
            two.clone(),
            Document::new(
                &two,
                format!("\\documentclass{{article}}{definition}\\input{{shared}}"),
                Some(8),
            ),
        );
        assert!(file_items(&items(&mut workspace, &shared, r"\loadpkg{fix|}")).is_empty());
        assert_eq!(
            complete(
                &mut workspace,
                &shared,
                "% !TeX root = one.tex\r\n\\loadpkg{fix|}",
                "fixture.sty"
            ),
            "% !TeX root = one.tex\r\n\\loadpkg{fixture}"
        );
    }
}

#[test]
fn indirect_package_links_and_rename_reject_disagreement_between_projects() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = r"\documentclass{article}\def\loadpkg{\usepackage}\input{shared}";
    let second = r"\documentclass{article}\newcommand{\loadpkg}[1]{Text}\input{shared}";
    let workspace = project(
        root,
        &[
            ("one.tex", first),
            ("two.tex", second),
            ("shared.tex", r"\loadpkg{fixture}"),
            ("fixture.sty", "Fixture"),
        ],
    );
    let shared = uri(root, "shared.tex");
    assert!(navigation::document_links(&workspace, &shared).is_empty());
    let rename = FileRename {
        old_uri: uri(root, "fixture.sty").into(),
        new_uri: uri(root, "renamed.sty").into(),
    };
    assert!(file_operations::will_rename(&workspace, &[rename]).is_err());
}

#[test]
fn package_paths_in_macro_data_and_uncertain_graphs_do_not_offer_file_actions() {
    for source in [
        r"\newcommand{\later}{\csuse{usepackage}{fixture}}",
        r"\newcommand{\hold}[1]{}\hold\csuse{usepackage}{fixture}",
        r"\renewcommand{\usepackage}[1]{}\csuse{usepackage}{fixture}",
        r"\endinput\csuse{usepackage}{fixture}",
        r"\def\loadpkg{\usepackage}\loadpkg{fixture}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = uri(root, "main.tex");
        let mut workspace = project(
            root,
            &[("main.tex", source), ("fixture.sty", r"\def\loadpkg#1{}")],
        );
        assert!(
            navigation::document_links(&workspace, &main).is_empty(),
            "{source}"
        );
        let rename = FileRename {
            old_uri: uri(root, "fixture.sty").into(),
            new_uri: uri(root, "renamed.sty").into(),
        };
        assert!(
            file_operations::will_rename(&workspace, &[rename])
                .unwrap()
                .is_none(),
            "{source}"
        );
        let marked = source.replacen("fixture", "fix|ture", 1);
        assert!(
            file_items(&items(&mut workspace, &main, &marked)).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn package_path_features_refresh_closed_external_aliases_without_changing_the_parent_version() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let definitions = directory.path().join("defs.sty");
    std::fs::write(&definitions, r"\def\loadpkg{\usepackage}").unwrap();
    let marked =
        r"\documentclass{article}\csname RequirePackage\endcsname{../defs}\loadpkg{fix|ture}";
    let mut workspace = project(
        &root,
        &[
            ("main.tex", &marked.replace('|', "")),
            ("fixture.sty", "Fixture"),
        ],
    );
    let main = uri(&root, "main.tex");
    workspace.load_dependencies(&main, 0);
    let fixture = uri(&root, "fixture.sty");
    // The package remains closed, so a later refresh reads its saved source.
    let definitions_uri = Url::from_file_path(&definitions).unwrap();
    workspace
        .documents
        .get_mut(&definitions_uri)
        .unwrap()
        .version = None;
    assert_eq!(
        complete(&mut workspace, &main, marked, "fixture.sty"),
        marked.replace("fix|ture", "fixture")
    );
    assert!(navigation::document_links(&workspace, &main)
        .iter()
        .any(|link| link.target.as_ref() == Some(&fixture)));
    let version = workspace.documents[&main].version;
    std::fs::write(&definitions, r"\newcommand{\loadpkg}[1]{Text}").unwrap();
    workspace.refresh_project(&main);
    assert_eq!(workspace.documents[&main].version, version);
    assert!(navigation::document_links(&workspace, &main)
        .iter()
        .all(|link| link.target.as_ref() != Some(&fixture)));
    assert!(file_items(&features::completion(
        &workspace,
        &main,
        workspace.documents[&main]
            .text
            .position(marked.find('|').unwrap())
    ))
    .is_empty());
    std::fs::write(
        &definitions,
        r"\def\loadpkg{\nextpkg}\def\nextpkg{\usepackage}",
    )
    .unwrap();
    workspace.refresh_project(&main);
    assert_eq!(
        complete(&mut workspace, &main, marked, "fixture.sty"),
        marked.replace("fix|ture", "fixture")
    );
}
