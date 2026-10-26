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
fn global_rule_labels_link_all_hooks_without_merging_local_definition_identity() {
    let source = "% global rules\r\n😀 \\AddToHook{one}[alpha]{A}\\AddToHook{one}[alpha]{B}\\AddToHook{two}[alpha]{C}\\AtBeginDocument[alpha]{D}\\AtEndDocument[alpha]{E}\\RemoveFromHook{one}[alpha]\\DeclareHookRule{one}{alpha}{after}{beta}\\DeclareDefaultHookRule{alpha}{before}{beta}\\DeclareDefaultHookRule{beta}{after}{alpha}\\ClearHookRule{two}{beta}{alpha}\\label{alpha}\\ref{alpha}\\def\\alpha{Text}\\NewHook{alpha}";
    let (workspace, uri) = setup(source);
    let sites = locations(&uri, source, "alpha");
    assert_eq!(sites.len(), 14);
    for (definitions, reads) in [
        (vec![0, 1], vec![5, 6, 7, 8]),
        (vec![2], vec![7, 8, 9]),
        (vec![3], vec![7, 8]),
        (vec![4], vec![7, 8]),
    ] {
        let expected_definitions: Vec<_> = definitions.iter().map(|&i| sites[i].clone()).collect();
        let expected_reads: Vec<_> = reads.iter().map(|&i| sites[i].clone()).collect();
        for i in definitions {
            assert_eq!(
                features::definitions(&workspace, &uri, sites[i].range.start),
                expected_definitions
            );
            assert_eq!(
                features::references(&workspace, &uri, sites[i].range.start, false),
                expected_reads
            );
        }
    }
    for i in [7, 8] {
        let at = sites[i].range.start;
        assert!(features::prepare_rename(&workspace, &uri, at)
            .unwrap()
            .is_none());
        assert!(features::rename(&workspace, &uri, at, "fresh")
            .unwrap()
            .is_none());
        assert_eq!(features::definitions(&workspace, &uri, at), sites[..5]);
        assert_eq!(
            features::references(&workspace, &uri, at, true),
            sites[..10]
        );
        assert_eq!(
            features::references(&workspace, &uri, at, false),
            sites[5..10]
        );
        let hover = features::hover(&workspace, &uri, at).unwrap();
        assert_eq!(hover.range, Some(sites[i].range));
        assert!(
            matches!(hover.contents, HoverContents::Markup(MarkupContent { value, .. }) if value.contains("All hooks (default ordering rule)"))
        );
        let highlights = editing::highlights(&workspace, &uri, at);
        assert_eq!(highlights.len(), 10);
        assert!(highlights[..5]
            .iter()
            .all(|item| item.kind == Some(DocumentHighlightKind::WRITE)));
        assert!(highlights[5..]
            .iter()
            .all(|item| item.kind == Some(DocumentHighlightKind::READ)));
    }
    assert_eq!(
        features::definitions(&workspace, &uri, sites[11].range.start),
        sites[10..11]
    );
}

#[test]
fn global_rule_completion_unions_hook_labels_and_preserves_full_field_ranges() {
    for tail in [
        r"\DeclareDefaultHookRule{alFAKE}{before}{beta}",
        r"\DeclareDefaultHookRule{beta}{after}{alFAKE}",
        r"\DeclareDefaultHookRule{alFAKE",
        r"\DeclareDefaultHookRule{beta}{after}{alFAKE",
    ] {
        let source = format!("% global\r\n😀 \\AddToHook{{one}}[alpha]{{A}}\\AddToHook{{two}}[alphabet]{{B}}\\AtEndDocument[alpine]{{C}}\\label{{albatross}}{tail}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let at = source.rfind("alFAKE").unwrap();
        let items = features::completion(&workspace, &uri, text.position(at + 2));
        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "alphabet", "alpine"],
            "{source}"
        );
        for item in items {
            assert_eq!(item.detail.as_deref(), Some("Code label across hooks"));
            assert_eq!(
                item.text_edit,
                Some(CompletionTextEdit::Edit(TextEdit {
                    range: text.range(at, at + 6),
                    new_text: item.label
                }))
            );
        }
    }
    for tail in [
        r"\DeclareDefaultHookRule{}{before}{beta}",
        r"\DeclareDefaultHookRule{",
    ] {
        let source = format!(r"\AddToHook{{one}}[alpha]{{A}}{tail}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let at =
            source.rfind(r"\DeclareDefaultHookRule{").unwrap() + r"\DeclareDefaultHookRule{".len();
        let items = features::completion(&workspace, &uri, text.position(at));
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].text_edit,
            Some(CompletionTextEdit::Edit(TextEdit {
                range: text.range(at, at),
                new_text: "alpha".into()
            }))
        );
    }
}

