use latex_lsp::{
    editing, features,
    text::Text,
    workspace::{Document, Workspace},
};
use lsp_types::*;

fn setup(source: &str, versioned: bool) -> (Workspace, Url) {
    let uri = Url::parse("file:///editing/main.tex").unwrap();
    let mut workspace = Workspace {
        versioned_edits: versioned,
        code_action_literals: true,
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(9)));
    (workspace, uri)
}

fn context(diagnostics: Vec<Diagnostic>) -> CodeActionContext {
    CodeActionContext {
        diagnostics,
        only: None,
        trigger_kind: None,
    }
}

fn include_warning(workspace: &mut Workspace, root: &Url, uri: &Url, name: &str) -> Diagnostic {
    let doc = &workspace.documents[uri];
    let argument = doc
        .syntax
        .commands
        .iter()
        .find(|command| command.name == "include")
        .unwrap()
        .argument
        .as_ref()
        .unwrap();
    let leading = argument.value.len() - argument.value.trim_start().len();
    let trailing = argument.value.trim_end().len();
    let diagnostic = Diagnostic {
        range: doc.text.range(
            argument.span.start + leading,
            argument.span.start + trailing,
        ),
        source: Some("latex-build".into()),
        code: Some(NumberOrString::String("missing-include".into())),
        message: format!("Included file `{name}` not found"),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        root.clone(),
        [(uri.clone(), vec![diagnostic.clone()])].into(),
    );
    diagnostic
}

#[test]
fn include_fix_requests_never_exceed_the_total_suggestion_limit() {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let source = "\\include{chapter}\n".repeat(64);
    std::fs::write(directory.path().join("main.tex"), &source).unwrap();
    for index in 0..8 {
        std::fs::write(
            directory.path().join(format!("chapter{index}.tex")),
            "Chapter.",
        )
        .unwrap();
    }
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.clone(), Some(1)));
    let text = &workspace.documents[&uri].text;
    let diagnostics: Vec<_> = workspace.documents[&uri]
        .syntax
        .commands
        .iter()
        .filter_map(|command| command.argument.as_ref())
        .map(|argument| Diagnostic {
            range: text.range(argument.span.start, argument.span.end),
            message: "Included file `chapter.tex` not found".into(),
            source: Some("latex-build".into()),
            code: Some(NumberOrString::String("missing-include".into())),
            ..Default::default()
        })
        .collect();
    let range = text.range(0, source.len());
    workspace
        .build_diagnostics
        .insert(uri.clone(), [(uri.clone(), diagnostics.clone())].into());
    let actions = editing::code_actions(&workspace, &uri, range, &context(diagnostics));
    assert_eq!(actions.len(), 256);
}

#[test]
fn missing_include_fixes_preserve_comments_utf16_prefixes_and_explicit_extensions() {
    for versioned in [false, true] {
        for extension in ["", ".tex"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            std::fs::create_dir(root.join("sections")).unwrap();
            std::fs::write(root.join("sections/chapter.tex"), "Chapter.\n").unwrap();
            let uri = Url::from_file_path(root.join("main.tex")).unwrap();
            let source =
                format!("😀 \\include % keep\r\n{{ sections/chapetr{extension} }} % keep\r\n");
            std::fs::write(root.join("main.tex"), &source).unwrap();
            let mut workspace = Workspace {
                versioned_edits: versioned,
                code_action_literals: true,
                ..Default::default()
            };
            workspace
                .documents
                .insert(uri.clone(), Document::new(&uri, source.clone(), Some(9)));
            let diagnostic = include_warning(&mut workspace, &uri, &uri, "sections/chapetr.tex");
            let actions = editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &context(vec![diagnostic]),
            );
            assert_eq!(actions.len(), 1, "{actions:?}");
            let edit = action_edit(&actions[0], &uri);
            assert_eq!(edit.new_text, format!("sections/chapter{extension}"));
            let updated = Text::new(source.clone())
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap();
            assert_eq!(updated.source, source.replace("chapetr", "chapter"));
        }
    }
}

#[test]
fn include_fix_commands_recheck_candidate_existence_original_existence_and_versions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let source = "\\include{chapetr}\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(root.join("chapter.tex"), "Chapter.\n").unwrap();
    let mut workspace = Workspace {
        apply_edits: true,
        ..Default::default()
    };
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(9)));
    let diagnostic = include_warning(&mut workspace, &uri, &uri, "chapetr.tex");
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic.clone()]),
    );
    assert_eq!(actions.len(), 1);
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!()
    };
    let argument = command.arguments.as_ref().unwrap()[0].clone();
    let resolve = |workspace: &Workspace| {
        editing::resolve_fix(workspace, serde_json::from_value(argument.clone()).unwrap())
    };
    assert!(resolve(&workspace).is_some());
    std::fs::remove_file(root.join("chapter.tex")).unwrap();
    assert!(resolve(&workspace).is_none());
    std::fs::write(root.join("chapter.tex"), "Chapter.\n").unwrap();
    std::fs::write(root.join("chapetr.tex"), "Now exists.\n").unwrap();
    assert!(resolve(&workspace).is_none());
    std::fs::remove_file(root.join("chapetr.tex")).unwrap();
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(resolve(&workspace).is_none());
    workspace.documents.insert(
        uri.clone(),
        Document::new(&uri, "\\include{another}\n".into(), Some(9)),
    );
    assert!(resolve(&workspace).is_none());
}

