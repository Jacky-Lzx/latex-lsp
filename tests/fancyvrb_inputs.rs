use lsp_types::*;
use std::path::Path;
use texquill::{
    editing::file_operations,
    language::{features, navigation, signature},
    project::workspace::Workspace,
    text::Text,
};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn apply(source: &str, edits: Vec<TextEdit>) -> String {
    let mut edits = edits;
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
        panic!()
    };
    let cursor = Text::new(source.clone()).position(offset);
    assert_eq!(edit.range.start.line, cursor.line);
    assert_eq!(edit.range.end.line, cursor.line);
    let from = Text::new(source.clone()).offset(edit.range.start).unwrap();
    assert!(item
        .filter_text
        .as_ref()
        .unwrap()
        .starts_with(&source[from..offset]));
    let mut edits = item.additional_text_edits.clone().unwrap_or_default();
    edits.push(edit.clone());
    let mut ranges = edits.iter().map(|edit| edit.range).collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start);
    assert!(ranges.windows(2).all(|pair| pair[0].end <= pair[1].start));
    apply(&source, edits)
}
fn edits(edit: WorkspaceEdit, main: &Url) -> Vec<TextEdit> {
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!()
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.uri, *main);
    assert_eq!(documents[0].text_document.version, Some(7));
    documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect()
}

#[test]
fn completion_quotes_spaces_and_detokenizes_unicode_with_plain_starred_and_partial_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    for name in [
        "sample.rs",
        "sample.unknown",
        "README",
        "sample one.rs",
        "样例, one.rs",
    ] {
        std::fs::write(root.join(name), "Code.").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (marked, name, expected) in [
        (
            r"\VerbatimInput{sam¦ple.rs}",
            "sample.rs",
            r"\VerbatimInput{sample.rs}",
        ),
        (
            r"\VerbatimInput*{sam¦ple.unknown}",
            "sample.unknown",
            r"\VerbatimInput*{sample.unknown}",
        ),
        (
            r"\VerbatimInput *[frame=single]{REA¦}",
            "README",
            r"\VerbatimInput *[frame=single]{README}",
        ),
        (
            r"\VerbatimInput{sam¦}",
            "sample one.rs",
            r#"\VerbatimInput{"sample one.rs"}"#,
        ),
        (
            r#"\VerbatimInput{"sam¦ple one.rs"}"#,
            "sample one.rs",
            r#"\VerbatimInput{"sample one.rs"}"#,
        ),
        (
            r"\VerbatimInput{样¦}",
            "样例, one.rs",
            r#"\VerbatimInput{\detokenize{"样例, one.rs"}}"#,
        ),
        (
            r#"\VerbatimInput{\detokenize{"样¦例, one.rs"}}"#,
            "样例, one.rs",
            r#"\VerbatimInput{\detokenize{"样例, one.rs"}}"#,
        ),
        (
            r"\VerbatimInput{sam¦",
            "sample one.rs",
            r#"\VerbatimInput{"sample one.rs""#,
        ),
        (
            r#"\VerbatimInput{"sam¦"#,
            "sample one.rs",
            r#"\VerbatimInput{"sample one.rs""#,
        ),
        (
            r#"\VerbatimInput{\detokenize{"样¦"#,
            "样例, one.rs",
            r#"\VerbatimInput{\detokenize{"样例, one.rs"}"#,
        ),
    ] {
        assert_eq!(
            complete(&mut workspace, &main, marked, name),
            expected,
            "{marked}"
        );
    }
}

#[test]
fn directory_completions_keep_suffixes_and_use_single_line_edits_for_multiline_wrappers() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    for name in ["code dir", "目录"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("sample.rs"), "Code.").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (marked, name, expected) in [
        (
            r#"\VerbatimInput{"co¦/sample.rs"}"#,
            "code dir/",
            r#"\VerbatimInput{"code dir/sample.rs"}"#,
        ),
        (
            r"\VerbatimInput{\detokenize{目¦/sample.rs}}",
            "目录/",
            r#"\VerbatimInput{\detokenize{"目录/sample.rs"}}"#,
        ),
        (
            "\\VerbatimInput{\n \"co¦/sample.rs\"\n}",
            "code dir/",
            r#"\VerbatimInput{"code dir/sample.rs"}"#,
        ),
        (
            "\\VerbatimInput{\r\n \\detokenize{\"目¦/sample.rs\"}\r\n}",
            "目录/",
            r#"\VerbatimInput{\detokenize{"目录/sample.rs"}}"#,
        ),
    ] {
        assert_eq!(complete(&mut workspace, &main, marked, name), expected);
    }
}

