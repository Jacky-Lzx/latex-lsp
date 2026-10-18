use latex_lsp::{
    features,
    text::Text,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

fn insert(workspace: &mut Workspace, name: &str, text: &str, version: Option<i32>) -> Url {
    let uri = Url::parse(&format!("file:///rename-test/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, text.into(), version));
    uri
}

fn edits_result(source: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let changes = edits
        .into_iter()
        .map(|edit| TextDocumentContentChangeEvent {
            range: Some(edit.range),
            range_length: None,
            text: edit.new_text,
        })
        .collect();
    Text::new(source.into()).changed(changes).unwrap().source
}

#[test]
fn renames_labels_in_multiple_files_but_not_comments_or_verbatim() {
    let mut workspace = Workspace::default();
    let declaration = insert(&mut workspace, "chapter.tex", "\\label{sec:old}", None);
    let source =
        "😀 \\ref{sec:old} \\cref{other, sec:old}\n% \\ref{sec:old}\n\\verb|\\ref{sec:old}|";
    let main = insert(&mut workspace, "main.tex", source, Some(7));
    let edit = features::rename(&workspace, &main, Position::new(0, 10), "sec:中文")
        .unwrap()
        .unwrap();
    let changes = edit.changes.unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(
        edits_result(
            &workspace.documents[&declaration].text.source,
            &changes[&declaration]
        ),
        r"\label{sec:中文}"
    );
    assert_eq!(
        edits_result(source, &changes[&main]),
        "😀 \\ref{sec:中文} \\cref{other, sec:中文}\n% \\ref{sec:old}\n\\verb|\\ref{sec:old}|"
    );
}

#[test]
fn citation_rename_updates_bibtex_keys_and_only_matching_citations() {
    let mut workspace = Workspace::default();
    let bib = insert(
        &mut workspace,
        "refs.bib",
        "@book{old, title={old}}\n@article{other, title={Old}}",
        None,
    );
    let main = insert(
        &mut workspace,
        "main.tex",
        r"\cite[see]{old, other}\citep{old}",
        Some(1),
    );
    let edit = features::rename(&workspace, &bib, Position::new(0, 8), "new2026")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits_result(&workspace.documents[&bib].text.source, &edit[&bib]),
        "@book{new2026, title={old}}\n@article{other, title={Old}}"
    );
    assert_eq!(
        edits_result(&workspace.documents[&main].text.source, &edit[&main]),
        r"\cite[see]{new2026, other}\citep{new2026}"
    );
}

#[test]
fn macro_rename_preserves_backslashes_and_carries_document_versions() {
    let mut workspace = Workspace {
        versioned_edits: true,
        ..Default::default()
    };
    let source = r"\newcommand{\foo}[1]{#1} \foo{中文}";
    let uri = insert(&mut workspace, "main.tex", source, Some(12));
    let Some(PrepareRenameResponse::RangeWithPlaceholder { range, placeholder }) =
        features::prepare_rename(&workspace, &uri, Position::new(0, 14)).unwrap()
    else {
        panic!("expected rename target");
    };
    assert_eq!(placeholder, "foo");
    assert_eq!(range.start.character, 13);
    let edit = features::rename(&workspace, &uri, Position::new(0, 14), "bar")
        .unwrap()
        .unwrap();
    let Some(DocumentChanges::Edits(documents)) = edit.document_changes else {
        panic!("expected versioned edit");
    };
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].text_document.version, Some(12));
    let edits = documents[0]
        .edits
        .iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit.clone(),
            _ => panic!(),
        })
        .collect::<Vec<_>>();
    assert_eq!(edits.len(), 2);
    assert_eq!(
        edits_result(source, &edits),
        r"\newcommand{\bar}[1]{#1} \bar{中文}"
    );
}

