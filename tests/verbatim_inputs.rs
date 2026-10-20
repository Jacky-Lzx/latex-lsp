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
        let marked = format!("% 😀\r\n\\verbatiminput * % keep\r\n{{{typed}}}{{body, untouched}}");
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
        complete(&mut workspace, &main, r"\verbatiminput{REA¦}", "README"),
        r"\verbatiminput{README}"
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
    workspace.open(main.clone(), r"\verbatiminput{sample.rs}".into(), 2);
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
        r"\verbatiminput{sample.rs}",
        edits.changes.unwrap()[&main].clone(),
    );
    assert_eq!(changed, r"\verbatiminput{renamed.payload}");
    workspace.open(main.clone(), r"\verbatiminput{sam}".into(), 3);
    let position = Text::new(r"\verbatiminput{sam}".into()).position(r"\verbatiminput{sam".len());
    let items = features::completion(&workspace, &main, position);
    assert!(
        !items.iter().any(|item| item.label == "sample.rs"),
        "{items:?}"
    );
    workspace.open(main.clone(), r"\verbatiminput{README}".into(), 4);
    assert!(
        navigation::document_links(&workspace, &main).is_empty(),
        "an exact lookup must not append a dot"
    );
}

#[test]
fn file_and_directory_moves_produce_versioned_edits_and_keep_stars_and_comments() {
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
    let source = "% 😀\r\n\\verbatiminput* % keep\r\n{code/sample.rs}\r\n";
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
        "verbatim does not fall back to the child directory"
    );
}

#[test]
fn static_code_files_are_build_inputs_without_importing_apparent_source_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    for name in ["README", "example.unknown", "example.tex"] {
        std::fs::write(root.join(name), r"\label{code-only}\input{unread}").unwrap();
    }
    std::fs::write(root.join("unread.tex"), r"\label{unread-label}").unwrap();
    let source = r"\documentclass{article}\verbatiminput{README}\verbatiminput{example.unknown}\verbatiminput{example.tex}\ref{code-only}";
    std::fs::write(root.join("main.tex"), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace.open(main.clone(), source.into(), 1);
    for recorded in [false, true] {
        if recorded {
            workspace
                .recorded_inputs
                .insert(main.clone(), [uri(root, "example.tex")].into());
        }
        let files = workspace.project_files(&main);
        for name in ["README", "example.unknown", "example.tex"] {
            assert!(files.contains(&uri(root, name)), "{name}: {files:?}");
            assert!(!workspace
                .project_documents(&main)
                .contains(&uri(root, name)));
        }
        assert!(!files.contains(&uri(root, "unread.tex")));
        let position = Text::new(source.into()).position(source.rfind("code-only").unwrap());
        assert!(features::definitions(&workspace, &main, position).is_empty());
    }
    std::fs::write(root.join("example.tex"), "Changed closed code.").unwrap();
    workspace.refresh_project(&main);
    assert_eq!(
        workspace.documents[&uri(root, "example.tex")].text.source,
        "Changed closed code."
    );
    assert_eq!(workspace.root_document(&uri(root, "README")).unwrap(), main);
}

#[test]
fn code_and_source_roles_are_independent_for_shared_files_and_explicit_inputs_win() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let code = uri(root, "example.tex");
    std::fs::write(root.join("example.tex"), r"\label{shared-label}").unwrap();
    let mut workspace = Workspace::default();
    for (name, inclusion) in [
        ("listing.tex", r"\verbatiminput{example.tex}"),
        ("source.tex", r"\input{example.tex}"),
        (
            "both.tex",
            r"\verbatiminput{example.tex}\input{example.tex}",
        ),
    ] {
        std::fs::write(
            root.join(name),
            format!("\\documentclass{{article}}{inclusion}\\ref{{shared-label}}"),
        )
        .unwrap();
    }
    workspace.add_root(root.into());
    for name in ["listing.tex", "source.tex", "both.tex"] {
        let main = uri(root, name);
        workspace
            .recorded_inputs
            .insert(main, [code.clone()].into());
    }
    for (name, is_source) in [
        ("listing.tex", false),
        ("source.tex", true),
        ("both.tex", true),
    ] {
        let main = uri(root, name);
        assert_eq!(
            workspace.project_documents(&main).contains(&code),
            is_source,
            "{name}"
        );
        assert!(workspace.project_files(&main).contains(&code));
        let source = &workspace.documents[&main].text;
        let position = source.position(source.source.rfind("shared-label").unwrap());
        assert_eq!(
            !features::definitions(&workspace, &main, position).is_empty(),
            is_source,
            "{name}"
        );
    }
}

