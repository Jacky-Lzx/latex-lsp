use latex_lsp::{
    language::features,
    project::workspace::{Document, Workspace},
};
use lsp_types::*;
use std::{collections::BTreeSet, path::Path};

const LOADERS: [&str; 4] = ["input", "include", "subfile", "InputIfFileExists"];
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn workspace(root: &Path, files: &[(&str, &str)]) -> Workspace {
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
fn call(name: &str, argument: &str) -> String {
    format!(
        "\\{name}{argument}{}",
        if name == "InputIfFileExists" {
            "{}{}"
        } else {
            ""
        }
    )
}
fn command_position(workspace: &Workspace, main: &Url, name: &str) -> Position {
    let doc = &workspace.documents[main];
    doc.text.position(doc.text.source.rfind(name).unwrap() + 3)
}

#[test]
fn quoted_source_paths_do_not_import_dependencies_or_symbols() {
    for loader in LOADERS {
        for argument in [
            "{chapter}",
            "% before 😀\r\n{chapter.tex}",
            "{chapter% filename 😀\r\n}",
            "{% before 😀\r\nchapter% filename 😀\r\n}",
        ] {
            let command = call(loader, argument);
            for body in [
                format!("\\detokenize{{{command}}}"),
                format!("\\string{command}"),
                format!("\\meaning{command}"),
                format!("\\newcommand{{\\future}}{{\\detokenize{{{command}}}}}\\future"),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!(
                    r"\documentclass{{article}}\newcommand{{\hold}}[1]{{}}\def\take#1STOP{{}}\begin{{document}}{body}\ifdefined\fromchapter\fromchapter\fi\end{{document}}"
                );
                let workspace = workspace(
                    root,
                    &[
                        ("main.tex", &source),
                        (
                            "chapter.tex",
                            r"\newcommand{\fromchapter}{Hidden}\input{nested}",
                        ),
                        ("nested.tex", r"\newcommand{\fromnested}{Hidden}"),
                    ],
                );
                let main = uri(root, "main.tex");
                let project = workspace.project_files(&main);
                for name in ["chapter.tex", "nested.tex"] {
                    assert!(
                        !project.contains(&uri(root, name)),
                        "{loader}/{body}: {project:?}"
                    );
                }
                let at = command_position(&workspace, &main, r"\fromchapter");
                assert!(
                    features::definitions(&workspace, &main, at).is_empty(),
                    "{loader}/{body}"
                );
                assert!(features::prepare_rename(&workspace, &main, at)
                    .unwrap()
                    .is_none());
                assert!(!features::completion(&workspace, &main, at)
                    .iter()
                    .any(|item| item.label == "fromchapter"));
            }
        }
    }
}

#[test]
fn direct_and_deferred_source_paths_preserve_real_source_editing() {
    for loader in LOADERS {
        let command = call(loader, "{chapter}");
        for body in [
            command.clone(),
            format!("\\newcommand{{\\future}}{{{command}}}\\future"),
            format!("\\def\\future{{{command}}}\\future"),
            format!("\\newcommand{{\\execute}}[1]{{#1}}\\execute{{{command}}}"),
            format!("\\newcommand{{\\execute}}[1]{{#1}}\\execute{command}"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let source = format!(
                r"\documentclass{{article}}\begin{{document}}{body}\fromchapter\end{{document}}"
            );
            let workspace = workspace(
                root,
                &[
                    ("main.tex", &source),
                    (
                        "chapter.tex",
                        r"\newcommand{\fromchapter}{Real}\input{nested}",
                    ),
                    ("nested.tex", r"\newcommand{\fromnested}{Real}"),
                ],
            );
            let main = uri(root, "main.tex");
            for name in ["chapter.tex", "nested.tex"] {
                assert!(
                    workspace.project_files(&main).contains(&uri(root, name)),
                    "{loader}/{body}"
                );
            }
            let at = command_position(&workspace, &main, r"\fromchapter");
            assert_eq!(
                features::definitions(&workspace, &main, at)[0].uri,
                uri(root, "chapter.tex")
            );
            let edit = features::rename(&workspace, &main, at, "renamedchapter")
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
fn source_roles_are_project_specific_and_use_active_cross_file_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = r"\documentclass{article}\input{settings}\begin{document}\detokenize{\input{chapter}}\string\include{chapter}\ifdefined\fromchapter\fromchapter\fi\end{document}";
    let second =
        r"\documentclass{article}\begin{document}\include{chapter}\fromchapter\end{document}";
    let workspace = workspace(
        root,
        &[
            ("first.tex", first),
            ("second.tex", second),
            ("settings.tex", r"\newcommand{\fromsettings}{Settings}"),
            ("chapter.tex", r"\newcommand{\fromchapter}{Real}"),
        ],
    );
    let first = uri(root, "first.tex");
    let second = uri(root, "second.tex");
    assert!(workspace
        .project_documents(&first)
        .contains(&uri(root, "settings.tex")));
    assert!(!workspace
        .project_documents(&first)
        .contains(&uri(root, "chapter.tex")));
    assert!(workspace
        .project_documents(&second)
        .contains(&uri(root, "chapter.tex")));
    assert!(features::definitions(
        &workspace,
        &first,
        command_position(&workspace, &first, r"\fromchapter")
    )
    .is_empty());
    let at = command_position(&workspace, &second, r"\fromchapter");
    assert_eq!(
        features::definitions(&workspace, &second, at)[0].uri,
        uri(root, "chapter.tex")
    );
    let edit = features::rename(&workspace, &second, at, "renamedchapter")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(edits)) = edit.document_changes else {
        panic!()
    };
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit.text_document.uri != first));
}

#[test]
fn quoted_literal_sources_cannot_supply_project_definitions_or_other_inputs() {
    for body in [
        r"\detokenize{\include{defs}}",
        r"\string\include{defs}",
        r"\meaning\include{defs}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!(
            r"\documentclass{{article}}\begin{{document}}{body}\ifdefined\readpart\readpart{{part}}\fi\end{{document}}"
        );
        let workspace = workspace(
            root,
            &[
                ("main.tex", &source),
                ("defs.tex", r"\def\readpart{\input}"),
                ("part.tex", "Hidden"),
            ],
        );
        let main = uri(root, "main.tex");
        let project = workspace.project_documents(&main);
        assert!(!project.contains(&uri(root, "defs.tex")), "{body}");
        assert!(!project.contains(&uri(root, "part.tex")), "{body}");
    }
}

