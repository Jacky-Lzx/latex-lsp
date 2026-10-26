use lsp_types::*;
use std::{collections::BTreeMap, path::Path};
use texquill::{
    editing::file_operations,
    project::workspace::{Document, Workspace},
    text::Text,
};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

#[test]
fn created_and_deleted_directories_refresh_sources_without_discarding_open_buffers() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    std::fs::create_dir(root.join("章节")).unwrap();
    std::fs::create_dir(root.join("章节-extra")).unwrap();
    for (name, source) in [
        ("章节/open.tex", "Saved"),
        ("章节/closed.tex", r"\label{closed}"),
        ("章节/refs.bib", "@book{book, title={Example}}"),
        ("章节/ignored.aux", "Generated"),
        ("章节-extra/neighbor.tex", "Neighbor"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let open = uri(root, "章节/open.tex");
    workspace.open(open.clone(), r"\label{unsaved}".into(), 9);
    let folder = uri(root, "章节");
    let alternate = folder
        .as_str()
        .replace("%E7%AB%A0%E8%8A%82", "%e7%ab%a0%e8%8a%82");
    let changes = file_operations::FileChanges::new([alternate.as_str()]).unwrap();
    assert!(!changes.affects(&uri(root, "章节-extra/neighbor.tex")));
    let affected = changes.refresh(&mut workspace);
    assert!(affected.contains(&uri(root, "章节/closed.tex")));
    assert!(affected.contains(&uri(root, "章节/refs.bib")));
    assert!(!workspace
        .documents
        .contains_key(&uri(root, "章节/ignored.aux")));
    assert_eq!(workspace.documents[&open].version, Some(9));
    assert_eq!(workspace.documents[&open].text.source, r"\label{unsaved}");
    workspace.reload(&uri(root, "章节-extra/neighbor.tex"));
    std::fs::remove_dir_all(root.join("章节")).unwrap();
    let affected = changes.refresh(&mut workspace);
    assert!(affected.contains(&uri(root, "章节/closed.tex")));
    assert!(!workspace
        .documents
        .contains_key(&uri(root, "章节/closed.tex")));
    assert!(!workspace
        .documents
        .contains_key(&uri(root, "章节/refs.bib")));
    assert_eq!(workspace.documents[&open].version, Some(9));
    assert!(workspace
        .documents
        .contains_key(&uri(root, "章节-extra/neighbor.tex")));
    assert!(file_operations::FileChanges::new([
        folder.as_str(),
        "https://example.test/remote.tex"
    ])
    .is_err());
}

#[test]
fn file_events_invalidate_only_affected_builds_and_preserve_artifact_input_checks() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("a")).unwrap();
    std::fs::create_dir(root.join("b")).unwrap();
    for name in ["a/main.tex", "b/main.tex"] {
        std::fs::write(
            root.join(name),
            r"\documentclass{article}\input{child}\input{fragment.inc}",
        )
        .unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let a = uri(root, "a/main.tex");
    let b = uri(root, "b/main.tex");
    let child = uri(root, "a/child.tex");
    let warning = Diagnostic::new_simple(Range::default(), "Old compiler warning".into());
    for main in [&a, &b] {
        workspace
            .project
            .build_diagnostics
            .insert(main.clone(), [(main.clone(), vec![warning.clone()])].into());
    }
    std::fs::write(root.join("a/child.tex"), r"\label{new}").unwrap();
    let changes = file_operations::FileChanges::new([child.as_str()]).unwrap();
    let affected = changes.refresh(&mut workspace);
    assert!(affected.contains(&a));
    assert!(!workspace.project.build_diagnostics.contains_key(&a));
    assert!(workspace.project.build_diagnostics.contains_key(&b));
    workspace
        .project
        .recorded_inputs
        .insert(a.clone(), [a.clone(), child.clone()].into());
    workspace
        .project
        .build_diagnostics
        .insert(a.clone(), [(a.clone(), vec![warning.clone()])].into());
    std::fs::remove_file(root.join("a/child.tex")).unwrap();
    changes.refresh(&mut workspace);
    assert!(!workspace.project.build_diagnostics.contains_key(&a));
    assert!(workspace.project.build_diagnostics.contains_key(&b));
    assert!(workspace.project.recorded_inputs[&a].contains(&child));
    // Images are not indexed as documents, but deleting a recorded image must
    // still invalidate the compiler diagnostics belonging to its project.
    let image = uri(root, "a/image.png");
    workspace
        .project
        .recorded_inputs
        .get_mut(&a)
        .unwrap()
        .insert(image.clone());
    workspace
        .project
        .build_diagnostics
        .insert(a.clone(), [(a.clone(), vec![warning])].into());
    file_operations::FileChanges::new([image.as_str()])
        .unwrap()
        .refresh(&mut workspace);
    assert!(!workspace.project.build_diagnostics.contains_key(&a));
    assert!(workspace.project.build_diagnostics.contains_key(&b));
    assert!(workspace.project.recorded_inputs[&a].contains(&image));
    let fragment = uri(root, "a/fragment.inc");
    std::fs::write(root.join("a/fragment.inc"), r"\label{fragment}").unwrap();
    file_operations::FileChanges::new([fragment.as_str()])
        .unwrap()
        .refresh(&mut workspace);
    assert!(workspace.documents.contains_key(&fragment));
}

fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

fn edits(edit: WorkspaceEdit) -> BTreeMap<Url, Vec<TextEdit>> {
    if let Some(changes) = edit.changes {
        return changes.into_iter().collect();
    }
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("missing edits");
    };
    documents
        .into_iter()
        .map(|document| {
            let edits = document
                .edits
                .into_iter()
                .map(|edit| match edit {
                    OneOf::Left(edit) => edit,
                    _ => panic!(),
                })
                .collect();
            (document.text_document.uri, edits)
        })
        .collect()
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

fn project(root: &Path) -> Workspace {
    std::fs::create_dir(root.join("chapters")).unwrap();
    for (name, source) in [
        ("main.tex", "\\documentclass{local}\n\\usepackage{pkg}\n\\input{chapters/old}\n\\bibliography{refs, other}\n\\includegraphics{fig}\n% \\input{chapters/old}\n\\verb|\\input{chapters/old}|\n"),
        ("chapters/old.tex", "% !TeX root = \"../main.tex\"\n\\input{../shared.v1}\n\\includegraphics{fig}\n"),
        ("shared.v1.tex", "Shared"), ("local.cls", "\\LoadClass{article}"),
        ("pkg.sty", "\\newcommand{\\hello}{Hello}"),
        ("refs.bib", "@book{key,title={Example}}"), ("other.bib", ""), ("fig.png", "png"),
    ] { std::fs::write(root.join(name), source).unwrap(); }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

#[test]
fn includeonly_paths_are_linked_and_renamed_without_becoming_input_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("chapters")).unwrap();
    let source = "\\documentclass{article}\n\\includeonly{chapters/old, chapters/second}\n% \\includeonly{chapters/old}\n";
    for (name, source) in [
        ("main.tex", source),
        ("chapters/old.tex", "Selected."),
        ("chapters/second.tex", "Other."),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = uri(root, "main.tex");
    assert_eq!(workspace.project_documents(&main), [main.clone()].into());
    let links = texquill::language::navigation::document_links(&workspace, &main);
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].target, Some(uri(root, "chapters/old.tex")));
    let edits = edits(
        file_operations::will_rename(
            &workspace,
            &[rename(root, "chapters/old.tex", "chapters/new.tex")],
        )
        .unwrap()
        .unwrap(),
    );
    let changed = apply(source, edits[&main].clone());
    assert!(changed.contains("\\includeonly{chapters/new, chapters/second}"));
    assert!(changed.contains("% \\includeonly{chapters/old}"));
}

fn extension_project(root: &Path) -> Workspace {
    std::fs::create_dir(root.join("chapters")).unwrap();
    for (name,source) in [
        ("main.tex", "\\documentclass{article}\n\\includeonly{chapters/selected}\n\\begin{document}\n\\input{chapters/input}\n\\include{chapters/selected}\n\\end{document}\n"),
        ("chapters/input.tex", "% !TeX root = ../main\n\\typeout{INPUT SOURCE}Input text.\n"),
        ("chapters/selected.tex", "\\typeout{INCLUDED SOURCE}Included text.\n"),
    ] { std::fs::write(root.join(name),source).unwrap(); }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

fn extension_renames(root: &Path) -> [FileRename; 3] {
    [
        rename(root, "main.tex", "paper.ltx"),
        rename(root, "chapters/input.tex", "chapters/input.ltx"),
        rename(root, "chapters/selected.tex", "chapters/renamed.tex"),
    ]
}

#[test]
fn changed_extensions_are_retained_when_tex_cannot_infer_them_and_packages_keep_required_extensions(
) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = extension_project(root);
    let changes = edits(
        file_operations::will_rename(&workspace, &extension_renames(root))
            .unwrap()
            .unwrap(),
    );
    let main = uri(root, "main.tex");
    let changed = apply(
        &workspace.documents[&main].text.source,
        changes[&main].clone(),
    );
    assert!(changed.contains("\\input{chapters/input.ltx}"));
    assert!(changed.contains("\\include{chapters/renamed}"));
    assert!(changed.contains("\\includeonly{chapters/renamed}"));
    let child = uri(root, "chapters/input.tex");
    assert!(apply(
        &workspace.documents[&child].text.source,
        changes[&child].clone()
    )
    .contains("% !TeX root = ../paper.ltx"));
    let error = file_operations::will_rename(
        &workspace,
        &[rename(
            root,
            "chapters/selected.tex",
            "chapters/selected.ltx",
        )],
    )
    .unwrap_err();
    assert!(error.message.contains(".tex"));

    let directory = tempfile::tempdir().unwrap();
    let workspace = project(directory.path());
    for (old, new, extension) in [
        ("pkg.sty", "pkg.cls", ".sty"),
        ("local.cls", "local.sty", ".cls"),
    ] {
        let error = file_operations::will_rename(&workspace, &[rename(directory.path(), old, new)])
            .unwrap_err();
        assert!(error.message.contains(extension), "{error:?}");
    }
}

