use latex_lsp::{features, signature, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:kernel-hooks.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.snippets = true;
    workspace.open(uri.clone(), source.into(), 5);
    (workspace, uri)
}

fn cases() -> Vec<(&'static str, &'static str, &'static str, u32)> {
    vec![
        (
            "AddToHook",
            "{begindocu¦ment}[mine]{Code}",
            "{hook}[label]{code}",
            0,
        ),
        (
            "AddToHook",
            "{begindocument}[mi¦ne]{Code}",
            "{hook}[label]{code}",
            1,
        ),
        (
            "AddToHook",
            "{begindocument}{Co¦de}",
            "{hook}[label]{code}",
            2,
        ),
        (
            "AddToHook",
            "{begindocument}[mine] ¦",
            "{hook}[label]{code}",
            2,
        ),
        (
            "AddToHookWithArguments",
            "{custom}[mine]{#1 Co¦de}",
            "{hook}[label]{code}",
            2,
        ),
        ("AddToHookNext", "{custom}{Co¦de}", "{hook}{code}", 1),
        (
            "AddToHookNextWithArguments",
            "{custom}{#1 Co¦de}",
            "{hook}{code}",
            1,
        ),
        ("RemoveFromHook", "{custom}[mi¦ne]", "{hook}[label]", 1),
        ("UseHook", "{cus¦tom}", "{hook}", 0),
        ("UseOneTimeHook", "{cus¦tom}", "{hook}", 0),
        ("AtBeginDocument", "[mi¦ne]{Code}", "[label]{code}", 0),
        ("AtBeginDocument", "{Co¦de}", "[label]{code}", 1),
        ("AtEndDocument", "[mine]{Co¦de}", "[label]{code}", 1),
        ("NewHook", "{cus¦tom}", "{hook}", 0),
        ("NewReversedHook", "{cus¦tom}", "{hook}", 0),
        (
            "NewMirroredHookPair",
            "{before}{af¦ter}",
            "{before hook}{after hook}",
            1,
        ),
        (
            "NewHookWithArguments",
            "{custom}{¦2}",
            "{hook}{argument count}",
            1,
        ),
        (
            "NewReversedHookWithArguments",
            "{custom}{¦2}",
            "{hook}{argument count}",
            1,
        ),
        (
            "NewMirroredHookPairWithArguments",
            "{before}{after}{¦2}",
            "{before hook}{after hook}{argument count}",
            2,
        ),
        (
            "DeclareHookRule",
            "{custom}{first}{bef¦ore}{last}",
            "{hook}{first label}{relation}{second label}",
            2,
        ),
        (
            "ClearHookRule",
            "{custom}{first}{la¦st}",
            "{hook}{first label}{second label}",
            2,
        ),
        (
            "DeclareDefaultHookRule",
            "{first}{before}{la¦st}",
            "{first label}{relation}{second label}",
            2,
        ),
        ("ClearHookNext", "{cus¦tom}", "{hook}", 0),
        ("SetDefaultHookLabel", "{mi¦ne}", "{label}", 0),
        ("PushDefaultHookLabel", "{mi¦ne}", "{label}", 0),
        ("ActivateGenericHook", "{env/quote/be¦fore}", "{hook}", 0),
        ("DisableGenericHook", "{env/quote/be¦fore}", "{hook}", 0),
        ("ShowHook", "{cus¦tom}", "{hook}", 0),
        ("LogHook", "{cus¦tom}", "{hook}", 0),
    ]
}

#[test]
fn kernel_hook_signatures_follow_direct_named_and_forwarding_calls() {
    for (name, tail, suffix, active) in cases() {
        for (preamble, call) in [
            (String::new(), format!(r"\{name}")),
            (String::new(), format!(r"\csuse{{{name}}}")),
            (String::new(), format!(r"\csname {name}\endcsname")),
            (format!(r"\def\forward{{\{name}}}"), r"\forward".into()),
        ] {
            let marked = format!("% 😀\r\n\\newcommand\\unicodeword{{😀}}{preamble}{call}{tail}");
            let (workspace, uri) = setup(&marked.replace('¦', ""));
            let text = &workspace.documents[&uri].text;
            let help = signature::help(&workspace, &uri, text.position(marked.find('¦').unwrap()))
                .unwrap_or_else(|| panic!("{marked}"));
            assert_eq!(
                help.signatures[0].label,
                format!("{call}{suffix}"),
                "{marked}"
            );
            assert_eq!(help.active_parameter, Some(active), "{marked}");
        }
    }
}