#[test]
fn recorder_confirmed_dynamic_sources_and_redefined_primitives_keep_save_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = r"\documentclass{article}\newcommand{\hold}[1]{}\def\picked{chapter}\begin{document}\detokenize{\input{chapter}}\input{\picked}\fromchapter\end{document}";
    let mut workspace = workspace(
        root,
        &[
            ("main.tex", source),
            (
                "chapter.tex",
                r"\newcommand{\fromchapter}{Real}\input{nested}",
            ),
            ("nested.tex", "Real"),
        ],
    );
    let main = uri(root, "main.tex");
    let chapter = uri(root, "chapter.tex");
    workspace
        .recorded_inputs
        .insert(main.clone(), BTreeSet::from([chapter.clone()]));
    assert!(workspace.project_files(&main).contains(&chapter));
    assert!(workspace
        .project_files(&main)
        .contains(&uri(root, "nested.tex")));
    let at = command_position(&workspace, &main, r"\fromchapter");
    assert_eq!(features::definitions(&workspace, &main, at)[0].uri, chapter);
    workspace.documents.insert(main.clone(), Document::new(&main, r"\documentclass{article}\def\meaning#1{#1}\begin{document}\meaning\input{chapter}\end{document}".into(), Some(8)));
    workspace.recorded_inputs.clear();
    assert!(workspace.project_files(&main).contains(&chapter));
    // Unknown user macros may execute their arguments and keep their sources.
    workspace.documents.insert(main.clone(), Document::new(&main, r"\documentclass{article}\newcommand{\hold}[1]{\detokenize{#1}}\begin{document}\hold{\input{chapter}}\end{document}".into(), Some(9)));
    assert!(workspace.project_files(&main).contains(&chapter));
    for source in [
        r"\documentclass{article}\def\picked{provider}\begin{document}\input{\picked}\meaning\input{chapter}\end{document}",
        r"\documentclass{article}\newcommand{\install}{\def\meaning##1{##1}}\begin{document}\install\meaning\input{chapter}\end{document}",
        r"\documentclass{article}\newcommand{\execute}[1]{#1}\begin{document}\let\meaning\execute\meaning\input{chapter}\end{document}",
        r"\documentclass{article}\def\picked{provider}\def\readpart{\input}\begin{document}\readpart{\picked}\meaning\input{chapter}\end{document}",
        r"\documentclass{article}\makeatletter\@input{provider.tex}\makeatother\begin{document}\meaning\input{chapter}\end{document}",
        r"\documentclass{article}\csname @input\endcsname{provider.tex}\begin{document}\meaning\input{chapter}\end{document}",
        r"\documentclass{article}\def\readername{@input}\csname\readername\endcsname{provider.tex}\begin{document}\meaning\input{chapter}\end{document}",
        r"\documentclass{article}\newcommand{\execute}[1]{#1}\execute{\input{provider.tex}}\begin{document}\meaning\input{chapter}\end{document}",
        r"\documentclass{article}\newcommand{\future}{\input{provider.tex}}\future\begin{document}\meaning\input{chapter}\end{document}",
    ] {
        workspace
            .documents
            .insert(main.clone(), Document::new(&main, source.into(), Some(10)));
        assert!(
            workspace.project_files(&main).contains(&chapter),
            "{source}"
        );
    }
}

