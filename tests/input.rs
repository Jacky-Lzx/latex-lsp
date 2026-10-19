use latex_lsp::{build, editing, features, file_operations, navigation, workspace::Workspace};
use std::path::Path;
use tower_lsp::lsp_types::*;

const SOURCE: &str = "\\documentclass{article}\n\\begin{document}\n\\input chapter.tex\n\\input\"my file\"\n{\\input chapter }\n% \\input ignored\n\\verb|\\input ignored|\n\\input % comment before filename\n  chapter\n\\end{document}\n";

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

#[test]
#[ignore = "requires pdflatex"]
fn real_missing_include_spelling_fixes_read_the_intended_chapter_after_recompiling() {
    for extension in ["", ".tex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::create_dir(root.join("sections")).unwrap();
        std::fs::write(
            root.join("sections/my chapter.tex"),
            "\\typeout{INCLUDED CORRECTED CHAPTER}Chapter.\n",
        )
        .unwrap();
        let source = format!("\\documentclass{{article}}\\begin{{document}}Start.\n\\include % keep\r\n{{sections/my chapetr{extension}}}\r\nEnd.\\end{{document}}\n");
        std::fs::write(&main, &source).unwrap();
        let config = latex_lsp::config::BuildConfig {
            executable: "pdflatex".into(),
            ..Default::default()
        };
        let compile = || {
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{}", outcome.output);
            outcome
        };
        compile();
        let outcome = compile();
        let main_uri = uri(root, "main.tex");
        let diagnostic = outcome.diagnostics[&main_uri]
            .iter()
            .find(|item| item.code == Some(NumberOrString::String("missing-include".into())))
            .unwrap()
            .clone();
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        workspace
            .build_diagnostics
            .insert(main_uri.clone(), outcome.diagnostics);
        let actions = editing::code_actions(
            &workspace,
            &main_uri,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic],
                ..Default::default()
            },
        );
        assert_eq!(actions.len(), 1, "{actions:?}");
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!()
        };
        let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&main_uri].len(), 1);
        let edit = &changes[&main_uri][0];
        assert_eq!(edit.new_text, format!("sections/my chapter{extension}"));
        let updated = latex_lsp::text::Text::new(source.clone())
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text.clone(),
            }])
            .unwrap();
        assert_eq!(updated.source, source.replace("chapetr", "chapter"));
        std::fs::write(&main, updated.source).unwrap();
        compile();
        let corrected = compile();
        assert!(
            corrected.diagnostics.values().all(Vec::is_empty),
            "{:?}",
            corrected.diagnostics
        );
        assert!(
            corrected.output.contains("INCLUDED CORRECTED CHAPTER"),
            "{}",
            corrected.output
        );
        assert!(corrected
            .inputs
            .unwrap()
            .contains(&uri(root, "sections/my chapter.tex")));
    }
}

fn project(root: &Path) -> Workspace {
    for (name, source) in [
        ("main.tex", SOURCE),
        ("chapter.tex", "\\typeout{BARE INPUT READ}Chapter.\n"),
        ("my file.tex", "\\typeout{QUOTED INPUT READ}Quoted.\n"),
        ("ignored.tex", "Ignored"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn renames(root: &Path) -> Vec<FileRename> {
    [
        ("chapter.tex", "new 中文.tex"),
        ("my file.tex", "quoted new.tex"),
    ]
    .into_iter()
    .map(|(old, new)| FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    })
    .collect()
}

fn changed(workspace: &Workspace, files: &[FileRename]) -> String {
    let changes = file_operations::will_rename(workspace, files)
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    let main = files[0]
        .old_uri
        .parse::<Url>()
        .unwrap()
        .join("main.tex")
        .unwrap();
    let mut edits = changes[&main].clone();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[&main]
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
        .source
}

#[test]
fn bare_and_quoted_input_paths_support_links_definitions_dependencies_and_root_detection() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    let main = uri(root, "main.tex");
    let chapter = uri(root, "chapter.tex");
    let quoted = uri(root, "my file.tex");
    assert_eq!(
        workspace.project_files(&main),
        [main.clone(), chapter.clone(), quoted.clone()].into()
    );
    assert_eq!(workspace.root_document(&quoted).unwrap(), main);
    let links = navigation::document_links(&workspace, &main);
    assert_eq!(links.len(), 4, "{links:?}");
    assert_eq!(
        links[0].range,
        Range::new(Position::new(2, 7), Position::new(2, 18))
    );
    assert_eq!(
        links[1].range,
        Range::new(Position::new(3, 7), Position::new(3, 14))
    );
    assert_eq!(
        features::definitions(&workspace, &main, Position::new(2, 10))[0].uri,
        chapter
    );
    assert!(workspace.diagnostics(&main).is_empty());
}

#[test]
fn bare_input_renames_add_braces_for_spaces_and_preserve_existing_quotes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    let source = changed(&workspace, &renames(root));
    assert!(source.contains("\\input {new 中文.tex}\n"), "{source}");
    assert!(source.contains("\\input\"quoted new\"\n"), "{source}");
    assert!(source.contains("{\\input {new 中文} }\n"), "{source}");
    assert!(
        source.contains("\\input % comment before filename\n  {new 中文}\n"),
        "{source}"
    );
    assert!(source.contains("% \\input ignored\n"));
    assert!(source.contains("\\verb|\\input ignored|\n"));
}

