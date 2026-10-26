use lsp_types::*;
use texquill::{
    compilation::build_log,
    editing,
    language::{features, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};

fn insert(workspace: &mut Workspace, name: &str, source: &str) -> Url {
    let uri = Url::parse(&format!("file:///subref/{name}")).unwrap();
    workspace
        .documents
        .insert(uri.clone(), Document::new(&uri, source.into(), Some(3)));
    uri
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

#[test]
fn subfigure_references_share_original_labels_without_merging_prefixed_label_names() {
    let mut workspace = Workspace::default();
    let declarations = r"\label{fig:left}\label{sub@fig:left}";
    let child = insert(&mut workspace, "figures.tex", declarations);
    let valid = "😀 \\ref{fig:left}\n\\subref{fig:left}\n\\subref % star\r\n * {fig:left}\n";
    let untouched = "\\ref{sub@fig:left}\n\\subref{sub@fig:left}\n% \\subref{fig:left}\n\\verb|\\subref{fig:left}|\n";
    let source = format!("{valid}{untouched}");
    let main = insert(&mut workspace, "main.tex", &source);
    let at = workspace.documents[&child]
        .text
        .position(declarations.find("fig:left").unwrap() + 3);
    assert_eq!(features::references(&workspace, &child, at, false).len(), 3);
    for (offset, _) in valid.match_indices("{fig:left}") {
        let point = workspace.documents[&main].text.position(offset + 4);
        let definitions = features::definitions(&workspace, &main, point);
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].uri, child);
        assert!(features::hover(&workspace, &main, point).is_some());
        assert!(features::prepare_rename(&workspace, &main, point)
            .unwrap()
            .is_some());
    }
    let edit = features::rename(&workspace, &child, at, "fig:renamed")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(
        apply(&source, edit[&main].clone()),
        format!(
            "{}{untouched}",
            valid.replace("{fig:left}", "{fig:renamed}")
        )
    );
    assert_eq!(
        apply(declarations, edit[&child].clone()),
        r"\label{fig:renamed}\label{sub@fig:left}"
    );
}

#[test]
fn subfigure_reference_completion_signatures_and_snippets_preserve_stars() {
    let mut workspace = Workspace::default();
    insert(&mut workspace, "figures.tex", r"\label{fig:left}");
    for marked in [r"\subref{fig:l¦BAD}", "\\subref % comment\n*{fig:l¦BAD}"] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        let main = insert(&mut workspace, "main.tex", &source);
        let point = Text::new(source.clone()).position(offset);
        let items = features::completion(&workspace, &main, point);
        let item = items.iter().find(|item| item.label == "fig:left").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing label completion")
        };
        assert_eq!(
            apply(&source, vec![edit.clone()]),
            source.replace("fig:lBAD", "fig:left")
        );
        let help = signature::help(&workspace, &main, point).unwrap();
        assert_eq!(help.signatures[0].label, r"\subref{keys}");
        assert_eq!(help.active_parameter, Some(0));
    }
    for source in [
        r"\subref[note]{fig:left}",
        r"\subref**{fig:left}",
        r"\subref{\dynamic}",
    ] {
        let main = insert(&mut workspace, "invalid.tex", source);
        let point = workspace.documents[&main]
            .text
            .position(source.rfind('{').unwrap() + 2);
        assert!(features::definitions(&workspace, &main, point).is_empty());
        assert!(features::prepare_rename(&workspace, &main, point)
            .unwrap()
            .is_none());
    }
    workspace.client.snippets = true;
    let main = insert(&mut workspace, "snippet.tex", r"\subr");
    let items = features::completion(&workspace, &main, Position::new(0, 5));
    let item = items.iter().find(|item| item.label == "subref").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
        panic!("missing command snippet")
    };
    assert_eq!(edit.new_text, "subref{${1:key}}");
}

#[test]
fn subfigure_warning_prefixes_select_source_keys_and_repair_their_own_namespace() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    for (call, prefix, key, replacement) in [
        (
            r"\subref{fig:lef}",
            "Reference",
            "sub@fig:lef",
            Some("fig:left"),
        ),
        (
            r"\subref*{fig:lef}",
            "Reference",
            "sub@fig:lef",
            Some("fig:left"),
        ),
        (
            r"\subref{fig:lef}",
            "Hyper reference",
            "fig:lef",
            Some("fig:left"),
        ),
        (r"\subref*{fig:lef}", "Hyper reference", "fig:lef", None),
        (r"\subref{fig:lef}", "Reference", "fig:lef", None),
        (r"\ref{fig:lef}", "Reference", "sub@fig:lef", None),
        (
            r"\ref{sub@fig:lef}",
            "Reference",
            "sub@fig:lef",
            Some("sub@fig:left"),
        ),
        (
            r"\subref{sub@fig:lef}",
            "Reference",
            "sub@sub@fig:lef",
            Some("sub@fig:left"),
        ),
    ] {
        let source = format!("\\label{{fig:left}}\\label{{sub@fig:left}}\n😀 {call}\n");
        std::fs::write(&main, &source).unwrap();
        let log = format!(
            "{}:2: LaTeX Warning: {prefix} `{key}' on page 1 undefined on input line 2.\n",
            main.display()
        );
        let diagnostics = build_log::parse(&main, &log);
        let diagnostic = &diagnostics[&uri][0];
        let start = source.rfind('{').unwrap() + 1;
        let expected = Text::new(source.clone()).range(start, source.rfind('}').unwrap());
        let mut workspace = Workspace {
            client: texquill::project::state::ClientCapabilities {
                code_action_literals: true,
                ..Default::default()
            },
            ..Default::default()
        };
        workspace.open(uri.clone(), source.clone(), 3);
        workspace
            .project
            .build_diagnostics
            .insert(uri.clone(), diagnostics.clone());
        let actions = editing::code_actions(
            &workspace,
            &uri,
            diagnostic.range,
            &CodeActionContext {
                diagnostics: vec![diagnostic.clone()],
                only: None,
                trigger_kind: None,
            },
        );
        if let Some(replacement) = replacement {
            assert_eq!(diagnostic.range, expected, "{call}/{prefix}");
            assert_eq!(actions.len(), 1, "{call}: {actions:?}");
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!("missing correction")
            };
            let edit = &action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri][0];
            assert_eq!(edit.range, expected);
            assert_eq!(edit.new_text, replacement);
        } else {
            assert_ne!(diagnostic.range, expected, "{call}/{prefix}");
            assert!(actions.is_empty(), "{call}: {actions:?}");
        }
    }
}