#[test]
fn commented_providers_keep_primitive_rebindings_in_the_dependency_context() {
    for (loader, extension) in [
        ("input", "tex"),
        ("include", "tex"),
        ("InputIfFileExists", "tex"),
        ("usepackage", "sty"),
        ("RequirePackage", "sty"),
        ("RequirePackageWithOptions", "sty"),
        ("documentclass", "cls"),
        ("LoadClass", "cls"),
        ("LoadClassWithOptions", "cls"),
    ] {
        for argument in ["{provider% filename 😀\r\n}", "{% before\nprovider}"] {
            let suffix = if loader == "InputIfFileExists" {
                "{}{}"
            } else {
                ""
            };
            for invocation in [
                format!("\\{loader}{argument}{suffix}"),
                format!("\\csname {loader}\\endcsname{argument}{suffix}"),
                format!(
                    "\\newcommand{{\\readprovider}}{{\\{loader}}}\\readprovider{argument}{suffix}"
                ),
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let source = format!("{invocation}\\meaning\\input{{chapter}}\\fromchapter");
                let provider = format!("provider.{extension}");
                let workspace = workspace(
                    root,
                    &[
                        ("main.tex", &source),
                        (&provider, r"\def\meaning#1{#1}"),
                        ("chapter.tex", r"\newcommand{\fromchapter}{Real}"),
                    ],
                );
                let main = uri(root, "main.tex");
                let project = workspace.project_files(&main);
                assert!(
                    project.contains(&uri(root, &provider)),
                    "{source}: {project:?}"
                );
                let chapter = uri(root, "chapter.tex");
                assert!(project.contains(&chapter), "{source}: {project:?}");
                let at = command_position(&workspace, &main, r"\fromchapter");
                assert_eq!(
                    features::definitions(&workspace, &main, at)[0].uri,
                    chapter,
                    "{source}"
                );
            }
        }
    }
}
