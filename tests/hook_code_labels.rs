use latex_lsp::{editing, language::features, project::workspace::Workspace, text::Text};
use lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:hook-code-labels.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(uri.clone(), source.into(), 5);
    (workspace, uri)
}

fn locations(uri: &Url, source: &str, needle: &str) -> Vec<Location> {
    let text = Text::new(source.into());
    source
        .match_indices(needle)
        .map(|(at, _)| Location {
            uri: uri.clone(),
            range: text.range(at, at + needle.len()),
        })
        .collect()
}

#[test]
fn code_labels_navigate_all_chunks_without_merging_hooks_or_other_namespaces() {
    let source = "% code labels\r\n😀 \\NewHook{one}\\NewHook{two}\\AddToHook{one}[alpha]{First}\\AddToHook{one}[alpha]{Second}\\AddToHook{two}[alpha]{Other}\\RemoveFromHook{one}[alpha]\\DeclareHookRule{one}{alpha}{before}{beta}\\ClearHookRule{one}{beta}{alpha}\\label{alpha}\\ref{alpha}\\def\\alpha{Text}\\NewHook{alpha}";
    let (workspace, uri) = setup(source);
    let all = locations(&uri, source, "alpha");
    assert_eq!(all.len(), 10);
    let expected: Vec<_> = [0, 1, 3, 4, 5].map(|index| all[index].clone()).into();
    for site in &expected {
        assert_eq!(
            features::definitions(&workspace, &uri, site.range.start),
            all[..2]
        );
        assert_eq!(
            features::references(&workspace, &uri, site.range.start, true),
            expected
        );
        assert_eq!(
            features::references(&workspace, &uri, site.range.start, false),
            expected[2..]
        );
        let hover = features::hover(&workspace, &uri, site.range.start).unwrap();
        assert_eq!(hover.range, Some(site.range));
        assert!(
            matches!(hover.contents, HoverContents::Markup(MarkupContent { value, .. })
            if value.contains("alpha") && value.contains("one") && value.contains("code label"))
        );
    }
    assert_eq!(
        features::definitions(&workspace, &uri, all[2].range.start),
        all[2..3]
    );
    assert_eq!(
        features::references(&workspace, &uri, all[2].range.start, true),
        all[2..3]
    );
    assert_eq!(
        features::definitions(&workspace, &uri, all[7].range.start),
        all[6..7]
    );
    assert_eq!(
        features::definitions(&workspace, &uri, all[9].range.start),
        all[9..]
    );
    let highlights = editing::highlights(&workspace, &uri, all[3].range.start);
    assert_eq!(highlights.len(), 5);
    assert!(highlights[..2]
        .iter()
        .all(|item| item.kind == Some(DocumentHighlightKind::WRITE)));
    assert!(highlights[2..]
        .iter()
        .all(|item| item.kind == Some(DocumentHighlightKind::READ)));
}

#[test]
fn code_label_fields_follow_confirmed_direct_named_and_forwarded_interfaces() {
    for (preamble, register, remove, rule, clear) in [
        (
            "",
            r"\AddToHook",
            r"\RemoveFromHook",
            r"\DeclareHookRule",
            r"\ClearHookRule",
        ),
        (
            "",
            r"\AddToHookWithArguments",
            r"\RemoveFromHook",
            r"\DeclareHookRule",
            r"\ClearHookRule",
        ),
        (
            "",
            r"\csuse{AddToHook}",
            r"\csname RemoveFromHook\endcsname",
            r"\csuse{DeclareHookRule}",
            r"\csname ClearHookRule\endcsname",
        ),
        (
            r"\def\register{\AddToHook}\def\remove{\RemoveFromHook}\def\rule{\DeclareHookRule}\def\clear{\ClearHookRule}",
            r"\register",
            r"\remove",
            r"\rule",
            r"\clear",
        ),
        (
            r"\NewDocumentCommand\register{}{\AddToHook}",
            r"\register",
            r"\RemoveFromHook",
            r"\DeclareHookRule",
            r"\ClearHookRule",
        ),
        (
            r"\newenvironment{register}{\AddToHook}{}",
            r"\register",
            r"\RemoveFromHook",
            r"\DeclareHookRule",
            r"\ClearHookRule",
        ),
    ] {
        let source = format!(
            r"{preamble}\NewHookWithArguments{{one}}{{1}}{register}{{one}}[alpha]{{Code}}{remove}{{one}}[alpha]{rule}{{one}}{{alpha}}{{after}}{{beta}}{clear}{{one}}{{alpha}}{{beta}}"
        );
        let (workspace, uri) = setup(&source);
        let all = locations(&uri, &source, "alpha");
        assert_eq!(all.len(), 4, "{source}");
        for site in &all {
            assert_eq!(
                features::definitions(&workspace, &uri, site.range.start),
                all[..1],
                "{source}"
            );
            assert_eq!(
                features::references(&workspace, &uri, site.range.start, true),
                all,
                "{source}"
            );
        }
    }
}

