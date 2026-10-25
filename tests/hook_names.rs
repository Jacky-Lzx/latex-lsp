use latex_lsp::{editing, features, text::Text, workspace::Workspace};
use tower_lsp::lsp_types::*;

fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:hook-names.tex").unwrap();
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
fn literal_hook_names_navigate_across_direct_named_and_forwarded_calls() {
    for (preamble, new, add, use_hook) in [
        ("", r"\NewHook", r"\AddToHook", r"\UseHook"),
        (
            "",
            r"\csuse{NewHook}",
            r"\csname AddToHook\endcsname",
            r"\csuse{UseHook}",
        ),
        (
            r"\def\declare{\NewHook}\def\register{\AddToHook}\def\invoke{\UseHook}",
            r"\declare",
            r"\register",
            r"\invoke",
        ),
    ] {
        let source =
            format!(r"{preamble}{new}{{custom}}{add}{{custom}}[mine]{{Code}}{use_hook}{{custom}}");
        let (workspace, uri) = setup(&source);
        let expected = locations(&uri, &source, "custom");
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
            assert_eq!(
                features::references(&workspace, &uri, site.range.start, false),
                expected[1..],
                "{source}"
            );
            let hover = features::hover(&workspace, &uri, site.range.start).unwrap();
            assert_eq!(hover.range, Some(site.range));
            assert!(
                matches!(hover.contents, HoverContents::Markup(MarkupContent { kind: MarkupKind::PlainText, value }) if value.contains("custom") && value.contains("LaTeX kernel hook"))
            );
        }
        let highlights = editing::highlights(&workspace, &uri, expected[2].range.start);
        assert_eq!(highlights.len(), 3);
        assert_eq!(highlights[0].kind, Some(DocumentHighlightKind::WRITE));
        assert!(highlights[1..]
            .iter()
            .all(|item| item.kind == Some(DocumentHighlightKind::READ)));
    }
}

#[test]
fn hook_declaration_variants_keep_mirrored_names_and_argument_counts_separate() {
    for declaration in [
        r"\NewHook{custom}",
        r"\NewReversedHook{custom}",
        r"\NewHookWithArguments{custom}{2}",
        r"\NewReversedHookWithArguments{custom}{2}",
        r"\NewMirroredHookPair{custom}{other}",
        r"\NewMirroredHookPairWithArguments{other}{custom}{2}",
    ] {
        let source = format!(r"{declaration}\AddToHook{{custom}}[other]{{Text}}\UseHook{{custom}}");
        let (workspace, uri) = setup(&source);
        let expected = locations(&uri, &source, "custom");
        assert_eq!(
            features::definitions(&workspace, &uri, expected[2].range.start),
            expected[..1],
            "{source}"
        );
        assert_eq!(
            features::references(&workspace, &uri, expected[2].range.start, true),
            expected,
            "{source}"
        );
    }
}

#[test]
fn hook_names_do_not_merge_with_labels_macros_or_code_labels() {
    let source = r"\NewHook{custom}\AddToHook{custom}[custom]{\ref{custom}}\DeclareHookRule{custom}{custom}{before}{last}\label{custom}\def\custom{Text}\UseHook{custom}";
    let (workspace, uri) = setup(source);
    let all = locations(&uri, source, "custom");
    let at = all.last().unwrap().range.start;
    assert_eq!(
        features::references(&workspace, &uri, at, true),
        [
            all[0].clone(),
            all[1].clone(),
            all[4].clone(),
            all[8].clone()
        ]
    );
    assert_eq!(
        features::definitions(&workspace, &uri, all[3].range.start),
        [all[6].clone()]
    );
    for index in [2, 5] {
        assert!(features::definitions(&workspace, &uri, all[index].range.start).is_empty());
    }
}

#[test]
fn normalized_registration_names_and_raw_use_names_keep_native_identity() {
    let source = r"\NewHook{ custom.before }\AddToHook{ custom.before }{Text}\UseHook{custom.before}\UseHook{ custom.before }\NewHook{./custom.before}\UseHook{./custom.before}";
    let (workspace, uri) = setup(source);
    let all = locations(&uri, source, "custom.before");
    assert_eq!(
        features::definitions(&workspace, &uri, all[2].range.start),
        all[..1]
    );
    assert_eq!(
        features::references(&workspace, &uri, all[2].range.start, true),
        all[..3]
    );
    for site in &all[3..] {
        assert!(features::definitions(&workspace, &uri, site.range.start).is_empty());
    }
}

