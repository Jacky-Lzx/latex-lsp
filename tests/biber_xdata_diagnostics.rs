use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

const CODE: &str = "invalid-bibliography-xdata";
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn found(name: &str) -> String {
    format!("INFO - Found BibTeX data source '{name}'\n")
}
fn message(key: &str, parent: &str, kind: usize, section: u32) -> String {
    match kind {
        0 => format!("I didn't find a database entry for xdata entry '{parent}' in entry '{key}' - ignoring (section {section})"),
        1 => format!("Entry '{key}' references XDATA entry '{parent}' which does not exist, not resolving (section {section})"),
        _ => format!("Entry '{key}' references XDATA entry '{parent}' which is not an XDATA entry, not resolving (section {section})"),
    }
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
fn native_xdata_templates_select_the_exact_literal_list_item_in_the_owner() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for value in ["{before, pére😀 ,after}", "\"before, pére😀 ,after\""] {
        let source = format!("% 😀\r\n@book{{café😀,crossref={{unrelated}},XDATA={value},title={{😀}}}}\r\n@book{{later,xdata={{pére😀}}}}");
        for kind in 0..3 {
            for (level, severity) in [
                ("WARN", DiagnosticSeverity::WARNING),
                ("ERROR", DiagnosticSeverity::ERROR),
            ] {
                // Unknown ERRORs invalidate history before dispatch. Native
                // templates are WARN; do not invent a recoverable ERROR rule.
                let message = message("café😀", "pére😀", kind, 7);
                let diagnostics = parse(
                    root,
                    &[("refs.data", &source)],
                    &format!(
                        "{}[1] Biber.pm:130> {level} - {message}\n",
                        found("refs.data")
                    ),
                );
                let target = if level == "WARN" {
                    "refs.data"
                } else {
                    "main.tex"
                };
                let item = &items(&diagnostics, &uri(root, target))[0];
                assert_eq!(item.severity, Some(severity));
                assert_eq!(item.source.as_deref(), Some("latex-build"));
                assert_eq!(item.message, format!("Biber: {message}"));
                if level == "WARN" {
                    assert_eq!(item.range, span(&source, "pére😀"));
                    assert_eq!(
                        item.data,
                        Some(
                            serde_json::json!({"key":"café😀","parent":"pére😀","field":"xdata","section":7,"reason":if kind==2 {"non-xdata-entry"} else {"missing-entry"}})
                        )
                    );
                } else {
                    assert_eq!(item.range, Range::default());
                    assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
                }
                assert!(item.related_information.is_none());
            }
        }
    }
}

#[test]
fn dynamic_inherited_invalid_and_ambiguous_xdata_values_fall_back_to_the_owner_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        "@string{inherit={pére😀}} @book{café😀,xdata=inherit}",
        "@book{café😀,xdata={pé} # {re😀}}",
        "@book{café😀,inherit={pére😀}}",
        "@book{café😀,crossref={ancestor}} @book{ancestor,xdata={pére😀}}",
        "@book{café😀,xdata={pére😀},XDATA={other}}",
        "@book{café😀,xdata={other},XDATA={pére😀}}",
        "@book{café😀,xdata={pére😀},XDATA=}",
        "@book{café😀,xdata={pére😀,pére😀}}",
        "@book{café😀,xdata={PÉRE😀}}",
        "@book{café😀,xdata={pére😀}",
        "@book{café😀,xdata={{pére😀}}}",
        "@book{café😀,xdata={pére😀\\relax}}",
        "@book{café😀,xdata={pére😀%comment\n}}",
        "@book{café😀,xdata={pére😀#}}",
        "@book{café😀,related={pére😀}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                message("café😀", "pére😀", 1, 99999)
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(
            item.range,
            span(source, "café😀"),
            "{source}\n{diagnostics:?}"
        );
        assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
        assert!(item
            .data
            .as_ref()
            .unwrap()
            .get("unresolvedSource")
            .is_none());
    }
}

#[test]
fn xdata_sources_require_complete_prior_inputs_and_case_sensitive_unique_normalized_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,xdata={pére😀}}";
    let record = format!("WARN - {}\n", message("café😀", "pére😀", 1, 0));
    for log in [
        record.clone(),
        format!("{record}{}", found("refs.data")),
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
                ("duplicate.data", "@book{café😀,xdata={pére😀}}"),
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
    }
    for source in [
        "@book{café😀,xdata={pére😀}} @book{café😀,xdata={pére😀}}",
        "@book{CAFÉ😀,xdata={pére😀}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}{record}", found("refs.data")),
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "main.tex")).len(),
            1,
            "{diagnostics:?}"
        );
    }
    let diagnostics = parse(
        root,
        &[
            ("refs.data", source),
            ("other.data", "@book{CAFÉ😀,xdata={pére😀}}"),
        ],
        &format!("{}{}{record}", found("refs.data"), found("other.data")),
    );
    assert_eq!(
        items(&diagnostics, &uri(root, "refs.data"))[0].range,
        span(source, "pére😀")
    );
    // Later inputs do not retroactively introduce an ambiguity.
    let diagnostics = parse(
        root,
        &[("refs.data", source), ("duplicate.data", source)],
        &format!("{}{record}{}", found("refs.data"), found("duplicate.data")),
    );
    assert_eq!(items(&diagnostics, &uri(root, "refs.data")).len(), 1);
}

