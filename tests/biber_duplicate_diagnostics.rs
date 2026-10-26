use lsp_types::*;
use std::{collections::BTreeMap, path::Path};
use texquill::{compilation::build_log, editing, project::workspace::Workspace, text::Text};

const CODE: &str = "duplicate-bibliography-key";
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn found(name: &str) -> String {
    format!("INFO - Found BibTeX data source '{name}'\n")
}
fn warning(key: &str, name: &str) -> String {
    format!("WARN - Duplicate entry key: '{key}' in file '{name}', skipping ...\n")
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
fn canonically_equivalent_duplicate_keys_remain_ambiguous_and_link_both_raw_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (composed, decomposed) in [("café😀", "café😀"), ("각😀", "각😀"), ("ḉ😀", "ḉ😀")]
    {
        for reported in [composed, decomposed] {
            let source = format!("% 😀\r\n@book{{{composed},title={{First}}}}\r\n@book{{{decomposed},title={{Second}}}}\r\n");
            let diagnostics = parse(
                root,
                &[("refs.data", &source)],
                &format!("{}{}", found("refs.data"), warning(reported, "refs.data")),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(item.range, span(&source, "% 😀"));
            assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
            let related = item.related_information.as_ref().unwrap();
            assert_eq!(related.len(), 2, "{diagnostics:?}");
            for (location, raw) in related.iter().zip([composed, decomposed]) {
                assert_eq!(
                    location.location,
                    Location::new(uri(root, "refs.data"), span(&source, raw))
                );
            }
        }
    }
}

#[test]
fn normalized_duplicate_keys_link_prior_canonical_sources_but_preserve_case_and_compatibility() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{café😀,title={Current}}\r\n@book{CAFÉ😀,title={Other case}}";
    let prior = "@book{café😀,title={Prior}} @book{CAFÉ😀,title={Other case}}";
    let log = format!(
        "{}{}{}{}",
        found("prior.data"),
        found("refs.data"),
        warning("café😀", "refs.data"),
        found("future.data")
    );
    let diagnostics = parse(
        root,
        &[
            ("refs.data", source),
            ("prior.data", prior),
            ("future.data", prior),
        ],
        &log,
    );
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(source, "café😀"));
    assert!(item.data.as_ref().unwrap().get("unresolvedEntry").is_none());
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(
        related[0].location,
        Location::new(uri(root, "prior.data"), span(prior, "café😀"))
    );
    let source = "@book{ﬃ, title={Ligature}} @book{ffi, title={ASCII}}";
    let diagnostics = parse(root, &[("refs.bib", source)], &warning("ﬃ", "refs.bib"));
    let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
    assert_eq!(item.range, span(source, "ﬃ"));
    assert!(item.related_information.is_none());
}

#[test]
fn normalized_duplicate_candidate_limit_is_shared_by_all_canonical_spellings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = "% 😀\r\n".to_string();
    let mut expected = Vec::new();
    for index in 0..32 {
        let key = if index % 2 == 0 {
            "café😀"
        } else {
            "café😀"
        };
        let start = source.len() + 6;
        source.push_str(&format!("@book{{{key},title={{Candidate}}}} "));
        expected.push((start, start + key.len()));
    }
    source.push_str("\r\n@book{unique😀,title={After cap}}\r\n");
    let log = format!(
        "{}{}",
        warning("café😀", "refs.bib"),
        warning("unique😀", "refs.bib")
    );
    let diagnostics = parse(root, &[("refs.bib", &source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.bib"));
    let item = items
        .iter()
        .find(|item| item.data.as_ref().unwrap()["key"] == "café😀")
        .unwrap();
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 16);
    let text = Text::new(source.clone());
    for (related, (start, end)) in related.iter().zip(expected) {
        assert_eq!(related.location.range, text.range(start, end));
    }
    let unique = items
        .iter()
        .find(|item| item.data.as_ref().unwrap()["key"] == "unique😀")
        .unwrap();
    assert_eq!(unique.range, span(&source, "unique😀"));
    assert!(unique.related_information.is_none());
}

#[test]
#[ignore = "requires biber"]
fn real_biber_canonical_duplicates_keep_both_spellings_and_clear_after_repair() {
    for keep_duplicates in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for stage in 0..3 {
            let other = if stage == 2 {
                "different😀"
            } else {
                "café😀"
            };
            let source = format!("{}% 😀\r\n@book{{café😀,title={{First}}}}\r\n@book{{{other},title={{Second}}}}\r\n", if stage == 1 { "\r\n" } else { "" });
            std::fs::write(root.join("refs.data"), &source).unwrap();
            let mut command = std::process::Command::new("biber");
            command.args(["--tool", "--output-file=processed.bib"]);
            if keep_duplicates {
                command.arg("--noskipduplicates");
            }
            let output = command.arg("refs.data").current_dir(root).output().unwrap();
            assert!(output.status.success(), "{output:?}");
            let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
            let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
            let items = items(&diagnostics, &uri(root, "refs.data"));
            if stage == 2 {
                assert!(items.is_empty(), "{log}\n{diagnostics:?}");
            } else {
                assert_eq!(items.len(), 1, "{log}\n{diagnostics:?}");
                let item = &items[0];
                assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
                let related = item.related_information.as_ref().unwrap();
                assert_eq!(related.len(), 2);
                assert_eq!(related[0].location.range, span(&source, "café😀"));
                assert_eq!(related[1].location.range, span(&source, "café😀"));
            }
            assert_eq!(
                std::fs::read_to_string(root.join("refs.data")).unwrap(),
                source
            );
        }
    }
}

