use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

const CODE: &str = "malformed-bibliography-name";
fn uri(root: &Path, file: &str) -> Url {
    Url::from_file_path(root.join(file)).unwrap()
}
fn found(file: &str) -> String {
    format!("INFO - Found BibTeX data source '{file}'\n")
}
fn message(key: &str, reason: &str, name: &str) -> String {
    match reason {
        "too-many-commas" => format!("Name \"{name}\" has too many commas, skipping entry '{key}'"),
        "consecutive-commas" => {
            format!("Name \"{name}\" is malformed (consecutive commas): skipping entry '{key}'")
        }
        "empty-name" => format!(
            "Name in key '{key}' is empty (probably consecutive 'and'): skipping entry '{key}'"
        ),
        _ => unreachable!(),
    }
}
fn parse(root: &Path, files: &[(&str, &str)], log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    for (file, source) in files {
        std::fs::write(root.join(file), source).unwrap();
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
    let start = source.find(marker).unwrap();
    Text::new(source.into()).range(start, start + marker.len())
}

#[test]
fn native_name_diagnostics_mark_only_the_unique_raw_key_and_preserve_reason_and_severity() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for reason in ["too-many-commas", "consecutive-commas", "empty-name"] {
        for key in ["café😀", "O'Connor.bib", "'quoted'"] {
            let source = format!(
                "% 😀\r\n@book{{{key},author={{\\\"O, Jane, Jr, Extra}},date={{2026}}}}\r\n"
            );
            let reported = key.replace('é', "é");
            let name = "Ö, Jane, Jr, Extra";
            let level = if reason == "empty-name" {
                "WARN"
            } else {
                "ERROR"
            };
            let message = message(&reported, reason, name);
            let diagnostics = parse(
                root,
                &[("refs.data", &source)],
                &format!("{}{level} - {message}\n", found("refs.data")),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(item.range, span(&source, key));
            assert_eq!(item.message, format!("Biber: {message}"));
            assert_eq!(
                item.severity,
                Some(if level == "ERROR" {
                    DiagnosticSeverity::ERROR
                } else {
                    DiagnosticSeverity::WARNING
                })
            );
            assert_eq!(item.source.as_deref(), Some("latex-build"));
            let mut data = serde_json::json!({"key":reported,"reason":reason});
            if reason != "empty-name" {
                data["name"] = name.into();
            }
            assert_eq!(item.data, Some(data));
            assert!(item.related_information.is_none());
        }
    }
}

#[test]
fn recoverable_name_errors_preserve_later_entry_diagnostics_but_unknown_errors_stop_history() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{café😀,author={Doe,, Jane}} @book{second😀,year={bad}}";
    let first = format!(
        "ERROR - {}\n",
        message("café😀", "consecutive-commas", "Doe,, Jane")
    );
    let second = format!(
        "ERROR - {}\n",
        message("second😀", "too-many-commas", "Other, Jane, Jr, Extra")
    );
    let year = "WARN - legacy year field 'bad' in entry 'second😀' is not an integer - this will probably not sort properly.\n";
    for failure in [
        "",
        "ERROR - Previous syntax failure\n",
        "ERROR - Name \"partial\" has too many commas\n",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}{first}{failure}{second}{year}", found("refs.data")),
        );
        let selected = items(&diagnostics, &uri(root, "refs.data"));
        assert_eq!(
            selected.len(),
            if failure.is_empty() { 2 } else { 1 },
            "{diagnostics:?}"
        );
        let target = uri(
            root,
            if failure.is_empty() {
                "refs.data"
            } else {
                "main.tex"
            },
        );
        let item = diagnostics[&target]
            .iter()
            .find(|item| {
                item.code
                    == Some(NumberOrString::String(
                        "invalid-bibliography-field-value".into(),
                    ))
            })
            .unwrap();
        assert_eq!(
            item.range,
            if failure.is_empty() {
                span(source, "bad")
            } else {
                Range::default()
            }
        );
        assert_eq!(
            item.data
                .as_ref()
                .unwrap()
                .get("unresolvedSource")
                .is_some(),
            !failure.is_empty()
        );
        if !failure.is_empty() {
            assert_eq!(
                items(&diagnostics, &uri(root, "main.tex"))[0]
                    .data
                    .as_ref()
                    .unwrap()["unresolvedSource"],
                true
            );
        }
    }
}