#[test]
fn include_candidates_use_the_diagnostic_build_root_instead_of_another_project_or_source_folder() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for folder in ["a", "b", "shared"] {
        std::fs::create_dir(root.join(folder)).unwrap();
    }
    for (name, source) in [
        (
            "a/main.tex",
            "\\documentclass{article}\\input{../shared/source}",
        ),
        (
            "b/main.tex",
            "\\documentclass{article}\\input{../shared/source}",
        ),
        ("shared/source.tex", "\\include{chapetr}\n"),
        ("a/chapter.tex", "A."),
        ("b/chapert.tex", "B."),
        ("shared/chapetr.tex", "Wrong working directory."),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let uri = Url::from_file_path(root.join("shared/source.tex")).unwrap();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for (folder, expected) in [("a", "chapter"), ("b", "chapert")] {
        workspace.build_diagnostics.clear();
        let main = Url::from_file_path(root.join(folder).join("main.tex")).unwrap();
        let diagnostic = include_warning(&mut workspace, &main, &uri, "chapetr.tex");
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic]),
        );
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert_eq!(action_edit(&actions[0], &uri).new_text, expected);
    }
}

#[test]
fn include_fixes_suppress_self_inclusion_known_cycles_nonfiles_and_mismatched_warnings() {
    for (main_name, candidate) in [
        ("chapter.tex", "Text."),
        ("main.tex", "\\input{main}"),
        ("main.tex", "\\input{bridge}"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "\\documentclass{article}\\include{chapetr}\n";
        std::fs::write(root.join(main_name), source).unwrap();
        if main_name != "chapter.tex" {
            std::fs::write(root.join("chapter.tex"), candidate).unwrap();
        }
        std::fs::write(root.join("bridge.tex"), "\\input{main}").unwrap();
        let uri = Url::from_file_path(root.join(main_name)).unwrap();
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        let diagnostic = include_warning(&mut workspace, &uri, &uri, "chapetr.tex");
        assert!(editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic])
        )
        .is_empty());
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    std::fs::write(root.join("main.tex"), "\\include{chapetr}\n").unwrap();
    std::fs::create_dir(root.join("chapter.tex")).unwrap();
    // A case-insensitive filesystem cannot create this second fixture beside
    // the directory above. Keep the wrong-extension candidate where possible.
    if !root.join("chapter.TEX").exists() {
        std::fs::write(root.join("chapter.TEX"), "Wrong extension.").unwrap();
    }
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    let diagnostic = include_warning(&mut workspace, &uri, &uri, "chapetr.tex");
    assert!(editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic.clone()])
    )
    .is_empty());
    std::fs::remove_dir(root.join("chapter.tex")).unwrap();
    std::fs::write(root.join("chapter.tex"), "Valid.").unwrap();
    let mut stale = diagnostic.clone();
    stale.message = "Included file `different.tex` not found".into();
    workspace
        .build_diagnostics
        .get_mut(&uri)
        .unwrap()
        .insert(uri.clone(), vec![stale.clone()]);
    assert!(editing::code_actions(&workspace, &uri, stale.range, &context(vec![stale])).is_empty());
}

#[test]
fn include_fixes_bound_suggestions_support_unicode_and_respect_static_includeonly_lists() {
    for (filter, original, candidate, expected) in [
        ("", "章节引沿", "章节引言", 1),
        ("\\includeonly{chapter,chapetr}", "chapetr", "chapter", 1),
        (
            "\\includeonly{chapter.tex,chapetr}",
            "chapetr",
            "chapter",
            1,
        ),
        ("\\includeonly{chapetr}", "chapetr", "chapter", 0),
        ("\\includeonly{\\selection}", "chapetr", "chapter", 0),
        ("\\includeonly{}", "chapetr", "chapter", 0),
        (
            "\\includeonly{% head, phantom, }\r\n chapter% tail\r\n,chapetr% typo\r\n}",
            "chapetr",
            "chapter",
            1,
        ),
        (
            "\\includeonly{% head, chapter, }\n chapetr% tail\n}",
            "chapetr",
            "chapter",
            0,
        ),
        (
            "\\includeonly{% head\n chapter,\\dynamic\n}",
            "chapetr",
            "chapter",
            0,
        ),
        ("\\includeonly{% empty, chapter\n}", "chapetr", "chapter", 0),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let uri = Url::from_file_path(root.join("main.tex")).unwrap();
        let source = format!("\\documentclass{{article}}{filter}\\include{{{original}}}\n");
        std::fs::write(root.join("main.tex"), source).unwrap();
        std::fs::write(root.join(format!("{candidate}.tex")), "Chapter.").unwrap();
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        let diagnostic = include_warning(&mut workspace, &uri, &uri, &format!("{original}.tex"));
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic]),
        );
        assert_eq!(actions.len(), expected, "{filter}: {actions:?}");
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    std::fs::write(root.join("main.tex"), "\\include{chapter}\n").unwrap();
    for index in 0..8 {
        std::fs::write(root.join(format!("chapter{index}.tex")), "Chapter.").unwrap();
    }
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    let diagnostic = include_warning(&mut workspace, &uri, &uri, "chapter.tex");
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic]),
    );
    assert_eq!(actions.len(), 5);
    assert_eq!(
        actions
            .iter()
            .map(|action| action_edit(action, &uri).new_text)
            .collect::<Vec<_>>(),
        (0..5)
            .map(|index| format!("chapter{index}"))
            .collect::<Vec<_>>()
    );
}

