use latex_lsp::{
    build, features, file_operations, navigation,
    text::Text,
    workspace::{Document, Workspace},
};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn project(root: &Path) -> Workspace {
    std::fs::create_dir(root.join("sections")).unwrap();
    for (name, source) in [
        ("main.tex", "\\documentclass{article}\n\\begin{document}\n\\input{sections/child}\n\\end{document}\n"),
        ("sections/child.tex", "% !TeX root = ../main.tex\n\\input{same}\n"),
        ("same.tex", "\\typeout{SELECTED ROOT FILE}Root file."),
        ("sections/same.tex", "\\typeout{SELECTED CHILD FILE}Child file."),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

fn changed(workspace: &Workspace, files: &[FileRename]) -> BTreeMap<Url, String> {
    file_operations::will_rename(workspace, files)
        .unwrap()
        .and_then(|edit| edit.changes)
        .unwrap_or_default()
        .into_iter()
        .map(|(uri, mut edits)| {
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
fn dependencies_links_and_completion_prefer_the_compilers_working_directory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let main = uri(root, "main.tex");
    let child = uri(root, "sections/child.tex");
    let files = workspace.project_documents(&main);
    assert!(files.contains(&uri(root, "same.tex")));
    assert!(!files.contains(&uri(root, "sections/same.tex")));
    let links = navigation::document_links(&workspace, &child);
    assert_eq!(links[0].target, Some(main));
    assert_eq!(links[1].target, Some(uri(root, "same.tex")));

    let source = "\\input{sa";
    workspace
        .documents
        .insert(child.clone(), Document::new(&child, source.into(), Some(1)));
    let completions = features::completion(
        &workspace,
        &child,
        Text::new(source.into()).position(source.len()),
    );
    let matches: Vec<_> = completions
        .iter()
        .filter(|item| item.label == "same.tex")
        .collect();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].detail.as_deref(), root.join("same.tex").to_str());
}

#[test]
fn include_appends_tex_instead_of_resolving_existing_files_with_other_or_missing_extensions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (name,source) in [
        ("main.tex", "\\documentclass{article}\\input{chapter.ltx}\\include{chapter.ltx}\\include{chapter}\\includeonly{chapter.ltx, chapter}"),
        ("chapter.ltx", "\\typeout{INPUT LTX SOURCE}Ltx source."),
        ("chapter.ltx.tex", "\\typeout{INCLUDE LTX TEX SOURCE}Ltx Tex source."),
        ("chapter.tex", "\\typeout{INCLUDE TEX SOURCE}Tex source."),
        ("chapter", "\\typeout{WRONG EXTENSIONLESS SOURCE}Wrong source."),
    ] { std::fs::write(root.join(name),source).unwrap(); }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    assert_eq!(
        workspace.project_documents(&main),
        [
            main.clone(),
            uri(root, "chapter.ltx.tex"),
            uri(root, "chapter.tex")
        ]
        .into()
    );
    let links = navigation::document_links(&workspace, &main);
    assert_eq!(
        links
            .iter()
            .map(|link| link.target.clone().unwrap())
            .collect::<Vec<_>>(),
        vec![
            uri(root, "chapter.ltx.tex"),
            uri(root, "chapter.ltx.tex"),
            uri(root, "chapter.tex"),
            uri(root, "chapter.ltx.tex"),
            uri(root, "chapter.tex")
        ]
    );
    std::fs::remove_file(root.join("chapter.ltx.tex")).unwrap();
    let links = navigation::document_links(&workspace, &main);
    assert_eq!(links.len(), 3);
    assert_eq!(links[0].target, Some(uri(root, "chapter.ltx")));
}

#[test]
#[ignore = "requires latexmk"]
fn actual_include_filename_selection_matches_explicit_tex_suffix_resolution() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (name,source) in [
        ("main.tex", "\\documentclass{article}\\begin{document}\\input{chapter.ltx}\\include{chapter.ltx}\\include{chapter}\\end{document}"),
        ("chapter.ltx", "\\typeout{INPUT LTX SOURCE}Ltx source."),
        ("chapter.ltx.tex", "\\typeout{INCLUDE LTX TEX SOURCE}Ltx Tex source."),
        ("chapter.tex", "\\typeout{INCLUDE TEX SOURCE}Tex source."),
        ("chapter", "\\typeout{WRONG EXTENSIONLESS SOURCE}Wrong source."),
    ] { std::fs::write(root.join(name),source).unwrap(); }
    let outcome = build::run(
        &root.join("main.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    for marker in ["INCLUDE LTX TEX SOURCE", "INCLUDE TEX SOURCE"] {
        assert!(
            outcome.output.contains(marker),
            "{marker}: {}",
            outcome.output
        );
    }
    assert!(!outcome.output.contains("WRONG EXTENSIONLESS SOURCE"));
    assert!(!outcome.output.contains("INPUT LTX SOURCE"));
}

#[test]
fn renames_update_the_selected_file_and_preserve_root_relative_child_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    let child = uri(root, "sections/child.tex");
    assert!(
        changed(&workspace, &[rename(root, "same.tex", "selected.tex")])[&child]
            .contains("\\input{selected}")
    );
    assert!(changed(
        &workspace,
        &[rename(root, "sections/same.tex", "sections/hidden.tex")]
    )
    .is_empty());

    let edits = changed(&workspace, &[rename(root, "sections", "parts/content")]);
    assert!(edits[&child].contains("% !TeX root = ../../main.tex"));
    assert!(edits[&child].contains("\\input{same}"));
    assert!(edits[&uri(root, "main.tex")].contains("\\input{parts/content/child}"));
}

