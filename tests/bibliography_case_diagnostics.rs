use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const CODE: &str = "bibliography-key-case-mismatch";

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn message(key: &str, other: &str, file: &str) -> String {
    format!("Possible typo (case mismatch) between datasource keys: '{key}' and '{other}' in file '{file}'")
}

fn found(file: &str) -> String {
    format!("INFO - Found BibTeX data source '{file}'\n")
}

fn warning(key: &str, other: &str, file: &str) -> String {
    format!("WARN - {}\n", message(key, other, file))
}

fn parse(root: &Path, inputs: &[(&str, &str)], log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    for (name, source) in inputs {
        std::fs::write(root.join(name), source).unwrap();
    }
    build_log::parse_bibliography(&root.join("main.tex"), log)
}

fn items(diagnostics: &BTreeMap<Url, Vec<Diagnostic>>, target: &Url) -> Vec<Diagnostic> {
    diagnostics
        .get(target)
        .into_iter()
        .flatten()
        .filter(|item| item.code == Some(NumberOrString::String(CODE.into())))
        .cloned()
        .collect()
}

fn span(source: &str, marker: &str) -> Range {
    let at = source.find(marker).unwrap();
    Text::new(source.into()).range(at, at + marker.len())
}

#[test]
fn biber_case_keys_normalize_both_spelling_ranges_across_prior_databases() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for earlier in ["café😀", "café😀"] {
        for current in ["CAFÉ😀", "CAFÉ😀"] {
            let source = format!("% 😀\r\n@book{{{current},title={{Current}}}}\r\n@book{{Café😀,title={{Distinct case}}}}");
            let prior = format!("% Parent 😀\r\n@book{{{earlier},title={{Prior}}}}");
            let log = format!(
                "{}{}{}{}",
                found("prior.data"),
                found("refs.data"),
                warning("CAFÉ😀", "café😀", "refs.data"),
                found("future.data")
            );
            let diagnostics = parse(
                root,
                &[
                    ("refs.data", &source),
                    ("prior.data", &prior),
                    ("future.data", &prior),
                ],
                &log,
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(item.range, span(&source, current));
            assert_eq!(
                item.data,
                Some(serde_json::json!({"key":"CAFÉ😀","otherKey":"café😀"}))
            );
            let related = item.related_information.as_ref().unwrap();
            assert_eq!(related.len(), 1);
            assert_eq!(
                related[0].location,
                Location::new(uri(root, "prior.data"), span(&prior, earlier))
            );
        }
    }
}

#[test]
fn biber_case_primary_normalization_collisions_remain_ambiguous() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{CAFÉ😀,title={First}}\r\n@book{CAFÉ😀,title={Second}}\r\n@book{café😀,title={Other}}";
    let diagnostics = parse(
        root,
        &[("refs.bib", source)],
        &warning("CAFÉ😀", "café😀", "refs.bib"),
    );
    let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
    assert_eq!(item.range, span(source, "% 😀"));
    assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].location.range, span(source, "café😀"));
    let source = "@book{CAFÉ😀,title={First}} @book{café😀,title={Other}} @book{café😀,title={Other duplicate}}";
    let diagnostics = parse(
        root,
        &[("refs.bib", source)],
        &warning("CAFÉ😀", "café😀", "refs.bib"),
    );
    let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
    assert_eq!(item.range, span(source, "CAFÉ😀"));
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 2);
    assert_eq!(related[0].location.range, span(source, "café😀"));
    assert_eq!(related[1].location.range, span(source, "café😀"));
}

