use latex_lsp::{
    editing::{self, file_operations},
    language::{features, navigation},
    project::workspace::{Document, Workspace},
};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

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
        client: latex_lsp::project::state::ClientCapabilities {
            versioned_edits: true,
            code_action_literals: true,
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
fn apply(workspace: &Workspace, edit: WorkspaceEdit) -> BTreeMap<Url, String> {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edits")
    };
    documents
        .into_iter()
        .map(|document| {
            let uri = document.text_document.uri;
            let doc = &workspace.documents[&uri];
            assert_eq!(document.text_document.version, doc.version);
            let mut edits: Vec<_> = document
                .edits
                .into_iter()
                .map(|edit| match edit {
                    OneOf::Left(edit) => edit,
                    _ => panic!(),
                })
                .collect();
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let updated = doc
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
            (uri, updated.source)
        })
        .collect()
}
fn actions(
    workspace: &Workspace,
    target: &Url,
    diagnostics: Vec<Diagnostic>,
) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        target,
        Range::new(Position::default(), Position::new(u32::MAX, 0)),
        &CodeActionContext {
            diagnostics,
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
}
fn call(form: &str, loader: &str) -> String {
    match form {
        "direct" => format!("\\{loader}"),
        "named" => format!("\\csuse{{{loader}}}"),
        "literal" => format!("\\csname {loader}\\endcsname"),
        "alias" => r"\readpart".into(),
        "named-alias" => r"\csuse{readpart}".into(),
        "literal-alias" => r"\csname readpart\endcsname".into(),
        _ => panic!(),
    }
}

#[test]
fn static_named_and_alias_inputs_discover_sources_link_and_pair_before_compiling() {
    for loader in ["input", "include"] {
        for form in ["named", "literal", "alias", "named-alias", "literal-alias"] {
            for parent_begin in [false, true] {
                for commented in [false, true] {
                    let directory = tempfile::tempdir().unwrap();
                    let root = directory.path();
                    let name = if commented {
                        "{% filename\r\npart% keep\r\n}"
                    } else {
                        "{part}"
                    };
                    let prefix=format!("% 😀\r\n\\documentclass{{article}}\\def\\readpart{{\\{loader}}}\\begin{{document}}");
                    let input = format!("{}% before\r\n{name}", call(form, loader));
                    let main = if parent_begin {
                        format!("{prefix}\\begin{{x}}Main {input} Tail\\end{{document}}")
                    } else {
                        format!("{prefix}{input} Main\\end{{x}} Tail\\end{{document}}")
                    };
                    let part = if parent_begin {
                        r"Part\end{x}"
                    } else {
                        r"\begin{x}Part"
                    };
                    let workspace = project(root, &[("main.tex", &main), ("part.tex", part)]);
                    let parent = uri(root, "main.tex");
                    let child = uri(root, "part.tex");
                    assert!(workspace.project_documents(&parent).contains(&child));
                    assert_eq!(workspace.root_document(&child).unwrap(), parent);
                    let links = navigation::document_links(&workspace, &parent);
                    assert_eq!(links.len(), 1, "{form} {loader} {links:?}");
                    assert_eq!(links[0].target, Some(child.clone()));
                    let at = workspace.documents[&parent]
                        .text
                        .position(main.rfind("part").unwrap() + 1);
                    assert_eq!(features::definitions(&workspace, &parent, at)[0].uri, child);
                    for target in [&parent, &child] {
                        assert!(workspace.diagnostics(target).is_empty(), "{form} {loader}");
                        assert!(actions(&workspace, target, vec![]).is_empty());
                    }
                }
            }
        }
    }
}

#[test]
fn ungrouped_and_quoted_alias_input_filenames_keep_their_token_boundaries() {
    for form in ["named", "literal", "alias", "named-alias"] {
        for name in ["part.tex", "\"part file.tex\""] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main=format!("\\documentclass{{article}}\\def\\readpart{{\\input}}\\begin{{document}}{} {name} \nTail\\end{{document}}",call(form,"input"));
            let filename = name.trim_matches('"');
            let workspace = project(root, &[("main.tex", &main), (filename, "Part")]);
            let parent = uri(root, "main.tex");
            let child = uri(root, filename);
            let links = navigation::document_links(&workspace, &parent);
            assert_eq!(links.len(), 1, "{form} {name}: {links:?}");
            assert_eq!(links[0].target, Some(child.clone()));
            let text = &workspace.documents[&parent].text;
            let range = links[0].range;
            assert_eq!(
                &main[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()],
                filename
            );
            assert!(workspace.project_documents(&parent).contains(&child));
        }
    }
}