#[test]
fn name_sources_need_complete_prior_inputs_and_unique_case_sensitive_normalized_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,author={Doe,, Jane}}";
    let warning = format!(
        "ERROR - {}\n",
        message("café😀", "consecutive-commas", "Doe,, Jane")
    );
    for log in [
        warning.clone(),
        format!("{warning}{}", found("refs.data")),
        format!("{}{}{warning}", found("refs.data"), found("missing.data")),
        format!(
            "{}ERROR - Previous syntax error\n{warning}",
            found("refs.data")
        ),
        format!(
            "{}[latex-lsp: middle of compiler output omitted]\n{warning}",
            found("refs.data")
        ),
        format!("{}{}{warning}", found("refs.data"), found("duplicate.data")),
    ] {
        let diagnostics = parse(
            root,
            &[
                ("refs.data", source),
                ("duplicate.data", "@book{café😀,author={Other,, Name}}"),
            ],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.range, Range::default());
        assert_eq!(
            item.data.as_ref().unwrap()["unresolvedSource"],
            true,
            "{log}\n{diagnostics:?}"
        );
        assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
    }
    for source in ["@book{café😀,} @book{café😀,}", "@book{CAFÉ😀,}"] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}{warning}", found("refs.data")),
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "main.tex"))[0]
                .data
                .as_ref()
                .unwrap()["unresolvedSource"],
            true
        );
    }
    for bytes in [vec![0xff], vec![b' '; 2 * 1024 * 1024 + 1]] {
        std::fs::write(root.join("unreadable.data"), bytes).unwrap();
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}{}{warning}",
                found("refs.data"),
                found("unreadable.data")
            ),
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "main.tex"))[0]
                .data
                .as_ref()
                .unwrap()["unresolvedSource"],
            true
        );
    }
}

#[test]
fn name_suffixes_inside_reported_names_never_supply_keys_or_source_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{café😀,author={Original}}";
    for name in [
        "file refs.bib, Jane, Jr, Extra",
        "bad\" has too many commas, skipping entry 'decoy' in file 'refs.bib'",
        "bad\" is malformed (consecutive commas): skipping entry 'decoy'",
    ] {
        for reason in ["too-many-commas", "consecutive-commas"] {
            let diagnostics = parse(
                root,
                &[
                    ("refs.data", source),
                    ("refs.bib", "@book{decoy,author={Other}}"),
                ],
                &format!(
                    "{}{}ERROR - {}\n",
                    found("refs.data"),
                    found("refs.bib"),
                    message("café😀", reason, name)
                ),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(item.range, span(source, "café😀"));
            assert_eq!(item.data.as_ref().unwrap()["name"], name);
            assert_eq!(item.data.as_ref().unwrap()["reason"], reason);
            assert!(items(&diagnostics, &uri(root, "refs.bib")).is_empty());
        }
    }
}

#[test]
fn malformed_name_templates_and_unstructured_records_do_not_receive_name_codes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let valid = message("alpha", "consecutive-commas", "Doe,, Jane");
    for body in [
        message("", "consecutive-commas", "Doe,, Jane"),
        message("alpha beta", "consecutive-commas", "Doe,, Jane"),
        message("alpha", "consecutive-commas", ""),
        "Name in key 'alpha' is empty (probably consecutive 'and'): skipping entry 'other'".into(),
        "Name \"Doe,, Jane\" is malformed: skipping entry 'alpha'".into(),
        format!("{valid} extra"),
        format!("Example: {valid}"),
        valid.trim_end_matches('\'').into(),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", "@book{alpha,}")],
            &format!("{}ERROR - {body}\n", found("refs.data")),
        );
        assert!(
            diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{body}\n{diagnostics:?}"
        );
    }
    for record in [
        format!("INFO - {valid}"),
        format!("DEBUG - {valid}"),
        format!(" : ERROR - {valid}"),
        format!("Warning--{valid}"),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", "@book{alpha,}")],
            &format!("{}{record}\n", found("refs.data")),
        );
        assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
    }
}

#[test]
fn name_diagnostics_move_with_keys_and_do_not_guess_comma_or_and_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for source in [
        "% 😀\r\n@book{café😀,author={Doe,, Jane}}",
        "% 😀\r\n@book{\r\n café😀,author={Doe,, Jane}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}ERROR - {}\n",
                found("refs.data"),
                message("café😀", "consecutive-commas", "Doe,, Jane")
            ),
        );
        workspace.open_with_language(uri(root, "refs.data"), source.into(), 7, "bibtex");
        let item = items(&diagnostics, &uri(root, "refs.data"))[0].clone();
        assert_eq!(item.range, span(source, "café😀"));
        workspace
            .build_diagnostics
            .insert(uri(root, "main.tex"), diagnostics);
        assert!(editing::code_actions(
            &workspace,
            &uri(root, "refs.data"),
            item.range,
            &CodeActionContext {
                diagnostics: vec![item],
                only: Some(vec![CodeActionKind::QUICKFIX]),
                trigger_kind: None
            }
        )
        .is_empty());
    }
}

#[test]
fn thousands_of_mixed_name_errors_and_warnings_share_exact_utf16_key_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = String::new();
    let mut log = found("refs.data");
    let mut offset = 0;
    let mut expected = BTreeMap::new();
    for index in 0..3000 {
        let key = format!("café{index}😀");
        let reported = format!("café{index}😀");
        let entry = format!("@book{{{key},title={{😀}},author={{Doe,, Jane}}}} ");
        let range = Range::new(
            Position::new(0, offset + 6),
            Position::new(0, offset + 6 + key.encode_utf16().count() as u32),
        );
        let reason = ["too-many-commas", "consecutive-commas", "empty-name"][index % 3];
        let level = if reason == "empty-name" {
            "WARN"
        } else {
            "ERROR"
        };
        log.push_str(&format!(
            "{level} - {}\n",
            message(&reported, reason, "Doe,, Jane")
        ));
        expected.insert(reported, (range, reason));
        offset += entry.encode_utf16().count() as u32;
        source.push_str(&entry);
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 3000);
    for item in items {
        let data = item.data.as_ref().unwrap();
        let (range, reason) = expected[data["key"].as_str().unwrap()];
        assert_eq!(item.range, range);
        assert_eq!(data["reason"], reason);
        assert!(data.get("unresolvedSource").is_none());
    }
}

