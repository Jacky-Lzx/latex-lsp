use latex_lsp::{
    features, signature,
    workspace::{Document, Workspace},
};
use std::path::Path;
use tower_lsp::lsp_types::*;

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
    let inputs = latex_lsp::recorder::inputs(
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
        assert!(value.contains("/a/"), "{value}");
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
    workspace.snippets = true;
    let prefix =
        "\\documentclass{article}\\usepackage{macros}\\input{chapter}\\bibliography{refs}\n";
    for (suffix, present, absent) in [
        (r"\ref{", "a:only", "b:only"),
        (r"\cite{", "aonlybook", "bonlybook"),
        (r"\begin{", "aonlyenvironment", "bonlyenvironment"),
        (r"\", "aonlycommand", "bonlycommand"),
    ] {
        let source = format!("{prefix}{suffix}");
        let position = latex_lsp::text::Text::new(source.clone()).position(source.len());
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
    let position = latex_lsp::text::Text::new(source.clone()).position(source.len());
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
    workspace.recorded_inputs.insert(
        main_uri.clone(),
        [main_uri.clone(), data_uri.clone()].into(),
    );
    workspace
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
    workspace.recorded_inputs.insert(
        main_uri.clone(),
        [main_uri.clone(), data_uri.clone()].into(),
    );
    workspace
        .recorded_binary_inputs
        .insert(main_uri, [data_uri].into());
    std::fs::rename(&old, &new).unwrap();
    latex_lsp::file_operations::did_rename(
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
    latex_lsp::file_operations::did_rename(
        &mut workspace,
        &[FileRename {
            old_uri: old_data_uri.to_string(),
            new_uri: data_uri.to_string(),
        }],
    )
    .unwrap();
    assert_eq!(
        workspace.recorded_binary_inputs[&main_uri],
        [data_uri.clone()].into()
    );
    assert!(workspace.recorded_inputs[&main_uri].contains(&data_uri));
    workspace.refresh_project(&main_uri);
    assert!(!workspace.documents.contains_key(&data_uri));
}