#[test]
fn local_declarations_and_nested_aliases_load_unindexed_external_sources() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        directory.path().join("defs.sty"),
        r"\ProvidesPackage{defs}\def\readpart{\input}",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("wrapper.inc"),
        r"\csname input\endcsname{../part.inc}",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("part.inc"),
        r"\newcommand{\externalword}{Part}\label{external}",
    )
    .unwrap();
    let main = r"\documentclass{article}\usepackage{../defs}\begin{document}\readpart{../wrapper.inc}\externalword\ref{external}\end{document}";
    let mut workspace = project(&root, &[("main.tex", main)]);
    let parent = uri(&root, "main.tex");
    let part = uri(directory.path(), "part.inc");
    assert!(!workspace.documents.contains_key(&part));
    workspace.load_dependencies(&parent, 0);
    assert!(workspace.documents.contains_key(&part));
    assert!(workspace.project_documents(&parent).contains(&part));
    for needle in ["externalword", "external}"] {
        let at = workspace.documents[&parent]
            .text
            .position(main.rfind(needle).unwrap() + 1);
        assert_eq!(features::definitions(&workspace, &parent, at)[0].uri, part);
    }
    // Closed package changes must expose a new alias target in an unchanged parent.
    std::fs::write(
        directory.path().join("defs.sty"),
        r"\ProvidesPackage{defs}\def\readpart{\secondpart}\def\secondpart{\input}",
    )
    .unwrap();
    workspace.refresh_project(&parent);
    assert!(workspace.project_documents(&parent).contains(&part));
    assert_eq!(navigation::document_links(&workspace, &parent).len(), 2);
}

#[test]
fn symbols_from_indirect_sources_participate_in_rename_without_other_projects() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = r"\documentclass{article}\begin{document}\csuse{input}{part}\partword\ref{kept}\end{document}";
    let child = r"\newcommand{\partword}{Part}\label{kept}";
    let other = r"\documentclass{article}\newcommand{\partword}{Other}\begin{document}\partword\end{document}";
    let workspace = project(
        root,
        &[
            ("main.tex", main),
            ("part.tex", child),
            ("other.tex", other),
        ],
    );
    let parent = uri(root, "main.tex");
    let part = uri(root, "part.tex");
    let at = workspace.documents[&parent]
        .text
        .position(main.find("partword").unwrap() + 1);
    let edit = features::rename(&workspace, &parent, at, "renamedword")
        .unwrap()
        .unwrap();
    let edited = apply(&workspace, edit);
    assert_eq!(edited.len(), 2);
    assert_eq!(edited[&parent], main.replace("partword", "renamedword"));
    assert_eq!(edited[&part], child.replace("partword", "renamedword"));
    assert!(!edited.contains_key(&uri(root, "other.tex")));
}

#[test]
fn indirect_loader_data_and_deferred_bodies_do_not_add_source_edges_or_links() {
    for invocation in [
        r"\csuse{input}{part}",
        r"\csname input\endcsname{part}",
        r"\readpart{part}",
    ] {
        for body in [
            format!(r"\newcommand{{\unused}}{{{invocation}}}"),
            format!(r"\newcommand{{\take}}[1]{{\detokenize{{#1}}}}\take{{{invocation}}}"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = format!(
                r"\documentclass{{article}}\def\readpart{{\input}}\begin{{document}}{body}\end{{document}}"
            );
            let workspace = project(
                root,
                &[("main.tex", &main), ("part.tex", r"\label{hidden}")],
            );
            let parent = uri(root, "main.tex");
            assert!(
                !workspace
                    .project_documents(&parent)
                    .contains(&uri(root, "part.tex")),
                "{body}"
            );
            assert!(
                navigation::document_links(&workspace, &parent).is_empty(),
                "{body}"
            );
        }
    }
}

#[test]
fn shared_indirect_calls_require_agreement_on_meaning_and_path() {
    for disagree in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let a = r"\documentclass{article}\def\readpart{\input}\begin{document}\input{shared}\end{document}";
        let b = if disagree {
            r"\documentclass{article}\newcommand{\readpart}[1]{Text}\begin{document}\input{shared}\end{document}"
        } else {
            a
        };
        let workspace = project(
            root,
            &[
                ("a.tex", a),
                ("b.tex", b),
                ("shared.tex", r"\readpart{part}"),
                ("part.tex", "Part"),
            ],
        );
        let shared = uri(root, "shared.tex");
        let links = navigation::document_links(&workspace, &shared);
        assert_eq!(links.len(), usize::from(!disagree), "{links:?}");
        let rename = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(root, "part.tex").into(),
                new_uri: uri(root, "changed.tex").into(),
            }],
        );
        if disagree {
            assert!(rename.is_err());
        } else {
            let edited = apply(&workspace, rename.unwrap().unwrap());
            assert_eq!(edited[&shared], r"\readpart{changed}");
        }
    }
}

