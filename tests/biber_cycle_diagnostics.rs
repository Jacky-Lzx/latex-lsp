use latex_lsp::{build_log, editing, text::Text, workspace::Workspace};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

const CODE: &str = "cyclic-bibliography-inheritance";
fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}
fn found(name: &str) -> String {
    format!("INFO - Found BibTeX data source '{name}'\n")
}
fn message(parent: &str, key: &str, xdata: bool) -> String {
    format!(
        "Circular {}inheritance between '{parent}'<->'{key}'",
        if xdata { "XDATA " } else { "" }
    )
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
fn crossref_cycles_use_the_reported_child_reference_and_related_parent_before_fatal_history_expires(
) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% 😀\r\n@book{café😀,crossref={pére😀}}\r\n@book{pére😀,CROSSREF=\"café😀\"}";
    let warning = message("café😀", "pére😀", false);
    let diagnostics = parse(
        root,
        &[("refs.data", source)],
        &format!(
            "{}[1] Utils.pm:409> ERROR - {warning}\n",
            found("refs.data")
        ),
    );
    let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
    let start = source.find("CROSSREF=\"").unwrap() + "CROSSREF=\"".len();
    assert_eq!(
        item.range,
        Text::new(source.into()).range(start, start + "café😀".len())
    );
    assert_eq!(item.message, format!("Biber: {warning}"));
    assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(
        item.data,
        Some(serde_json::json!({"key":"pére😀","parent":"café😀","inheritance":"crossref"}))
    );
    assert_eq!(
        item.related_information.as_ref().unwrap()[0].location,
        Location::new(uri(root, "refs.data"), span(source, "café😀"))
    );
}

#[test]
fn whole_entry_and_granular_xdata_cycles_mark_only_the_reported_child_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for source in [
        "% 😀\r\n@xdata{café😀,xdata={pére😀}} @xdata{pére😀,xdata={café😀}}",
        "% 😀\r\n@xdata{café😀,title={xdata=pére😀-title}} @xdata{pére😀,title={xdata=café😀-title}}",
    ] {
        let diagnostics=parse(root,&[("refs.data",source)],&format!("{}ERROR - {}\n",found("refs.data"),message("café😀","pére😀",true)));
        let item=&items(&diagnostics,&uri(root,"refs.data"))[0];
        let start=source.find("@xdata{pére😀").unwrap()+"@xdata{".len();
        assert_eq!(item.range,Text::new(source.into()).range(start,start+"pére😀".len()));
        assert_eq!(item.data.as_ref().unwrap()["inheritance"],"xdata");
        assert!(item.data.as_ref().unwrap().get("unresolvedField").is_none());
    }
}

#[test]
fn self_cycles_keep_the_key_as_related_only_when_the_primary_range_is_a_crossref_value() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for xdata in [false, true] {
        let field = if xdata { "xdata" } else { "crossref" };
        let source = format!("% 😀\r\n@book{{café😀,{field}={{café😀}}}}");
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!(
                "{}ERROR - {}\n",
                found("refs.data"),
                message("café😀", "café😀", xdata)
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        if xdata {
            assert!(item.related_information.is_none());
            assert_eq!(item.range, span(&source, "café😀"));
        } else {
            assert_eq!(item.related_information.as_ref().unwrap().len(), 1);
            assert_eq!(
                item.related_information.as_ref().unwrap()[0].location.range,
                span(&source, "café😀")
            );
            assert_ne!(item.range, span(&source, "café😀"));
        }
    }
}

#[test]
fn macro_mapped_duplicate_or_incomplete_crossrefs_fall_back_to_the_unique_child_key() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for body in [
        "crossref=parent",
        "crossref=",
        "inherit={pére😀}",
        "crossref={pére😀},CROSSREF={pére😀}",
        "crossref={pé} # {re😀}",
        "crossref={other}",
    ] {
        let source = format!("@book{{café😀,{body}}} @book{{pére😀,crossref={{café😀}}}}");
        let diagnostics = parse(
            root,
            &[("refs.data", &source)],
            &format!(
                "{}ERROR - {}\n",
                found("refs.data"),
                message("pére😀", "café😀", false)
            ),
        );
        let item = &items(&diagnostics, &uri(root, "refs.data"))[0];
        assert_eq!(item.range, span(&source, "café😀"));
        assert_eq!(item.data.as_ref().unwrap()["unresolvedField"], true);
    }
}

