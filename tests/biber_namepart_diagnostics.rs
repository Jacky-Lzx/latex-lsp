use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const CODE: &str = "invalid-bibliography-namepart";
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn found(name: &str) -> String {
    format!("INFO - Found BibTeX data source '{name}'\n")
}
fn warning(key: &str, field: &str, namepart: &str) -> String {
    format!("Invalid namepart '{namepart}' found in extended name format name '{field}' in entry '{key}', ignoring")
}
fn parse(root: &Path, files: &[(&str, &str)], log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    for (name, source) in files {
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
fn extended_name_warnings_preserve_parts_and_locate_the_entire_model_field_value() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (field, reported) in [
        ("AUTHOR", "author"),
        ("TRANſLATOR", "translator"),
        ("CUSTOMNAME", "customname"),
    ] {
        for (value, marker) in [
            (
                "{family=Doe, given=Jane, Bogus=Extra}",
                "family=Doe, given=Jane, Bogus=Extra",
            ),
            (
                "\"family=Doe, given=Jane, Bogus=Extra\"",
                "family=Doe, given=Jane, Bogus=Extra",
            ),
            ("names # { and Smith}", "names # { and Smith}"),
            ("{}", "{}"),
        ] {
            let source = format!("% 😀\r\n@book{{café😀,{field}={value}}}\r\n");
            let message = warning("café😀", reported, "bogus");
            let diagnostics = parse(
                root,
                &[("refs.data", &source)],
                &format!(
                    "{}[42] Utils.pm:395> WARN - {message}\n",
                    found("refs.data")
                ),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(item.range, span(&source, marker));
            assert_eq!(item.message, format!("Biber: {message}"));
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(item.source.as_deref(), Some("latex-build"));
            assert_eq!(
                item.data,
                Some(serde_json::json!({"key":"café😀","field":reported,"namepart":"bogus"}))
            );
        }
    }
}

#[test]
fn mapped_duplicate_model_colliding_and_incomplete_fields_use_the_unique_owner_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for body in [
        "note={family=Doe, bogus=Extra}}",
        "translator={family=Doe},TRANSLATOR={family=Smith}}",
        "TRANSLATOR={family=Doe},TRANſLATOR={family=Smith}}",
        "translator=}",
        "translator={family=Doe}",
    ] {
        let source = format!("% 😀\r\n@book{{café😀,{body}");
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                warning("café😀", "translator", "bogus")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(&source, "café😀"));
        assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
    }
}

#[test]
fn unknown_history_unreadable_sources_and_normalized_owner_collisions_keep_root_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,author={family=Doe, bogus=Extra}}";
    let message = format!("WARN - {}\n", warning("café😀", "author", "bogus"));
    for log in [
        message.clone(),
        format!("{message}{}", found("refs.data")),
        format!("{}{}{message}", found("refs.data"), found("missing.data")),
        format!("{}{}{message}", found("refs.data"), found("duplicate.data")),
        format!("{}ERROR - Unknown failure\n{message}", found("refs.data")),
        format!(
            "{}[latex-lsp: middle of compiler output omitted]\n{message}",
            found("refs.data")
        ),
    ] {
        let diagnostics = parse(
            root,
            &[
                ("refs.data", source),
                ("duplicate.data", "@book{café😀,author={family=Smith}}"),
            ],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(
            item.data.as_ref().unwrap()["unresolvedSource"],
            true,
            "{log}"
        );
    }
    let source = format!("{source} @book{{café😀,author={{family=Smith}}}}");
    let diagnostics = parse(
        root,
        &[("refs.data", &source)],
        &format!("{}{message}", found("refs.data")),
    );
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
}

#[test]
fn nameparts_and_keys_preserve_punctuation_while_fixed_suffixes_delimit_fields() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{O'Connor😀,author={family=Doe}}";
    for namepart in [
        "bad'part",
        "namé😀",
        "bad field",
        "x' found in extended name format name 'fake' in entry 'bogus",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                warning("O'Connor😀", "author", namepart)
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(source, "family=Doe"));
        assert_eq!(item.data.as_ref().unwrap()["namepart"], namepart);
    }
}

#[test]
fn namepart_locations_use_only_prior_case_sensitive_distinct_sources() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{owner,author={family=Doe}} @book{OWNER,author={family=Other}}";
    let message = warning("owner", "author", "bogus");
    let diagnostics = parse(
        root,
        &[("refs.data", source), ("future.data", "@book{owner,}")],
        &format!(
            "{}{}WARN - {message}\n{}",
            found("refs.data"),
            found("./refs.data"),
            found("future.data")
        ),
    );
    assert_eq!(
        items(&diagnostics, &uri(root, "refs.data"))[0].range,
        span(source, "family=Doe")
    );
}

