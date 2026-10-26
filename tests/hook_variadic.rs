use lsp_types::*;
use texquill::{
    language::{features, signature},
    project::workspace::Workspace,
};

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:variadic-hook.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.client.snippets = true;
    workspace.open(uri.clone(), source.into(), 5);
    (workspace, uri)
}

#[test]
fn hook_use_signatures_follow_declared_counts_and_literal_fallback_counts() {
    for (declaration, call, tail, suffix, active) in [
        (
            r"\NewHookWithArguments{custom}{2}",
            "UseHook",
            "{custom}{First}{Sec¦ond}",
            "{hook}{argument 1}{argument 2}",
            2,
        ),
        (
            r"\def\declare{\NewHookWithArguments}\declare{custom}{2}",
            "UseHookWithArguments",
            "{custom}{0}{First}{Sec¦ond}",
            "{hook}{argument count}{argument 1}{argument 2}",
            3,
        ),
        (
            r"\csuse{NewHookWithArguments}{custom}{2}",
            "UseHookWithArguments",
            "{custom}{0}{First}{Sec¦ond}",
            "{hook}{argument count}{argument 1}{argument 2}",
            3,
        ),
        (
            r"\NewHookWithArguments{custom}{2}",
            "UseHookWithArguments",
            "{custom}{0}{First}{Sec¦ond}",
            "{hook}{argument count}{argument 1}{argument 2}",
            3,
        ),
        (
            "",
            "UseHookWithArguments",
            "{missing}{2}{First}{Sec¦ond}",
            "{hook}{argument count}{argument 1}{argument 2}",
            3,
        ),
        (
            r"\NewMirroredHookPairWithArguments{before}{custom}{1}",
            "UseHookWithArguments",
            "{custom}{9}{Da¦ta}",
            "{hook}{argument count}{argument 1}",
            2,
        ),
    ] {
        for (preamble, invoke) in [
            (String::new(), format!(r"\{call}")),
            (String::new(), format!(r"\csuse{{{call}}}")),
            (String::new(), format!(r"\csname {call}\endcsname")),
            (format!(r"\def\invoke{{\{call}}}"), r"\invoke".into()),
        ] {
            let marked = format!("{declaration}{preamble}{invoke}{tail}");
            let at = marked.find('¦').unwrap();
            let source = marked.replace('¦', "");
            let (workspace, uri) = setup(&source);
            let help = signature::help(
                &workspace,
                &uri,
                workspace.documents[&uri].text.position(at),
            )
            .unwrap_or_else(|| panic!("{marked}"));
            assert_eq!(
                help.signatures[0].label,
                format!("{invoke}{suffix}"),
                "{marked}"
            );
            assert_eq!(help.active_parameter, Some(active), "{marked}");
        }
    }
}

