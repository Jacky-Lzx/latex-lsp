use latex_lsp::{compilation::build_log, editing, project::workspace::Workspace, text::Text};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const NAME: &str = "invalid-bibliography-field";
const VALUE: &str = "invalid-bibliography-field-value";
fn uri(root: &Path, file: &str) -> Url {
    Url::from_file_path(root.join(file)).unwrap()
}
fn found(file: &str) -> String {
    format!("INFO - Found BibTeX data source '{file}'\n")
}
fn name(key: &str, file: &str, field: &str) -> String {
    format!("Datamodel: Entry '{key}' ({file}): Field '{field}' invalid in data model - ignoring")
}
fn value(key: &str, file: &str, field: &str) -> String {
    format!("Datamodel: Entry '{key}' ({file}): Invalid value of field '{field}' must be datatype 'integer' - ignoring field")
}
fn parse(root: &Path, inputs: &[(&str, &str)], log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    for (file, source) in inputs {
        std::fs::write(root.join(file), source).unwrap();
    }
    build_log::parse_bibliography(&root.join("main.tex"), log)
}
fn items(diagnostics: &BTreeMap<Url, Vec<Diagnostic>>, target: &Url) -> Vec<Diagnostic> {
    diagnostics.get(target).into_iter().flatten().filter(|item| matches!(item.code.as_ref(), Some(NumberOrString::String(code)) if code == NAME || code == VALUE)).cloned().collect()
}
fn span(source: &str, needle: &str) -> Range {
    let start = source.find(needle).unwrap();
    Text::new(source.into()).range(start, start + needle.len())
}

fn unqualified(field: &str, key: &str, value: &str) -> String {
    match field {
        "year" | "month" => format!("legacy {field} field '{value}' in entry '{key}' is not an integer - this will probably not sort properly."),
        "isbn" => format!("ISBN '{value}' in entry '{key}' is invalid - run biber with '--validate_datamodel' for details."),
        "pages" => range(key, "pages"),
        _ => unreachable!(),
    }
}

fn range(key: &str, field: &str) -> String {
    format!("Range field '{field}' in entry '{key}' is malformed, falling back to literal")
}

#[test]
fn unqualified_default_field_warnings_mark_values_across_all_prior_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for field in ["year", "month", "isbn", "pages"] {
        for (rhs, selected) in [
            ("{invalid😀}", "invalid😀"),
            ("\"invalid😀\"", "invalid😀"),
            ("{}", "{}"),
            ("bad # {😀}", "bad # {😀}"),
        ] {
            let source = format!("% 😀\r\n@book{{café😀,{field}={rhs}}}\r\n");
            let message = unqualified(field, "café😀", "invalid😀");
            let log = format!(
                "{}{}WARN - {message}\n",
                found("refs.data"),
                found("other.data")
            );
            let diagnostics = parse(
                root,
                &[
                    ("refs.data", &source),
                    ("other.data", "@book{CAFÉ😀,year={Other}}"),
                ],
                &log,
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(item.range, span(&source, selected), "{diagnostics:?}");
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(item.message, format!("Biber: {message}"));
            assert_eq!(item.source.as_deref(), Some("latex-build"));
            assert_eq!(
                item.data,
                Some(serde_json::json!({"key":"café😀","field":field}))
            );
        }
    }
}

#[test]
fn unqualified_warning_values_cannot_supply_fake_keys_paths_or_line_numbers() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let value = "bad' in entry 'decoy' (decoy.bib): line 99 - WARN - ISBN 'nested";
    for field in ["year", "month", "isbn", "pages"] {
        for key in ["café😀", "O'Connor.bib", "'quoted'"] {
            let source = format!("% 😀\r\n@book{{{key},{field}={{{value}}}}}");
            let message = unqualified(field, key, value);
            let diagnostics = parse(
                root,
                &[
                    ("refs.data", &source),
                    ("decoy.bib", "@book{decoy,year={Bad},isbn={Bad}}"),
                ],
                &format!(
                    "{}{}WARN - {message}\n",
                    found("refs.data"),
                    found("decoy.bib")
                ),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(item.range, span(&source, value));
            assert_eq!(item.data.as_ref().unwrap()["key"], key);
            assert_eq!(item.message, format!("Biber: {message}"));
        }
    }
}

#[test]
fn unqualified_warnings_require_complete_prior_biber_input_history() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{alpha,year={bad},month={bad},isbn={bad},pages={-12}}";
    for field in ["year", "month", "isbn", "pages"] {
        let warning = format!("WARN - {}\n", unqualified(field, "alpha", "bad"));
        for log in [
            warning.clone(),
            format!("{warning}{}", found("refs.bib")),
            format!("Database file #1: refs.bib\n{warning}"),
            format!("INFO - Example: Found BibTeX data source 'refs.bib'\n{warning}"),
            format!("DEBUG - Found BibTeX data source 'refs.bib'\n{warning}"),
            format!(
                "{}{}ERROR - Previous parse failure\n{warning}",
                found("refs.bib"),
                found("other.bib")
            ),
            format!(
                "{}{}[latex-lsp: middle of compiler output omitted]\n{warning}",
                found("refs.bib"),
                found("other.bib")
            ),
            format!("{}{}{warning}", found("refs.bib"), found("missing.bib")),
            format!(
                "{}INFO - Found BibTeX data source 'unterminated\n{warning}",
                found("refs.bib")
            ),
            format!("{}{warning}", found(" refs.bib ")),
        ] {
            let diagnostics = parse(
                root,
                &[
                    ("refs.bib", source),
                    ("other.bib", "@book{other,year={2026}}"),
                ],
                &log,
            );
            assert!(
                items(&diagnostics, &uri(root, "refs.bib")).is_empty(),
                "{log}\n{diagnostics:?}"
            );
            let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
            assert_eq!(
                item.data.as_ref().unwrap()["unresolvedSource"],
                true,
                "{log}"
            );
            assert_eq!(item.range, Range::default());
        }
    }
}