#[test]
fn hook_calls_in_data_and_redefined_interfaces_do_not_create_navigation() {
    for tail in [
        r"\typeout{\unexpanded{\UseHook{custom}}}",
        r"\newcommand\discard[1]{}\discard{\UseHook{custom}}",
        r"\string\UseHook{custom}",
        r"\def\UseHook#1{}\UseHook{custom}",
        r"\def\csuse#1#2{}\csuse{UseHook}{custom}",
        r"\def\csname#1\endcsname{}\csname UseHook\endcsname{custom}",
    ] {
        let source = format!(r"\NewHook{{custom}}{tail}");
        let (workspace, uri) = setup(&source);
        let all = locations(&uri, &source, "custom");
        assert!(
            features::definitions(&workspace, &uri, all.last().unwrap().range.start).is_empty(),
            "{source}"
        );
    }
    let source = r"\NewHook{custom}\newcommand\later{\UseHook{custom}}";
    let (workspace, uri) = setup(source);
    let all = locations(&uri, source, "custom");
    assert_eq!(
        features::definitions(&workspace, &uri, all[1].range.start),
        all[..1]
    );
}

#[test]
fn hook_name_completion_replaces_the_suffix_with_exact_utf16_ranges() {
    for call in [
        r"\UseHook",
        r"\csuse{UseHook}",
        r"\csname UseHook\endcsname",
    ] {
        for suffix in ["}", ""] {
            let source = format!(
                "% hooks\r\n\\NewHook{{custom}}\\NewHook{{other}}😀 {call}{{cusTOM{suffix}"
            );
            let (workspace, uri) = setup(&source);
            let text = &workspace.documents[&uri].text;
            let start = source.rfind("cusTOM").unwrap();
            let items = features::completion(&workspace, &uri, text.position(start + 3));
            let custom = items.iter().find(|item| item.label == "custom").unwrap();
            assert_eq!(custom.kind, Some(CompletionItemKind::EVENT));
            assert_eq!(
                custom.text_edit,
                Some(CompletionTextEdit::Edit(TextEdit {
                    range: text.range(start, start + 6),
                    new_text: "custom".into()
                }))
            );
            assert!(!items.iter().any(|item| item.label == "other"));
        }
    }
}

#[test]
fn empty_hook_completion_and_raw_whitespace_preserve_the_selected_identity() {
    for tail in [r"\UseHook{}", r"\UseHook{", r"\AddToHook{}", r"\AddToHook{"] {
        let source = format!(r"\NewHook{{custom}}{tail}");
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        let at = source.rfind('{').unwrap() + 1;
        let items = features::completion(&workspace, &uri, text.position(at));
        let item = items.iter().find(|item| item.label == "custom").unwrap();
        assert_eq!(
            item.text_edit,
            Some(CompletionTextEdit::Edit(TextEdit {
                range: text.range(at, at),
                new_text: "custom".into()
            }))
        );
    }
    for tail in [r"\UseHook{ cus}", r"\UseHook{ cus"] {
        let source = format!(r"\NewHook{{custom}}{tail}");
        let (workspace, uri) = setup(&source);
        let at = workspace.documents[&uri]
            .text
            .position(source.rfind("cus").unwrap() + 3);
        assert!(!features::completion(&workspace, &uri, at)
            .iter()
            .any(|item| item.label == "custom"));
    }
}

#[test]
fn unsaved_cross_file_forwarders_refresh_names_without_parent_edits() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let provider = main.join("hooks.tex").unwrap();
    let source = "\\input{hooks}\r\n😀 \\invoke{custom}";
    let definitions = r"\NewHook{custom}\def\invoke{\UseHook}";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(provider.to_file_path().unwrap(), definitions).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(main.clone(), source.into(), 5);
    let query = locations(&main, source, "custom")[0].range.start;
    let expected = locations(&provider, definitions, "custom");
    assert_eq!(features::definitions(&workspace, &main, query), expected);
    workspace.open(
        provider.clone(),
        r"\NewHook{custom}\def\invoke#1{}".into(),
        12,
    );
    assert!(features::definitions(&workspace, &main, query).is_empty());
    assert_eq!(workspace.documents[&main].version, Some(5));
    workspace.open(provider.clone(), definitions.into(), 13);
    assert_eq!(features::definitions(&workspace, &main, query), expected);
    workspace.documents.remove(&provider);
    workspace.reload(&provider);
    assert_eq!(features::definitions(&workspace, &main, query), expected);
}

#[test]
fn shared_hook_calls_require_consistent_meanings_in_all_consumers() {
    let directory = tempfile::tempdir().unwrap();
    let a = Url::from_file_path(directory.path().join("a.tex")).unwrap();
    let b = a.join("b.tex").unwrap();
    let shared = a.join("shared.tex").unwrap();
    let declarations = r"\NewHook{custom}\UseHook{custom}";
    let source =
        r"\documentclass{article}\input{shared}\begin{document}\UseHook{custom}\end{document}";
    std::fs::write(shared.to_file_path().unwrap(), declarations).unwrap();
    std::fs::write(a.to_file_path().unwrap(), source).unwrap();
    std::fs::write(b.to_file_path().unwrap(), source).unwrap();
    let mut workspace = Workspace::default();
    workspace.config.distribution.enabled = false;
    workspace.open(a.clone(), source.into(), 5);
    workspace.open(b.clone(), source.into(), 7);
    let sites = locations(&shared, declarations, "custom");
    let own = locations(&a, source, "custom");
    assert_eq!(
        features::definitions(&workspace, &shared, sites[1].range.start),
        sites[..1]
    );
    assert_eq!(
        features::references(&workspace, &a, own[0].range.start, true).len(),
        4
    );
    workspace.open(b.clone(), format!(r"\def\UseHook#1{{}}{source}"), 9);
    assert!(features::definitions(&workspace, &shared, sites[1].range.start).is_empty());
    assert_eq!(
        features::definitions(&workspace, &a, own[0].range.start),
        sites[..1]
    );
    assert_eq!(
        features::references(&workspace, &a, own[0].range.start, true).len(),
        2
    );
    workspace.open(b, source.into(), 11);
    assert_eq!(
        features::definitions(&workspace, &shared, sites[1].range.start),
        sites[..1]
    );
}