#[test]
fn indirect_file_renames_preserve_calls_and_wrap_unquoted_names_with_spaces() {
    for loader in ["input", "include"] {
        for form in ["named", "literal", "alias", "named-alias"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let ungrouped = loader == "input";
            let suffix = if ungrouped { " part.tex \n" } else { "{part}" };
            let main = format!(
                r"\documentclass{{article}}\def\readpart{{\{loader}}}\begin{{document}}{}{suffix}Tail\end{{document}}",
                call(form, loader)
            );
            let workspace = project(root, &[("main.tex", &main), ("part.tex", "Part")]);
            let parent = uri(root, "main.tex");
            let edit = file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: uri(root, "part.tex").into(),
                    new_uri: uri(root, "new part.tex").into(),
                }],
            )
            .unwrap()
            .unwrap();
            let edited = apply(&workspace, edit);
            let expected = if ungrouped {
                main.replace("part.tex", "{new part.tex}%")
            } else {
                main.replace("{part}", "{new part}")
            };
            assert_eq!(edited[&parent], expected, "{form} {loader}");
            if loader == "include" {
                assert!(file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: uri(root, "part.tex").into(),
                        new_uri: uri(root, "part.txt").into()
                    }]
                )
                .is_err());
            }
        }
    }
}

#[test]
fn dependency_alias_changes_invalidate_parent_environment_fixes_and_file_links() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = r"\documentclass{article}\usepackage{defs}\begin{document}\readpart{part}Tail\end{document}";
    let mut workspace = project(
        root,
        &[
            ("main.tex", main),
            ("part.tex", r"\begin{x}Part"),
            ("defs.sty", r"\def\readpart{\input}"),
        ],
    );
    let parent = uri(root, "main.tex");
    let diagnostic = workspace.diagnostics(&parent).remove(0);
    let old = || editing::FixRequest {
        uri: parent.clone(),
        version: Some(7),
        diagnostic: diagnostic.clone(),
        replacement: Some("\\end{x}\n".into()),
    };
    assert!(editing::resolve_fix(&workspace, old()).is_some());
    let defs = uri(root, "defs.sty");
    workspace.documents.insert(
        defs.clone(),
        Document::new(&defs, r"\newcommand{\readpart}[1]{Text}".into(), Some(8)),
    );
    assert!(workspace.diagnostics(&parent).is_empty());
    assert!(editing::resolve_fix(&workspace, old()).is_none());
    assert!(actions(&workspace, &parent, vec![diagnostic]).is_empty());
    assert_eq!(navigation::document_links(&workspace, &parent).len(), 1);
    assert!(!workspace
        .project_documents(&parent)
        .contains(&uri(root, "part.tex")));
}

#[test]
fn cyclic_aliases_redefined_loaders_and_unstable_discovery_do_not_guess_edges() {
    for definition in [
        r"\def\readpart{\nextpart}\def\nextpart{\readpart}",
        r"\def\input#1{}\def\readpart{\input}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = format!(
            r"\documentclass{{article}}{definition}\begin{{document}}\readpart{{part}}\end{{document}}"
        );
        let workspace = project(root, &[("main.tex", &main), ("part.tex", "Part")]);
        let parent = uri(root, "main.tex");
        assert!(!workspace
            .project_documents(&parent)
            .contains(&uri(root, "part.tex")));
        assert!(navigation::document_links(&workspace, &parent).is_empty());
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = r"\documentclass{article}\def\readpart{\input}\begin{document}\begin{x}\readpart{part}\end{document}";
    let workspace = project(
        root,
        &[("main.tex", main), ("part.tex", r"\def\input#1{}\end{x}")],
    );
    let parent = uri(root, "main.tex");
    assert!(!workspace
        .project_documents(&parent)
        .contains(&uri(root, "part.tex")));
    assert!(navigation::document_links(&workspace, &parent).is_empty());
    assert!(actions(&workspace, &parent, vec![]).is_empty());
}

