use lsp_types::*;
use std::{collections::BTreeMap, path::Path};
use texquill::{
    editing,
    project::workspace::{Document, Workspace},
};

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn project(root: &Path, files: &[(&str, &str)]) -> Workspace {
    for (name, text) in files {
        std::fs::write(root.join(name), text).unwrap();
    }
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            versioned_edits: true,
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    for doc in workspace.documents.values_mut() {
        doc.version = Some(7);
    }
    workspace
}
fn actions(workspace: &Workspace, uri: &Url, diagnostics: Vec<Diagnostic>) -> CodeActionResponse {
    editing::code_actions(
        workspace,
        uri,
        Range::new(Position::new(0, 0), Position::new(u32::MAX, 0)),
        &CodeActionContext {
            diagnostics,
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        },
    )
}
fn apply(workspace: &Workspace, uri: &Url, edit: WorkspaceEdit) -> String {
    let Some(DocumentChanges::Edits(mut changes)) = edit.document_changes else {
        panic!("expected versioned edit")
    };
    assert_eq!(changes.len(), 1);
    let change = changes.pop().unwrap();
    assert_eq!(change.text_document.uri, *uri);
    assert_eq!(change.text_document.version, Some(7));
    let mut edits: Vec<_> = change
        .edits
        .into_iter()
        .map(|edit| match edit {
            OneOf::Left(edit) => edit,
            _ => panic!(),
        })
        .collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    workspace.documents[uri]
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
fn calls() -> [(&'static str, &'static str); 5] {
    [
        (r"\csuse{begin}", r"\csuse{end}"),
        (r"\csname begin\endcsname", r"\csname end\endcsname"),
        (r"\start", r"\finish"),
        (r"\csuse{start}", r"\csuse{finish}"),
        (r"\csname start\endcsname", r"\csname finish\endcsname"),
    ]
}

#[test]
fn mixed_environment_calls_do_not_offer_false_diagnostics_or_destructive_fixes() {
    for (start, finish) in calls() {
        for side in ["begin", "end", "both"] {
            for external in [false, true] {
                for bare in [false, true] {
                    let temp = tempfile::tempdir().unwrap();
                    let root = temp.path();
                    let declaration = r"\newenvironment{x}{}{}\def\start{\begin}\def\finish{\end}";
                    let preamble = if external {
                        r"\usepackage{aliases}"
                    } else {
                        declaration
                    };
                    let begin = if side == "end" { r"\begin" } else { start };
                    let end = if side == "begin" { r"\end" } else { finish };
                    let name = if bare {
                        " x"
                    } else {
                        "{% name\r\nx% keep\r\n}"
                    };
                    let source = format!("% 😀\r\n\\documentclass{{article}}{preamble}\\begin{{document}}{begin}{name}Body{end}{name}Tail\\end{{document}}");
                    let other = r"\documentclass{article}\newcommand{\start}[1]{#1}\newcommand{\finish}[1]{#1}\begin{document}Other\end{document}";
                    let workspace = project(
                        root,
                        &[
                            ("main.tex", &source),
                            ("aliases.sty", if external { declaration } else { "" }),
                            ("other.tex", other),
                        ],
                    );
                    let main = uri(root, "main.tex");
                    assert!(
                        workspace.diagnostics(&main).is_empty(),
                        "{source}: {:?}",
                        workspace.diagnostics(&main)
                    );
                    assert!(actions(&workspace, &main, vec![]).is_empty(), "{source}");
                    for old in &workspace.documents[&main].syntax.environment_fixes {
                        assert!(
                            editing::resolve_fix(
                                &workspace,
                                editing::FixRequest {
                                    uri: main.clone(),
                                    version: Some(7),
                                    diagnostic: old.diagnostic.clone(),
                                    replacement: Some(old.edit.new_text.clone())
                                }
                            )
                            .is_none(),
                            "{source}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn indirect_unexpected_ends_remove_the_complete_invocation_and_keep_the_tail() {
    for (_, end) in calls() {
        for name in [" x", "{% name\r\nx% keep\r\n}"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let invocation = format!("{end}{name}");
            let source = format!(
                r"\documentclass{{article}}\def\finish{{\end}}\begin{{document}}Before{invocation}Tail\end{{document}}"
            );
            let workspace = project(root, &[("main.tex", &source)]);
            let main = uri(root, "main.tex");
            let diagnostics = workspace.diagnostics(&main);
            assert_eq!(diagnostics.len(), 1, "{source}: {diagnostics:?}");
            assert_eq!(diagnostics[0].message, r"Unexpected \end{x}");
            let available = actions(&workspace, &main, diagnostics.clone());
            assert_eq!(available.len(), 1, "{available:?}");
            let CodeActionOrCommand::CodeAction(action) = available[0].clone() else {
                panic!()
            };
            let expected = source.replace(&invocation, "");
            assert_eq!(apply(&workspace, &main, action.edit.unwrap()), expected);
            let edit = editing::resolve_fix(
                &workspace,
                editing::FixRequest {
                    uri: main.clone(),
                    version: Some(7),
                    diagnostic: diagnostics[0].clone(),
                    replacement: Some(String::new()),
                },
            )
            .unwrap();
            assert_eq!(apply(&workspace, &main, edit), expected);
        }
    }
}

#[test]
fn missing_indirect_ends_are_inserted_before_the_enclosing_end() {
    for (begin, _) in calls() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source=format!("\\documentclass{{article}}\\def\\start{{\\begin}}\\begin{{document}}{begin}{{x}}Body\\end{{document}}\r\n");
        let workspace = project(root, &[("main.tex", &source)]);
        let main = uri(root, "main.tex");
        let diagnostics = workspace.diagnostics(&main);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(
            diagnostics[0].message,
            r"Unclosed environment `x` before \end{document}"
        );
        let CodeActionOrCommand::CodeAction(action) =
            actions(&workspace, &main, diagnostics).remove(0)
        else {
            panic!()
        };
        assert_eq!(
            apply(&workspace, &main, action.edit.unwrap()),
            source.replace(r"\end{document}", "\\end{x}\r\n\\end{document}")
        );
    }
}

#[test]
fn alias_changes_in_another_file_invalidate_old_environment_fixes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = r"\documentclass{article}\usepackage{aliases}\begin{document}\start{x}Body\end{x}\end{document}";
    let mut workspace = project(
        root,
        &[
            ("main.tex", source),
            ("aliases.sty", r"\newcommand{\start}[1]{#1}"),
        ],
    );
    let main = uri(root, "main.tex");
    let aliases = uri(root, "aliases.sty");
    let diagnostic = workspace.diagnostics(&main).remove(0);
    let request = editing::FixRequest {
        uri: main.clone(),
        version: Some(7),
        diagnostic: diagnostic.clone(),
        replacement: Some(String::new()),
    };
    assert!(editing::resolve_fix(
        &workspace,
        editing::FixRequest {
            uri: main.clone(),
            version: Some(7),
            diagnostic: diagnostic.clone(),
            replacement: Some(String::new())
        }
    )
    .is_some());
    workspace.documents.insert(
        aliases.clone(),
        Document::new(&aliases, r"\def\start{\begin}".into(), Some(8)),
    );
    assert!(workspace.diagnostics(&main).is_empty());
    assert!(actions(&workspace, &main, vec![diagnostic]).is_empty());
    assert!(editing::resolve_fix(&workspace, request).is_none());
}

#[test]
fn shared_files_only_offer_pairing_fixes_agreed_by_every_project() {
    for agree in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let a =
            r"\documentclass{article}\def\finish{\end}\begin{document}\input{shared}\end{document}";
        let b = if agree {
            a
        } else {
            r"\documentclass{article}\newcommand{\finish}[1]{#1}\begin{document}\input{shared}\end{document}"
        };
        let workspace = project(
            root,
            &[
                ("a.tex", a),
                ("b.tex", b),
                ("shared.tex", r"\finish{x}Tail"),
            ],
        );
        let shared = uri(root, "shared.tex");
        let diagnostics = workspace.diagnostics(&shared);
        assert_eq!(diagnostics.len(), usize::from(agree), "{diagnostics:?}");
        assert_eq!(
            actions(&workspace, &shared, diagnostics).len(),
            usize::from(agree)
        );
    }
}

#[test]
fn uncertain_names_do_not_hide_independent_pairing_errors_or_verbatim_errors() {
    for deferred in [
        r"\newcommand{\unused}{\csuse{begin}{x}}",
        r"\newcommand{\unused}[1]{\csuse{begin}{x}#1}",
        r"\def\unused#1{\csuse{begin}{x}#1}",
        r"\NewDocumentCommand{\unused}{m}{\csuse{begin}{x}#1}",
        r"\newcommand{\unused}[1]{\begin{#1}#1}",
        r"\newcommand{\take}[1]{\detokenize{#1}}\take{\csuse{begin}{x}}",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = format!(
            r"\documentclass{{article}}\begin{{document}}{deferred}\begin{{other}}Body\end{{document}}"
        );
        let workspace = project(root, &[("main.tex", &source)]);
        let main = uri(root, "main.tex");
        let diagnostics = workspace.diagnostics(&main);
        assert_eq!(diagnostics.len(), 1, "{deferred}: {diagnostics:?}");
        assert!(
            diagnostics[0].message.contains("environment `other`"),
            "{diagnostics:?}"
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = r"\documentclass{article}\begin{document}\csuse{begin}{x}Body\csuse{end}{x}\begin{verbatim} raw { \end{x}";
    let mut workspace = project(root, &[("main.tex", source)]);
    let main = uri(root, "main.tex");
    let build = Diagnostic {
        message: "Actual build error".into(),
        source: Some("latex-build".into()),
        ..Default::default()
    };
    workspace
        .project
        .build_diagnostics
        .insert(main.clone(), BTreeMap::from([(main.clone(), vec![build])]));
    let diagnostics = workspace.diagnostics(&main);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message == "Unclosed environment `verbatim`"),
        "{diagnostics:?}"
    );
    assert!(diagnostics
        .iter()
        .any(|d| d.message == "Actual build error"));
}