#[test]
#[ignore = "requires latexmk"]
fn changing_source_extensions_preserves_input_includeonly_and_magic_root_compilation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = extension_project(root);
    let files = extension_renames(root);
    let edit = file_operations::will_rename(&workspace, &files)
        .unwrap()
        .unwrap();
    for (uri, changes) in edits(edit) {
        std::fs::write(
            uri.to_file_path().unwrap(),
            apply(&workspace.documents[&uri].text.source, changes),
        )
        .unwrap();
    }
    for file in &files {
        std::fs::rename(
            Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
            Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
        )
        .unwrap();
    }
    file_operations::did_rename(&mut workspace, &files).unwrap();
    let paper = uri(root, "paper.ltx");
    assert_eq!(
        workspace
            .root_document(&uri(root, "chapters/input.ltx"))
            .unwrap(),
        paper
    );
    let outcome = texquill::compilation::build::run(
        &root.join("paper.ltx"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    assert!(outcome.output.contains("INPUT SOURCE"));
    assert!(outcome.output.contains("INCLUDED SOURCE"));
    assert!(outcome.pdf.unwrap().ends_with("paper.pdf"));
}

#[test]
#[ignore = "requires latexmk"]
fn selected_chapters_still_compile_after_includeonly_references_are_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("chapters")).unwrap();
    for (name,source) in [
        ("main.tex", "\\documentclass{article}\n\\includeonly{chapters/old}\n\\begin{document}\n\\include{chapters/old}\n\\include{chapters/excluded}\n\\end{document}\n"),
        ("chapters/old.tex", "\\typeout{SELECTED CHAPTER}Selected."),
        ("chapters/excluded.tex", "\\typeout{EXCLUDED CHAPTER}Excluded."),
    ] { std::fs::write(root.join(name),source).unwrap(); }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for renamed in [false, true] {
        let outcome = texquill::compilation::build::run(
            &root.join("main.tex"),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        assert!(outcome.output.contains("SELECTED CHAPTER"));
        assert!(!outcome.output.contains("EXCLUDED CHAPTER"));
        let inputs = outcome.inputs.unwrap();
        assert!(inputs.contains(&uri(
            root,
            if renamed {
                "chapters/new.tex"
            } else {
                "chapters/old.tex"
            }
        )));
        // Modern LaTeX may record existence checks for excluded chapters too;
        // the typeout marker above verifies that their contents did not run.
        if renamed {
            break;
        }
        let files = [rename(root, "chapters/old.tex", "chapters/new.tex")];
        let edit = file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap();
        for (uri, edits) in edits(edit) {
            std::fs::write(
                uri.to_file_path().unwrap(),
                apply(&workspace.documents[&uri].text.source, edits),
            )
            .unwrap();
        }
        std::fs::rename(root.join("chapters/old.tex"), root.join("chapters/new.tex")).unwrap();
        file_operations::did_rename(&mut workspace, &files).unwrap();
    }
}

#[test]
fn recorded_dependencies_follow_file_and_directory_moves() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let main = uri(root, "main.tex");
    let chapter = uri(root, "chapters/old.tex");
    let bibliography = uri(root, "chapters/refs.bib");
    std::fs::write(root.join("chapters/refs.bib"), "@book{example}\n").unwrap();
    workspace
        .project
        .bibliography_inputs
        .insert(main.clone(), [bibliography].into());
    workspace
        .project
        .recorded_inputs
        .insert(main.clone(), [main, chapter].into());
    std::fs::rename(root.join("main.tex"), root.join("paper.tex")).unwrap();
    std::fs::rename(root.join("chapters"), root.join("sections")).unwrap();
    file_operations::did_rename(
        &mut workspace,
        &[
            rename(root, "main.tex", "paper.tex"),
            rename(root, "chapters", "sections"),
        ],
    )
    .unwrap();
    let paper = uri(root, "paper.tex");
    let chapter = uri(root, "sections/old.tex");
    assert_eq!(
        workspace.project.recorded_inputs[&paper],
        [paper.clone(), chapter.clone()].into()
    );
    assert!(workspace.project_documents(&paper).contains(&chapter));
    let bibliography = uri(root, "sections/refs.bib");
    assert_eq!(
        workspace.project.bibliography_inputs[&paper],
        [bibliography.clone()].into()
    );
    assert!(workspace.project_documents(&paper).contains(&bibliography));
}

