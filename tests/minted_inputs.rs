use latex_lsp::{
    features, file_operations, navigation, signature, text::Text, workspace::Workspace,
};
use lsp_types::*;
use std::path::Path;

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

fn complete(workspace: &mut Workspace, main: &Url, marked: &str, name: &str) -> String {
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    workspace.open(main.clone(), source.clone(), 7);
    let items = features::completion(workspace, main, Text::new(source.clone()).position(offset));
    let item = items
        .iter()
        .find(|item| item.label == name)
        .unwrap_or_else(|| panic!("{marked}: {items:?}"));
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("{item:?}")
    };
    assert_eq!(edit.range.start.line, edit.range.end.line);
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    apply(&source, edits)
}

#[test]
fn filename_completion_preserves_headers_and_python_filename_strings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for file in [
        "sample.rs",
        "sample.unknown",
        "README",
        "sample one.rs",
        "样例, one.rs",
    ] {
        std::fs::write(root.join(file), "Code.").unwrap();
    }
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (typed, name, expected) in [
        ("sam¦ple.rs", "sample.rs", "sample.rs"),
        ("sam¦ple.unknown", "sample.unknown", "sample.unknown"),
        ("REA¦", "README", "README"),
        ("sam¦", "sample one.rs", "sample one.rs"),
        ("样¦", "样例, one.rs", r"\detokenize{样例, one.rs}"),
        (
            r"\detokenize{样例, o¦ld.rs}",
            "样例, one.rs",
            r"\detokenize{样例, one.rs}",
        ),
    ] {
        let prefix = "% 😀\r\n\\inputminted[breaklines,firstline=2] % options\r\n{rust} % path\r\n";
        let marked = format!("{prefix}{{{typed}}}{{Following prose}}");
        let completed = complete(&mut workspace, &main, &marked, name);
        assert_eq!(
            completed,
            format!("{prefix}{{{expected}}}{{Following prose}}")
        );
        workspace.open(main.clone(), completed.clone(), 8);
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1, "{completed}: {links:?}");
        assert_eq!(links[0].target, Some(uri(root, name)));
        assert_eq!(
            features::definitions(&workspace, &main, links[0].range.start)[0].uri,
            uri(root, name)
        );
        assert!(workspace.project_files(&main).contains(&uri(root, name)));
    }
    assert_eq!(
        complete(
            &mut workspace,
            &main,
            r"\inputminted{rust}{sam¦",
            "sample.rs"
        ),
        r"\inputminted{rust}{sample.rs"
    );
    assert_eq!(
        complete(
            &mut workspace,
            &main,
            r"\inputminted{rust}{\detokenize{sam¦",
            "sample.rs"
        ),
        r"\inputminted{rust}{\detokenize{sample.rs}"
    );
}

#[test]
fn file_moves_use_the_project_root_and_versioned_edits_preserve_language_and_options() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("chapters")).unwrap();
    std::fs::create_dir(root.join("code")).unwrap();
    std::fs::create_dir(root.join("chapters/code")).unwrap();
    std::fs::write(
        root.join("main.tex"),
        r"\documentclass{article}\input{chapters/child}",
    )
    .unwrap();
    std::fs::write(root.join("code/sample.rs"), "Expected").unwrap();
    std::fs::write(root.join("chapters/code/sample.rs"), "Decoy").unwrap();
    let child = uri(root, "chapters/child.tex");
    let source = "% 😀\r\n\\inputminted[linenos]{rust}% path\r\n{code/sample.rs}\r\n";
    std::fs::write(child.to_file_path().unwrap(), source).unwrap();
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(child.clone(), source.into(), 7);
    assert_eq!(
        navigation::document_links(&workspace, &child)[0].target,
        Some(uri(root, "code/sample.rs"))
    );
    for (old, new, expected) in [
        ("code/sample.rs", "code/renamed.py", "code/renamed.py"),
        ("code", "new dir", "new dir/sample.rs"),
        (
            "code/sample.rs",
            "code/例子.rs",
            r"\detokenize{code/例子.rs}",
        ),
    ] {
        let edit = file_operations::will_rename(
            &workspace,
            &[FileRename {
                old_uri: uri(root, old).into(),
                new_uri: uri(root, new).into(),
            }],
        )
        .unwrap()
        .unwrap();
        let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
            panic!()
        };
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].text_document.uri, child);
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
}