fn action_edit(action: &CodeActionOrCommand, uri: &Url) -> TextEdit {
    let CodeActionOrCommand::CodeAction(action) = action else {
        panic!("missing code action");
    };
    let workspace_edit = action.edit.as_ref().unwrap();
    if let Some(changes) = &workspace_edit.changes {
        return changes[uri][0].clone();
    }
    let Some(DocumentChanges::Edits(documents)) = &workspace_edit.document_changes else {
        panic!("missing edit");
    };
    assert_eq!(documents[0].text_document.uri, *uri);
    assert_eq!(documents[0].text_document.version, Some(9));
    let OneOf::Left(edit) = &documents[0].edits[0] else {
        panic!("unexpected annotation");
    };
    edit.clone()
}

fn destination_diagnostic(workspace: &mut Workspace, uri: &Url) -> Diagnostic {
    let doc = &workspace.documents[uri];
    let entry = &doc.syntax.anchor_references[0];
    let diagnostic = Diagnostic {
        range: doc.text.range(entry.span.start, entry.span.end),
        source: Some("latex-build".into()),
        code: Some(NumberOrString::String("undefined-pdf-destination".into())),
        data: Some(serde_json::json!({"destination":entry.name})),
        message: format!(
            "warning  (pdf backend): unreferenced destination with name '{}'",
            entry.name
        ),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        uri.clone(),
        [(uri.clone(), vec![diagnostic.clone()])].into(),
    );
    diagnostic
}

#[test]
fn pdfbookmark_destinations_offer_spelling_candidates_with_the_literal_level_suffix() {
    let (mut workspace, uri) = setup(
        "\\pdfbookmark{Title}{intro}\\hyperlink{itnro.0}{Text}\n",
        true,
    );
    let diagnostic = destination_diagnostic(&mut workspace, &uri);
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic]),
    );
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(action_edit(&actions[0], &uri).new_text, "intro.0");
}

#[test]
fn destination_candidates_use_the_diagnostic_builds_read_sources_before_counting_duplicates() {
    let (mut workspace, uri) = setup(
        "\\hypertarget{last}{Candidate}\\hyperlink{lost}{Text}\n",
        true,
    );
    let excluded = Url::parse("file:///editing/excluded.tex").unwrap();
    workspace.documents.insert(
        excluded.clone(),
        Document::new(
            &excluded,
            "\\hypertarget{last}{Other}\\hypertarget{loft}{Other}\n".into(),
            None,
        ),
    );
    workspace
        .recorded_inputs
        .insert(uri.clone(), [uri.clone(), excluded.clone()].into());
    let diagnostic = destination_diagnostic(&mut workspace, &uri);
    workspace
        .build_diagnostic_sources
        .insert(uri.clone(), [uri.clone()].into());
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic.clone()]),
    );
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(action_edit(&actions[0], &uri).new_text, "last");
    workspace
        .build_diagnostic_sources
        .insert(uri.clone(), [uri.clone(), excluded].into());
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic]),
    );
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(action_edit(&actions[0], &uri).new_text, "loft");
}

#[test]
fn destination_spelling_fixes_preserve_compound_comments_and_independent_labels() {
    for versioned in [false, true] {
        for (link, declaration, expected) in [
            (
                "\\hyperlink{section.itnro}{Text}",
                "\\hyperdef{section}{intro}{Point}",
                "\\hyperlink{section.intro}{Text}",
            ),
            (
                "\\hyperref{}{section}% keep\r\n{itnro}{Text}",
                "\\hypertarget{section.intro}{Point}",
                "\\hyperref{}{section}% keep\r\n{intro}{Text}",
            ),
            (
                "\\hyperref{}{sectoin}% keep\r\n{intro}{Text}",
                "\\hyperdef{section}{intro}{Point}",
                "\\hyperref{}{}% keep\r\n{section.intro}{Text}",
            ),
        ] {
            let source = format!("😀 {link}\\label{{section.itnro}}\r\n");
            let (mut workspace, uri) = setup(&source, versioned);
            let declarations = Url::parse("file:///editing/destinations.tex").unwrap();
            let unrelated = Url::parse("file:///other/main.tex").unwrap();
            for (target, text) in [
                (&declarations, declaration),
                (&unrelated, "\\hypertarget{section.itnro}{Other}"),
            ] {
                workspace
                    .documents
                    .insert(target.clone(), Document::new(target, text.into(), None));
            }
            workspace
                .recorded_inputs
                .insert(uri.clone(), [uri.clone(), declarations].into());
            let diagnostic = destination_diagnostic(&mut workspace, &uri);
            let actions = editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &context(vec![diagnostic]),
            );
            assert_eq!(actions.len(), 1, "{actions:?}");
            let edit = action_edit(&actions[0], &uri);
            let changed = workspace.documents[&uri]
                .text
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap();
            assert_eq!(
                changed.source,
                format!("😀 {expected}\\label{{section.itnro}}\r\n")
            );
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!()
            };
            assert_eq!(action.title, "Replace with PDF destination `section.intro`");
            assert_eq!(action.is_preferred, Some(false));
        }
    }
}