#[test]
#[ignore = "requires pdflatex and a current LaTeX installation"]
fn renamed_custom_environments_still_compile() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.tex");
    let uri = Url::from_file_path(&path).unwrap();
    let source = "\\documentclass{article}\n\\NewDocumentEnvironment{custombox}{m}{\\begin{quote}#1}{\\end{quote}}\n\\begin{document}\n\\begin{custombox}{Hello}\nWorld\n\\end{custombox}\n\\end{document}\n";
    let mut workspace = Workspace::default();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    let position = Text::new(source.into()).position(source.find("custombox").unwrap() + 2);
    let edits = features::rename(&workspace, &uri, position, "remarkbox")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(edits[&uri].len(), 3);
    std::fs::write(&path, edits_result(source, &edits[&uri])).unwrap();
    let config = latex_lsp::config::BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = latex_lsp::build::run(&path, &config, &std::sync::atomic::AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
}

#[test]
fn custom_environment_rename_updates_declarations_begin_and_end_across_files() {
    let mut workspace = Workspace::default();
    let declaration = insert(
        &mut workspace,
        "environments.tex",
        r"\NewDocumentEnvironment{custom}{O{x}}{}{}",
        Some(2),
    );
    let source =
        "😀 \\begin{custom}\nText\n\\end{custom}\n% \\begin{custom}\n\\verb|\\end{custom}|";
    let main = insert(&mut workspace, "main.tex", source, Some(5));
    let position = Text::new(source.into()).position(source.find("custom").unwrap() + 2);
    let Some(PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. }) =
        features::prepare_rename(&workspace, &main, position).unwrap()
    else {
        panic!();
    };
    assert_eq!(placeholder, "custom");
    assert_eq!(
        features::definitions(&workspace, &main, position)[0].uri,
        declaration
    );
    assert_eq!(
        features::references(&workspace, &main, position, false).len(),
        2
    );
    assert_eq!(
        features::references(&workspace, &main, position, true).len(),
        3
    );
    assert!(features::hover(&workspace, &main, position).is_some());
    let symbols = features::symbols(&workspace.documents[&declaration]);
    assert_eq!(symbols[0].name, "custom");
    assert_eq!(symbols[0].kind, SymbolKind::CLASS);
    let edits = features::rename(&workspace, &main, position, "renamed*")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        edits_result(
            &workspace.documents[&declaration].text.source,
            &edits[&declaration]
        ),
        r"\NewDocumentEnvironment{renamed*}{O{x}}{}{}"
    );
    assert_eq!(
        edits_result(source, &edits[&main]),
        "😀 \\begin{renamed*}\nText\n\\end{renamed*}\n% \\begin{custom}\n\\verb|\\end{custom}|"
    );
    for name in ["itemize", "section", "bad name"] {
        assert!(features::rename(&workspace, &main, position, name).is_err());
    }
    let macros = insert(
        &mut workspace,
        "macros.tex",
        r"\newcommand{\taken}{X}\newcommand{\endoccupied}{Y}",
        None,
    );
    assert!(features::rename(&workspace, &main, position, "taken").is_err());
    assert!(features::rename(&workspace, &main, position, "occupied").is_err());
    let macro_position = Text::new(workspace.documents[&macros].text.source.clone()).position(15);
    assert!(features::rename(&workspace, &macros, macro_position, "custom").is_err());
    insert(
        &mut workspace,
        "duplicate.tex",
        r"\newenvironment{custom}{}{}",
        None,
    );
    assert!(features::prepare_rename(&workspace, &main, position).is_err());
}

