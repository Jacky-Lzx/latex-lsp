use latex_lsp::{compilation::build_log, editing, project::workspace::Workspace, text::Text};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const CODE: &str = "missing-bibliography-field";
fn uri(root: &Path, file: &str) -> Url {
    Url::from_file_path(root.join(file)).unwrap()
}
fn found(file: &str) -> String {
    format!("INFO - Found BibTeX data source '{file}'\n")
}
fn message(key: &str, file: &str, requirement: &str) -> String {
    format!("Datamodel: Entry '{key}' ({file}): Missing mandatory field {requirement}")
}
fn parse(root: &Path, inputs: &[(&str, &str)], log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    for (file, source) in inputs {
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
    let at = source.find(marker).unwrap();
    Text::new(source.into()).range(at, at + marker.len())
}

#[test]
fn required_single_fields_and_one_of_groups_mark_only_the_reported_entry_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source =
        "% 😀\r\n@book{other,author={Elsewhere}}\r\n@book{café😀,note={author and year}}\r\n";
    for (requirement, expected) in [
        (
            "'author'",
            serde_json::json!({"key":"café😀","field":"author"}),
        ),
        (
            "'custom:field'",
            serde_json::json!({"key":"café😀","field":"custom:field"}),
        ),
        (
            "- one of 'date, year' must be defined",
            serde_json::json!({"key":"café😀","fields":["date","year"]}),
        ),
        (
            "- one of 'author, editor, translator' must be defined",
            serde_json::json!({"key":"café😀","fields":["author","editor","translator"]}),
        ),
    ] {
        let message = message("café😀", "refs.data", requirement);
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}WARN - {message}\n", found("refs.data")),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(source, "café😀"));
        assert_eq!(item.data, Some(expected));
        assert_eq!(item.message, format!("Biber: {message}"));
        assert_eq!(item.source.as_deref(), Some("latex-build"));
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert!(item.related_information.is_none());
    }
}

#[test]
fn required_field_messages_keep_complete_special_filenames_and_literal_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for file in [
        "author's refs,part.data",
        "refs.bib': copy.data",
        "refs): Missing mandatory field 'author'.data",
        "refs WARN - ERROR -.data",
        "references",
        "refs.bst",
    ] {
        for key in ["O'Connor.bib", "key\"(old)", "'quoted'"] {
            let source = format!("% 😀\r\n@book{{{key},title={{Title}}}}");
            let diagnostics = parse(
                root,
                &[(file, &source), ("refs.bib", "@book{decoy,}")],
                &format!("{}WARN - {}\n", found(file), message(key, file, "'author'")),
            );
            let item = &items(&diagnostics, &uri(root, file))[0];
            assert_eq!(item.range, span(&source, key), "{diagnostics:?}");
            assert_eq!(
                item.data,
                Some(serde_json::json!({"key":key,"field":"author"}))
            );
        }
    }
}

#[test]
fn required_fields_never_borrow_missing_or_ambiguous_entries_from_other_databases() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        "% Header 😀\r\n@book{CAFÉ😀,title={Case distinct}}",
        "% Header 😀\r\n@book{café😀,} @book{café😀,}",
        "% Header 😀\r\n@comment{@book{café😀,}}",
        "% Header 😀\r\n@string{café😀={Not an entry}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.bib", source), ("other.bib", "@book{café😀,}")],
            &format!(
                "{}WARN - {}\n",
                found("other.bib"),
                message("café😀", "refs.bib", "'author'")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
        assert_eq!(item.range, span(source, "% Header 😀"));
        assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
        assert!(items(&diagnostics, &uri(root, "other.bib")).is_empty());
    }
}

#[test]
fn required_field_warnings_with_missing_unconfirmed_or_unreadable_sources_keep_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("large.bib"), " ".repeat(2 * 1024 * 1024 + 1)).unwrap();
    std::fs::write(root.join("binary.bib"), [0xff]).unwrap();
    for file in [
        "missing.bib",
        "unconfirmed.data",
        "large.bib",
        "binary.bib",
        " refs.bib",
        "refs.bib ",
        "\"refs.bib\"",
    ] {
        let diagnostics = parse(
            root,
            &[
                ("refs.bib", "@book{alpha,}"),
                ("unconfirmed.data", "@book{alpha,}"),
            ],
            &format!("WARN - {}\n", message("alpha", file, "'author'")),
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(
            item.data.as_ref().unwrap()["unresolvedSource"],
            true,
            "{file}"
        );
        assert_eq!(item.range, Range::default());
        assert!(items(&diagnostics, &uri(root, "refs.bib")).is_empty());
    }
}

