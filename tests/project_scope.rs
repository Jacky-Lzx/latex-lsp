use lsp_types::*;
use std::path::Path;
use texquill::{
    language::{features, signature},
    project::workspace::{Document, Workspace},
};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project() -> (tempfile::TempDir, Workspace) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(root.join(format!("{name}/main.tex")),"\\documentclass{article}\n\\usepackage{macros}\n\\begin{document}\n\\input{chapter}\n\\ref{sharedkey} \\cite{book} \\pair{first}{second}\n\\begin{panel}{first}{second}\\end{panel}\n\\bibliography{refs}\n\\end{document}\n").unwrap();
        std::fs::write(
            root.join(format!("{name}/chapter.tex")),
            format!("\\label{{sharedkey}}\\label{{{name}:only}}\n"),
        )
        .unwrap();
        std::fs::write(
            root.join(format!("{name}/refs.bib")),
            format!(
                "@book{{book,title={{Shared name}}}}\n@book{{{name}onlybook,title={{Local}}}}\n"
            ),
        )
        .unwrap();
        let count = if name == "a" { 2 } else { 1 };
        std::fs::write(root.join(format!("{name}/macros.sty")),format!("\\newcommand{{\\pair}}[{count}]{{#1}}\n\\newcommand{{\\{name}onlycommand}}{{Local}}\n\\newenvironment{{panel}}[{count}]{{#1}}{{}}\n\\newenvironment{{{name}onlyenvironment}}{{}}{{}}\n")).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    (directory, workspace)
}

#[test]
fn recorded_local_and_shared_data_remains_outside_the_latex_symbol_index() {
    let directory = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n").unwrap();
    let local = directory.path().join("table.CSV");
    let external = shared.path().join("data.json");
    let system_package = shared.path().join("system.sty");
    for path in [&local, &external, &system_package] {
        std::fs::write(path, "\\label{data-source}\\input{not-a-dependency}\n").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let uri = Url::from_file_path(&main).unwrap();
    let inputs = texquill::compilation::recorder::inputs(
        &main,
        &format!(
            "INPUT {}\nINPUT {}\nINPUT {}\nINPUT {}\n",
            main.display(),
            local.display(),
            external.display(),
            system_package.display()
        ),
    );
    let selected = workspace.project_inputs(&uri, &inputs);
    let expected = [&main, &local, &external]
        .map(|path| Url::from_file_path(path).unwrap())
        .into();
    assert_eq!(selected, expected);
    workspace
        .project
        .recorded_inputs
        .insert(uri.clone(), selected.clone());
    workspace.refresh_project(&uri);
    assert_eq!(workspace.project_files(&uri), selected);
    assert_eq!(workspace.documents.len(), 1);
    // Closed data watcher updates must not silently introduce LaTeX symbols.
    for data in [&local, &external] {
        workspace.reload(&Url::from_file_path(data).unwrap());
    }
    assert_eq!(workspace.documents.len(), 1);
}

#[test]
fn explicit_data_extension_source_inputs_still_load_nested_sources_and_update_on_reload() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = directory.path().join("notes.txt");
    let child = directory.path().join("details.csv");
    std::fs::write(&main, "\\documentclass{article}\n\\input{notes.txt}\n").unwrap();
    std::fs::write(&source, "\\label{note}\\input{details.csv}\n").unwrap();
    std::fs::write(&child, "\\label{detail}\n").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let uri = Url::from_file_path(&main).unwrap();
    workspace.refresh_project(&uri);
    let source_uri = Url::from_file_path(&source).unwrap();
    let child_uri = Url::from_file_path(&child).unwrap();
    assert_eq!(
        workspace.documents[&source_uri].syntax.labels[0].name,
        "note"
    );
    assert_eq!(
        workspace.documents[&child_uri].syntax.labels[0].name,
        "detail"
    );
    std::fs::write(&child, "\\label{updated-detail}\n").unwrap();
    workspace.reload(&child_uri);
    assert_eq!(
        workspace.documents[&child_uri].syntax.labels[0].name,
        "updated-detail"
    );
}

