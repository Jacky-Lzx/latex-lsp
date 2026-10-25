use latex_lsp::{compilation::build_log, editing, project::workspace::Workspace, text::Text};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const CODE: &str = "conflicting-bibliography-alias";
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn found(name: &str) -> String {
    format!("INFO - Found BibTeX data source '{name}'\n")
}
fn duplicate(alias: &str, owner: &str) -> String {
    format!("Entry alias '{alias}' already has an alias '{owner}', skipping ...")
}
fn collision(alias: &str) -> String {
    format!("Citekey alias '{alias}' is also a real entry key, skipping ...")
}
fn recursion(alias: &str) -> String {
    format!("BAD RECURSION! Entry alias '{alias}' is identical to the entry key, skipping ...")
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
    let start = source.find(marker).unwrap();
    Text::new(source.into()).range(start, start + marker.len())
}

#[test]
fn conflicts_locate_the_skipped_alias_and_link_the_accepted_owner_or_real_entry() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (source, message, reason, marker, related) in [
        (
            "% 😀\r\n@book{café😀,ids={pére😀}}\r\n@book{second,IDS=\"before, pére😀,after\"}",
            duplicate("pére😀", "café😀"),
            "duplicate-alias",
            "pére😀",
            "café😀",
        ),
        (
            "% 😀\r\n@book{main,ids={pére😀}}\r\n@book{pére😀,title={Actual}}",
            collision("pére😀"),
            "entry-key-collision",
            "pére😀",
            "pére😀",
        ),
        (
            "% 😀\r\n@book{pére😀,title={Actual}}\r\n@book{main,ids={pére😀}}",
            "Entry alias 'pére😀' is also a real entry key, skipping ...".into(),
            "entry-key-collision",
            "pére😀",
            "pére😀",
        ),
        (
            "% 😀\r\n@book{café😀,ids={before, café😀,after}}",
            recursion("café😀"),
            "self-alias",
            "café😀",
            "café😀",
        ),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}[1] Biber.pm:130> WARN - {message}\n", found("refs.data")),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(source, marker), "{diagnostics:?}");
        assert_eq!(item.message, format!("Biber: {message}"));
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(item.source.as_deref(), Some("latex-build"));
        let data = item.data.as_ref().unwrap();
        assert_eq!(data["reason"], reason);
        assert!(data.get("unresolvedSource").is_none());
        assert!(item
            .related_information
            .as_ref()
            .unwrap()
            .iter()
            .any(|item| item.location
                == Location::new(uri(root, "refs.data"), span(source, related))));
        assert!(item
            .related_information
            .as_ref()
            .unwrap()
            .iter()
            .all(|related| related.location != Location::new(uri(root, "refs.data"), item.range)));
    }
}

#[test]
fn prior_inputs_and_nonstandard_database_names_resolve_cross_file_alias_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = "% 😀\r\n@book{café😀,ids={pére😀}}";
    let current = "% 😀\r\n@book{second,IDS=\"before, pére😀,after\"}";
    let diagnostics = parse(
        root,
        &[
            ("first data", first),
            ("refs.data", current),
            ("future.data", current),
        ],
        &format!(
            "{}{}WARN - {}\n{}",
            found("first data"),
            found("refs.data"),
            duplicate("pére😀", "café😀"),
            found("future.data")
        ),
    );
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(current, "pére😀"));
    assert_eq!(item.related_information.as_ref().unwrap().len(), 2);
    assert!(item
        .related_information
        .as_ref()
        .unwrap()
        .iter()
        .all(|related| related.location.uri == uri(root, "first data")));
    assert!(items(&diagnostics, &uri(root, "future.data")).is_empty());
}

#[test]
fn ambiguous_skipped_aliases_remain_root_diagnostics_with_bounded_related_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for count in [3, 20] {
        let source = (0..count)
            .map(|i| format!("@book{{owner{i},ids={{café😀}}}} "))
            .collect::<String>();
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!(
                "{}WARN - {}\n",
                found("refs.data"),
                duplicate("café😀", "owner0")
            ),
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.range, Range::default());
        assert_eq!(item.data.as_ref().unwrap()["unresolvedAlias"], true);
        assert_eq!(
            item.related_information.as_ref().unwrap().len(),
            (count + 1).min(16)
        );
    }
    let source = "@book{main,ids={café😀,café😀}} @book{café😀,title={Actual}}";
    let diagnostics = parse(
        root,
        &[("refs.data", source)],
        &format!("{}WARN - {}\n", found("refs.data"), collision("café😀")),
    );
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
}