fn native(
    root: &Path,
    source: &str,
    config: Option<&str>,
) -> (i32, String, BTreeMap<Url, Vec<Diagnostic>>) {
    std::fs::write(root.join("refs.data"), source).unwrap();
    let mut command = std::process::Command::new("biber");
    command.args(["--tool", "--output-file=processed.bib"]);
    if let Some(config) = config {
        std::fs::write(root.join("model.conf"), config).unwrap();
        command.arg("--configfile=model.conf");
    }
    let output = command.arg("refs.data").current_dir(root).output().unwrap();
    let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("refs.data")).unwrap(),
        source
    );
    let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
    (output.status.code().unwrap(), log, diagnostics)
}

#[test]
#[ignore = "requires biber"]
fn real_biber_multiple_name_errors_keep_following_field_warnings_precise_and_clear_after_repair() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for stage in 0..3 {
        let (first, second, year) = if stage == 2 {
            ("Doe, Jane", "Other, John", "2026")
        } else {
            ("Doe, Jane, Jr, Extra", "Other,, John", "bad")
        };
        let source=format!("{}% 😀\r\n@book{{café😀,author={{{first}}},date={{2026}}}}\r\n@book{{café2😀,author={{{second}}},date={{2026}}}}\r\n@book{{café3😀,author={{Doe, Jane}},year={{{year}}}}}\r\n",if stage==0 {""} else {"\r\n"});
        let (status, log, diagnostics) = native(root, &source, None);
        assert_eq!(status, if stage == 2 { 0 } else { 2 }, "{log}");
        let items = items(&diagnostics, &uri(root, "refs.data"));
        if stage == 2 {
            assert!(items.is_empty(), "{log}\n{diagnostics:?}");
            continue;
        }
        assert_eq!(items.len(), 2, "{log}\n{diagnostics:?}");
        for (key, reported, reason) in [
            ("café😀", "café😀", "too-many-commas"),
            ("café2😀", "café2😀", "consecutive-commas"),
        ] {
            let item = items
                .iter()
                .find(|item| item.data.as_ref().unwrap()["key"] == reported)
                .unwrap();
            assert_eq!(item.range, span(&source, key));
            assert_eq!(item.data.as_ref().unwrap()["reason"], reason);
            assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR));
        }
        let year = diagnostics[&uri(root, "refs.data")]
            .iter()
            .find(|item| {
                item.code
                    == Some(NumberOrString::String(
                        "invalid-bibliography-field-value".into(),
                    ))
            })
            .unwrap();
        assert_eq!(year.range, span(&source, "bad"));
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_empty_decoded_and_custom_model_names_keep_key_only_ranges() {
    let model = r#"<config><datamodel><fields><field fieldtype="list" datatype="name">strasse</field></fields><entryfields><entrytype>book</entrytype><field>strasse</field></entryfields></datamodel></config>"#;
    for (field, value, reason, config, status) in [
        (
            "author",
            "Doe, Jane and and Smith, John",
            "empty-name",
            None,
            0,
        ),
        (
            "author",
            r#"\"O, Jane, Jr, Extra"#,
            "too-many-commas",
            None,
            2,
        ),
        (
            "Straße",
            "Other,, Author",
            "consecutive-commas",
            Some(model),
            2,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for stage in 0..3 {
            let value = if stage == 2 { "Doe, Jane" } else { value };
            let source = format!(
                "{}% 😀\r\n@book{{café😀,title={{Title}},{field}={{{value}}},date={{2026}}}}\r\n",
                if stage == 0 { "" } else { "\r\n" }
            );
            let (actual, log, diagnostics) = native(root, &source, config);
            assert_eq!(actual, if stage == 2 { 0 } else { status }, "{log}");
            let items = items(&diagnostics, &uri(root, "refs.data"));
            if stage == 2 {
                assert!(items.is_empty(), "{log}\n{diagnostics:?}");
                continue;
            }
            assert_eq!(items.len(), 1, "{log}\n{diagnostics:?}");
            assert_eq!(items[0].range, span(&source, "café😀"));
            assert_eq!(items[0].data.as_ref().unwrap()["reason"], reason);
            assert!(items[0].related_information.is_none());
            if value.starts_with("\\\"") {
                assert_eq!(
                    items[0].data.as_ref().unwrap()["name"],
                    "Ö, Jane, Jr, Extra"
                );
            }
        }
    }
}
