use latex_lsp::{config::Config, text::Text, workspace::Workspace};
use lsp_types::*;

const CODE: &str = "repeated-bibliography-field";
fn diagnostics(ws: &Workspace, uri: &Url) -> Vec<Diagnostic> {
    ws.diagnostics(uri)
        .into_iter()
        .filter(|item| {
            item.source.as_deref() == Some("latex-lsp")
                && item.code == Some(NumberOrString::String(CODE.into()))
        })
        .collect()
}
fn setup(source: &str) -> (Workspace, Url) {
    let uri = Url::parse("untitled:Library.bib").unwrap();
    let mut ws = Workspace::default();
    ws.open(uri.clone(), source.into(), 1);
    (ws, uri)
}
fn span(source: &str, value: &str) -> Range {
    let at = source.find(value).unwrap();
    Text::new(source.into()).range(at, at + value.len())
}

#[test]
fn checks_assigned_fields_case_insensitively_and_links_first_name_with_utf16_crlf() {
    let source = "% 😀\r\n@book{key,title={First},TiTlE={中},TITLE={Third},custom:field={one},CUSTOM:FIELD={two}}\r\n";
    let (ws, uri) = setup(source);
    let items = diagnostics(&ws, &uri);
    assert_eq!(items.len(), 3);
    for (i, field) in ["TiTlE", "TITLE", "CUSTOM:FIELD"].iter().enumerate() {
        assert_eq!(items[i].range, span(source, field));
        assert_eq!(items[i].severity, Some(DiagnosticSeverity::WARNING));
        let first = if i == 2 { "custom:field" } else { "title" };
        assert_eq!(
            items[i].related_information.as_ref().unwrap()[0].location,
            Location::new(uri.clone(), span(source, first))
        );
    }
}

#[test]
fn only_same_entry_fields_collide_and_comments_literals_special_entries_are_opaque() {
    let source = "% @book{fake,title={X},title={Y}}\n@comment{title={X},title={Y}}\n@string{title={X}}\n@book{a,title={title={X},title={Y}}}\n@book(b,title=\"first\",note={title={X}},title=prefix # {second})\n@book{c,title={Unrelated}}";
    let (ws, uri) = setup(source);
    let items = diagnostics(&ws, &uri);
    assert_eq!(items.len(), 1, "{items:?}");
    let duplicate = source.find("title=prefix").unwrap();
    assert_eq!(
        items[0].range,
        Text::new(source.into()).range(duplicate, duplicate + 5)
    );
    assert_eq!(items[0].data.as_ref().unwrap()["field"], "title");
    let (ws, uri) = setup("@book{k,FÖ={one},fö={two}}@book{k2,FÖ={one},FÖ={two}}");
    assert_eq!(diagnostics(&ws, &uri).len(), 1);
}

#[test]
fn incomplete_assignments_do_not_warn_and_edits_or_configuration_clear_results() {
    for source in [
        "@book{k,title={one},TITLE}",
        "@book{k,title={one},TITLE=}",
        "@book{k,title={one},TITLE={unfinished",
        "@book{k,title=not valid,TITLE={two}}",
    ] {
        let (ws, uri) = setup(source);
        assert!(diagnostics(&ws, &uri).is_empty(), "{source}");
    }
    let (ws, uri) = setup("@book{k,title={one},TITLE={two}");
    assert_eq!(diagnostics(&ws, &uri).len(), 1);
    let source = "@book{k,title={one},TITLE={two}}";
    let (mut ws, uri) = setup(source);
    assert_eq!(diagnostics(&ws, &uri).len(), 1);
    ws.open(uri.clone(), "@book{k,title={one},note={two}}".into(), 2);
    assert!(diagnostics(&ws, &uri).is_empty());
    ws.open(uri.clone(), source.into(), 3);
    assert_eq!(diagnostics(&ws, &uri).len(), 1);
    ws.config =
        Config::parse(serde_json::json!({"diagnostics":{"duplicateBibliographyFields":false}}))
            .unwrap();
    assert!(diagnostics(&ws, &uri).is_empty());
}

