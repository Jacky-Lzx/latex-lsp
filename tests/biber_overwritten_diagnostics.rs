use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const CODE: &str = "overwritten-bibliography-field";
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn found(name: &str) -> String {
    format!("INFO - Found BibTeX data source '{name}'\n")
}
fn warning(key: &str, field: &str) -> String {
    format!("Overwriting field '{field}' with {field} value from field 'date' for entry '{key}'")
}
fn parse(root: &Path, files: &[(&str, &str)], log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    for (file, source) in files {
        std::fs::write(root.join(file), source).unwrap();
    }
    build_log::parse_bibliography(&root.join("main.tex"), log)
}
fn items(diagnostics: &BTreeMap<Url, Vec<Diagnostic>>, uri: &Url) -> Vec<Diagnostic> {
    diagnostics
        .get(uri)
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
fn overwritten_values_and_the_replacing_date_keep_raw_ranges_and_native_messages() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for field in ["year", "month"] {
        for (rhs, marker) in [
            ("{1999😀}", "1999😀"),
            ("\"1999😀\"", "1999😀"),
            ("{}", "{}"),
            ("legacy # {😀}", "legacy # {😀}"),
        ] {
            let source = format!(
                "% 😀\r\n@book{{café😀,{}={rhs},DATE={{2026-10-23}}}}\r\n",
                field.to_ascii_uppercase()
            );
            let message = warning("café😀", field);
            let diagnostics = parse(
                root,
                &[
                    ("refs.data", &source),
                    ("other.data", "@book{CAFÉ😀,date={2000}}"),
                ],
                &format!(
                    "{}{}WARN - {message}\n",
                    found("refs.data"),
                    found("other.data")
                ),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(item.range, span(&source, marker), "{diagnostics:?}");
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(item.source.as_deref(), Some("latex-build"));
            assert_eq!(item.message, format!("Biber: {message}"));
            assert_eq!(
                item.data,
                Some(serde_json::json!({"key":"café😀","field":field,"overwrittenBy":"date"}))
            );
            let related = item.related_information.as_ref().unwrap();
            assert_eq!(related.len(), 1);
            assert_eq!(
                related[0].location,
                Location::new(uri(root, "refs.data"), span(&source, "2026-10-23"))
            );
        }
    }
}

#[test]
fn overwritten_and_replacing_fields_preserve_independent_ambiguities() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (source, marker, unresolved, related) in [
        (
            "@book{café😀,year={1999},YEAR={2000},date={2026-10-23}}",
            "café😀",
            true,
            true,
        ),
        (
            "@book{café😀,year={1999},YEAR=,date={2026-10-23}}",
            "café😀",
            true,
            true,
        ),
        (
            "@book{café😀,year={1999},date={2026-10-23},DATE={2027}}",
            "1999",
            false,
            false,
        ),
        ("@book{café😀,year={1999},date=}", "1999", false, false),
        (
            "@book{café😀,year={1999},crossref={parent}} @book{parent,date={2026-10-23}}",
            "1999",
            false,
            false,
        ),
        (
            "@book{café😀,publicationyear={1999},date={2026-10-23}}",
            "café😀",
            true,
            true,
        ),
        (
            "@book{café😀,year={1999},published={2026-10-23}}",
            "1999",
            false,
            false,
        ),
        (
            "@book{café😀,publicationyear={1999},published={2026-10-23}}",
            "café😀",
            true,
            false,
        ),
        (
            "@book{café😀,year={1999},date={2026-10-23}",
            "café😀",
            true,
            false,
        ),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                warning("café😀", "year")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(
            item.range,
            span(source, marker),
            "{source}\n{diagnostics:?}"
        );
        assert_eq!(
            item.data.as_ref().unwrap().get("unresolvedField").is_some(),
            unresolved
        );
        assert_eq!(item.related_information.is_some(), related);
        if let Some(related) = &item.related_information {
            assert_eq!(related[0].location.range, span(source, "2026-10-23"));
        }
    }
}