#[test]
fn global_rules_follow_confirmed_named_and_forwarded_interfaces_and_bare_fields() {
    for (preamble, call) in [
        ("", r"\DeclareDefaultHookRule"),
        ("", r"\csuse{DeclareDefaultHookRule}"),
        ("", r"\csname DeclareDefaultHookRule\endcsname"),
        (r"\def\order{\DeclareDefaultHookRule}", r"\order"),
        (
            r"\NewDocumentCommand\order{}{\DeclareDefaultHookRule}",
            r"\order",
        ),
        (
            r"\newenvironment{order}{\DeclareDefaultHookRule}{}",
            r"\order",
        ),
        (
            r"\def\DeclareHookRule{\DeclareDefaultHookRule}",
            r"\DeclareHookRule",
        ),
    ] {
        let source = format!(
            r"{preamble}\AddToHook{{one}}[alpha]{{A}}\AddToHook{{two}}[alpha]{{B}}{call}{{alpha}}{{after}}{{beta}}"
        );
        let (workspace, uri) = setup(&source);
        let sites = locations(&uri, &source, "alpha");
        assert_eq!(
            features::definitions(&workspace, &uri, sites[2].range.start),
            sites[..2],
            "{source}"
        );
        assert_eq!(
            features::references(&workspace, &uri, sites[2].range.start, true),
            sites,
            "{source}"
        );
    }
    let source = "\\AddToHook{one}[alphabet]{A}\\AddToHook{two}[beta]{B}\\DeclareDefaultHookRule % boundary\n{ alphabet }{\\unknown}{ beta }\\DeclareDefaultHookRule a{before}b";
    let (workspace, uri) = setup(source);
    let sites = locations(&uri, source, "alphabet");
    assert_eq!(
        features::definitions(&workspace, &uri, sites[1].range.start),
        sites[..1]
    );
    let at = source.rfind("a{before}").unwrap();
    let text = &workspace.documents[&uri].text;
    let items = features::completion(&workspace, &uri, text.position(at + 1));
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].text_edit,
        Some(CompletionTextEdit::Edit(TextEdit {
            range: text.range(at, at + 1),
            new_text: "{alphabet}".into()
        }))
    );
}

#[test]
fn reserved_global_rule_table_and_current_local_forwarders_keep_their_scope() {
    for tail in [
        r"\DeclareHookRule{ ?? }{alpha}{after}{beta}",
        r"\ClearHookRule{??}{alpha}{beta}",
        r"\DeclareDefaultHookRule{\unknown}{after}{alpha}",
        r"\DeclareDefaultHookRule{beta}{\unknown}{alpha}",
    ] {
        let source = format!(r"\AddToHook{{one}}[alpha]{{A}}\AddToHook{{two}}[alpha]{{B}}{tail}");
        let (workspace, uri) = setup(&source);
        let sites = locations(&uri, &source, "alpha");
        assert_eq!(
            features::definitions(&workspace, &uri, sites[2].range.start),
            sites[..2],
            "{source}"
        );
        assert_eq!(
            features::references(&workspace, &uri, sites[2].range.start, true),
            sites,
            "{source}"
        );
    }
    let source = r"\def\DeclareDefaultHookRule{\DeclareHookRule}\AddToHook{one}[alpha]{A}\AddToHook{two}[alpha]{B}\DeclareDefaultHookRule{one}{alpha}{after}{beta}";
    let (workspace, uri) = setup(source);
    let sites = locations(&uri, source, "alpha");
    assert_eq!(
        features::definitions(&workspace, &uri, sites[2].range.start),
        sites[..1]
    );
    assert_eq!(
        features::references(&workspace, &uri, sites[2].range.start, true),
        [sites[0].clone(), sites[2].clone()]
    );
}

