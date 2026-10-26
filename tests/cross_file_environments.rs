use lsp_types::*;
use std::path::Path;
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
        panic!()
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
fn first_edit(workspace: &Workspace, uri: &Url) -> WorkspaceEdit {
    let CodeActionOrCommand::CodeAction(action) =
        actions(workspace, uri, workspace.diagnostics(uri)).remove(0)
    else {
        panic!()
    };
    action.edit.unwrap()
}

#[test]
fn environments_pair_across_inputs_in_execution_order() {
    for (begin, end) in [
        (r"\begin", r"\end"),
        (r"\csuse{begin}", r"\csuse{end}"),
        (r"\start", r"\finish"),
    ] {
        for parent_begin in [false, true] {
            for nested in [false, true] {
                for bare in [false, true] {
                    let temp = tempfile::tempdir().unwrap();
                    let root = temp.path();
                    let name = if bare {
                        " x"
                    } else {
                        "{% name\r\nx% keep\r\n}"
                    };
                    let input = if nested {
                        r"\input{wrapper}"
                    } else {
                        r"\input{part}"
                    };
                    let preamble="% 😀\r\n\\documentclass{article}\\def\\start{\\begin}\\def\\finish{\\end}\\begin{document}";
                    let (main, part) = if parent_begin {
                        (
                            format!("{preamble}{begin}{name}Main {input} Tail\\end{{document}}"),
                            format!("Part {end}{name}"),
                        )
                    } else {
                        (
                            format!("{preamble}{input} Main {end}{name} Tail\\end{{document}}"),
                            format!("{begin}{name}Part"),
                        )
                    };
                    let workspace = project(
                        root,
                        &[
                            ("main.tex", &main),
                            ("part.tex", &part),
                            ("wrapper.tex", r"\input{part}"),
                            (
                                "other.tex",
                                r"\documentclass{article}\newcommand{\start}[1]{#1}\begin{document}Other\end{document}",
                            ),
                        ],
                    );
                    for name in ["main.tex", "part.tex"] {
                        let target = uri(root, name);
                        assert!(
                            workspace.diagnostics(&target).is_empty(),
                            "{begin} {parent_begin} {nested} {bare} {name}: {:?}",
                            workspace.diagnostics(&target)
                        );
                        assert!(actions(&workspace, &target, vec![]).is_empty());
                    }
                }
            }
        }
    }
}

#[test]
fn cross_file_missing_ends_use_the_closing_files_position_and_related_opener() {
    for child_closes in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let main = if child_closes {
            "% 😀\r\n\\documentclass{article}\\begin{document}\\begin{outer}\\begin{x}Before\\input{part} Tail\\end{document}"
        } else {
            "% 😀\r\n\\documentclass{article}\\begin{document}\\input{part} Tail\\end{document}"
        };
        let part = if child_closes {
            r"Part\csname end\endcsname{outer}After"
        } else {
            r"\csuse{begin}{x}Part"
        };
        let workspace = project(root, &[("main.tex", main), ("part.tex", part)]);
        let main_uri = uri(root, "main.tex");
        let part_uri = uri(root, "part.tex");
        let (target, opener) = if child_closes {
            (&part_uri, &main_uri)
        } else {
            (&main_uri, &part_uri)
        };
        let diagnostics = workspace.diagnostics(target);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let related = diagnostics[0].related_information.as_ref().unwrap();
        assert_eq!(related[0].location.uri, *opener);
        let edited = apply(&workspace, target, first_edit(&workspace, target));
        let expected = if child_closes {
            part.replace(r"\csname end", "\\end{x}\n\\csname end")
        } else {
            main.replace(r"\end{document}", "\\end{x}\r\n\\end{document}")
        };
        assert_eq!(edited, expected);
        assert!(workspace.diagnostics(opener).is_empty());
    }
}

#[test]
fn unexpected_child_ends_remove_the_whole_indirect_call() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let main = r"\documentclass{article}\begin{document}\input{part}Main\end{document}";
    let invocation = "\\csname end\\endcsname{% name\r\nx% keep\r\n}";
    let part = format!("Before{invocation}Tail");
    let workspace = project(root, &[("main.tex", main), ("part.tex", &part)]);
    let child = uri(root, "part.tex");
    assert_eq!(workspace.diagnostics(&child).len(), 1);
    assert_eq!(
        apply(&workspace, &child, first_edit(&workspace, &child)),
        "BeforeTail"
    );
    assert!(workspace.diagnostics(&uri(root, "main.tex")).is_empty());
}

#[test]
fn child_changes_invalidate_parent_fixes_without_a_parent_version_change() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let main = r"\documentclass{article}\begin{document}\input{part}Tail\end{document}";
    let mut workspace = project(root, &[("main.tex", main), ("part.tex", r"\begin{x}Part")]);
    let parent = uri(root, "main.tex");
    let child = uri(root, "part.tex");
    let diagnostic = workspace.diagnostics(&parent).remove(0);
    let replacement = "\\end{x}\n";
    let request = || editing::FixRequest {
        uri: parent.clone(),
        version: Some(7),
        diagnostic: diagnostic.clone(),
        replacement: Some(replacement.into()),
    };
    assert!(editing::resolve_fix(&workspace, request()).is_some());
    workspace.documents.insert(
        child.clone(),
        Document::new(&child, r"\begin{x}Part\end{x}".into(), Some(8)),
    );
    assert!(workspace.diagnostics(&parent).is_empty());
    assert!(editing::resolve_fix(&workspace, request()).is_none());
    assert!(actions(&workspace, &parent, vec![diagnostic]).is_empty());
}