#[test]
fn environment_declaration_variants_are_indexed_and_complete() {
    for command in [
        "newenvironment",
        "renewenvironment",
        "provideenvironment",
        "NewDocumentEnvironment",
        "RenewDocumentEnvironment",
        "ProvideDocumentEnvironment",
        "DeclareDocumentEnvironment",
        "NewEnviron",
        "RenewEnviron",
        "newtheorem",
    ] {
        let mut workspace = Workspace::default();
        let declaration = insert(
            &mut workspace,
            "env.tex",
            &format!("\\{command}{{special}}{{}}{{}}"),
            Some(1),
        );
        let main = insert(&mut workspace, "main.tex", r"\begin{spe", Some(1));
        assert_eq!(
            workspace.documents[&declaration].syntax.environments.len(),
            1,
            "{command}"
        );
        assert!(
            features::completion(&workspace, &main, Position::new(0, 10))
                .iter()
                .any(|item| item.label == "special"),
            "{command}"
        );
        let edit = features::rename(
            &workspace,
            &declaration,
            Position::new(0, command.len() as u32 + 3),
            "different",
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
        assert!(edits_result(
            &workspace.documents[&declaration].text.source,
            &edit[&declaration]
        )
        .contains("{different}"));
    }
    let mut workspace = Workspace::default();
    let uri = insert(
        &mut workspace,
        "main.tex",
        r"\begin{itemize}\end{itemize}",
        Some(1),
    );
    assert!(
        features::prepare_rename(&workspace, &uri, Position::new(0, 9))
            .unwrap()
            .is_none()
    );
}

#[test]
fn modern_and_unbraced_macro_declarations_support_navigation_completion_and_rename() {
    for declaration in [
        r"\newcommand\hello[1]{Hello #1}",
        r"\renewcommand* \hello [1] {Hello #1}",
        "\\providecommand % declaration\n\\hello[1]{Hello #1}",
        r"\DeclareMathOperator*{\hello}{hello}",
        r"\DeclareRobustCommand\hello[1]{#1}",
        r"\NewDocumentCommand{\hello}{m}{#1}",
        r"\RenewDocumentCommand\hello{m}{#1}",
        r"\ProvideDocumentCommand{\hello}{O{x}}{#1}",
        r"\DeclareDocumentCommand{\hello}{m}{#1}",
        r"\def\hello#1{Hello #1}",
        r"\gdef\hello#1{Hello #1}",
        r"\edef\hello{Hello}",
        r"\xdef\hello{Hello}",
        r"\let\hello=\textbf",
        r"\long\def\hello#1{Hello #1}",
    ] {
        let mut workspace = Workspace::default();
        let macros = insert(&mut workspace, "macros.tex", declaration, Some(1));
        let main = insert(&mut workspace, "main.tex", "\\hello{World}\n\\hel", Some(3));
        let definitions = &workspace.documents[&macros].syntax.definitions;
        assert_eq!(definitions.len(), 1, "{declaration}");
        assert_eq!(definitions[0].name, "hello");
        assert!(
            features::diagnostics(&workspace.documents[&macros]).is_empty(),
            "{declaration}"
        );
        assert_eq!(
            features::definitions(&workspace, &main, Position::new(0, 3))[0].uri,
            macros
        );
        assert_eq!(
            features::references(&workspace, &main, Position::new(0, 3), false).len(),
            1,
            "{declaration}"
        );
        assert_eq!(
            features::references(&workspace, &main, Position::new(0, 3), true).len(),
            2,
            "{declaration}"
        );
        assert!(features::completion(&workspace, &main, Position::new(1, 4))
            .iter()
            .any(|item| item.label == "hello"));
        let declaration_position = workspace.documents[&macros]
            .text
            .position(definitions[0].span.start + 2);
        let edits = features::rename(&workspace, &macros, declaration_position, "greeting")
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert!(
            edits_result(declaration, &edits[&macros]).contains("\\greeting"),
            "{declaration}"
        );
        assert_eq!(
            edits_result(&workspace.documents[&main].text.source, &edits[&main]),
            "\\greeting{World}\n\\hel"
        );
    }
}

#[test]
fn rejects_conflicts_ambiguous_definitions_invalid_names_and_builtins() {
    let mut workspace = Workspace::default();
    let uri = insert(
        &mut workspace,
        "main.tex",
        r"\label{old}\label{taken}\ref{old}\newcommand{\foo}{x}\foo",
        Some(1),
    );
    assert!(features::rename(&workspace, &uri, Position::new(0, 8), "taken").is_err());
    for name in ["", "bad name", "a,b", "a}x", "\\new"] {
        assert!(features::rename(&workspace, &uri, Position::new(0, 8), name).is_err());
    }
    assert!(
        features::prepare_rename(&workspace, &uri, Position::new(0, 3))
            .unwrap()
            .is_none()
    );
    let macro_position = workspace.documents[&uri]
        .text
        .position(workspace.documents[&uri].text.source.rfind("foo").unwrap() + 1);
    assert!(features::rename(&workspace, &uri, macro_position, "section").is_err());
    insert(&mut workspace, "another.tex", r"\label{old}", None);
    assert!(features::prepare_rename(&workspace, &uri, Position::new(0, 8)).is_err());
}

#[test]
fn independent_projects_rename_their_own_labels_citations_macros_and_environments() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(root.join(format!("{name}/main.tex")),"\\documentclass{article}\n\\usepackage{local}\n\\begin{document}\n\\input{chapter}\n\\ref{sec:intro} \\cite{book} \\localmacro \\begin{panel}Text\\end{panel}\n\\bibliography{refs}\n\\end{document}\n").unwrap();
        std::fs::write(
            root.join(format!("{name}/chapter.tex")),
            "\\label{sec:intro}\n",
        )
        .unwrap();
        std::fs::write(
            root.join(format!("{name}/refs.bib")),
            "@book{book,title={Example}}\n",
        )
        .unwrap();
        std::fs::write(
            root.join(format!("{name}/local.sty")),
            "\\newcommand{\\localmacro}{Hello}\n\\newenvironment{panel}{}{}\n",
        )
        .unwrap();
    }
    // Names in another paper must neither cause conflicts nor receive edits.
    std::fs::write(
        root.join("b/chapter.tex"),
        "\\label{sec:intro}\\label{sec:new}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("b/refs.bib"),
        "@book{book,title={B}}\n@book{newbook,title={Other}}\n",
    )
    .unwrap();
    std::fs::write(root.join("b/local.sty"),"\\newcommand{\\localmacro}{B}\\newcommand{\\renamedmacro}{B}\\newenvironment{panel}{}{}\\newenvironment{renamedpanel}{}{}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a/main.tex")).unwrap();
    let source = &workspace.documents[&main].text;
    for (needle, offset, new_name) in [
        ("ref{sec:intro}", 5, "sec:new"),
        ("cite{book}", 6, "newbook"),
        ("\\localmacro", 2, "renamedmacro"),
        ("begin{panel}", 7, "renamedpanel"),
    ] {
        let position = source.position(source.source.find(needle).unwrap() + offset);
        assert!(features::prepare_rename(&workspace, &main, position)
            .unwrap()
            .is_some());
        let changes = features::rename(&workspace, &main, position, new_name)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        assert!(changes.len() >= 2, "{needle}: {changes:?}");
        assert!(
            changes
                .keys()
                .all(|uri| uri.to_file_path().unwrap().starts_with(root.join("a"))),
            "{needle}: {changes:?}"
        );
        assert!(changes[&main]
            .iter()
            .all(|edit| edit.new_text.trim_start_matches('\\') == new_name));
    }
}