#[test]
fn destination_fixes_reject_ambiguous_known_stale_and_forged_evidence() {
    for (declarations, mode) in [
        (
            "\\hypertarget{section.intro}{Text}\\hyperdef{section}{intro}{Duplicate}",
            "valid",
        ),
        (
            "\\hypertarget{section.itnro}{Known}\\hypertarget{section.intro}{Other}",
            "valid",
        ),
        ("\\label{section.intro}", "valid"),
        ("\\hypertarget{section.\\intro}{Dynamic}", "valid"),
        ("\\hypertarget{section.intro}{Text}", "range"),
        ("\\hypertarget{section.intro}{Text}", "data"),
        ("\\hypertarget{section.intro}{Text}", "message"),
        ("\\hypertarget{section.intro}{Text}", "code"),
        ("\\hypertarget{section.intro}{Text}", "stale"),
    ] {
        let source = format!("{declarations}\n\\hyperlink{{section.itnro}}{{section.itnro}}\n");
        let (mut workspace, uri) = setup(&source, false);
        let mut diagnostic = destination_diagnostic(&mut workspace, &uri);
        match mode {
            "range" => {
                let start = source.rfind("section.itnro").unwrap();
                diagnostic.range = workspace.documents[&uri].text.range(start, start + 13);
            }
            "data" => diagnostic.data = Some(serde_json::json!({"destination":"forged"})),
            "message" => {
                diagnostic.message =
                    "warning  (pdf backend): unreferenced destination with name 'forged'".into()
            }
            "code" => diagnostic.code = None,
            "stale" => {
                workspace.documents.insert(
                    uri.clone(),
                    Document::new(&uri, source.replace("section.itnro", "changed"), Some(9)),
                );
            }
            _ => {}
        }
        workspace.build_diagnostics.insert(
            uri.clone(),
            [(uri.clone(), vec![diagnostic.clone()])].into(),
        );
        assert!(
            editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &context(vec![diagnostic])
            )
            .is_empty(),
            "{mode}: {declarations}"
        );
    }
}

#[test]
fn legacy_destination_commands_distinguish_compound_replacements_and_revalidate_candidates() {
    let source = "\\hypertarget{sectionx.foo}{X}\\hypertarget{sectiony.foo}{Y}\n\\hyperref{}{sectionz}% keep\r\n{foo}{Text}";
    let (mut workspace, uri) = setup(source, false);
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let diagnostic = destination_diagnostic(&mut workspace, &uri);
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic.clone()]),
    );
    assert_eq!(actions.len(), 2, "{actions:?}");
    let mut replacements = std::collections::BTreeSet::new();
    for action in actions {
        let CodeActionOrCommand::Command(command) = action else {
            panic!()
        };
        let value = command.arguments.unwrap()[0].clone();
        replacements.insert(value["replacement"].as_str().unwrap().to_owned());
        let resolved =
            editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap())
                .unwrap();
        assert_eq!(
            resolved.changes.unwrap()[&uri][0].new_text,
            value["replacement"]
        );
        workspace.documents.get_mut(&uri).unwrap().version = Some(10);
        assert!(
            editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap())
                .is_none()
        );
        workspace.documents.get_mut(&uri).unwrap().version = Some(9);
        let mut forged = value;
        forged["replacement"] = serde_json::json!("}% keep\r\n{forged");
        assert!(
            editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none()
        );
    }
    assert_eq!(
        replacements,
        [
            "}% keep\r\n{sectionx.foo".into(),
            "}% keep\r\n{sectiony.foo".into()
        ]
        .into()
    );
    workspace.clear_build_diagnostics(&uri);
    assert!(editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic])
    )
    .is_empty());
}

#[test]
fn hyperref_label_fixes_preserve_link_text_and_reject_forged_text_ranges() {
    for versioned in [false, true] {
        let source = "\\label{sec:intro}\n\\hyperref[sec:itnro]{sec:itnro}\n";
        let (mut workspace, uri) = setup(source, versioned);
        let text = &workspace.documents[&uri].text;
        let key = source.find("sec:itnro").unwrap();
        let body = source.rfind("sec:itnro").unwrap();
        let body_range = text.range(body, body + 9);
        let mut diagnostic = Diagnostic {
            range: text.range(key, key + 9),
            source: Some("latex-build".into()),
            message:
                "LaTeX Warning: Hyper reference `sec:itnro' on page 1 undefined on input line 2."
                    .into(),
            ..Default::default()
        };
        // hyperref uses a distinct warning prefix, which must still identify a label.
        workspace.build_diagnostics.insert(
            uri.clone(),
            [(uri.clone(), vec![diagnostic.clone()])].into(),
        );
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic.clone()]),
        );
        assert_eq!(actions.len(), 1, "{actions:?}");
        let edit = action_edit(&actions[0], &uri);
        assert_eq!(edit.range, diagnostic.range);
        assert_eq!(edit.new_text, "sec:intro");
        diagnostic.range = body_range;
        workspace.build_diagnostics.insert(
            uri.clone(),
            [(uri.clone(), vec![diagnostic.clone()])].into(),
        );
        assert!(editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic])
        )
        .is_empty());
    }
    let source = "\\label{sec:intro]}\n\\hyperref[sec:intro]{Text}\n";
    let (mut workspace, uri) = setup(source, false);
    let at = source.rfind("sec:intro").unwrap();
    let diagnostic = Diagnostic {
        range: workspace.documents[&uri].text.range(at, at + 9),
        source: Some("latex-build".into()),
        message: "LaTeX Warning: Hyper reference `sec:intro' on page 1 undefined on input line 2."
            .into(),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        uri.clone(),
        [(uri.clone(), vec![diagnostic.clone()])].into(),
    );
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic]),
    );
    assert_eq!(actions.len(), 1, "{actions:?}");
    let edit = action_edit(&actions[0], &uri);
    assert_eq!(edit.new_text, "{sec:intro]}");
    let changed = workspace.documents[&uri]
        .text
        .changed(vec![TextDocumentContentChangeEvent {
            range: Some(edit.range),
            range_length: None,
            text: edit.new_text,
        }])
        .unwrap();
    assert_eq!(
        changed.source,
        "\\label{sec:intro]}\n\\hyperref[{sec:intro]}]{Text}\n"
    );
    let position = changed.position(changed.source.rfind("sec:intro]").unwrap() + 3);
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, changed.source, Some(10)));
    assert_eq!(features::definitions(&workspace, &uri, position).len(), 1);
}