#[test]
fn missing_root_candidates_still_allow_document_relative_fallbacks() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    std::fs::remove_file(root.join("same.tex")).unwrap();
    let child = uri(root, "sections/child.tex");
    assert_eq!(
        navigation::document_links(&workspace, &child)[1].target,
        Some(uri(root, "sections/same.tex"))
    );
    let edits = changed(&workspace, &[rename(root, "sections", "parts/content")]);
    assert!(edits[&child].contains("\\input{same}"));
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn actual_compiler_selection_matches_links_before_and_after_file_moves() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    for renamed in [false, true] {
        let main = root.join("main.tex");
        let outcome = build::run(&main, &Default::default(), &Default::default());
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        assert!(outcome.output.contains("SELECTED ROOT FILE"));
        assert!(!outcome.output.contains("SELECTED CHILD FILE"));
        let inputs = outcome.inputs.unwrap();
        let selected = uri(root, if renamed { "selected.tex" } else { "same.tex" });
        assert!(inputs.contains(&selected));
        assert!(!inputs.contains(&uri(root, "sections/same.tex")));
        let child = uri(
            root,
            if renamed {
                "parts/content/child.tex"
            } else {
                "sections/child.tex"
            },
        );
        assert_eq!(
            navigation::document_links(&workspace, &child)[1].target,
            Some(selected)
        );
        if renamed {
            break;
        }
        let files = [
            rename(root, "same.tex", "selected.tex"),
            rename(root, "sections", "parts/content"),
        ];
        for (uri, source) in changed(&workspace, &files) {
            std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
        }
        std::fs::create_dir(root.join("parts")).unwrap();
        std::fs::rename(root.join("same.tex"), root.join("selected.tex")).unwrap();
        std::fs::rename(root.join("sections"), root.join("parts/content")).unwrap();
        file_operations::did_rename(&mut workspace, &files).unwrap();
    }
}

#[test]
fn input_renames_reject_implicit_tex_suffixes_that_would_select_another_source() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    std::fs::write(root.join("renamed.ltx.tex"), "Other source.").unwrap();
    let error =
        file_operations::will_rename(&workspace, &[rename(root, "same.tex", "renamed.ltx")])
            .unwrap_err();
    assert!(error.message.contains("different file"), "{error}");
}