#[test]
fn unqualified_warning_keys_must_be_unique_after_normalization_in_every_database() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (first, second) in [
        ("@book{café😀,year={bad}}", "@book{café😀,year={Other}}"),
        (
            "@book{café😀,year={bad}} @book{café😀,year={Duplicate}}",
            "@book{other,year={2026}}",
        ),
        (
            "@book{other,year={2026}}",
            "@book{café😀,year={bad}} @book{café😀,year={Duplicate}}",
        ),
        ("@book{other,year={2026}}", "@book{CAFÉ😀,year={bad}}"),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.bib", first), ("other.bib", second)],
            &format!(
                "{}{}WARN - {}\n",
                found("refs.bib"),
                found("other.bib"),
                unqualified("year", "café😀", "bad")
            ),
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "main.tex"))[0]
                .data
                .as_ref()
                .unwrap()["unresolvedSource"],
            true,
            "{diagnostics:?}"
        );
        assert!(items(&diagnostics, &uri(root, "refs.bib")).is_empty());
        assert!(items(&diagnostics, &uri(root, "other.bib")).is_empty());
    }
}

#[test]
fn unqualified_values_without_a_unique_complete_field_fall_back_to_the_entry_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        "@book{café😀,year={First},YEAR={Second}}",
        "@book{café😀,year=}",
        "@book{café😀,year={Unclosed}",
        "@book{café😀,crossref={parent}} @book{parent,year={bad}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                unqualified("year", "café😀", "bad")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(source, "café😀"), "{diagnostics:?}");
        assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
    }
}

#[test]
fn unqualified_warning_source_limits_and_unreadable_candidates_preserve_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("refs.data"), "@book{alpha,year={bad}}").unwrap();
    let warning = format!("WARN - {}\n", unqualified("year", "alpha", "bad"));
    for bytes in [vec![0xff], vec![b' '; 2 * 1024 * 1024 + 1]] {
        std::fs::write(root.join("other.data"), bytes).unwrap();
        let diagnostics = parse(
            root,
            &[],
            &format!("{}{}{warning}", found("refs.data"), found("other.data")),
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "main.tex"))[0]
                .data
                .as_ref()
                .unwrap()["unresolvedSource"],
            true
        );
    }
    let mut log = found("refs.data");
    for index in 0..1024 {
        let file = format!("input{index}.data");
        std::fs::write(root.join(&file), "").unwrap();
        log.push_str(&found(&file));
    }
    log.push_str(&warning);
    let diagnostics = parse(root, &[], &log);
    assert_eq!(
        items(&diagnostics, &uri(root, "main.tex"))[0]
            .data
            .as_ref()
            .unwrap()["unresolvedSource"],
        true
    );
}

#[test]
#[cfg(unix)]
fn unqualified_history_deduplicates_canonical_inputs_with_special_names() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let file = "author's refs,part.data";
    let source = "@book{café😀,year={bad😀}}";
    std::fs::write(root.join(file), source).unwrap();
    std::os::unix::fs::symlink(root.join(file), root.join("alias.tex")).unwrap();
    let diagnostics = parse(
        root,
        &[],
        &format!(
            "{}{}{}WARN - {}\n",
            found(file),
            found("alias.tex"),
            found(&format!("./{file}")),
            unqualified("year", "café😀", "bad😀")
        ),
    );
    assert_eq!(
        items(&diagnostics, &uri(root, file))[0].range,
        span(source, "bad😀")
    );
}