#[test]
fn only_complete_extended_name_warning_templates_are_classified() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for message in [
        warning("owner", "author", ""),
        warning("bad key", "author", "bogus"),
        warning("owner", "bad field", "bogus"),
        format!("{} trailing", warning("owner", "author", "bogus")),
        warning("owner", "author", "bogus")
            .trim_end_matches("', ignoring")
            .into(),
        "Invalid namepart 'bogus' found in name 'author' in entry 'owner', ignoring".into(),
    ] {
        let diagnostics = parse(root, &[], &format!("WARN - {message}\n"));
        assert!(
            diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{message}"
        );
    }
    let diagnostics = parse(
        root,
        &[("refs.data", "@book{owner,author={family=Doe}}")],
        &format!(
            "{}INFO - {}\n",
            found("refs.data"),
            warning("owner", "author", "bogus")
        ),
    );
    assert!(diagnostics
        .values()
        .flatten()
        .all(|item| item.code != Some(NumberOrString::String(CODE.into()))));
}

#[test]
fn oversized_history_inputs_do_not_leave_partial_namepart_field_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let oversized = format!("%{}\n", "x".repeat(2 * 1024 * 1024));
    let diagnostics = parse(
        root,
        &[
            ("refs.data", "@book{owner,author={family=Doe}}"),
            ("large.data", &oversized),
        ],
        &format!(
            "{}{}WARN - {}\n",
            found("refs.data"),
            found("large.data"),
            warning("owner", "author", "bogus")
        ),
    );
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
}

#[test]
fn unknown_nameparts_offer_no_guessed_model_changes_or_name_rewrites() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let diagnostics = parse(
        root,
        &[("refs.data", "@book{owner,author={family=Doe, bogus=Extra}}")],
        &format!(
            "{}WARN - {}\n",
            found("refs.data"),
            warning("owner", "author", "bogus")
        ),
    );
    let target = uri(root, "refs.data");
    let item = &items(&diagnostics, &target)[0];
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    let context = CodeActionContext {
        diagnostics: vec![item.clone()],
        only: Some(vec![CodeActionKind::QUICKFIX]),
        trigger_kind: None,
    };
    assert!(editing::code_actions(&workspace, &target, item.range, &context).is_empty());
}

fn native(
    root: &Path,
    source: &str,
    config: Option<&str>,
) -> (String, BTreeMap<Url, Vec<Diagnostic>>) {
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    std::fs::write(root.join("refs.data"), source).unwrap();
    let mut command = std::process::Command::new("biber");
    command.args(["--tool", "--output-file=processed.bib"]);
    if let Some(config) = config {
        std::fs::write(root.join("mapping.conf"), config).unwrap();
        command.arg("--configfile=mapping.conf");
    }
    let output = command.arg("refs.data").current_dir(root).output().unwrap();
    let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
    assert!(output.status.success(), "{log}");
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
    (log, diagnostics)
}

#[test]
#[ignore = "requires biber"]
fn real_biber_extended_nameparts_use_raw_unicode_field_values_move_and_clear_after_repair() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (field, reported) in [("AUTHOR", "author"), ("TRANſLATOR", "translator")] {
        let body = "family=Doe, given=Jane, NaMé😀=Extra";
        let source = format!("% 😀\r\n@book{{café😀,{field}={{{body}}}}}");
        for current in [source.to_string(), format!("% moved 😀\r\n{source}")] {
            let (log, diagnostics) = native(root, &current, None);
            let matches = items(&diagnostics, &uri(root, "refs.data"));
            assert_eq!(matches.len(), 1, "{log}");
            assert_eq!(matches[0].range, span(&current, body));
            assert_eq!(
                matches[0].data,
                Some(serde_json::json!({"key":"café😀","field":reported,"namepart":"namé😀"}))
            );
        }
        let repaired = source.replace(", NaMé😀=Extra", "");
        let (_, diagnostics) = native(root, &repaired, None);
        assert!(diagnostics
            .values()
            .flatten()
            .all(|item| item.code != Some(NumberOrString::String(CODE.into()))));
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_macro_mapped_and_model_colliding_name_fields_keep_proven_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@string{names={family=Doe, given=Jane, bogus=Extra}} @book{owner,AUTHOR=names}";
    let (log, diagnostics) = native(root, source, None);
    let matches = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(matches.len(), 1, "{log}");
    let start = source.rfind("names}").unwrap();
    assert_eq!(
        matches[0].range,
        Text::new(source.into()).range(start, start + 5)
    );
    let source =
        "@book{owner,TRANSLATOR={family=Doe, bogus=Extra},TRANſLATOR={family=Smith, unknown=Data}}";
    let (log, diagnostics) = native(root, source, None);
    let matches = items(&diagnostics, &uri(root, "refs.data"));
    assert!(!matches.is_empty(), "{log}");
    for item in matches {
        assert_eq!(item.range, span(source, "owner"));
        assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
    }
    let mapping = r#"<config><sourcemap><maps datatype="bibtex"><map><map_step map_field_source="note" map_field_target="author"/></map></maps></sourcemap></config>"#;
    let source = "@book{owner,note={family=Doe, given=Jane, bogus=Extra}}";
    let (log, diagnostics) = native(root, source, Some(mapping));
    let matches = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(matches.len(), 1, "{log}");
    assert_eq!(matches[0].range, span(source, "owner"));
    assert_eq!(matches[0].data.as_ref().unwrap()["unresolvedField"], true);
}