#[test]
fn batched_file_renames_update_tex_bib_packages_images_and_magic_root_comments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    workspace.client.versioned_edits = true;
    let main = uri(root, "main.tex");
    let source = workspace.documents[&main].text.source.clone();
    workspace
        .documents
        .insert(main.clone(), Document::new(&main, source.clone(), Some(7)));
    let files = [
        rename(root, "main.tex", "paper.tex"),
        rename(root, "chapters/old.tex", "chapters/新章节.tex"),
        rename(root, "refs.bib", "library.bib"),
        rename(root, "pkg.sty", "package.sty"),
        rename(root, "fig.png", "diagram.png"),
    ];
    let edit = file_operations::will_rename(&workspace, &files)
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(documents)) = &edit.document_changes else {
        panic!();
    };
    assert_eq!(
        documents
            .iter()
            .find(|doc| doc.text_document.uri == main)
            .unwrap()
            .text_document
            .version,
        Some(7)
    );
    let edits = edits(edit);
    let changed_main = apply(&source, edits[&main].clone());
    assert!(changed_main.contains("\\input{chapters/新章节}"));
    assert!(changed_main.contains("\\usepackage{package}"));
    assert!(changed_main.contains("\\bibliography{library, other}"));
    assert!(changed_main.contains("\\includegraphics{diagram}"));
    assert!(changed_main.contains("% \\input{chapters/old}"));
    assert!(changed_main.contains("\\verb|\\input{chapters/old}|"));
    let chapter = uri(root, "chapters/old.tex");
    let changed_chapter = apply(
        &workspace.documents[&chapter].text.source,
        edits[&chapter].clone(),
    );
    assert!(changed_chapter.contains("% !TeX root = \"../paper.tex\""));
    assert!(changed_chapter.contains("\\includegraphics{diagram}"));
    assert!(root.join("main.tex").exists());
    assert!(!root.join("paper.tex").exists());
    // Simulate the client applying edits, then moving files and notifying LSP.
    for (uri, edits) in edits {
        let doc = &workspace.documents[&uri];
        let source = apply(&doc.text.source, edits);
        std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
        if let Some(version) = doc.version {
            workspace
                .documents
                .insert(uri.clone(), Document::new(&uri, source, Some(version + 1)));
        }
    }
    for file in &files {
        std::fs::rename(
            Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
            Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
        )
        .unwrap();
    }
    let affected = file_operations::did_rename(&mut workspace, &files).unwrap();
    let paper = uri(root, "paper.tex");
    assert!(affected.contains(&main) && affected.contains(&paper));
    assert!(!workspace.documents.contains_key(&main));
    assert_eq!(workspace.documents[&paper].version, Some(8));
    let chapter = uri(root, "chapters/新章节.tex");
    assert_eq!(workspace.root_document(&chapter).unwrap(), paper);
    assert!(workspace
        .project_documents(&paper)
        .contains(&uri(root, "library.bib")));
}

#[test]
fn file_urls_preserve_percent_encoding_when_assets_are_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    let original = uri(root, "original file.pdf");
    std::fs::write(original.to_file_path().unwrap(), "pdf").unwrap();
    let new = uri(root, "renamed file#1.pdf");
    let main = uri(root, "main.tex");
    let source = format!("\\url{{{original}}}");
    workspace
        .documents
        .insert(main.clone(), Document::new(&main, source.clone(), Some(1)));
    let files = [FileRename {
        old_uri: original.to_string(),
        new_uri: new.to_string(),
    }];
    let edits = edits(
        file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap(),
    );
    let source = apply(&source, edits[&main].clone());
    assert_eq!(source, format!("\\url{{{new}}}"));
    let document = Document::new(&main, source, Some(2));
    assert!(texquill::language::features::diagnostics(&document).is_empty());
}