#[test]
fn definitions_references_hover_and_signatures_respect_the_active_paper() {
    let (directory, workspace) = project();
    let root = directory.path();
    let main = uri(root, "a/main.tex");
    let text = &workspace.documents[&main].text;
    for (needle, offset, target_file) in [
        ("ref{sharedkey}", 5, "a/chapter.tex"),
        ("cite{book}", 6, "a/refs.bib"),
        ("\\pair{first}", 2, "a/macros.sty"),
        ("begin{panel}", 7, "a/macros.sty"),
    ] {
        let position = text.position(text.source.find(needle).unwrap() + offset);
        let definitions = features::definitions(&workspace, &main, position);
        assert_eq!(definitions.len(), 1, "{needle}: {definitions:?}");
        assert_eq!(definitions[0].uri, uri(root, target_file));
        let references = features::references(&workspace, &main, position, true);
        assert!(
            references.iter().all(|location| location
                .uri
                .to_file_path()
                .unwrap()
                .starts_with(root.join("a"))),
            "{references:?}"
        );
        let hover = features::hover(&workspace, &main, position).unwrap();
        let HoverContents::Markup(MarkupContent { value, .. }) = hover.contents else {
            panic!();
        };
        if target_file.ends_with(".bib") {
            assert_eq!(value, "```bibtex\n@book{book,title={Shared name}}\n```");
        } else {
            assert!(value.contains("/a/"), "{value}");
        }
    }
    let position = text.position(text.source.find("second}").unwrap() + 3);
    let help = signature::help(&workspace, &main, position).unwrap();
    assert_eq!(help.signatures[0].label, r"\pair{#1}{#2}");
    assert_eq!(help.active_parameter, Some(1));
    let position = text.position(text.source.rfind("second}").unwrap() + 3);
    let help = signature::help(&workspace, &main, position).unwrap();
    assert_eq!(help.signatures[0].label, r"\begin{panel}{#1}{#2}");
    assert_eq!(help.active_parameter, Some(2));
}