#[test]
fn unqualified_sources_are_checked_at_each_warning_without_borrowing_future_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{café😀,year={bad😀}}";
    let message = unqualified("year", "café😀", "bad😀");
    let diagnostics = parse(
        root,
        &[("first.data", source), ("second.data", source)],
        &format!(
            "{}WARN - {message}\n{}WARN - {message}\n",
            found("first.data"),
            found("second.data")
        ),
    );
    let item = &items(&diagnostics, &uri(root, "first.data"))[0];
    assert_eq!(item.range, span(source, "bad😀"));
    assert_eq!(
        items(&diagnostics, &uri(root, "main.tex"))[0]
            .data
            .as_ref()
            .unwrap()["unresolvedSource"],
        true
    );
    assert!(items(&diagnostics, &uri(root, "second.data")).is_empty());
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace
        .build_diagnostics
        .insert(uri(root, "main.tex"), diagnostics.clone());
    assert!(editing::code_actions(
        &workspace,
        &uri(root, "first.data"),
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
fn unqualified_warning_total_read_budget_does_not_skip_unchecked_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut log = found("refs.data");
    std::fs::write(root.join("refs.data"), "@book{alpha,year={bad}}").unwrap();
    let filler = " ".repeat(2 * 1024 * 1024);
    for index in 0..8 {
        let file = format!("input{index}.data");
        std::fs::write(root.join(&file), &filler).unwrap();
        log.push_str(&found(&file));
    }
    log.push_str(&format!("WARN - {}\n", unqualified("year", "alpha", "bad")));
    let diagnostics = parse(root, &[], &log);
    assert_eq!(
        items(&diagnostics, &uri(root, "main.tex"))[0]
            .data
            .as_ref()
            .unwrap()["unresolvedSource"],
        true
    );
    assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
}

#[test]
fn unqualified_field_messages_require_complete_templates_and_native_warning_records() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for field in ["year", "month", "isbn", "pages"] {
        let message = unqualified(field, "alpha", "bad");
        for record in [
            format!("INFO - {message}"),
            format!("DEBUG - {message}"),
            format!(" : WARN - {message}"),
            format!("Warning--{message}"),
            format!("WARN - Example: {message}"),
            format!("WARN - {message} trailing text"),
            format!("WARN - {}", &message[..message.len() - 1]),
        ] {
            let diagnostics = parse(
                root,
                &[("refs.bib", "@book{alpha,year={bad},isbn={bad}}")],
                &format!("{}{record}\n", found("refs.bib")),
            );
            assert!(
                diagnostics
                    .values()
                    .flatten()
                    .all(|item| item.code != Some(NumberOrString::String(VALUE.into()))),
                "{record}: {diagnostics:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_default_field_warnings_move_and_clear_without_datamodel_validation() {
    for (field, invalid, valid) in [
        ("year", "invalid😀", "2026"),
        ("month", "invalid😀", "10"),
        ("isbn", "invalid😀", "9780306406157"),
        ("pages", "-12", "12--15"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let file = "author's refs,part.data";
        for stage in 0..3 {
            let value = if stage == 2 { valid } else { invalid };
            let source = format!("{}% 😀\r\n@book{{café😀,title={{Title}},author={{Doe, Jane}},{field}={{{value}}}}}\r\n", if stage == 1 { "\r\n" } else { "" });
            std::fs::write(root.join(file), &source).unwrap();
            let output = std::process::Command::new("biber")
                .args(["--tool", "--output-file=processed.bib", file])
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
                assert_eq!(items[0].range, span(&source, value));
                assert_eq!(
                    items[0].message,
                    format!("Biber: {}", unqualified(field, "café😀", value))
                );
                assert_eq!(
                    items[0].data,
                    Some(serde_json::json!({"key":"café😀","field":field}))
                );
            }
            assert_eq!(std::fs::read_to_string(root.join(file)).unwrap(), source);
        }
    }
}

#[test]
fn invalid_field_names_mark_only_the_reported_entry_and_preserve_native_messages() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for field in ["UNKNOWNFIELD", "custom:field", "field$name", "fÖÖ"] {
        let source = format!("% 😀\r\n@book{{other,{field}={{Wrong}}}}\r\n@book(\r\n alpha😀,title={{😀 Title}},\r\n {field} % 😀\r\n = {{Value}})\r\n");
        let reported = if field == "fÖÖ" {
            "fÖÖ".into()
        } else {
            field.to_ascii_lowercase()
        };
        let message = name("alpha😀", "refs.data", &reported);
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!("{}WARN - {message}\n", found("refs.data")),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        let start = source.rfind(field).unwrap();
        assert_eq!(
            item.range,
            Text::new(source.clone()).range(start, start + field.len())
        );
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(item.code, Some(NumberOrString::String(NAME.into())));
        assert_eq!(item.message, format!("Biber: {message}"));
        assert_eq!(item.source.as_deref(), Some("latex-build"));
        assert_eq!(
            item.data,
            Some(serde_json::json!({"key":"alpha😀","field":reported}))
        );
    }
}

#[test]
fn invalid_field_value_records_select_literals_expressions_and_visible_empty_values() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (rhs, selected) in [
        ("{bad😀}", "bad😀"),
        ("\"bad😀\"", "bad😀"),
        ("{}", "{}"),
        ("\"\"", "\"\""),
        ("{bad {😀}}", "bad {😀}"),
        ("bad # {😀}", "bad # {😀}"),
        ("123", "123"),
    ] {
        let source = format!(
            "% 😀\r\n@book{{alpha😀,volume % comment\r\n = % 😀\r\n {rhs},title={{Title}}}}\r\n"
        );
        let diagnostics = parse(
            root,
            &[("refs.bib", &source)],
            &format!("WARN - {}\n", value("alpha😀", "refs.bib", "volume")),
        );
        let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
        assert_eq!(
            item.range,
            span(&source, selected),
            "{rhs}: {diagnostics:?}"
        );
        assert_eq!(item.code, Some(NumberOrString::String(VALUE.into())));
        assert_eq!(
            item.data,
            Some(serde_json::json!({"key":"alpha😀","field":"volume"}))
        );
    }
}

#[test]
fn native_validation_templates_keep_name_and_value_codes_separate() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{alpha,field={bad}}";
    let cases = [
        ("Datamodel: Entry 'alpha' (refs.bib): Invalid field 'field' for entrytype 'book'", NAME),
        ("Datamodel: Entry 'alpha' (refs.bib): Invalid ISBN in value of field 'field'", VALUE),
        ("Datamodel: Entry 'alpha' (refs.bib): Invalid ISSN in value of field 'field'", VALUE),
        ("Datamodel: Entry 'alpha' (refs.bib): Invalid ISMN in value of field 'field'", VALUE),
        ("Datamodel: Entry 'alpha' (refs.bib): Invalid value (pattern match fails) for field 'field'", VALUE),
        ("Datamodel: Entry 'alpha' (refs.bib): Invalid value of field 'field' must be '>=-2' - ignoring field", VALUE),
        ("Datamodel: Entry 'alpha' (refs.bib): Invalid value of field 'field' must be '<=20.5' - ignoring field", VALUE),
        ("Entry 'alpha' (refs.bib): Invalid format 'bad' of date field 'field' - ignoring", VALUE),
        ("Entry 'alpha' (refs.bib): Invalid format 'bad' of end date field 'field' - ignoring", VALUE),
        ("Entry 'alpha' (refs.bib): Invalid format 'bad\" and '\\ stuff' of date field 'field' - ignoring", VALUE),
    ];
    for (message, code) in cases {
        let diagnostics = parse(
            root,
            &[("refs.bib", source)],
            &format!("WARN - {message}\n"),
        );
        let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
        assert_eq!(
            item.range,
            span(source, if code == NAME { "field" } else { "bad" }),
            "{message}"
        );
        assert_eq!(item.code, Some(NumberOrString::String(code.into())));
    }
}

#[test]
fn invalid_fields_never_borrow_a_missing_or_ambiguous_entry_from_other_sources() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        "% Header\n@book{ALPHA,wrong={Value}}",
        "% Header\n@book{alpha,wrong={First}} @book{alpha,wrong={Second}}",
        "% Header\n@string{alpha={Not an entry}}",
        "% Header\n@comment{@book{alpha,wrong={Hidden}}}",
    ] {
        let diagnostics = parse(
            root,
            &[
                ("refs.bib", source),
                ("other.bib", "@book{alpha,wrong={Other}}"),
            ],
            &format!(
                "{}WARN - {}\n",
                found("other.bib"),
                name("alpha", "refs.bib", "wrong")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
        assert_eq!(item.range, span(source, "% Header"));
        assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
    }
}

#[test]
fn incomplete_duplicate_and_inherited_fields_use_the_unique_entry_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        "@book{alpha,title={No matching field}}",
        "@book{alpha,wrong={First},WRONG={Second}}",
        "@book{alpha,wrong={Unclosed}",
        "@book{alpha,wrong=}",
        "@book{alpha,wrong}",
        "@book{alpha,wrong={bad} trailing}",
        "@book{alpha,crossref={parent}} @book{parent,wrong={Inherited}}",
        "@book{other,wrong={Elsewhere}} @book{alpha,title={No field}}",
    ] {
        for message in [
            name("alpha", "refs.bib", "wrong"),
            value("alpha", "refs.bib", "wrong"),
        ] {
            let diagnostics = parse(
                root,
                &[("refs.bib", source)],
                &format!("WARN - {message}\n"),
            );
            let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
            assert_eq!(
                item.range,
                span(source, "alpha"),
                "{source}: {diagnostics:?}"
            );
            assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
        }
    }
}