#[test]
fn ambiguous_duplicate_keys_link_each_exact_candidate_without_selecting_a_declaration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{alpha😀,title={First}}\r\n@book{ALPHA😀,title={Case distinct}}\r\n@book{alpha😀,title={Second}}\r\n@book{alpha😀,title={Third}}\r\n";
    let log = format!(
        "{}{}{}",
        found("refs.data"),
        warning("alpha😀", "refs.data"),
        warning("alpha😀", "refs.data")
    );
    let diagnostics = parse(root, &[("refs.data", source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 1, "{diagnostics:?}");
    let item = &items[0];
    assert_eq!(item.range, span(source, "% 😀"));
    assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
    assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
    assert_eq!(item.data.as_ref().unwrap()["key"], "alpha😀");
    assert_eq!(item.source.as_deref(), Some("latex-build"));
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 3);
    for (related, line) in related.iter().zip([1, 3, 4]) {
        assert_eq!(related.location.uri, uri(root, "refs.data"));
        assert_eq!(
            related.location.range,
            Range::new(Position::new(line, 6), Position::new(line, 13))
        );
        assert!(related.message.contains("alpha😀"));
    }
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace
        .project
        .build_diagnostics
        .insert(uri(root, "main.tex"), diagnostics.clone());
    assert!(editing::code_actions(
        &workspace,
        &uri(root, "refs.data"),
        item.range,
        &CodeActionContext {
            diagnostics: vec![item.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None
        }
    )
    .is_empty());
}

#[test]
fn unique_current_duplicate_key_links_only_prior_confirmed_databases_and_exact_case() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{alpha😀,title={Current}}\r\n";
    let prior = "@book{ALPHA😀,title={Different}}\n@book{alpha😀,title={Earlier}}\n";
    let log = format!(
        "{}{}{}{}",
        found("prior.data"),
        found("refs.data"),
        warning("alpha😀", "refs.data"),
        found("future.data")
    );
    let diagnostics = parse(
        root,
        &[
            ("refs.data", source),
            ("prior.data", prior),
            ("future.data", prior),
        ],
        &log,
    );
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(source, "alpha😀"));
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(
        related[0].location,
        Location::new(uri(root, "prior.data"), span(prior, "alpha😀"))
    );
    assert!(item.data.as_ref().unwrap().get("unresolvedEntry").is_none());
}