#[test]
fn cycle_errors_consume_only_their_own_pre_error_history_and_never_revive_later_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,crossref={pére😀},year={bad}} @book{pére😀,crossref={café😀}}";
    let year="WARN - legacy year field 'bad' in entry 'café😀' is not an integer - this will probably not sort properly.\n";
    let error = format!("ERROR - {}\n", message("pére😀", "café😀", false));
    let diagnostics = parse(
        root,
        &[("refs.data", source)],
        &format!(
            "{}{year}{error}{year}{}{error}",
            found("refs.data"),
            found("refs.data")
        ),
    );
    assert_eq!(items(&diagnostics, &uri(root, "refs.data")).len(), 1);
    assert_eq!(items(&diagnostics, &uri(root, "main.tex")).len(), 1);
    let field_code = Some(NumberOrString::String(
        "invalid-bibliography-field-value".into(),
    ));
    assert_eq!(
        diagnostics[&uri(root, "refs.data")]
            .iter()
            .filter(|item| item.code == field_code)
            .count(),
        1
    );
    assert_eq!(
        diagnostics[&uri(root, "main.tex")]
            .iter()
            .filter(|item| item.code == field_code)
            .count(),
        1
    );
}

#[test]
fn history_failures_missing_inputs_and_canonical_child_collisions_preserve_root_errors() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,crossref={pére😀}} @book{pére😀,crossref={café😀}}";
    let error = format!("ERROR - {}\n", message("pére😀", "café😀", false));
    for log in [
        error.clone(),
        format!("{error}{}", found("refs.data")),
        format!("{}{}{error}", found("refs.data"), found("missing.data")),
        format!("{}ERROR - Unknown failure\n{error}", found("refs.data")),
        format!(
            "{}[latex-lsp: middle of compiler output omitted]\n{error}",
            found("refs.data")
        ),
        format!("{}{}{error}", found("refs.data"), found("duplicate.data")),
    ] {
        let diagnostics = parse(
            root,
            &[("refs.data", source), ("duplicate.data", "@book{café😀,}")],
            &log,
        );
        let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
        assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
    }
}

#[test]
fn ambiguous_cycle_keys_have_bounded_related_candidates_and_no_arbitrary_primary() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut source = "@book{pére😀,crossref={café😀}} ".to_string();
    for i in 0..20 {
        source.push_str(&format!("@book{{café😀,title={{{i}}}}} "));
    }
    let diagnostics = parse(
        root,
        &[("refs.data", &source)],
        &format!(
            "{}ERROR - {}\n",
            found("refs.data"),
            message("pére😀", "café😀", false)
        ),
    );
    let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
    assert_eq!(item.range, Range::default());
    assert_eq!(item.related_information.as_ref().unwrap().len(), 16);
}

#[test]
fn cross_database_cycles_ignore_future_collisions_and_never_recover_after_the_fatal_record() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let parent = "% 😀\r\n@book{pére😀,crossref={café😀}}";
    let child = "% 😀\r\n@book{café😀,crossref={pére😀}}";
    let error = format!("ERROR - {}\n", message("pére😀", "café😀", false));
    let diagnostics = parse(
        root,
        &[
            ("parent data", parent),
            ("child.data", child),
            ("future.data", "@book{café😀,}"),
        ],
        &format!(
            "{}{}{error}{}{error}",
            found("parent data"),
            found("child.data"),
            found("future.data")
        ),
    );
    let item = &items(&diagnostics, &uri(root, "child.data"))[0];
    assert_eq!(item.range, span(child, "pére😀"));
    assert_eq!(
        item.related_information.as_ref().unwrap()[0].location,
        Location::new(uri(root, "parent data"), span(parent, "pére😀"))
    );
    let later = &items(&diagnostics, &uri(root, "main.tex"))[0];
    assert_eq!(later.data.as_ref().unwrap()["unresolvedSource"], true);
    assert!(later.related_information.is_none());
}

#[test]
fn oversized_prior_databases_do_not_leave_partial_cycle_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,crossref={pére😀}} @book{pére😀,crossref={café😀}}";
    let oversized = format!("%{}", "x".repeat(2 * 1024 * 1024));
    let diagnostics = parse(
        root,
        &[("refs.data", source), ("oversized.data", &oversized)],
        &format!(
            "{}{}ERROR - {}\n",
            found("refs.data"),
            found("oversized.data"),
            message("pére😀", "café😀", false)
        ),
    );
    let item = &items(&diagnostics, &uri(root, "main.tex"))[0];
    assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
    assert!(item.related_information.is_none());
}

#[test]
fn cycle_templates_reject_ambiguous_delimiters_trailing_text_and_dynamic_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for text in [
        "Circular inheritance between 'parent'<->''",
        "Circular inheritance between 'par ent'<->'child'",
        "Circular inheritance between 'parent'<->'child' trailing",
        "Circular inheritance between 'parent'<->'child'<->'extra'",
        "Circular XDATA inheritance between 'parent'->'child'",
        "Circular inheritance between 'parent'<->'child",
    ] {
        let diagnostics = parse(root, &[], &format!("ERROR - {text}\n"));
        assert!(
            diagnostics
                .values()
                .flatten()
                .all(|item| item.code != Some(NumberOrString::String(CODE.into()))),
            "{text}"
        );
    }
}