#[test]
fn capitalized_and_named_page_reference_fixes_match_the_warning_kinds_each_star_form_emits() {
    for command in ["Ref", "Ref *", "autopageref", "autopageref *"] {
        for hyper_warning in [false, true] {
            let source = format!("\\label{{sec:intro}}\n\\{command}{{sec:itnro}}\n");
            let (mut workspace, uri) = setup(&source, true);
            let start = source.find("sec:itnro").unwrap();
            let diagnostic = Diagnostic {
                range: workspace.documents[&uri].text.range(start, start + 9),
                source: Some("latex-build".into()),
                message: format!(
                    "LaTeX Warning: {} `sec:itnro' on page 1 undefined on input line 2.",
                    if hyper_warning {
                        "Hyper reference"
                    } else {
                        "Reference"
                    }
                ),
                ..Default::default()
            };
            workspace.build_diagnostics.insert(
                uri.clone(),
                [(uri.clone(), vec![diagnostic.clone()])].into(),
            );
            let actions = editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &context(vec![diagnostic.clone()]),
            );
            if hyper_warning && command.contains('*') {
                assert!(actions.is_empty(), "{command}: {actions:?}");
            } else {
                assert_eq!(actions.len(), 1, "{command}: {actions:?}");
                let edit = action_edit(&actions[0], &uri);
                assert_eq!(edit.range, diagnostic.range);
                assert_eq!(edit.new_text, "sec:intro");
            }
        }
    }
}

#[test]
fn refcount_assignment_label_fixes_select_the_second_argument_and_reject_counter_ranges() {
    for command in [
        "setcounterref",
        "addtocounterref",
        "setcounterpageref",
        "addtocounterpageref",
    ] {
        for versioned in [false, true] {
            let source = format!("\\label{{sec:intro}}\n\\{command}{{sec:itnro}}{{sec:itnro}}\n");
            let (mut workspace, uri) = setup(&source, versioned);
            let text = &workspace.documents[&uri].text;
            let second = source.rfind("sec:itnro").unwrap();
            let first = source.find("sec:itnro").unwrap();
            let first_range = text.range(first, first + 9);
            let mut diagnostic = Diagnostic {
                range: text.range(second, second + 9),
                source: Some("latex-build".into()),
                message:
                    "LaTeX Warning: Reference `sec:itnro' on page 1 undefined on input line 2."
                        .into(),
                ..Default::default()
            };
            workspace.build_diagnostics.insert(
                uri.clone(),
                [(uri.clone(), vec![diagnostic.clone()])].into(),
            );
            let actions = editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &context(vec![diagnostic.clone()]),
            );
            assert_eq!(actions.len(), 1, "{actions:?}");
            let edit = action_edit(&actions[0], &uri);
            assert_eq!(edit.range, diagnostic.range);
            assert_eq!(edit.new_text, "sec:intro");
            let changed = workspace.documents[&uri]
                .text
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap()
                .source;
            assert_eq!(
                changed,
                format!("\\label{{sec:intro}}\n\\{command}{{sec:itnro}}{{sec:intro}}\n")
            );
            diagnostic.range = first_range;
            workspace.build_diagnostics.insert(
                uri.clone(),
                [(uri.clone(), vec![diagnostic.clone()])].into(),
            );
            assert!(editing::code_actions(
                &workspace,
                &uri,
                diagnostic.range,
                &context(vec![diagnostic])
            )
            .is_empty());
        }
    }
}

#[test]
fn quick_fixes_repair_braces_and_environments_including_comments_raw_text_and_crlf() {
    for source in [
        "😀 }",
        "{中文",
        "{content % trailing comment",
        "\\begin{equation}\nx=1\n",
        "\\begin{equation}\r\nx=1",
        "\\begin{itemize}\n\\begin{equation}\nx=1\n\\end{itemize}",
        "\\end{itemize}",
        "\\begin{verbatim}\n{ % raw",
    ] {
        for versioned in [false, true] {
            let (workspace, uri) = setup(source, versioned);
            let diagnostics = features::diagnostics(&workspace.documents[&uri]);
            assert_eq!(diagnostics.len(), 1, "{source}");
            let actions = editing::code_actions(
                &workspace,
                &uri,
                diagnostics[0].range,
                &context(diagnostics.clone()),
            );
            assert_eq!(actions.len(), 1, "{source}");
            let edit = action_edit(&actions[0], &uri);
            if source.contains("\r\n") {
                assert!(edit.new_text.contains("\r\n"));
            }
            let repaired = Text::new(source.into())
                .changed(vec![TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                }])
                .unwrap();
            let document = Document::new(&uri, repaired.source, Some(10));
            assert!(
                features::diagnostics(&document).is_empty(),
                "{source}: {:?}",
                document.syntax.diagnostics
            );
        }
    }
}

