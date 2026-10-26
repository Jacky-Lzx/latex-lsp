use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;
use std::{collections::BTreeSet, path::Path};

const LOADERS: [(&str, &str); 6] = [
    ("usepackage", "sty"),
    ("RequirePackage", "sty"),
    ("RequirePackageWithOptions", "sty"),
    ("documentclass", "cls"),
    ("LoadClass", "cls"),
    ("LoadClassWithOptions", "cls"),
];
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn project(root: &Path, source: &str, extension: &str) -> Workspace {
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(
        root.join(format!("refs.{extension}")),
        r"\newcommand{\unreadcommand}{Hidden}\input{nested}",
    )
    .unwrap();
    std::fs::write(
        root.join("nested.tex"),
        r"\newcommand{\nestedcommand}{Hidden}",
    )
    .unwrap();
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
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
fn at(workspace: &Workspace, main: &Url) -> Position {
    let doc = &workspace.documents[main];
    doc.text
        .position(doc.text.source.rfind("unreadcommand").unwrap() + 3)
}

#[test]
fn quoted_loader_tokens_do_not_import_packages_classes_or_their_symbols() {
    for (name, extension) in LOADERS {
        for argument in [
            "{refs}",
            "% before 😀\r\n{refs}% after\r\n",
            "{refs% filename 😀\r\n}",
            "{% before 😀\r\nrefs% filename 😀\r\n}",
        ] {
            let command = format!("\\{name}{argument}");
            for body in [
                format!("\\meaning{command}"),
                format!("\\string{command}"),
                format!("\\detokenize{{{command}}}"),
                format!("\\csuse{{meaning}}{command}"),
                format!("\\csname meaning\\endcsname{command}"),
                format!("\\newcommand{{\\future}}{{\\meaning{command}}}\\future"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!(
                    r"\documentclass{{article}}\begin{{document}}{body}\ifdefined\unreadcommand\unreadcommand\fi\end{{document}}"
                );
                let workspace = project(root, &source, extension);
                let main = uri(root, "main.tex");
                let files = workspace.project_files(&main);
                assert!(
                    !files.contains(&uri(root, &format!("refs.{extension}"))),
                    "{name}/{body}: {files:?}"
                );
                assert!(
                    !files.contains(&uri(root, "nested.tex")),
                    "{name}/{body}: {files:?}"
                );
                let position = at(&workspace, &main);
                assert!(
                    features::definitions(&workspace, &main, position).is_empty(),
                    "{name}/{body}"
                );
                assert!(
                    features::prepare_rename(&workspace, &main, position)
                        .unwrap()
                        .is_none(),
                    "{name}/{body}"
                );
                assert!(
                    !features::completion(&workspace, &main, position)
                        .iter()
                        .any(|item| item.label == "unreadcommand"),
                    "{name}/{body}"
                );
            }
        }
    }
}

#[test]
fn real_direct_deferred_and_forwarded_loaders_keep_symbols_and_versioned_edits() {
    for (name, extension) in LOADERS {
        let command = format!("\\{name}{{refs}}");
        for body in [
            command.clone(),
            format!("\\newcommand{{\\future}}{{{command}}}\\future"),
            format!("\\newcommand{{\\execute}}[1]{{#1}}\\execute{{{command}}}"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!(
                r"\documentclass{{article}}\begin{{document}}{body}\unreadcommand\end{{document}}"
            );
            let workspace = project(root, &source, extension);
            let main = uri(root, "main.tex");
            let target = uri(root, &format!("refs.{extension}"));
            assert!(
                workspace.project_files(&main).contains(&target),
                "{name}/{body}"
            );
            assert!(
                workspace
                    .project_files(&main)
                    .contains(&uri(root, "nested.tex")),
                "{name}/{body}"
            );
            let position = at(&workspace, &main);
            assert_eq!(
                features::definitions(&workspace, &main, position)[0].uri,
                target,
                "{name}/{body}"
            );
            let edit = features::rename(&workspace, &main, position, "renamedcommand")
                .unwrap()
                .unwrap();
            let Some(DocumentChanges::Edits(edits)) = edit.document_changes else {
                panic!()
            };
            assert_eq!(edits.len(), 2);
            assert!(edits
                .iter()
                .all(|edit| edit.text_document.version == Some(7)));
        }
    }
}

#[test]
fn recorder_sources_dynamic_inputs_and_primitive_rebindings_keep_potential_loader_dependencies() {
    for (name, extension) in LOADERS {
        let command = format!("\\meaning\\{name}{{refs}}");
        for (body, recorded) in [
            (command.clone(), true),
            (format!("\\def\\meaning#1{{#1}}{command}"), false),
            (
                format!("\\newcommand{{\\install}}{{\\def\\meaning##1{{##1}}}}\\install{command}"),
                false,
            ),
            (
                format!("\\newcommand{{\\execute}}[1]{{#1}}\\let\\meaning\\execute{command}"),
                false,
            ),
            (
                format!("\\def\\picked{{provider}}\\input{{\\picked}}{command}"),
                false,
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source =
                format!(r"\documentclass{{article}}\begin{{document}}{body}\end{{document}}");
            let mut workspace = project(root, &source, extension);
            let main = uri(root, "main.tex");
            let target = uri(root, &format!("refs.{extension}"));
            if recorded {
                workspace
                    .project
                    .recorded_inputs
                    .insert(main.clone(), BTreeSet::from([target.clone()]));
            }
            assert!(
                workspace.project_files(&main).contains(&target),
                "{name}/{body}"
            );
        }
    }
}

#[test]
fn dynamic_siblings_and_fragmented_provider_names_keep_potential_loader_dependencies() {
    for (name, extension) in LOADERS {
        for provider in [
            r"\def\picked{provider}\usepackage{known,\picked}",
            "\\usepackage{pro% split 😀\r\nvider}",
            "\\input{pro% split 😀\r\nvider}",
            "\\csname input\\endcsname{pro% split 😀\r\nvider}",
            "\\newcommand{\\readprovider}{\\input}\\readprovider{pro% split 😀\r\nvider}",
            "\\documentclass{pro% split 😀\r\nvider}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!("{provider}\\meaning\\{name}{{refs}}");
            let workspace = project(root, &source, extension);
            let main = uri(root, "main.tex");
            assert!(
                workspace
                    .project_files(&main)
                    .contains(&uri(root, &format!("refs.{extension}"))),
                "{source}"
            );
        }
    }
}