#[test]
fn only_complete_native_required_field_templates_receive_the_code() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for requirement in ["'author'", "- one of 'date, year' must be defined"] {
        let message = message("alpha", "refs.bib", requirement);
        for prefix in ["", "[0] ", "[17] Biber.pm:130> "] {
            for (level, severity) in [
                ("WARN", DiagnosticSeverity::WARNING),
                ("ERROR", DiagnosticSeverity::ERROR),
            ] {
                let diagnostics = parse(
                    root,
                    &[("refs.bib", "@book{alpha,}")],
                    &format!("{prefix}{level} - {message}\n"),
                );
                assert_eq!(
                    items(&diagnostics, &uri(root, "refs.bib"))[0].severity,
                    Some(severity)
                );
            }
            for level in ["INFO", "DEBUG", "TRACE"] {
                assert!(parse(
                    root,
                    &[("refs.bib", "@book{alpha,}")],
                    &format!("{prefix}{level} - {message}\n")
                )
                .is_empty());
            }
        }
        for record in [
            format!(" : WARN - {message}"),
            format!("Warning--{message}"),
            format!("WARN - Example: {message}"),
            format!("WARN - {message} trailing text"),
        ] {
            let diagnostics = parse(
                root,
                &[("refs.bib", "@book{alpha,}")],
                &format!("{record}\n"),
            );
            assert!(
                diagnostics
                    .values()
                    .flatten()
                    .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
                "{record}"
            );
        }
    }
    for requirement in [
        "'author",
        "''",
        "'author editor'",
        "- one of '' must be defined",
        "- one of 'date, ' must be defined",
        "- one of 'date, year'",
        "- one of 'date,year' must be defined",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.bib", "@book{alpha,}")],
            &format!("WARN - {}\n", message("alpha", "refs.bib", requirement)),
        );
        assert!(
            diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{requirement}: {diagnostics:?}"
        );
    }
}

#[test]
fn required_field_key_cache_refreshes_without_inventing_an_insertion_quickfix() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let log = format!(
        "{}WARN - {}\n",
        found("refs.data"),
        message(
            "café😀",
            "refs.data",
            "- one of 'date, year' must be defined"
        )
    );
    for prefix in ["% 😀\r\n", "% Moved 😀\r\n\r\n"] {
        let source = format!("{prefix}@book{{café😀,title={{Title}}}}");
        let diagnostics = parse(root, &[("refs.data", &source)], &log);
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(&source, "café😀"));
        let mut workspace = Workspace {
            client: latex_lsp::project::state::ClientCapabilities {
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
    assert!(parse(
        root,
        &[("refs.data", "@book{café😀,date={2026}}")],
        &found("refs.data")
    )
    .is_empty());
}

#[test]
#[cfg(unix)]
fn required_field_source_aliases_keep_canonical_database_roles() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{café😀,}";
    std::fs::write(root.join("payload.tex"), source).unwrap();
    std::os::unix::fs::symlink(root.join("payload.tex"), root.join("refs.data")).unwrap();
    let diagnostics = parse(
        root,
        &[],
        &format!(
            "{}WARN - {}\n",
            found("refs.data"),
            message("café😀", "payload.tex", "'author'")
        ),
    );
    assert_eq!(
        items(&diagnostics, &uri(root, "payload.tex"))[0].range,
        span(source, "café😀")
    );
}

#[test]
fn thousands_of_required_field_warnings_reuse_exact_normalized_key_positions_on_one_line() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = String::new();
    let mut log = found("refs.data");
    let mut expected = BTreeMap::new();
    let mut column = 0;
    for index in 0..1500 {
        let key = format!("café{index}😀");
        let reported = format!("café{index}😀");
        let entry = format!("@book{{{key},note={{😀}}}} ");
        expected.insert(
            reported.clone(),
            Range::new(
                Position::new(0, column + 6),
                Position::new(0, column + 6 + key.encode_utf16().count() as u32),
            ),
        );
        column += entry.encode_utf16().count() as u32;
        source.push_str(&entry);
        for requirement in ["'author'", "- one of 'date, year' must be defined"] {
            log.push_str(&format!(
                "WARN - {}\n",
                message(&reported, "refs.data", requirement)
            ));
        }
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 3000);
    for item in items {
        assert_eq!(
            item.range,
            expected[item.data.as_ref().unwrap()["key"].as_str().unwrap()]
        );
        assert!(item.data.as_ref().unwrap().get("unresolvedEntry").is_none());
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_required_fields_and_groups_move_and_clear_after_values_are_supplied() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let file = "author's refs,part.data";
    for stage in 0..3 {
        let fields = if stage == 2 {
            "title={Title},author={Doe, Jane},date={2026}"
        } else {
            ""
        };
        let source = format!(
            "{}% 😀\r\n@book{{café😀,{fields}}}\r\n",
            if stage == 1 { "\r\n" } else { "" }
        );
        std::fs::write(root.join(file), &source).unwrap();
        let output = std::process::Command::new("biber")
            .args([
                "--tool",
                "--validate-datamodel",
                "--output-file=processed.bib",
                file,
            ])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let log = std::fs::read_to_string(root.join(format!("{file}.blg"))).unwrap();
        let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let items = items(&diagnostics, &uri(root, file));
        if stage == 2 {
            assert!(items.is_empty(), "{log}\n{diagnostics:?}");
        } else {
            assert_eq!(items.len(), 3, "{log}\n{diagnostics:?}");
            for item in &items {
                assert_eq!(item.range, span(&source, "café😀"));
            }
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item.data.as_ref().unwrap()["field"] == "author"
                        || item.data.as_ref().unwrap()["field"] == "title")
                    .count(),
                2
            );
            assert_eq!(
                items
                    .iter()
                    .find(|item| item.data.as_ref().unwrap().get("fields").is_some())
                    .unwrap()
                    .data
                    .as_ref()
                    .unwrap()["fields"],
                serde_json::json!(["date", "year"])
            );
        }
        assert_eq!(std::fs::read_to_string(root.join(file)).unwrap(), source);
    }
}