#[test]
fn literal_and_generated_subfigure_warning_names_keep_distinct_source_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "😀 \\subref{fig:lef} \\ref{sub@fig:lef}\n";
    std::fs::write(&main, source).unwrap();
    let log = format!(
        "{}:1: LaTeX Warning: Reference `sub@fig:lef' on page 1 undefined on input line 1.\n",
        main.display()
    );
    let diagnostics = build_log::parse(&main, &log);
    let mut actual: Vec<_> = diagnostics[&uri].iter().map(|item| item.range).collect();
    actual.sort_by_key(|range| range.start);
    let text = Text::new(source.into());
    let first = source.find("fig:lef").unwrap();
    let second = source.find("sub@fig:lef").unwrap();
    assert_eq!(
        actual,
        vec![
            text.range(first, first + 7),
            text.range(second, second + 11)
        ]
    );
}

#[test]
#[ignore = "requires pdflatex, subcaption and hyperref"]
fn real_subfigure_warning_repairs_and_label_renames_keep_both_auxiliary_labels() {
    use std::sync::atomic::AtomicBool;
    use texquill::{compilation::build, config::BuildConfig};

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let figures = root.join("figures.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let child = Url::from_file_path(&figures).unwrap();
    let source = "\\documentclass{article}\n\\usepackage{subcaption}\n\\usepackage{hyperref}\n\\begin{document}\n\\input{figures}\n\\ref{fig:left}; \\subref{fig:left}; \\subref*{fig:lef}.\n\\end{document}\n";
    std::fs::write(&main, source).unwrap();
    std::fs::write(&figures, "\\begin{figure}\\begin{subfigure}{.4\\linewidth}\\centering A\\caption{First}\\label{fig:left}\\end{subfigure}\\caption{Pair}\\end{figure}\n").unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let compile = || {
        let first = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(first.status, "success", "{}", first.output);
        let second = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(second.status, "success", "{}", second.output);
        second
    };
    let before = compile();
    let diagnostic = before.diagnostics[&uri]
        .iter()
        .find(|item| item.message.contains("Reference `sub@fig:lef'"))
        .unwrap()
        .clone();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace.open(uri.clone(), source.into(), 3);
    workspace
        .project
        .build_diagnostics
        .insert(uri.clone(), before.diagnostics);
    let actions = editing::code_actions(
        &workspace,
        &uri,
        diagnostic.range,
        &CodeActionContext {
            diagnostics: vec![diagnostic],
            only: None,
            trigger_kind: None,
        },
    );
    assert_eq!(actions.len(), 1);
    let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
        panic!("missing correction")
    };
    let edits = action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri].clone();
    let repaired = apply(source, edits);
    std::fs::write(&main, &repaired).unwrap();
    workspace.open(uri.clone(), repaired, 4);
    for (version, name) in ["fig:updated", "fig:final"].into_iter().enumerate() {
        let source = &workspace.documents[&child].text.source;
        let at = workspace.documents[&child]
            .text
            .position(source.find("\\label{").unwrap() + "\\label{fig".len());
        assert_eq!(features::references(&workspace, &child, at, false).len(), 3);
        let edits = features::rename(&workspace, &child, at, name)
            .unwrap()
            .unwrap()
            .changes
            .unwrap();
        for (document, edits) in edits {
            let updated = apply(&workspace.documents[&document].text.source, edits);
            std::fs::write(document.to_file_path().unwrap(), &updated).unwrap();
            workspace.open(document, updated, version as i32 + 5);
        }
        let result = compile();
        assert!(
            !result
                .diagnostics
                .values()
                .flatten()
                .any(|item| item.message.contains("undefined")),
            "{}",
            result.output
        );
        let aux = std::fs::read_to_string(result.pdf.unwrap().with_extension("aux")).unwrap();
        assert!(
            aux.contains(&format!("\\newlabel{{{name}}}{{{{1a}}")),
            "{aux}"
        );
        assert!(
            aux.contains(&format!("\\newlabel{{sub@{name}}}{{{{a}}")),
            "{aux}"
        );
    }
}
