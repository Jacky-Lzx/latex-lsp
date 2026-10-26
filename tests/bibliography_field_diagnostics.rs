use lsp_types::*;
use std::{collections::BTreeMap, path::Path};
use texquill::{compilation::build_log, text::Text};

const CODE: &str = "repeated-bibliography-field";

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn warning(key: &str, field: &str, line: u32, file: &str) -> String {
    format!("Warning--I'm ignoring {key}'s extra \"{field}\" field\n--line {line} of file {file}\n")
}
fn parse(root: &Path, source: &str, log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    std::fs::write(root.join("refs.bib"), source).unwrap();
    build_log::parse_bibliography(&root.join("main.tex"), log)
}
fn fields(found: &BTreeMap<Url, Vec<Diagnostic>>, uri: &Url) -> Vec<Diagnostic> {
    found
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
fn duplicate_fields_use_name_ranges_and_link_first_assignments_with_utf16_and_quoted_keys() {
    let source = "% 😀\r\n@book{entry\"key'x😀,title={First}, TiTlE={中😀}}\r\n";
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let log = format!(
        "Database file #1: refs.bib\n{}",
        warning("ENTRY\"KEY'X😀", "title", 2, "refs.bib")
    );
    let found = parse(root, source, &log);
    assert_eq!(found.len(), 1);
    let items = fields(&found, &uri(root, "refs.bib"));
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.range, span(source, "TiTlE"));
    assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
    assert_eq!(item.source.as_deref(), Some("latex-build"));
    assert_eq!(
        item.message,
        "BibTeX: I'm ignoring ENTRY\"KEY'X😀's extra \"title\" field"
    );
    assert_eq!(item.data.as_ref().unwrap()["field"], "title");
    let related = item.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(
        related[0].location,
        Location::new(uri(root, "refs.bib"), span(source, "title"))
    );
}

#[test]
fn multiline_values_and_delimiters_report_the_original_field_name() {
    for (source, line) in [
        (
            "@book{entry,\n title={First},\n TiTlE\n =\n {Second\n line}}\n",
            6,
        ),
        (
            "@book(entry,title={First},\n TiTlE={Second}\n , note={Note})\n",
            3,
        ),
        ("@book{entry,title={First},\n TiTlE={Second}\n }\n", 3),
        (
            "@book{entry,title={First},\n TiTlE={Second} #\n {Third}}\n",
            3,
        ),
        ("@book{entry,title={},TiTlE={Second}}\n", 1),
        (
            "@book{entry,title={First},TiTlE={Second},\n note={unfinished",
            1,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(root, source, &warning("entry", "title", line, "refs.bib"));
        let items = fields(&found, &uri(root, "refs.bib"));
        assert_eq!(items[0].range, span(source, "TiTlE"), "{source}");
        assert!(items[0].related_information.is_some());
    }
}

#[test]
fn complete_same_line_warning_groups_follow_duplicate_source_order() {
    let source = "@book{entry,title={First},TiTlE={Second},TITLE={Third}}\n";
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let log = format!(
        "Database file #1: refs.bib\n{}{}",
        warning("entry", "title", 1, "refs.bib"),
        warning("entry", "title", 1, "refs.bib")
    );
    let found = parse(root, source, &log);
    let items = fields(&found, &uri(root, "refs.bib"));
    assert_eq!(
        items.iter().map(|item| item.range).collect::<Vec<_>>(),
        [span(source, "TiTlE"), span(source, "TITLE")]
    );
    for item in items {
        assert_eq!(
            item.related_information.unwrap()[0].location.range,
            span(source, "title")
        );
    }
}

#[test]
fn large_same_line_warning_groups_preserve_all_utf16_field_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = format!(
        "@book{{entry,title={{First}},{}}}\n",
        "TITLE={中😀},".repeat(3000)
    );
    let log = format!(
        "Database file #1: refs.bib\n{}",
        warning("entry", "title", 1, "refs.bib").repeat(3000)
    );
    let found = parse(root, &source, &log);
    let items = fields(&found, &uri(root, "refs.bib"));
    assert_eq!(items.len(), 3000);
    let first = "@book{entry,title={First},".encode_utf16().count() as u32;
    let stride = "TITLE={中😀},".encode_utf16().count() as u32;
    let earlier = span(&source, "title");
    for (i, item) in items.iter().enumerate() {
        let column = first + i as u32 * stride;
        assert_eq!(
            item.range,
            Range::new(Position::new(0, column), Position::new(0, column + 5))
        );
        assert_eq!(
            item.related_information.as_ref().unwrap()[0].location.range,
            earlier
        );
    }
}

