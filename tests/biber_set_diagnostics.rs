#[path = "support/biber.rs"]
mod biber;

use lsp_types::*;
use std::{collections::BTreeMap, path::Path};
use texquill::{compilation::build_log, editing, project::workspace::Workspace, text::Text};

const CODE: &str = "invalid-bibliography-set";
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn found(name: &str) -> String {
    format!("INFO - Found BibTeX data source '{name}'\n")
}
fn message(key: &str, kind: usize) -> String {
    match kind {
        0 => format!("Field 'crossref' is no longer needed in set entries in Biber - ignoring in entry '{key}'"),
        1 => format!("Field 'entryset' is no longer needed in set member entries in Biber - ignoring in entry '{key}'"),
        2 => format!("Set entry '{key}' has no entryset field, ignoring"),
        _ => unreachable!(),
    }
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
fn obsolete_set_fields_mark_raw_model_names_even_for_dynamic_values() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (kind, field) in [(0, "CROSSREF"), (0, "croſſref"), (1, "ENTRYſET")] {
        for value in ["{member}", "\"member\"", "names # {member}", "{}"] {
            let source = format!("% 😀\r\n@set{{café😀,{field}={value}}}\r\n");
            let warning = message("café😀", kind);
            let diagnostics = parse(
                root,
                &[("refs.data", &source)],
                &format!("{}WARN - {warning}\n", found("refs.data")),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(item.range, span(&source, field));
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(item.message, format!("Biber: {warning}"));
            assert_eq!(item.source.as_deref(), Some("latex-build"));
            assert_eq!(
                item.data,
                Some(
                    serde_json::json!({"key":"café😀","field":if kind==0 {"crossref"} else {"entryset"},"reason":"obsolete-field"})
                )
            );
        }
    }
}

#[test]
fn absent_set_members_mark_the_key_even_when_source_maps_removed_a_raw_field() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        "% 😀\r\n@set{café😀,title={Set}}",
        "@set{café😀,ENTRYSET={member}}",
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source)],
            &format!("{}WARN - {}\n", found("refs.data"), message("café😀", 2)),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(source, "café😀"));
        assert_eq!(
            item.data,
            Some(
                serde_json::json!({"key":"café😀","field":"entryset","reason":"missing-entryset"})
            )
        );
    }
}

#[test]
fn ambiguous_incomplete_and_mapped_fields_fall_back_to_the_unique_owner() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for body in [
        "crossref={a},CROSSREF={b}}",
        "CROSSREF={a},croſſref={b}}",
        "note={mapped}}",
        "crossref=}",
        "crossref={a}",
    ] {
        let source = format!("% 😀\r\n@set{{café😀,{body}");
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!("{}WARN - {}\n", found("refs.data"), message("café😀", 0)),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(&source, "café😀"), "{source}");
        assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
    }
}

#[test]
fn unknown_or_unreadable_history_and_canonical_key_collisions_keep_root_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@set{café😀,crossref={parent}}";
    let warning = format!("WARN - {}\n", message("café😀", 0));
    for log in [
        warning.clone(),
        format!("{warning}{}", found("refs.data")),
        format!("{}{}{warning}", found("refs.data"), found("missing.data")),
        format!("{}ERROR - Unknown failure\n{warning}", found("refs.data")),
        format!(
            "{}[texquill: middle of compiler output omitted]\n{warning}",
            found("refs.data")
        ),
        format!("{}{}{warning}", found("refs.data"), found("duplicate.data")),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source), ("duplicate.data", "@set{café😀,}")],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(
            item.data.as_ref().unwrap()["unresolvedSource"],
            true,
            "{log}"
        );
        assert!(items(&diagnostics, &uri(root, "refs.data")).is_empty());
    }
    let source = format!("{source} @set{{café😀,crossref={{second}}}}");
    let diagnostics = parse(
        root,
        &[("refs.data", &source)],
        &format!("{}{warning}", found("refs.data")),
    );
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
}

#[test]
fn history_is_ordered_case_sensitive_and_deduplicates_actual_sources() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@set{owner,crossref={parent}} @set{OWNER,crossref={other}}";
    let diagnostics = parse(
        root,
        &[("refs.data", source), ("future.data", "@set{owner,}")],
        &format!(
            "{}{}WARN - {}\n{}",
            found("refs.data"),
            found("./refs.data"),
            message("owner", 0),
            found("future.data")
        ),
    );
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(source, "crossref"));
}

#[test]
fn quoted_key_punctuation_cannot_override_the_fixed_native_template() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for key in ["O'Connor😀", "entry.bib", "a\"b", "a'has'no'entryset"] {
        let source = format!("@set{{{key},crossref={{parent}}}}");
        for kind in [0, 2] {
            let diagnostics = parse(
                root,
                &[("refs.data", &source)],
                &format!("{}WARN - {}\n", found("refs.data"), message(key, kind)),
            );
            let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
            assert_eq!(
                item.range,
                span(&source, if kind == 0 { "crossref" } else { key })
            );
        }
    }
}