#[test]
fn shared_declarations_rename_every_dependent_project_and_check_all_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")),"\\documentclass{article}\n\\usepackage{common}\n\\begin{document}\n\\sharedmacro \\cite{sharedbook}\n\\bibliography{common}\n\\end{document}\n").unwrap();
    }
    std::fs::write(
        root.join("common.sty"),
        "\\newcommand{\\sharedmacro}{Hello}",
    )
    .unwrap();
    std::fs::write(root.join("common.bib"), "@book{sharedbook,title={Example}}").unwrap();
    std::fs::write(root.join("other.tex"),"\\documentclass{article}\\newcommand{\\sharedmacro}{Other}\\begin{document}\\sharedmacro\\cite{sharedbook}\\bibliography{other}\\end{document}").unwrap();
    std::fs::write(root.join("other.bib"), "@book{sharedbook,title={Other}}").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a.tex")).unwrap();
    let b = Url::from_file_path(root.join("b.tex")).unwrap();
    let shared = Url::from_file_path(root.join("common.sty")).unwrap();
    let position = workspace.documents[&main].text.position(
        workspace.documents[&main]
            .text
            .source
            .find("\\sharedmacro")
            .unwrap()
            + 2,
    );
    let changes = features::rename(&workspace, &main, position, "renamedmacro")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        changes
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [main.clone(), b.clone(), shared.clone()].into()
    );
    // Starting at the shared definition works even though its root is ambiguous.
    assert!(workspace.root_document(&shared).is_err());
    let definition = workspace.documents[&shared].text.position(
        workspace.documents[&shared]
            .text
            .source
            .find("\\sharedmacro")
            .unwrap()
            + 2,
    );
    assert_eq!(
        features::rename(&workspace, &shared, definition, "renamedmacro")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()
            .len(),
        3
    );
    let citation = workspace.documents[&main].text.position(
        workspace.documents[&main]
            .text
            .source
            .find("sharedbook")
            .unwrap()
            + 2,
    );
    let changes = features::rename(&workspace, &main, citation, "newbook")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        changes
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        [
            main.clone(),
            b.clone(),
            Url::from_file_path(root.join("common.bib")).unwrap()
        ]
        .into()
    );
    let source = format!(
        "{}\n\\newcommand{{\\conflict}}{{Conflict}}",
        workspace.documents[&b].text.source
    );
    workspace
        .documents
        .insert(b.clone(), Document::new(&b, source, None));
    assert!(features::rename(&workspace, &main, position, "conflict").is_err());
    let source = format!(
        "{}\n\\newcommand{{\\sharedmacro}}{{Override}}",
        workspace.documents[&b].text.source
    );
    workspace
        .documents
        .insert(b.clone(), Document::new(&b, source, None));
    assert!(features::prepare_rename(&workspace, &main, position).is_err());
}