#[test]
fn duplicate_key_candidates_survive_incomplete_history_only_in_the_explicit_current_file() {
    for problem in [
        "ERROR - malformed input\n",
        "INFO - Found BibTeX data source 'missing.data'\n",
        "[texquill: middle of compiler output omitted]\n",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "@book{alpha,title={First}} @book{alpha,title={Second}}";
        let log = format!(
            "{}{}{}{}",
            found("prior.data"),
            found("refs.data"),
            problem,
            warning("alpha", "refs.data")
        );
        let diagnostics = parse(
            root,
            &[
                ("refs.data", source),
                ("prior.data", "@book{alpha,title={Prior}}"),
            ],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        let related = item.related_information.as_ref().unwrap();
        assert_eq!(related.len(), 2, "{problem}: {diagnostics:?}");
        assert!(related
            .iter()
            .all(|r| r.location.uri == uri(root, "refs.data")));
    }
}

#[test]
fn native_duplicate_formats_keep_full_special_filenames_and_static_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in [
        "author's refs,part.data",
        "refs.bib': copy.data",
        "refs WARN - ERROR -.data",
        "refs.bst",
        "references",
    ] {
        for key in ["O'Connor2026", "entry.bib", "key\"(old)", "中文😀"] {
            let source = format!("% Header\n@book{{{key},title={{Current}}}}\n");
            for suffix in [", skipping ...", ""] {
                let log = format!("{}[0] Biber.pm:130> WARN - Duplicate entry key: '{key}' in file '{name}'{suffix}\n", found(name));
                let diagnostics = parse(
                    root,
                    &[(name, &source), ("refs.bib", "@book{unrelated}")],
                    &log,
                );
                let item = &items(&diagnostics, &uri(root, name))[0];
                assert_eq!(item.range, span(&source, key), "{log}");
                assert!(item.related_information.is_none());
                assert_eq!(
                    item.message,
                    format!("Biber: Duplicate entry key: '{key}' in file '{name}'{suffix}")
                );
            }
        }
    }
}

#[test]
fn duplicate_warnings_never_borrow_a_missing_or_unconfirmed_primary_source() {
    for name in [
        "missing.bib",
        "unconfirmed.data",
        " refs.bib",
        "refs.bib ",
        "\"refs.bib\"",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "@book{alpha,title={Decoy}}";
        let log = format!("{}{}", found("prior.data"), warning("alpha", name));
        let diagnostics = parse(
            root,
            &[
                ("refs.bib", source),
                ("prior.data", source),
                ("unconfirmed.data", source),
            ],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
        assert!(item.related_information.is_none());
    }
}

#[test]
fn duplicate_warning_with_missing_current_key_keeps_explicit_source_and_related_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% Header\n@book{ALPHA,title={Wrong case}}\n@string{alpha={Not an entry}}";
    let log = format!(
        "{}{}{}",
        found("prior.data"),
        found("refs.data"),
        warning("alpha", "refs.data")
    );
    let diagnostics = parse(
        root,
        &[
            ("refs.data", source),
            ("prior.data", "@book{alpha,title={Prior}}"),
        ],
        &log,
    );
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(source, "% Header"));
    assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
    assert_eq!(
        item.related_information.as_ref().unwrap()[0].location.uri,
        uri(root, "prior.data")
    );
}

#[test]
fn duplicate_related_locations_have_a_fixed_bound_and_refresh_between_logs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = format!(
        "% Header\n{}",
        "@book{alpha,title={Candidate}}\n".repeat(30)
    );
    let log = format!("{}{}", found("refs.data"), warning("alpha", "refs.data"));
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.related_information.as_ref().unwrap().len(), 16);
    let source = "% Moved\r\n\r\n@book{alpha,title={Only}}\r\n";
    let diagnostics = parse(root, &[("refs.data", source)], &log);
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(source, "alpha"));
    assert!(item.related_information.is_none());
    let diagnostics = parse(root, &[("refs.data", source)], &found("refs.data"));
    assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
}

#[test]
#[ignore = "requires biber"]
fn real_biber_duplicate_warnings_link_exact_candidates_and_keep_native_selection() {
    for name in ["refs.data", "author's refs.data"] {
        for keep_duplicates in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            for stage in 0..3 {
                let source = match stage {
                    0 => "% 😀\r\n@book{alpha😀,title={First}}\r\n@book{alpha😀,title={Second}}\r\n@book{alpha😀,title={Third}}\r\n",
                    1 => "\r\n% 😀\r\n@book{alpha😀,title={First}}\r\n@book{alpha😀,title={Second}}\r\n@book{alpha😀,title={Third}}\r\n",
                    _ => "@book{alpha😀,title={First}}\r\n@book{second😀,title={Second}}\r\n@book{third😀,title={Third}}\r\n",
                };
                std::fs::write(root.join(name), source).unwrap();
                let mut command = std::process::Command::new("biber");
                command.args(["--tool", "--output-file=processed.bib"]);
                if keep_duplicates {
                    command.arg("--noskipduplicates");
                }
                let output = command.arg(name).current_dir(root).output().unwrap();
                assert!(output.status.success(), "{output:?}");
                assert_eq!(std::fs::read_to_string(root.join(name)).unwrap(), source);
                let log = std::fs::read_to_string(root.join(format!("{name}.blg"))).unwrap();
                let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
                let entries = std::fs::read_to_string(root.join("processed.bib")).unwrap();
                assert!(entries.contains(if keep_duplicates && stage < 2 {
                    "Third"
                } else {
                    "First"
                }));
                if stage == 2 {
                    assert!(items(&diagnostics, &uri(root, name)).is_empty());
                    assert!(entries.contains("Second") && entries.contains("Third"));
                } else {
                    let items = items(&diagnostics, &uri(root, name));
                    assert_eq!(items.len(), 1, "{diagnostics:?}\n{log}");
                    let item = &items[0];
                    assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
                    assert_eq!(item.related_information.as_ref().unwrap().len(), 3);
                    assert_eq!(
                        item.related_information.as_ref().unwrap()[0]
                            .location
                            .range
                            .start
                            .line,
                        stage + 1
                    );
                    if !keep_duplicates {
                        assert!(!entries.contains("Second") && !entries.contains("Third"));
                    }
                }
            }
        }
    }
}