#[test]
fn declared_zero_overrides_supplied_count_and_keeps_following_reference_executable() {
    for call in [
        r"\UseHookWithArguments{custom}{1}",
        r"\csuse{UseHookWithArguments}{custom}{1}",
        r"\csname UseHookWithArguments\endcsname{custom}{1}",
        r"\def\invoke{\UseHookWithArguments}\invoke{custom}{1}",
    ] {
        let source =
            format!(r"\NewHookWithArguments{{custom}}{{0}}\label{{paper}}{call}\ref{{paper}}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let declaration = source.find("paper").unwrap();
        assert_eq!(
            features::definitions(
                &workspace,
                &uri,
                text.position(source.rfind("paper").unwrap() + 1)
            ),
            [Location {
                uri: uri.clone(),
                range: text.range(declaration, declaration + 5)
            }],
            "{source}"
        );
    }
}

#[test]
fn hook_data_parameters_are_opaque_and_names_include_both_variadic_interfaces() {
    for call in [
        "UseHook",
        "UseHookWithArguments",
        "UseOneTimeHookWithArguments",
    ] {
        let header = if call.ends_with("WithArguments") {
            "{custom}{1}"
        } else {
            "{custom}"
        };
        let source = format!(
            r"\NewHookWithArguments{{custom}}{{1}}\label{{paper}}\{call}{header}{{\ref{{paper}}}}\ref{{paper}}"
        );
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        assert!(
            features::definitions(
                &workspace,
                &uri,
                text.position(source.find(r"\ref{paper}").unwrap() + 7)
            )
            .is_empty(),
            "{source}"
        );
        assert_eq!(
            features::definitions(
                &workspace,
                &uri,
                text.position(source.rfind("paper").unwrap() + 1)
            )
            .len(),
            1,
            "{source}"
        );
        assert_eq!(
            features::definitions(
                &workspace,
                &uri,
                text.position(source.rfind("custom").unwrap() + 2)
            ),
            [Location {
                uri: uri.clone(),
                range: text.range(22, 28)
            }],
            "{source}"
        );
    }
}

#[test]
fn unsaved_provider_counts_refresh_help_and_reference_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = main.join("hooks.tex").unwrap();
    let source = r"\documentclass{article}\input{hooks}\label{paper}\UseHookWithArguments{custom}{1}{\ref{paper}}\ref{paper}";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(provider.to_file_path().unwrap(), r"\NewHook{custom}").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(main.clone(), source.into(), 5);
    for (version, declaration, suffix, count) in [
        (12, r"\NewHook{custom}", "{hook}{argument count}", 1),
        (
            13,
            r"\NewHookWithArguments{custom}{1}",
            "{hook}{argument count}{argument 1}",
            0,
        ),
        (14, r"\NewHook{custom}", "{hook}{argument count}", 1),
    ] {
        workspace.open(provider.clone(), declaration.into(), version);
        let text = &workspace.documents[&main].text;
        let help = signature::help(
            &workspace,
            &main,
            text.position(source.find("{1}").unwrap() + 1),
        )
        .unwrap();
        assert_eq!(
            help.signatures[0].label,
            format!(r"\UseHookWithArguments{suffix}")
        );
        assert_eq!(
            features::definitions(
                &workspace,
                &main,
                text.position(source.find(r"\ref{paper}").unwrap() + 7)
            )
            .len(),
            count
        );
        assert_eq!(
            features::definitions(
                &workspace,
                &main,
                text.position(source.rfind("paper").unwrap() + 1)
            )
            .len(),
            1
        );
        assert_eq!(workspace.documents[&main].version, Some(5));
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn native_declared_zero_and_missing_hook_counts_keep_executable_references_and_edits() {
    for (declaration, call) in [
        (
            r"\NewHookWithArguments{custom}{0}",
            r"\UseHookWithArguments{custom}{1}",
        ),
        (
            r"\NewHookWithArguments{custom}{0}",
            r"\csname UseHookWithArguments\endcsname{custom}{1}",
        ),
        ("", r"\UseHookWithArguments{missing}{2}{A}{B}"),
    ] {
        let source = format!(
            r"\documentclass{{article}}{declaration}\begin{{document}}\newcommand\word{{\typeout{{EXECUTED}}Text}}\word\label{{paper}}{call}\ref{{paper}}\word\end{{document}}"
        );
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        assert_eq!(
            features::definitions(
                &workspace,
                &uri,
                text.position(source.rfind("paper").unwrap() + 1)
            )
            .len(),
            1
        );
        let mut edits = features::rename(
            &workspace,
            &uri,
            text.position(source.rfind("word").unwrap() + 1),
            "fresh",
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap()[&uri]
            .clone();
        assert_eq!(edits.len(), 3);
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
                log.lines().filter(|line| *line == "EXECUTED").count(),
                2,
                "{call}: {log}"
            );
            std::fs::read_to_string(aux).unwrap()
        };
        assert_eq!(compile(&source), compile(&updated.source));
    }
}
