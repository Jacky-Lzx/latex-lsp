use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const CODE: &str = "invalid-bibliography-xdata";
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn found(name: &str) -> String {
    format!("INFO - Found BibTeX data source '{name}'\n")
}
fn message(
    key: &str,
    field: &str,
    kind: usize,
    parent: &str,
    source_field: &str,
    position: &str,
) -> String {
    let body=match kind {
        0=>format!("Entry '{key}' has XDATA reference from field '{field}' that contains no source field"),
        1=>format!("Field '{field}' in entry '{key}' references XDATA field '{source_field}' in entry '{parent}' and this field does not exist, not resolving"),
        2=>format!("Field '{field}' in entry '{key}' which xdata references field '{source_field}' in entry '{parent}' are not the same types, not resolving"),
        _=>format!("Field '{field}' in entry '{key}' references field '{source_field}' position {position} in entry '{parent}' and this position does not exist, not resolving"),
    };
    format!("{body} (section 7)")
}
fn reason(kind: usize) -> &'static str {
    match kind {
        0 => "unspecified-source-field",
        1 => "missing-source-field",
        2 => "incompatible-field-types",
        _ => "missing-position",
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
fn granular_xdata_templates_mark_whole_referring_values_and_preserve_native_context() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (rhs, marker) in [
        ("{xdata=pére😀-title}", "xdata=pére😀-title"),
        ("\"xdata=pére😀-title\"", "xdata=pére😀-title"),
        ("inherit # {😀}", "inherit # {😀}"),
        ("{}", "{}"),
        (
            "{prefix and xdata=pére😀-title}",
            "prefix and xdata=pére😀-title",
        ),
    ] {
        let source =
            format!("% 😀\r\n@book{{café😀,TITLE={rhs}}}\r\n@xdata{{pére😀,publisher={{Parent}}}}");
        for kind in 0..4 {
            let message = message("café😀", "title", kind, "pére😀", "publisher", "2");
            let diagnostics = parse(
                root,
                &[("refs.data", &source)],
                &format!("{}[0] Biber.pm:130> WARN - {message}\n", found("refs.data")),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(
                item.range,
                span(&source, marker),
                "{source}\n{diagnostics:?}"
            );
            let mut expected = serde_json::json!({"key":"café😀","field":"title","section":7,"reason":reason(kind)});
            if kind != 0 {
                expected["parent"] = "pére😀".into();
                expected["sourceField"] = "publisher".into();
            }
            if kind == 3 {
                expected["sourcePosition"] = "2".into();
            }
            assert_eq!(item.data, Some(expected));
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(item.source.as_deref(), Some("latex-build"));
            assert_eq!(item.message, format!("Biber: {message}"));
            assert!(item.related_information.is_none());
        }
    }
}

#[test]
fn granular_field_names_keep_input_spelling_even_when_model_names_collide() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source="% 😀\r\n@book{café😀,Straße={xdata=first-title},STRASSE={xdata=second-title},IſBN={xdata=third-title},ISBN={xdata=fourth-title},CAFÉ={xdata=fifth-title}}";
    for (reported, marker) in [
        ("straße", "xdata=first-title"),
        ("strasse", "xdata=second-title"),
        ("iſbn", "xdata=third-title"),
        ("isbn", "xdata=fourth-title"),
        ("cafÉ", "xdata=fifth-title"),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                message("café😀", reported, 0, "", "", "")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(
            item.range,
            span(source, marker),
            "{reported}\n{diagnostics:?}"
        );
        assert!(item.data.as_ref().unwrap().get("unresolvedField").is_none());
    }
    let source = "@book{café😀,CAfé={xdata=first-title},café={xdata=second-title}}";
    let diagnostics = parse(
        root,
        &[("refs.data", source)],
        &format!(
            "{}WARN - {}\n",
            found("refs.data"),
            message("café😀", "café", 0, "", "", "")
        ),
    );
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(source, "café😀"));
    assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
}

#[test]
fn missing_duplicate_inherited_and_incomplete_granular_fields_use_the_owner_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        "@book{café😀,inherit={xdata=parent-title}}",
        "@book{café😀,TITLE={xdata=parent-title},title={other}}",
        "@book{café😀,TITLE={xdata=parent-title},title=}",
        "@book{café😀,title=}",
        "@book{café😀,title={xdata=parent-title}",
        "@book{café😀,xdata={ancestor}} @xdata{ancestor,title={xdata=parent-title}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                message("café😀", "title", 1, "parent", "title", "")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(
            item.range,
            span(source, "café😀"),
            "{source}\n{diagnostics:?}"
        );
        assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
    }
}