#[test]
fn completions_and_user_snippets_exclude_symbols_from_independent_papers() {
    let (directory, mut workspace) = project();
    let root = directory.path();
    let main = uri(root, "a/main.tex");
    workspace.client.snippets = true;
    let prefix =
        "\\documentclass{article}\\usepackage{macros}\\input{chapter}\\bibliography{refs}\n";
    for (suffix, present, absent) in [
        (r"\ref{", "a:only", "b:only"),
        (r"\cite{", "aonlybook", "bonlybook"),
        (r"\begin{", "aonlyenvironment", "bonlyenvironment"),
        (r"\", "aonlycommand", "bonlycommand"),
    ] {
        let source = format!("{prefix}{suffix}");
        let position = texquill::text::Text::new(source.clone()).position(source.len());
        workspace
            .documents
            .insert(main.clone(), Document::new(&main, source, Some(2)));
        let items = features::completion(&workspace, &main, position);
        assert!(
            items.iter().any(|item| item.label == present),
            "{suffix}: {items:?}"
        );
        assert!(
            !items.iter().any(|item| item.label == absent),
            "{suffix}: {items:?}"
        );
    }
    let source = format!("{prefix}\\pa");
    let position = texquill::text::Text::new(source.clone()).position(source.len());
    workspace
        .documents
        .insert(main.clone(), Document::new(&main, source, Some(3)));
    let items = features::completion(&workspace, &main, position);
    let pair = items.iter().find(|item| item.label == "pair").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &pair.text_edit else {
        panic!();
    };
    assert_eq!(edit.new_text, "pair{${1:arg1}}{${2:arg2}}");
    let bib = uri(root, "a/refs.bib");
    let source = "@book{book,title={Example}}\n@book{aonlybook,title={Local}}\n@incollection{child,crossref={}}";
    workspace
        .documents
        .insert(bib.clone(), Document::new(&bib, source.into(), Some(1)));
    let position = workspace.documents[&bib]
        .text
        .position(source.rfind("{}").unwrap() + 1);
    let items = features::completion(&workspace, &bib, position);
    assert!(items.iter().any(|item| item.label == "aonlybook"));
    assert!(!items.iter().any(|item| item.label == "bonlybook"));
}

#[test]
fn recorded_binary_roles_do_not_block_explicit_source_inclusion() {
    let directory = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let data = shared.path().join("table.tex");
    std::fs::write(&main, "\\documentclass{article}\n").unwrap();
    std::fs::write(&data, "\\label{data-source}\n").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let main_uri = Url::from_file_path(&main).unwrap();
    let data_uri = Url::from_file_path(&data).unwrap();
    workspace.project.recorded_inputs.insert(
        main_uri.clone(),
        [main_uri.clone(), data_uri.clone()].into(),
    );
    workspace
        .project
        .recorded_binary_inputs
        .insert(main_uri.clone(), [data_uri.clone()].into());
    workspace.refresh_project(&main_uri);
    assert!(!workspace.documents.contains_key(&data_uri));
    workspace.reload(&data_uri);
    assert!(!workspace.documents.contains_key(&data_uri));
    std::fs::write(
        &main,
        format!("\\documentclass{{article}}\\input{{{}}}\n", data.display()),
    )
    .unwrap();
    workspace.refresh_project(&main_uri);
    assert_eq!(
        workspace.documents[&data_uri].syntax.labels[0].name,
        "data-source"
    );
}

#[test]
fn directory_renames_preserve_recorded_binary_roles_for_new_source_looking_paths() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old");
    let new = directory.path().join("new");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("main.tex"), "\\documentclass{article}\n").unwrap();
    // The recorder initially admitted this path through a binary alias.
    std::fs::write(old.join("data.payload"), "\\label{not-a-source}\n").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(old.clone());
    let main_uri = Url::from_file_path(old.join("main.tex")).unwrap();
    let data_uri = Url::from_file_path(old.join("data.payload")).unwrap();
    workspace.project.recorded_inputs.insert(
        main_uri.clone(),
        [main_uri.clone(), data_uri.clone()].into(),
    );
    workspace
        .project
        .recorded_binary_inputs
        .insert(main_uri.clone(), [data_uri].into());
    workspace.project.recorded_bindings.insert(
        main_uri,
        [(old.join("alias.csv"), old.join("data.payload"))].into(),
    );
    std::fs::rename(&old, &new).unwrap();
    texquill::editing::file_operations::did_rename(
        &mut workspace,
        &[FileRename {
            old_uri: Url::from_file_path(&old).unwrap().to_string(),
            new_uri: Url::from_file_path(&new).unwrap().to_string(),
        }],
    )
    .unwrap();
    let main_uri = Url::from_file_path(new.join("main.tex")).unwrap();
    let old_data_uri = Url::from_file_path(new.join("data.payload")).unwrap();
    let data_uri = Url::from_file_path(new.join("data.tex")).unwrap();
    std::fs::rename(new.join("data.payload"), new.join("data.tex")).unwrap();
    texquill::editing::file_operations::did_rename(
        &mut workspace,
        &[FileRename {
            old_uri: old_data_uri.to_string(),
            new_uri: data_uri.to_string(),
        }],
    )
    .unwrap();
    assert_eq!(
        workspace.project.recorded_binary_inputs[&main_uri],
        [data_uri.clone()].into()
    );
    assert!(workspace.project.recorded_inputs[&main_uri].contains(&data_uri));
    assert_eq!(
        workspace.project.recorded_bindings[&main_uri],
        [(new.join("alias.csv"), new.join("data.tex"))].into()
    );
    workspace.refresh_project(&main_uri);
    assert!(!workspace.documents.contains_key(&data_uri));
}

#[test]
fn directory_and_file_moves_update_bibliography_binding_names_and_targets() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old");
    let new = directory.path().join("new");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("main.tex"), "\\documentclass{article}\n").unwrap();
    std::fs::write(old.join("physical.bib"), "@book{entry}\n").unwrap();
    let main = Url::from_file_path(old.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.project.bibliography_bindings.insert(
        main,
        [(old.join("refs.bib"), old.join("physical.bib"))].into(),
    );
    std::fs::rename(&old, &new).unwrap();
    texquill::editing::file_operations::did_rename(
        &mut workspace,
        &[FileRename {
            old_uri: Url::from_file_path(&old).unwrap().to_string(),
            new_uri: Url::from_file_path(&new).unwrap().to_string(),
        }],
    )
    .unwrap();
    let target = new.join("renamed.bib");
    std::fs::rename(new.join("physical.bib"), &target).unwrap();
    texquill::editing::file_operations::did_rename(
        &mut workspace,
        &[FileRename {
            old_uri: Url::from_file_path(new.join("physical.bib"))
                .unwrap()
                .to_string(),
            new_uri: Url::from_file_path(&target).unwrap().to_string(),
        }],
    )
    .unwrap();
    assert_eq!(
        workspace.project.bibliography_bindings
            [&Url::from_file_path(new.join("main.tex")).unwrap()],
        [(new.join("refs.bib"), target)].into()
    );
}