#[test]
fn inheritance_cycle_diagnostics_offer_no_guessed_field_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@book{café😀,crossref={café😀}}";
    let diagnostics = parse(
        root,
        &[("refs.data", source)],
        &format!(
            "{}ERROR - {}\n",
            found("refs.data"),
            message("café😀", "café😀", false)
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
) -> (bool, String, BTreeMap<Url, Vec<Diagnostic>>) {
    std::fs::write(root.join("refs.data"), source).unwrap();
    let mut command = std::process::Command::new("biber");
    command.args([
        "--tool",
        "--output-resolve-crossrefs",
        "--output-resolve-xdata",
        "--output-file=processed.bib",
    ]);
    if let Some(config) = config {
        std::fs::write(root.join("biber.conf"), config).unwrap();
        command.arg("--configfile=biber.conf");
    }
    let output = command.arg("refs.data").current_dir(root).output().unwrap();
    let log = std::fs::read_to_string(root.join("refs.data.blg")).unwrap();
    let diagnostics = parse(root, &[], &log);
    (output.status.success(), log, diagnostics)
}
fn raw_key(key: &str) -> &'static str {
    match key {
        "café😀" => "café😀",
        "pére😀" => "pére😀",
        _ => panic!("Unexpected key: {key}"),
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_crossref_and_xdata_cycles_move_and_clear_after_repair() {
    for (source,xdata) in [
        ("@book{café😀,crossref={pére😀}} @book{pére😀,crossref={café😀}}",false),
        ("@book{café😀,crossref={café😀}}",false),
        ("@xdata{café😀,xdata={pére😀}} @xdata{pére😀,xdata={café😀}} @book{child,xdata={café😀}}",true),
        ("@xdata{café😀,xdata={café😀}} @book{child,xdata={café😀}}",true),
        ("@xdata{café😀,title={xdata=pére😀-title}} @xdata{pére😀,title={xdata=café😀-title}} @book{child,xdata={café😀}}",true),
    ] {
        let directory=tempfile::tempdir().unwrap();let root=directory.path();
        for stage in 0..3 {
            let current=match stage {0=>source.to_string(),1=>format!("% 😀\r\n{source}"),_=>source.replace("crossref={café😀}","").replace("xdata={café😀}","").replace("title={xdata=café😀-title}","title={Resolved}")};
            let (success,log,diagnostics)=native(root,&current,None);
            if stage==2 {assert!(success,"{current}\n{log}");assert!(diagnostics.values().flatten().all(|item|item.code!=Some(NumberOrString::String(CODE.into()))));continue;}
            assert!(!success,"{log}");
            let items=items(&diagnostics,&uri(root,"refs.data"));
            assert_eq!(items.len(),1,"{current}\n{log}\n{diagnostics:?}");
            let item=&items[0];
            let data=item.data.as_ref().unwrap();
            let target=raw_key(data["key"].as_str().unwrap());
            let parent=raw_key(data["parent"].as_str().unwrap());
            let text=Text::new(current.clone());
            let syntax=latex_lsp::syntax::Syntax::bibtex(&text);
            let expected=if xdata {
                let entry=syntax.citations.iter().find(|entry|entry.name==target).unwrap();
                text.range(entry.span.start,entry.span.end)
            } else {
                let marker=format!("crossref={{{parent}}}");
                let start=current.find(&marker).unwrap()+"crossref={".len();
                text.range(start,start+parent.len())
            };
            assert_eq!(item.range,expected,"{current}\n{log}");
            assert_eq!(item.range.start.line,if stage==0 {0}else{1});
            assert_eq!(item.severity,Some(DiagnosticSeverity::ERROR));
        }
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_macro_mapped_and_alias_crossref_cycles_use_original_child_keys() {
    let config = r#"<config><sourcemap><maps datatype="bibtex"><map><map_step map_field_source="inherit" map_field_target="crossref"/></map></maps></sourcemap></config>"#;
    for (source,config) in [
        ("@string{one={pére😀}} @string{two={café😀}} @book{café😀,crossref=one} @book{pére😀,crossref=two}",None),
        ("@book{café😀,inherit={pére😀}} @book{pére😀,inherit={café😀}}",Some(config)),
        ("@book{café😀,ids={first😀},crossref={second😀}} @book{pére😀,ids={second😀},crossref={first😀}}",None),
    ] {
        let directory=tempfile::tempdir().unwrap();let root=directory.path();
        let (success,log,diagnostics)=native(root,source,config);
        assert!(!success,"{log}");
        let items=items(&diagnostics,&uri(root,"refs.data"));
        assert_eq!(items.len(),1,"{source}\n{log}\n{diagnostics:?}");
        let item=&items[0];let data=item.data.as_ref().unwrap();
        let key=raw_key(data["key"].as_str().unwrap());
        let text=Text::new(source.into());
        let syntax=latex_lsp::syntax::Syntax::bibtex(&text);
        let entry=syntax.citations.iter().find(|entry|entry.name==key).unwrap();
        assert_eq!(item.range,text.range(entry.span.start,entry.span.end));
        assert_eq!(data["unresolvedField"],true);
    }
}
