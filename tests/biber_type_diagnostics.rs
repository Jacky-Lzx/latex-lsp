use lsp_types::*;
use std::{collections::BTreeMap, path::Path};
use texquill::{compilation::build_log, editing, project::workspace::Workspace, text::Text};

const CODE: &str = "invalid-bibliography-entry-type";
fn uri(root: &Path, file: &str) -> Url {
    Url::from_file_path(root.join(file)).unwrap()
}
fn found(file: &str) -> String {
    format!("INFO - Found BibTeX data source '{file}'\n")
}
fn message(key: &str, file: &str, kind: &str) -> String {
    format!("Datamodel: Entry '{key}' ({file}): Invalid entry type '{kind}' - defaulting to 'misc'")
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

const TYPES: &[(&str, &str)] = &[
    ("boook", "boook"),
    ("BOOOK", "boook"),
    ("CUSTÖM", "custöm"),
    ("CUSTÖM", "custöm"),
    ("Straße", "strasse"),
    ("STRAẞE", "strasse"),
    ("ΟΣ", "οσ"),
    ("Ος", "οσ"),
    ("İTYPE", "i̇type"),
    ("ıTYPE", "ıtype"),
    ("ﬃTYPE", "ffitype"),
];

#[test]
fn native_type_names_use_full_unicode_folding_and_preserve_raw_utf16_spans() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (kind, reported) in TYPES {
        let source = format!("% 😀\r\n@book{{other,title={{Wrong entry}}}}\r\n@ \r\n{kind} % 😀\r\n( café😀,title={{Title}})\r\n");
        let message = message("café😀", "refs.data", reported);
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!("{}WARN - {message}\n", found("refs.data")),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(&source, kind), "{kind}: {diagnostics:?}");
        assert_eq!(
            item.data,
            Some(serde_json::json!({"key":"café😀","entryType":reported}))
        );
        assert_eq!(item.message, format!("Biber: {message}"));
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(item.source.as_deref(), Some("latex-build"));
        assert!(item.related_information.is_none());
    }
}

#[test]
fn type_mismatches_and_incomplete_entries_use_the_key_without_blaming_a_different_type() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (source, reported) in [
        ("@book{café😀,title={Title}}", "boook"),
        ("@ıTYPE{café😀,title={Title}}", "itype"),
        ("@Straße{café😀,title={Title}}", "strase"),
        ("@boook{café😀,title={Unclosed}", "boook"),
        ("@boook{café😀,", "boook"),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                message("café😀", "refs.data", reported)
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(source, "café😀"), "{diagnostics:?}");
        assert_eq!(item.data.as_ref().unwrap()["unresolvedType"], true);
        assert!(item.data.as_ref().unwrap().get("unresolvedEntry").is_none());
    }
}

#[test]
fn missing_or_canonically_ambiguous_type_owners_never_borrow_another_entry() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        "% Header 😀\r\n@boook{CAFÉ😀,}",
        "% Header 😀\r\n@boook{café😀,} @boook{café😀,}",
        "% Header 😀\r\n@comment{@boook{café😀,}}",
        "% Header 😀\r\n@string{café😀={Not an entry}}",
        "% Header 😀\r\n@boook{other,title={café😀}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.bib", source), ("other.bib", "@boook{café😀,}")],
            &format!(
                "{}WARN - {}\n",
                found("other.bib"),
                message("café😀", "refs.bib", "boook")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
        assert_eq!(item.range, span(source, "% Header 😀"));
        assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
        assert!(items(&diagnostics, &uri(root, "other.bib")).is_empty());
    }
}