#[test]
fn granular_sources_require_prior_complete_history_and_unique_normalized_owners() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,title={xdata=parent-title}}";
    let record = format!(
        "WARN - {}\n",
        message("café😀", "title", 1, "parent", "title", "")
    );
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
                ("duplicate.data", "@book{café😀,title={xdata=parent-title}}"),
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
    let diagnostics = parse(
        root,
        &[("refs.data", source)],
        &format!(
            "{}{record}ERROR - Earlier failure\n{record}",
            found("refs.data")
        ),
    );
    assert_eq!(items(&diagnostics, &uri(root, "refs.data")).len(), 1);
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
}

#[test]
fn granular_templates_reject_truncated_fields_keys_sections_and_position_excerpts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let valid = message("child", "title", 3, "parent", "author", "2");
    for body in [
        valid.replace("'title'", "''"),
        valid.replace("'author'", "'two fields'"),
        valid.replace("'child'", "'two keys'"),
        valid.replace("'parent'", "''"),
        valid.replace("position 2", "position -1"),
        valid.replace("position 2", "position +1"),
        valid.replace("position 2", "position ２"),
        valid.replace("position 2", "position "),
        valid.replace("section 7", "section -1"),
        valid.replace("section 7", "section 4294967296"),
        valid.replace("this position", "this field"),
        format!("{valid} extra"),
        format!("Example: {valid}"),
        valid.trim_end_matches(')').into(),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", "@book{child,title={xdata=parent-title}}")],
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
            &[("refs.data", "@book{child,title={xdata=parent-title}}")],
            &format!("{}{line}\n", found("refs.data")),
        );
        assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
    }
}

#[test]
fn granular_diagnostics_move_with_whole_values_and_do_not_guess_xdata_repairs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = Workspace {
        code_action_literals: true,
        ..Default::default()
    };
    workspace.add_root(root.into());
    for source in [
        "@book{café😀,title={xdata=pére😀-title}}",
        "% 😀\r\n@book{café😀,title={\r\n xdata=pére😀-title}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                message("café😀", "title", 1, "pére😀", "title", "")
            ),
        );
        let item = items(&diagnostics, &uri(root, "refs.data"))[0].clone();
        let start = source.find("title={").unwrap() + "title={".len();
        let end = start + source[start..].find('}').unwrap();
        assert_eq!(item.range, Text::new(source.into()).range(start, end));
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
}