#[test]
fn displayed_tex_files_are_data_dependencies_while_explicit_source_inputs_keep_their_role() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let data = uri(root, "example.tex");
    std::fs::write(root.join("example.tex"), r"\label{code-only}\input{unread}").unwrap();
    std::fs::write(root.join("unread.tex"), r"\label{unread-label}").unwrap();
    std::fs::write(root.join("README"), "Text").unwrap();
    for (file, extra) in [("display.tex", ""), ("source.tex", r"\input{example.tex}")] {
        std::fs::write(root.join(file), format!("\\documentclass{{article}}\\inputminted{{latex}}{{example.tex}}\\inputminted{{text}}{{README}}{extra}\\ref{{code-only}}")).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (file, source_role) in [("display.tex", false), ("source.tex", true)] {
        let main = uri(root, file);
        for recorded in [false, true] {
            if recorded {
                workspace
                    .recorded_inputs
                    .insert(main.clone(), [data.clone()].into());
            }
            let files = workspace.project_files(&main);
            assert!(files.contains(&data));
            assert!(files.contains(&uri(root, "README")));
            assert_eq!(files.contains(&uri(root, "unread.tex")), source_role);
            assert_eq!(
                workspace.project_documents(&main).contains(&data),
                source_role
            );
            let doc = &workspace.documents[&main];
            let at = doc
                .text
                .position(doc.text.source.rfind("code-only").unwrap());
            assert_eq!(
                !features::definitions(&workspace, &main, at).is_empty(),
                source_role
            );
        }
    }
}

#[test]
fn uncertain_options_overrides_and_import_prefixes_suppress_file_inference() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("sample.rs"), "Code").unwrap();
    let main = uri(root, "main.tex");
    for (settings, call) in [
        ("", r"\inputminted*{rust}{sample.rs}"),
        ("", r"\inputminted[linenos][extra]{rust}{sample.rs}"),
        ("", r"\inputminted{\language}{sample.rs}"),
        ("", r"\inputminted{rust}{\filename}"),
        ("", r#"\inputminted{rust}{"sample.rs"}"#),
        ("", r#"\inputminted{rust}{\detokenize{"sample.rs"}}"#),
        ("", r"\inputminted[escapeinside=||]{rust}{sample.rs}"),
        ("", r"\inputminted[texcomments]{rust}{sample.rs}"),
        ("", r"\inputminted[mathescape=true]{rust}{sample.rs}"),
        ("", r"\inputminted[unknown=true]{rust}{sample.rs}"),
        (
            r"\setminted{escapeinside=||}",
            r"\inputminted{rust}{sample.rs}",
        ),
        (
            r"\setminted[rust]{mathescape}",
            r"\inputminted{rust}{sample.rs}",
        ),
        (
            r"\fvset{commandchars=ABC}",
            r"\inputminted{rust}{sample.rs}",
        ),
        (
            r"\renewcommand{\inputminted}[2]{}",
            r"\inputminted{rust}{sample.rs}",
        ),
        (
            r"\def\import@path{other/}",
            r"\inputminted{rust}{sample.rs}",
        ),
        (
            r"\import{other/}{chapter}",
            r"\inputminted{rust}{sample.rs}",
        ),
        (
            r"\pgfkeys{/minted/cmd/linenos/.code={}}",
            r"\inputminted{rust}{sample.rs}",
        ),
    ] {
        std::fs::write(root.join("settings.sty"), settings).unwrap();
        let source = format!("\\documentclass{{article}}\\usepackage{{settings}}{call}");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        workspace.open(main.clone(), source.clone(), 7);
        assert!(
            navigation::document_links(&workspace, &main)
                .iter()
                .all(|link| link.target != Some(uri(root, "sample.rs"))),
            "{settings}/{call}"
        );
        let at = source
            .rfind("sample.rs")
            .map(|at| at + 3)
            .unwrap_or(source.len());
        assert!(
            !features::completion(&workspace, &main, Text::new(source).position(at))
                .iter()
                .any(|item| item.label == "sample.rs"),
            "{settings}/{call}"
        );
        assert!(
            !workspace
                .project_files(&main)
                .contains(&uri(root, "sample.rs")),
            "{settings}/{call}"
        );
    }
}

#[test]
fn filename_lookup_preserves_tex_suffix_priority_and_settings_are_independent_of_listings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["sample.rs", "sample.rs.tex"] {
        std::fs::write(root.join(name), "Code").unwrap();
    }
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let source = r"\documentclass{article}\lstset{inputpath=other}\setminted[rust]{linenos,breaklines}\inputminted[mathescape=false]{rust}{sample.rs}";
    workspace.open(main.clone(), source.into(), 7);
    assert_eq!(
        navigation::document_links(&workspace, &main)
            .last()
            .unwrap()
            .target,
        Some(uri(root, "sample.rs.tex"))
    );
    let items = features::completion(
        &workspace,
        &main,
        Text::new(source.into()).position(source.rfind("sample").unwrap() + 3),
    );
    assert!(!items.iter().any(|item| item.label == "sample.rs"));
    assert!(items.iter().any(|item| item.label == "sample.rs.tex"));
    for (marked, active) in [
        (r"\inputminted[lin¦enos]{rust}{sample.rs}", 0),
        (r"\inputminted[linenos]{ru¦st}{sample.rs}", 1),
        (r"\inputminted[linenos]{rust}{sam¦ple.rs}", 2),
    ] {
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 8);
        let help = signature::help(&workspace, &main, Text::new(source).position(at)).unwrap();
        assert_eq!(
            help.signatures[0].label,
            r"\inputminted[options]{language}{file}"
        );
        assert_eq!(help.active_parameter, Some(active));
    }
}