#[test]
fn input_rename_checks_consider_files_arriving_and_leaving_in_the_same_batch() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    std::fs::write(root.join("renamed.ltx.tex"), "Shadow.").unwrap();
    let child = uri(root, "sections/child.tex");
    let edits = changed(
        &workspace,
        &[
            rename(root, "same.tex", "renamed.ltx"),
            rename(root, "renamed.ltx.tex", "other.tex"),
        ],
    );
    assert!(edits[&child].contains(r"\input{renamed.ltx}"));
    let error = file_operations::will_rename(
        &workspace,
        &[
            rename(root, "same.tex", "new.ltx"),
            rename(root, "renamed.ltx.tex", "new.ltx.tex"),
        ],
    )
    .unwrap_err();
    assert!(error.message.contains("different file"), "{error}");
}

#[test]
fn unrelated_file_moves_cannot_redirect_an_unchanged_input() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let child = uri(root, "sections/child.tex");
    std::fs::write(root.join("literal.ltx"), "Expected source.").unwrap();
    std::fs::write(root.join("other.tex"), "Other source.").unwrap();
    workspace.open(child.clone(), r"\input{literal.ltx}".into(), 3);
    assert_eq!(
        navigation::document_links(&workspace, &child)[0].target,
        Some(uri(root, "literal.ltx"))
    );
    let error =
        file_operations::will_rename(&workspace, &[rename(root, "other.tex", "literal.ltx.tex")])
            .unwrap_err();
    assert!(error.message.contains("different file"), "{error}");
}

#[test]
fn incoming_root_inputs_preserve_document_relative_fallbacks_with_explicit_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    std::fs::remove_file(root.join("same.tex")).unwrap();
    std::fs::write(root.join("other.tex"), "Other source.").unwrap();
    let edits = changed(&workspace, &[rename(root, "other.tex", "same.tex")]);
    assert!(edits[&uri(root, "sections/child.tex")].contains(r"\input{sections/same.tex}"));
}

#[test]
fn input_shadow_validation_preserves_valid_bare_quoted_and_unicode_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let child = uri(root, "sections/child.tex");
    workspace.open(
        child.clone(),
        "\\input same\n\\input\"same\"\n\\input{same}\n".into(),
        4,
    );
    let edits = changed(&workspace, &[rename(root, "same.tex", "新 章节.ltx")]);
    assert_eq!(
        edits[&child],
        "\\input {新 章节.ltx}\n\\input\"新 章节.ltx\"\n\\input{新 章节.ltx}\n"
    );
    std::fs::write(root.join("新 章节.ltx.tex"), "Shadow.").unwrap();
    assert!(
        file_operations::will_rename(&workspace, &[rename(root, "same.tex", "新 章节.ltx")],)
            .is_err()
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn source_extension_rename_rejects_wrong_compiler_inputs_and_allows_a_batch_removing_the_shadow() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    std::fs::write(root.join("renamed.ltx.tex"), r"\typeout{WRONG INPUT}Other.").unwrap();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let before = build::run(&root.join("main.tex"), &config, &Default::default());
    assert_eq!(before.status, "success", "{}", before.output);
    let dangerous = [rename(root, "same.tex", "renamed.ltx")];
    assert!(file_operations::will_rename(&workspace, &dangerous).is_err());
    let safe = [
        dangerous[0].clone(),
        rename(root, "renamed.ltx.tex", "other.tex"),
    ];
    for (uri, source) in changed(&workspace, &safe) {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    std::fs::rename(root.join("same.tex"), root.join("renamed.ltx")).unwrap();
    std::fs::rename(root.join("renamed.ltx.tex"), root.join("other.tex")).unwrap();
    file_operations::did_rename(&mut workspace, &safe).unwrap();
    let after = build::run(&root.join("main.tex"), &config, &Default::default());
    assert_eq!(after.status, "success", "{}", after.output);
    assert!(
        after.output.contains("SELECTED ROOT FILE"),
        "{}",
        after.output
    );
    assert!(!after.output.contains("WRONG INPUT"), "{}", after.output);
    assert!(after.inputs.unwrap().contains(&uri(root, "renamed.ltx")));
}
