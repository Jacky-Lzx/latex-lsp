use latex_lsp::{features, workspace::Workspace};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, text) in files {
        std::fs::write(root.join(name), text).unwrap();
    }
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Workspace::default()
    };
    workspace.add_root(root.into());
    for doc in workspace.documents.values_mut() {
        doc.version = Some(7);
    }
    workspace
}
fn changes(workspace: &Workspace, edit: WorkspaceEdit) -> BTreeMap<Url, String> {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edits")
    };
    documents
        .into_iter()
        .map(|document| {
            let uri = document.text_document.uri;
            assert_eq!(
                document.text_document.version,
                workspace.documents[&uri].version
            );
            let mut edits: Vec<_> = document
                .edits
                .into_iter()
                .map(|edit| match edit {
                    OneOf::Left(edit) => edit,
                    _ => panic!(),
                })
                .collect();
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            let source = workspace.documents[&uri]
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
                .source;
            (uri, source)
        })
        .collect()
}

#[test]
fn indirect_environment_names_support_navigation_and_versioned_rename() {
    for (begin, end) in [
        (r"\begin", r"\csuse{end}"),
        (r"\csuse{begin}", r"\end"),
        (r"\csuse{begin}", r"\csuse{end}"),
        (r"\csname begin\endcsname", r"\csname end\endcsname"),
        (r"\start", r"\finish"),
        (r"\csuse{start}", r"\csuse{finish}"),
        (r"\csname start\endcsname", r"\csname finish\endcsname"),
    ] {
        for external in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let declaration = r"\newenvironment{panel}[1]{#1}{}\def\start{\begin}\def\finish{\end}";
            let code = format!("\\documentclass{{article}}{}\\begin{{document}}😀 {begin}{{% name\r\npanel% keep\r\n}}{{Text}}{end}{{panel}}\\end{{document}}", if external { r"\usepackage{aliases}" } else { declaration });
            let source =
                format!("{code}% ignored \\csuse{{begin}}{{panel}}\n\\verb|\\start{{panel}}|\n");
            let macros = if external { declaration } else { "" };
            let other = r"\documentclass{article}\newenvironment{panel}{}{}\newcommand{\start}[1]{#1}\begin{document}Other\end{document}";
            let workspace = project(
                root,
                &[
                    ("main.tex", &source),
                    ("aliases.sty", macros),
                    ("other.tex", other),
                ],
            );
            let main = uri(root, "main.tex");
            let defs = if external {
                uri(root, "aliases.sty")
            } else {
                main.clone()
            };
            let definition_offset = workspace.documents[&defs]
                .text
                .source
                .find("panel")
                .unwrap();
            for (target, offset) in [
                (&defs, definition_offset),
                (&main, source.find("panel% keep").unwrap()),
                (&main, source.rfind("{panel}\\end{document}").unwrap() + 1),
            ] {
                let position = workspace.documents[target].text.position(offset + 2);
                assert_eq!(
                    features::definitions(&workspace, target, position).len(),
                    1,
                    "{begin}: {target}"
                );
                assert_eq!(
                    features::references(&workspace, target, position, true).len(),
                    3,
                    "{begin}: {target}"
                );
                assert!(features::hover(&workspace, target, position).is_some());
                assert!(
                    matches!(features::prepare_rename(&workspace, target, position).unwrap(), Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) if placeholder == "panel")
                );
                let edited = changes(
                    &workspace,
                    features::rename(&workspace, target, position, "renamed")
                        .unwrap()
                        .unwrap(),
                );
                assert_eq!(
                    edited[&main],
                    format!(
                        "{}% ignored \\csuse{{begin}}{{panel}}\n\\verb|\\start{{panel}}|\n",
                        code.replace("panel", "renamed")
                    )
                );
                assert!(!edited.contains_key(&uri(root, "other.tex")));
                if external {
                    assert_eq!(edited[&defs], declaration.replace("panel", "renamed"));
                }
            }
        }
    }
}

#[test]
fn shared_alias_calls_require_the_same_meaning_in_every_consumer() {
    for agree in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let a = r"\documentclass{article}\usepackage{defs}\def\start{\begin}\def\finish{\end}\begin{document}\input{shared}\end{document}";
        let b = if agree {
            a
        } else {
            r"\documentclass{article}\usepackage{defs}\newcommand{\start}[1]{#1}\newcommand{\finish}[1]{#1}\begin{document}\input{shared}\end{document}"
        };
        let workspace = project(
            root,
            &[
                ("a.tex", a),
                ("b.tex", b),
                ("defs.sty", r"\newenvironment{panel}[1]{#1}{}"),
                ("shared.tex", r"\start{panel}{Text}\finish{panel}"),
            ],
        );
        let defs = uri(root, "defs.sty");
        let position = workspace.documents[&defs].text.position(
            workspace.documents[&defs]
                .text
                .source
                .find("panel")
                .unwrap()
                + 1,
        );
        let result = features::rename(&workspace, &defs, position, "renamed");
        if agree {
            let edited = changes(&workspace, result.unwrap().unwrap());
            assert_eq!(edited.len(), 2);
            assert_eq!(
                edited[&uri(root, "shared.tex")],
                r"\start{renamed}{Text}\finish{renamed}"
            );
        } else {
            assert!(result.is_err());
            assert!(features::prepare_rename(&workspace, &defs, position).is_err());
        }
    }
}

#[test]
fn indirect_names_inside_macro_arguments_are_not_editable_references() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = r"\documentclass{article}\newenvironment{panel}{}{}\newcommand{\take}[1]{\detokenize{#1}}\begin{document}\take{\csuse{begin}{panel}}\end{document}";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let position = workspace.documents[&main]
        .text
        .position(source.find("panel").unwrap() + 1);
    assert_eq!(
        features::references(&workspace, &main, position, true).len(),
        1
    );
    assert!(features::rename(&workspace, &main, position, "renamed").is_err());
    let callback = workspace.documents[&main]
        .text
        .position(source.rfind("panel").unwrap() + 1);
    assert!(features::prepare_rename(&workspace, &main, callback)
        .unwrap()
        .is_none());
}

#[test]
fn redefined_begin_does_not_turn_literal_arguments_into_environment_names() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = r"\documentclass{article}\newenvironment{panel}{}{}\begin{document}\renewcommand{\begin}[1]{#1}\csuse{begin}{panel}\end{document}";
    let workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let position = workspace.documents[&main]
        .text
        .position(source.find("panel").unwrap() + 1);
    let edited = changes(
        &workspace,
        features::rename(&workspace, &main, position, "renamed")
            .unwrap()
            .unwrap(),
    );
    assert_eq!(edited[&main], source.replacen("panel", "renamed", 1));
    let literal = workspace.documents[&main]
        .text
        .position(source.rfind("panel").unwrap() + 1);
    assert!(features::definitions(&workspace, &main, literal).is_empty());
}