#[test]
fn invalid_verbatim_calls_and_redefinitions_disable_path_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    std::fs::write(root.join("sample.rs"), "code").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for marked in [
        r"\verbatiminput[options]{sam¦ple.rs}",
        r"\verbatiminput*[options]{sam¦ple.rs}",
        r"\verbatiminput**{sam¦ple.rs}",
        r"\verbatiminput * *{sam¦ple.rs}",
        r"\verbatiminput{\macro/sam¦ple.rs}",
        r"\renewcommand{\verbatiminput}[1]{}\verbatiminput{sam¦ple.rs}",
        r"\def\verbatim@input#1#2{}\verbatiminput{sam¦ple.rs}",
        r"\let\verbatim@readfile\input\verbatiminput{sam¦ple.rs}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 3);
        assert!(
            navigation::document_links(&workspace, &main).is_empty(),
            "{marked}"
        );
        assert!(
            !workspace
                .project_files(&main)
                .contains(&uri(root, "sample.rs")),
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
                    new_uri: uri(root, "new.rs").into(),
                }]
            )
            .unwrap()
            .is_none(),
            "{marked}"
        );
    }
}

#[test]
fn command_families_have_independent_configuration_guards() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    std::fs::write(root.join("sample.rs"), "Code.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (settings, supported) in [
        (r"\lstset{inputpath=other}", "verbatiminput"),
        (r"\lstset{style=custom}", "verbatiminput"),
        (r"\renewcommand{\lstinputlisting}[1]{}", "verbatiminput"),
        (r"\renewcommand{\verbatiminput}[1]{}", "lstinputlisting"),
        (r"\def\verbatim@input#1#2{}", "lstinputlisting"),
    ] {
        let source =
            format!(r"{settings}\lstinputlisting{{sample.rs}}\verbatiminput*{{sample.rs}}");
        workspace.open(main.clone(), source, 3);
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1, "{settings}: {links:?}");
        let doc = &workspace.documents[&main];
        let start = doc.text.offset(links[0].range.start).unwrap();
        assert!(doc.text.source[..start].ends_with(&format!(
            "\\{supported}{}{{",
            if supported == "verbatiminput" {
                "*"
            } else {
                ""
            }
        )));
        assert!(workspace
            .project_files(&main)
            .contains(&uri(root, "sample.rs")));
    }
}

#[test]
fn displayed_settings_are_ignored_but_redefined_commands_do_not_hide_executed_settings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    let settings = uri(root, "settings.tex");
    std::fs::write(root.join("sample.rs"), "Code.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (display, redefinition, displayed_settings, remaining) in [
        (
            "verbatiminput",
            r"\renewcommand{\verbatiminput}[1]{\input{#1}}",
            r"\lstset{inputpath=other}",
            0,
        ),
        (
            "lstinputlisting",
            r"\renewcommand{\lstinputlisting}[1]{\input{#1}}",
            r"\renewcommand{\verbatiminput}[1]{}",
            0,
        ),
    ] {
        std::fs::write(root.join("settings.tex"), displayed_settings).unwrap();
        workspace.open(settings.clone(), displayed_settings.into(), 1);
        for redefined in [false, true] {
            let source = format!(
                r"\documentclass{{article}}{}\{display}{{settings.tex}}\lstinputlisting{{sample.rs}}\verbatiminput{{sample.rs}}",
                if redefined { redefinition } else { "" }
            );
            workspace.open(main.clone(), source, 2);
            workspace
                .recorded_inputs
                .insert(main.clone(), [settings.clone()].into());
            let links = navigation::document_links(&workspace, &main);
            assert_eq!(
                links.len(),
                if redefined { remaining } else { 3 },
                "{display}/{redefined}: {links:?}"
            );
        }
    }
}

#[test]
fn signatures_follow_the_single_group_and_optional_star() {
    let main = Url::parse("file:///verbatim/main.tex").unwrap();
    let mut workspace = Workspace::default();
    for marked in [
        r"\verbatiminput{sam¦ple.rs}",
        r"\verbatiminput*{sam¦ple.rs}",
        r"\verbatiminput * % keep
{sam¦ple.rs}",
        r"\verbatiminput{¦",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 1);
        let help =
            latex_lsp::signature::help(&workspace, &main, Text::new(source).position(offset))
                .unwrap();
        assert_eq!(help.active_parameter, Some(0), "{marked}");
        assert_eq!(help.signatures[0].label, r"\verbatiminput{file}");
    }
    for marked in [
        r"\verbatiminput[x]{sam¦ple.rs}",
        r"\verbatiminput**{sam¦ple.rs}",
        r"\verbatiminput{sample.rs}{out¦side}",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 1);
        assert!(
            latex_lsp::signature::help(&workspace, &main, Text::new(source).position(offset))
                .is_none(),
            "{marked}"
        );
    }
    assert!(latex_lsp::catalog::COMMANDS.iter().any(
        |(name, snippet, _)| *name == "verbatiminput" && *snippet == "verbatiminput{${1:file}}"
    ));
}