#[test]
fn cached_group_positions_cross_multiline_values_and_keep_unicode_name_widths() {
    let source = "% 😀\r\n@book(entry,custom😀={First},\r\n CUSTOM😀={Second\r\n line},Custom😀={Third})\r\n";
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let log = format!(
        "Database file #1: refs.bib\n{}{}",
        warning("ENTRY", "CUSTOM😀", 4, "refs.bib"),
        warning("entry", "custom😀", 4, "refs.bib")
    );
    let found = parse(root, source, &log);
    let items = fields(&found, &uri(root, "refs.bib"));
    assert_eq!(items.len(), 2);
    for (item, marker, key, field) in [
        (&items[0], "CUSTOM😀", "ENTRY", "CUSTOM😀"),
        (&items[1], "Custom😀", "entry", "custom😀"),
    ] {
        assert_eq!(item.range, span(source, marker));
        assert_eq!(item.data.as_ref().unwrap()["key"], key);
        assert_eq!(item.data.as_ref().unwrap()["field"], field);
        let related = &item.related_information.as_ref().unwrap()[0];
        assert_eq!(related.location.range, span(source, "custom😀"));
        assert!(related.message.contains("`custom😀`"));
    }
}

#[test]
fn cached_groups_recheck_input_completeness_after_intervening_errors() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{entry,title={First},TiTlE={Second},TITLE={Third}}\r\n";
    let log = format!(
        "Database file #1: refs.bib\n{}Illegal end---line 1 of file refs.bib\n{}",
        warning("entry", "title", 1, "refs.bib"),
        warning("entry", "title", 1, "refs.bib")
    );
    let found = parse(root, source, &log);
    let items = fields(&found, &uri(root, "refs.bib"));
    assert_eq!(items.len(), 2);
    let precise = items
        .iter()
        .find(|item| item.related_information.is_some())
        .unwrap();
    assert_eq!(precise.range, span(source, "TiTlE"));
    let fallback = items
        .iter()
        .find(|item| item.related_information.is_none())
        .unwrap();
    assert_eq!(fallback.data.as_ref().unwrap()["unresolvedField"], true);
    assert_eq!(
        fallback.range,
        Text::new(source.into()).range(0, source.trim_end().len())
    );
}

#[test]
fn cached_groups_keep_source_databases_and_independent_line_groups_separate() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = "@book{entry,title={First},TiTlE={Second},TITLE={Third}}\n";
    let second = "% 😀\r\n@book{entry,title={Other},tItLe={中😀},TITLE={Last}}\r\n";
    std::fs::write(root.join("other.bib"), second).unwrap();
    let log = format!(
        "Database file #1: refs.bib\nDatabase file #2: other.bib\n{}{}{}{}",
        warning("entry", "title", 1, "refs.bib"),
        warning("entry", "title", 2, "other.bib"),
        warning("entry", "title", 1, "refs.bib"),
        warning("entry", "title", 2, "other.bib")
    );
    let found = parse(root, first, &log);
    for (name, source, marker) in [("refs.bib", first, "TiTlE"), ("other.bib", second, "tItLe")] {
        let target = uri(root, name);
        let items = fields(&found, &target);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].range, span(source, marker));
        assert_eq!(items[1].range, span(source, "TITLE"));
        for item in items {
            assert_eq!(
                item.related_information.unwrap()[0].location,
                Location::new(target.clone(), span(source, "title"))
            );
        }
    }
}

#[test]
fn incomplete_or_ambiguous_groups_do_not_choose_one_of_several_fields() {
    let source = "@book{entry,title={First},TiTlE={Second},TITLE={Third}}\r\n";
    for (prefix, repetitions) in [
        ("", 1),
        ("", 3),
        ("[texquill: middle of compiler output omitted]\n", 2),
        ("Illegal end---line 1 of file refs.bib\n", 2),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let log = format!(
            "Database file #1: refs.bib\n{prefix}{}",
            warning("entry", "title", 1, "refs.bib").repeat(repetitions)
        );
        let found = parse(root, source, &log);
        let items = fields(&found, &uri(root, "refs.bib"));
        // Unresolved identical records are deduplicated to the same line.
        assert_eq!(items.len(), 1);
        for item in items {
            assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
            assert!(item.related_information.is_none());
            assert_eq!(
                item.range.end.character as usize,
                source.trim_end().encode_utf16().count()
            );
        }
    }
}

