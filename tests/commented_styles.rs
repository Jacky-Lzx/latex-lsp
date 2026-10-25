use latex_lsp::{build, features, file_operations, navigation, text::Text, workspace::Workspace};
use lsp_types::*;
use std::path::Path;

const STYLE: &str = r#"ENTRY { title } {} {}
FUNCTION {book} { "\bibitem{" write$ cite$ write$ "}" write$ newline$ title write$ newline$ }
READ
FUNCTION {begin.bib} { "\begin{thebibliography}{9}" write$ newline$ }
EXECUTE {begin.bib}
ITERATE {call.type$}
FUNCTION {end.bib} { "\end{thebibliography}" write$ newline$ }
EXECUTE {end.bib}
"#;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn rename(root: &Path, old: &str, new: &str) -> FileRename {
    FileRename {
        old_uri: uri(root, old).to_string(),
        new_uri: uri(root, new).to_string(),
    }
}

fn apply(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
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

#[test]
fn commented_style_names_preserve_single_comma_filenames_and_edit_only_the_real_path() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("project")).unwrap();
    std::fs::create_dir(root.join("shared")).unwrap();
    for name in ["local,part", "local", "part", "phantom"] {
        std::fs::write(root.join(format!("shared/{name}.bst")), STYLE).unwrap();
    }
    for suffix in ["", ".bst"] {
        let source = format!("\\bibliographystyle{{% emoji😀 ../shared/phantom, }} ignored\r\n  ../shared/local,part{suffix}% tail\r\n  % footer\r\n\t}}\r\n\\verb|\\bibliographystyle{{../shared/local,part{suffix}}}|\r\n");
        std::fs::write(root.join("project/main.tex"), &source).unwrap();
        let main = uri(root, "project/main.tex");
        let target = uri(root, "shared/local,part.bst");
        let mut workspace = Workspace::default();
        workspace.add_root(root.join("project"));
        workspace.open(main.clone(), source.clone(), 7);
        let files = workspace.project_files(&main);
        assert!(files.contains(&target));
        for name in ["local", "part", "phantom"] {
            assert!(!files.contains(&uri(root, &format!("shared/{name}.bst"))));
        }
        let links = navigation::document_links(&workspace, &main);
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(links[0].target, Some(target.clone()));
        let start = source.find("  ../shared/local,part").unwrap() + 2;
        let text = Text::new(source.clone());
        assert_eq!(
            links[0].range,
            text.range(start, start + "../shared/local,part".len() + suffix.len())
        );
        assert_eq!(
            features::definitions(&workspace, &main, text.position(start + 17))[0].uri,
            target
        );
        assert!(file_operations::will_rename(
            &workspace,
            &[rename(root, "shared/phantom.bst", "shared/unused.bst")]
        )
        .unwrap()
        .is_none());
        assert!(file_operations::will_rename(
            &workspace,
            &[rename(root, "shared/local,part.bst", "shared/other.txt")]
        )
        .is_err());
        for (old, new, from, to) in [
            (
                "shared/local,part.bst",
                "shared/new,style.bst",
                "\n  ../shared/local,part",
                "\n  ../shared/new,style",
            ),
            (
                "shared",
                "styles",
                "\n  ../shared/local,part",
                "\n  ../styles/local,part",
            ),
        ] {
            let changes = file_operations::will_rename(&workspace, &[rename(root, old, new)])
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[&main].len(), 1);
            assert_eq!(apply(&source, &changes[&main]), source.replace(from, to));
        }
    }
}

#[test]
fn style_completion_uses_the_entire_single_name_and_accepts_only_tex_ignored_indentation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("local,part.bst"), STYLE).unwrap();
    std::fs::write(root.join("local, part.bst"), STYLE).unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for (marked, expected) in [
        (
            r"\bibliographystyle{local,pa¦BAD}",
            r"\bibliographystyle{local,part}",
        ),
        (
            r"\bibliographystyle{local,pa¦BAD.bst}",
            r"\bibliographystyle{local,part.bst}",
        ),
        (
            "\\bibliographystyle{% before😀\r\n  local,pa¦BAD% tail\r\n  }",
            "\\bibliographystyle{% before😀\r\n  local,part% tail\r\n  }",
        ),
        (
            "\\bibliographystyle{% before\n\tlocal,pa¦",
            "\\bibliographystyle{% before\n\tlocal,part",
        ),
        (r"\bibliographystyle{¦}", r"\bibliographystyle{local,part}"),
        (
            "\\bibliographystyle{% head\n  ¦% tail\n}",
            "\\bibliographystyle{% head\n  local,part% tail\n}",
        ),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(main.clone(), source.clone(), 7);
        let items = features::completion(
            &workspace,
            &main,
            Text::new(source.clone()).position(offset),
        );
        assert!(!items.iter().any(|item| item.label == "local, part.bst"));
        let item = items
            .into_iter()
            .find(|item| item.label == "local,part.bst")
            .unwrap_or_else(|| panic!("{marked}"));
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing style completion edit")
        };
        let mut edits = item.additional_text_edits.unwrap_or_default();
        edits.push(edit);
        assert_eq!(apply(&source, &edits), expected, "{marked}");
    }
}