#[test]
fn alias_owner_and_real_key_collisions_never_choose_an_arbitrary_definition() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (source, message) in [
        (
            "@book{main,ids={alias}} @book{alias,} @book{alias,}",
            collision("alias"),
        ),
        (
            "@book{main,ids={alias}} @book{main,} @book{alias,}",
            collision("alias"),
        ),
        (
            "@book{main,ids={alias}} @book{main,} @book{second,ids={alias}}",
            duplicate("alias", "main"),
        ),
        ("@book{main,ids={main}} @book{main,}", recursion("main")),
        (
            "@book{main,ids={alias}} @book{MAIN,} @book{ALIAS,}",
            collision("alias"),
        ),
        (
            "@book{main,ids={alias}} @book{second,ids={alias}}",
            duplicate("alias", "MAIN"),
        ),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}WARN - {message}\n", found("refs.data")),
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "main.tex")).len(),
            1,
            "{diagnostics:?}"
        );
    }
}

#[test]
fn dynamic_duplicate_incomplete_and_lookalike_ids_fields_only_provide_confirmed_related_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for field in [
        "ids=names",
        "ids={ali} # {as}",
        "ids={alias},IDS={alias}",
        "ids={{alias}}",
        "IDſ={alias}",
        "ids={alias\\relax}",
        "ids={alias%comment\n}",
    ] {
        let source = format!("@string{{names={{alias}}}} @book{{main,{field}}} @book{{alias,}}");
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!("{}WARN - {}\n", found("refs.data"), collision("alias")),
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(
            item.data.as_ref().unwrap()["unresolvedAlias"],
            true,
            "{source}"
        );
        assert_eq!(
            item.related_information.as_ref().unwrap().len(),
            1,
            "{source}"
        );
    }
    let source = "@book{alias,} @book{main,ids={alias}";
    let diagnostics = parse(
        root,
        &[("refs.data", source)],
        &format!("{}WARN - {}\n", found("refs.data"), collision("alias")),
    );
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
}

#[test]
fn missing_unreadable_truncated_or_failed_prior_history_does_not_borrow_future_sources() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{main,ids={alias}} @book{alias,}";
    let warning = format!("WARN - {}\n", collision("alias"));
    for log in [
        warning.clone(),
        format!("{warning}{}", found("refs.data")),
        format!("{}{}{warning}", found("refs.data"), found("missing.data")),
        format!("{}ERROR - Unknown failure\n{warning}", found("refs.data")),
        format!(
            "{}[latex-lsp: middle of compiler output omitted]\n{warning}",
            found("refs.data")
        ),
    ] {
        let diagnostics = parse(root, &[("refs.data", source)], &log);
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
        assert!(item.related_information.is_none(), "{log}\n{diagnostics:?}");
    }
}

#[test]
fn cache_success_does_not_outlive_input_history_failure_or_new_ambiguity() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{main,ids={alias}} @book{alias,}";
    let warning = format!("WARN - {}\n", collision("alias"));
    for between in [
        "ERROR - Unknown failure\n".to_string(),
        found("duplicate.data"),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source), ("duplicate.data", "@book{main,}")],
            &format!("{}{warning}{between}{warning}", found("refs.data")),
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "refs.data")).len(),
            1,
            "{diagnostics:?}"
        );
        assert_eq!(
            items(&diagnostics, &uri(root, "main.tex")).len(),
            1,
            "{diagnostics:?}"
        );
    }
}

#[test]
fn complete_native_templates_are_required_and_other_log_levels_do_not_create_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{main,ids={alias}} @book{alias,}";
    for message in [
        "Entry alias '' already has an alias 'main', skipping ...",
        "Entry alias 'alias' already has an alias '', skipping ...",
        "Entry alias 'ali as' already has an alias 'main', skipping ...",
        "Entry alias 'alias' already has an alias 'main', skipping",
        "Entry alias 'alias' already has an alias 'main', skipping ... trailing",
        "Citekey alias 'alias' is also a real entry key",
        "BAD RECURSION! Entry alias 'alias' is identical to the entry key, skipping",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}WARN - {message}\n", found("refs.data")),
        );
        assert!(
            diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{message}"
        );
    }
    for level in ["INFO", "DEBUG", "TRACE"] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}{level} - {}\n", found("refs.data"), collision("alias")),
        );
        assert!(diagnostics
            .values()
            .flatten()
            .all(|item| item.code != Some(NumberOrString::String(CODE.into()))));
    }
}