#[test]
fn file_url_renames_preserve_source_queries_fragments_and_tex_escapes() {
    for directory_move in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("assets")).unwrap();
        let old = uri(root, "assets/original file#?.pdf");
        std::fs::write(old.to_file_path().unwrap(), "pdf").unwrap();
        let new = uri(
            root,
            if directory_move {
                "新 assets/original file#?.pdf"
            } else {
                "assets/新 file#?.pdf"
            },
        );
        let mut source = String::new();
        let mut expected = String::new();
        let tails = [
            "",
            "?download=1",
            "#page=3",
            "?download=1#section.2",
            "?",
            "#",
            "?#",
            "?q=some%20text%23%3F#part%202",
            r"?q=some\%20text\&mode=1\#page=3",
            r"\#page=3",
        ];
        for tail in tails {
            for (command, rest) in [
                ("url", ""),
                ("href", "{Visible text}"),
                ("hyperref", "{chapter}{intro}{Visible text}"),
            ] {
                source.push_str(&format!("\\{command}{{{old}{tail}}}{rest}\n"));
                expected.push_str(&format!("\\{command}{{{new}{tail}}}{rest}\n"));
            }
        }
        let protected = format!(
            "% \\href{{{old}#page=3}}{{Comment}}\n\\verb|\\url{{{old}#page=3}}|\n\
             \\href{{https://example.test/manual.pdf?download=1#page=3}}{{Remote}}\n\
             Same text: {old}\n"
        );
        source.push_str(&protected);
        expected.push_str(&protected);
        let main = uri(root, "main.tex");
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        workspace.open(main.clone(), source.clone(), 7);
        let file = if directory_move {
            rename(root, "assets", "新 assets")
        } else {
            FileRename {
                old_uri: old.to_string(),
                new_uri: new.to_string(),
            }
        };
        let changes = edits(
            file_operations::will_rename(&workspace, &[file])
                .unwrap()
                .unwrap(),
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[&main].len(), tails.len() * 3);
        assert_eq!(apply(&source, changes[&main].clone()), expected);
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn real_file_url_renames_preserve_pdf_link_destinations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let old = uri(root, "old manual.pdf");
    let new = uri(root, "new manual#1.pdf");
    std::fs::write(old.to_file_path().unwrap(), "pdf").unwrap();
    let main = uri(root, "main.tex");
    let source = format!(
        "\\pdfobjcompresslevel=0\\pdfcompresslevel=0\n\
         \\documentclass{{article}}\\usepackage{{hyperref}}\\begin{{document}}\n\
         \\url{{{old}?download=1#page=3}}\n\
         \\href{{{old}?download=1#page=4}}{{Manual}}\n\
         \\hyperref{{{old}?download=1}}{{chapter}}{{intro}}{{Chapter}}\n\
         \\end{{document}}\n"
    );
    std::fs::write(root.join("main.tex"), &source).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let config = texquill::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    for target in [&old, &new] {
        let outcome =
            texquill::compilation::build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        let data =
            std::fs::read(config.artifact_path(&root.join("main.tex"), "pdf").unwrap()).unwrap();
        let pdf = String::from_utf8_lossy(&data);
        let mut targets: Vec<_> = pdf
            .split("/URI")
            .skip(1)
            .filter_map(|tail| {
                tail.trim_start()
                    .strip_prefix('(')?
                    .split_once(')')
                    .map(|(uri, _)| uri)
            })
            .collect();
        // A URL that wraps across lines has one annotation per line.
        targets.dedup();
        assert_eq!(
            targets,
            [
                format!("{target}?download=1#page=3"),
                format!("{target}?download=1#page=4"),
                format!("{target}?download=1#chapter.intro"),
            ]
        );
        if target == &old {
            let files = [FileRename {
                old_uri: old.to_string(),
                new_uri: new.to_string(),
            }];
            let changes = edits(
                file_operations::will_rename(&workspace, &files)
                    .unwrap()
                    .unwrap(),
            );
            std::fs::write(
                root.join("main.tex"),
                apply(&source, changes[&main].clone()),
            )
            .unwrap();
            std::fs::rename(old.to_file_path().unwrap(), new.to_file_path().unwrap()).unwrap();
            file_operations::did_rename(&mut workspace, &files).unwrap();
        }
    }
}

#[test]
fn directory_moves_rebase_references_inside_moved_documents() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    let files = [rename(root, "chapters", "parts/content")];
    let edits = edits(
        file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap(),
    );
    let main = uri(root, "main.tex");
    let chapter = uri(root, "chapters/old.tex");
    let changed_main = apply(
        &workspace.documents[&main].text.source,
        edits[&main].clone(),
    );
    assert!(changed_main.contains("\\input{parts/content/old}"));
    let changed = apply(
        &workspace.documents[&chapter].text.source,
        edits[&chapter].clone(),
    );
    assert!(changed.contains("../../main.tex"));
    assert!(changed.contains("\\input{../../shared.v1}"));
    // The image was relative to the project root, which remains unchanged.
    assert!(changed.contains("\\includegraphics{fig}"));
}

#[test]
fn moving_an_entire_project_preserves_references_and_updates_workspace_roots() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("old");
    let new = directory.path().join("new");
    std::fs::create_dir(&root).unwrap();
    let mut workspace = project(&root);
    let files = [FileRename {
        old_uri: Url::from_file_path(&root).unwrap().to_string(),
        new_uri: Url::from_file_path(&new).unwrap().to_string(),
    }];
    assert!(file_operations::will_rename(&workspace, &files)
        .unwrap()
        .is_none());
    std::fs::rename(&root, &new).unwrap();
    file_operations::did_rename(&mut workspace, &files).unwrap();
    assert!(workspace.roots.contains(&new));
    assert!(!workspace.roots.contains(&root));
    assert_eq!(workspace.documents.len(), 7);
    let main = uri(&new, "main.tex");
    assert_eq!(
        workspace
            .root_document(&uri(&new, "chapters/old.tex"))
            .unwrap(),
        main
    );
}