#[test]
fn unicode_key_and_field_normalization_preserves_source_ranges_and_detects_collisions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for field in ["FÖÖ", "FÖÖ"] {
        let source = format!("% 😀\r\n@book{{café😀,{field}={{Value}}}}\r\n");
        let diagnostics = parse(
            root,
            &[("refs.bib", &source)],
            &format!("WARN - {}\n", name("café😀", "refs.bib", "fÖÖ")),
        );
        let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
        assert_eq!(item.range, span(&source, field));
        assert!(item.data.as_ref().unwrap().get("unresolvedField").is_none());
    }
    let source = "@book{café,FÖÖ={First},FÖÖ={Second}}";
    let diagnostics = parse(
        root,
        &[("refs.bib", source)],
        &format!("WARN - {}\n", name("café", "refs.bib", "fÖÖ")),
    );
    let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
    assert_eq!(item.range, span(source, "café"));
    assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
    let source = "% Header\n@book{café,wrong={First}} @book{café,wrong={Second}}";
    let diagnostics = parse(
        root,
        &[("refs.bib", source)],
        &format!("WARN - {}\n", name("café", "refs.bib", "wrong")),
    );
    let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
    assert_eq!(item.range, span(source, "% Header"));
    assert_eq!(item.data.as_ref().unwrap()["unresolvedEntry"], true);
}

#[test]
fn malformed_range_warnings_keep_raw_field_spellings_and_whole_expressions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (rhs, marker) in [
        ("{-12}", "-12"),
        ("\"-12\"", "-12"),
        ("{12--15, -99}", "12--15, -99"),
        ("bad # {-12}", "bad # {-12}"),
    ] {
        let source = format!("% 😀\r\n@book{{café😀,PAGES={rhs},Pageſ={{-13}}}}\r\n");
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!(
                "{}WARN - {}\nWARN - {}\n",
                found("refs.data"),
                range("café😀", "pages"),
                range("café😀", "pageſ")
            ),
        );
        let items = items(&diagnostics, &uri(root, "refs.data"));
        assert_eq!(items.len(), 2, "{source}\n{diagnostics:?}");
        for (field, marker) in [("pages", marker), ("pageſ", "-13")] {
            let item = items
                .iter()
                .find(|item| item.data.as_ref().unwrap()["field"] == field)
                .unwrap();
            assert_eq!(item.range, span(&source, marker), "{source}\n{item:?}");
            assert_eq!(
                item.data,
                Some(serde_json::json!({"key":"café😀","field":field}))
            );
            assert_eq!(item.message, format!("Biber: {}", range("café😀", field)));
        }
    }
}

#[test]
fn malformed_range_warnings_fall_back_for_incomplete_duplicate_or_inherited_input_fields() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for assignments in [
        "pages={-12},PAGES={-13}",
        "pages={-12},PAGES=",
        "pages",
        "pages=",
        "crossref={parent}",
    ] {
        let source =
            format!("% 😀\r\n@book{{café😀,{assignments}}}\r\n@book{{parent,pages={{-12}}}}");
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                range("café😀", "pages")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(
            item.range,
            span(&source, "café😀"),
            "{source}\n{diagnostics:?}"
        );
        assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
    }
}

#[test]
fn malformed_range_templates_and_legacy_field_names_require_complete_native_identifiers() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for message in [
        range("alpha", ""), range("alpha", "pages' in entry 'decoy"), range("alpha", "{pages}"), range("", "pages"), range("alpha beta", "pages"),
        "Range field 'pages' in entry 'alpha' is malformed".into(),
        "Range field 'pages' in entry 'alpha' is malformed, falling back to literal extra".into(),
        "legacy day field 'bad' in entry 'alpha' is not an integer - this will probably not sort properly.".into(),
        "legacy MONTH field 'bad' in entry 'alpha' is not an integer - this will probably not sort properly.".into(),
    ] {
        let diagnostics = parse(root, &[("refs.data", "@book{alpha,pages={-12},month={bad}}")], &format!("{}WARN - {message}\n", found("refs.data")));
        assert!(diagnostics.values().flatten().all(|item| item.code != Some(NumberOrString::String(VALUE.into()))), "{message}\n{diagnostics:?}");
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_range_input_aliases_move_and_clear_independently() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for stage in 0..3 {
        let first = if stage == 2 { "12--15" } else { "-12" };
        let second = if stage == 2 { "20--23" } else { "-13" };
        let source = format!("% 😀\r\n@book{{café😀,title={{Title}},author={{Doe, Jane}},year={{2026}},{}PAGES={{{first}}},Pageſ={{{second}}}}}\r\n", if stage == 0 { "" } else { "\r\n " });
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
            assert!(items.is_empty(), "{source}\n{log}\n{diagnostics:?}");
            continue;
        }
        assert_eq!(items.len(), 2, "{source}\n{log}\n{diagnostics:?}");
        for (field, marker) in [("pages", first), ("pageſ", second)] {
            let item = items
                .iter()
                .find(|item| item.data.as_ref().unwrap()["field"] == field)
                .unwrap();
            assert_eq!(
                item.range,
                span(&source, marker),
                "{source}\n{log}\n{item:?}"
            );
            assert_eq!(
                item.data,
                Some(serde_json::json!({"key":"café😀","field":field}))
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.join("refs.data")).unwrap(),
            source
        );
    }
}