#[test]
#[ignore = "requires biber"]
fn real_biber_case_warnings_normalize_unicode_spelling_move_and_clear_after_repair() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for stage in 0..3 {
        let key = if stage == 2 {
            "different😀"
        } else {
            "CAFÉ😀"
        };
        let source = format!(
            "{}% 😀\r\n@book{{café😀,title={{First}}}}\r\n@book{{{key},title={{Second}}}}\r\n",
            if stage == 1 { "\r\n" } else { "" }
        );
        std::fs::write(root.join("refs.data"), &source).unwrap();
        let output = std::process::Command::new("biber")
            .args(["--tool", "--output-file=processed.bib", "refs.data"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
        let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let items = items(&diagnostics, &uri(root, "refs.data"));
        if stage == 2 {
            assert!(items.is_empty(), "{log}\n{diagnostics:?}");
        } else {
            assert_eq!(items.len(), 1, "{log}\n{diagnostics:?}");
            assert_eq!(items[0].range, span(&source, "CAFÉ😀"));
            assert_eq!(
                items[0].related_information.as_ref().unwrap()[0]
                    .location
                    .range,
                span(&source, "café😀")
            );
            assert_eq!(
                items[0].data,
                Some(serde_json::json!({"key":"CAFÉ😀","otherKey":"café😀"}))
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.join("refs.data")).unwrap(),
            source
        );
    }
}

#[test]
fn biber_case_warnings_mark_exact_keys_and_link_other_spelling_with_utf16_and_crlf() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (key, other) in [
        ("ALPHA😀", "alpha😀"),
        ("ΚΛΕΙΔΊ", "κλειδί"),
        ("O'CONNOR\"", "O'Connor\""),
    ] {
        let source = format!("% 😀\r\n@book{{{other},title={{First}}}}\r\n@ custom_type({key},\r\n title={{Second}})\r\n");
        let log = format!("{}{}", found("refs.data"), warning(key, other, "refs.data"));
        let diagnostics = parse(root, &[("refs.data", &source)], &log);
        let items = items(&diagnostics, &uri(root, "refs.data"));
        assert_eq!(items.len(), 1, "{diagnostics:?}");
        let item = &items[0];
        assert_eq!(item.range, span(&source, key));
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(item.source.as_deref(), Some("latex-build"));
        assert_eq!(
            item.message,
            format!("Biber: {}", message(key, other, "refs.data"))
        );
        assert_eq!(
            item.data,
            Some(serde_json::json!({"key":key,"otherKey":other}))
        );
        let related = item.related_information.as_ref().unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(
            related[0].location,
            Location::new(uri(root, "refs.data"), span(&source, other))
        );
        assert!(related[0].message.contains(other));
    }
}

#[test]
fn first_key_is_never_borrowed_from_another_database_or_case_folded() {
    for source in [
        "% Header\n@book{alpha,title={Only other spelling}}",
        "% Header\n@book{ALPHA,title={First}} @book{ALPHA,title={Duplicate}}",
        "% Header\n@book{unrelated,title={ALPHA}}",
        "% Header\n@comment{@book{ALPHA,title={Hidden}}}",
        "% Header\n@string{ALPHA={Not an entry}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let log = format!(
            "{}{}{}",
            found("other.bib"),
            found("refs.bib"),
            warning("ALPHA", "alpha", "refs.bib")
        );
        let diagnostics = parse(
            root,
            &[
                ("refs.bib", source),
                ("other.bib", "@book{ALPHA,title={Wrong source}}"),
            ],
            &log,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
        assert_eq!(
            item.range,
            Range::new(Position::new(0, 0), Position::new(0, 8))
        );
        assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
    }
}

#[test]
fn other_spelling_may_have_multiple_exact_locations_but_never_case_folded_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{alpha,title={First}}\n@book{ALPHA,title={Current}}\n@book{alpha,title={Duplicate}}\n@book{Alpha,title={Different spelling}}\n";
    let diagnostics = parse(
        root,
        &[("refs.bib", source)],
        &warning("ALPHA", "alpha", "refs.bib"),
    );
    let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
    assert_eq!(item.range, span(source, "ALPHA"));
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 2);
    assert_eq!(related[0].location.range.start.line, 0);
    assert_eq!(related[1].location.range.start.line, 2);
    let diagnostics = parse(
        root,
        &[(
            "refs.bib",
            "@book{ALPHA,title={Current}} @book{Alpha,title={Different}}",
        )],
        &warning("ALPHA", "alpha", "refs.bib"),
    );
    let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
    assert!(item.related_information.is_none());
    assert_eq!(item.data.as_ref().unwrap()["unresolvedOtherKey"], true);
}

#[test]
fn related_spelling_can_come_from_a_previously_read_database_but_not_a_future_one() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let earlier = "% Header\n@book{alpha,title={First}}";
    let current = "% 😀\r\n@book{ALPHA,title={Current}}";
    let inputs = [
        ("earlier.data", earlier),
        ("refs.data", current),
        ("future.data", "@book{alpha,title={Future}}"),
    ];
    let log = format!(
        "{}{}{}{}",
        found("earlier.data"),
        found("refs.data"),
        warning("ALPHA", "alpha", "refs.data"),
        found("future.data")
    );
    let diagnostics = parse(root, &inputs, &log);
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(current, "ALPHA"));
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(
        related[0].location,
        Location::new(uri(root, "earlier.data"), span(earlier, "alpha"))
    );
}