#[test]
#[ignore = "requires latexmk, BibTeX and a TeX installation"]
fn renamed_projects_still_compile_after_root_and_dependency_moves() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("chapters")).unwrap();
    for (name,source) in [
        ("main.tex", "\\documentclass{article}\n\\usepackage{mypackage}\n\\begin{document}\n\\input{chapters/chapter}\n\\cite{example}\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}\n"),
        ("chapters/chapter.tex", "% !TeX root = ../main.tex\n\\input{shared}\n\\hello\n"),
        ("shared.tex", "Shared text."),
        ("mypackage.sty", "\\newcommand{\\hello}{Hello}"),
        ("references.bib", "@book{example, author={A}, title={Example}, year={2025}, publisher={P}}"),
    ] { std::fs::write(root.join(name),source).unwrap(); }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let files = [
        rename(root, "main.tex", "book/paper.tex"),
        rename(root, "chapters", "parts/content"),
        rename(root, "mypackage.sty", "renamedpackage.sty"),
        rename(root, "references.bib", "library.bib"),
    ];
    let edits = edits(
        file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap(),
    );
    for (uri, edits) in edits {
        let source = apply(&workspace.documents[&uri].text.source, edits);
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    std::fs::create_dir(root.join("book")).unwrap();
    std::fs::create_dir(root.join("parts")).unwrap();
    for file in &files {
        std::fs::rename(
            Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
            Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
        )
        .unwrap();
    }
    file_operations::did_rename(&mut workspace, &files).unwrap();
    let main = uri(root, "book/paper.tex");
    assert_eq!(
        workspace
            .root_document(&uri(root, "parts/content/chapter.tex"))
            .unwrap(),
        main
    );
    let outcome = texquill::compilation::build::run(
        &main.to_file_path().unwrap(),
        &Default::default(),
        &std::sync::atomic::AtomicBool::new(false),
    );
    assert_eq!(
        outcome.status, "success",
        "{}\n{}\n{:?}",
        outcome.message, outcome.output, outcome.diagnostics
    );
    assert!(outcome.pdf.unwrap().is_file());
}

#[test]
fn invalid_batches_and_unrepresentable_references_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = project(root);
    assert!(
        file_operations::will_rename(&workspace, &[rename(root, "pkg.sty", "bad#name.sty")])
            .is_err()
    );
    assert!(file_operations::will_rename(
        &workspace,
        &[
            rename(root, "refs.bib", "new.bib"),
            rename(root, "other.bib", "new.bib")
        ]
    )
    .is_err());
    let remote = FileRename {
        old_uri: "https://example.com/main.tex".into(),
        new_uri: "file:///new.tex".into(),
    };
    assert!(file_operations::will_rename(&workspace, &[remote]).is_err());
    assert!(file_operations::will_rename(&workspace, &[])
        .unwrap()
        .is_none());
}

fn shared_inputs_project(root: &Path) -> Workspace {
    std::fs::write(
        root.join("shared.tex"),
        "\\input{setup}\n\\includegraphics[width=1cm]{plot}\n",
    )
    .unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::create_dir(root.join(format!("{name}/book"))).unwrap();
        std::fs::write(root.join(format!("{name}/main.tex")),"\\documentclass{article}\n\\usepackage{graphicx}\n\\begin{document}\n\\input{../shared}\n\\end{document}\n").unwrap();
        std::fs::write(
            root.join(format!("{name}/setup.tex")),
            "Project settings.\n",
        )
        .unwrap();
        std::fs::write(root.join(format!("{name}/plot.pdf")), "pdf").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    workspace
}

#[test]
fn shared_file_links_do_not_choose_arbitrary_targets_and_build_inputs_use_the_calling_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = shared_inputs_project(root);
    let shared = uri(root, "shared.tex");
    assert!(texquill::language::navigation::document_links(&workspace, &shared).is_empty());
    for (name, other) in [("a", "b"), ("b", "a")] {
        let inputs = workspace.project_files(&uri(root, &format!("{name}/main.tex")));
        assert!(inputs.contains(&uri(root, &format!("{name}/plot.pdf"))));
        assert!(!inputs.contains(&uri(root, &format!("{other}/plot.pdf"))));
    }
    let source = format!(
        "{}\\url{{https://example.test/}}",
        workspace.documents[&shared].text.source
    );
    workspace
        .documents
        .insert(shared.clone(), Document::new(&shared, source, Some(1)));
    let links = texquill::language::navigation::document_links(&workspace, &shared);
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].target.as_ref().unwrap().as_str(),
        "https://example.test/"
    );
    let doc = &workspace.documents[&shared];
    let position = doc
        .text
        .position(doc.text.source.find("setup").unwrap() + 3);
    let items = texquill::language::features::completion(&workspace, &shared, position);
    assert!(items.iter().any(|item| item.label == "setup.tex"));
}