#[test]
#[cfg(unix)]
fn static_aliases_preserve_database_style_and_tex_roles_across_reload_open_and_moves() {
    use texquill::project::workspace::DocumentKind;
    for extension in ["payload", "tex", "csv", "png", "bst"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let database = root.join(format!("database.{extension}"));
        let package = root.join("package.bib");
        let style = root.join("style.bib");
        let bibliography = "@book{entry,title={Database}}\n";
        std::fs::write(&database, bibliography).unwrap();
        std::fs::write(&package, "\\newcommand{\\fromalias}{Package}\n").unwrap();
        std::fs::write(
            &style,
            "@book{not-a-database,title={Style text}}\n\\newcommand{\\notatexmacro}{}\n",
        )
        .unwrap();
        let database_alias = root.join("refs.bib");
        std::os::unix::fs::symlink(&database, &database_alias).unwrap();
        std::os::unix::fs::symlink(&package, root.join("alias.sty")).unwrap();
        std::os::unix::fs::symlink(&style, root.join("chosen.bst")).unwrap();
        let source = "\\documentclass{article}\n\\usepackage{alias}\n\\bibliographystyle{chosen}\n\\bibliography{refs}\n\\cite{entry} \\fromalias\n";
        std::fs::write(root.join("main.tex"), source).unwrap();
        let main = uri(root, "main.tex");
        let data_uri = Url::from_file_path(&database).unwrap();
        let package_uri = Url::from_file_path(&package).unwrap();
        let style_uri = Url::from_file_path(&style).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        workspace.refresh_project(&main);
        assert_eq!(
            workspace.documents[&data_uri].kind,
            DocumentKind::Bibtex,
            "{extension}"
        );
        assert_eq!(workspace.documents[&package_uri].kind, DocumentKind::Latex);
        assert_eq!(
            workspace.documents[&style_uri].kind,
            DocumentKind::BibtexStyle
        );
        assert!(workspace.documents[&style_uri].syntax.citations.is_empty());
        assert!(workspace.documents[&style_uri].syntax.commands.is_empty());
        let at = workspace.documents[&main]
            .text
            .position(source.find("cite{entry}").unwrap() + 6);
        assert_eq!(
            features::definitions(&workspace, &main, at)[0].uri,
            data_uri
        );
        assert!(features::completion(&workspace, &main, at)
            .iter()
            .any(|item| item.label == "entry"));
        let rename = features::rename(&workspace, &main, at, "renamed")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(rename.len(), 2);
        assert_eq!(rename[&data_uri][0].new_text, "renamed");
        let macro_at = workspace.documents[&main]
            .text
            .position(source.find("\\fromalias").unwrap() + 3);
        assert_eq!(
            features::definitions(&workspace, &main, macro_at)[0].uri,
            package_uri
        );
        let macro_rename = features::rename(&workspace, &main, macro_at, "renamedalias")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert_eq!(macro_rename.len(), 2);
        assert_eq!(macro_rename[&package_uri][0].new_text, "\\renamedalias");
        for input in [&data_uri, &package_uri, &style_uri] {
            workspace.reload(input);
        }
        assert_eq!(workspace.documents[&data_uri].kind, DocumentKind::Bibtex);
        assert_eq!(workspace.documents[&package_uri].kind, DocumentKind::Latex);
        assert_eq!(
            workspace.documents[&style_uri].kind,
            DocumentKind::BibtexStyle
        );
        // An editor may select a mode from the physical extension. Reopening
        // and refreshing still keep the role confirmed by the source command.
        workspace.open(
            data_uri.clone(),
            bibliography.replace("Database", "Unsaved"),
            3,
        );
        workspace.refresh_project(&main);
        assert_eq!(workspace.documents[&data_uri].kind, DocumentKind::Bibtex);
        assert_eq!(workspace.documents[&data_uri].version, Some(3));
        assert!(workspace.documents[&data_uri]
            .text
            .source
            .contains("Unsaved"));
        let moved = root.join("renamed.tex");
        std::fs::rename(&database, &moved).unwrap();
        texquill::editing::file_operations::did_rename(
            &mut workspace,
            &[FileRename {
                old_uri: data_uri.to_string(),
                new_uri: Url::from_file_path(&moved).unwrap().to_string(),
            }],
        )
        .unwrap();
        let moved_uri = Url::from_file_path(&moved).unwrap();
        assert_eq!(workspace.documents[&moved_uri].kind, DocumentKind::Bibtex);
        assert_eq!(workspace.documents[&moved_uri].version, Some(3));
        assert!(workspace.documents[&moved_uri]
            .text
            .source
            .contains("Unsaved"));
        std::fs::remove_file(database_alias).unwrap();
        std::os::unix::fs::symlink(&moved, root.join("refs.bib")).unwrap();
        workspace.refresh_project(&main);
        assert_eq!(
            features::definitions(&workspace, &main, at)[0].uri,
            moved_uri
        );
    }
}

