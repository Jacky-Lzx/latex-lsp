use latex_lsp::{
    language::{features, signature},
    project::workspace::Workspace,
};
use lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:hook-conditions.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.snippets = true;
    workspace.open(uri.clone(), source.into(), 5);
    (workspace, uri)
}

fn conditions() -> [&'static str; 11] {
    [
        r"\NewHook{custom}\IfHookEmptyTF{custom}{\let\ifblank\identity}{}",
        r"\NewHook{custom}\IfHookEmptyT{custom}{\let\ifblank\identity}",
        r"\NewHook{custom}\AddToHook{custom}{Text}\IfHookEmptyF{custom}{\let\ifblank\identity}",
        r"\NewHook{custom}\IfHookExistsTF{custom}{\let\ifblank\identity}{}",
        r"\NewHook{custom}\csuse{IfHookEmptyTF}{custom}{\let\ifblank\identity}{}",
        r"\NewHook{custom}\csname IfHookEmptyTF\endcsname{custom}{\let\ifblank\identity}{}",
        r"\NewHook{custom}\def\choose{\IfHookEmptyTF}\choose{custom}{\let\ifblank\identity}{}",
        r"\NewHook{custom}\let\choose\IfHookEmptyTF\choose{custom}{\let\ifblank\identity}{}",
        r"\NewHook{custom}\let\savedblank\ifblank\IfHookEmptyTF{custom}{\let\ifblank\identity}{\let\ifblank\savedblank}",
        r"\NewHook{custom}\def\other{Before}\IfHookEmptyTF{custom}{\patchcmd{\other}{Before}{After}{\let\ifblank\identity}{}}{}",
        r"\NewHook{custom}\def\other{Before}\IfHookEmptyTF{custom}{\identity{\patchcmd{\other}{Before}{After}{\let\ifblank\identity}{}}}{}",
    ]
}

fn source(condition: &str) -> String {
    format!(
        r"\documentclass{{article}}\usepackage{{etoolbox}}\newcommand\word{{\typeout{{LSP-WORD}}}}\def\identity#1{{#1}}\begin{{document}}{condition}\ifblank{{\word}}{{\word}}{{\word}}\label{{safe}}\ref{{safe}}\end{{document}}"
    )
}

fn checked_macro_edits(workspace: &Workspace, uri: &Url, source: &str) -> Option<WorkspaceEdit> {
    let at = workspace.documents[uri]
        .text
        .position(source.rfind("word").unwrap() + 1);
    match features::rename(workspace, uri, at, "term") {
        Ok(Some(edit)) => {
            assert_eq!(edit.changes.as_ref().unwrap()[uri].len(), 4, "{source}");
            Some(edit)
        }
        _ => None,
    }
}

