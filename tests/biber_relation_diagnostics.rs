use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

const CODE: &str = "missing-bibliography-relation";
const FIELDS: [&str; 4] = ["crossref", "xref", "related", "entryset"];
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn found(name: &str) -> String {
    format!("INFO - Found BibTeX data source '{name}'\n")
}
fn message(key: &str, parent: &str, field: &str, section: u32) -> String {
    let kind = match field {
        "related" => "related entry",
        "entryset" => "static set member",
        _ => field,
    };
    format!("I didn't find a database entry for {kind} '{parent}' in entry '{key}' - ignoring (section {section})")
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
fn value_span(source: &str, field: &str, parent: &str) -> Range {
    let start = source.find(&format!("{field}=")).unwrap();
    let offset = source[start..].find(parent).unwrap() + start;
    Text::new(source.into()).range(offset, offset + parent.len())
}

#[test]
fn relations_and_xdata_with_the_same_target_keep_their_own_field_and_list_item_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source="% 😀\r\n@set{café😀,CROSSREF={pére😀},XREF=\"pére😀\",RELATED={before, pére😀 ,after},ENTRYSET=\"before,pére😀,after\",XDATA={before,pére😀,after},title={😀}}";
    let mut log = found("refs.data");
    for field in FIELDS {
        log.push_str(&format!(
            "[1] Biber.pm:130> WARN - {}\n",
            message("café😀", "pére😀", field, 7)
        ));
    }
    log.push_str("WARN - Entry 'café😀' references XDATA entry 'pére😀' which does not exist, not resolving (section 7)\n");
    let diagnostics = parse(root, &[("refs.data", source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 4, "{diagnostics:?}");
    for item in items {
        let data = item.data.as_ref().unwrap();
        let field = data["field"].as_str().unwrap();
        assert_eq!(
            item.range,
            value_span(source, &field.to_ascii_uppercase(), "pére😀")
        );
        assert_eq!(
            item.message,
            format!("Biber: {}", message("café😀", "pére😀", field, 7))
        );
        assert_eq!(
            item.data,
            Some(serde_json::json!({"key":"café😀","parent":"pére😀","field":field,"section":7}))
        );
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(item.source.as_deref(), Some("latex-build"));
        assert!(item.related_information.is_none());
    }
    let xdata = diagnostics[&uri(root, "refs.data")]
        .iter()
        .find(|item| item.code == Some(NumberOrString::String("invalid-bibliography-xdata".into())))
        .unwrap();
    assert_eq!(xdata.range, value_span(source, "XDATA", "pére😀"));
}

#[test]
fn nonliteral_duplicate_inherited_and_incomplete_relations_use_only_the_owner_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for field in FIELDS {
        for source in [
            format!("@string{{inherit={{pére😀}}}} @book{{café😀,{field}=inherit}}"),
            format!("@book{{café😀,{field}={{pé}} # {{re😀}}}}"),
            "@book{café😀,inherit={pére😀}}".to_string(),
            format!("@book{{café😀,xdata={{ancestor}}}} @xdata{{ancestor,{field}={{pére😀}}}}"),
            format!("@book{{café😀,{field}={{pére😀}},{field}={{other}}}}"),
            format!(
                "@book{{café😀,{field}={{other}},{}={{pére😀}}}}",
                field.to_ascii_uppercase()
            ),
            format!("@book{{café😀,{field}={{pére😀}},{field}=}}"),
            format!("@book{{café😀,{field}={{pére😀,pére😀}}}}"),
            format!("@book{{café😀,{field}={{PÉRE😀}}}}"),
            format!("@book{{café😀,{field}={{pére😀}}"),
            format!("@book{{café😀,{field}={{{{pére😀}}}}}}"),
            format!("@book{{café😀,{field}={{pére😀\\relax}}}}"),
            format!("@book{{café😀,{field}={{pére😀%comment\n}}}}"),
        ] {
            let diagnostics = parse(
                root,
                &[("refs.data", &source)],
                &format!(
                    "{}WARN - {}\n",
                    found("refs.data"),
                    message("café😀", "pére😀", field, 0)
                ),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(
                item.range,
                span(&source, "café😀"),
                "{field}\n{source}\n{diagnostics:?}"
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
}

#[test]
fn relation_owners_require_complete_prior_inputs_and_unique_case_sensitive_normalized_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,crossref={pére😀}}";
    let warning = format!("WARN - {}\n", message("café😀", "pére😀", "crossref", 0));
    for log in [
        warning.clone(),
        format!("{warning}{}", found("refs.data")),
        format!(
            "{}{}{warning}",
            found("refs.data"),
            found("unreadable.data")
        ),
        format!("{}ERROR - Earlier failure\n{warning}", found("refs.data")),
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
                ("duplicate.data", "@book{café😀,crossref={pére😀}}"),
            ],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.range, Range::default(), "{log}\n{diagnostics:?}");
        assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
    }
    for source in [
        "@book{café😀,crossref={pére😀}} @book{café😀,crossref={pére😀}}",
        "@book{CAFÉ😀,crossref={pére😀}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}{warning}", found("refs.data")),
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "main.tex")).len(),
            1,
            "{diagnostics:?}"
        );
    }
    let diagnostics = parse(
        root,
        &[("refs.data", source), ("duplicate.data", source)],
        &format!("{}{warning}{}", found("refs.data"), found("duplicate.data")),
    );
    assert_eq!(
        items(&diagnostics, &uri(root, "refs.data"))[0].range,
        span(source, "pére😀")
    );
    let diagnostics = parse(
        root,
        &[("refs.data", source)],
        &format!(
            "{}{warning}ERROR - Earlier failure\n{warning}",
            found("refs.data")
        ),
    );
    assert_eq!(items(&diagnostics, &uri(root, "refs.data")).len(), 1);
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
}