#[test]
fn compiler_field_locations_suppress_only_precisely_resolved_static_duplicates() {
    let source = "@book{k,title={one},TITLE={two},TiTlE={three}}";
    let (mut ws, uri) = setup(source);
    let root = Url::parse("file:///tmp/static-fields-main.tex").unwrap();
    let duplicate = span(source, "TITLE");
    let compiler = Diagnostic {
        range: duplicate,
        source: Some("latex-build".into()),
        code: Some(NumberOrString::String(CODE.into())),
        data: Some(serde_json::json!({"key":"k","field":"title"})),
        message: "BibTeX: extra field".into(),
        ..Default::default()
    };
    ws.build_diagnostics.insert(
        root.clone(),
        std::collections::BTreeMap::from([(uri.clone(), vec![compiler.clone()])]),
    );
    let items = diagnostics(&ws, &uri);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].range, span(source, "TiTlE"));
    let mut unresolved = compiler;
    unresolved.data = Some(serde_json::json!({"field":"title","unresolvedField":true}));
    ws.build_diagnostics
        .get_mut(&root)
        .unwrap()
        .insert(uri.clone(), vec![unresolved]);
    assert_eq!(diagnostics(&ws, &uri).len(), 2);
}

#[test]
fn indexes_many_unicode_duplicate_fields_in_one_line_without_repeated_line_scans() {
    let source = format!("@book{{key,title={{First}},{}note={{Last}}}}\r\n@book{{other,title={{First}},TITLE={{Second}}}}",
        "TITLE={😀},".repeat(3000));
    let (ws, uri) = setup(&source);
    let items = diagnostics(&ws, &uri);
    assert_eq!(items.len(), 3001);
    let text = Text::new(source.clone());
    for (item, (at, _)) in items.iter().zip(source.match_indices("TITLE")) {
        assert_eq!(item.range, text.range(at, at + 5));
        assert_eq!(item.related_information.as_ref().unwrap().len(), 1);
    }
}

#[test]
fn editor_language_and_project_input_roles_control_which_documents_are_checked() {
    let mut ws = Workspace::default();
    let uri = Url::parse("untitled:Library.tex").unwrap();
    let source = "@book{k,title={one},TITLE={two}}";
    ws.open_with_language(uri.clone(), source.into(), 1, "bibtex");
    assert_eq!(diagnostics(&ws, &uri).len(), 1);
    ws.open_with_language(uri.clone(), source.into(), 2, "latex");
    assert!(diagnostics(&ws, &uri).is_empty());
    let style = Url::parse("untitled:Library.bst").unwrap();
    ws.open(style.clone(), source.into(), 1);
    assert!(diagnostics(&ws, &style).is_empty());
}

#[test]
fn directory_configuration_disables_static_fields_without_affecting_compiler_warnings() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join(".latex-lsp.json"),
        r#"{"diagnostics":{"duplicateBibliographyFields":false}}"#,
    )
    .unwrap();
    std::fs::write(
        directory.path().join("refs.bib"),
        "@book{k,title={one},TITLE={two}}",
    )
    .unwrap();
    let uri = Url::from_file_path(directory.path().join("refs.bib")).unwrap();
    let mut ws = Workspace::default();
    ws.add_root(directory.path().into());
    assert!(diagnostics(&ws, &uri).is_empty());
    let compiler = Diagnostic {
        source: Some("latex-build".into()),
        code: Some(NumberOrString::String(CODE.into())),
        message: "Native warning".into(),
        ..Default::default()
    };
    ws.build_diagnostics.insert(
        uri.clone(),
        std::collections::BTreeMap::from([(uri.clone(), vec![compiler.clone()])]),
    );
    assert!(ws.diagnostics(&uri).contains(&compiler));
}

#[test]
#[cfg(unix)]
fn bibliography_dependency_roles_enable_checks_for_symlink_targets_with_tex_suffixes() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("main.tex"),
        "\\documentclass{article}\\bibliography{refs}",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("payload.tex"),
        "@book{k,title={one},TITLE={two}}",
    )
    .unwrap();
    std::os::unix::fs::symlink("payload.tex", directory.path().join("refs.bib")).unwrap();
    let uri = Url::from_file_path(directory.path().join("payload.tex")).unwrap();
    let mut ws = Workspace::default();
    ws.add_root(directory.path().into());
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    ws.load_dependencies(&main, 0);
    assert_eq!(diagnostics(&ws, &uri).len(), 1);
    ws.open_with_language(
        uri.clone(),
        "@book{k,title={one},TITLE={two}}".into(),
        1,
        "latex",
    );
    assert_eq!(diagnostics(&ws, &uri).len(), 1);
}