#[test]
fn native_type_records_keep_complete_special_filenames_and_literal_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for file in [
        "author's refs,part.data",
        "refs.bib': copy.data",
        "refs): Invalid entry type 'boook' - defaulting to 'misc'.data",
        "refs WARN - ERROR -.data",
        "refs.bst",
        "references",
    ] {
        for key in ["O'Connor.bib", "key\"(old)", "'quoted'"] {
            let source = format!("% 😀\r\n@boook{{{key},title={{Title}}}}");
            let diagnostics = parse(
                root,
                &[(file, &source), ("refs.bib", "@boook{decoy,}")],
                &format!("{}WARN - {}\n", found(file), message(key, file, "boook")),
            );
            let item = &items(&diagnostics, &uri(root, file))[0];
            assert_eq!(item.range, span(&source, "boook"), "{diagnostics:?}");
            assert_eq!(
                item.data,
                Some(serde_json::json!({"key":key,"entryType":"boook"}))
            );
        }
    }
}

#[test]
fn type_records_require_complete_native_templates_and_actual_record_levels() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let message = message("alpha", "refs.bib", "boook");
    for header in ["", "[0] ", "[17] Biber.pm:130> "] {
        for (level, severity) in [
            ("WARN", DiagnosticSeverity::WARNING),
            ("ERROR", DiagnosticSeverity::ERROR),
        ] {
            let diagnostics = parse(
                root,
                &[("refs.bib", "@boook{alpha,}")],
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
                &[("refs.bib", "@boook{alpha,}")],
                &format!("{header}{level} - {message}\n")
            )
            .is_empty());
        }
    }
    for record in [
        format!(" : WARN - {message}"), format!("Warning--{message}"), format!("WARN - Example: {message}"),
        format!("WARN - {message} trailing text"), format!("WARN - {}", message.trim_end_matches('\'')),
        "WARN - Datamodel: Entry 'alpha' (refs.bib): Invalid entry type '' - defaulting to 'misc'".into(),
        "WARN - Datamodel: Entry 'alpha' (refs.bib): Invalid entry type 'bad type' - defaulting to 'misc'".into(),
        "WARN - Datamodel: Entry 'alpha' (refs.bib): Invalid entry type 'boook' - defaulting to 'book'".into(),
    ] {
        let diagnostics = parse(root, &[("refs.bib", "@boook{alpha,}")], &format!("{record}\n"));
        assert!(diagnostics.values().flatten().all(|item| item.code != Some(NumberOrString::String(CODE.into()))), "{record}: {diagnostics:?}");
    }
}

#[test]
fn missing_unconfirmed_and_unreadable_type_sources_preserve_the_coded_root_warning() {
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
                ("refs.bib", "@boook{alpha,}"),
                ("unconfirmed.data", "@boook{alpha,}"),
            ],
            &format!("WARN - {}\n", message("alpha", file, "boook")),
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.range, Range::default());
        assert_eq!(
            item.data.as_ref().unwrap()["unresolvedSource"],
            true,
            "{file}"
        );
        assert!(items(&diagnostics, &uri(root, "refs.bib")).is_empty());
    }
}

#[test]
fn type_diagnostics_refresh_raw_name_ranges_without_an_unverified_misc_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let log = format!(
        "{}WARN - {}\n",
        found("refs.data"),
        message("café😀", "refs.data", "strasse")
    );
    for prefix in ["% 😀\r\n", "% Moved 😀\r\n\r\n"] {
        let source = format!("{prefix}@Straße{{café😀,title={{Title}}}}");
        let diagnostics = parse(root, &[("refs.data", &source)], &log);
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(&source, "Straße"));
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
    assert!(parse(
        root,
        &[("refs.data", "@book{café😀,title={Title}}")],
        &found("refs.data")
    )
    .is_empty());
}

#[test]
#[cfg(unix)]
fn type_diagnostics_keep_canonical_source_alias_roles() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@Straße{café😀,title={Title}}";
    std::fs::write(root.join("payload.tex"), source).unwrap();
    std::os::unix::fs::symlink(root.join("payload.tex"), root.join("refs.data")).unwrap();
    let diagnostics = parse(
        root,
        &[],
        &format!(
            "{}WARN - {}\n",
            found("refs.data"),
            message("café😀", "payload.tex", "strasse")
        ),
    );
    assert_eq!(
        items(&diagnostics, &uri(root, "payload.tex"))[0].range,
        span(source, "Straße")
    );
}