#[test]
fn hook_condition_branches_never_expose_partial_string_reader_renames() {
    for condition in conditions() {
        let source = source(condition);
        let (workspace, uri) = setup(&source);
        checked_macro_edits(&workspace, &uri, &source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("safe").unwrap() + 1);
        assert_eq!(
            features::rename(&workspace, &uri, at, "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            2
        );
    }
}

#[test]
fn hook_conditions_have_native_signatures_and_readonly_name_navigation() {
    for (name, tail, suffix, active) in [
        (
            "IfHookEmptyTF",
            "{custom}{True}{Fal¦se}",
            "{hook}{true code}{false code}",
            2,
        ),
        ("IfHookEmptyT", "{custom}{Tr¦ue}", "{hook}{true code}", 1),
        ("IfHookEmptyF", "{custom}{Fal¦se}", "{hook}{false code}", 1),
        (
            "IfHookExistsTF",
            "{custom}{True}{Fal¦se}",
            "{hook}{true code}{false code}",
            2,
        ),
    ] {
        for (preamble, call) in [
            (String::new(), format!(r"\{name}")),
            (String::new(), format!(r"\csuse{{{name}}}")),
            (String::new(), format!(r"\csname {name}\endcsname")),
            (format!(r"\def\choose{{\{name}}}"), r"\choose".into()),
        ] {
            let marked = format!(r"\NewHook{{custom}}{preamble}{call}{tail}");
            let at = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            let (workspace, uri) = setup(&source);
            let text = &workspace.documents[&uri].text;
            let help = signature::help(&workspace, &uri, text.position(at)).unwrap();
            assert_eq!(help.signatures[0].label, format!("{call}{suffix}"));
            assert_eq!(help.active_parameter, Some(active));
            let name_at = text.position(source.rfind("custom").unwrap() + 2);
            assert_eq!(
                features::definitions(&workspace, &uri, name_at),
                [Location {
                    uri: uri.clone(),
                    range: text.range(9, 15)
                }]
            );
        }
    }
}

#[test]
fn hook_conditions_keep_raw_names_and_offer_exact_command_snippets() {
    for (name, snippet) in [
        (
            "IfHookEmptyTF",
            "{${1:hook}}{${2:true code}}{${3:false code}}",
        ),
        ("IfHookEmptyT", "{${1:hook}}{${2:true code}}"),
        ("IfHookEmptyF", "{${1:hook}}{${2:false code}}"),
        (
            "IfHookExistsTF",
            "{${1:hook}}{${2:true code}}{${3:false code}}",
        ),
    ] {
        let source = format!(r"\{name}");
        let (workspace, uri) = setup(&source);
        let items = features::completion(
            &workspace,
            &uri,
            workspace.documents[&uri].text.position(source.len()),
        );
        let item = items.iter().find(|item| item.label == name).unwrap();
        assert_eq!(item.insert_text_format, Some(InsertTextFormat::SNIPPET));
        assert_eq!(
            item.text_edit,
            Some(CompletionTextEdit::Edit(TextEdit {
                range: workspace.documents[&uri].text.range(1, source.len()),
                new_text: format!("{name}{snippet}"),
            }))
        );

        for hook in ["custom", " custom "] {
            let source = format!(r"\NewHook{{ custom }}\{name}{{{hook}}}{{True}}{{False}}");
            let (workspace, uri) = setup(&source);
            let text = &workspace.documents[&uri].text;
            let at = text.position(source.rfind("custom").unwrap() + 2);
            let definitions = features::definitions(&workspace, &uri, at);
            if hook == "custom" {
                assert_eq!(definitions.len(), 1);
                assert_eq!(definitions[0].range, text.range(10, 16));
            } else {
                assert!(definitions.is_empty(), "{source}");
            }
        }
    }
}

#[test]
fn ordinary_condition_branches_keep_structural_references_and_exact_completed_boundaries() {
    for (name, branches) in [
        ("IfHookEmptyTF", r"{\ref{safe}}{\ref{safe}}"),
        ("IfHookExistsTF", r"{\ref{safe}}{\ref{safe}}"),
        ("IfHookEmptyT", r"{\ref{safe}}"),
        ("IfHookEmptyF", r"{\ref{safe}}"),
    ] {
        let source =
            format!(r"\NewHook{{custom}}\label{{safe}}\{name}{{custom}}{branches}\ref{{safe}}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let expected = source.match_indices("safe").count();
        for (at, _) in source.match_indices(r"\ref{safe}") {
            assert_eq!(
                features::definitions(&workspace, &uri, text.position(at + 6)).len(),
                1,
                "{source}"
            );
        }
        let at = text.position(source.rfind("safe").unwrap() + 1);
        assert_eq!(
            features::rename(&workspace, &uri, at, "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            expected,
            "{source}"
        );
    }
}

#[test]
fn incomplete_condition_branches_protect_reference_tokens() {
    for name in [
        "IfHookEmptyTF",
        "IfHookEmptyT",
        "IfHookEmptyF",
        "IfHookExistsTF",
    ] {
        for call in [
            format!(r"\{name}"),
            format!(r"\csuse{{{name}}}"),
            format!(r"\csname {name}\endcsname"),
        ] {
            let source = format!(r"\label{{safe}}{call}{{custom}}{{\ref{{safe}}");
            let (workspace, uri) = setup(&source);
            let at = workspace.documents[&uri]
                .text
                .position(source.rfind("safe").unwrap() + 1);
            assert!(
                features::definitions(&workspace, &uri, at).is_empty(),
                "{source}"
            );
            assert!(
                !matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
                "{source}"
            );
        }
    }
}

#[test]
fn dormant_and_unrelated_conditions_preserve_native_string_reader_edits() {
    for condition in [
        r"\NewHook{custom}\IfHookEmptyTF{custom}{Text}{Other}",
        r"\NewHook{custom}\def\other{}\IfHookEmptyTF{custom}{\let\other\identity}{}",
        r"\NewHook{custom}\newcommand\later{\IfHookEmptyTF{custom}{\let\ifblank\identity}{}}",
    ] {
        let source = source(condition);
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("word").unwrap() + 1);
        assert_eq!(
            features::rename(&workspace, &uri, at, "term")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            3,
            "{source}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_hook_condition_mutations_preserve_execution_and_complete_independent_edits() {
    for condition in conditions() {
        let directory = tempfile::tempdir().unwrap();
        let source = source(condition);
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let compile = |source: &str| {
            std::fs::write(directory.path().join("main.tex"), source).unwrap();
            let aux = directory.path().join("main.aux");
            if aux.exists() {
                std::fs::remove_file(&aux).unwrap();
            }
            for _ in 0..2 {
                let output = std::process::Command::new("pdflatex")
                    .args([
                        "-no-shell-escape",
                        "-interaction=nonstopmode",
                        "-halt-on-error",
                        "main.tex",
                    ])
                    .current_dir(directory.path())
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{condition}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            assert_eq!(
                log.lines().filter(|line| *line == "LSP-WORD").count(),
                3,
                "{condition}"
            );
            assert!(!log.contains("undefined"), "{log}");
            std::fs::read_to_string(aux).unwrap()
        };
        let before = compile(&source);
        let apply = |edit: WorkspaceEdit| {
            let mut edits = edit.changes.unwrap()[&uri].clone();
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
            text.clone()
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
        };
        if let Some(edit) = checked_macro_edits(&workspace, &uri, &source) {
            assert_eq!(compile(&apply(edit)), before);
        }
        let at = text.position(source.rfind("safe").unwrap() + 1);
        let edit = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap();
        assert_eq!(edit.changes.as_ref().unwrap()[&uri].len(), 2);
        let changed = apply(edit);
        assert_eq!(changed, source.replace("{safe}", "{fresh}"));
        assert_eq!(compile(&changed).replace("{fresh}", "{safe}"), before);
    }
}
