use latex_lsp::{compilation::build_log, editing, project::workspace::Workspace, text::Text};
use lsp_types::*;
use std::{collections::BTreeMap, path::Path};

const CODE: &str = "undefined-bibliography-entry-type";

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn warning(key: &str, location: &str) -> String {
    format!("Warning--entry type for \"{key}\" isn't style-file defined\n{location}")
}

fn parse(root: &Path, source: &str, tail: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(
        root.join("main.tex"),
        r"\documentclass{article}\bibliography{refs}",
    )
    .unwrap();
    std::fs::write(root.join("refs.bib"), source).unwrap();
    build_log::parse_bibliography(
        &root.join("main.tex"),
        &format!("Database file #1: refs.bib\n{tail}"),
    )
}

fn diagnostic(found: &BTreeMap<Url, Vec<Diagnostic>>, target: &Url) -> Diagnostic {
    found[target]
        .iter()
        .find(|item| item.code == Some(NumberOrString::String(CODE.into())))
        .unwrap()
        .clone()
}

#[test]
fn native_two_line_type_warnings_highlight_the_associated_type_in_utf16_and_crlf() {
    for (kind, key) in [
        ("artcle", "entry"),
        ("custom_type", "book(old)"),
        ("éType😀", "键😀"),
        ("a@", "O'Connor\"2026"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("😀 @ {kind}\r\n({key},\r\n title={{@book{{fake}}}})\r\n");
        let found = parse(root, &source, &warning(key, "--line 2 of file refs.bib\n"));
        let item = diagnostic(&found, &uri(root, "refs.bib"));
        let offset = source.find(kind).unwrap();
        assert_eq!(
            item.range,
            Text::new(source).range(offset, offset + kind.len())
        );
        assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(item.source.as_deref(), Some("latex-build"));
        assert_eq!(item.data.as_ref().unwrap()["entryType"], kind);
        assert_eq!(item.data.as_ref().unwrap()["key"], key);
    }
}

#[test]
fn explicit_database_and_key_line_disambiguate_repeated_keys_and_other_types() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("other.bib"), "@wrong{entry,title={Other}}").unwrap();
    let source="@earlier{entry,title={First}}\n@bad{entry,title={Second}} @different{other,title={Other}}\n";
    let log = format!(
        "Database file #2: other.bib\n{}",
        warning("entry", "--line 2 of file refs.bib\n")
    );
    let found = parse(root, source, &log);
    assert_eq!(found.len(), 1);
    let item = diagnostic(&found, &uri(root, "refs.bib"));
    assert_eq!(
        item.range,
        Range::new(Position::new(1, 1), Position::new(1, 4))
    );
    assert_eq!(item.data.as_ref().unwrap()["entryType"], "bad");
}

#[test]
fn ambiguous_missing_or_case_mismatched_keys_never_borrow_another_type() {
    for source in [
        "@bad{entry,title={One}} @other{entry,title={Two}}",
        "@bad{ENTRY,title={Case mismatch}}",
        "@bad{other,title={entry}}",
        "@string{entry={Value}}",
        "@preamble{\"entry\"}",
        "@comment{@bad{entry,title={Hidden}}}",
        "% @bad{entry,title={Hidden}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            source,
            &warning("entry", "--line 1 of file refs.bib\n"),
        );
        let item = diagnostic(&found, &uri(root, "refs.bib"));
        assert_eq!(
            item.data.as_ref().unwrap()["unresolvedEntry"],
            true,
            "{source}"
        );
        assert_eq!(
            item.range.end.character,
            source.encode_utf16().count() as u32
        );
        assert!(item.data.as_ref().unwrap().get("entryType").is_none());
    }
}

#[test]
fn incomplete_bodies_keep_the_type_associated_with_the_read_header() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let found = parse(
        root,
        "@artcle\n{entry,title={Still being typed",
        &warning("entry", "--line 2 of file refs.bib\n"),
    );
    assert_eq!(
        diagnostic(&found, &uri(root, "refs.bib")).range,
        Range::new(Position::new(0, 1), Position::new(0, 7))
    );
}

#[test]
fn missing_unconfirmed_and_malformed_locations_keep_the_warning_on_the_root() {
    for location in [
        "",
        "--line +1 of file refs.bib\n",
        "--line 1x of file refs.bib\n",
        "\n--line 1 of file refs.bib\n",
        "--line 1 of file missing.bib\n",
        "--line 1 of file main.aux\n",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join("main.aux"), "@bad{entry,title={Unrelated}}").unwrap();
        let found = parse(
            root,
            "@bad{entry,title={Actual}}",
            &warning("entry", location),
        );
        assert_eq!(found.len(), 1, "{location}: {found:?}");
        let item = diagnostic(&found, &uri(root, "main.tex"));
        assert_eq!(item.data.as_ref().unwrap()["unresolvedSource"], true);
    }
    for line in [0, 1000] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            "@bad{entry,title={Actual}}",
            &warning("entry", &format!("--line {line} of file refs.bib\n")),
        );
        let item = diagnostic(&found, &uri(root, "refs.bib"));
        assert_eq!(item.data.as_ref().unwrap()["unresolvedLine"], true);
        assert_eq!(item.data.as_ref().unwrap()["reportedLine"], line);
    }
}