#[test]
fn thousands_of_type_and_field_records_keep_original_ranges_on_one_unicode_line() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = String::new();
    let mut log = found("refs.data");
    let mut expected = BTreeMap::new();
    let mut column = 0;
    for index in 0..3000 {
        let key = format!("café{index}😀");
        let reported = format!("café{index}😀");
        let kind = if index % 2 == 0 { "Straße" } else { "ﬃTYPE" };
        let folded = if index % 2 == 0 { "strasse" } else { "ffitype" };
        let entry = format!("@{kind}{{{key},wrong={{😀 Bad}}}} ");
        let kind_start = column + 1;
        let field_start = column
            + 1
            + kind.encode_utf16().count() as u32
            + 1
            + key.encode_utf16().count() as u32
            + 1;
        expected.insert(
            reported.clone(),
            (
                Range::new(
                    Position::new(0, kind_start),
                    Position::new(0, kind_start + kind.encode_utf16().count() as u32),
                ),
                Range::new(
                    Position::new(0, field_start),
                    Position::new(0, field_start + 5),
                ),
            ),
        );
        column += entry.encode_utf16().count() as u32;
        source.push_str(&entry);
        log.push_str(&format!("WARN - {}\nWARN - Datamodel: Entry '{reported}' (refs.data): Field 'wrong' invalid in data model - ignoring\n", message(&reported, "refs.data", folded)));
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let all = &diagnostics[&uri(root, "refs.data")];
    assert_eq!(all.len(), 6000);
    for item in all {
        let (kind, field) = expected[item.data.as_ref().unwrap()["key"].as_str().unwrap()];
        assert_eq!(
            item.range,
            if item.code == Some(NumberOrString::String(CODE.into())) {
                kind
            } else {
                field
            }
        );
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_unicode_type_warnings_move_and_clear_after_the_raw_type_is_repaired() {
    for (kind, reported) in TYPES {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let file = "author's refs,part.data";
        for stage in 0..3 {
            let selected = if stage == 2 { "book" } else { kind };
            let source = format!("{}% 😀\r\n@{selected}{{café😀,title={{Title}},author={{Doe, Jane}},date={{2026}}}}\r\n", if stage == 1 { "\r\n" } else { "" });
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
                assert_eq!(items.len(), 1, "{log}\n{diagnostics:?}");
                assert_eq!(items[0].range, span(&source, kind));
                assert_eq!(
                    items[0].message,
                    format!("Biber: {}", message("café😀", file, reported))
                );
                assert_eq!(
                    items[0].data,
                    Some(serde_json::json!({"key":"café😀","entryType":reported}))
                );
            }
            assert_eq!(std::fs::read_to_string(root.join(file)).unwrap(), source);
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_mapped_invalid_types_do_not_mark_an_originally_valid_raw_type() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{café😀,title={Title},author={Doe, Jane},date={2026}}\r\n";
    std::fs::write(root.join("refs.data"), source).unwrap();
    std::fs::write(root.join("mapping.conf"), r#"<config><sourcemap><maps datatype="bibtex"><map><map_step map_type_source="book" map_type_target="boook"/></map></maps></sourcemap></config>"#).unwrap();
    let output = std::process::Command::new("biber")
        .args([
            "--tool",
            "--configfile=mapping.conf",
            "--validate-datamodel",
            "--output-file=processed.bib",
            "refs.data",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
    let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 1, "{log}\n{diagnostics:?}");
    assert_eq!(items[0].range, span(source, "café😀"));
    assert_eq!(items[0].data.as_ref().unwrap()["unresolvedType"], true);
    assert_eq!(
        std::fs::read_to_string(root.join("refs.data")).unwrap(),
        source
    );
}