#[test]
fn model_field_warnings_use_full_unicode_folding_for_names_and_values() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (raw, reported) in [
        ("FÖÖ", "föö"),
        ("FÖÖ", "föö"),
        ("Straße", "strasse"),
        ("STRAẞE", "strasse"),
        ("ΟΣ", "οσ"),
        ("Ος", "οσ"),
        ("İTYPE", "i̇type"),
        ("ıTYPE", "ıtype"),
        ("ﬃTYPE", "ffitype"),
        ("IſBN", "isbn"),
    ] {
        let source = format!("% 😀\r\n@book{{café😀,{raw}={{invalid😀}}}}\r\n");
        for (suffix, code, marker) in [
            (format!("Invalid field '{reported}' for entrytype 'book'"), NAME, raw),
            (format!("Invalid value of field '{reported}' must be datatype 'integer' - ignoring field"), VALUE, "invalid😀"),
            (format!("Invalid value of field '{reported}' must be '>=0' - ignoring field"), VALUE, "invalid😀"),
            (format!("Invalid value of field '{reported}' must be '<=9' - ignoring field"), VALUE, "invalid😀"),
            (format!("Invalid value (pattern match fails) for field '{reported}'"), VALUE, "invalid😀"),
            (format!("Invalid ISBN in value of field '{reported}'"), VALUE, "invalid😀"),
            (format!("Invalid ISSN in value of field '{reported}'"), VALUE, "invalid😀"),
            (format!("Invalid ISMN in value of field '{reported}'"), VALUE, "invalid😀"),
        ] {
            let log = format!("WARN - Datamodel: Entry 'café😀' (refs.bib): {suffix}\n");
            let diagnostics = parse(root, &[("refs.bib", &source)], &log);
            let items = items(&diagnostics, &uri(root, "refs.bib"));
            assert_eq!(items.len(), 1, "{source}\n{log}\n{diagnostics:?}");
            let item = &items[0];
            assert_eq!(item.range, span(&source, marker), "{source}\n{log}");
            assert_eq!(item.code, Some(NumberOrString::String(code.into())));
            assert_eq!(item.data, Some(serde_json::json!({"key":"café😀","field":reported})));
        }
    }
}

#[test]
fn model_field_collisions_never_select_an_exact_raw_spelling() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for assignments in [
        "strasse={12},Straße={invalid😀}",
        "Straße={invalid😀},STRASSE={12}",
        "strasse={12},STRAẞE=",
        "strasse={12},Straße",
        "strasse={12},Straße={bad} trailing",
        "strasse={12},STRAẞE={Other}",
    ] {
        let source = format!("% 😀\r\n@book{{café😀,{assignments}}}\r\n");
        for suffix in [
            "Invalid field 'strasse' for entrytype 'book'",
            "Invalid value of field 'strasse' must be datatype 'integer' - ignoring field",
        ] {
            let diagnostics = parse(
                root,
                &[("refs.bib", &source)],
                &format!("WARN - Datamodel: Entry 'café😀' (refs.bib): {suffix}\n"),
            );
            let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
            assert_eq!(
                item.range,
                span(&source, "café😀"),
                "{source}\n{diagnostics:?}"
            );
            assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
        }
    }
}

#[test]
fn unknown_input_fields_and_date_messages_keep_distinct_non_ascii_spellings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (first, first_reported, second, second_reported) in [
        ("Straße", "straße", "STRASSE", "strasse"),
        ("FÖÖ", "fÖÖ", "föö", "föö"),
        ("ΟΣ", "ΟΣ", "Ος", "Ος"),
        ("İTYPE", "İtype", "itype", "itype"),
        ("IſBN", "iſbn", "ISBN", "isbn"),
    ] {
        let source =
            format!("% 😀\r\n@book{{café😀,{first}={{First😀}},{second}={{Second😀}}}}\r\n");
        for (reported, raw, value) in [
            (first_reported, first, "First😀"),
            (second_reported, second, "Second😀"),
        ] {
            let diagnostics = parse(
                root,
                &[("refs.bib", &source)],
                &format!("WARN - {}\n", name("café😀", "refs.bib", reported)),
            );
            let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
            assert_eq!(item.range, span(&source, raw), "{source}\n{diagnostics:?}");
            assert!(item.data.as_ref().unwrap().get("unresolvedField").is_none());
            for part in ["date", "end date"] {
                let log = format!("WARN - Entry 'café😀' (refs.bib): Invalid format '{value}' of {part} field '{reported}' - ignoring\n");
                let diagnostics = parse(root, &[("refs.bib", &source)], &log);
                let item = &items(&diagnostics, &uri(root, "refs.bib"))[0];
                assert_eq!(
                    item.range,
                    span(&source, value),
                    "{source}\n{log}\n{diagnostics:?}"
                );
                assert!(item.data.as_ref().unwrap().get("unresolvedField").is_none());
            }
        }
    }
}

#[test]
fn unqualified_isbn_warnings_fold_model_aliases_and_preserve_collisions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (assignments, marker, unresolved) in [
        ("IſBN={invalid😀}", "invalid😀", false),
        ("iſbn={invalid😀}", "invalid😀", false),
        ("ISBN={9780306406157},IſBN={invalid😀}", "café😀", true),
        ("IſBN={invalid😀},isbn={9780306406157}", "café😀", true),
        ("isbn={9780306406157},IſBN=", "café😀", true),
    ] {
        let source = format!("% 😀\r\n@book{{café😀,{assignments}}}\r\n");
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                unqualified("isbn", "café😀", "invalid😀")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(
            item.range,
            span(&source, marker),
            "{source}\n{diagnostics:?}"
        );
        assert_eq!(
            item.data.as_ref().unwrap().get("unresolvedField").is_some(),
            unresolved
        );
    }
}