#[test]
fn hook_snippets_omit_optional_labels_and_keep_existing_arguments() {
    for (name, _, _, _) in cases() {
        for following in ["", "{existing}"] {
            let source = format!(r"\{name}{following}");
            let (workspace, uri) = setup(&source);
            let item =
                features::completion(&workspace, &uri, Position::new(0, name.len() as u32 + 1))
                    .into_iter()
                    .find(|item| item.label == name)
                    .unwrap_or_else(|| panic!("{name}"));
            let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
                panic!()
            };
            if following.is_empty() {
                assert!(edit.new_text.contains("${1:"), "{edit:?}");
                assert!(!edit.new_text.contains('['), "{edit:?}");
            } else {
                assert_eq!(edit.new_text, name);
            }
        }
    }
}

#[test]
fn hook_arguments_are_data_and_completed_calls_leave_following_references_available() {
    for call in [
        r"\AddToHook{\ref{paper}}[mine]{Code}",
        r"\AddToHookWithArguments{custom}[\ref{paper}]{Code}",
        r"\AddToHookNext{\ref{paper}}{Code}",
        r"\AddToHookNextWithArguments{\ref{paper}}{Code}",
        r"\AtBeginDocument[\ref{paper}]{Code}",
        r"\AtEndDocument[\ref{paper}]{Code}",
        r"\RemoveFromHook{\ref{paper}}[mine]",
        r"\UseHook{\ref{paper}}",
        r"\DeclareHookRule{custom}{first}{before}{\ref{paper}}",
    ] {
        let source = format!(r"\label{{paper}}{call}\ref{{paper}}\label{{safe}}\ref{{safe}}");
        let (workspace, uri) = setup(&source);
        let text = Text::new(source.clone());
        let data_at = text.position(source.find(r"\ref{paper}").unwrap() + 7);
        assert!(
            features::definitions(&workspace, &uri, data_at).is_empty(),
            "{source}"
        );
        assert!(
            features::rename(&workspace, &uri, text.position(9), "fresh").is_err(),
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
            2,
            "{source}"
        );
    }
}