#[test]
fn bare_hook_names_and_boundary_comments_have_precise_ranges() {
    let source = "\\NewMirroredHookPair ab\\AddToHook a{Code}\\UseHook a\\NewHook{% before\ncustom% after\n}\\UseHook{custom}";
    let (workspace, uri) = setup(source);
    let text = &workspace.documents[&uri].text;
    let declaration = source.find(" ab").unwrap() + 1;
    let use_hook = source.find(r"\UseHook a").unwrap() + 9;
    assert_eq!(
        features::definitions(&workspace, &uri, text.position(use_hook)),
        [Location {
            uri: uri.clone(),
            range: text.range(declaration, declaration + 1)
        }]
    );
    let all = locations(&uri, source, "custom");
    assert_eq!(
        features::definitions(&workspace, &uri, all[1].range.start),
        all[..1]
    );
}

#[test]
#[ignore = "requires pdflatex and etoolbox"]
fn native_hook_names_preserve_identity_and_independent_label_edits() {
    for (preamble, body, names, events) in [
        (
            r"\NewHook{custom}\AddToHook{custom}[mine]{\typeout{LSP-HOOK:A}}",
            r"\UseHook{custom}",
            vec!["custom"],
            vec!["LSP-HOOK:A"],
        ),
        (
            r"\csuse{NewHook}{custom}\csname AddToHook\endcsname{custom}{\typeout{LSP-HOOK:A}}",
            r"\csuse{UseHook}{custom}",
            vec!["custom"],
            vec!["LSP-HOOK:A"],
        ),
        (
            r"\def\declare{\NewHook}\declare{custom}\def\register{\AddToHook}\register{custom}{\typeout{LSP-HOOK:A}}\def\invoke{\UseHook}",
            r"\invoke{custom}",
            vec!["custom"],
            vec!["LSP-HOOK:A"],
        ),
        (
            r"\NewMirroredHookPair{custom/before}{custom/after}\AddToHook{custom/before}{\typeout{LSP-HOOK:A}}\AddToHook{custom/after}{\typeout{LSP-HOOK:B}}",
            r"\UseHook{custom/before}\UseHook{custom/after}",
            vec!["custom/before", "custom/after"],
            vec!["LSP-HOOK:A", "LSP-HOOK:B"],
        ),
        (
            r"\NewHook{ custom }\AddToHook{ custom }{\typeout{LSP-HOOK:A}}",
            r"\UseHook{custom}\UseHook{ custom }",
            vec!["custom"],
            vec!["LSP-HOOK:A"],
        ),
        (
            r"\NewReversedHook{custom.before}\AddToHook{custom.before}{\typeout{LSP-HOOK:A}}",
            r"\UseOneTimeHook{custom.before}",
            vec!["custom.before"],
            vec!["LSP-HOOK:A"],
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let source = format!(
            r"\documentclass{{article}}\usepackage{{etoolbox}}{preamble}\begin{{document}}{body}\label{{safe}}\ref{{safe}}\end{{document}}"
        );
        let (workspace, uri) = setup(&source);
        let text = &workspace.documents[&uri].text;
        for name in names {
            let all = locations(&uri, &source, name);
            assert_eq!(
                features::definitions(&workspace, &uri, all[2].range.start),
                all[..1],
                "{source}"
            );
            assert_eq!(
                features::references(&workspace, &uri, all[2].range.start, true),
                all[..3],
                "{source}"
            );
            if all.len() == 4 {
                assert!(features::definitions(&workspace, &uri, all[3].range.start).is_empty());
            }
        }
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
            assert_eq!(
                log.lines()
                    .filter(|line| line.starts_with("LSP-HOOK:"))
                    .collect::<Vec<_>>(),
                events
            );
            assert!(!log.contains("undefined"), "{log}");
            std::fs::read_to_string(aux).unwrap()
        };
        let before = compile(&source);
        let at = text.position(source.rfind("safe").unwrap() + 1);
        let mut edits = features::rename(&workspace, &uri, at, "fresh")
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
        assert_eq!(changed.source, source.replace("{safe}", "{fresh}"));
        assert_eq!(
            compile(&changed.source).replace("{fresh}", "{safe}"),
            before
        );
    }
}
