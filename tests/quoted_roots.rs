use lsp_types::Url;
use std::path::Path;
use texquill::{
    language::{code_lens, features},
    project::workspace::{Document, Workspace},
};

fn uri(root: &Path, file: &str) -> Url {
    Url::from_file_path(root.join(file)).unwrap()
}
fn project(root: &Path, example: &str) -> Workspace {
    for (name, text) in [
        (
            "main.tex",
            r"\documentclass{article}\begin{document}\input{shared}\end{document}",
        ),
        ("example.tex", example),
        (
            "shared.tex",
            r"Shared. \ifdefined\examplemacro\examplemacro\fi",
        ),
        ("provider.tex", r"\def\meaning#1{#1}"),
    ] {
        std::fs::write(root.join(name), text).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

#[test]
fn quoted_document_markers_do_not_create_roots_or_shared_build_lenses() {
    for marker in [
        r"\documentclass{article}",
        "\\documentclass{article% name 😀\r\n}",
        r"\begin{document}",
    ] {
        for statement in [
            format!("\\meaning{marker}"),
            format!("\\string{marker}"),
            format!("\\detokenize{{{marker}}}"),
            format!("\\csname meaning\\endcsname{marker}"),
            format!("\\csuse{{meaning}}{marker}"),
            format!("\\newcommand{{\\future}}{{\\meaning{marker}}}\\future"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let workspace = project(
                root,
                &format!("\\newcommand{{\\examplemacro}}{{Fake}}{statement}\\input{{shared}}"),
            );
            let shared = uri(root, "shared.tex");
            let main = uri(root, "main.tex");
            assert_eq!(
                workspace.root_document(&shared).unwrap(),
                main,
                "{statement}"
            );
            let lenses = code_lens::lenses(&workspace, &shared);
            assert_eq!(lenses.len(), 1, "{statement}: {lenses:?}");
            assert_eq!(lenses[0].command.as_ref().unwrap().title, "Build main.tex");
            let doc = &workspace.documents[&shared];
            let at = doc
                .text
                .position(doc.text.source.rfind("examplemacro").unwrap() + 3);
            assert!(
                features::definitions(&workspace, &shared, at).is_empty(),
                "{statement}"
            );
            assert!(
                code_lens::lenses(&workspace, &uri(root, "example.tex")).is_empty(),
                "{statement}"
            );
        }
    }
}

#[test]
fn declaration_targets_and_copy_sources_do_not_become_document_roots() {
    for statement in [
        r"\let\saved\begin{document}",
        r"\let\saved\documentclass{article}",
        r"\def\documentclass#1{}",
        r"\let\documentclass\relax",
        r"\futurelet\documentclass\relax\relax",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let workspace = project(root, &format!(r"{statement}\input{{shared}}"));
        let shared = uri(root, "shared.tex");
        assert_eq!(
            workspace.root_document(&shared).unwrap(),
            uri(root, "main.tex"),
            "{statement}"
        );
        assert_eq!(
            code_lens::lenses(&workspace, &shared).len(),
            1,
            "{statement}"
        );
    }
}

#[test]
fn actual_and_uncertain_root_markers_keep_shared_root_ambiguity() {
    for marker in [r"\documentclass{article}", r"\begin{document}"] {
        for statement in [
            marker.to_string(),
            format!("\\newcommand{{\\future}}{{{marker}}}\\future"),
            format!("\\def\\meaning#1{{#1}}\\meaning{marker}"),
            format!("\\input{{provider}}\\meaning{marker}"),
            format!("\\def\\picked{{provider}}\\input{{\\picked}}\\meaning{marker}"),
            format!("\\newcommand{{\\execute}}[1]{{#1}}\\execute{{\\input{{provider}}}}\\meaning{marker}"),
            format!("\\csname @input\\endcsname{{provider.tex}}\\meaning{marker}"),
            format!("\\scantokens{{\\input{{provider}}}}\\meaning{marker}"),
        ] {
            let directory=tempfile::tempdir().unwrap();let root=directory.path();
            let workspace=project(root,&format!("{statement}\\input{{shared}}"));let shared=uri(root,"shared.tex");
            assert!(workspace.root_document(&shared).unwrap_err().contains("Multiple root documents"),"{statement}");
            assert_eq!(code_lens::lenses(&workspace,&shared).len(),2,"{statement}");
        }
    }
}

#[test]
fn explicit_roots_magic_comments_and_live_buffer_changes_keep_their_priority() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root, r"\meaning\documentclass{article}\input{shared}");
    let shared = uri(root, "shared.tex");
    let example = uri(root, "example.tex");
    let main = uri(root, "main.tex");
    assert_eq!(workspace.root_document(&shared).unwrap(), main);
    workspace.config.build.root_file = Some("example.tex".into());
    assert_eq!(workspace.root_document(&shared).unwrap(), example);
    assert_eq!(
        code_lens::lenses(&workspace, &shared)[0]
            .command
            .as_ref()
            .unwrap()
            .title,
        "Build example.tex"
    );
    workspace.config.build.root_file = None;
    workspace.documents.insert(
        shared.clone(),
        Document::new(
            &shared,
            "% !TeX root = example.tex\nShared.".into(),
            Some(8),
        ),
    );
    assert_eq!(workspace.root_document(&shared).unwrap(), example);
    workspace.documents.insert(
        shared.clone(),
        Document::new(&shared, "Shared.".into(), Some(9)),
    );
    workspace.documents.insert(
        example.clone(),
        Document::new(
            &example,
            r"\documentclass{article}\input{shared}".into(),
            Some(8),
        ),
    );
    assert!(workspace
        .root_document(&shared)
        .unwrap_err()
        .contains("Multiple root documents"));
    workspace.documents.insert(
        example.clone(),
        Document::new(
            &example,
            r"\meaning\documentclass{article}\input{shared}".into(),
            Some(9),
        ),
    );
    assert_eq!(workspace.root_document(&shared).unwrap(), main);
}