#[test]
fn bare_input_selection_ranges_respect_utf16_and_eof_and_macro_paths_remain_unresolved() {
    for source in ["😀 \\input 中文", "\\input\"中文 file\"", "{\\input 中文 }"] {
        let uri = Url::parse("file:///input/main.tex").unwrap();
        let doc = latex_lsp::workspace::Document::new(&uri, source.into(), Some(1));
        let offset = source.find("中文").unwrap();
        let selections = editing::selection_ranges(&doc, &[doc.text.position(offset)]).unwrap();
        let mut current = &selections[0];
        loop {
            let start = doc.text.offset(current.range.start).unwrap();
            let end = doc.text.offset(current.range.end).unwrap();
            assert!(start <= offset && offset <= end && end <= source.len());
            if let Some(parent) = &current.parent {
                current = parent;
            } else {
                break;
            }
        }
    }
    for source in [
        "\\input chapter\\suffix",
        "\\input chapter% joined comment\nnext",
        "\\input\"chapter\".tex",
        "{\\input chapter}",
    ] {
        let text = latex_lsp::text::Text::new(source.into());
        let syntax = latex_lsp::syntax::Syntax::latex(&text);
        assert!(
            syntax
                .commands
                .iter()
                .find(|command| command.name == "input")
                .unwrap()
                .argument
                .is_none(),
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn bare_and_quoted_inputs_still_compile_after_file_renames_introduce_spaces_and_unicode() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let before = build::run(&root.join("main.tex"), &config, &Default::default());
    assert_eq!(before.status, "success", "{}", before.output);
    assert!(
        before.output.contains("BARE INPUT READ") && before.output.contains("QUOTED INPUT READ")
    );
    let files = renames(root);
    std::fs::write(root.join("main.tex"), changed(&workspace, &files)).unwrap();
    for file in &files {
        std::fs::rename(
            Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
            Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
        )
        .unwrap();
    }
    file_operations::did_rename(&mut workspace, &files).unwrap();
    let after = build::run(&root.join("main.tex"), &config, &Default::default());
    assert_eq!(after.status, "success", "{}", after.output);
    assert!(after.output.contains("BARE INPUT READ") && after.output.contains("QUOTED INPUT READ"));
    let links = navigation::document_links(&workspace, &uri(root, "main.tex"));
    assert_eq!(links.len(), 4);
    assert!(links
        .iter()
        .all(|link| !link.target.as_ref().unwrap().path().contains("chapter")));
}

#[test]
#[ignore = "requires pdflatex and xelatex"]
fn unicode_input_filename_renames_read_the_moved_file_for_bare_and_quoted_forms() {
    for engine in ["pdflatex", "xelatex"] {
        for quoted in [false, true] {
            for stem in ["新章节", "新 章节"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let command = if quoted {
                    "\\input\"old\" \\relax"
                } else {
                    "\\input old \\relax"
                };
                let source = format!("\\documentclass{{article}}\n% 😀 retain this comment\n\\begin{{document}}Before.\n{command}\n\\end{{document}}\n");
                std::fs::write(root.join("main.tex"), &source).unwrap();
                std::fs::write(
                    root.join("old.tex"),
                    "\\typeout{UNICODE-INPUT-READ}Input.\n",
                )
                .unwrap();
                let mut workspace = Workspace::default();
                workspace.add_root(root.into());
                let config = latex_lsp::config::BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                };
                let before = build::run(&root.join("main.tex"), &config, &Default::default());
                assert_eq!(before.status, "success", "{}", before.output);
                let name = format!("{stem}.tex");
                let files = [FileRename {
                    old_uri: uri(root, "old.tex").to_string(),
                    new_uri: uri(root, &name).to_string(),
                }];
                let updated = changed(&workspace, &files);
                let expected =
                    format!("\\input{}{{{stem}}} \\relax", if quoted { "" } else { " " });
                assert!(updated.contains(&expected), "{updated}");
                std::fs::write(root.join("main.tex"), updated).unwrap();
                std::fs::rename(root.join("old.tex"), root.join(&name)).unwrap();
                file_operations::did_rename(&mut workspace, &files).unwrap();
                let after = build::run(&root.join("main.tex"), &config, &Default::default());
                assert_eq!(
                    after.status, "success",
                    "{engine}, {quoted}, {stem}: {}",
                    after.output
                );
                assert!(
                    after.output.contains("UNICODE-INPUT-READ"),
                    "{}",
                    after.output
                );
                assert!(after.inputs.unwrap().contains(&uri(root, &name)));
                let links = navigation::document_links(&workspace, &uri(root, "main.tex"));
                assert_eq!(links.len(), 1);
                assert_eq!(links[0].target, Some(uri(root, &name)));
            }
        }
    }
}