#[test]
fn visible_whitespace_blank_lines_dynamic_and_spliced_style_names_do_not_create_references() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("local,part.bst"), STYLE).unwrap();
    let main = uri(root, "main.tex");
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for arguments in [
        "{ local,pa¦rt}",
        "{local,pa¦rt }",
        "{\nlocal,pa¦rt}",
        "{local,pa¦rt\n}",
        "{ % head\n local,pa¦rt% tail\n}",
        "{% head\n local,pa¦rt % tail\n}",
        "{% head\n\nlocal,pa¦rt% tail\n}",
        "{local,pa¦rt% tail\n\n}",
        "{local,pa¦% joined\nrt}",
        r"{local,pa¦rt\suffix}",
        r"{{local,pa¦rt}}",
        r"*{local,pa¦rt}",
        r"[note]{local,pa¦rt}",
        "{% comment local,pa¦rt\n}",
    ] {
        let marked = format!("\\bibliographystyle{arguments}");
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        std::fs::write(root.join("main.tex"), &source).unwrap();
        workspace.open(main.clone(), source.clone(), 7);
        assert!(
            navigation::document_links(&workspace, &main).is_empty(),
            "{marked}"
        );
        assert!(
            !features::completion(&workspace, &main, Text::new(source).position(offset))
                .iter()
                .any(|item| item.label == "local,part.bst"),
            "{marked}"
        );
        assert!(
            file_operations::will_rename(
                &workspace,
                &[rename(root, "local,part.bst", "other.bst")]
            )
            .unwrap()
            .is_none(),
            "{marked}"
        );
    }
}

#[test]
#[ignore = "requires latexmk, pdflatex, xelatex and BibTeX"]
fn commented_comma_style_names_keep_real_bibtex_inputs_after_file_and_directory_renames() {
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("styles")).unwrap();
        std::fs::write(root.join("styles/local,part.bst"), STYLE).unwrap();
        std::fs::write(
            root.join("references.bib"),
            "@book{example,title={Expected}}\n",
        )
        .unwrap();
        let mut source = "\\documentclass{article}\r\n\\begin{document}\r\n\\cite{example}\r\n\\bibliographystyle{% keep styles/phantom\r\n  styles/local,part% tail\r\n  }\r\n\\bibliography{references}\r\n\\end{document}\r\n".to_string();
        let main = uri(root, "main.tex");
        let config = latex_lsp::config::BuildConfig {
            engine: latex_lsp::config::Engine::from_program(engine),
            ..Default::default()
        };
        let mut workspace = Workspace::default();
        for (old, new) in [
            ("styles/local,part.bst", "styles/new,style.bst"),
            ("styles", "papers"),
        ] {
            std::fs::write(root.join("main.tex"), &source).unwrap();
            workspace.add_root(root.into());
            workspace.open(main.clone(), source.clone(), 7);
            let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            let current = if old.ends_with(".bst") {
                old
            } else {
                "styles/new,style.bst"
            };
            assert!(
                outcome
                    .bibliography_inputs
                    .as_ref()
                    .is_some_and(|inputs| inputs.contains(&uri(root, current))),
                "{engine}: {}",
                outcome.output
            );
            let change = rename(root, old, new);
            let edits = file_operations::will_rename(&workspace, std::slice::from_ref(&change))
                .unwrap()
                .unwrap()
                .changes
                .unwrap();
            source = apply(&source, &edits[&main]);
            std::fs::rename(root.join(old), root.join(new)).unwrap();
            file_operations::did_rename(&mut workspace, &[change]).unwrap();
        }
        std::fs::write(root.join("main.tex"), &source).unwrap();
        let outcome = build::run(&root.join("main.tex"), &config, &Default::default());
        assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
        assert!(outcome
            .bibliography_inputs
            .unwrap()
            .contains(&uri(root, "papers/new,style.bst")));
    }
}