#[test]
fn shared_input_and_image_file_edits_require_agreement_from_all_consumers() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = shared_inputs_project(root);
    let shared = uri(root, "shared.tex");
    for files in [
        vec![rename(root, "a/setup.tex", "a/config.tex")],
        vec![rename(root, "a/plot.pdf", "a/chart.pdf")],
        vec![rename(root, "a/main.tex", "a/book/main.tex")],
    ] {
        assert!(file_operations::will_rename(&workspace, &files)
            .unwrap_err()
            .message
            .contains("different replacements"));
    }
    let files = [
        rename(root, "a/setup.tex", "a/config.tex"),
        rename(root, "b/setup.tex", "b/config.tex"),
    ];
    let changes = edits(
        file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap(),
    );
    assert_eq!(changes[&shared].len(), 1);
    assert_eq!(
        apply(
            &workspace.documents[&shared].text.source,
            changes[&shared].clone()
        ),
        "\\input{config}\n\\includegraphics[width=1cm]{plot}\n"
    );
    let files = [
        rename(root, "a/plot.pdf", "a/chart.pdf"),
        rename(root, "b/plot.pdf", "b/chart.pdf"),
    ];
    let changes = edits(
        file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap(),
    );
    assert_eq!(changes[&shared].len(), 1);
    assert!(apply(
        &workspace.documents[&shared].text.source,
        changes[&shared].clone()
    )
    .contains("{chart}"));
}