#[test]
fn only_complete_native_duplicate_records_receive_the_duplicate_code() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{alpha,title={Current}}";
    let message = "Duplicate entry key: 'alpha' in file 'refs.bib', skipping ...";
    for header in ["", "[0] ", "[0] Biber.pm:130> ", "Biber.pm:130> "] {
        for (level, severity) in [
            ("WARN", DiagnosticSeverity::WARNING),
            ("ERROR", DiagnosticSeverity::ERROR),
        ] {
            let diagnostics = parse(
                root,
                &[("refs.bib", source)],
                &format!("{header}{level} - {message}\n"),
            );
            assert_eq!(
                items(&diagnostics, &uri(root, "refs.bib"))[0].severity,
                Some(severity)
            );
        }
        for level in ["INFO", "DEBUG", "TRACE"] {
            assert!(parse(
                root,
                &[("refs.bib", source)],
                &format!("{header}{level} - {message}\n")
            )
            .is_empty());
        }
    }
    for line in [
        format!("WARN - Example: {message}"),
        format!(" : WARN - {message}"),
        format!("Warning--{message}"),
        "WARN - Duplicate entry key: 'alpha' in file 'refs.bib' trailing example".into(),
        "WARN - Duplicate entry key: 'alpha' in file 'refs.bib".into(),
        "WARN - Duplicate entry key: 'dynamic key' in file 'refs.bib', skipping ...".into(),
    ] {
        let diagnostics = parse(root, &[("refs.bib", source)], &format!("{line}\n"));
        assert!(
            diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{line}: {diagnostics:?}"
        );
    }
}

#[test]
fn biber_error_recovery_does_not_match_a_truncated_key_to_an_apostrophe_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% Header\n@book{O'Connor,title={First}}\n@book{O'Connor,title={Second}}\n";
    let log = format!(
        "{}{}ERROR - BibTeX subsystem: syntax error\n{}",
        found("prior.data"),
        found("refs.data"),
        warning("O", "refs.data")
    );
    let diagnostics = parse(
        root,
        &[
            ("refs.data", source),
            ("prior.data", "@book{O,title={Untrusted}}"),
        ],
        &log,
    );
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(source, "% Header"));
    assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
    assert!(item.related_information.is_none());
}

#[test]
fn unavailable_duplicate_sources_do_not_link_keys_in_another_database() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("large.bib"), " ".repeat(2 * 1024 * 1024 + 1)).unwrap();
    std::fs::write(root.join("binary.bib"), [0xff, 0xfe]).unwrap();
    for name in ["large.bib", "binary.bib", "missing.bib"] {
        let log = format!("{}{}", found("prior.data"), warning("alpha", name));
        let diagnostics = parse(
            root,
            &[("prior.data", "@book{alpha,title={Unrelated}}")],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
        assert!(item.related_information.is_none());
    }
}

