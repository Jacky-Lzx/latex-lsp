use latex_lsp::{language::features, project::workspace::Workspace};
use lsp_types::*;

fn setup(call: &str) -> (Workspace, Url, String) {
    let source = format!(
        r"\documentclass{{article}}\usepackage{{etoolbox}}\NewHookWithArguments{{custom}}{{1}}\AddToHookWithArguments{{custom}}{{\typeout{{CAPTURE:\detokenize{{#1}}}}}}\begin{{document}}\newcommand\word{{\typeout{{EXECUTED}}Text}}{call}\word\label{{safe}}\ref{{safe}}\end{{document}}"
    );
    let uri = Url::parse("untitled:hook-environment.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.clone(), 5);
    (workspace, uri, source)
}

const CALLS: &[&str] = &[
    r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\begin{invoke}{\word}\end{invoke}",
    r"\NewDocumentEnvironment{invoke}{m}{\UseHook{custom}{#1}}{}\begin{invoke}{\word}\end{invoke}",
    r"\NewDocumentEnvironment{invoke}{m}{}{\UseHook{custom}{#1}}\begin{invoke}{\word}\end{invoke}",
    r"\newenvironment{invoke}[1][\word]{\UseHook{custom}{#1}}{}\begin{invoke}\end{invoke}",
    r"\NewDocumentEnvironment{invoke}{O{\word}}{}{\UseHook{custom}{#1}}\begin{invoke}\end{invoke}",
    r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\csuse{begin}{invoke}{\word}\end{invoke}",
    r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\csname begin\endcsname{invoke}{\word}\end{invoke}",
    r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\def\start{\begin}\start{invoke}{\word}\end{invoke}",
    r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\let\start\begin\start{invoke}{\word}\end{invoke}",
    r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\def\outer#1{\begin{invoke}{#1}\end{invoke}}\outer{\word}",
    r"\def\inner#1{\UseHook{custom}{#1}}\newenvironment{invoke}[1]{\inner{#1}}{}\begin{invoke}{\word}\end{invoke}",
    r"\newenvironment{inner}[1]{\UseHook{custom}{#1}}{}\newenvironment{invoke}[1]{\begin{inner}{#1}}{\end{inner}}\begin{invoke}{\word}\end{invoke}",
    r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\NewEnvironmentCopy{hookcopy}{invoke}\renewenvironment{invoke}[1]{Text}{}\begin{hookcopy}{\word}\end{hookcopy}",
    r"\newenvironment{invoke}[2]{\UseHook{custom}{#2}}{}\NewEnvironmentCopy{hookcopy}{invoke}\renewenvironment{invoke}[1]{Text}{}\begin{hookcopy}{Ignored}{\word}\end{hookcopy}",
    r"\NewDocumentEnvironment{invoke}{mm}{}{\UseHook{custom}{#2}}\NewEnvironmentCopy{hookcopy}{invoke}\RenewDocumentEnvironment{invoke}{m}{Text}{}\begin{hookcopy}{Ignored}{\word}\end{hookcopy}",
];

#[test]
fn environment_arguments_and_defaults_cannot_be_renamed_as_executable_macros() {
    for call in CALLS {
        let (workspace, uri, source) = setup(call);
        let text = &workspace.documents[&uri].text;
        let at = text.position(source.rfind("word").unwrap() + 1);
        assert!(
            features::rename(&workspace, &uri, at, "fresh").is_err(),
            "{call}"
        );
        assert!(
            features::prepare_rename(&workspace, &uri, at).is_err(),
            "{call}"
        );
        let safe = text.position(source.rfind("safe").unwrap() + 1);
        assert_eq!(
            features::rename(&workspace, &uri, safe, "fresh")
                .unwrap_or_else(|error| panic!("{call}: {error:?}"))
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            2,
            "{call}"
        );
    }
}

#[test]
fn independent_environment_parameters_keep_complete_macro_edits() {
    for call in [
        r"\newenvironment{invoke}[1]{\UseHook{custom}{Literal}#1}{}\begin{invoke}{\word}\end{invoke}",
        r"\NewDocumentEnvironment{invoke}{m}{}{\UseHook{custom}{Literal}#1}\begin{invoke}{\word}\end{invoke}",
    ] {
        let (workspace, uri, source) = setup(call);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("word").unwrap() + 1);
        assert_eq!(
            features::rename(&workspace, &uri, at, "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            3,
            "{call}"
        );
    }
    let (workspace, uri, source) = setup(
        r"\newcommand\other{Text}\newenvironment{invoke}[2]{\UseHook{custom}{#1}#2}{}\begin{invoke}{\word}{\other}\end{invoke}\other",
    );
    let text = &workspace.documents[&uri].text;
    assert!(features::rename(
        &workspace,
        &uri,
        text.position(source.rfind("word").unwrap() + 1),
        "fresh"
    )
    .is_err());
    assert_eq!(
        features::rename(
            &workspace,
            &uri,
            text.position(source.rfind("other").unwrap() + 1),
            "fresh"
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap()[&uri]
            .len(),
        3
    );
}

#[test]
fn dynamic_and_argument_consuming_environment_forwarders_cannot_miss_literal_data() {
    for call in [
        r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\def\chosen{invoke}\begin{\chosen}{\word}\end{invoke}",
        r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\def\start#1{\begin}\start{Ignored}{invoke}{\word}\end{invoke}",
        r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}\def\start#1{\begin}\def\outer{\start}\outer{Ignored}{invoke}{\word}\end{invoke}",
    ] {
        let (workspace, uri, source) = setup(call);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("word").unwrap() + 1);
        assert!(
            features::rename(&workspace, &uri, at, "fresh").is_err(),
            "{call}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_environment_data_and_independent_label_edits_preserve_capture_and_full_aux() {
    for call in CALLS {
        let (workspace, uri, source) = setup(call);
        let directory = tempfile::tempdir().unwrap();
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
        let text = &workspace.documents[&uri].text;
        let at = text.position(source.rfind("word").unwrap() + 1);
        assert!(
            features::rename(&workspace, &uri, at, "fresh").is_err(),
            "{call}"
        );
        let safe = text.position(source.rfind("safe").unwrap() + 1);
        let mut edits = features::rename(&workspace, &uri, safe, "fresh")
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
            original,
            "{call}"
        );
    }
}

#[test]
fn unsaved_environment_bodies_defaults_and_shared_consumers_refresh_protection() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = main.join("hooks.tex").unwrap();
    let shared = main.join("shared.tex").unwrap();
    let other = main.join("other.tex").unwrap();
    let root =
        r"\documentclass{article}\NewHookWithArguments{custom}{1}\input{hooks}\input{shared}";
    let reader = r"\newenvironment{invoke}[1]{\UseHook{custom}{#1}}{}";
    let independent = r"\newenvironment{invoke}[1]{\UseHook{custom}{Literal}#1}{}";
    let body =
        r"\newcommand\word{Text}\begin{invoke}{\word}\end{invoke}\word\label{safe}\ref{safe}";
    for (uri, source) in [(&main, root), (&provider, reader), (&shared, body)] {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(main.clone(), root.into(), 5);
    workspace.open(shared.clone(), body.into(), 7);
    let at = workspace.documents[&shared]
        .text
        .position(body.rfind("word").unwrap() + 1);
    for (version, definitions, blocked) in [
        (12, reader, true),
        (13, independent, false),
        (
            14,
            r"\NewDocumentEnvironment{invoke}{m}{}{\UseHook{custom}{#1}}",
            true,
        ),
        (
            15,
            r"\NewDocumentEnvironment{invoke}{m}{}{\UseHook{custom}{Literal}#1}",
            false,
        ),
    ] {
        workspace.open(provider.clone(), definitions.into(), version);
        let result = features::rename(&workspace, &shared, at, "fresh");
        if blocked {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap().unwrap().changes.unwrap()[&shared].len(), 3);
        }
    }
    workspace.open(provider.clone(), independent.into(), 16);
    workspace.open(
        other.clone(),
        format!(
            r"\documentclass{{article}}\NewHookWithArguments{{custom}}{{1}}{reader}\input{{shared}}"
        ),
        9,
    );
    assert!(features::rename(&workspace, &shared, at, "fresh").is_err());
    assert_eq!(workspace.documents[&main].version, Some(5));
    assert_eq!(workspace.documents[&shared].version, Some(7));
    workspace.open(other, String::new(), 10);
    assert_eq!(
        features::rename(&workspace, &shared, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&shared]
            .len(),
        3
    );

    let body = r"\newcommand\word{Text}\begin{invoke}\end{invoke}\word";
    workspace.open(shared.clone(), body.into(), 8);
    let at = workspace.documents[&shared]
        .text
        .position(body.rfind("word").unwrap() + 1);
    workspace.open(
        provider.clone(),
        r"\NewDocumentEnvironment{invoke}{O{\word}}{}{\UseHook{custom}{#1}}".into(),
        17,
    );
    assert!(features::rename(&workspace, &shared, at, "fresh").is_err());
    workspace.open(
        provider,
        r"\NewDocumentEnvironment{invoke}{O{\word}}{}{\UseHook{custom}{Literal}#1}".into(),
        18,
    );
    let changes = features::rename(&workspace, &shared, at, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap();
    assert_eq!(changes.values().map(Vec::len).sum::<usize>(), 3);
}