#[test]
fn thousands_of_input_and_model_field_warnings_share_one_forward_utf16_index() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = String::new();
    let mut log = found("refs.data");
    let mut expected = BTreeMap::new();
    let mut offset = 0;
    for index in 0..1500 {
        let key = format!("café{index}😀");
        let reported_key = format!("café{index}😀");
        let entry = format!(
            "@book{{{key},Straße={{invalid😀}},STRASSE={{12}},IſBN={{bad😀}},FÖÖ={{Other😀}}}} "
        );
        let at = |marker: &str| {
            let start = entry.find(marker).unwrap();
            let start = offset + entry[..start].encode_utf16().count() as u32;
            Range::new(
                Position::new(0, start),
                Position::new(0, start + marker.encode_utf16().count() as u32),
            )
        };
        for (code, field, marker) in [
            (NAME, "straße", "Straße"),
            (NAME, "strasse", "STRASSE"),
            (VALUE, "strasse", key.as_str()),
            (VALUE, "isbn", "bad😀"),
            (VALUE, "föö", "Other😀"),
        ] {
            expected.insert((reported_key.clone(), code, field), at(marker));
        }
        source.push_str(&entry);
        offset += entry.encode_utf16().count() as u32;
        log.push_str(&format!(
            "WARN - {}\nWARN - {}\nWARN - {}\nWARN - {}\nWARN - {}\n",
            name(&reported_key, "refs.data", "straße"),
            name(&reported_key, "refs.data", "strasse"),
            value(&reported_key, "refs.data", "strasse"),
            unqualified("isbn", &reported_key, "bad😀"),
            value(&reported_key, "refs.data", "föö")
        ));
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 7500);
    for item in items {
        let data = item.data.as_ref().unwrap();
        let NumberOrString::String(code) = item.code.as_ref().unwrap() else {
            unreachable!()
        };
        let field = data["field"].as_str().unwrap();
        assert_eq!(
            item.range,
            expected[&(
                data["key"].as_str().unwrap().to_string(),
                code.as_str(),
                field
            )]
        );
        assert_eq!(
            data.get("unresolvedField").is_some(),
            code == VALUE && field == "strasse"
        );
    }
}

#[test]
fn special_filenames_and_bibliography_roles_are_taken_from_complete_native_records() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for file in [
        "author's refs,part.data",
        "refs (part): Field.data",
        "refs): Field 'fake.bib",
        "WARN - ERROR - refs.data",
        "refs.bst",
        "refs.tex",
        "references",
    ] {
        let source = "@book{entry.bib,wrong={Value}}";
        let log = format!(
            "{}WARN - {}\nWARN - {}\n",
            found(file),
            name("entry.bib", file, "wrong"),
            value("entry.bib", file, "wrong")
        );
        let diagnostics = parse(
            root,
            &[
                (file, source),
                ("fake.bib", "@book{entry.bib,wrong={Decoy}}"),
            ],
            &log,
        );
        let items = items(&diagnostics, &uri(root, file));
        assert_eq!(items.len(), 2, "{file}: {diagnostics:?}");
        assert!(items.iter().any(|item| item.range == span(source, "wrong")));
        assert!(items.iter().any(|item| item.range == span(source, "Value")));
    }
}

#[test]
fn missing_unconfirmed_trimmed_and_oversized_sources_keep_the_root_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("large.bib"), " ".repeat(2 * 1024 * 1024 + 1)).unwrap();
    std::fs::write(root.join("binary.bib"), [0xff, 0xfe]).unwrap();
    for file in [
        "missing.bib",
        "unconfirmed.data",
        " refs.bib",
        "refs.bib ",
        "\"refs.bib\"",
        "large.bib",
        "binary.bib",
    ] {
        let diagnostics = parse(
            root,
            &[
                ("refs.bib", "@book{alpha,wrong={Decoy}}"),
                ("unconfirmed.data", "@book{alpha,wrong={Other}}"),
            ],
            &format!(
                "{}WARN - {}\n",
                found("refs.bib"),
                name("alpha", file, "wrong")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
    }
}

#[test]
fn ambiguous_native_date_boundaries_never_select_a_filename_fragment() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let file = "refs): Invalid format 'fragment.bib";
    let message =
        format!("Entry 'alpha' ({file}): Invalid format 'bad' of date field 'date' - ignoring");
    let diagnostics = parse(
        root,
        &[
            (file, "@book{alpha,date={bad}}"),
            ("refs", "@book{alpha,date={Decoy}}"),
        ],
        &format!("{}WARN - {message}\n", found(file)),
    );
    let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
    assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
    let message = "Entry 'alpha' (refs.bib): Invalid format 'text): Invalid format 'bad' of date field 'date' - ignoring";
    let diagnostics = parse(
        root,
        &[
            ("refs.bib", "@book{alpha,date={bad}}"),
            (
                "refs.bib): Invalid format 'text",
                "@book{alpha,date={Decoy}}",
            ),
        ],
        &format!("WARN - {message}\n"),
    );
    assert_eq!(
        items(&diagnostics, &uri(root, "main.tex"))[0]
            .data
            .as_ref()
            .unwrap()["unresolvedSource"],
        true
    );
}

#[test]
fn record_levels_and_complete_templates_control_field_diagnostic_recognition() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{alpha,wrong={Value}}";
    for header in ["", "[0] ", "[0] Biber.pm:130> "] {
        for (level, severity) in [
            ("WARN", DiagnosticSeverity::WARNING),
            ("ERROR", DiagnosticSeverity::ERROR),
        ] {
            let diagnostics = parse(
                root,
                &[("refs.bib", source)],
                &format!("{header}{level} - {}\n", name("alpha", "refs.bib", "wrong")),
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
                &format!("{header}{level} - {}\n", name("alpha", "refs.bib", "wrong"))
            )
            .is_empty());
        }
    }
    for message in [
        "Datamodel: Entry 'alpha' (refs.bib): Missing mandatory field 'wrong'",
        "Datamodel: Entry 'alpha' (refs.bib): Field 'wrong' invalid in data model - ignoring trailing text",
        "Datamodel: Entry 'alpha' (refs.bib): Invalid value of field 'wrong' must be '<=NaN' - ignoring field",
        "Datamodel: Entry 'alpha' (refs.bib): Invalid value of field 'wrong' must be datatype 'integer' - ignoring field trailing text",
        "Entry 'alpha' (refs.bib): Invalid format 'bad' of date field 'wrong'",
        "Example: Datamodel: Entry 'alpha' (refs.bib): Field 'wrong' invalid in data model - ignoring",
    ] {
        let diagnostics = parse(root, &[("refs.bib", source)], &format!("WARN - {message}\n"));
        assert!(items(&diagnostics, &uri(root, "refs.bib")).is_empty(), "{message}: {diagnostics:?}");
    }
    for prefix in [" : WARN - ", "Warning--"] {
        let diagnostics = parse(
            root,
            &[("refs.bib", source)],
            &format!("{prefix}{}\n", name("alpha", "refs.bib", "wrong")),
        );
        assert!(diagnostics
            .values()
            .flatten()
            .all(|item| item.code != Some(NumberOrString::String(NAME.into()))));
    }
}