#[test]
fn global_rule_data_dynamic_labels_and_replaced_interfaces_do_not_create_navigation() {
    for tail in [
        r"\detokenize{\DeclareDefaultHookRule{alpha}{after}{beta}}",
        r"\string\DeclareDefaultHookRule{alpha}{after}{beta}",
        r"\NewHookWithArguments{capture}{1}\UseHook{capture}{\DeclareDefaultHookRule{alpha}{after}{beta}}",
        r"\def\DeclareDefaultHookRule#1#2#3{}\DeclareDefaultHookRule{alpha}{after}{beta}",
        r"\def\order#1{\DeclareDefaultHookRule}\order{Ignore}{alpha}{after}{beta}",
        r"\DeclareDefaultHookRule{alpha\which}{after}{beta}",
        r"\DeclareDefaultHookRule{./alpha}{after}{beta}",
        "\\DeclareDefaultHookRule{al% split\nalpha}{after}{beta}",
    ] {
        let source = format!(r"\AddToHook{{one}}[alpha]{{A}}{tail}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("alpha").unwrap());
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn global_rule_providers_refresh_unsaved_interfaces_and_isolate_unrelated_roots() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = main.join("hooks.tex").unwrap();
    let other = main.join("other.tex").unwrap();
    let source = "% rules\r\n\\documentclass{article}\\input{hooks}😀 \\order{alpha}{after}{beta}";
    let supplied =
        r"\def\order{\DeclareDefaultHookRule}\AddToHook{one}[alpha]{A}\AddToHook{two}[alpha]{B}";
    for (uri, text) in [(&main, source), (&provider, supplied)] {
        std::fs::write(uri.to_file_path().unwrap(), text).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(main.clone(), source.into(), 5);
    let at = locations(&main, source, "alpha")[0].range.start;
    let expected = locations(&provider, supplied, "alpha");
    assert_eq!(features::definitions(&workspace, &main, at), expected);
    let replaced = supplied.replace(
        r"\def\order{\DeclareDefaultHookRule}",
        r"\def\order#1#2#3{}",
    );
    workspace.open(provider.clone(), replaced, 12);
    assert!(features::definitions(&workspace, &main, at).is_empty());
    workspace.open(provider.clone(), supplied.into(), 13);
    assert_eq!(features::definitions(&workspace, &main, at), expected);
    let local = supplied.replace(r"\DeclareDefaultHookRule", r"\DeclareHookRule");
    workspace.open(provider.clone(), local, 14);
    assert!(features::definitions(&workspace, &main, at).is_empty());
    workspace.documents.remove(&provider);
    workspace.reload(&provider);
    assert_eq!(features::definitions(&workspace, &main, at), expected);
    workspace.open(
        other,
        r"\documentclass{article}\AddToHook{other}[alpha]{Other}".into(),
        8,
    );
    assert_eq!(features::definitions(&workspace, &main, at), expected);
    assert_eq!(workspace.documents[&main].version, Some(5));
}

#[test]
fn shared_global_rule_fields_require_consensus_without_losing_local_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let a = Url::from_file_path(directory.path().join("a.tex")).unwrap();
    let b = a.join("b.tex").unwrap();
    let shared = a.join("shared.tex").unwrap();
    let supplied = r"\AddToHook{one}[alpha]{A}\AddToHook{two}[alpha]{B}\DeclareDefaultHookRule{alpha}{after}{beta}";
    let source = r"\documentclass{article}\input{shared}\begin{document}\RemoveFromHook{one}[alpha]\end{document}";
    for (uri, text) in [(&a, source), (&b, source), (&shared, supplied)] {
        std::fs::write(uri.to_file_path().unwrap(), text).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(a.clone(), source.into(), 5);
    workspace.open(b.clone(), source.into(), 7);
    let sites = locations(&shared, supplied, "alpha");
    let global = sites[2].range.start;
    let local = locations(&a, source, "alpha")[0].range.start;
    assert_eq!(
        features::definitions(&workspace, &shared, global),
        sites[..2]
    );
    assert_eq!(
        features::references(&workspace, &shared, global, true).len(),
        5
    );
    assert_eq!(features::references(&workspace, &a, local, true).len(), 4);
    workspace.open(
        b.clone(),
        format!(r"\def\DeclareDefaultHookRule#1#2#3{{}}{source}"),
        9,
    );
    assert!(features::definitions(&workspace, &shared, global).is_empty());
    assert_eq!(features::definitions(&workspace, &a, local), sites[..1]);
    assert_eq!(features::references(&workspace, &a, local, true).len(), 3);
    workspace.open(b, source.into(), 11);
    assert_eq!(
        features::definitions(&workspace, &shared, global),
        sites[..2]
    );
    assert_eq!(features::references(&workspace, &a, local, true).len(), 4);
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
fn lifecycle_labels_share_kernel_hook_identity_and_keep_begin_end_namespaces_separate() {
    let source = "% lifecycle\r\n😀 \\AtBeginDocument[alpha]{A1}\\AtBeginDocument[alpha]{A2}\\AddToHook{begindocument}[alpha]{A3}\\AtEndDocument[alpha]{E1}\\AtEndDocument[alpha]{E2}\\AddToHook{enddocument}[alpha]{E3}\\RemoveFromHook{begindocument}[alpha]\\DeclareHookRule{begindocument}{alpha}{after}{beta}\\ClearHookRule{enddocument}{beta}{alpha}\\label{alpha}\\ref{alpha}\\def\\alpha{Text}";
    let (workspace, uri) = setup(source);
    let all = locations(&uri, source, "alpha");
    assert_eq!(all.len(), 12);
    for (hook, registrations, uses) in [
        ("begindocument", [0, 1, 2], vec![6, 7]),
        ("enddocument", [3, 4, 5], vec![8]),
    ] {
        let definitions: Vec<_> = registrations.iter().map(|&i| all[i].clone()).collect();
        let reads: Vec<_> = uses.iter().map(|&i| all[i].clone()).collect();
        let expected: Vec<_> = definitions.iter().chain(&reads).cloned().collect();
        for site in &expected {
            assert_eq!(
                features::definitions(&workspace, &uri, site.range.start),
                definitions
            );
            assert_eq!(
                features::references(&workspace, &uri, site.range.start, true),
                expected
            );
            assert_eq!(
                features::references(&workspace, &uri, site.range.start, false),
                reads
            );
            let hover = features::hover(&workspace, &uri, site.range.start).unwrap();
            assert_eq!(hover.range, Some(site.range));
            assert!(
                matches!(hover.contents, HoverContents::Markup(MarkupContent { value, .. }) if value.contains(hook) && value.contains("code label"))
            );
        }
        let highlights = editing::highlights(&workspace, &uri, reads[0].range.start);
        assert_eq!(highlights.len(), expected.len());
        assert!(highlights[..3]
            .iter()
            .all(|item| item.kind == Some(DocumentHighlightKind::WRITE)));
    }
    assert_eq!(
        features::definitions(&workspace, &uri, all[10].range.start),
        all[9..10]
    );
}

#[test]
fn lifecycle_label_completion_filters_implicit_hook_and_preserves_entire_suffix() {
    for (command, hook, expected) in [
        (r"\AtBeginDocument", "begindocument", "alphabet"),
        (r"\AtEndDocument", "enddocument", "alpine"),
    ] {
        for tail in ["[alFAKE]{Code}", "[alFAKE", "[]", "["] {
            let source = format!("% lifecycle\r\n😀 \\AtBeginDocument[alphabet]{{Begin}}\\AtEndDocument[alpine]{{End}}{command}{tail}");
            let (workspace, uri) = setup(&source);
            let start = source.rfind('[').unwrap() + 1;
            let length = if tail.contains("alFAKE") { 6 } else { 0 };
            let text = &workspace.documents[&uri].text;
            let items =
                features::completion(&workspace, &uri, text.position(start + length.min(2)));
            assert_eq!(items.len(), 1, "{source}");
            assert_eq!(items[0].label, expected);
            assert_eq!(
                items[0].detail.as_deref(),
                Some(format!("Code label in hook {hook}").as_str())
            );
            assert_eq!(
                items[0].text_edit,
                Some(CompletionTextEdit::Edit(TextEdit {
                    range: text.range(start, start + length),
                    new_text: expected.into()
                }))
            );
        }
    }
}

#[test]
fn lifecycle_labels_follow_static_named_traditional_xparse_and_environment_forwarders() {
    for (preamble, begin, end) in [
        (
            "",
            r"\csuse{AtBeginDocument}",
            r"\csname AtEndDocument\endcsname",
        ),
        (
            r"\def\startup{\AtBeginDocument}\def\shutdown{\AtEndDocument}",
            r"\startup",
            r"\shutdown",
        ),
        (
            r"\NewDocumentCommand\startup{}{\AtBeginDocument}\NewDocumentCommand\shutdown{}{\AtEndDocument}",
            r"\startup",
            r"\shutdown",
        ),
        (
            r"\newenvironment{startup}{\AtBeginDocument}{}\newenvironment{shutdown}{}{\AtEndDocument}",
            r"\startup",
            r"\endshutdown",
        ),
    ] {
        let source = format!(
            r"{preamble}{begin}[alpha]{{Start}}{end}[alpha]{{End}}\RemoveFromHook{{begindocument}}[alpha]\RemoveFromHook{{enddocument}}[alpha]"
        );
        let (workspace, uri) = setup(&source);
        let all = locations(&uri, &source, "alpha");
        assert_eq!(all.len(), 4);
        for (declaration, usage) in [(0, 2), (1, 3)] {
            let expected = [all[declaration].clone(), all[usage].clone()];
            for site in &expected {
                assert_eq!(
                    features::definitions(&workspace, &uri, site.range.start),
                    expected[..1],
                    "{source}"
                );
                assert_eq!(
                    features::references(&workspace, &uri, site.range.start, true),
                    expected,
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn lifecycle_labels_require_both_interfaces_and_follow_the_current_builtin_leaf() {
    for call in [
        r"\def\AddToHook#1[#2]#3{}\AtBeginDocument[alpha]{Code}",
        r"\def\AddToHook#1[#2]#3{}\AtEndDocument[alpha]{Code}",
        r"\def\AtBeginDocument[#1]#2{}\AtBeginDocument[alpha]{Code}",
        r"\def\AtEndDocument[#1]#2{}\AtEndDocument[alpha]{Code}",
        r"\def\csuse#1{}\csuse{AtBeginDocument}[alpha]{Code}",
        r"\detokenize{\AtBeginDocument[alpha]{Code}}",
        r"\string\AtBeginDocument[alpha]{Code}",
        r"\NewHookWithArguments{capture}{1}\UseHook{capture}{\AtEndDocument[alpha]{Code}}",
        r"\AtBeginDocument[./alpha]{Code}",
        r"\AtEndDocument[alpha\which]{Code}",
        r"\AtBeginDocument{alpha}",
    ] {
        let source = format!(r"\AtBeginDocument[alpha]{{Start}}\AtEndDocument[alpha]{{End}}{call}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("alpha").unwrap());
        assert!(
            features::definitions(&workspace, &uri, at).is_empty(),
            "{source}"
        );
    }
    let source = r"\def\AtBeginDocument{\AtEndDocument}\AtBeginDocument[alpha]{Moved}\AtEndDocument[alpha]{End}\RemoveFromHook{enddocument}[alpha]\RemoveFromHook{begindocument}[alpha]";
    let (workspace, uri) = setup(source);
    let all = locations(&uri, source, "alpha");
    for site in &all[..3] {
        assert_eq!(
            features::definitions(&workspace, &uri, site.range.start),
            all[..2]
        );
        assert_eq!(
            features::references(&workspace, &uri, site.range.start, true),
            all[..3]
        );
    }
    assert!(features::definitions(&workspace, &uri, all[3].range.start).is_empty());
    let source = r"\def\AtBeginDocument{\AddToHook}\AtBeginDocument{custom}[alpha]{Code}\RemoveFromHook{custom}[alpha]";
    let (workspace, uri) = setup(source);
    let all = locations(&uri, source, "alpha");
    assert_eq!(
        features::definitions(&workspace, &uri, all[1].range.start),
        all[..1]
    );
}

#[test]
fn lifecycle_label_providers_refresh_unsaved_meaning_and_all_shared_consumers() {
    let directory = tempfile::tempdir().unwrap();
    let a = Url::from_file_path(directory.path().join("a.tex")).unwrap();
    let b = a.join("b.tex").unwrap();
    let provider = a.join("hooks.tex").unwrap();
    let provided =
        r"\AtBeginDocument[alpha]{Start}\AtEndDocument[alpha]{End}\def\register{\AtBeginDocument}";
    let source = r"\documentclass{article}\input{hooks}\register[alpha]{Own}\begin{document}\RemoveFromHook{begindocument}[alpha]\end{document}";
    for (uri, text) in [(&a, source), (&b, source), (&provider, provided)] {
        std::fs::write(uri.to_file_path().unwrap(), text).unwrap();
    }
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(a.clone(), source.into(), 5);
    workspace.open(b.clone(), source.into(), 7);
    let sites = locations(&provider, provided, "alpha");
    let own = locations(&a, source, "alpha");
    let expected = [own[0].clone(), sites[0].clone()];
    assert_eq!(
        features::definitions(&workspace, &a, own[1].range.start),
        expected
    );
    assert_eq!(
        features::references(&workspace, &a, own[1].range.start, true).len(),
        5
    );
    workspace.open(b.clone(), format!(r"\def\AddToHook#1[#2]#3{{}}{source}"), 9);
    assert!(features::definitions(&workspace, &provider, sites[0].range.start).is_empty());
    assert_eq!(
        features::definitions(&workspace, &a, own[1].range.start),
        own[..1]
    );
    assert_eq!(
        features::references(&workspace, &a, own[1].range.start, true).len(),
        2
    );
    workspace.open(b, source.into(), 11);
    assert_eq!(
        features::definitions(&workspace, &a, own[1].range.start),
        expected
    );
    let moved = provided.replace("AtBeginDocument", "AtEndDocument");
    workspace.open(provider.clone(), moved, 12);
    // The alias also moved to AtEndDocument, so neither begin registration remains.
    assert!(features::definitions(&workspace, &a, own[1].range.start).is_empty());
    workspace.open(provider.clone(), provided.into(), 13);
    assert_eq!(
        features::definitions(&workspace, &a, own[1].range.start),
        expected
    );
    workspace.open(
        provider.clone(),
        format!(r"\def\AddToHook#1[#2]#3{{}}{provided}"),
        14,
    );
    assert!(features::definitions(&workspace, &a, own[1].range.start).is_empty());
    workspace.documents.remove(&provider);
    workspace.reload(&provider);
    assert_eq!(
        features::definitions(&workspace, &a, own[1].range.start),
        expected
    );
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
        r"\def\DeclareDefaultHookRule#1#2#3{}\DeclareDefaultHookRule{alpha}{after}{beta}",
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
fn native_reserved_global_rules_clears_and_local_overrides_match_source_scope() {
    let base = r"\documentclass{article}\NewHook{one}\NewHook{two}\AddToHook{one}[alpha]{\typeout{EVENT:one-A}}\AddToHook{one}[beta]{\typeout{EVENT:one-B}}\AddToHook{two}[alpha]{\typeout{EVENT:two-A}}\AddToHook{two}[beta]{\typeout{EVENT:two-B}}";
    for (rule, events, local_override) in [
        (
            r"\DeclareDefaultHookRule{alpha}{after}{beta}",
            ["one-B", "one-A", "two-B", "two-A"],
            false,
        ),
        (
            r"\DeclareHookRule{ ?? }{alpha}{after}{beta}",
            ["one-B", "one-A", "two-B", "two-A"],
            false,
        ),
        (
            r"\DeclareDefaultHookRule{alpha}{after}{beta}\ClearHookRule{??}{alpha}{beta}",
            ["one-A", "one-B", "two-A", "two-B"],
            false,
        ),
        (
            r"\DeclareDefaultHookRule{alpha}{after}{beta}\DeclareHookRule{one}{alpha}{before}{beta}",
            ["one-A", "one-B", "two-B", "two-A"],
            true,
        ),
    ] {
        let source = format!(
            r"{base}{rule}\begin{{document}}\UseHook{{one}}\UseHook{{two}}Text\end{{document}}"
        );
        let (workspace, uri) = setup(&source);
        let sites = locations(&uri, &source, "alpha");
        for (i, site) in sites.iter().enumerate().skip(2) {
            let expected = if local_override && i == 3 {
                &sites[..1]
            } else {
                &sites[..2]
            };
            assert_eq!(
                features::definitions(&workspace, &uri, site.range.start),
                expected
            );
        }
        assert_eq!(
            features::references(&workspace, &uri, sites[2].range.start, true),
            sites
        );
        assert_eq!(
            compile(&source).0,
            events.map(|event| format!("EVENT:{event}"))
        );
    }
}

#[test]
#[ignore = "requires native pdflatex"]
fn native_global_rule_navigation_and_ordinary_key_edits_preserve_all_hook_events() {
    for (preamble, call) in [
        ("", r"\DeclareDefaultHookRule"),
        ("", r"\csname DeclareDefaultHookRule\endcsname"),
        (r"\def\order{\DeclareDefaultHookRule}", r"\order"),
    ] {
        let source = format!(
            r"\documentclass{{article}}{preamble}\NewHook{{one}}\NewHook{{two}}\AddToHook{{one}}[alpha]{{\typeout{{EVENT:one-A}}}}\AddToHook{{one}}[beta]{{\typeout{{EVENT:one-B}}}}\AddToHook{{two}}[alpha]{{\typeout{{EVENT:two-A}}}}\AddToHook{{two}}[beta]{{\typeout{{EVENT:two-B}}}}{call}{{alpha}}{{after}}{{beta}}\begin{{document}}\UseHook{{one}}\UseHook{{two}}Text\label{{alpha}}\ref{{alpha}}\end{{document}}"
        );
        let (workspace, uri) = setup(&source);
        let sites = locations(&uri, &source, "alpha");
        assert_eq!(sites.len(), 5);
        assert_eq!(
            features::definitions(&workspace, &uri, sites[2].range.start),
            sites[..2]
        );
        assert_eq!(
            features::references(&workspace, &uri, sites[2].range.start, true),
            sites[..3]
        );
        let before = compile(&source);
        assert_eq!(
            before.0,
            ["EVENT:one-B", "EVENT:one-A", "EVENT:two-B", "EVENT:two-A"]
        );
        let mut edits = features::rename(&workspace, &uri, sites[4].range.start, "fresh")
            .unwrap()
            .unwrap()
            .changes
            .unwrap()
            .remove(&uri)
            .unwrap();
        assert_eq!(
            edits,
            sites[3..]
                .iter()
                .map(|site| TextEdit {
                    range: site.range,
                    new_text: "fresh".into()
                })
                .collect::<Vec<_>>()
        );
        edits.reverse();
        let changed = Text::new(source)
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
        let after = compile(&changed.source);
        assert_eq!(before.0, after.0);
        assert_eq!(before.1, after.1.replace("{fresh}", "{alpha}"));
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

#[test]
#[ignore = "requires native pdflatex and etoolbox"]
fn native_lifecycle_labels_keep_combined_chunks_rules_and_ordinary_key_edits() {
    for (preamble, begin, end, label) in [
        ("", r"\AtBeginDocument", r"\AtEndDocument", "alpha"),
        (
            r"\usepackage{etoolbox}",
            r"\csuse{AtBeginDocument}",
            r"\csname AtEndDocument\endcsname",
            "alpha",
        ),
        (
            r"\def\startup{\AtBeginDocument}\def\shutdown{\AtEndDocument}",
            r"\startup",
            r"\shutdown",
            "alpha",
        ),
        (
            r"\newenvironment{startup}{\AtBeginDocument}{}\newenvironment{shutdown}{}{\AtEndDocument}",
            r"\startup",
            r"\endshutdown",
            "alpha",
        ),
        (
            "",
            r"\AtBeginDocument",
            r"\AtEndDocument",
            " % before\nalpha% after\n",
        ),
    ] {
        let source = format!("% lifecycle 😀\r\n\\documentclass{{article}}{preamble}{begin}[{label}]{{\\typeout{{EVENT:A1}}}}{begin}[{label}]{{\\typeout{{EVENT:A2}}}}\\AddToHook{{begindocument}}[alpha]{{\\typeout{{EVENT:A3}}}}\\AddToHook{{begindocument}}[beta]{{\\typeout{{EVENT:B}}}}{end}[{label}]{{\\typeout{{EVENT:E1}}}}{end}[{label}]{{\\typeout{{EVENT:E2}}}}\\AddToHook{{enddocument}}[beta]{{\\typeout{{EVENT:Z}}}}\\DeclareHookRule{{begindocument}}{{alpha}}{{after}}{{beta}}\\DeclareHookRule{{enddocument}}{{alpha}}{{after}}{{beta}}\\begin{{document}}\\typeout{{EVENT:BODY}}Text\\label{{alpha}}\\ref{{alpha}}\\end{{document}}");
        let (workspace, uri) = setup(&source);
        let all = locations(&uri, &source, "alpha");
        assert_eq!(all.len(), 9);
        for (registrations, usage) in [(0..3, 5), (3..5, 6)] {
            let mut refs = all[registrations.clone()].to_vec();
            refs.push(all[usage].clone());
            for site in &refs {
                assert_eq!(
                    features::definitions(&workspace, &uri, site.range.start),
                    all[registrations.clone()],
                    "{source}"
                );
                assert_eq!(
                    features::references(&workspace, &uri, site.range.start, true),
                    refs,
                    "{source}"
                );
            }
        }
        assert_eq!(
            features::definitions(&workspace, &uri, all[8].range.start),
            all[7..8]
        );
        let (events, aux) = compile(&source);
        assert_eq!(
            events,
            [
                "EVENT:B",
                "EVENT:A1",
                "EVENT:A2",
                "EVENT:A3",
                "EVENT:BODY",
                "EVENT:Z",
                "EVENT:E1",
                "EVENT:E2"
            ]
        );
        let text = &workspace.documents[&uri].text;
        let mut edits = features::rename(&workspace, &uri, all[8].range.start, "fresh")
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
        assert_eq!(
            changed.source,
            source
                .replace(r"\label{alpha}", r"\label{fresh}")
                .replace(r"\ref{alpha}", r"\ref{fresh}")
        );
        let (after_events, after_aux) = compile(&changed.source);
        assert_eq!(events, after_events);
        assert_eq!(aux, after_aux.replace("{fresh}", "{alpha}"));
    }
}

#[test]
#[ignore = "requires native pdflatex"]
fn native_lifecycle_interface_changes_move_or_discard_callbacks_with_equal_aux() {
    let mut original_aux = None;
    for (preamble, hook, events) in [
        (
            r"\AtBeginDocument[alpha]{\typeout{EVENT:begin}}",
            Some("begindocument"),
            vec!["EVENT:begin", "EVENT:BODY"],
        ),
        (
            r"\def\AtBeginDocument{\AtEndDocument}\AtBeginDocument[alpha]{\typeout{EVENT:moved}}",
            Some("enddocument"),
            vec!["EVENT:BODY", "EVENT:moved"],
        ),
        (
            r"{\long\def\AddToHook#1[#2]#3{}\AtBeginDocument[alpha]{\typeout{EVENT:ignored}}}",
            None,
            vec!["EVENT:BODY"],
        ),
        (
            r"{\long\def\AtBeginDocument[#1]#2{}\AtBeginDocument[alpha]{\typeout{EVENT:ignored}}}",
            None,
            vec!["EVENT:BODY"],
        ),
        (
            r"\NewHookWithArguments{capture}{1}\UseHook{capture}{\AtEndDocument[alpha]{\typeout{EVENT:ignored}}}",
            None,
            vec!["EVENT:BODY"],
        ),
    ] {
        let source = format!(
            r"\documentclass{{article}}{preamble}\begin{{document}}\typeout{{EVENT:BODY}}Text\label{{safe}}\ref{{safe}}\end{{document}}"
        );
        let (workspace, uri) = setup(&source);
        let all = locations(&uri, &source, "alpha");
        assert_eq!(all.len(), 1);
        let definitions = features::definitions(&workspace, &uri, all[0].range.start);
        if let Some(hook) = hook {
            assert_eq!(definitions, all);
            let hover = features::hover(&workspace, &uri, all[0].range.start).unwrap();
            assert!(
                matches!(hover.contents, HoverContents::Markup(MarkupContent {value, ..}) if value.contains(hook))
            );
        } else {
            assert!(definitions.is_empty(), "{source}");
        }
        let (actual, aux) = compile(&source);
        assert_eq!(actual, events);
        if let Some(before) = &original_aux {
            assert_eq!(&aux, before);
        } else {
            original_aux = Some(aux);
        }
    }
}