#[test]
fn incomplete_biber_histories_keep_current_key_ranges_without_guessing_other_databases() {
    for prefix in [
        "INFO - Found BibTeX data source 'missing.data'\n",
        "ERROR - Prior syntax error\n",
        "[latex-lsp: middle of compiler output omitted]\n",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let current = "@book{ALPHA,title={Current}}";
        let log = format!(
            "{}{}{}{}",
            found("earlier.data"),
            prefix,
            found("refs.data"),
            warning("ALPHA", "alpha", "refs.data")
        );
        let diagnostics = parse(
            root,
            &[
                ("earlier.data", "@book{alpha,title={Other}}"),
                ("refs.data", current),
            ],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(current, "ALPHA"));
        assert!(item.related_information.is_none());
        assert_eq!(item.data.as_ref().unwrap()["unresolvedOtherKey"], true);
    }
}

#[test]
fn missing_unreadable_or_unconfirmed_sources_never_borrow_prefix_files_or_other_entries() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("invalid.bib"), [0xff, 0xfe]).unwrap();
    std::fs::write(root.join("oversized.bib"), " ".repeat(2 * 1024 * 1024 + 1)).unwrap();
    std::fs::create_dir(root.join("directory.bib")).unwrap();
    for file in [
        "missing.bib",
        "invalid.bib",
        "oversized.bib",
        "directory.bib",
        "refs.bib.backup",
        "unconfirmed.data",
    ] {
        let log = format!("{}{}", found("refs.bib"), warning("ALPHA", "alpha", file));
        let diagnostics = parse(
            root,
            &[
                (
                    "refs.bib",
                    "@book{ALPHA,title={First}} @book{alpha,title={Second}}",
                ),
                ("unconfirmed.data", "@book{ALPHA} @book{alpha}"),
            ],
            &log,
        );
        assert!(
            items(&diagnostics, &uri(root, "refs.bib")).is_empty(),
            "{file}: {diagnostics:?}"
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
        assert!(item.related_information.is_none());
    }
}

#[test]
fn complete_native_file_names_keep_embedded_quotes_punctuation_and_log_looking_text() {
    for name in [
        "author's refs,part.data",
        "refs.bib': copy.data",
        "refs line 700 ERROR - entry 'fake'.tex",
        "refs in file other.bib.data",
        "references",
        "refs.bst",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "% 😀\r\n@book{alpha,title={First}} @book{ALPHA,title={Current}}";
        let log = format!("{}{}", found(name), warning("ALPHA", "alpha", name));
        let diagnostics = parse(
            root,
            &[
                (name, source),
                ("refs.bib", "@book{ALPHA,title={Decoy}}"),
                ("other.bib", "@book{alpha,title={Decoy}}"),
            ],
            &log,
        );
        assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:?}");
        let item = &items(&diagnostics, &uri(root, name))[0];
        assert_eq!(item.range, span(source, "ALPHA"));
        assert_eq!(
            item.related_information.as_ref().unwrap()[0].location.uri,
            uri(root, name)
        );
    }
}

#[test]
fn filename_edges_that_the_resolver_would_trim_never_select_a_different_file() {
    for name in [" refs.bib", "refs.bib ", "\"refs.bib\""] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let log = format!("{}{}", found(name), warning("ALPHA", "alpha", name));
        let diagnostics = parse(
            root,
            &[(
                "refs.bib",
                "@book{ALPHA,title={Decoy}} @book{alpha,title={Other}}",
            )],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
    }
}