#[test]
fn field_location_caches_refresh_between_logs_without_proposing_deletions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let log = format!(
        "{}WARN - {}\n",
        found("refs.data"),
        name("alpha", "refs.data", "wrong")
    );
    for source in [
        "@book{alpha,wrong={First}}",
        "% 😀\r\n@book{alpha,title={😀},\r\n wrong={Moved}}\r\n",
    ] {
        let diagnostics = parse(root, &[("refs.data", source)], &log);
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(source, "wrong"));
        let mut workspace = Workspace {
            code_action_literals: true,
            ..Default::default()
        };
        workspace.add_root(root.into());
        workspace
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
        &[("refs.data", "@book{alpha,title={Repaired}}")],
        &found("refs.data")
    )
    .is_empty());
}

#[test]
#[cfg(unix)]
fn field_warnings_use_canonical_database_aliases_and_literal_key_names() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{O'Connor.bib,wrong={Value}}";
    std::fs::write(root.join("payload.tex"), source).unwrap();
    std::os::unix::fs::symlink(root.join("payload.tex"), root.join("refs.data")).unwrap();
    let diagnostics = parse(
        root,
        &[],
        &format!(
            "{}WARN - {}\n",
            found("refs.data"),
            name("O'Connor.bib", "payload.tex", "wrong")
        ),
    );
    assert_eq!(
        items(&diagnostics, &uri(root, "payload.tex"))[0].range,
        span(source, "wrong")
    );
}