#[test]
fn xdata_read_limits_and_complete_special_source_roles_preserve_conservative_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{café\"'😀,xdata={pére\"'😀}}";
    let warning = message("café\"'😀", "pére\"'😀", 1, 99999);
    for filename in ["db ' weird WARN - file.tex", "refs", "refs.png", "refs.bst"] {
        let diagnostics = parse(
            root,
            &[(filename, source)],
            &format!("{}WARN - {warning}\n", found(filename)),
        );
        let item = &items(&diagnostics, &uri(root, filename))[0];
        assert_eq!(item.range, span(source, "pére\"'😀"));
    }
    let warning = message("child", "parent", 0, 1);
    let record = format!("WARN - {warning}\n");
    let source = "@book{child,xdata={parent}}";
    let too_large = format!("{}\n{source}", " ".repeat(2 * 1024 * 1024));
    let diagnostics = parse(
        root,
        &[("refs.data", &too_large)],
        &format!("{}{record}", found("refs.data")),
    );
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
    let mut log = found("refs.data");
    let filler = " ".repeat(2 * 1024 * 1024 - 1);
    for index in 0..9 {
        let name = format!("empty{index}.data");
        std::fs::write(root.join(&name), &filler).unwrap();
        log.push_str(&found(&name));
    }
    log.push_str(&record);
    let diagnostics = parse(root, &[("refs.data", source)], &log);
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
    let mut log = found("refs.data");
    for index in 0..1024 {
        log.push_str(&found(&format!("input{index}.data")));
    }
    log.push_str(&record);
    let diagnostics = parse(root, &[("refs.data", source)], &log);
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
}

#[test]
#[cfg(unix)]
fn xdata_symbolic_source_aliases_do_not_create_false_owner_ambiguities() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{child,xdata={parent}}";
    std::fs::write(root.join("refs.data"), source).unwrap();
    std::os::unix::fs::symlink(root.join("refs.data"), root.join("alias.data")).unwrap();
    let diagnostics = parse(
        root,
        &[],
        &format!(
            "{}{}WARN - {}\n",
            found("refs.data"),
            found("alias.data"),
            message("child", "parent", 0, 1)
        ),
    );
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(source, "parent"));
    assert!(item
        .data
        .as_ref()
        .unwrap()
        .get("unresolvedSource")
        .is_none());
}

#[test]
fn malformed_sections_templates_and_nonwarning_records_do_not_receive_xdata_codes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let valid = message("child", "parent", 1, 99999);
    for body in [
        valid.replace("99999", "-1"),
        valid.replace("99999", "+1"),
        valid.replace("99999", ""),
        valid.replace("99999", "１２"),
        valid.replace("99999", "4294967296"),
        valid.replace("99999", "1 2"),
        valid.replace("'child'", "''"),
        valid.replace("'parent'", "'two keys'"),
        format!("{valid} extra"),
        format!("Example: {valid}"),
        valid.trim_end_matches(')').into(),
        valid.replace("XDATA", "crossref"),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", "@book{child,xdata={parent}}")],
            &format!("{}WARN - {body}\n", found("refs.data")),
        );
        assert!(
            diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{body}\n{diagnostics:?}"
        );
    }
    for line in [
        format!("INFO - {valid}"),
        format!("DEBUG - {valid}"),
        format!(" : WARN - {valid}"),
        format!("Warning--{valid}"),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", "@book{child,xdata={parent}}")],
            &format!("{}{line}\n", found("refs.data")),
        );
        assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
    }
}

#[test]
fn xdata_ranges_refresh_after_saved_edits_without_guessed_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for source in [
        "@book{café😀,xdata={pére😀}}",
        "% 😀\r\n@book{café😀,xdata={first,\r\n pére😀}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                message("café😀", "pére😀", 2, 1)
            ),
        );
        let item = items(&diagnostics, &uri(root, "refs.data"))[0].clone();
        assert_eq!(item.range, span(source, "pére😀"));
        workspace.open_with_language(uri(root, "refs.data"), source.into(), 7, "bibtex");
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
    let diagnostics = parse(
        root,
        &[("refs.data", "@book{café😀,xdata={pére😀}} @xdata{pére😀,}")],
        &found("refs.data"),
    );
    assert!(diagnostics
        .values()
        .flatten()
        .all(|item| item.code != Some(NumberOrString::String(CODE.into()))));
}

