use latex_lsp::{
    language::features,
    parser::syntax::Syntax,
    project::workspace::{Document, Workspace},
    text::Text,
};
use lsp_types::*;

#[test]
fn meaning_reads_one_control_without_executing_its_interface() {
    for (name, tail) in [
        ("newcommand", r"\ordinary{Body}"),
        ("def", r"\ordinary{Body}"),
        ("newenvironment", "{panel}{}{}"),
        ("newif", r"\ifdraft"),
        ("label", "{real}"),
        ("definecolor", "{shade}{rgb}{0,0,0}"),
    ] {
        let source = format!(r"\meaning\{name}{tail}");
        let parsed = Syntax::latex(&Text::new(source.clone()));
        assert!(parsed.definitions.is_empty(), "{source}: {parsed:?}");
        assert!(parsed.environments.is_empty(), "{source}: {parsed:?}");
        assert!(parsed.conditionals.is_empty(), "{source}: {parsed:?}");
        assert!(parsed.labels.is_empty(), "{source}: {parsed:?}");
        assert!(parsed.colors.is_empty(), "{source}: {parsed:?}");
        let operand = parsed
            .commands
            .iter()
            .find(|command| command.name == name)
            .unwrap();
        assert!(
            parsed.control_operands.contains(&operand.span.start),
            "{source}"
        );
        assert!(operand.argument.is_none(), "{source}");
    }
    for source in [
        r"\meaning\verb|\newcommand{\ordinary}{Body}|",
        r"\meaning\begin{verbatim}\newcommand{\ordinary}{Body}",
        r"\meaning\detokenize{\newcommand{\ordinary}{Body}}",
        r"\meaning\string\newcommand{\ordinary}{Body}",
        r"\meaning\meaning\newcommand{\ordinary}{Body}",
        r"\meaning*\newcommand{\ordinary}{Body}",
        r"\meaning=\newcommand{\ordinary}{Body}",
    ] {
        let parsed = Syntax::latex(&Text::new(source.into()));
        assert!(parsed.diagnostics.is_empty(), "{source}: {parsed:?}");
        assert_eq!(
            parsed
                .definitions
                .iter()
                .filter(|entry| entry.name == "ordinary")
                .count(),
            1,
            "{source}: {parsed:?}"
        );
    }
}

#[test]
fn meaning_brace_tokens_preserve_groups_macro_bodies_and_real_errors() {
    for source in [
        r"\meaning{",
        r"\meaning}",
        r"\meaning{\meaning}",
        r"\meaning{\let\saved=}",
        r"\let\saved={\meaning}",
        r"{\meaning{Text}",
        r"{\meaning}Text}",
        r"\meaning % comment
        {",
        r"\newcommand{\wrapper}{\meaning{}}",
        r"\def\wrapper#1\meaning{Body}",
    ] {
        let parsed = Syntax::latex(&Text::new(source.into()));
        assert!(parsed.diagnostics.is_empty(), "{source}: {parsed:?}");
        assert!(parsed.fixes.is_empty(), "{source}: {parsed:?}");
    }
    for (source, message) in [
        (r"\meaning{ {", "Unclosed brace"),
        (r"\meaning} }", "Unexpected closing brace"),
        (r"\newcommand{\wrapper}{\meaning{", "Unclosed brace"),
        (r"\def\meaning{}\meaning{", "Unclosed brace"),
    ] {
        let parsed = Syntax::latex(&Text::new(source.into()));
        assert!(!parsed.diagnostics.is_empty(), "{source}: {parsed:?}");
        assert!(
            parsed
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.message == message),
            "{source}: {parsed:?}"
        );
    }
}

#[test]
fn inspected_reference_controls_do_not_edit_the_following_group() {
    let source = r"\label{real}\meaning\ref{real}\ref{real}";
    let uri = Url::parse("file:///meaning-operands/main.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
    let document = &workspace.documents[&uri];
    let inspected_group = source.find(r"\meaning\ref{").unwrap() + r"\meaning\ref{".len();
    let position = document.text.position(inspected_group + 2);
    assert!(latex_lsp::language::signature::help(&workspace, &uri, position).is_none());
    assert!(features::completion(&workspace, &uri, position).is_empty());
    assert!(features::prepare_rename(&workspace, &uri, position)
        .unwrap()
        .is_none());
    let edit = features::rename(
        &workspace,
        &uri,
        document.text.position(source.find("real").unwrap()),
        "renamed",
    )
    .unwrap()
    .unwrap();
    let mut edits = edit.changes.unwrap().remove(&uri).unwrap();
    assert_eq!(edits.len(), 2);
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let changed = document
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
        .unwrap();
    assert_eq!(
        changed.source,
        r"\label{renamed}\meaning\ref{real}\ref{renamed}"
    );
}

#[test]
fn meaning_references_support_cross_file_navigation_and_rename() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("main.tex"), r"\input{definitions}\input{content}").unwrap();
    std::fs::write(
        root.join("definitions.tex"),
        r"\newcommand{\ordinary}[1]{#1}",
    )
    .unwrap();
    let source = concat!(
        r"\meaning\newcommand\ordinary{Follower}\meaning\ordinary{Uncalled}\ordinary{Body}",
        r"\expandafter\meaning\csname ordinary\endcsname",
    );
    std::fs::write(root.join("content.tex"), source).unwrap();
    let uri = Url::from_file_path(root.join("content.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let document = &workspace.documents[&uri];
    let position = document
        .text
        .position(source.find(r"\meaning\ordinary").unwrap() + r"\meaning\".len());
    let definitions = features::definitions(&workspace, &uri, position);
    assert_eq!(definitions.len(), 1, "{definitions:?}");
    assert_eq!(
        definitions[0].uri.to_file_path().unwrap(),
        root.join("definitions.tex")
    );
    let changes = features::rename(&workspace, &uri, position, "renamedordinary")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes.values().map(Vec::len).sum::<usize>(), 5);
    let uncalled = document.text.position(source.find("Uncalled").unwrap() + 2);
    assert!(latex_lsp::language::signature::help(&workspace, &uri, uncalled).is_none());
}

#[test]
fn rebound_inspection_keeps_possible_inputs_without_importing_copied_sources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for (name, source) in [
        (
            "main.tex",
            r"\documentclass{article}\let\saved\input{copied}\input{provider}\begin{document}\meaning\input{observed}\end{document}",
        ),
        ("provider.tex", r"\def\meaning#1{#1}"),
        ("copied.tex", r"\newcommand{\fromcopied}{Fake}"),
        ("observed.tex", r"\newcommand{\fromobserved}{Real}"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let project = workspace.project_documents(&main);
    assert!(project.contains(&Url::from_file_path(root.join("provider.tex")).unwrap()));
    assert!(project.contains(&Url::from_file_path(root.join("observed.tex")).unwrap()));
    assert!(
        !project.contains(&Url::from_file_path(root.join("copied.tex")).unwrap()),
        "{project:?}"
    );
}

#[test]
fn rebound_meaning_does_not_hide_executed_declarations() {
    for prefix in [
        r"\def\meaning{}",
        r"\csdef{meaning}{}",
        r"\expandafter\def\csname meaning\endcsname{}",
    ] {
        let source = format!(r"{prefix}\meaning\newcommand{{\ordinary}}{{Body}}");
        let parsed = Syntax::latex(&Text::new(source.clone()));
        assert!(
            parsed
                .definitions
                .iter()
                .any(|entry| entry.name == "ordinary"),
            "{source}: {parsed:?}"
        );
    }
}