#[test]
fn code_label_completion_keeps_the_hook_namespace_and_replaces_exact_suffixes() {
    for tail in [
        r"\RemoveFromHook{one}[alFAKE]",
        r"\RemoveFromHook{one}[alFAKE",
        r"\DeclareHookRule{one}{alFAKE}{before}{beta}",
        r"\ClearHookRule{one}{beta}{alFAKE}",
        r"\ClearHookRule{one}{beta}{alFAKE",
        r"\AddToHook{one}[ alFAKE ]{Code}",
    ] {
        let source = format!("% labels\r\n😀 \\AddToHook{{one}}[alpha]{{Code}}\\AddToHook{{two}}[alphabet]{{Other}}\\label{{albatross}}{tail}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let at = source.rfind("alFAKE").unwrap();
        let items = features::completion(&workspace, &uri, text.position(at + 2));
        let item = items.iter().find(|item| item.label == "alpha").unwrap();
        assert_eq!(
            item.text_edit,
            Some(CompletionTextEdit::Edit(TextEdit {
                range: text.range(at, at + 6),
                new_text: "alpha".into(),
            })),
            "{source}"
        );
        assert!(
            !items
                .iter()
                .any(|item| matches!(item.label.as_str(), "alphabet" | "albatross")),
            "{source}"
        );
    }
    for tail in [r"\RemoveFromHook{one}[]", r"\RemoveFromHook{one}["] {
        let source = format!(r"\AddToHook{{one}}[alpha]{{Code}}{tail}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let at = source.rfind('[').unwrap() + 1;
        let items = features::completion(&workspace, &uri, text.position(at));
        assert!(items.iter().any(|item| item.label == "alpha"
            && item.text_edit
                == Some(CompletionTextEdit::Edit(TextEdit {
                    range: text.range(at, at),
                    new_text: "alpha".into(),
                }))));
    }
}

#[test]
fn normalized_comments_whitespace_and_bare_rule_labels_have_exact_ranges() {
    let source = "\\AddToHook{ % hook\none% end\n }[ % before\nalpha% after\n ]{Code}\\RemoveFromHook{ one }[ alpha ]\\DeclareHookRule{ one }{ alpha }{ before }{ beta }\\ClearHookRule{ one }{ beta }{ alpha }";
    let (workspace, uri) = setup(source);
    let all = locations(&uri, source, "alpha");
    assert_eq!(all.len(), 4);
    for site in &all {
        assert_eq!(
            features::definitions(&workspace, &uri, site.range.start),
            all[..1]
        );
        assert_eq!(
            features::references(&workspace, &uri, site.range.start, true),
            all
        );
    }
    let source = r"\AddToHook a[b]{Code}\DeclareHookRule ab{before}c\ClearHookRule acb";
    let (workspace, uri) = setup(source);
    let text = &workspace.documents[&uri].text;
    let declaration = source.find("[b]").unwrap() + 1;
    let expected = [Location {
        uri: uri.clone(),
        range: text.range(declaration, declaration + 1),
    }];
    for at in [source.find("ab{").unwrap() + 1, source.len() - 1] {
        assert_eq!(
            features::definitions(&workspace, &uri, text.position(at)),
            expected
        );
    }
    let source = r"\AddToHook a[beta]{Code}\ClearHookRule acb";
    let (workspace, uri) = setup(source);
    let text = &workspace.documents[&uri].text;
    let items = features::completion(&workspace, &uri, text.position(source.len()));
    assert!(items.iter().any(|item| item.label == "beta"
        && item.text_edit
            == Some(CompletionTextEdit::Edit(TextEdit {
                range: text.range(source.len() - 1, source.len()),
                new_text: "{beta}".into(),
            }))));
}

#[test]
fn data_operands_dynamic_fields_and_unconfirmed_interfaces_never_create_label_navigation() {
    for tail in [
        r"\detokenize{\RemoveFromHook{one}[alpha]}",
        r"\string\RemoveFromHook{one}[alpha]",
        r"\NewHookWithArguments{capture}{1}\UseHook{capture}{\RemoveFromHook{one}[alpha]}",
        r"\def\RemoveFromHook#1[#2]{}\RemoveFromHook{one}[alpha]",
        r"\RemoveFromHook{\which}[alpha]",
        r"\RemoveFromHook{./one}[alpha]",
        r"\RemoveFromHook{one}[./alpha]",
        r"\RemoveFromHook{one}[alpha\which]",
        "\\RemoveFromHook{one}[al% split\nalpha]",
        r"\DeclareDefaultHookRule{alpha}{after}{beta}",
    ] {
        let source = format!(r"\AddToHook{{one}}[alpha]{{Code}}{tail}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("alpha").unwrap() + 1);
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{source}"
        );
    }
    for label in ["*", "top-level", "."] {
        let source = format!(r"\AddToHook{{one}}[{label}]{{Code}}\RemoveFromHook{{one}}[{label}]");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind(label).unwrap());
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn unsaved_label_providers_refresh_without_parent_edits_or_unrelated_project_merges() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = main.join("hooks.tex").unwrap();
    let other = main.join("other.tex").unwrap();
    let source = "\\documentclass{article}\\input{hooks}\r\n\\begin{document}😀 \\remove{one}[alpha]\\end{document}";
    let definitions = r"\AddToHook{one}[alpha]{Code}\def\remove{\RemoveFromHook}";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(provider.to_file_path().unwrap(), definitions).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(main.clone(), source.into(), 5);
    let at = locations(&main, source, "alpha")[0].range.start;
    let expected = locations(&provider, definitions, "alpha");
    assert_eq!(features::definitions(&workspace, &main, at), expected);
    workspace.open(
        provider.clone(),
        r"\AddToHook{one}[alpha]{Code}\def\remove#1[#2]{}".into(),
        12,
    );
    assert!(features::definitions(&workspace, &main, at).is_empty());
    assert_eq!(workspace.documents[&main].version, Some(5));
    workspace.open(provider.clone(), definitions.into(), 13);
    assert_eq!(features::definitions(&workspace, &main, at), expected);
    let two = format!(r"{definitions}\AddToHook{{one}}[alpha]{{More}}");
    workspace.open(provider.clone(), two.clone(), 14);
    assert_eq!(
        features::definitions(&workspace, &main, at),
        locations(&provider, &two, "alpha")
    );
    workspace.documents.remove(&provider);
    workspace.reload(&provider);
    assert_eq!(features::definitions(&workspace, &main, at), expected);
    workspace.open(
        other,
        r"\documentclass{article}\AddToHook{one}[alpha]{Other}".into(),
        9,
    );
    assert_eq!(features::definitions(&workspace, &main, at), expected);
}

#[test]
fn shared_code_label_calls_require_consensus_and_recover_when_a_consumer_changes() {
    let directory = tempfile::tempdir().unwrap();
    let a = Url::from_file_path(directory.path().join("a.tex")).unwrap();
    let b = a.join("b.tex").unwrap();
    let shared = a.join("shared.tex").unwrap();
    let declarations = r"\AddToHook{one}[alpha]{Code}\RemoveFromHook{one}[alpha]";
    let source = r"\documentclass{article}\input{shared}\begin{document}\RemoveFromHook{one}[alpha]\end{document}";
    for (uri, text) in [(&a, source), (&b, source), (&shared, declarations)] {
        std::fs::write(uri.to_file_path().unwrap(), text).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(a.clone(), source.into(), 5);
    workspace.open(b.clone(), source.into(), 7);
    let sites = locations(&shared, declarations, "alpha");
    let own = locations(&a, source, "alpha")[0].range.start;
    assert_eq!(
        features::definitions(&workspace, &shared, sites[1].range.start),
        sites[..1]
    );
    assert_eq!(features::references(&workspace, &a, own, true).len(), 4);
    workspace.open(
        b.clone(),
        format!(r"\def\RemoveFromHook#1[#2]{{}}{source}"),
        9,
    );
    assert!(features::definitions(&workspace, &shared, sites[1].range.start).is_empty());
    assert_eq!(features::definitions(&workspace, &a, own), sites[..1]);
    assert_eq!(features::references(&workspace, &a, own, true).len(), 2);
    workspace.open(b, source.into(), 11);
    assert_eq!(
        features::definitions(&workspace, &shared, sites[1].range.start),
        sites[..1]
    );
    assert_eq!(features::references(&workspace, &a, own, true).len(), 4);
}

fn compile(source: &str) -> (Vec<String>, String) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("main.tex"), source).unwrap();
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
    (
        log.lines()
            .filter(|line| line.starts_with("EVENT:"))
            .map(str::to_owned)
            .collect(),
        std::fs::read_to_string(directory.path().join("main.aux")).unwrap(),
    )
}

#[test]
#[ignore = "requires native pdflatex"]
fn native_code_label_chunks_rules_and_removal_match_navigation() {
    for (preamble, add, remove, rule, label) in [
        (
            "",
            r"\AddToHook",
            r"\RemoveFromHook",
            r"\DeclareHookRule",
            "alpha",
        ),
        (
            "",
            r"\csname AddToHook\endcsname",
            r"\csname RemoveFromHook\endcsname",
            r"\csname DeclareHookRule\endcsname",
            "alpha",
        ),
        (
            r"\def\register{\AddToHook}\def\remove{\RemoveFromHook}\def\ordering{\DeclareHookRule}",
            r"\register",
            r"\remove",
            r"\ordering",
            "alpha",
        ),
        (
            "",
            r"\AddToHook",
            r"\RemoveFromHook",
            r"\DeclareHookRule",
            " % before\nalpha% after\n",
        ),
    ] {
        let source = format!("\\documentclass{{article}}{preamble}\\NewHook{{one}}{add}{{one}}[{label}]{{\\typeout{{EVENT:A1}}}}{add}{{one}}[{label}]{{\\typeout{{EVENT:A2}}}}{add}{{one}}[beta]{{\\typeout{{EVENT:B}}}}{rule}{{one}}{{alpha}}{{after}}{{beta}}\\begin{{document}}\\UseHook{{one}}{remove}{{one}}[alpha]\\UseHook{{one}}Text\\label{{safe}}\\ref{{safe}}\\end{{document}}");
        let (workspace, uri) = setup(&source);
        let all = locations(&uri, &source, "alpha");
        for site in &all {
            assert_eq!(
                features::definitions(&workspace, &uri, site.range.start),
                all[..2]
            );
            assert_eq!(
                features::references(&workspace, &uri, site.range.start, true),
                all
            );
        }
        let (events, aux) = compile(&source);
        assert_eq!(events, ["EVENT:B", "EVENT:A1", "EVENT:A2", "EVENT:B"]);
        let text = &workspace.documents[&uri].text;
        let mut edits = features::rename(
            &workspace,
            &uri,
            text.position(source.rfind("safe").unwrap()),
            "fresh",
        )
        .unwrap()
        .unwrap()
        .changes
        .unwrap()[&uri]
            .clone();
        assert_eq!(edits.len(), 2);
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
        let (after_events, after_aux) = compile(&changed.source);
        assert_eq!(events, after_events);
        assert_eq!(aux, after_aux.replace("{fresh}", "{safe}"));
    }
}

#[test]
#[ignore = "requires native pdflatex"]
fn native_global_default_rules_connect_labels_across_distinct_hooks() {
    let source = r"\documentclass{article}\NewHook{one}\NewHook{two}\AddToHook{one}[alpha]{\typeout{EVENT:one-alpha}}\AddToHook{one}[beta]{\typeout{EVENT:one-beta}}\AddToHook{two}[alpha]{\typeout{EVENT:two-alpha}}\AddToHook{two}[beta]{\typeout{EVENT:two-beta}}\DeclareDefaultHookRule{alpha}{after}{beta}\begin{document}\UseHook{one}\UseHook{two}Text\label{safe}\ref{safe}\end{document}";
    let partial = source.replace(r"\AddToHook{one}[alpha]", r"\AddToHook{one}[fresh]");
    let default_rule = partial.replace(
        r"\DeclareDefaultHookRule{alpha}",
        r"\DeclareDefaultHookRule{fresh}",
    );
    let complete = default_rule.replace(r"\AddToHook{two}[alpha]", r"\AddToHook{two}[fresh]");
    let before = compile(source);
    let one_changed = compile(&partial);
    let rule_changed = compile(&default_rule);
    let all_changed = compile(&complete);
    assert_eq!(
        before.0,
        [
            "EVENT:one-beta",
            "EVENT:one-alpha",
            "EVENT:two-beta",
            "EVENT:two-alpha"
        ]
    );
    assert_eq!(
        one_changed.0,
        [
            "EVENT:one-alpha",
            "EVENT:one-beta",
            "EVENT:two-beta",
            "EVENT:two-alpha"
        ]
    );
    assert_eq!(
        rule_changed.0,
        [
            "EVENT:one-beta",
            "EVENT:one-alpha",
            "EVENT:two-alpha",
            "EVENT:two-beta"
        ]
    );
    assert_eq!(all_changed.0, before.0);
    for (_, aux) in [one_changed, rule_changed, all_changed] {
        assert_eq!(aux, before.1);
    }
}