#[test]
fn thousands_of_field_warnings_keep_exact_utf16_ranges_on_one_line() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = String::new();
    let mut log = found("refs.data");
    let mut column = 0;
    let mut expected = BTreeMap::new();
    for index in 0..3000 {
        let key = format!("key{index}😀");
        let entry = format!("@book{{{key},wrong={{😀 Bad}}}} ");
        let name_start = column + 6 + key.encode_utf16().count() as u32 + 1;
        let value_start = name_start + 7;
        expected.insert(
            key.clone(),
            (
                Range::new(
                    Position::new(0, name_start),
                    Position::new(0, name_start + 5),
                ),
                Range::new(
                    Position::new(0, value_start),
                    Position::new(0, value_start + 6),
                ),
            ),
        );
        column += entry.encode_utf16().count() as u32;
        source.push_str(&entry);
        log.push_str(&format!(
            "WARN - {}\nWARN - {}\n",
            name(&key, "refs.data", "wrong"),
            value(&key, "refs.data", "wrong")
        ));
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 6000);
    for item in items {
        let (name, value) = expected[item.data.as_ref().unwrap()["key"].as_str().unwrap()];
        assert_eq!(
            item.range,
            if item.code == Some(NumberOrString::String(NAME.into())) {
                name
            } else {
                value
            }
        );
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_field_validation_ranges_move_and_clear_after_repair() {
    let cases = [
        ("book", "wrong", "Unknown", "note", "Kept", NAME),
        ("book", "volume", "invalid", "volume", "12", VALUE),
        ("book", "isbn", "invalid", "isbn", "9780306406157", VALUE),
        ("article", "issn", "invalid", "issn", "0378-5955", VALUE),
        ("book", "issn", "invalid", "note", "Kept", NAME),
        ("book", "ismn", "invalid", "ismn", "M230671187", VALUE),
        ("book", "date", "2026-99-33", "date", "2026-10-23", VALUE),
        ("book", "date", "2026/invalid", "date", "2026/2027", VALUE),
        ("book", "year", "invalid", "year", "2026", VALUE),
    ];
    for (index, (kind, field, invalid, repaired_field, repaired_value, code)) in
        cases.into_iter().enumerate()
    {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let file = if index % 2 == 0 {
            "refs.data"
        } else {
            "author's refs,part.data"
        };
        for stage in 0..3 {
            let (field, value) = if stage == 2 {
                (repaired_field, repaired_value)
            } else {
                (field, invalid)
            };
            let extra = if field == "year" || field == "date" {
                ""
            } else {
                ",year={2026}"
            };
            let source = format!("{}% 😀\r\n@{kind}{{café😀,title={{Title}},author={{Doe, Jane}}{extra},\r\n {field}={{{value}}}}}\r\n", if stage == 1 { "\r\n" } else { "" });
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
            assert_eq!(std::fs::read_to_string(root.join(file)).unwrap(), source);
            let log = std::fs::read_to_string(root.join(format!("{file}.blg"))).unwrap();
            let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
            let items = items(&diagnostics, &uri(root, file));
            if stage == 2 {
                assert!(items.is_empty(), "{source}\n{log}\n{diagnostics:?}");
                continue;
            }
            let item = items
                .iter()
                .find(|item| item.code == Some(NumberOrString::String(code.into())))
                .unwrap_or_else(|| panic!("{source}\n{log}\n{diagnostics:?}"));
            assert_eq!(
                item.range,
                span(&source, if code == NAME { field } else { value }),
                "{log}"
            );
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert!(
                item.data.as_ref().unwrap().get("unresolvedField").is_none(),
                "{log}"
            );
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_model_field_aliases_move_and_clear_with_custom_models() {
    for (raw, model, reported) in [
        ("FÖÖ", "föö", "föö"),
        ("FÖÖ", "föö", "föö"),
        ("Straße", "strasse", "strasse"),
        ("STRAẞE", "strasse", "strasse"),
        ("ΟΣ", "οσ", "οσ"),
        ("Ος", "οσ", "οσ"),
        ("İTYPE", "i̇type", "i̇type"),
        ("ıTYPE", "ıtype", "ıtype"),
        ("ﬃTYPE", "ffitype", "ffitype"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let config = format!("<config><datamodel><fields><field fieldtype=\"field\" datatype=\"integer\">{model}</field></fields><entryfields><entrytype>book</entrytype><field>{model}</field></entryfields></datamodel></config>");
        std::fs::write(root.join("model.conf"), config).unwrap();
        for stage in 0..3 {
            let value = if stage == 2 { "12" } else { "invalid😀" };
            let source = format!("% 😀\r\n@book{{café😀,title={{Title}},author={{Doe, Jane}},date={{2026}},{}{raw}={{{value}}}}}\r\n", if stage == 0 { "" } else { "\r\n " });
            std::fs::write(root.join("refs.data"), &source).unwrap();
            let output = std::process::Command::new("biber")
                .args([
                    "--tool",
                    "--configfile=model.conf",
                    "--validate-datamodel",
                    "--output-file=processed.bib",
                    "refs.data",
                ])
                .current_dir(root)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                std::fs::read_to_string(root.join("refs.data")).unwrap(),
                source
            );
            let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
            let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
            let items = items(&diagnostics, &uri(root, "refs.data"));
            if stage == 2 {
                assert!(items.is_empty(), "{source}\n{log}\n{diagnostics:?}");
            } else {
                assert_eq!(items.len(), 1, "{source}\n{log}\n{diagnostics:?}");
                assert_eq!(items[0].range, span(&source, value), "{source}\n{log}");
                assert_eq!(items[0].code, Some(NumberOrString::String(VALUE.into())));
                assert_eq!(
                    items[0].data,
                    Some(serde_json::json!({"key":"café😀","field":reported}))
                );
            }
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_model_collisions_and_input_spellings_remain_independent() {
    for (assignments, config, expected) in [
        (
            "STRASSE={12},Straße={invalid😀}",
            Some("book"),
            vec![(VALUE, "strasse", "café😀", true)],
        ),
        (
            "Straße={First😀},STRASSE={Second😀}",
            Some("book"),
            vec![(VALUE, "strasse", "café😀", true)],
        ),
        (
            "Straße={invalid😀}",
            Some("article"),
            vec![
                (NAME, "strasse", "Straße", false),
                (VALUE, "strasse", "invalid😀", false),
            ],
        ),
        (
            "Straße={First😀},STRASSE={Second😀}",
            None,
            vec![
                (NAME, "straße", "Straße", false),
                (NAME, "strasse", "STRASSE", false),
            ],
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        if let Some(kind) = config {
            std::fs::write(root.join("model.conf"), format!("<config><datamodel><fields><field fieldtype=\"field\" datatype=\"integer\">strasse</field></fields><entryfields><entrytype>{kind}</entrytype><field>strasse</field></entryfields></datamodel></config>")).unwrap();
        }
        let source = format!("% 😀\r\n@book{{café😀,title={{Title}},author={{Doe, Jane}},date={{2026}},{assignments}}}\r\n");
        std::fs::write(root.join("refs.data"), &source).unwrap();
        let mut command = std::process::Command::new("biber");
        command.args([
            "--tool",
            "--validate-datamodel",
            "--output-file=processed.bib",
        ]);
        if config.is_some() {
            command.arg("--configfile=model.conf");
        }
        let output = command.arg("refs.data").current_dir(root).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
        let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let items = items(&diagnostics, &uri(root, "refs.data"));
        assert_eq!(
            items.len(),
            expected.len(),
            "{source}\n{log}\n{diagnostics:?}"
        );
        for (code, field, marker, unresolved) in expected {
            let item = items
                .iter()
                .find(|item| {
                    item.code == Some(NumberOrString::String(code.into()))
                        && item.data.as_ref().unwrap()["field"] == field
                })
                .unwrap();
            assert_eq!(
                item.range,
                span(&source, marker),
                "{source}\n{log}\n{item:?}"
            );
            assert_eq!(
                item.data.as_ref().unwrap().get("unresolvedField").is_some(),
                unresolved
            );
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_default_isbn_aliases_move_collide_and_clear_without_validation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for stage in 0..4 {
        let prefix = if stage == 2 {
            "ISBN={9780306406157},"
        } else {
            ""
        };
        let value = if stage == 3 {
            "9780306406157"
        } else {
            "invalid😀"
        };
        let source = format!("% 😀\r\n@book{{café😀,title={{Title}},author={{Doe, Jane}},date={{2026}},{prefix}{}IſBN={{{value}}}}}\r\n", if stage == 0 { "" } else { "\r\n " });
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
        if stage == 3 {
            assert!(items.is_empty(), "{source}\n{log}\n{diagnostics:?}");
            continue;
        }
        assert_eq!(items.len(), 1, "{source}\n{log}\n{diagnostics:?}");
        assert_eq!(
            items[0].range,
            span(&source, if stage == 2 { "café😀" } else { value }),
            "{log}"
        );
        assert_eq!(
            items[0]
                .data
                .as_ref()
                .unwrap()
                .get("unresolvedField")
                .is_some(),
            stage == 2
        );
        assert_eq!(items[0].data.as_ref().unwrap()["field"], "isbn");
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_unicode_normalization_preserves_unique_fields_and_refuses_collisions() {
    for (source, expected, unresolved) in [
        ("@book{café😀,FÖÖ={Value}}\n", "FÖÖ", ""),
        ("@book{café😀,FÖÖ={Value}}\n", "FÖÖ", ""),
        (
            "@book{café😀,FÖÖ={First},FÖÖ={Second}}\n",
            "café😀",
            "unresolvedField",
        ),
        (
            "% Header\n@book{café😀,wrong={First}}\n@book{café😀,wrong={Second}}\n",
            "% Header",
            "unresolvedEntry",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("refs.bib"), source).unwrap();
        let output = std::process::Command::new("biber")
            .args([
                "--tool",
                "--validate-datamodel",
                "--output-file=processed.bib",
                "refs.bib",
            ])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let log = std::fs::read_to_string(root.join("refs.bib.blg")).unwrap();
        let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let items = items(&diagnostics, &uri(root, "refs.bib"));
        assert!(!items.is_empty(), "{log}\n{diagnostics:?}");
        for item in items {
            assert_eq!(
                item.range,
                span(source, expected),
                "{source}\n{log}\n{item:?}"
            );
            if !unresolved.is_empty() {
                assert_eq!(item.data.as_ref().unwrap()[unresolved], true);
            }
        }
    }
}
