use latex_lsp::{
    features,
    workspace::{DocumentKind, Workspace},
};
use std::{collections::BTreeSet, path::Path};
use tower_lsp::lsp_types::*;

const COMMANDS: [&str; 5] = [
    "bibliography",
    "nobibliography",
    "addbibresource",
    "addglobalbib",
    "bibliographystyle",
];
fn extension(name: &str) -> &str {
    if name == "bibliographystyle" {
        "bst"
    } else {
        "bib"
    }
}
fn resource_name(name: &str) -> &str {
    if matches!(name, "addbibresource" | "addglobalbib") {
        "refs.bib"
    } else {
        "refs"
    }
}
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn project(root: &Path, source: &str, name: &str) -> Workspace {
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(
        root.join(format!("refs.{}", extension(name))),
        "@book{alpha,title={BibliographyUnique}}\n",
    )
    .unwrap();
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
fn citation_position(workspace: &Workspace, main: &Url) -> Position {
    let doc = &workspace.documents[main];
    doc.text
        .position(doc.text.source.rfind("alpha").unwrap() + 2)
}

#[test]
fn quoted_resource_tokens_do_not_import_saved_dependencies_or_citation_definitions() {
    for name in COMMANDS {
        for argument in [
            format!("{{{}}}", resource_name(name)),
            format!("% before 😀\r\n{{refs.{}}}", extension(name)),
        ] {
            let command = format!("\\{name}{argument}");
            for body in [
                format!("\\meaning{command}"),
                format!("\\csuse{{meaning}}{command}"),
                format!("\\csname meaning\\endcsname{command}"),
                format!("\\newcommand{{\\future}}{{\\meaning{command}}}\\future"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!(
                    r"\documentclass{{article}}\begin{{document}}{body}\cite{{alpha}}\end{{document}}"
                );
                let workspace = project(root, &source, name);
                let main = uri(root, "main.tex");
                let target = uri(root, &format!("refs.{}", extension(name)));
                assert!(
                    !workspace.project_files(&main).contains(&target),
                    "{name}/{body}"
                );
                let at = citation_position(&workspace, &main);
                assert!(
                    features::definitions(&workspace, &main, at).is_empty(),
                    "{name}/{body}"
                );
                assert!(
                    features::prepare_rename(&workspace, &main, at)
                        .unwrap()
                        .is_none(),
                    "{name}/{body}"
                );
            }
        }
    }
}

#[test]
fn ordinary_deferred_and_forwarded_resource_calls_keep_their_data_roles_and_symbol_edits() {
    for name in COMMANDS {
        let command = format!("\\{name}{{{}}}", resource_name(name));
        for body in [
            command.clone(),
            format!("\\newcommand{{\\future}}{{{command}}}\\future"),
            format!("\\newcommand{{\\execute}}[1]{{#1}}\\execute{{{command}}}"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!(
                r"\documentclass{{article}}\begin{{document}}{body}\cite{{alpha}}\end{{document}}"
            );
            let workspace = project(root, &source, name);
            let main = uri(root, "main.tex");
            let target = uri(root, &format!("refs.{}", extension(name)));
            assert!(
                workspace.project_files(&main).contains(&target),
                "{name}/{body}"
            );
            assert_eq!(
                workspace.documents[&target].kind,
                if name == "bibliographystyle" {
                    DocumentKind::BibtexStyle
                } else {
                    DocumentKind::Bibtex
                }
            );
            let at = citation_position(&workspace, &main);
            let definitions = features::definitions(&workspace, &main, at);
            if name == "bibliographystyle" {
                assert!(definitions.is_empty());
                continue;
            }
            assert_eq!(definitions[0].uri, target, "{name}/{body}");
            let edit = features::rename(&workspace, &main, at, "renamedalpha")
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
fn recorded_resources_and_uncertain_primitive_bindings_remain_potential_build_dependencies() {
    for name in COMMANDS {
        let command = format!("\\{name}{{{}}}", resource_name(name));
        for (body,recorded) in [
            (format!("\\meaning{command}"),true),
            (format!("\\def\\picked{{provider}}\\input{{\\picked}}\\meaning{command}"),false),
            (format!("\\newcommand{{\\install}}{{\\def\\meaning##1{{##1}}}}\\install\\meaning{command}"),false),
            (format!("\\newcommand{{\\execute}}[1]{{#1}}\\let\\meaning\\execute\\meaning{command}"),false),
            (format!("\\makeatletter\\@input{{provider.tex}}\\makeatother\\meaning{command}"),false),
            (format!("\\makeatletter\\@input@{{provider.tex}}\\makeatother\\meaning{command}"),false),
            (format!("\\makeatletter\\@@input{{provider.tex}}\\makeatother\\meaning{command}"),false),
            (format!("\\csname @input\\endcsname{{provider.tex}}\\meaning{command}"),false),
            (format!("\\csuse{{@input}}{{provider.tex}}\\meaning{command}"),false),
            (format!("\\def\\readername{{@input}}\\csuse{{\\readername}}{{provider.tex}}\\meaning{command}"),false),
            (format!("\\def\\readername{{@input}}\\csname\\readername\\endcsname{{provider.tex}}\\meaning{command}"),false),
            (format!("\\csuse{{% reader\n@input}}{{provider.tex}}\\meaning{command}"),false),
            (format!("\\csname @input% reader\n\\endcsname{{provider.tex}}\\meaning{command}"),false),
            (format!("\\newcommand{{\\execute}}[1]{{#1}}\\execute{{\\input{{provider.tex}}}}\\meaning{command}"),false),
            (format!("\\newcommand{{\\future}}{{\\input{{provider.tex}}}}\\future\\meaning{command}"),false),
            (format!("\\scantokens{{\\input{{provider.tex}}}}\\meaning{command}"),false),
        ]{
            let directory=tempfile::tempdir().unwrap();let root=directory.path();
            let source=format!(r"\documentclass{{article}}\begin{{document}}{body}\end{{document}}");
            let mut workspace=project(root,&source,name);let main=uri(root,"main.tex");let target=uri(root,&format!("refs.{}",extension(name)));
            if recorded{workspace.bibliography_inputs.insert(main.clone(),BTreeSet::from([target.clone()]));}
            assert!(workspace.project_files(&main).contains(&target),"{name}/{body}");
        }
    }
}