#[test]
fn fixes_filter_requested_kinds_ranges_and_stale_or_forged_diagnostics() {
    let (mut workspace, uri) = setup("ok\n{broken", false);
    let diagnostic = features::diagnostics(&workspace.documents[&uri])[0].clone();
    let mut request = context(vec![diagnostic.clone()]);
    request.only = Some(vec![CodeActionKind::REFACTOR]);
    assert!(editing::code_actions(&workspace, &uri, diagnostic.range, &request).is_empty());
    request.only = Some(vec![CodeActionKind::QUICKFIX]);
    assert_eq!(
        editing::code_actions(&workspace, &uri, diagnostic.range, &request).len(),
        1
    );
    assert!(editing::code_actions(&workspace, &uri, Range::default(), &request).is_empty());
    request.diagnostics[0].source = Some("latex-build".into());
    assert!(editing::code_actions(&workspace, &uri, diagnostic.range, &request).is_empty());
    request.diagnostics = vec![diagnostic.clone()];
    request.diagnostics[0].data = Some(serde_json::json!({"edit":{"newText":"malicious"}}));
    let action = editing::code_actions(&workspace, &uri, diagnostic.range, &request);
    assert_eq!(action_edit(&action[0], &uri).new_text, "}");
    workspace.documents.insert(
        uri.clone(),
        Document::new(&uri, "ok\n{fixed}".into(), Some(10)),
    );
    assert!(editing::code_actions(&workspace, &uri, diagnostic.range, &request).is_empty());
}

#[test]
fn highlights_distinguish_local_declarations_and_references_and_skip_opaque_text() {
    let source =
        "\\label{sec:x} \\ref{sec:x} \\cref{other,sec:x}\n% \\ref{sec:x}\n\\verb|\\ref{sec:x}|";
    let (workspace, uri) = setup(source, false);
    let position = workspace.documents[&uri]
        .text
        .position(source.find("ref{sec:x}").unwrap() + 6);
    let highlights = editing::highlights(&workspace, &uri, position);
    assert_eq!(highlights.len(), 3);
    assert_eq!(
        highlights
            .iter()
            .filter(|h| h.kind == Some(DocumentHighlightKind::WRITE))
            .count(),
        1
    );
    assert_eq!(
        highlights
            .iter()
            .filter(|h| h.kind == Some(DocumentHighlightKind::READ))
            .count(),
        2
    );
}

#[test]
fn selection_ranges_expand_through_nested_groups_sections_and_the_document() {
    let source = "\\section{Intro}\r\n😀 \\textbf{中文 {nested words}}\r\n% comment\r\n\\section{Next}\r\nend\r\n";
    let (workspace, uri) = setup(source, false);
    let doc = &workspace.documents[&uri];
    let word = source.find("nested").unwrap();
    let positions = [
        doc.text.position(word + 2),
        Position::new(1, 2),
        doc.text.position(source.len()),
    ];
    let selections = editing::selection_ranges(doc, &positions).unwrap();
    assert_eq!(selections.len(), positions.len());
    assert_eq!(selections[0].range, doc.text.range(word, word + 6));
    let mut found_group = false;
    let mut found_command = false;
    for (selection, position) in selections.iter().zip(positions) {
        let mut current = selection;
        assert!(current.range.start <= position && position <= current.range.end);
        while let Some(parent) = &current.parent {
            assert!(
                parent.range.start <= current.range.start && current.range.end <= parent.range.end
            );
            assert_ne!(parent.range, current.range);
            let start = doc.text.offset(parent.range.start).unwrap();
            let end = doc.text.offset(parent.range.end).unwrap();
            found_group |= &source[start..end] == "{nested words}";
            found_command |= &source[start..end] == "\\textbf{中文 {nested words}}";
            current = parent;
        }
        assert_eq!(current.range, doc.text.range(0, source.len()));
    }
    assert!(found_group && found_command);
    assert!(editing::selection_ranges(doc, &[Position::new(1, 1)]).is_none());
    let (workspace, uri) = setup("", false);
    assert_eq!(
        editing::selection_ranges(&workspace.documents[&uri], &[Position::default()]).unwrap()[0]
            .range,
        Range::default()
    );
}

