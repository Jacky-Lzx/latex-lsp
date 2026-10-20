use latex_lsp::{features, file_operations, navigation, text::Text, workspace::Workspace};
use std::path::Path;
use tower_lsp::lsp_types::*;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
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

fn complete(workspace: &mut Workspace, main: &Url, marked: &str, label: &str) -> String {
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    workspace.open(main.clone(), source.clone(), 3);
    let items = features::completion(workspace, main, Text::new(source.clone()).position(offset));
    let item = items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("{marked}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    let start = Text::new(source.clone()).offset(edit.range.start).unwrap();
    assert!(item
        .filter_text
        .as_ref()
        .unwrap()
        .starts_with(&source[start..offset]));
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    apply(&source, edits)
}

#[test]
fn completion_and_navigation_preserve_arbitrary_file_names_without_importing_symbols() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    std::fs::create_dir(root.join("code,one")).unwrap();
    for name in [
        "sample.rs",
        "sample.unknown",
        "sample",
        "sample.tex",
        "code,one/样例, one.py",
    ] {
        std::fs::write(root.join(name), r"\label{code-only}").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (typed, name) in [
        ("sam¦ple.rs", "sample.rs"),
        ("sam¦ple.unknown", "sample.unknown"),
        ("sam¦ple.tex", "sample.tex"),
        ("code,one/样例, o¦ld.py", "code,one/样例, one.py"),
    ] {
        let marked = format!("% 😀\r\n\\lstinputlisting[language=Python,title={{A,B [ C]}}] % keep\r\n{{{typed}}}{{body, untouched}}");
        let completed = complete(&mut workspace, &main, &marked, name);
        assert_eq!(completed, marked.replace(typed, name).replace('¦', ""));
        workspace.open(main.clone(), completed.clone(), 4);
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1, "{completed}: {links:?}");
        assert_eq!(links[0].target, Some(uri(root, name)));
        assert_eq!(
            features::definitions(&workspace, &main, links[0].range.start)[0].uri,
            uri(root, name)
        );
        let document = &workspace.documents[&main];
        assert!(!workspace
            .project_documents(&main)
            .contains(&uri(root, name)));
        assert!(features::symbols(document)
            .iter()
            .all(|entry| entry.name != "code-only"));
    }
    // Without a .tex competitor, extensionless files also complete exactly.
    std::fs::write(root.join("README"), "plain code").unwrap();
    assert_eq!(
        complete(&mut workspace, &main, r"\lstinputlisting{REA¦}", "README"),
        r"\lstinputlisting{README}"
    );
}

#[test]
fn tex_suffix_shadowing_is_shared_by_completion_navigation_and_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    for name in ["main.tex", "sample.rs", "sample.rs.tex", "README."] {
        std::fs::write(root.join(name), "code").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace.open(main.clone(), r"\lstinputlisting{sample.rs}".into(), 2);
    assert_eq!(
        navigation::document_links(&workspace, &main)[0].target,
        Some(uri(root, "sample.rs.tex"))
    );
    let edits = file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: uri(root, "sample.rs.tex").into(),
            new_uri: uri(root, "renamed.payload").into(),
        }],
    )
    .unwrap()
    .unwrap();
    let changed = apply(
        r"\lstinputlisting{sample.rs}",
        edits.changes.unwrap()[&main].clone(),
    );
    assert_eq!(changed, r"\lstinputlisting{renamed.payload}");
    workspace.open(main.clone(), r"\lstinputlisting{sam}".into(), 3);
    let position = Text::new(r"\lstinputlisting{sam}".into()).position(20);
    let items = features::completion(&workspace, &main, position);
    assert!(
        !items.iter().any(|item| item.label == "sample.rs"),
        "{items:?}"
    );
    workspace.open(main.clone(), r"\lstinputlisting{README}".into(), 4);
    assert!(
        navigation::document_links(&workspace, &main).is_empty(),
        "an exact lookup must not append a dot"
    );
}