#[test]
fn thousands_of_xdata_list_items_share_forward_utf16_ranges_with_adjacent_field_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = String::new();
    let mut log = found("refs.data");
    let mut expected = BTreeMap::new();
    let mut expected_years = Vec::new();
    let mut offset = 0u32;
    for index in 0..1500 {
        let key = format!("café{index}😀");
        let reported = format!("café{index}😀");
        let prefix = format!("@book{{{key},crossref={{other}},xdata={{");
        let mut entry = prefix.clone();
        for n in 0..3 {
            let parent = format!("pére{index}_{n}😀");
            let reported_parent = format!("pére{index}_{n}😀");
            let start = offset + entry.encode_utf16().count() as u32;
            let range = Range::new(
                Position::new(0, start),
                Position::new(0, start + parent.encode_utf16().count() as u32),
            );
            expected.insert((reported.clone(), reported_parent.clone()), range);
            log.push_str(&format!(
                "WARN - {}\n",
                message(&reported, &reported_parent, n, 99999)
            ));
            entry.push_str(&parent);
            if n < 2 {
                entry.push_str(", ");
            }
        }
        entry.push_str("},year={");
        let start = offset + entry.encode_utf16().count() as u32;
        expected_years.push(Range::new(
            Position::new(0, start),
            Position::new(0, start + 5),
        ));
        entry.push_str("bad😀},title={😀}} ");
        log.push_str(&format!("WARN - legacy year field 'bad😀' in entry '{reported}' is not an integer - this will probably not sort properly.\n"));
        offset += entry.encode_utf16().count() as u32;
        source.push_str(&entry);
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 4500);
    for item in items {
        let data = item.data.as_ref().unwrap();
        assert_eq!(
            item.range,
            expected[&(
                data["key"].as_str().unwrap().to_string(),
                data["parent"].as_str().unwrap().to_string()
            )]
        );
        assert!(data.get("unresolvedField").is_none());
    }
    let values: Vec<_> = diagnostics[&uri(root, "refs.data")]
        .iter()
        .filter(|item| {
            item.code
                == Some(NumberOrString::String(
                    "invalid-bibliography-field-value".into(),
                ))
        })
        .collect();
    assert_eq!(values.len(), 1500);
    for (item, range) in values.iter().zip(expected_years) {
        assert_eq!(item.range, range);
    }
}

fn native(
    root: &Path,
    source: &str,
    config: Option<&str>,
) -> (String, BTreeMap<Url, Vec<Diagnostic>>) {
    std::fs::write(root.join("refs.data"), source).unwrap();
    let mut command = std::process::Command::new("biber");
    command.args([
        "--tool",
        "--output-resolve-xdata",
        "--output-file=processed.bib",
    ]);
    if let Some(config) = config {
        std::fs::write(root.join("model.conf"), config).unwrap();
        command.arg("--configfile=model.conf");
    }
    let output = command.arg("refs.data").current_dir(root).output().unwrap();
    let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
    assert!(output.status.success(), "{log}");
    assert_eq!(
        std::fs::read_to_string(root.join("refs.data")).unwrap(),
        source
    );
    let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
    (log, diagnostics)
}

#[test]
#[ignore = "requires biber"]
fn real_biber_missing_and_wrong_type_xdata_references_move_and_clear_after_repair() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for stage in 0..4 {
        let source=format!("% 😀\r\n@book{{café😀,title={{Title}},author={{Doe, Jane}},date={{2026}},xdata={{{}pére😀}}}}\r\n{}",if stage>0 {"valid,\r\n "} else {""},match stage {0=>"",1=>"@xdata{valid,publisher={Publisher}}",2=>"@xdata{valid,publisher={Publisher}} @book{pére😀,title={Parent}}",_=>"@xdata{valid,publisher={Publisher}} @xdata{pére😀,publisher={Parent}}"});
        let (log, diagnostics) = native(root, &source, None);
        let items = items(&diagnostics, &uri(root, "refs.data"));
        assert_eq!(
            items.len(),
            if stage < 2 {
                2
            } else if stage == 2 {
                1
            } else {
                0
            },
            "{log}\n{diagnostics:?}"
        );
        for item in items {
            assert_eq!(item.range, span(&source, "pére😀"), "{log}");
            assert_eq!(
                item.data.as_ref().unwrap()["reason"],
                if stage == 2 {
                    "non-xdata-entry"
                } else {
                    "missing-entry"
                }
            );
            assert_eq!(item.data.as_ref().unwrap()["section"], 99999);
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_macro_source_maps_and_duplicate_references_keep_key_fallbacks() {
    let mapping = r#"<config><sourcemap><maps datatype="bibtex"><map><map_step map_field_source="inherit" map_field_target="xdata"/></map></maps></sourcemap></config>"#;
    for (source, config) in [
        (
            "@string{inherit={pére😀}} @book{café😀,title={Title},xdata=inherit}",
            None,
        ),
        (
            "@book{café😀,title={Title},inherit={pére😀}}",
            Some(mapping),
        ),
        ("@book{café😀,title={Title},xdata={pére😀,pére😀}}", None),
        (
            "@book{café😀,title={Title},xdata={first},XDATA={pére😀}}",
            None,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let (log, diagnostics) = native(root, source, config);
        let items = items(&diagnostics, &uri(root, "refs.data"));
        assert_eq!(items.len(), 2, "{source}\n{log}\n{diagnostics:?}");
        for item in items {
            assert_eq!(item.range, span(source, "café😀"), "{source}\n{log}");
            assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
        }
    }
}
