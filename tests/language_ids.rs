use latex_lsp::{
    editing::file_operations,
    language::features,
    project::workspace::{DocumentKind, Workspace},
    text::Text,
};
use lsp_types::*;

#[test]
fn supporting_sources_are_discovered_and_explicit_inputs_keep_project_scope_and_versioned_moves() {
    for extension in ["bbx", "cbx", "lbx", "dbx", "fd", "ldf"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let uri = |name: &str| Url::from_file_path(root.join(name)).unwrap();
        let local_name = format!("local.{extension}");
        let other_name = format!("other.{extension}");
        let unused_name = format!("unused.{extension}");
        let source = format!("\\documentclass{{article}}\n\\input{{{local_name}}}\n\\shared");
        for (name, text) in [
            ("a.tex", source.as_str()),
            (
                "b.tex",
                &format!("\\documentclass{{article}}\\input{{{other_name}}}\\shared"),
            ),
            (local_name.as_str(), "\\newcommand\\shared[1]{#1}"),
            (other_name.as_str(), "\\newcommand\\shared[2]{#1#2}"),
            (unused_name.as_str(), "\\newcommand\\unusedstyle{}"),
        ] {
            std::fs::write(root.join(name), text).unwrap();
        }
        let mut workspace = Workspace {
            versioned_edits: true,
            snippets: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        assert_eq!(
            workspace.documents[&uri(&unused_name)].kind,
            DocumentKind::Latex
        );
        let symbols = latex_lsp::language::navigation::workspace_symbols(&workspace, "unusedstyle");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].location.uri, uri(&unused_name));
        let main = uri("a.tex");
        workspace.open(main.clone(), source.clone(), 8);
        let text = Text::new(source.clone());
        let position = text.position(source.len() - 2);
        let definitions = features::definitions(&workspace, &main, position);
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].uri, uri(&local_name));
        let item = features::completion(&workspace, &main, text.position(source.len()))
            .into_iter()
            .find(|item| item.label == "shared")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!()
        };
        assert_eq!(edit.new_text, "shared{${1:arg1}}");
        assert_eq!(
            latex_lsp::language::signature::help(&workspace, &main, text.position(source.len()))
                .unwrap()
                .signatures[0]
                .label,
            "\\shared{#1}"
        );
        let new_name = format!("renamed.{extension}");
        let edit = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(&local_name).into(),
                new_uri: uri(&new_name).into(),
            }],
        )
        .unwrap()
        .unwrap();
        let Some(DocumentChanges::Edits(edits)) = edit.document_changes else {
            panic!()
        };
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].text_document.uri, main);
        assert_eq!(edits[0].text_document.version, Some(8));
        assert_eq!(edits[0].edits.len(), 1);
        let OneOf::Left(edit) = &edits[0].edits[0] else {
            panic!()
        };
        assert_eq!(edit.new_text, new_name);
        assert_eq!(
            &source[text.offset(edit.range.start).unwrap()..text.offset(edit.range.end).unwrap()],
            local_name
        );
    }
}