#[test]
fn navigation_and_versioned_renames_preserve_options_comments_and_filename_wrappers() {
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
    std::fs::write(root.join("code/sample.rs"), "Code.").unwrap();
    std::fs::write(root.join("chapters/code/sample.rs"), "Decoy.").unwrap();
    let child = uri(root, "chapters/child.tex");
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            versioned_edits: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    for (value, ascii, unicode) in [
        (
            "code/sample.rs",
            r#""new, dir/sample.rs""#,
            r#"\detokenize{"代码/sample.rs"}"#,
        ),
        (
            r#""code/sample.rs""#,
            r#""new, dir/sample.rs""#,
            r#"\detokenize{"代码/sample.rs"}"#,
        ),
        (
            r"\detokenize{code/sample.rs}",
            r#"\detokenize{"new, dir/sample.rs"}"#,
            r"\detokenize{代码/sample.rs}",
        ),
    ] {
        let source = format!(
            "% 😀\r\n\\VerbatimInput *[frame=single,label={{A,B [ C]}}] % keep\r\n{{{value}}}\r\n"
        );
        std::fs::write(child.to_file_path().unwrap(), &source).unwrap();
        workspace.open(child.clone(), source.clone(), 7);
        let links = navigation::document_links(&workspace, &child);
        assert_eq!(links.len(), 1, "{source}: {links:?}");
        assert_eq!(links[0].target, Some(uri(root, "code/sample.rs")));
        assert_eq!(
            features::definitions(&workspace, &child, links[0].range.start)[0].uri,
            uri(root, "code/sample.rs")
        );
        for (new, expected) in [("new, dir", ascii), ("代码", unicode)] {
            let edit = file_operations::will_rename(
                &workspace,
                &[FileRename {
                    old_uri: uri(root, "code").into(),
                    new_uri: uri(root, new).into(),
                }],
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                apply(&source, edits(edit, &child)),
                source.replace(value, expected)
            );
        }
    }
    std::fs::remove_file(root.join("code/sample.rs")).unwrap();
    assert!(navigation::document_links(&workspace, &child).is_empty());
}

#[test]
fn tex_shadowing_and_data_roles_are_consistent_for_quoted_and_detokenized_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    let data = uri(root, "sample.rs.tex");
    std::fs::write(root.join("sample.rs"), "Decoy.").unwrap();
    std::fs::write(
        root.join("sample.rs.tex"),
        r"\label{code-only}\input{unread}\fvset{commandchars=bad}",
    )
    .unwrap();
    std::fs::write(root.join("unread.tex"), r"\label{unread}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for value in ["sample.rs", r#""sample.rs""#, r"\detokenize{sample.rs}"] {
        let source =
            format!(r"\documentclass{{article}}\VerbatimInput{{{value}}}\ref{{code-only}}");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        workspace.open(main.clone(), source.clone(), 7);
        for recorded in [false, true] {
            if recorded {
                workspace
                    .project
                    .recorded_inputs
                    .insert(main.clone(), [data.clone()].into());
            }
            assert_eq!(
                navigation::document_links(&workspace, &main)[0].target,
                Some(data.clone())
            );
            assert!(workspace.project_files(&main).contains(&data));
            assert!(!workspace.project_documents(&main).contains(&data));
            assert!(!workspace
                .project_files(&main)
                .contains(&uri(root, "unread.tex")));
            let pos = Text::new(source.clone()).position(source.rfind("code-only").unwrap());
            assert!(features::definitions(&workspace, &main, pos).is_empty());
        }
        workspace.project.recorded_inputs.clear();
    }
}