#[test]
fn shared_file_reference_missing_in_one_project_cannot_be_rewritten_for_another() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let workspace = shared_inputs_project(root);
    std::fs::remove_file(root.join("b/setup.tex")).unwrap();
    let error =
        file_operations::will_rename(&workspace, &[rename(root, "a/setup.tex", "a/config.tex")])
            .unwrap_err();
    assert!(error.message.contains("consistently"), "{error}");
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn both_projects_compile_after_shared_input_and_image_paths_are_rebased() {
    let image = tempfile::tempdir().unwrap();
    std::fs::write(
        image.path().join("image.tex"),
        "\\documentclass{article}\\begin{document}Image\\end{document}",
    )
    .unwrap();
    let image_build = texquill::compilation::build::run(
        &image.path().join("image.tex"),
        &Default::default(),
        &Default::default(),
    );
    assert_eq!(image_build.status, "success", "{}", image_build.output);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = shared_inputs_project(root);
    for name in ["a", "b"] {
        std::fs::copy(
            image_build.pdf.as_ref().unwrap(),
            root.join(format!("{name}/plot.pdf")),
        )
        .unwrap();
    }
    let files = [
        rename(root, "a/main.tex", "a/book/main.tex"),
        rename(root, "b/main.tex", "b/book/main.tex"),
        rename(root, "a/setup.tex", "a/config.tex"),
        rename(root, "b/setup.tex", "b/config.tex"),
    ];
    let changes = edits(
        file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap(),
    );
    let shared = uri(root, "shared.tex");
    assert_eq!(changes[&shared].len(), 2);
    for (uri, edits) in changes {
        let source = apply(&workspace.documents[&uri].text.source, edits);
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    for file in &files {
        std::fs::rename(
            Url::parse(&file.old_uri).unwrap().to_file_path().unwrap(),
            Url::parse(&file.new_uri).unwrap().to_file_path().unwrap(),
        )
        .unwrap();
    }
    file_operations::did_rename(&mut workspace, &files).unwrap();
    assert_eq!(
        workspace.documents[&shared].text.source,
        "\\input{../config}\n\\includegraphics[width=1cm]{../plot}\n"
    );
    for name in ["a", "b"] {
        let outcome = texquill::compilation::build::run(
            &root.join(format!("{name}/book/main.tex")),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(
            outcome.status, "success",
            "{}\n{}",
            outcome.message, outcome.output
        );
        assert!(outcome.pdf.unwrap().is_file());
    }
}

#[test]
fn package_and_class_links_always_append_the_loader_suffix() {
    for (command, extension) in [
        ("usepackage", "sty"),
        ("RequirePackage", "sty"),
        ("documentclass", "cls"),
        ("LoadClass", "cls"),
    ] {
        for name in [
            "custom".to_owned(),
            "custom.inc".to_owned(),
            format!("custom.{extension}"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            std::fs::write(root.join(&name), "Wrong literal file.").unwrap();
            let expected = format!("{name}.{extension}");
            std::fs::write(root.join(&expected), "Expected loaded source.").unwrap();
            let source = if command == "documentclass" {
                format!("\\documentclass{{{name}}}")
            } else {
                format!("\\documentclass{{article}}\\{command}{{{name}}}")
            };
            std::fs::write(root.join("main.tex"), source).unwrap();
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let main = uri(root, "main.tex");
            let links = texquill::language::navigation::document_links(&workspace, &main);
            assert_eq!(links.len(), 1, "{command}/{name}: {links:?}");
            assert_eq!(
                links[0].target,
                Some(uri(root, &expected)),
                "{command}/{name}"
            );
            let dependencies = workspace.project_files(&main);
            assert!(dependencies.contains(&uri(root, &expected)));
            assert!(!dependencies.contains(&uri(root, &name)));
            std::fs::remove_file(root.join(&expected)).unwrap();
            assert!(texquill::language::navigation::document_links(&workspace, &main).is_empty());
        }
    }
}

#[test]
fn loader_renames_strip_the_appended_suffix_and_completion_avoids_doubled_extensions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = project(root);
    for name in ["local.cls.cls", "pkg.sty.sty"] {
        std::fs::write(root.join(name), "Expected loaded source.").unwrap();
    }
    let main = uri(root, "main.tex");
    let source = r"\documentclass{local.cls}\usepackage{pkg.sty}";
    workspace.open(main.clone(), source.into(), 1);
    let changes = edits(
        file_operations::will_rename(
            &workspace,
            &[
                rename(root, "local.cls.cls", "new-class.cls"),
                rename(root, "pkg.sty.sty", "new-package.sty"),
            ],
        )
        .unwrap()
        .unwrap(),
    );
    assert_eq!(
        apply(source, changes[&main].clone()),
        r"\documentclass{new-class}\usepackage{new-package}"
    );
    let source = r"\usepackage{pkg.st";
    workspace.open(main.clone(), source.into(), 2);
    let items = texquill::language::features::completion(
        &workspace,
        &main,
        Text::new(source.into()).position(source.len()),
    );
    let item = items.iter().find(|item| item.label == "pkg.sty").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!()
    };
    assert_eq!(edit.new_text, "pkg");
}

#[test]
#[ignore = "requires pdflatex"]
fn actual_package_and_class_loading_matches_navigation_and_renamed_base_names() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (name, source) in [
        (
            "main.tex",
            r"\documentclass{local.cls}\usepackage{pkg.sty}\begin{document}Text.\end{document}",
        ),
        ("local.cls", r"\errmessage{WRONG CLASS}"),
        (
            "local.cls.cls",
            r"\LoadClass{article}\typeout{EXPECTED CLASS}",
        ),
        ("pkg.sty", r"\errmessage{WRONG PACKAGE}"),
        ("pkg.sty.sty", r"\typeout{EXPECTED PACKAGE}"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let config = texquill::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let main = uri(root, "main.tex");
    for renamed in [false, true] {
        let outcome =
            texquill::compilation::build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        assert!(
            outcome.output.contains("EXPECTED CLASS"),
            "{}",
            outcome.output
        );
        assert!(
            outcome.output.contains("EXPECTED PACKAGE"),
            "{}",
            outcome.output
        );
        let inputs = outcome.inputs.unwrap();
        let links = texquill::language::navigation::document_links(&workspace, &main);
        for name in if renamed {
            ["new-class.cls", "new-package.sty"]
        } else {
            ["local.cls.cls", "pkg.sty.sty"]
        } {
            assert!(inputs.contains(&uri(root, name)));
            assert!(links
                .iter()
                .any(|link| link.target == Some(uri(root, name))));
        }
        if renamed {
            break;
        }
        let files = [
            rename(root, "local.cls.cls", "new-class.cls"),
            rename(root, "pkg.sty.sty", "new-package.sty"),
        ];
        let changes = edits(
            file_operations::will_rename(&workspace, &files)
                .unwrap()
                .unwrap(),
        );
        std::fs::write(
            root.join("main.tex"),
            apply(
                &workspace.documents[&main].text.source,
                changes[&main].clone(),
            ),
        )
        .unwrap();
        std::fs::rename(root.join("local.cls.cls"), root.join("new-class.cls")).unwrap();
        std::fs::rename(root.join("pkg.sty.sty"), root.join("new-package.sty")).unwrap();
        file_operations::did_rename(&mut workspace, &files).unwrap();
    }
}

#[test]
fn incoming_root_files_do_not_redirect_package_class_style_or_database_fallbacks() {
    for (command, extension, expected) in [
        ("RequirePackage", "sty", "chapters/item"),
        ("LoadClass", "cls", "chapters/item"),
        ("bibliographystyle", "bst", "chapters/item.bst"),
        ("bibliography", "bib", "chapters/item.bib"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut workspace = project(root);
        std::fs::write(root.join(format!("chapters/item.{extension}")), "Expected.").unwrap();
        std::fs::write(root.join(format!("other.{extension}")), "New shadow.").unwrap();
        let child = uri(root, "chapters/old.tex");
        let source = format!("\\{command}{{item}}");
        workspace.open(child.clone(), source.clone(), 1);
        let files = [rename(
            root,
            &format!("other.{extension}"),
            &format!("item.{extension}"),
        )];
        let edit = file_operations::will_rename(&workspace, &files)
            .unwrap()
            .unwrap();
        let changes = edits(edit);
        assert_eq!(
            apply(&source, changes[&child].clone()),
            format!("\\{command}{{{expected}}}"),
            "{command}"
        );
    }
}