#[test]
fn unused_literal_sources_cannot_supply_aliases_for_indirect_calls() {
    for body in [
        r"\newcommand{\unused}{\def\readpart{\input}}",
        r"\newenvironment{unused}{\def\readpart{\input}}{}",
        r"\newcommand{\take}[1]{\detokenize{#1}}\take{\def\readpart{\input}}",
        r"\newcommand{\take}[1]{\detokenize{#1}}\take{\csdef{readpart}{\input}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = format!(
            r"\documentclass{{article}}{body}\begin{{document}}\readpart{{part}}\end{{document}}"
        );
        let workspace = project(root, &[("main.tex", &main), ("part.tex", "Part")]);
        let parent = uri(root, "main.tex");
        assert!(!workspace
            .project_documents(&parent)
            .contains(&uri(root, "part.tex")));
        assert!(navigation::document_links(&workspace, &parent).is_empty());
    }
    for inert in [
        r"\newcommand{\unused}{\input{defs}}",
        r"\newcommand{\take}[1]{\detokenize{#1}}\take{\input{defs}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = format!(
            r"\documentclass{{article}}{inert}\begin{{document}}\readpart{{part}}\end{{document}}"
        );
        let workspace = project(
            root,
            &[
                ("main.tex", &main),
                ("defs.tex", r"\def\readpart{\input}"),
                ("part.tex", "Part"),
            ],
        );
        let parent = uri(root, "main.tex");
        assert!(
            !workspace
                .project_documents(&parent)
                .contains(&uri(root, "part.tex")),
            "{inert}"
        );
        assert!(!navigation::document_links(&workspace, &parent)
            .iter()
            .any(|link| link.target == Some(uri(root, "part.tex"))));
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main="\\documentclass{article}\\def\\readpart{\\input}\\begin{document}\\input{wrapper}\\end{document}";
    let workspace = project(
        root,
        &[
            ("main.tex", main),
            ("wrapper.tex", "Part\\endinput\nIgnored\\readpart{part}"),
            ("part.tex", "Hidden"),
        ],
    );
    let parent = uri(root, "main.tex");
    assert!(!workspace
        .project_documents(&parent)
        .contains(&uri(root, "part.tex")));
    assert!(navigation::document_links(&workspace, &uri(root, "wrapper.tex")).is_empty());
}

#[test]
fn incoming_files_cannot_redirect_indirect_input_lookup() {
    for loader in ["input", "include"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = format!(
            r"\documentclass{{article}}\def\readpart{{\{loader}}}\begin{{document}}\input{{chapters/use}}\end{{document}}"
        );
        let workspace = project(
            root,
            &[
                ("main.tex", &main),
                ("chapters/use.tex", r"\readpart{item}"),
                ("chapters/item.tex", "Expected"),
                ("other.tex", "Incoming"),
            ],
        );
        let edit = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(root, "other.tex").into(),
                new_uri: uri(root, "item.tex").into(),
            }],
        )
        .unwrap()
        .unwrap();
        let edited = apply(&workspace, edit);
        assert_eq!(
            edited[&uri(root, "chapters/use.tex")],
            r"\readpart{chapters/item.tex}",
            "{loader}"
        );
    }
}

#[test]
fn grouping_renamed_input_filenames_preserves_the_consumed_space_token() {
    for form in ["direct", "named", "literal", "alias"] {
        for delimiter in [" ", "  ", "\t", "\n", "\r\n", " \n", "\n\n"] {
            for quoted in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let old = if quoted { "part file.tex" } else { "part.tex" };
                let spelling = if quoted {
                    format!("\"{old}\"")
                } else {
                    old.into()
                };
                let new = if quoted {
                    "新 part.tex"
                } else {
                    "new part.tex"
                };
                let start = format!(
                    "\\documentclass{{article}}\\def\\readpart{{\\input}}\\begin{{document}}Before {} ",
                    call(form, "input")
                );
                let tail = r"Tail\end{document}";
                let main = format!("{start}{spelling}{delimiter}{tail}");
                let workspace = project(root, &[("main.tex", &main), (old, "Part")]);
                let edit = file_operations::will_rename(
                    &workspace,
                    &[FileRename {
                        old_uri: uri(root, old).into(),
                        new_uri: uri(root, new).into(),
                    }],
                )
                .unwrap()
                .unwrap();
                let edited = apply(&workspace, edit);
                let expected = if delimiter.contains(['\r', '\n']) {
                    format!("{start}{{{new}}}%{delimiter}{tail}")
                } else {
                    format!("{start}{{{new}}}{tail}")
                };
                assert_eq!(
                    edited[&uri(root, "main.tex")],
                    expected,
                    "{form} {delimiter:?} {quoted}"
                );
            }
        }
    }
}