#[test]
#[cfg(unix)]
fn duplicate_key_locations_deduplicate_canonical_aliases_and_exclude_a_unique_primary() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::os::unix::fs::symlink(root.join("refs.data"), root.join("alias.data")).unwrap();
    let log = format!(
        "{}{}{}",
        found("alias.data"),
        found("refs.data"),
        warning("alpha", "refs.data")
    );
    for (source, count) in [
        ("@book{alpha,title={Only}}", 0),
        ("@book{alpha,title={First}} @book{alpha,title={Second}}", 2),
    ] {
        let diagnostics = parse(root, &[("refs.data", source)], &log);
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        let related = item.related_information.as_deref().unwrap_or_default();
        assert_eq!(related.len(), count);
        assert!(related
            .iter()
            .all(|r| r.location.uri == uri(root, "refs.data")));
    }
}
#[test]
fn thousands_of_duplicate_and_case_warnings_keep_all_ranges_on_a_single_unicode_line() {
    for duplicate in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut source = String::new();
        let mut log = found("refs.data");
        let mut column = 0;
        let mut expected = BTreeMap::new();
        for index in 0..3000 {
            let key = if duplicate {
                format!("key{index}😀")
            } else {
                format!("KEY{index}😀")
            };
            let other = format!("key{index}😀");
            let mut ranges = Vec::new();
            for spelling in [&key, &other] {
                let entry = format!("@book{{{spelling},title={{😀 Title}}}} ");
                let start = column + 6;
                ranges.push(Range::new(
                    Position::new(0, start),
                    Position::new(0, start + spelling.encode_utf16().count() as u32),
                ));
                column += entry.encode_utf16().count() as u32;
                source.push_str(&entry);
            }
            if duplicate {
                log.push_str(&warning(&key, "refs.data"));
            } else {
                log.push_str(&format!("WARN - Possible typo (case mismatch) between datasource keys: '{key}' and '{other}' in file 'refs.data'\n"));
            }
            expected.insert(key, ranges);
        }
        let diagnostics = parse(root, &[("refs.data", &source)], &log);
        let items = &diagnostics[&uri(root, "refs.data")];
        assert_eq!(items.len(), 3000);
        for item in items {
            let ranges = &expected[item.data.as_ref().unwrap()["key"].as_str().unwrap()];
            let related = item.related_information.as_ref().unwrap();
            if duplicate {
                assert_eq!(
                    item.range,
                    Range::new(Position::new(0, 0), Position::new(0, column))
                );
                assert_eq!(related.len(), 2);
                assert_eq!(related[0].location.range, ranges[0]);
                assert_eq!(related[1].location.range, ranges[1]);
            } else {
                assert_eq!(item.range, ranges[0]);
                assert_eq!(related.len(), 1);
                assert_eq!(related[0].location.range, ranges[1]);
            }
        }
    }
}

#[test]
fn cached_key_positions_continue_across_crlf_and_entries_after_the_related_limit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = format!(
        "% 😀\r\n{}@book{{BETA😀,title={{Later 🧩}}}}\r\n@book{{beta😀,title={{Other}}}}\r\n",
        "@book{alpha,title={😀}}\r\n".repeat(30)
    );
    let log = format!("{}{}WARN - Possible typo (case mismatch) between datasource keys: 'BETA😀' and 'beta😀' in file 'refs.data'\n", found("refs.data"), warning("alpha", "refs.data"));
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    assert_eq!(
        items(&diagnostics, &uri(root, "refs.data"))[0]
            .related_information
            .as_ref()
            .unwrap()
            .len(),
        16
    );
    let case = diagnostics[&uri(root, "refs.data")]
        .iter()
        .find(|item| {
            item.code
                == Some(NumberOrString::String(
                    "bibliography-key-case-mismatch".into(),
                ))
        })
        .unwrap();
    assert_eq!(case.range, span(&source, "BETA😀"));
    assert_eq!(
        case.related_information.as_ref().unwrap()[0].location.range,
        span(&source, "beta😀")
    );
}

#[test]
fn cached_key_locations_do_not_restore_cross_database_trust_after_a_biber_error() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{alpha,title={First}} @book{alpha,title={Second}} @book{BETA,title={Later}}";
    let log = format!("{}{}{}ERROR - malformed input\nWARN - Possible typo (case mismatch) between datasource keys: 'BETA' and 'beta' in file 'refs.data'\n", found("prior.data"), found("refs.data"), warning("alpha", "refs.data"));
    let diagnostics = parse(
        root,
        &[
            ("refs.data", source),
            (
                "prior.data",
                "@book{alpha,title={Prior}} @book{beta,title={Other}}",
            ),
        ],
        &log,
    );
    let duplicate = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(duplicate.related_information.as_ref().unwrap().len(), 3);
    let case = diagnostics[&uri(root, "refs.data")]
        .iter()
        .find(|item| {
            item.code
                == Some(NumberOrString::String(
                    "bibliography-key-case-mismatch".into(),
                ))
        })
        .unwrap();
    assert_eq!(case.range, span(source, "BETA"));
    assert!(case.related_information.is_none());
    assert_eq!(case.data.as_ref().unwrap()["unresolvedOtherKey"], true);
}