#[test]
fn thousands_of_granular_warnings_share_exact_whole_value_ranges_in_the_input_namespace() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = String::new();
    let mut log = found("refs.data");
    let mut expected = BTreeMap::new();
    let mut offset = 0u32;
    for index in 0..1500 {
        let key = format!("café{index}😀");
        let reported = format!("café{index}😀");
        let mut entry = format!("@book{{{key},");
        for (kind, (raw, field)) in [
            ("TITLE", "title"),
            ("AUTHOR", "author"),
            ("Straße", "straße"),
            ("STRASSE", "strasse"),
        ]
        .into_iter()
        .enumerate()
        {
            entry.push_str(&format!("{raw}={{"));
            let value = format!("xdata=pére{index}😀-title-2");
            let start = offset + entry.encode_utf16().count() as u32;
            let range = Range::new(
                Position::new(0, start),
                Position::new(0, start + value.encode_utf16().count() as u32),
            );
            expected.insert((reported.clone(), field), (range, reason(kind)));
            entry.push_str(&value);
            entry.push_str("},");
            log.push_str(&format!(
                "WARN - {}\n",
                message(&reported, field, kind, "pére😀", "title", "2")
            ));
        }
        entry.push_str("publisher={😀}} ");
        offset += entry.encode_utf16().count() as u32;
        source.push_str(&entry);
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), 6000);
    for item in items {
        let data = item.data.as_ref().unwrap();
        let (range, reason) = expected[&(
            data["key"].as_str().unwrap().to_string(),
            data["field"].as_str().unwrap(),
        )];
        assert_eq!(item.range, range);
        assert_eq!(data["reason"], reason);
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
fn real_biber_granular_xdata_values_move_and_clear_after_field_and_position_repairs() {
    for (field, value, parent, repaired_parent, reason) in [
        (
            "title",
            "xdata=pére😀",
            "@xdata{pére😀,title={Parent}}",
            "@xdata{pére😀,title={Parent}}",
            "unspecified-source-field",
        ),
        (
            "title",
            "xdata=pére😀-title",
            "@xdata{pére😀,publisher={Publisher}}",
            "@xdata{pére😀,title={Parent}}",
            "missing-source-field",
        ),
        (
            "title",
            "xdata=pére😀-author",
            "@xdata{pére😀,author={Doe, Jane}}",
            "@xdata{pére😀,author={Doe, Jane}}",
            "incompatible-field-types",
        ),
        (
            "author",
            "xdata=pére😀-author-2",
            "@xdata{pére😀,author={Doe, Jane}}",
            "@xdata{pére😀,author={Doe, Jane and Smith, John}}",
            "missing-position",
        ),
        (
            "publisher",
            "xdata=pére😀-publisher-2",
            "@xdata{pére😀,publisher={Publisher}}",
            "@xdata{pére😀,publisher={First and Second}}",
            "missing-position",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for stage in 0..3 {
            let value = if stage == 2
                && matches!(
                    reason,
                    "unspecified-source-field" | "incompatible-field-types"
                ) {
                "xdata=pére😀-title"
            } else {
                value
            };
            let parent = if stage == 2 {
                if reason == "incompatible-field-types" {
                    "@xdata{pére😀,title={Parent}}"
                } else {
                    repaired_parent
                }
            } else {
                parent
            };
            let source = format!(
                "% 😀\r\n@book{{café😀,{}{field}={{{value}}}}}\r\n{parent}\r\n",
                if stage > 0 { "\r\n " } else { "" }
            );
            let (log, diagnostics) = native(root, &source, None);
            let items = items(&diagnostics, &uri(root, "refs.data"));
            assert_eq!(
                items.len(),
                usize::from(stage < 2),
                "{reason}\n{source}\n{log}\n{diagnostics:?}"
            );
            if let Some(item) = items.first() {
                let start = source.find(&format!("{field}={{")).unwrap() + field.len() + 2;
                let end = start + source[start..].find('}').unwrap();
                assert_eq!(item.range, Text::new(source.clone()).range(start, end));
                assert_eq!(item.data.as_ref().unwrap()["reason"], reason);
            }
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_granular_input_collisions_macros_and_sourcemaps_keep_raw_field_locations() {
    let model = r#"<config><datamodel><fields><field fieldtype="field" datatype="literal">strasse</field></fields><entryfields><entrytype>book</entrytype><field>strasse</field></entryfields></datamodel></config>"#;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,Straße={xdata=first},STRASSE={xdata=second}}";
    let (log, diagnostics) = native(root, source, Some(model));
    let actual = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(actual.len(), 2, "{log}\n{diagnostics:?}");
    for (field, marker) in [("straße", "xdata=first"), ("strasse", "xdata=second")] {
        let item = actual
            .iter()
            .find(|item| item.data.as_ref().unwrap()["field"] == field)
            .unwrap();
        assert_eq!(item.range, span(source, marker));
    }
    let mapping = r#"<config><sourcemap><maps datatype="bibtex"><map><map_step map_field_source="inherit" map_field_target="title"/></map></maps></sourcemap></config>"#;
    for (source,config,marker,fallback) in [
        ("@string{inherit={xdata=pére😀-title}} @book{café😀,title=inherit} @xdata{pére😀,publisher={Parent}}",None,"inherit} @xdata",false),
        ("@book{café😀,inherit={xdata=pére😀-title}} @xdata{pére😀,publisher={Parent}}",Some(mapping),"café😀",true),
    ] {
        let (log,diagnostics)=native(root,source,config);
        let actual=items(&diagnostics,&uri(root,"refs.data"));
        assert_eq!(actual.len(),1,"{log}\n{diagnostics:?}");
        let expected=if fallback {span(source,marker)} else {
            let start=source.find("title=inherit").unwrap()+"title=".len();
            Text::new(source.into()).range(start,start+"inherit".len())
        };
        assert_eq!(actual[0].range,expected);
        assert_eq!(actual[0].data.as_ref().unwrap().get("unresolvedField").is_some(),fallback);
    }
}