#[test]
fn groups_beyond_the_record_limit_keep_proven_single_matches_and_ambiguous_line_fallbacks() {
    let log = format!(
        "Database file #1: refs.bib\n{}",
        warning("entry", "title", 1, "refs.bib").repeat(16_385)
    );
    for source in [
        "@book{entry,title={First},TiTlE={Second}}\r\n",
        "@book{entry,title={First},TiTlE={Second},TITLE={Third}}\r\n",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(root, source, &log);
        let items = fields(&found, &uri(root, "refs.bib"));
        assert_eq!(items.len(), 1);
        if source.contains("TITLE") {
            assert_eq!(
                items[0].range,
                Text::new(source.into()).range(0, source.trim_end().len())
            );
            assert_eq!(items[0].data.as_ref().unwrap()["unresolvedField"], true);
            assert!(items[0].related_information.is_none());
        } else {
            assert_eq!(items[0].range, span(source, "TiTlE"));
            assert_eq!(
                items[0].related_information.as_ref().unwrap()[0]
                    .location
                    .range,
                span(source, "title")
            );
        }
    }
}

#[test]
fn different_fields_and_reported_lines_have_independent_warning_groups() {
    let source="@book{entry,title={First},note={First},TiTlE={Second},NOTE={Second}}\n@book{entry,title={First},TITLE={Second}}\n";
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let log = format!(
        "{}{}{}",
        warning("ENTRY", "title", 1, "refs.bib"),
        warning("ENTRY", "note", 1, "refs.bib"),
        warning("ENTRY", "title", 2, "refs.bib")
    );
    let found = parse(root, source, &log);
    let items = fields(&found, &uri(root, "refs.bib"));
    assert_eq!(
        items.iter().map(|item| item.range).collect::<Vec<_>>(),
        [
            span(source, "TiTlE"),
            span(source, "NOTE"),
            span(source, "TITLE")
        ]
    );
}

#[test]
fn source_echoes_literals_special_constructs_and_other_entries_cannot_supply_a_field() {
    for source in [
        "@book{other,title={First},title={Second}}",
        "@book{entry,title={First}}",
        "% @book{entry,title={First},title={Second}}",
        "@comment{@book{entry,title={First},title={Second}}}",
        "@preamble{\"@book{entry,title={First},title={Second}}\"}",
        "@book{entry,note={title={First},title={Second}}}",
        "@book{entry,title={First},title=}",
        "@book{entry,title=,title={Second}}",
        "@book{entry,title={First},note=,title={Second}}",
        "@book{entry,title={First},title={unfinished",
        "@book{entry,title={First},title={Second}} @book{ENTRY,title={First},title={Second}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(root, source, &warning("entry", "title", 1, "refs.bib"));
        let items = fields(&found, &uri(root, "refs.bib"));
        assert!(items[0].related_information.is_none(), "{source}");
        assert_eq!(
            items[0].data.as_ref().unwrap()["unresolvedField"],
            true,
            "{source}"
        );
    }
}

#[test]
fn malformed_warning_records_and_other_tools_do_not_gain_native_field_codes() {
    for line in [
        "Example Warning--I'm ignoring entry's extra \"title\" field",
        "Warning--I'm ignoring entry's extra \"title\" field (example)",
        "Warning--I'm ignoring entry with spaces's extra \"title\" field",
        "Warning--I'm ignoring entry's extra \"123field\" field",
        "Warning--I'm ignoring entry's extra \"ti\"tle\" field",
        "WARN - I'm ignoring entry's extra \"title\" field",
        "INFO - Warning--I'm ignoring entry's extra \"title\" field",
        " : Warning--I'm ignoring entry's extra \"title\" field",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            "@book{entry,title={First},title={Second}}",
            &format!("{line}\n--line 1 of file refs.bib\n"),
        );
        assert!(
            found
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{line}"
        );
    }
}

#[test]
fn missing_field_continuations_preserve_the_next_independent_warning() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{entry,title={First},title=Missing}";
    let log = "Database file #1: refs.bib\nWarning--I'm ignoring entry's extra \"title\" field\nWarning--string name \"Missing\" is undefined\n--line 1 of file refs.bib\n";
    let found = parse(root, source, log);
    let pending = fields(&found, &uri(root, "main.tex"));
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].data.as_ref().unwrap()["unresolvedSource"], true);
    let missing = found[&uri(root, "refs.bib")]
        .iter()
        .find(|item| {
            item.code
                == Some(NumberOrString::String(
                    "undefined-bibliography-string".into(),
                ))
        })
        .unwrap();
    assert_eq!(missing.range, span(source, "Missing"));
}