#[test]
fn executable_options_invalid_shapes_macros_and_redefinitions_do_not_guess_data_or_path_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    let code = uri(root, "sample.rs");
    std::fs::write(root.join("sample.rs"), r"\label{executed}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for marked in [
        r"\VerbatimInput**{sam¦ple.rs}",
        r"\VerbatimInput[a][b]{sam¦ple.rs}",
        r"\VerbatimInput[commandchars=abc]{sam¦ple.rs}",
        r"\VerbatimInput[codes=anything]{sam¦ple.rs}",
        r"\VerbatimInput[unknown=value]{sam¦ple.rs}",
        r"\VerbatimInput[fontsize=\small]{sam¦ple.rs}",
        r"\VerbatimInput{\macro/sam¦ple.rs}",
        r"\VerbatimInput{\detokenizeX{sam¦ple.rs}}",
        r"\VerbatimInput{\detokenize{sam¦ple.rs} tail}",
        r"\fvset{commandchars=abc}\VerbatimInput{sam¦ple.rs}",
        r"\renewcommand{\VerbatimInput}[1]{}\VerbatimInput{sam¦ple.rs}",
        r"\def\FV@Input#1{}\VerbatimInput{sam¦ple.rs}",
        r"\def\FV@@Input{}\VerbatimInput{sam¦ple.rs}",
        r"\def\FV@ReadLine{}\VerbatimInput{sam¦ple.rs}",
        r"\let\FVC@VerbatimInput\relax\VerbatimInput{sam¦ple.rs}",
        r"\def\KV@FV@frame#1{}\VerbatimInput[frame=single]{sam¦ple.rs}",
        r"\setkeys{FV}{commandchars=abc}\VerbatimInput{sam¦ple.rs}",
        r"\define@key{FV}{frame}{}\VerbatimInput[frame=single]{sam¦ple.rs}",
        r"\setkeys{\family}{commandchars=abc}\VerbatimInput{sam¦ple.rs}",
        r"\FV@AddToHook\FV@CatCodesHook{\catcode92=0}\VerbatimInput{sam¦ple.rs}",
        r"\let\setkeys\custom\fvset{frame=single}\VerbatimInput{sam¦ple.rs}",
        r"\let\detokenize\relax\VerbatimInput{sam¦ple.rs}",
        r"\RecustomVerbatimCommand{\VerbatimInput}{VerbatimInput}{}\VerbatimInput{sam¦ple.rs}",
    ] {
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        assert!(
            navigation::document_links(&workspace, &main).is_empty(),
            "{marked}"
        );
        assert!(!workspace.project_files(&main).contains(&code), "{marked}");
        let items = features::completion(&workspace, &main, Text::new(source).position(at));
        assert!(
            !items.iter().any(|item| item.label == "sample.rs"),
            "{marked}: {items:?}"
        );
        workspace
            .project
            .recorded_inputs
            .insert(main.clone(), [code.clone()].into());
        assert!(
            workspace.project_documents(&main).contains(&code),
            "recorded executable code must retain its source role: {marked}"
        );
        workspace.project.recorded_inputs.clear();
    }
    workspace.open(main.clone(), r"\VerbatimInput{sample one.rs}".into(), 7);
    std::fs::write(root.join("sample one.rs"), "Code.").unwrap();
    assert!(navigation::document_links(&workspace, &main).is_empty());
    workspace.open(
        main.clone(),
        r"\fvset{commandchars=abc}\VerbatimInput{sample.rs}".into(),
        7,
    );
    workspace
        .project
        .recorded_inputs
        .insert(main.clone(), [code.clone()].into());
    assert!(
        workspace.project_documents(&main).contains(&code),
        "executable code must not be excluded as display data"
    );
}

#[test]
fn families_keep_independent_settings_and_signatures_respect_single_optional_and_star() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = uri(root, "main.tex");
    std::fs::write(root.join("sample.rs"), "Code.").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for settings in [
        r"\lstset{inputpath=other}",
        r"\renewcommand{\verbatiminput}[1]{}",
        r"\fvset{commandchars=none,frame=single}",
        r"\setkeys{other}{commandchars=abc}",
        r"\define@key{other}{frame}{}",
    ] {
        workspace.open(
            main.clone(),
            format!(r"{settings}\VerbatimInput{{sample.rs}}"),
            7,
        );
        assert_eq!(
            navigation::document_links(&workspace, &main).len(),
            1,
            "{settings}"
        );
    }
    workspace.open(main.clone(),r"\fvset{commandchars=abc}\lstinputlisting{sample.rs}\verbatiminput{sample.rs}\VerbatimInput{sample.rs}".into(),7);
    assert_eq!(navigation::document_links(&workspace, &main).len(), 2);
    for (marked, active) in [
        (r"\VerbatimInput[fra¦me=single]{sample.rs}", 0),
        (r"\VerbatimInput *[frame=single]{sam¦ple.rs}", 1),
        (r#"\VerbatimInput{\detokenize{"sam¦ple.rs"}}"#, 1),
    ] {
        let at = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        let help = signature::help(&workspace, &main, Text::new(source).position(at)).unwrap();
        assert_eq!(help.active_parameter, Some(active));
        assert_eq!(help.signatures[0].label, r"\VerbatimInput[options]{file}");
    }
}