#[test]
fn compiler_reference_fixes_use_current_project_keys_and_revalidate_command_requests() {
    let source = "😀 \\cref{sec:itnro, sec:other} \\cite{smith2025}\n";
    let (mut workspace, uri) = setup(source, true);
    let labels = Url::parse("file:///editing/chapter.tex").unwrap();
    let bibliography = Url::parse("file:///editing/refs.bib").unwrap();
    let unrelated = Url::parse("file:///unrelated/main.tex").unwrap();
    for (uri, source) in [
        (&labels, r"\label{sec:intro}\label{sec:other}"),
        (&bibliography, "@book{smith2026,title={Example}}"),
        (&unrelated, r"\label{sec:itnro}\label{sec:itnor}"),
    ] {
        workspace
            .documents
            .insert(uri.clone(), Document::new(uri, source.into(), None));
    }
    workspace.recorded_inputs.insert(
        uri.clone(),
        [uri.clone(), labels.clone(), bibliography].into(),
    );
    let text = &workspace.documents[&uri].text;
    let label_start = source.find("sec:itnro").unwrap();
    let citation_start = source.find("smith2025").unwrap();
    let label = Diagnostic {
        range: text.range(label_start, label_start + 9),
        source: Some("latex-build".into()),
        message: "LaTeX Warning: Reference `sec:itnro' on page 1 undefined on input line 1.".into(),
        ..Default::default()
    };
    let citation = Diagnostic {
        range: text.range(citation_start, citation_start + 9),
        source: Some("latex-build".into()),
        message: "LaTeX Warning: Citation `smith2025' on page 1 undefined on input line 1.".into(),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        uri.clone(),
        std::collections::BTreeMap::from([(uri.clone(), vec![label.clone(), citation.clone()])]),
    );
    let actions =
        editing::code_actions(&workspace, &uri, label.range, &context(vec![label.clone()]));
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(action_edit(&actions[0], &uri).new_text, "sec:intro");
    let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
        panic!();
    };
    assert_eq!(action.is_preferred, Some(false));
    let actions = editing::code_actions(
        &workspace,
        &uri,
        citation.range,
        &context(vec![citation.clone()]),
    );
    assert_eq!(actions.len(), 1);
    assert_eq!(action_edit(&actions[0], &uri).new_text, "smith2026");
    let mut forged = label.clone();
    forged.message =
        "LaTeX Warning: Reference `forged' on page 1 undefined on input line 1.".into();
    assert!(
        editing::code_actions(&workspace, &uri, label.range, &context(vec![forged])).is_empty()
    );
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let actions =
        editing::code_actions(&workspace, &uri, label.range, &context(vec![label.clone()]));
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!();
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    let request = serde_json::from_value(value.clone()).unwrap();
    assert!(editing::resolve_fix(&workspace, request).is_some());
    let mut forged = value.clone();
    forged["replacement"] = serde_json::json!("sec:itnor");
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace.documents.get_mut(&uri).unwrap().version = Some(9);
    workspace.documents.remove(&labels);
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace.clear_build_diagnostics(&uri);
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(value).unwrap()).is_none());
}

#[test]
fn compiler_reference_suggestions_reject_ambiguous_dynamic_known_and_inexact_keys() {
    for (key, definitions, expected, precise) in [
        (
            "sec:valve",
            r"\label{sec:value}\label{sec:value}",
            None,
            true,
        ),
        (
            "sec:value",
            r"\label{sec:value}\label{sec:valve}",
            None,
            true,
        ),
        ("sec:value", r"\label{sec:\value}", None, true),
        (r"sec:\value", r"\label{sec:value}", None, true),
        ("ab", r"\label{ac}", None, true),
        ("章节序眼", r"\label{章节序言}", Some("章节序言"), true),
        ("sec:valve", r"\label{sec:value}", None, false),
    ] {
        let source = format!("\\ref{{{key}}}");
        let (mut workspace, uri) = setup(&source, false);
        let definitions_uri = Url::parse("file:///editing/definitions.tex").unwrap();
        workspace.documents.insert(
            definitions_uri.clone(),
            Document::new(&definitions_uri, definitions.into(), None),
        );
        workspace
            .recorded_inputs
            .insert(uri.clone(), [uri.clone(), definitions_uri].into());
        let range = if precise {
            workspace.documents[&uri].text.range(5, 5 + key.len())
        } else {
            workspace.documents[&uri].text.range(0, source.len())
        };
        let diagnostic = Diagnostic {
            range,
            source: Some("latex-build".into()),
            message: format!(
                "LaTeX Warning: Reference `{key}' on page 1 undefined on input line 1."
            ),
            ..Default::default()
        };
        workspace.build_diagnostics.insert(
            uri.clone(),
            std::collections::BTreeMap::from([(uri.clone(), vec![diagnostic.clone()])]),
        );
        let actions = editing::code_actions(&workspace, &uri, range, &context(vec![diagnostic]));
        if let Some(expected) = expected {
            assert_eq!(actions.len(), 1);
            assert_eq!(action_edit(&actions[0], &uri).new_text, expected);
        } else {
            assert!(actions.is_empty(), "{key}: {actions:?}");
        }
    }
}

fn undefined_command_diagnostic(workspace: &mut Workspace, uri: &Url, name: &str) -> Diagnostic {
    let text = &workspace.documents[uri].text;
    let start = text.source.find(&format!("\\{name}")).unwrap();
    let diagnostic = Diagnostic {
        range: text.range(start, start + name.len() + 1),
        source: Some("latex-build".into()),
        message: "Undefined control sequence.".into(),
        code: Some(NumberOrString::String("undefined-control-sequence".into())),
        data: Some(serde_json::json!({"command":name})),
        ..Default::default()
    };
    workspace.build_diagnostics.insert(
        uri.clone(),
        std::collections::BTreeMap::from([(uri.clone(), vec![diagnostic.clone()])]),
    );
    diagnostic
}