#[test]
#[cfg(unix)]
fn type_warnings_preserve_database_roles_through_aliases_and_literal_path_fields() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("physical.data"), "@artcle{entry,title={Actual}}").unwrap();
    let name = "alias---line 9 of file decoy.bib";
    symlink("physical.data", root.join(name)).unwrap();
    let log = format!(
        "Database file #1: {name}\n{}",
        warning("entry", &format!("--line 1 of file {name}\n"))
    );
    let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
    assert_eq!(found.len(), 1);
    let item = diagnostic(&found, &uri(root, "physical.data"));
    assert_eq!(
        item.range,
        Range::new(Position::new(0, 1), Position::new(0, 7))
    );
    for alias in ["main.aux", "local.bst"] {
        symlink("physical.data", root.join(alias)).unwrap();
        let log = format!(
            "Database file #1: {name}\n{}",
            warning("entry", &format!("--line 1 of file {alias}\n"))
        );
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        assert_eq!(
            diagnostic(&found, &uri(root, "main.tex"))
                .data
                .as_ref()
                .unwrap()["unresolvedSource"],
            true
        );
    }
}

#[test]
fn invalid_native_database_records_do_not_confirm_an_unrelated_suffix() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("refs.data"), "@artcle{entry,title={Actual}}").unwrap();
    let log = format!(
        "Database file #1: refs.data\n{}",
        warning("entry", "--line 1 of file refs.data\n")
    );
    let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
    assert_eq!(
        diagnostic(&found, &uri(root, "main.tex"))
            .data
            .as_ref()
            .unwrap()["unresolvedSource"],
        true
    );
}

#[test]
fn unknown_style_types_do_not_offer_unverified_spelling_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@artcle{entry,title={Title}}\n@article{other,title={Other}}";
    let found = parse(
        root,
        source,
        &warning("entry", "--line 1 of file refs.bib\n"),
    );
    let target = uri(root, "refs.bib");
    let item = diagnostic(&found, &target);
    let mut workspace = Workspace {
        client: latex_lsp::project::state::ClientCapabilities {
            code_action_literals: true,
            ..Default::default()
        },
        ..Default::default()
    };
    workspace.add_root(root.into());
    workspace
        .project
        .build_diagnostics
        .insert(uri(root, "main.tex"), found);
    assert!(editing::code_actions(
        &workspace,
        &target,
        item.range,
        &CodeActionContext {
            diagnostics: vec![item],
            only: Some(vec![CodeActionKind::QUICKFIX]),
            trigger_kind: None
        }
    )
    .is_empty());
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_type_warnings_mark_the_header_and_clear_after_using_a_style_defined_type() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@artcle\n{entry,\n title={Expected title}\n}\n";
    parse(root, source, "");
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{ENTRY}\n\\bibstyle{fixture}\n\\bibdata{refs}\n",
    )
    .unwrap();
    std::fs::write(root.join("fixture.bst"),"ENTRY {title} {} {}\nFUNCTION {article} {title write$ newline$}\nFUNCTION {default.type} {article}\nREAD\nITERATE {call.type$}\n").unwrap();
    let compile = || {
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
        std::fs::read_to_string(root.join("main.blg")).unwrap()
    };
    let log = compile();
    assert!(log.contains("--line 2 of file refs.bib"));
    let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
    let item = diagnostic(&found, &uri(root, "refs.bib"));
    assert_eq!(
        item.range,
        Range::new(Position::new(0, 1), Position::new(0, 7))
    );
    assert_eq!(item.data.as_ref().unwrap()["key"], "entry");
    std::fs::write(root.join("refs.bib"), source.replace("artcle", "article")).unwrap();
    let log = compile();
    assert!(!log.contains("isn't style-file defined"), "{log}");
    assert!(!build_log::parse_bibliography(&root.join("main.tex"), &log)
        .values()
        .flatten()
        .any(|item| item.code == Some(NumberOrString::String(CODE.into()))));
    assert_eq!(
        std::fs::read_to_string(root.join("main.bbl"))
            .unwrap()
            .trim(),
        "Expected title"
    );
}