#[test]
fn shared_references_to_different_project_definitions_cannot_be_renamed() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("shared.tex"), "See \\ref{sec:intro}.\n").unwrap();
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")),"\\documentclass{article}\n\\begin{document}\n\\label{sec:intro}\n\\input{shared}\n\\end{document}\n").unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a.tex")).unwrap();
    let position = workspace.documents[&main].text.position(
        workspace.documents[&main]
            .text
            .source
            .find("sec:intro")
            .unwrap()
            + 2,
    );
    assert!(features::prepare_rename(&workspace, &main, position)
        .unwrap_err()
        .message
        .contains("shared occurrence"));
    assert!(features::rename(&workspace, &main, position, "sec:new").is_err());
    // A second consumer without any matching definition is equally unsafe.
    let b = Url::from_file_path(root.join("b.tex")).unwrap();
    let source = workspace.documents[&b]
        .text
        .source
        .replace("\\label{sec:intro}", "");
    workspace
        .documents
        .insert(b.clone(), Document::new(&b, source, None));
    assert!(features::rename(&workspace, &main, position, "sec:new").is_err());
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn both_projects_compile_after_renaming_a_shared_macro() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a", "b"] {
        std::fs::write(root.join(format!("{name}.tex")),"\\documentclass{article}\n\\usepackage{common}\n\\begin{document}\\greet\\end{document}\n").unwrap();
    }
    std::fs::write(root.join("common.sty"), "\\newcommand{\\greet}{Hello}\n").unwrap();
    let other = "\\documentclass{article}\\newcommand{\\greet}{Other}\\begin{document}\\greet\\end{document}";
    std::fs::write(root.join("other.tex"), other).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("a.tex")).unwrap();
    let position = workspace.documents[&main].text.position(
        workspace.documents[&main]
            .text
            .source
            .find("\\greet")
            .unwrap()
            + 2,
    );
    let changes = features::rename(&workspace, &main, position, "salute")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 3);
    for (uri, edits) in changes {
        let source = edits_result(&workspace.documents[&uri].text.source, &edits);
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(root.join("other.tex")).unwrap(),
        other
    );
    for name in ["a", "b"] {
        let outcome = latex_lsp::build::run(
            &root.join(format!("{name}.tex")),
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