#[test]
fn file_and_directory_moves_produce_versioned_edits_and_keep_optional_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir_all(root.join("chapters")).unwrap();
    std::fs::create_dir(root.join("code")).unwrap();
    std::fs::create_dir_all(root.join("chapters/code")).unwrap();
    std::fs::write(
        root.join("main.tex"),
        r"\documentclass{article}\input{chapters/child}",
    )
    .unwrap();
    std::fs::write(root.join("code/sample.rs"), "expected").unwrap();
    std::fs::write(root.join("chapters/code/sample.rs"), "decoy").unwrap();
    let child = uri(root, "chapters/child.tex");
    let source = "% 😀\r\n\\lstinputlisting[language=Rust] % keep\r\n{code/sample.rs}\r\n";
    std::fs::write(child.to_file_path().unwrap(), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace.open(child.clone(), source.into(), 7);
    workspace.versioned_edits = true;
    assert_eq!(
        navigation::document_links(&workspace, &child)[0].target,
        Some(uri(root, "code/sample.rs"))
    );
    for (old, new, expected) in [
        ("code/sample.rs", "code/renamed.py", "code/renamed.py"),
        ("code", "new, dir", "new, dir/sample.rs"),
    ] {
        let changes = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(root, old).into(),
                new_uri: uri(root, new).into(),
            }],
        )
        .unwrap()
        .unwrap();
        let Some(DocumentChanges::Edits(documents)) = changes.document_changes else {
            panic!()
        };
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].text_document.version, Some(7));
        let edits = documents[0]
            .edits
            .iter()
            .map(|edit| match edit {
                OneOf::Left(edit) => edit.clone(),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            apply(source, edits),
            source.replace("code/sample.rs", expected)
        );
    }
    std::fs::remove_file(root.join("code/sample.rs")).unwrap();
    assert!(
        navigation::document_links(&workspace, &child).is_empty(),
        "listings does not fall back to the child directory"
    );
}

#[test]
fn invalid_call_shapes_and_dynamic_search_settings_do_not_produce_path_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    std::fs::write(root.join("sample.rs"), "code").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for marked in [
        r"\lstinputlisting*{sam¦ple.rs}",
        r"\lstinputlisting[a][b]{sam¦ple.rs}",
        r"\lstinputlisting[title=[nested]]{sam¦ple.rs}",
        r"\lstinputlisting[inputpath=other]{sam¦ple.rs}",
        r"\lstinputlisting[style=custom]{sam¦ple.rs}",
        r"\lstinputlisting[unknown=value]{sam¦ple.rs}",
        r"\lstinputlisting[title=\custom]{sam¦ple.rs}",
        r"\lstset{inputpath=other}\lstinputlisting{sam¦ple.rs}",
        r"\lstset{style=custom}\lstinputlisting{sam¦ple.rs}",
        r"\renewcommand{\lstinputlisting}[1]{}\lstinputlisting{sam¦ple.rs}",
        r"\lstinputlisting{\macro/sam¦ple.rs}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 3);
        assert!(
            navigation::document_links(&workspace, &main).is_empty(),
            "{marked}"
        );
        let items = features::completion(&workspace, &main, Text::new(source).position(offset));
        assert!(
            !items.iter().any(|item| item.label == "sample.rs"),
            "{marked}: {items:?}"
        );
        assert!(
            file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: uri(root, "sample.rs").into(),
                    new_uri: uri(root, "new.rs").into()
                }]
            )
            .unwrap()
            .is_none(),
            "{marked}"
        );
    }
    // Plain global formatting options retain the known default lookup.
    workspace.open(
        main.clone(),
        r"\lstset{language=Rust,columns=fullflexible}\lstinputlisting{sample.rs}".into(),
        4,
    );
    assert_eq!(navigation::document_links(&workspace, &main).len(), 1);
}

#[test]
fn command_completion_and_signature_follow_one_optional_argument_without_a_star() {
    let mut workspace = Workspace::default();
    let main = Url::parse("file:///listings/main.tex").unwrap();
    for (marked, active) in [
        (r"\lstinputlisting[language=Ru¦st]{sample.rs}", 0),
        (r"\lstinputlisting[title=[literal]{sam¦ple.rs}", 1),
        (r"\lstinputlisting{sam¦ple.rs}", 1),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 1);
        let help =
            latex_lsp::signature::help(&workspace, &main, Text::new(source).position(offset))
                .unwrap();
        assert_eq!(help.signatures[0].label, r"\lstinputlisting[options]{file}");
        assert_eq!(help.active_parameter, Some(active));
    }
    for marked in [
        r"\lstinputlisting*{sam¦ple.rs}",
        r"\lstinputlisting[a][b]{sam¦ple.rs}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 1);
        assert!(
            latex_lsp::signature::help(&workspace, &main, Text::new(source).position(offset))
                .is_none()
        );
    }
    workspace.open(main.clone(), r"\lstinputlis".into(), 1);
    assert!(
        features::completion(&workspace, &main, Position::new(0, 12))
            .iter()
            .any(|item| item.label == "lstinputlisting")
    );
}