#[test]
fn ordinary_unreferenced_files_follow_the_new_extension_after_a_move() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("unreferenced.tex");
    let new = directory.path().join("renamed.bib");
    std::fs::write(&old, "@book{entry,title={Title}}\n").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    std::fs::rename(&old, &new).unwrap();
    texquill::editing::file_operations::did_rename(
        &mut workspace,
        &[FileRename {
            old_uri: Url::from_file_path(&old).unwrap().to_string(),
            new_uri: Url::from_file_path(&new).unwrap().to_string(),
        }],
    )
    .unwrap();
    let doc = &workspace.documents[&Url::from_file_path(&new).unwrap()];
    assert_eq!(doc.kind, texquill::project::workspace::DocumentKind::Bibtex);
    assert_eq!(doc.syntax.citations[0].name, "entry");
}

#[test]
#[cfg(unix)]
fn current_static_roles_replace_previous_recorder_roles_without_losing_unsaved_text() {
    use texquill::project::workspace::DocumentKind;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let target = root.join("physical.payload");
    let main = uri(root, "main.tex");
    let target_uri = Url::from_file_path(&target).unwrap();
    std::fs::write(&target, "\\newcommand{\\fromalias}{Text}\n").unwrap();
    for alias in ["alias.sty", "refs.bib", "chosen.bst"] {
        std::os::unix::fs::symlink(&target, root.join(alias)).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace.open(
        main.clone(),
        "\\documentclass{article}\\usepackage{alias}\\fromalias\n".into(),
        1,
    );
    workspace.project.recorded_bindings.insert(
        main.clone(),
        [(root.join("alias.sty"), target.clone())].into(),
    );
    workspace
        .project
        .recorded_inputs
        .insert(main.clone(), [target_uri.clone()].into());
    let bibliography = "@book{entry,title={Book}}\n";
    std::fs::write(&target, bibliography).unwrap();
    workspace.open(
        main.clone(),
        "\\documentclass{article}\\bibliography{refs}\\cite{entry}\n".into(),
        2,
    );
    workspace.refresh_project(&main);
    assert_eq!(workspace.documents[&target_uri].kind, DocumentKind::Bibtex);
    assert_eq!(
        workspace.documents[&target_uri].syntax.citations[0].name,
        "entry"
    );
    workspace.open(
        target_uri.clone(),
        bibliography.replace("Book", "Unsaved"),
        1,
    );
    workspace.reload(&target_uri);
    assert_eq!(workspace.documents[&target_uri].kind, DocumentKind::Bibtex);
    assert!(workspace.documents[&target_uri]
        .text
        .source
        .contains("Unsaved"));
    workspace.open(
        main.clone(),
        "\\documentclass{article}\\bibliographystyle{chosen}\n".into(),
        3,
    );
    workspace.refresh_project(&main);
    assert_eq!(
        workspace.documents[&target_uri].kind,
        DocumentKind::BibtexStyle
    );
    assert_eq!(workspace.documents[&target_uri].version, Some(1));
    assert!(workspace.documents[&target_uri]
        .text
        .source
        .contains("Unsaved"));
    assert!(workspace.documents[&target_uri].syntax.citations.is_empty());
    workspace.open(
        main.clone(),
        "\\documentclass{article}\\usepackage{alias}\n".into(),
        4,
    );
    workspace.refresh_project(&main);
    assert_eq!(workspace.documents[&target_uri].kind, DocumentKind::Latex);
    assert_eq!(workspace.documents[&target_uri].version, Some(1));
    assert!(workspace.documents[&target_uri]
        .text
        .source
        .contains("Unsaved"));
}