#[test]
fn only_complete_native_set_templates_are_classified() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for warning in [
        message("", 0),
        message("bad key", 1),
        format!("{} trailing", message("owner", 2)),
        message("owner", 0).trim_end_matches('\'').into(),
        "Set entry 'owner' has no entryset field".into(),
        "Field 'xref' is no longer needed in set entries in Biber - ignoring in entry 'owner'"
            .into(),
        "Field 'entryset' is no longer needed in set entries in Biber - ignoring in entry 'owner'"
            .into(),
    ] {
        let diagnostics = parse(root, &[], &format!("WARN - {warning}\n"));
        assert!(
            diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{warning}"
        );
    }
    let diagnostics = parse(
        root,
        &[("refs.data", "@set{owner,crossref={parent}}")],
        &format!("{}INFO - {}\n", found("refs.data"), message("owner", 0)),
    );
    assert!(diagnostics
        .values()
        .flatten()
        .all(|item| item.code != Some(NumberOrString::String(CODE.into()))));
}

#[test]
fn oversized_prior_sources_prevent_partial_set_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let oversized = format!("%{}\n", "x".repeat(2 * 1024 * 1024));
    let diagnostics = parse(
        root,
        &[
            ("refs.data", "@set{owner,crossref={parent}}"),
            ("large.data", &oversized),
        ],
        &format!(
            "{}{}WARN - {}\n",
            found("refs.data"),
            found("large.data"),
            message("owner", 0)
        ),
    );
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
}

#[test]
fn native_set_reports_do_not_offer_guessed_field_deletions_or_insertions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    for kind in 0..3 {
        let diagnostics = parse(
            root,
            &[(
                "refs.data",
                "@set{owner,crossref={parent},entryset={child}}",
            )],
            &format!("{}WARN - {}\n", found("refs.data"), message("owner", kind)),
        );
        let target = uri(root, "refs.data");
        let item = &items(&diagnostics, &target)[0];
        let context = CodeActionContext {
            diagnostics: vec![item.clone()],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None,
        };
        assert!(editing::code_actions(&workspace, &target, item.range, &context).is_empty());
    }
}

fn native(root: &Path, source: &str, control: &str) -> (String, BTreeMap<Url, Vec<Diagnostic>>) {
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    std::fs::write(root.join("refs.data"), source).unwrap();
    std::fs::write(root.join("main.bcf"), control).unwrap();
    let output = std::process::Command::new("biber")
        .args(["--noconf", "main"])
        .current_dir(root)
        .output()
        .unwrap();
    let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
    assert!(
        output.status.success(),
        "{log}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join("main.bbl").is_file());
    let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
    (log, diagnostics)
}

#[test]
#[ignore = "requires biber"]
fn real_normal_mode_biber_set_warnings_move_and_clear_after_repair() {
    let control = biber::control_file();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for (kind,source,marker) in [
        (0,"@set{café😀,entryset={child},croſſref={parent}} @book{child,title={Child}} @book{parent,title={Parent}}","croſſref"),
        (1,"@set{group,entryset={café😀}} @book{café😀,ENTRYSET={group},title={Child}}","ENTRYSET"),
        (2,"@set{café😀,title={Empty}}","café😀"),
    ] {
        for current in [source.to_string(),format!("% moved 😀\r\n{source}")] {
            let (log,diagnostics)=native(root,&current,&control);
            let matches=items(&diagnostics,&uri(root,"refs.data"));
            assert_eq!(matches.len(),1,"{log}\n{diagnostics:?}");
            assert_eq!(matches[0].range,span(&current,marker));
            assert_eq!(matches[0].message,format!("Biber: {}",message("café😀",kind)));
        }
        let (_,diagnostics)=native(root,"@set{group,entryset={child}} @book{child,title={Child}}",&control);
        assert!(diagnostics.values().flatten().all(|item|item.code!=Some(NumberOrString::String(CODE.into()))));
    }
}

#[test]
#[ignore = "requires biber"]
fn real_normal_mode_biber_model_collisions_and_source_maps_keep_conservative_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let control = biber::control_file();
    let source="@set{owner,entryset={child},CROSSREF={parent},croſſref={parent}} @book{child,title={Child}} @book{parent,title={Parent}}";
    let (_, diagnostics) = native(root, source, &control);
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    assert_eq!(item.range, span(source, "owner"));
    assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
    let mapped=control.replace("<bibdata",r#"<sourcemap><maps datatype="bibtex" level="user"><map><map_step map_field_source="note" map_field_target="crossref"/></map></maps></sourcemap><bibdata"#);
    let source="@set{owner,entryset={child},note={parent}} @book{child,title={Child}} @book{parent,title={Parent}}";
    let (log, diagnostics) = native(root, source, &mapped);
    let matches = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(matches.len(), 1, "{log}");
    assert_eq!(matches[0].range, span(source, "owner"));
    assert_eq!(matches[0].data.as_ref().unwrap()["unresolvedField"], true);
    let removed = control.replace("<bibdata", r#"<sourcemap><maps datatype="bibtex" level="user"><map><map_step map_field_source="entryset" map_field_target="note"/></map></maps></sourcemap><bibdata"#);
    let source = "@set{owner,ENTRYSET={child}} @book{child,title={Child}}";
    let (log, diagnostics) = native(root, source, &removed);
    let matches = items(&diagnostics, &uri(root, "refs.data"));
    assert_eq!(matches.len(), 1, "{log}");
    assert_eq!(matches[0].range, span(source, "owner"));
    assert_eq!(
        matches[0].data,
        Some(serde_json::json!({"key":"owner","field":"entryset","reason":"missing-entryset"}))
    );
}