#[test]
fn overwrite_sources_require_complete_prior_history_and_unique_normalized_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,year={1999},date={2026-10-23}}";
    let record = format!("WARN - {}\n", warning("café😀", "year"));
    for log in [
        record.clone(),
        format!("{record}{}", found("refs.data")),
        format!("{}{record}", found("missing.data")),
        format!("{}{}{record}", found("refs.data"), found("missing.data")),
        format!("{}ERROR - Earlier failure\n{record}", found("refs.data")),
        format!(
            "{}[latex-lsp: middle of compiler output omitted]\n{record}",
            found("refs.data")
        ),
        format!("{}{}{record}", found("refs.data"), found("duplicate.data")),
    ] {
        let diagnostics = parse(
            root,
            &[
                ("refs.data", source),
                ("duplicate.data", "@book{café😀,year={1998},date={2026}}"),
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
        assert_eq!(item.data.as_ref().unwrap()["overwrittenBy"], "date");
        assert!(item.related_information.is_none());
        assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
    }
    for duplicate in [
        "@book{café😀,year={1999}} @book{café😀,year={1998}}",
        "@book{CAFÉ😀,year={1999}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", duplicate)],
            &format!("{}{record}", found("refs.data")),
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
fn overwrite_templates_accept_only_the_native_date_components_and_complete_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for message in [
        warning("alpha", "day"),
        warning("alpha", "YEAR"),
        warning("", "year"),
        warning("alpha beta", "year"),
        "Overwriting field 'year' with month value from field 'date' for entry 'alpha'".into(),
        "Overwriting field 'year' with year value from field 'origdate' for entry 'alpha'".into(),
        "Overwriting field 'year' with year value from field 'date' for entry 'alpha".into(),
        format!("{} extra", warning("alpha", "year")),
        format!("Example: {}", warning("alpha", "year")),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", "@book{alpha,year={1999},date={2026}}")],
            &format!("{}WARN - {message}\n", found("refs.data")),
        );
        assert!(
            diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{message}\n{diagnostics:?}"
        );
    }
    for level in ["INFO", "DEBUG"] {
        let diagnostics = parse(
            root,
            &[("refs.data", "@book{alpha,year={1999},date={2026}}")],
            &format!(
                "{}{level} - {}\n",
                found("refs.data"),
                warning("alpha", "year")
            ),
        );
        assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
    }
    for key in ["café😀", "O'Connor.bib", "'quoted'"] {
        let source = format!("% 😀\r\n@book{{{key},year={{1999}},date={{2026-10-23}}}}\r\n");
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!("{}WARN - {}\n", found("refs.data"), warning(key, "year")),
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "refs.data"))[0].range,
            span(&source, "1999")
        );
    }
}

#[test]
fn overwrite_diagnostics_refresh_without_choosing_a_field_to_delete_or_rewrite() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let log = format!(
        "{}WARN - {}\n",
        found("refs.data"),
        warning("café😀", "year")
    );
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for source in [
        "% 😀\r\n@book{café😀,year={1999},date={2026-10-23}}",
        "% 😀\r\n@book{café😀,\r\n year={1999},\r\n date={2026-10-23}}",
    ] {
        let diagnostics = parse(root, &[("refs.data", source)], &log);
        workspace.open_with_language(uri(root, "refs.data"), source.into(), 7, "bibtex");
        let item = items(&diagnostics, &uri(root, "refs.data"))[0].clone();
        assert_eq!(item.range, span(source, "1999"));
        assert_eq!(
            item.related_information.as_ref().unwrap()[0].location.range,
            span(source, "2026-10-23")
        );
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
    assert!(parse(
        root,
        &[("refs.data", "@book{café😀,year={2026},date={2026-10-23}}")],
        &found("refs.data")
    )
    .is_empty());
}

#[test]
fn thousands_of_overwrites_share_field_and_related_date_ranges_on_one_unicode_line() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = String::new();
    let mut log = found("refs.data");
    let mut expected = BTreeMap::new();
    let mut offset = 0;
    for index in 0..1500 {
        let key = format!("café{index}😀");
        let entry = format!(
            "@book{{café{index}😀,title={{😀}},year={{1999}},month={{2}},date={{2026-10-23}}}} "
        );
        let at = |needle: &str, skip: usize, length| {
            let start = entry.find(needle).unwrap() + skip;
            let start = offset + entry[..start].encode_utf16().count() as u32;
            Range::new(Position::new(0, start), Position::new(0, start + length))
        };
        let date = at("2026-10-23", 0, 10);
        for (field, range) in [("year", at("year={", 6, 4)), ("month", at("month={", 7, 1))] {
            expected.insert((key.clone(), field), (range, date));
            log.push_str(&format!("WARN - {}\n", warning(&key, field)));
        }
        offset += entry.encode_utf16().count() as u32;
        source.push_str(&entry);
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 3000);
    for item in items {
        let data = item.data.as_ref().unwrap();
        let (range, date) = expected[&(
            data["key"].as_str().unwrap().to_string(),
            data["field"].as_str().unwrap(),
        )];
        assert_eq!(item.range, range);
        let related = item.related_information.as_ref().unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].location.range, date);
    }
}