#[test]
fn verbatim_package_modes_keep_whitespace_paths_out_of_completion_and_file_moves() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["sample.rs", "sample one.rs"] {
        std::fs::write(root.join(name), "Code").unwrap();
    }
    let main = uri(root, "main.tex");
    let child = uri(root, "child.tex");
    for settings in [
        r"\usepackage[verbatim]{minted}",
        r"\RequirePackage[draft]{minted}",
        r"\PassOptionsToPackage{verbatim}{minted}\usepackage{minted}",
    ] {
        std::fs::write(
            root.join("main.tex"),
            format!("\\documentclass{{article}}{settings}\\input{{child}}"),
        )
        .unwrap();
        std::fs::write(root.join("child.tex"), r"\inputminted{rust}{sample.rs}").unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        workspace.open(child.clone(), r"\inputminted{rust}{sample.rs}".into(), 7);
        assert_eq!(
            navigation::document_links(&workspace, &child)[0].target,
            Some(uri(root, "sample.rs")),
            "{settings}"
        );
        let items = features::completion(&workspace, &child, Position::new(0, 22));
        assert!(
            items.iter().any(|item| item.label == "sample.rs"),
            "{settings}: {items:?}"
        );
        assert!(!items.iter().any(|item| item.label == "sample one.rs"));
        assert!(
            file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: uri(root, "sample.rs").into(),
                    new_uri: uri(root, "new file.rs").into()
                }]
            )
            .is_err(),
            "{settings}"
        );
        workspace.open(
            child.clone(),
            r"\inputminted{rust}{sample one.rs}".into(),
            8,
        );
        assert!(
            navigation::document_links(&workspace, &child).is_empty(),
            "{settings}"
        );
        assert!(!workspace
            .project_files(&main)
            .contains(&uri(root, "sample one.rs")));
    }
}

#[test]
#[ignore = "requires pdflatex, minted v3, and latexminted on PATH"]
fn real_inputminted_highlighting_confirms_spaced_unicode_filenames_and_data_roles() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("_minted")).unwrap();
    let mut source = "\\documentclass{article}\n\\usepackage[highlightmode=immediate]{minted}\n\\begin{document}\n".to_owned();
    for (index, name) in ["sample.rs", "sample one.rs", "样例, one.rs", "README"]
        .into_iter()
        .enumerate()
    {
        std::fs::write(
            root.join(name),
            format!("\\label{{phantom}}\\input{{missing}} % MINTEDSOURCE{index}\n"),
        )
        .unwrap();
        source.push_str(&format!(
            "\\inputminted[linenos]{{latex}}{{\\detokenize{{{name}}}}}\n"
        ));
    }
    source.push_str("\\label{real}\\end{document}\n");
    std::fs::write(root.join("main.tex"), &source).unwrap();
    let output = std::process::Command::new("pdflatex")
        .args([
            "-interaction=nonstopmode",
            "-halt-on-error",
            "-shell-restricted",
            "-recorder",
            "main.tex",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    let log = std::fs::read_to_string(root.join("main.log")).unwrap();
    assert!(output.status.success(), "{log}");
    // Fast-first mode may compile only placeholders. Check actual Pygments
    // output so a successful TeX exit or an external TEXINPUTS fixture cannot
    // silently stand in for the files created by this test.
    let highlights = std::fs::read_dir(root.join("_minted"))
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".highlight.minted")
        })
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(highlights.len(), 4, "{log}");
    for index in 0..4 {
        assert!(highlights
            .iter()
            .any(|text| text.contains(&format!("MINTEDSOURCE{index}"))));
    }
    let aux = std::fs::read_to_string(root.join("main.aux")).unwrap();
    assert!(aux.contains("\\newlabel{real}"));
    assert!(!aux.contains("phantom"));
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace.open(main.clone(), source, 7);
    assert_eq!(navigation::document_links(&workspace, &main).len(), 4);
    let files = workspace.project_files(&main);
    for name in ["sample.rs", "sample one.rs", "样例, one.rs", "README"] {
        assert!(files.contains(&uri(root, name)), "{name}: {files:?}");
        assert!(!workspace
            .project_documents(&main)
            .contains(&uri(root, name)));
    }
    let edit = file_operations::will_rename(
        &workspace,
        &[FileRename {
            old_uri: uri(root, "sample one.rs").into(),
            new_uri: uri(root, "renamed 样例.rs").into(),
        }],
    )
    .unwrap()
    .unwrap();
    let source = &workspace.documents[&main].text.source;
    let updated = apply(source, edit.changes.unwrap()[&main].clone());
    assert!(updated.contains(r"\detokenize{renamed 样例.rs}"));
    std::fs::rename(root.join("sample one.rs"), root.join("renamed 样例.rs")).unwrap();
    std::fs::write(root.join("main.tex"), updated).unwrap();
    let output = std::process::Command::new("pdflatex")
        .args([
            "-interaction=nonstopmode",
            "-halt-on-error",
            "-shell-restricted",
            "main.tex",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
