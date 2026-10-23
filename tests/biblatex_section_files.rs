use latex_lsp::{
    features, file_operations, navigation,
    text::Text,
    workspace::{DocumentKind, Workspace},
};
use tower_lsp::lsp_types::*;
fn setup(source: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace, Url) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.tex"), source).unwrap();
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let mut ws = Workspace::default();
    ws.config.distribution.enabled = false;
    ws.add_root(dir.path().into());
    let main = Url::from_file_path(dir.path().join("main.tex")).unwrap();
    ws.open(main.clone(), source.into(), 6);
    (dir, ws, main)
}
fn apply(source: &str, mut edits: Vec<TextEdit>) -> String {
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
const DATA: &str = "@book{selected,title={Selected}}\n\\newcommand\\leaked{}";
#[test]
fn explicit_section_files_join_dependencies_and_move_as_native_list_items() {
    for call in [
        r"\newrefsection",
        r"\refsection",
        r"\begin{refsection}",
        r"\csuse{newrefsection}",
        r"\csname newrefsection\endcsname",
        r"\choose",
    ] {
        let source = format!(
            r"\documentclass{{article}}\def\choose{{\newrefsection}}\addbibresource{{default.bib}}\addglobalbib{{global.bib}}\addsectionbib[label=unused]{{unused.bib}}{call}[selected,other.bib]\leaked"
        );
        let (dir, ws, main) = setup(
            &source,
            &[
                ("default.bib", DATA),
                ("global.bib", DATA),
                ("unused.bib", DATA),
                ("selected.bib", DATA),
                ("other.bib", DATA),
            ],
        );
        let selected = Url::from_file_path(dir.path().join("selected.bib")).unwrap();
        let unused = Url::from_file_path(dir.path().join("unused.bib")).unwrap();
        assert!(ws.dependencies(&main).contains(&selected), "{call}");
        assert!(!ws.dependencies(&main).contains(&unused), "{call}");
        assert_eq!(ws.documents[&selected].kind, DocumentKind::Bibtex);
        assert_eq!(
            navigation::document_links(&ws, &main)
                .iter()
                .filter(|link| link.target.as_ref() == Some(&selected))
                .count(),
            1,
            "{call}"
        );
        let at = Text::new(source.clone()).position(source.rfind("leaked").unwrap() + 2);
        assert!(features::definitions(&ws, &main, at).is_empty());
        let edit = file_operations::will_rename(
            &ws,
            &[FileRename {
                old_uri: selected.to_string(),
                new_uri: Url::from_file_path(dir.path().join("renamed%file.bib"))
                    .unwrap()
                    .to_string(),
            }],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            apply(&source, edit.changes.unwrap().remove(&main).unwrap()),
            source.replace("[selected,", "[renamed%file.bib,"),
            "{call}"
        );
        assert!(file_operations::will_rename(
            &ws,
            &[FileRename {
                old_uri: selected.to_string(),
                new_uri: Url::from_file_path(dir.path().join("bad,name.bib"))
                    .unwrap()
                    .to_string()
            }]
        )
        .is_err());
    }
}
#[test]
fn selected_labels_load_the_registered_file_without_becoming_editable_filename_text() {
    let source =
        r"\documentclass{article}\addsectionbib[label=chosen]{selected.data}\newrefsection[chosen]";
    let (dir, ws, main) = setup(source, &[("selected.data", DATA), ("chosen.bib", DATA)]);
    let selected = Url::from_file_path(dir.path().join("selected.data")).unwrap();
    let false_file = Url::from_file_path(dir.path().join("chosen.bib")).unwrap();
    assert!(ws.dependencies(&main).contains(&selected));
    assert!(!ws.dependencies(&main).contains(&false_file));
    assert_eq!(ws.documents[&selected].kind, DocumentKind::Bibtex);
    assert_eq!(
        navigation::document_links(&ws, &main)
            .iter()
            .filter(|link| link.target.as_ref() == Some(&selected))
            .count(),
        1
    );
    let edit = file_operations::will_rename(
        &ws,
        &[FileRename {
            old_uri: selected.to_string(),
            new_uri: Url::from_file_path(dir.path().join("renamed.data"))
                .unwrap()
                .to_string(),
        }],
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        apply(source, edit.changes.unwrap().remove(&main).unwrap()),
        source.replace("selected.data", "renamed.data")
    );
}
#[test]
fn resource_labels_shadow_paths_and_ambiguous_or_nonlocal_labels_do_not_fall_back_to_files() {
    for registrations in [
        r"\addsectionbib[label=chosen]{a.bib}\addsectionbib[label=chosen]{b.bib}",
        r"\addsectionbib[label=chosen,location=remote]{a.bib}",
    ] {
        let source = format!(r"\documentclass{{article}}{registrations}\newrefsection[chosen]");
        let (dir, ws, main) = setup(
            &source,
            &[("a.bib", DATA), ("b.bib", DATA), ("chosen.bib", DATA)],
        );
        assert!(!ws
            .dependencies(&main)
            .contains(&Url::from_file_path(dir.path().join("chosen.bib")).unwrap()));
        assert_eq!(ws.dependencies(&main).len(), 0, "{source}");
    }
    let source = r"\documentclass{article}\addsectionbib[label=selected.bib]{a.bib}\newrefsection[selected.bib]";
    let (dir, ws, main) = setup(source, &[("a.bib", DATA), ("selected.bib", DATA)]);
    let a = Url::from_file_path(dir.path().join("a.bib")).unwrap();
    let false_file = Url::from_file_path(dir.path().join("selected.bib")).unwrap();
    assert!(
        ws.dependencies(&main).contains(&a),
        "deps={:?}, docs={:?}",
        ws.dependencies(&main),
        ws.documents.keys().collect::<Vec<_>>()
    );
    assert!(!ws.dependencies(&main).contains(&false_file));
    let edit = file_operations::will_rename(
        &ws,
        &[FileRename {
            old_uri: a.to_string(),
            new_uri: Url::from_file_path(dir.path().join("b.bib"))
                .unwrap()
                .to_string(),
        }],
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        apply(source, edit.changes.unwrap().remove(&main).unwrap()),
        source.replace("{a.bib}", "{b.bib}")
    );
}
#[test]
fn raw_percent_lists_and_registered_exact_names_keep_distinct_file_lookup_rules() {
    let source =
        r"\documentclass{article}\addsectionbib{exact}\newrefsection[refs%data.bib,second,exact]";
    let (dir, ws, main) = setup(
        source,
        &[
            ("refs%data.bib", DATA),
            ("second.bib", DATA),
            ("exact", DATA),
            ("exact.bib", DATA),
        ],
    );
    for filename in ["refs%data.bib", "second.bib", "exact"] {
        assert!(
            ws.dependencies(&main)
                .contains(&Url::from_file_path(dir.path().join(filename)).unwrap()),
            "{filename}"
        );
    }
    assert!(!ws
        .dependencies(&main)
        .contains(&Url::from_file_path(dir.path().join("exact.bib")).unwrap()));
}
#[test]
fn section_filename_completion_replaces_one_item_and_preserves_following_resources() {
    for marked in [
        r"\newrefsection[sel¦BAD,other.bib]",
        r"\newrefsection[¦]",
        r"\begin{refsection}[other.bib,sel¦BAD]",
        r"\csuse{newrefsection}[sel¦",
        r"\choose[sel¦BAD]",
    ] {
        let marked =
            format!("% 😀\r\n\\documentclass{{article}}\\def\\choose{{\\newrefsection}}{marked}");
        let source = marked.replace('¦', "");
        let (_dir, ws, main) = setup(
            &source,
            &[
                ("selected.bib", DATA),
                ("other.bib", DATA),
                ("selected,wrong.bib", DATA),
            ],
        );
        let at = Text::new(source.clone()).position(marked.find('¦').unwrap());
        let items = features::completion(&ws, &main, at);
        assert!(
            !items.iter().any(|item| item.label == "selected,wrong.bib"),
            "{marked}"
        );
        let item = items
            .into_iter()
            .find(|item| item.label == "selected.bib")
            .unwrap_or_else(|| panic!("{marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        let expected = marked
            .replace("sel¦BAD", "selected.bib")
            .replace("sel¦", "selected.bib")
            .replace('¦', "selected.bib");
        assert_eq!(apply(&source, vec![edit]), expected, "{marked}");
    }
}

#[test]
fn section_data_overrides_and_uncalled_declarations_do_not_load_files() {
    for call in [
        r"\newcommand\discard[1]{}\discard{\newrefsection[selected]}",
        r"\newcommand\newrefsection[1][]{}\newrefsection[selected]",
        r"\newcommand\unused{\newrefsection[selected]}",
        r"\verb|\newrefsection[selected]|",
    ] {
        let source = format!(r"\documentclass{{article}}{call}");
        let (dir, ws, main) = setup(&source, &[("selected.bib", DATA)]);
        let selected = Url::from_file_path(dir.path().join("selected.bib")).unwrap();
        assert!(!ws.dependencies(&main).contains(&selected), "{call}");
        assert!(navigation::document_links(&ws, &main).is_empty(), "{call}");
    }
}
#[test]
fn cross_file_aliases_and_unsaved_registrations_update_selected_dependencies() {
    let source = r"\documentclass{article}\usepackage{provider}\choose[chosen]";
    let (dir, mut ws, main) = setup(
        source,
        &[
            (
                "provider.sty",
                r"\def\choose{\newrefsection}\addsectionbib[label=chosen]{a.bib}",
            ),
            ("a.bib", DATA),
            ("b.bib", DATA),
        ],
    );
    let provider = Url::from_file_path(dir.path().join("provider.sty")).unwrap();
    let a = Url::from_file_path(dir.path().join("a.bib")).unwrap();
    let b = Url::from_file_path(dir.path().join("b.bib")).unwrap();
    assert!(
        ws.project_files(&main).contains(&a),
        "deps={:?}, docs={:?}",
        ws.project_files(&main),
        ws.documents.keys().collect::<Vec<_>>()
    );
    assert!(!ws.project_files(&main).contains(&b));
    ws.open(
        provider.clone(),
        r"\def\choose{\newrefsection}\addsectionbib[label=chosen]{b.bib}".into(),
        9,
    );
    assert!(!ws.project_files(&main).contains(&a));
    assert!(ws.project_files(&main).contains(&b));
    let edit = file_operations::will_rename(
        &ws,
        &[FileRename {
            old_uri: b.to_string(),
            new_uri: Url::from_file_path(dir.path().join("renamed.bib"))
                .unwrap()
                .to_string(),
        }],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap();
    assert_eq!(edit.len(), 1);
    assert_eq!(
        apply(
            &ws.documents[&provider].text.source,
            edit[&provider].clone()
        ),
        r"\def\choose{\newrefsection}\addsectionbib[label=chosen]{renamed.bib}"
    );
}

#[test]
#[ignore = "requires pdflatex, biblatex and Biber"]
fn real_explicit_section_file_moves_preserve_biber_resolution() {
    let source = r"\documentclass{article}\usepackage{biblatex}\begin{document}\newrefsection[section]\nocite{selected}\printbibliography\end{document}";
    let (dir, ws, main) = setup(source, &[("section.bib", DATA)]);
    let bib = Url::from_file_path(dir.path().join("section.bib")).unwrap();
    std::fs::write(
        bib.to_file_path().unwrap(),
        "@book{selected,author={Doe, Jane},title={Selected},year={2026}}",
    )
    .unwrap();
    let renamed = Url::from_file_path(dir.path().join("selected%renamed.bib")).unwrap();
    let edits = file_operations::will_rename(
        &ws,
        &[FileRename {
            old_uri: bib.to_string(),
            new_uri: renamed.to_string(),
        }],
    )
    .unwrap()
    .unwrap()
    .changes
    .unwrap()
    .remove(&main)
    .unwrap();
    let updated = apply(source, edits);
    assert_eq!(
        updated,
        source.replace("[section]", "[selected%renamed.bib]")
    );
    for (text, moved) in [(source, false), (updated.as_str(), true)] {
        if moved {
            std::fs::rename(bib.to_file_path().unwrap(), renamed.to_file_path().unwrap()).unwrap();
        }
        std::fs::write(main.to_file_path().unwrap(), text).unwrap();
        let latex = || {
            let output = std::process::Command::new("pdflatex")
                .args([
                    "-no-shell-escape",
                    "-interaction=nonstopmode",
                    "-halt-on-error",
                    "main.tex",
                ])
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        };
        latex();
        let output = std::process::Command::new("biber")
            .arg("main")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        latex();
        latex();
        let log = std::fs::read_to_string(dir.path().join("main.log")).unwrap();
        assert!(!log.contains("undefined"), "{log}");
        let bbl = std::fs::read_to_string(dir.path().join("main.bbl")).unwrap();
        assert!(bbl.contains("Selected"), "{bbl}");
        let bcf = std::fs::read_to_string(dir.path().join("main.bcf")).unwrap();
        assert!(
            bcf.contains(if moved {
                ">selected%renamed.bib<"
            } else {
                ">section.bib<"
            }),
            "{bcf}"
        );
    }
}

#[test]
fn section_filename_edits_cannot_be_redirected_by_labels_or_close_the_list() {
    let source = r"\documentclass{article}\addsectionbib[label=shadow.bib]{other.bib}\newrefsection[old.bib]";
    let (dir, ws, main) = setup(
        source,
        &[
            ("old.bib", DATA),
            ("other.bib", DATA),
            ("shadow.bib", DATA),
            ("broken].bib", DATA),
            ("broken.bib.bib", DATA),
        ],
    );
    let old = Url::from_file_path(dir.path().join("old.bib")).unwrap();
    for name in ["shadow.bib", "broken].bib", "broken.bib.bib"] {
        assert!(
            file_operations::will_rename(
                &ws,
                &[FileRename {
                    old_uri: old.to_string(),
                    new_uri: Url::from_file_path(dir.path().join(name))
                        .unwrap()
                        .to_string()
                }]
            )
            .is_err(),
            "{name}"
        );
    }
    let at = Text::new(source.into()).position(source.rfind("old.bib").unwrap());
    let items = features::completion(&ws, &main, at);
    assert!(!items.iter().any(|item| matches!(
        item.label.as_str(),
        "shadow.bib" | "broken].bib" | "broken.bib.bib"
    )));
    assert!(items.iter().any(|item| item.label == "old.bib"));
}