fn native(
    root: &Path,
    source: &str,
    mapping: Option<&str>,
) -> (String, BTreeMap<Url, Vec<Diagnostic>>) {
    std::fs::write(root.join("refs.data"), source).unwrap();
    let mut command = std::process::Command::new("biber");
    command.args(["--tool", "--output-file=processed.bib"]);
    if let Some(mapping) = mapping {
        std::fs::write(root.join("mapping.conf"), mapping).unwrap();
        command.arg("--configfile=mapping.conf");
    }
    let output = command.arg("refs.data").current_dir(root).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(root.join("refs.data")).unwrap(),
        source
    );
    let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
    let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
    (log, diagnostics)
}

#[test]
#[ignore = "requires biber"]
fn real_biber_date_overwrites_move_and_clear_for_both_field_orders_and_date_ranges() {
    for date_first in [false, true] {
        for date in ["2026-10-23", "2026-10-23/2027-11-24"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            for stage in 0..3 {
                let year = if stage == 2 { "2026" } else { "1999" };
                let month = if stage == 2 { "10" } else { "2" };
                let fields = format!("YEAR={{{year}}},MONTH={{{month}}}");
                let fields = if date_first {
                    format!("DATE={{{date}}},{fields}")
                } else {
                    format!("{fields},DATE={{{date}}}")
                };
                let source = format!(
                    "% 😀\r\n@book{{café😀,title={{Title}},author={{Doe, Jane}},{}{fields}}}\r\n",
                    if stage == 0 { "" } else { "\r\n " }
                );
                let (log, diagnostics) = native(root, &source, None);
                let items = items(&diagnostics, &uri(root, "refs.data"));
                if stage == 2 {
                    assert!(items.is_empty(), "{source}\n{log}\n{diagnostics:?}");
                    continue;
                }
                assert_eq!(items.len(), 2, "{source}\n{log}\n{diagnostics:?}");
                for (field, marker) in [("year", year), ("month", month)] {
                    let item = items
                        .iter()
                        .find(|item| item.data.as_ref().unwrap()["field"] == field)
                        .unwrap();
                    // The month numeral also occurs inside the date value.
                    let prefix = format!("{}={{", field.to_ascii_uppercase());
                    let at = source.find(&prefix).unwrap() + prefix.len();
                    assert_eq!(
                        item.range,
                        Text::new(source.clone()).range(at, at + marker.len()),
                        "{source}\n{log}"
                    );
                    assert_eq!(
                        item.related_information.as_ref().unwrap()[0].location.range,
                        span(&source, date)
                    );
                    assert_eq!(
                        item.data,
                        Some(
                            serde_json::json!({"key":"café😀","field":field,"overwrittenBy":"date"})
                        )
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_sourcemap_overwrites_only_link_fields_present_in_the_original_source() {
    for (fields, steps, raw_year, raw_date) in [
        (
            "year={1999},month={2},published={2026-10-23}",
            r#"<map_step map_field_source="published" map_field_target="date"/>"#,
            true,
            false,
        ),
        (
            "publicationyear={1999},date={2026-10-23}",
            r#"<map_step map_field_source="publicationyear" map_field_target="year"/>"#,
            false,
            true,
        ),
        (
            "publicationyear={1999},published={2026-10-23}",
            r#"<map_step map_field_source="publicationyear" map_field_target="year"/><map_step map_field_source="published" map_field_target="date"/>"#,
            false,
            false,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source =
            format!("% 😀\r\n@book{{café😀,title={{Title}},author={{Doe, Jane}},{fields}}}\r\n");
        let config = format!("<config><sourcemap><maps datatype=\"bibtex\"><map>{steps}</map></maps></sourcemap></config>");
        let (log, diagnostics) = native(root, &source, Some(&config));
        let items = items(&diagnostics, &uri(root, "refs.data"));
        assert_eq!(
            items.len(),
            if raw_year { 2 } else { 1 },
            "{source}\n{log}\n{diagnostics:?}"
        );
        let item = items
            .iter()
            .find(|item| item.data.as_ref().unwrap()["field"] == "year")
            .unwrap();
        assert_eq!(
            item.range,
            span(&source, if raw_year { "1999" } else { "café😀" })
        );
        assert_eq!(
            item.data.as_ref().unwrap().get("unresolvedField").is_some(),
            !raw_year
        );
        assert_eq!(item.related_information.is_some(), raw_date);
        if raw_date {
            assert_eq!(
                item.related_information.as_ref().unwrap()[0].location.range,
                span(&source, "2026-10-23")
            );
        }
    }
}