#[test]
#[ignore = "requires Neovim 0.11+"]
fn neovim_bib_filetype_enables_unsaved_nonstandard_files_and_incremental_edits() {
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("language.lua");
    std::fs::write(&script, include_str!("fixtures/neovim_language.lua")).unwrap();
    let output = std::process::Command::new("nvim")
        .args(["--headless", "-u", "NONE", "-i", "NONE", "-n", "-l"])
        .arg(script)
        .env("LATEX_LSP_TEST_BIN", env!("CARGO_BIN_EXE_latex-lsp"))
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .env("XDG_STATE_HOME", directory.path().join("state"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn editor_languages_select_parsers_for_untitled_and_misleading_names() {
    let source = "@book{entry,title={Text}}\n\\label{label}\n";
    for (language_id, kind) in [
        ("latex", DocumentKind::Latex),
        ("tex", DocumentKind::Latex),
        ("plaintex", DocumentKind::Latex),
        ("bibtex", DocumentKind::Bibtex),
        ("bib", DocumentKind::Bibtex),
        ("bst", DocumentKind::BibtexStyle),
        ("bibtex-style", DocumentKind::BibtexStyle),
    ] {
        for name in [
            "untitled:Untitled-1",
            "untitled:wrong.bib",
            "untitled:wrong.bst",
        ] {
            let uri = Url::parse(name).unwrap();
            let mut workspace = Workspace::default();
            workspace.open_with_language(uri.clone(), source.into(), 1, language_id);
            let doc = &workspace.documents[&uri];
            assert_eq!(doc.kind, kind, "{name}/{language_id}");
            assert_eq!(doc.kind_hint, None, "editor identity is not a project role");
            let symbols = features::symbols(doc);
            let names: Vec<_> = symbols.iter().map(|symbol| symbol.name.as_str()).collect();
            assert_eq!(
                names,
                match kind {
                    DocumentKind::Latex => vec!["label"],
                    DocumentKind::Bibtex => vec!["entry"],
                    DocumentKind::BibtexStyle => vec![],
                },
                "{name}/{language_id}"
            );
            assert!(workspace.root_document(&uri).is_err());
        }
    }
}

#[test]
fn unknown_language_ids_retain_suffix_inference() {
    for language_id in ["", "plaintext", "unknown", "BIBTEX"] {
        for (name, kind) in [
            ("untitled:refs.bib", DocumentKind::Bibtex),
            ("untitled:style.bst", DocumentKind::BibtexStyle),
            ("untitled:main.tex", DocumentKind::Latex),
        ] {
            let uri = Url::parse(name).unwrap();
            let mut workspace = Workspace::default();
            workspace.open_with_language(uri.clone(), "".into(), 1, language_id);
            assert_eq!(workspace.documents[&uri].kind, kind);
        }
    }
}

#[test]
fn dependencies_override_editor_languages_before_and_after_opening() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let uri = |name| Url::from_file_path(root.join(name)).unwrap();
    let main = uri("main.tex");
    let database = uri("refs.data");
    let chapter = uri("chapter.bib");
    let source = "\\documentclass{article}\\addbibresource{refs.data}\\input{chapter.bib}";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(database.to_file_path().unwrap(), "@book{saved,}").unwrap();
    std::fs::write(chapter.to_file_path().unwrap(), r"\label{saved}").unwrap();
    for open_first in [false, true] {
        let mut workspace = Workspace::default();
        // An explicit root avoids discovery caused by opening the first file.
        workspace.roots.insert(root.into());
        if !open_first {
            workspace.open(main.clone(), source.into(), 1);
        }
        workspace.open_with_language(database.clone(), "@book{unsaved,}".into(), 7, "latex");
        workspace.open_with_language(chapter.clone(), r"\label{unsaved}".into(), 9, "bib");
        if open_first {
            assert_eq!(workspace.documents[&database].kind, DocumentKind::Latex);
            assert_eq!(workspace.documents[&chapter].kind, DocumentKind::Bibtex);
            workspace.open(main.clone(), source.into(), 1);
        }
        assert_eq!(workspace.documents[&database].kind, DocumentKind::Bibtex);
        assert_eq!(workspace.documents[&chapter].kind, DocumentKind::Latex);
        for (target, version) in [(&database, 7), (&chapter, 9)] {
            let doc = &workspace.documents[target];
            assert_eq!(doc.version, Some(version));
            assert_eq!(features::symbols(doc)[0].name, "unsaved");
        }
    }
}

#[test]
fn moving_open_documents_retains_client_language_but_closed_documents_use_new_suffix() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let old = Url::from_file_path(root.join("refs.data")).unwrap();
    let new = Url::from_file_path(root.join("refs.tex")).unwrap();
    let source = "@book{entry,title={Text}}\n\\label{label}";
    std::fs::write(old.to_file_path().unwrap(), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.open_with_language(old.clone(), source.into(), 7, "bib");
    std::fs::rename(old.to_file_path().unwrap(), new.to_file_path().unwrap()).unwrap();
    file_operations::did_rename(
        &mut workspace,
        &[FileRename {
            old_uri: old.to_string(),
            new_uri: new.to_string(),
        }],
    )
    .unwrap();
    assert!(!workspace.documents.contains_key(&old));
    assert_eq!(workspace.documents[&new].kind, DocumentKind::Bibtex);
    assert_eq!(workspace.documents[&new].version, Some(7));
    assert_eq!(
        features::symbols(&workspace.documents[&new])[0].name,
        "entry"
    );

    let old = Url::from_file_path(root.join("closed.bib")).unwrap();
    let new = Url::from_file_path(root.join("closed.tex")).unwrap();
    std::fs::write(old.to_file_path().unwrap(), source).unwrap();
    workspace.reload(&old);
    assert_eq!(workspace.documents[&old].kind, DocumentKind::Bibtex);
    std::fs::rename(old.to_file_path().unwrap(), new.to_file_path().unwrap()).unwrap();
    file_operations::did_rename(
        &mut workspace,
        &[FileRename {
            old_uri: old.to_string(),
            new_uri: new.to_string(),
        }],
    )
    .unwrap();
    assert_eq!(workspace.documents[&new].kind, DocumentKind::Latex);
    assert_eq!(workspace.documents[&new].version, None);
    assert_eq!(
        features::symbols(&workspace.documents[&new])[0].name,
        "label"
    );
}