#[test]
fn relation_templates_require_the_native_kind_keys_section_and_record_level() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let valid = message("child", "parent", "crossref", 99999);
    for body in [
        valid.replace("crossref", "crossrefs"),
        valid.replace("crossref", "dynamic set member"),
        valid.replace("crossref", "related"),
        valid.replace("crossref", "entryset"),
        valid.replace("'parent'", "''"),
        valid.replace("'child'", "'two keys'"),
        valid.replace("99999", "-1"),
        valid.replace("99999", "4294967296"),
        valid.replace("99999", "１２"),
        valid.replace("99999", ""),
        valid.trim_end_matches(')').into(),
        format!("{valid} extra"),
        format!("Example: {valid}"),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", "@book{child,crossref={parent}}")],
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
            &[("refs.data", "@book{child,crossref={parent}}")],
            &format!("{}{line}\n", found("refs.data")),
        );
        assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
    }
}

#[test]
fn relation_locations_move_after_saving_and_do_not_guess_target_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for field in FIELDS {
        for source in [
            format!("@book{{café😀,{field}={{pére😀}}}}"),
            format!("% 😀\r\n@book{{café😀,{field}={{\r\n pére😀 }} }}"),
        ] {
            let diagnostics = parse(
                root,
                &[("refs.data", &source)],
                &format!(
                    "{}WARN - {}\n",
                    found("refs.data"),
                    message("café😀", "pére😀", field, 0)
                ),
            );
            let item = items(&diagnostics, &uri(root, "refs.data"))[0].clone();
            assert_eq!(item.range, span(&source, "pére😀"));
            workspace.open_with_language(uri(root, "refs.data"), source, 7, "bibtex");
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
}

#[test]
fn thousands_of_relations_and_xdata_share_one_forward_utf16_index_without_mixing_fields() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = String::new();
    let mut log = found("refs.data");
    let mut offset = 0u32;
    let mut expected = BTreeMap::new();
    for index in 0..1500 {
        let key = format!("café{index}😀");
        let reported = format!("café{index}😀");
        let parent = format!("pére{index}😀");
        let reported_parent = format!("pére{index}😀");
        let mut entry = format!("@set{{{key},");
        for field in ["crossref", "xref", "xdata", "related", "entryset"] {
            entry.push_str(&format!("{field}={{"));
            if matches!(field, "xdata" | "related" | "entryset") {
                entry.push_str("before,");
            }
            let start = offset + entry.encode_utf16().count() as u32;
            let range = Range::new(
                Position::new(0, start),
                Position::new(0, start + parent.encode_utf16().count() as u32),
            );
            expected.insert((reported.clone(), field), range);
            entry.push_str(&parent);
            entry.push_str("},");
            if field == "xdata" {
                log.push_str(&format!("WARN - Entry '{reported}' references XDATA entry '{reported_parent}' which does not exist, not resolving (section 99999)\n"));
            } else {
                log.push_str(&format!(
                    "WARN - {}\n",
                    message(&reported, &reported_parent, field, 99999)
                ));
            }
        }
        entry.push_str("title={😀}} ");
        offset += entry.encode_utf16().count() as u32;
        source.push_str(&entry);
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    assert_eq!(diagnostics[&uri(root, "refs.data")].len(), 7500);
    assert_eq!(items(&diagnostics, &uri(root, "refs.data")).len(), 6000);
    for item in &diagnostics[&uri(root, "refs.data")] {
        let data = item.data.as_ref().unwrap();
        assert_eq!(
            item.range,
            expected[&(
                data["key"].as_str().unwrap().to_string(),
                data["field"].as_str().unwrap()
            )]
        );
        assert!(data.get("unresolvedField").is_none());
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
        "--output-resolve-crossrefs",
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
fn real_biber_missing_relations_move_and_clear_after_targets_are_added() {
    for field in FIELDS {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for stage in 0..3 {
            let kind = if field == "entryset" { "set" } else { "book" };
            let source = format!(
                "% 😀\r\n@{kind}{{café😀,{field}={{{}pére😀}}}}\r\n{}",
                if stage > 0 { "\r\n " } else { "" },
                if stage == 2 {
                    "@book{pére😀,title={Parent},date={2026}}"
                } else {
                    ""
                }
            );
            let (log, diagnostics) = native(root, &source, None);
            let items = items(&diagnostics, &uri(root, "refs.data"));
            assert_eq!(
                items.len(),
                usize::from(stage < 2),
                "{field}\n{source}\n{log}\n{diagnostics:?}"
            );
            if let Some(item) = items.first() {
                assert_eq!(item.range, span(&source, "pére😀"));
                assert_eq!(
                    item.data,
                    Some(
                        serde_json::json!({"key":"café😀","parent":"pére😀","field":field,"section":99999})
                    )
                );
            }
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_macro_and_sourcemap_relations_fall_back_to_original_owner_keys() {
    for field in FIELDS {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let kind = if field == "entryset" { "set" } else { "book" };
        let config = format!(
            r#"<config><sourcemap><maps datatype="bibtex"><map><map_step map_field_source="inherit" map_field_target="{field}"/></map></maps></sourcemap></config>"#
        );
        for (source, config) in [
            (
                format!("@string{{inherit={{pére😀}}}} @{kind}{{café😀,{field}=inherit}}"),
                None,
            ),
            (
                format!("@{kind}{{café😀,inherit={{pére😀}}}}"),
                Some(config.as_str()),
            ),
        ] {
            let (log, diagnostics) = native(root, &source, config);
            let items = items(&diagnostics, &uri(root, "refs.data"));
            assert_eq!(items.len(), 1, "{source}\n{log}\n{diagnostics:?}");
            assert_eq!(items[0].range, span(&source, "café😀"));
            assert_eq!(items[0].data.as_ref().unwrap()["unresolvedField"], true);
        }
    }
}