#[test]
fn undefined_command_fixes_use_builtin_and_project_commands_preserving_arguments_and_utf16() {
    for (name, replacement) in [
        ("sectoin", "section"),
        ("reporttitel", "reporttitle"),
        ("documentwidht", "documentwidth"),
        ("ifdrafft", "ifdraft"),
        ("thecliam", "theclaim"),
        ("endpanle", "endpanel"),
        ("wriet", "write"),
        ("escapcehar", "escapechar"),
    ] {
        let source = format!("😀 \\{name}{{Existing}} trailing text.\n");
        let (mut workspace, uri) = setup(&source, true);
        let declarations = Url::parse("file:///editing/definitions.tex").unwrap();
        let unrelated = Url::parse("file:///unrelated/main.tex").unwrap();
        workspace.documents.insert(declarations.clone(), Document::new(&declarations, r"\newcommand{\reporttitle}[1]{#1}\newlength{\documentwidth}\newif\ifdraft\newtheorem{claim}{Claim}\newenvironment{panel}{}{}".into(), None));
        workspace.documents.insert(
            unrelated.clone(),
            Document::new(&unrelated, format!("\\newcommand{{\\{name}}}{{}}"), None),
        );
        workspace
            .recorded_inputs
            .insert(uri.clone(), [uri.clone(), declarations].into());
        let diagnostic = undefined_command_diagnostic(&mut workspace, &uri, name);
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic]),
        );
        let expected = format!("\\{replacement}");
        let action = actions
            .iter()
            .find(|action| action_edit(action, &uri).new_text == expected)
            .unwrap_or_else(|| panic!("{name}: {actions:?}"));
        let edit = action_edit(action, &uri);
        let changed = workspace.documents[&uri]
            .text
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text,
            }])
            .unwrap();
        assert_eq!(
            changed.source,
            format!("😀 \\{replacement}{{Existing}} trailing text.\n")
        );
        let CodeActionOrCommand::CodeAction(action) = action else {
            panic!()
        };
        assert_eq!(action.is_preferred, Some(false));
        assert!(action.title.contains(&expected));
        assert!(actions.len() <= 5);
    }
}

#[test]
fn command_spelling_fixes_reject_known_ambiguous_ignored_and_unproven_names() {
    for (source, name, definitions, mode) in [
        (r"\section{Title}", "section", "", "precise"),
        (r"\write{Known primitive}", "write", "", "precise"),
        (
            r"\reporttitle{Title}",
            "reporttitle",
            r"\newcommand{\reporttitle}[1]{#1}",
            "precise",
        ),
        (
            r"\reporttitel{Title}",
            "reporttitel",
            r"\newcommand{\reporttitle}[1]{#1}\newcommand{\reporttitle}[1]{#1}",
            "precise",
        ),
        (r"% \sectoin{Title}", "sectoin", "", "precise"),
        (r"\verb|\sectoin{Title}|", "sectoin", "", "precise"),
        (r"\fr{Title}", "fr", "", "precise"),
        (r"\sectoin{Title}", "sectoin", "", "missing-data"),
        (r"\sectoin{Title}", "sectoin", "", "wrong-data"),
        (r"\sectoin{Title}", "sectoin", "", "line-range"),
        (r"\sectoin{Title}", "sectoin", "", "macro-expansion"),
    ] {
        let (mut workspace, uri) = setup(source, false);
        let declarations = Url::parse("file:///editing/definitions.tex").unwrap();
        workspace.documents.insert(
            declarations.clone(),
            Document::new(&declarations, definitions.into(), None),
        );
        workspace
            .recorded_inputs
            .insert(uri.clone(), [uri.clone(), declarations].into());
        let mut diagnostic = undefined_command_diagnostic(&mut workspace, &uri, name);
        match mode {
            "missing-data" => diagnostic.data = None,
            "wrong-data" => diagnostic.data = Some(serde_json::json!({"command":"other"})),
            "line-range" => {
                diagnostic.range = workspace.documents[&uri].text.range(0, source.len())
            }
            "macro-expansion" => diagnostic.code = None,
            _ => {}
        }
        workspace
            .build_diagnostics
            .get_mut(&uri)
            .unwrap()
            .get_mut(&uri)
            .unwrap()[0] = diagnostic.clone();
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &context(vec![diagnostic]),
        );
        assert!(actions.is_empty(), "{name}/{mode}: {actions:?}");
    }
}

#[test]
fn command_spelling_requests_revalidate_version_diagnostic_and_replacement() {
    let (mut workspace, uri) = setup(r"\sectoin{Existing}", true);
    workspace.code_action_literals = false;
    workspace.apply_edits = true;
    let diagnostic = undefined_command_diagnostic(&mut workspace, &uri, "sectoin");
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &context(vec![diagnostic]),
    );
    let CodeActionOrCommand::Command(command) = &actions[0] else {
        panic!()
    };
    let value = command.arguments.as_ref().unwrap()[0].clone();
    assert_eq!(value["replacement"], "\\section");
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_some()
    );
    let mut forged = value.clone();
    forged["replacement"] = serde_json::json!("\\subsection");
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(forged).unwrap()).is_none());
    workspace.documents.get_mut(&uri).unwrap().version = Some(10);
    assert!(
        editing::resolve_fix(&workspace, serde_json::from_value(value.clone()).unwrap()).is_none()
    );
    workspace.documents.get_mut(&uri).unwrap().version = Some(9);
    workspace.clear_build_diagnostics(&uri);
    assert!(editing::resolve_fix(&workspace, serde_json::from_value(value).unwrap()).is_none());
}