#[test]
fn biber_record_levels_and_native_boundaries_control_case_warning_recognition() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{alpha,title={First}} @book{ALPHA,title={Current}}";
    for prefix in ["", "[17] ", "[0] Utils.pm:395> ", "Utils.pm:395> "] {
        for (level, severity) in [
            ("WARN", DiagnosticSeverity::WARNING),
            ("ERROR", DiagnosticSeverity::ERROR),
        ] {
            let log = format!(
                "{prefix}{level} - {}\n",
                message("ALPHA", "alpha", "refs.bib")
            );
            let diagnostics = parse(root, &[("refs.bib", source)], &log);
            assert_eq!(
                items(&diagnostics, &uri(root, "refs.bib"))[0].severity,
                Some(severity)
            );
        }
        for level in ["INFO", "DEBUG", "TRACE"] {
            let log = format!(
                "{prefix}{level} - {}\n",
                message("ALPHA", "alpha", "refs.bib")
            );
            assert!(parse(root, &[("refs.bib", source)], &log).is_empty());
        }
    }
    for text in [
        format!("Warning--{}", message("ALPHA", "alpha", "refs.bib")),
        format!(" : WARN - {}", message("ALPHA", "alpha", "refs.bib")),
        format!("WARN - Example: {}", message("ALPHA", "alpha", "refs.bib")),
        "WARN - Possible typo (case mismatch) between citation and datasource keys: 'ALPHA' and 'alpha' in file 'refs.bib'".into(),
        "WARN - Possible typo (case mismatch) between datasource keys: 'ALPHA' and 'alpha' in file 'refs.bib".into(),
        format!("WARN - {}", message("alpha", "alpha", "refs.bib")),
        format!("WARN - {}", message("ALPHA extra", "alpha", "refs.bib")),
    ] {
        let diagnostics = parse(root, &[("refs.bib", source)], &format!("{text}\n"));
        assert!(diagnostics.values().flatten().all(|item| item.code != Some(NumberOrString::String(CODE.into()))), "{text}: {diagnostics:?}");
    }
}

#[test]
fn related_locations_are_bounded_and_case_warnings_do_not_offer_unverified_merges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = format!(
        "@book{{ALPHA,title={{Current}}}}\n{}",
        "@book{alpha,title={Other}}\n".repeat(30)
    );
    let diagnostics = parse(
        root,
        &[("refs.bib", &source)],
        &warning("ALPHA", "alpha", "refs.bib"),
    );
    let item = items(&diagnostics, &uri(root, "refs.bib"))[0].clone();
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 16);
    assert_eq!(related[0].location.range.start.line, 1);
    assert_eq!(related[15].location.range.start.line, 16);
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace
        .build_diagnostics
        .insert(uri(root, "main.tex"), diagnostics);
    assert!(editing::code_actions(
        &workspace,
        &uri(root, "refs.bib"),
        item.range,
        &CodeActionContext {
            diagnostics: vec![item],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None
        }
    )
    .is_empty());
}

#[cfg(unix)]
#[test]
fn database_aliases_preserve_physical_keys_and_deduplicate_related_paths() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{alpha,title={First}} @book{ALPHA,title={Current}}";
    std::fs::write(root.join("physical.tex"), source).unwrap();
    symlink("physical.tex", root.join("alias.data")).unwrap();
    let log = format!(
        "{}{}{}",
        found("alias.data"),
        found("physical.tex"),
        warning("ALPHA", "alpha", "physical.tex")
    );
    let diagnostics = parse(root, &[], &log);
    let item = &items(&diagnostics, &uri(root, "physical.tex"))[0];
    assert_eq!(item.range, span(source, "ALPHA"));
    assert_eq!(item.related_information.as_ref().unwrap().len(), 1);
    assert_eq!(
        item.related_information.as_ref().unwrap()[0].location.uri,
        uri(root, "physical.tex")
    );
}

#[test]
#[ignore = "requires Biber"]
fn real_biber_case_warnings_keep_both_entries_refresh_positions_and_clear_after_repair() {
    for name in ["refs.data", "author's refs,part.data"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for (prefix, key) in [
            ("% 😀\r\n", "ALPHA😀"),
            ("% Moved 😀\r\n\r\n", "ALPHA😀"),
            ("% Repaired\r\n", "different😀"),
        ] {
            let source = format!(
                "{prefix}@book{{alpha😀,title={{First}}}}\r\n@book{{{key},title={{Second}}}}\r\n"
            );
            parse(root, &[(name, &source)], "");
            let output = std::process::Command::new("biber")
                .args(["--tool", "--output-file=processed.bib", name])
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert_eq!(std::fs::read(root.join(name)).unwrap(), source.as_bytes());
            let log = std::fs::read_to_string(root.join(format!("{name}.blg"))).unwrap();
            let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
            let entries = std::fs::read_to_string(root.join("processed.bib")).unwrap();
            assert!(entries.contains("First") && entries.contains("Second"));
            if key == "ALPHA😀" {
                let item = &items(&diagnostics, &uri(root, name))[0];
                assert_eq!(item.range, span(&source, key));
                assert_eq!(
                    item.related_information.as_ref().unwrap()[0].location.range,
                    span(&source, "alpha😀")
                );
                assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            } else {
                assert!(diagnostics.is_empty(), "{diagnostics:?}\n{log}");
            }
        }
    }
}