#[test]
fn missing_lines_files_and_wrong_roles_keep_fallback_diagnostics() {
    for (line, file) in [
        (0, "refs.bib"),
        (400, "refs.bib"),
        (1, "missing.bib"),
        (1, "main.aux"),
        (1, "style.bst"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for path in ["main.aux", "style.bst"] {
            std::fs::write(root.join(path), "@book{entry,title={First},title={Second}}").unwrap();
        }
        let found = parse(
            root,
            "@book{entry,title={First},title={Second}}",
            &warning("entry", "title", line, file),
        );
        let target = if file == "refs.bib" {
            "refs.bib"
        } else {
            "main.tex"
        };
        let items = fields(&found, &uri(root, target));
        assert_eq!(items.len(), 1);
        assert!(items[0].related_information.is_none());
        assert_eq!(
            items[0].data.as_ref().unwrap()[if file == "refs.bib" {
                "unresolvedLine"
            } else {
                "unresolvedSource"
            }],
            true
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let found = parse(
        root,
        "@book{entry,title={First},title={Second}}",
        "Warning--I'm ignoring entry's extra \"title\" field\n",
    );
    assert_eq!(
        fields(&found, &uri(root, "main.tex"))[0]
            .data
            .as_ref()
            .unwrap()["unresolvedSource"],
        true
    );
}

#[test]
#[cfg(unix)]
fn database_aliases_keep_physical_source_and_related_field_locations() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{entry,custom:field={First},custom:FIELD={Second}}";
    std::fs::write(root.join("physical.data"), source).unwrap();
    symlink("physical.data", root.join("alias.bib")).unwrap();
    let found = parse(
        root,
        "",
        &format!(
            "Database file #1: alias.bib\n{}",
            warning("entry", "custom:field", 1, "alias.bib")
        ),
    );
    let items = fields(&found, &uri(root, "physical.data"));
    assert_eq!(items[0].range, span(source, "custom:FIELD"));
    assert_eq!(
        items[0].related_information.as_ref().unwrap()[0]
            .location
            .uri,
        uri(root, "physical.data")
    );
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_duplicate_field_warnings_follow_value_end_lines_and_record_order() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{ENTRY}\n\\bibstyle{fixture}\n\\bibdata{refs}\n",
    )
    .unwrap();
    std::fs::write(root.join("fixture.bst"),"ENTRY {title} {} {}\nFUNCTION {book} {}\nFUNCTION {emit} {title write$ newline$}\nREAD\nITERATE {emit}\n").unwrap();
    for source in [
        "@book{entry,title={First},\n TiTlE\n =\n {Second\n line}}\n",
        "@book(entry,title={First},\n TiTlE={Second}\n , TITLE={Third}\n )\n",
        "@book{entry,title={First},TiTlE={Second},TITLE={Third}}\n",
    ] {
        parse(root, source, "");
        let output = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("main.bbl"))
                .unwrap()
                .trim(),
            "First"
        );
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let items = fields(&found, &uri(root, "refs.bib"));
        let mut expected = vec![span(source, "TiTlE")];
        if source.contains("TITLE") {
            expected.push(span(source, "TITLE"));
        }
        assert_eq!(
            items.iter().map(|item| item.range).collect::<Vec<_>>(),
            expected,
            "{log}"
        );
        assert!(items
            .iter()
            .all(
                |item| item.related_information.as_ref().unwrap()[0].location.range
                    == span(source, "title")
            ));
    }
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_large_field_groups_preserve_each_ignored_name_and_clear_after_repair() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{ENTRY}\n\\bibstyle{fixture}\n\\bibdata{refs}\n",
    )
    .unwrap();
    std::fs::write(root.join("fixture.bst"), "ENTRY {title} {} {}\nFUNCTION {book} {}\nFUNCTION {emit} {title write$ newline$}\nREAD\nITERATE {emit}\n").unwrap();
    for count in [3000, 0] {
        let prefix = "@book{entry,title={First},";
        let duplicate = "TITLE={Next},";
        let source = format!("{prefix}{}}}\n", duplicate.repeat(count));
        parse(root, &source, "");
        let output = std::process::Command::new("bibtex")
            .arg("main")
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
        assert_eq!(log.matches("'s extra \"title\" field").count(), count);
        assert_eq!(
            std::fs::read_to_string(root.join("main.bbl"))
                .unwrap()
                .trim(),
            "First"
        );
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let items = fields(&found, &uri(root, "refs.bib"));
        assert_eq!(items.len(), count);
        let earlier = span(&source, "title");
        for (i, item) in items.iter().enumerate() {
            let column = (prefix.len() + i * duplicate.len()) as u32;
            assert_eq!(
                item.range,
                Range::new(Position::new(0, column), Position::new(0, column + 5))
            );
            assert_eq!(
                item.related_information.as_ref().unwrap()[0].location.range,
                earlier
            );
        }
        if count == 0 {
            assert!(found.is_empty(), "{found:?}\n{log}");
        }
    }
}