#[test]
fn repeated_source_edits_require_the_same_error_in_every_occurrence() {
    for healthy_first in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let main = format!(
            r"\documentclass{{article}}\begin{{document}}{}\input{{part}}\input{{part}}\end{{document}}",
            if healthy_first { r"\begin{x}" } else { "" }
        );
        let workspace = project(root, &[("main.tex", &main), ("part.tex", r"\end{x}Tail")]);
        let child = uri(root, "part.tex");
        assert_eq!(
            workspace.diagnostics(&child).len(),
            usize::from(!healthy_first)
        );
        assert_eq!(
            actions(&workspace, &child, vec![]).len(),
            usize::from(!healthy_first)
        );
        if !healthy_first {
            assert_eq!(
                apply(&workspace, &child, first_edit(&workspace, &child)),
                "Tail"
            );
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let main = r"\documentclass{article}\begin{document}\input{part}\input{part}\end{document}";
    let workspace = project(root, &[("main.tex", main), ("part.tex", r"\begin{x}Part")]);
    let parent = uri(root, "main.tex");
    assert_eq!(workspace.diagnostics(&parent).len(), 1);
    assert_eq!(
        apply(&workspace, &parent, first_edit(&workspace, &parent)),
        main.replace(r"\end{document}", "\\end{x}\n\\end{x}\n\\end{document}")
    );
    let workspace = project(
        root,
        &[("main.tex", main), ("part.tex", r"\begin{x}Part\end{x}")],
    );
    assert!(workspace.diagnostics(&parent).is_empty());
    assert!(workspace.diagnostics(&uri(root, "part.tex")).is_empty());
}

#[test]
fn shared_files_require_agreement_between_all_root_execution_orders() {
    for (a_open, b_open) in [(false, false), (true, false), (true, true)] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = |open| {
            format!(
                r"\documentclass{{article}}\begin{{document}}{}\input{{part}}\end{{document}}",
                if open { r"\begin{x}" } else { "" }
            )
        };
        let a = source(a_open);
        let b = source(b_open);
        let workspace = project(
            root,
            &[("a.tex", &a), ("b.tex", &b), ("part.tex", r"\end{x}Tail")],
        );
        let child = uri(root, "part.tex");
        assert_eq!(
            actions(&workspace, &child, vec![]).len(),
            usize::from(!a_open && !b_open)
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let a = r"\documentclass{article}\begin{document}\input{part}\end{x}\end{document}";
    let b = r"\documentclass{article}\begin{document}\input{part}Tail\end{document}";
    let workspace = project(
        root,
        &[("a.tex", a), ("b.tex", b), ("part.tex", r"\begin{x}Part")],
    );
    assert!(workspace.diagnostics(&uri(root, "a.tex")).is_empty());
    assert!(workspace.diagnostics(&uri(root, "part.tex")).is_empty());
    let target = uri(root, "b.tex");
    assert_eq!(workspace.diagnostics(&target).len(), 1);
    assert_eq!(
        apply(&workspace, &target, first_edit(&workspace, &target)),
        b.replace(r"\end{document}", "\\end{x}\n\\end{document}")
    );
}

#[test]
fn uncertain_input_sequences_do_not_offer_partial_stack_edits() {
    for code in [
        r"\input{missing}",
        r"\input{\filename}",
        r"\iftrue\input{part}\fi",
        r"\def\input#1{}\input{part}",
        r"\input{cycle}",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let main = format!(
            r"\documentclass{{article}}\begin{{document}}\begin{{x}}{code}\end{{document}}"
        );
        let workspace = project(
            root,
            &[
                ("main.tex", &main),
                ("part.tex", r"\end{x}"),
                ("cycle.tex", r"\input{cycle}"),
            ],
        );
        let parent = uri(root, "main.tex");
        assert!(actions(&workspace, &parent, vec![]).is_empty(), "{code}");
    }
    for code in [
        r"\newcommand{\unused}{\input{part}}",
        r"\newcommand{\take}[1]{\detokenize{#1}}\take{\input{part}}",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let main = format!(
            r"\documentclass{{article}}\begin{{document}}\begin{{x}}{code}\end{{document}}"
        );
        let workspace = project(root, &[("main.tex", &main), ("part.tex", r"\end{x}")]);
        let diagnostics = workspace.diagnostics(&uri(root, "main.tex"));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("Unclosed environment `x`")),
            "{code}: {diagnostics:?}"
        );
    }
}

#[test]
fn endinput_requires_an_unchanged_primitive_and_a_clean_line_ending() {
    for (definition, part) in [
        ("", "Part\\endinput Keep\\end{x}\nIgnored\\end{x}"),
        ("", "Part\\end{x}\\endinput % stop\nIgnored\\end{x}"),
        (r"\def\endinput{}", r"Part\endinput Keep\end{x}"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let main = format!(
            r"\documentclass{{article}}{definition}\begin{{document}}\begin{{x}}\input{{part}}Tail\end{{document}}"
        );
        let workspace = project(root, &[("main.tex", &main), ("part.tex", part)]);
        for name in ["main.tex", "part.tex"] {
            let target = uri(root, name);
            assert!(workspace.diagnostics(&target).is_empty(), "{part}");
            assert!(actions(&workspace, &target, vec![]).is_empty(), "{part}");
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let main = r"\documentclass{article}\begin{document}\begin{x}\input{part}Tail\end{document}";
    let part = "Part\\endinput\nIgnored\\end{y}";
    let workspace = project(root, &[("main.tex", main), ("part.tex", part)]);
    let parent = uri(root, "main.tex");
    let diagnostics = workspace.diagnostics(&parent);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("Unclosed environment `x`"));
    assert_eq!(
        apply(&workspace, &parent, first_edit(&workspace, &parent)),
        main.replace(r"\end{document}", "\\end{x}\n\\end{document}")
    );
}