#[test]
fn many_alias_warnings_on_one_unicode_line_keep_exact_cached_ranges_after_candidate_limits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = "% 😀\r\n".to_string();
    for i in 0..20 {
        source.push_str(&format!("@book{{sharedowner{i},ids={{shared😀}}}} "));
    }
    let mut log = found("refs.data");
    let mut expected = Vec::new();
    for i in 0..1500 {
        source.push_str(&format!(
            "@book{{owner{i},ids={{alias{i}😀}}}} @book{{alias{i}😀,}} "
        ));
        log.push_str(&format!("WARN - {}\n", collision(&format!("alias{i}😀"))));
    }
    let text = Text::new(source.clone());
    for i in 0..1500 {
        let marker = format!("alias{i}😀");
        let start = source.find(&marker).unwrap();
        expected.push(text.range(start, start + marker.len()));
    }
    let diagnostics = parse(root, &[("refs.data", &source)], &log);
    let items = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(items.len(), expected.len());
    for (item, expected) in items.iter().zip(expected) {
        assert_eq!(item.range, expected);
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_alias_conflicts_move_and_clear_when_the_ids_declarations_are_repaired() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (source, reason) in [
        (
            "@book{café😀,ids={pére😀}} @book{second,ids={pére😀}}",
            "duplicate-alias",
        ),
        (
            "@book{main,ids={pére😀}} @book{pére😀,title={Actual}}",
            "entry-key-collision",
        ),
        (
            "@book{pére😀,title={Actual}} @book{main,ids={pére😀}}",
            "entry-key-collision",
        ),
        ("@book{café😀,ids={café😀}}", "self-alias"),
    ] {
        for stage in 0..3 {
            let current = match stage {
                0 => source.to_string(),
                1 => format!("% 😀\r\n{source}"),
                _ => source
                    .replace("pére😀}", "newalias😀}")
                    .replace("ids={pére😀}", "ids={legacy😀}")
                    .replace("ids={café😀}", "ids={legacy😀}"),
            };
            std::fs::write(root.join("refs.data"), &current).unwrap();
            let run = std::process::Command::new("biber")
                .args(["--tool", "--output-file=processed.bib", "refs.data"])
                .current_dir(root)
                .output()
                .unwrap();
            let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
            assert!(run.status.success(), "{log}");
            let diagnostics = parse(root, &[], &log);
            if stage == 2 {
                assert!(!log.contains("WARN -"), "{current}\n{log}");
                assert!(diagnostics
                    .values()
                    .flatten()
                    .all(|item| item.code != Some(NumberOrString::String(CODE.into()))));
                continue;
            }
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(
                item.data.as_ref().unwrap()["reason"],
                reason,
                "{current}\n{log}"
            );
            assert_eq!(item.range.start.line, if stage == 0 { 0 } else { 1 });
            let marker = if reason == "duplicate-alias" {
                "pére😀"
            } else if reason == "self-alias" {
                "café😀"
            } else {
                "pére😀"
            };
            assert_eq!(item.range, span(&current, marker));
        }
    }
}

#[test]
fn alias_conflicts_offer_no_guessed_quick_fix() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{main,ids={alias}} @book{alias,}";
    let diagnostics = parse(
        root,
        &[("refs.data", source)],
        &format!("{}WARN - {}\n", found("refs.data"), collision("alias")),
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

#[test]
#[ignore = "requires biber"]
fn real_biber_macro_aliases_fall_back_and_sourcemaps_keep_original_ids_locations() {
    let config = r#"<config><sourcemap><maps datatype="bibtex"><map><map_step map_field_source="ids" map_field_target="note"/></map></maps></sourcemap></config>"#;
    for (source, config, reason) in [
        (
            "@string{names={pére😀}} @book{café😀,ids={pére😀}} @book{second,ids=names}",
            None,
            "duplicate-alias",
        ),
        (
            "@string{names={pére😀}} @book{main,ids=names} @book{pére😀,}",
            None,
            "entry-key-collision",
        ),
        (
            "@string{names={café😀}} @book{café😀,ids=names}",
            None,
            "self-alias",
        ),
        (
            "@book{main,ids={pére😀}} @book{pére😀,}",
            Some(config),
            "entry-key-collision",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("refs.data"), source).unwrap();
        let mut command = std::process::Command::new("biber");
        command.args(["--tool", "--output-file=processed.bib"]);
        if let Some(config) = config {
            std::fs::write(root.join("biber.conf"), config).unwrap();
            command.arg("--configfile=biber.conf");
        }
        let run = command.arg("refs.data").current_dir(root).output().unwrap();
        let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
        assert!(run.status.success(), "{log}");
        let diagnostics = parse(root, &[], &log);
        let target = uri(
            root,
            if config.is_some() {
                "refs.data"
            } else {
                "main.tex"
            },
        );
        let items = items(&diagnostics, &target);
        assert_eq!(items.len(), 1, "{source}\n{log}\n{diagnostics:?}");
        let item = &items[0];
        assert_eq!(item.data.as_ref().unwrap()["reason"], reason);
        if config.is_some() {
            assert_eq!(item.range, span(source, "pére😀"));
            let output = std::fs::read_to_string(root.join("processed.bib")).unwrap();
            assert!(output.contains("NOTE ="), "{output}");
            assert!(!output.contains("IDS ="), "{output}");
        } else {
            assert_eq!(item.data.as_ref().unwrap()["unresolvedAlias"], true);
            assert!(item.related_information.is_some());
        }
    }
}
