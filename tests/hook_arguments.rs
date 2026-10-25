use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:hook-arguments.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 5);
    (workspace, uri)
}

fn source(call: &str) -> String {
    format!(
        r"\documentclass{{article}}\usepackage{{etoolbox}}\NewHookWithArguments{{custom}}{{1}}\AddToHookWithArguments{{custom}}{{\typeout{{CAPTURE:\detokenize{{#1}}}}}}\begin{{document}}\newcommand\word{{Text}}{call}\word\label{{safe}}\ref{{safe}}\end{{document}}"
    )
}

#[test]
fn hook_argument_tokens_cannot_be_renamed_as_executable_macros() {
    for call in [
        r"\UseHook{custom}\word",
        r"\UseOneTimeHook{custom}\word",
        r"\UseHookWithArguments{custom}{0}\word",
        r"\UseOneTimeHookWithArguments{custom}{0}\word",
        r"\csuse{UseHook}{custom}\word",
        r"\csname UseHook\endcsname{custom}\word",
        r"\def\usecustom{\UseHook}\usecustom{custom}\word",
        r"\let\usecustom\UseHook\usecustom{custom}\word",
        r"\let\usecustom\UseHook\def\UseHook#1{Text}\usecustom{custom}\word",
    ] {
        let source = source(call);
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let at = text.position(source.rfind("word").unwrap() + 1);
        assert!(
            features::rename(&workspace, &uri, at, "fresh").is_err(),
            "{source}"
        );
        assert!(
            features::prepare_rename(&workspace, &uri, at).is_err(),
            "{source}"
        );
        let at = text.position(source.rfind("safe").unwrap() + 1);
        let edit = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap();
        assert_eq!(edit.changes.unwrap()[&uri].len(), 2, "{source}");
    }
}

#[test]
fn grouped_hook_data_and_consumed_reference_commands_cannot_change_displayed_keys() {
    for (arity, call) in [
        ("1", r"\UseHook{custom}{\word}"),
        ("2", r"\UseHook{custom}{Text}\word"),
        ("2", r"\UseHookWithArguments{custom}{0}{Text}\word"),
        ("1+1", r"\UseHook{custom}{Text}\word"),
    ] {
        let source = source(call).replacen("{custom}{1}", &format!("{{custom}}{{{arity}}}"), 1);
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("word").unwrap() + 1);
        assert!(
            features::rename(&workspace, &uri, at, "fresh").is_err(),
            "{source}"
        );
    }
    let source = r"\NewHookWithArguments{custom}{1}\UseHook{custom}\label{paper}\ref{paper}\label{safe}\ref{safe}";
    let (workspace, uri) = setup(source);
    let text = &workspace.documents[&uri].text;
    assert!(features::rename(
        &workspace,
        &uri,
        text.position(source.rfind("paper").unwrap() + 1),
        "fresh"
    )
    .is_err());
    assert_eq!(
        features::rename(
            &workspace,
            &uri,
            text.position(source.rfind("safe").unwrap() + 1),
            "fresh"
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap()[&uri]
            .len(),
        2
    );
}

#[test]
fn separate_hooks_and_ordinary_redefinitions_do_not_block_unrelated_macro_edits() {
    for (preamble, call) in [
        ("", r"\NewHook{other}\UseHook{other}\word"),
        (r"\def\UseHook#1{Text}", r"\UseHook{custom}\word"),
        (
            r"\def\usecustom{\UseHook}\def\UseHook#1{Text}",
            r"\usecustom{custom}\word",
        ),
        ("", r"\UseHook{ custom }\word"),
    ] {
        let source = source(&format!("{preamble}{call}"));
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("word").unwrap() + 1);
        let edit = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap();
        assert_eq!(edit.changes.unwrap()[&uri].len(), 3, "{source}");
    }
}

#[test]
fn unsaved_provider_arities_and_shared_consumers_refresh_rename_protection() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = main.join("hooks.tex").unwrap();
    let shared = main.join("shared.tex").unwrap();
    let other = main.join("other.tex").unwrap();
    let declaration = r"\NewHookWithArguments{custom}{1}";
    let source = r"\documentclass{article}\input{hooks}\input{shared}";
    let body = r"\newcommand\word{Text}\UseHook{custom}\word\word\label{safe}\ref{safe}";
    for (uri, source) in [(&main, source), (&provider, declaration), (&shared, body)] {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(main.clone(), source.into(), 5);
    workspace.open(shared.clone(), body.into(), 7);
    let at = workspace.documents[&shared]
        .text
        .position(body.rfind("word").unwrap() + 1);
    assert!(features::rename(&workspace, &shared, at, "fresh").is_err());
    workspace.open(provider.clone(), r"\NewHook{custom}".into(), 12);
    assert_eq!(
        features::rename(&workspace, &shared, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&shared]
            .len(),
        3
    );
    assert_eq!(workspace.documents[&main].version, Some(5));
    assert_eq!(workspace.documents[&shared].version, Some(7));
    workspace.open(
        other.clone(),
        r"\documentclass{article}\NewHookWithArguments{custom}{1}\input{shared}".into(),
        9,
    );
    // Both roots consume the same tokens. A zero-arity meaning in one root
    // cannot justify editing literal hook data in the other root.
    assert!(features::rename(&workspace, &shared, at, "fresh").is_err());
    let independent = main.join("independent.tex").unwrap();
    let isolated =
        r"\documentclass{article}\NewHook{custom}\newcommand\word{Text}\UseHook{custom}\word\word";
    workspace.open(independent.clone(), isolated.into(), 3);
    let isolated_at = workspace.documents[&independent]
        .text
        .position(isolated.rfind("word").unwrap() + 1);
    assert_eq!(
        features::rename(&workspace, &independent, isolated_at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&independent]
            .len(),
        3
    );
    workspace.open(provider, declaration.into(), 13);
    assert!(features::rename(&workspace, &shared, at, "fresh").is_err());
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_hook_argument_edits_preserve_captured_text_and_executed_macro_counts() {
    for call in [
        r"\UseHook{custom}\word",
        r"\UseOneTimeHook{custom}\word",
        r"\UseHookWithArguments{custom}{0}\word",
        r"\UseOneTimeHookWithArguments{custom}{0}\word",
        r"\csuse{UseHook}{custom}\word",
        r"\csname UseHook\endcsname{custom}\word",
        r"\def\usecustom{\UseHook}\usecustom{custom}\word",
        r"\let\usecustom\UseHook\usecustom{custom}\word",
        r"\let\usecustom\UseHook\def\UseHook#1{Text}\usecustom{custom}\word",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = source(call).replace(r"\word{Text}", r"\word{\typeout{EXECUTED}Text}");
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
                    "{call}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            assert_eq!(
                log.lines()
                    .filter(|line| *line == "CAPTURE:\\word ")
                    .count(),
                1,
                "{call}: {log}"
            );
            assert_eq!(
                log.lines().filter(|line| *line == "EXECUTED").count(),
                1,
                "{call}: {log}"
            );
            std::fs::read_to_string(aux).unwrap()
        };
        let original = compile(&source);
        let at = text.position(source.rfind("word").unwrap() + 1);
        assert!(
            features::rename(&workspace, &uri, at, "fresh").is_err(),
            "{call}"
        );
        let at = text.position(source.rfind("safe").unwrap() + 1);
        let mut edits = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&uri]
            .clone();
        assert_eq!(edits.len(), 2);
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let updated = text
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
            compile(&updated.source).replace("{fresh}", "{safe}"),
            original
        );
    }
}