#[test]
fn unsaved_hook_redefinitions_replace_builtin_signatures_without_parent_edits() {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = uri.join("commands.sty").unwrap();
    let source = r"\documentclass{article}\input{commands.sty}\AddToHook{custom}[mine]{Code}";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    std::fs::write(provider.to_file_path().unwrap(), "").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 5);
    let at = workspace.documents[&uri]
        .text
        .position(source.find("custom").unwrap() + 1);
    for (replacement, suffix) in [
        ("", "{hook}[label]{code}"),
        (r"\renewcommand\AddToHook[1]{}", "{#1}"),
        ("", "{hook}[label]{code}"),
    ] {
        workspace.open(provider.clone(), replacement.into(), 12);
        assert_eq!(
            signature::help(&workspace, &uri, at).unwrap().signatures[0].label,
            format!(r"\AddToHook{suffix}")
        );
        assert_eq!(workspace.documents[&uri].version, Some(5));
    }
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_hook_order_arguments_and_optional_labels_survive_independent_label_edits() {
    let directory = tempfile::tempdir().unwrap();
    let source = r"\documentclass{article}
\usepackage{etoolbox}
\NewHook{custom}
\NewHook{unused}
\AddToHook{unused}{\begin{quote}\(}
\NewHookWithArguments{arguments}{1}
\AddToHook{custom}[first]{\typeout{LSP-HOOK:FIRST}\ref{safe}}
\AddToHook{custom}[last]{\typeout{LSP-HOOK:LAST}}
\AddToHook{custom}[removed]{\errmessage{Removed hook executed}}
\RemoveFromHook{custom}[removed]
\DeclareHookRule{custom}{first}{before}{last}
\AddToHookNext{custom}{\typeout{LSP-HOOK:NEXT}}
\AddToHookWithArguments{arguments}[mine]{\typeout{LSP-HOOK:ARG:#1}}
\AddToHookNextWithArguments{arguments}{\typeout{LSP-HOOK:NEXTARG:#1}}
\AtBeginDocument[mine]{\typeout{LSP-HOOK:BEGIN}}
\AtEndDocument[mine]{\typeout{LSP-HOOK:END}}
\begin{document}
\section{Heading}\label{safe}\ref{safe}
\UseHook{custom}\UseHook{custom}
\UseHookWithArguments{arguments}{1}{Value}
\UseHookWithArguments{arguments}{1}{Again}
\end{document}";
    let (workspace, uri) = setup(source);
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
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
        assert!(!log.contains("undefined"), "{log}");
        let markers = log
            .lines()
            .filter(|line| line.starts_with("LSP-HOOK:"))
            .collect::<Vec<_>>();
        assert_eq!(
            markers,
            [
                "LSP-HOOK:BEGIN",
                "LSP-HOOK:FIRST",
                "LSP-HOOK:LAST",
                "LSP-HOOK:NEXT",
                "LSP-HOOK:FIRST",
                "LSP-HOOK:LAST",
                "LSP-HOOK:ARG:Value",
                "LSP-HOOK:NEXTARG:Value",
                "LSP-HOOK:ARG:Again",
                "LSP-HOOK:END"
            ]
        );
        std::fs::read_to_string(aux).unwrap()
    };
    let before = compile(source);
    let at = text.position(source.rfind("safe").unwrap() + 1);
    let mut edits = features::rename(&workspace, &uri, at, "fresh")
        .unwrap()
        .unwrap()
        .changes
        .unwrap()[&uri]
        .clone();
    assert_eq!(edits.len(), 3);
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let changed = text
        .clone()
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
    assert_eq!(changed.source, source.replace("{safe}", "{fresh}"));
    assert_eq!(
        compile(&changed.source).replace("{fresh}", "{safe}"),
        before
    );
}

#[test]
fn unfinished_hook_arguments_and_forwarded_calls_keep_reference_tokens_protected() {
    for tail in [
        r"\AddToHook{custom}[mine]{\ref{paper}",
        r"\AddToHook{custom}[\ref{paper}",
        r"\AddToHookWithArguments{custom}{\ref{paper}",
        r"\AddToHookNext{custom}{\ref{paper}",
        r"\AtBeginDocument[mine]{\ref{paper}",
        r"\AtEndDocument[\ref{paper}",
        r"\def\forward{\AddToHook}\forward{custom}{\ref{paper}",
        r"\csuse{AddToHook}{custom}{\ref{paper}",
        r"\csname AddToHook\endcsname{custom}{\ref{paper}",
    ] {
        let source = format!(r"\label{{paper}}{tail}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("paper").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{source}"
        );
        assert!(
            !matches!(features::rename(&workspace, &uri, at, "fresh"), Ok(Some(_))),
            "{source}"
        );
        assert!(!matches!(
            features::prepare_rename(&workspace, &uri, at),
            Ok(Some(_))
        ));
    }
}

#[test]
fn completed_optional_and_argument_free_hook_commands_preserve_following_controls() {
    for call in [
        r"\RemoveFromHook{custom}",
        r"\RemoveFromHook{custom}[mine]",
        r"\AddToHook{custom}{Code}",
        r"\AtBeginDocument{Code}",
        r"\AtEndDocument{Code}",
        r"\PopDefaultHookLabel",
        r"\DebugHooksOn",
        r"\DebugHooksOff",
    ] {
        let source = format!(r"\label{{safe}}{call}\ref{{safe}}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("safe").unwrap() + 1);
        assert_eq!(
            features::definitions(&workspace, &uri, at).len(),
            1,
            "{source}"
        );
        assert_eq!(
            features::rename(&workspace, &uri, at, "fresh")
                .unwrap()
                .unwrap()
                .changes
                .unwrap()[&uri]
                .len(),
            2,
            "{source}"
        );
    }
}

#[test]
fn stored_hook_code_preserves_structural_reference_navigation_and_renames() {
    for call in [
        r"\AddToHook{custom}[mine]{\ref{paper}}",
        r"\AddToHookWithArguments{custom}[mine]{\ref{paper}}",
        r"\AddToHookNext{custom}{\ref{paper}}",
        r"\AddToHookNextWithArguments{custom}{\ref{paper}}",
        r"\AtBeginDocument[mine]{\ref{paper}}",
        r"\AtEndDocument{\ref{paper}}",
        r"\def\forward{\AddToHook}\forward{custom}{\ref{paper}}",
        r"\csuse{AddToHook}{custom}{\ref{paper}}",
        r"\csname AddToHook\endcsname{custom}{\ref{paper}}",
    ] {
        let source = format!(r"\label{{paper}}{call}\ref{{paper}}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let at = text.position(source.find(r"\ref{paper}").unwrap() + 7);
        assert_eq!(
            features::definitions(&workspace, &uri, at).len(),
            1,
            "{source}"
        );
        assert_eq!(
            features::rename(&workspace, &uri, at, "fresh")
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
fn stored_readers_do_not_certify_the_execution_identity_of_hook_labels() {
    for call in [
        r"\AddToHook{custom}[mine]",
        r"\AddToHookNext{custom}",
        r"\AtBeginDocument[mine]",
        r"\csuse{AddToHook}{custom}",
        r"\csname AddToHook\endcsname{custom}",
        r"\def\forward{\AddToHook}\forward{custom}",
    ] {
        let source = format!(
            r"\newcommand\literal[1]{{\detokenize{{#1}}}}{call}{{\literal{{\label{{same}}}}\typeout{{\label{{other}}}}}}\ref{{same}}\ref{{other}}"
        );
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        for name in ["same", "other"] {
            let at = text.position(source.rfind(name).unwrap() + 1);
            assert_eq!(
                features::definitions(&workspace, &uri, at).len(),
                1,
                "{source}"
            );
        }
    }
}

#[test]
fn stored_hook_environments_do_not_pair_with_immediate_document_environments() {
    for call in [
        r"\AddToHook{unused}{\begin{quote}}",
        r"\AddToHookNext{unused}{\begin{quote}}",
        r"\def\forward{\AddToHook}\forward{unused}{\begin{quote}}",
        r"\csuse{AddToHook}{unused}{\begin{quote}}",
    ] {
        let source =
            format!(r"\documentclass{{article}}{call}\begin{{document}}Text\end{{document}}");
        let (mut workspace, uri) = setup(&source);
        assert!(
            workspace.diagnostics(&uri).is_empty(),
            "{call}: {:?}",
            workspace.diagnostics(&uri)
        );
        workspace.open(
            uri.clone(),
            source.replace("Text", r"\begin{itemize}Text"),
            6,
        );
        let diagnostics = workspace.diagnostics(&uri);
        assert_eq!(diagnostics.len(), 1, "{call}: {diagnostics:?}");
        assert!(
            diagnostics[0].message.contains("itemize"),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn stored_math_delimiters_do_not_change_immediate_math_state() {
    for call in [
        r"\AddToHook{unused}{\(}",
        r"\AddToHookNext{unused}{$}",
        r"\AtEndDocument{\[}",
        r"\def\forward{\AddToHook}\forward{unused}{$}",
        r"\csuse{AddToHook}{unused}{\(}",
    ] {
        let source =
            format!(r"\documentclass{{article}}{call}\begin{{document}}$x$\end{{document}}");
        let (mut workspace, uri) = setup(&source);
        workspace.config.diagnostics.math_delimiters = true;
        assert!(
            workspace.diagnostics(&uri).is_empty(),
            "{call}: {:?}",
            workspace.diagnostics(&uri)
        );
        workspace.open(uri.clone(), source.replace("$x$", "$x"), 6);
        let diagnostics = workspace.diagnostics(&uri);
        assert_eq!(diagnostics.len(), 1, "{call}: {diagnostics:?}");
        assert_eq!(
            diagnostics[0].code,
            Some(NumberOrString::String("unclosed-math".into()))
        );
    }
}

fn string_mutation_cases() -> [(&'static str, &'static str, usize); 7] {
    [
        (
            r"\NewHook{custom}\AddToHook{custom}{\let\ifblank\identity}",
            r"\UseHook{custom}",
            3,
        ),
        (
            r"\NewHook{custom}\AddToHook{custom}{\let\ifblank\identity}",
            "",
            1,
        ),
        (r"\AtBeginDocument{\let\ifblank\identity}", "", 3),
        (
            r"\NewHook{custom}\let\savedblank\ifblank\AddToHook{custom}{\let\ifblank\identity}\let\ifblank\savedblank",
            r"\UseHook{custom}",
            3,
        ),
        (
            r"\NewHook{custom}\UseOneTimeHook{custom}\AddToHook{custom}{\let\ifblank\identity}",
            "",
            3,
        ),
        (
            r"\NewHook{custom}\def\forward{\AddToHook}\forward{custom}{\let\ifblank\identity}",
            r"\UseHook{custom}",
            3,
        ),
        (
            r"\NewHook{custom}\csuse{AddToHook}{custom}{\let\ifblank\identity}",
            r"\UseHook{custom}",
            3,
        ),
    ]
}

#[test]
fn registered_string_mutations_never_rename_only_some_executed_macro_calls() {
    for (register, trigger, count) in string_mutation_cases() {
        let (workspace, uri) = setup(&format!(
            r"\documentclass{{article}}\usepackage{{etoolbox}}\def\identity#1{{#1}}\newcommand\word{{Text}}{register}\begin{{document}}{trigger}\ifblank{{\word}}{{\word}}{{\word}}\label{{safe}}\ref{{safe}}\end{{document}}"
        ));
        let text = &workspace.documents[&uri].text;
        let at = text.position(text.source.rfind("word").unwrap() + 1);
        if let Ok(Some(edit)) = features::rename(&workspace, &uri, at, "term") {
            assert_eq!(
                edit.changes.unwrap()[&uri].len(),
                if count == 3 { 4 } else { 3 },
                "{register}/{trigger}"
            );
        }
        let at = text.position(text.source.rfind("safe").unwrap() + 1);
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
#[ignore = "requires pdflatex and etoolbox"]
fn native_hook_string_mutations_reject_partial_edits_and_preserve_independent_renames() {
    for (register, trigger, count) in string_mutation_cases() {
        let directory = tempfile::tempdir().unwrap();
        let source = format!(
            r"\documentclass{{article}}\usepackage{{etoolbox}}\def\identity#1{{#1}}\newcommand\word{{\typeout{{LSP-WORD}}}}{register}\begin{{document}}\section{{Heading}}{trigger}\ifblank{{\word}}{{\word}}{{\word}}\label{{safe}}\ref{{safe}}\end{{document}}"
        );
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
                    "{register}/{trigger}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
            assert_eq!(log.matches("LSP-WORD").count(), count, "{log}");
            assert!(!log.contains("undefined"), "{log}");
            std::fs::read_to_string(aux).unwrap()
        };
        let apply = |mut edits: Vec<TextEdit>| {
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
        };
        let before = compile(&source);
        let at = text.position(source.rfind("word").unwrap() + 1);
        if let Ok(Some(edit)) = features::rename(&workspace, &uri, at, "term") {
            assert_eq!(
                compile(&apply(edit.changes.unwrap()[&uri].clone()).source),
                before
            );
        }
        let at = text.position(source.rfind("safe").unwrap() + 1);
        let edits = features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()[&uri]
            .clone();
        assert_eq!(edits.len(), 2);
        let changed = apply(edits);
        assert_eq!(changed.source, source.replace("{safe}", "{fresh}"));
        assert_eq!(
            compile(&changed.source).replace("{fresh}", "{safe}"),
            before
        );
    }
}
