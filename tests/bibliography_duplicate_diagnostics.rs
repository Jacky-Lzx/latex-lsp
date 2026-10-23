use latex_lsp::{build_log, text::Text};
use std::{collections::BTreeMap, path::Path};
use tower_lsp::lsp_types::*;

const CODE: &str = "repeated-bibliography-entry";

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

fn parse(root: &Path, files: &[(&str, &str)], log: &str) -> BTreeMap<Url, Vec<Diagnostic>> {
    std::fs::write(root.join("main.tex"), r"\documentclass{article}").unwrap();
    for (name, source) in files {
        std::fs::write(root.join(name), source).unwrap();
    }
    build_log::parse_bibliography(&root.join("main.tex"), log)
}

fn repeated(diagnostics: &[Diagnostic]) -> &Diagnostic {
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == Some(NumberOrString::String(CODE.into())))
        .unwrap()
}

#[test]
fn repeated_entries_use_exact_utf16_ranges_and_link_to_earlier_case_insensitive_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source =
        "% 😀\r\n@misc{Entry😀,title={First}}\r\n@misc\r\n{eNTRY😀\r\n,title={Second}}\r\n";
    let found = parse(root, &[("refs.bib", source)], "Database file #1: refs.bib\nRepeated entry---line 4 of file refs.bib\n : {eNTRY😀\n :           \nI'm skipping whatever remains of this entry\n");
    let diagnostic = repeated(&found[&uri(root, "refs.bib")]);
    let text = Text::new(source.into());
    let offset = source.find("eNTRY😀").unwrap();
    assert_eq!(
        diagnostic.range,
        text.range(offset, offset + "eNTRY😀".len())
    );
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(diagnostic.source.as_deref(), Some("latex-build"));
    assert_eq!(diagnostic.message, "BibTeX: Repeated entry");
    assert_eq!(diagnostic.data.as_ref().unwrap()["key"], "eNTRY😀");
    let related = diagnostic.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    let offset = source.find("Entry😀").unwrap();
    assert_eq!(
        related[0].location,
        Location::new(
            uri(root, "refs.bib"),
            text.range(offset, offset + "Entry😀".len())
        )
    );
}

#[test]
fn related_entries_follow_observed_database_order_and_exclude_later_declarations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let files = [
        ("defs.bib", "@misc{entry,title={First}}"),
        (
            "refs.bib",
            "@misc{Entry,title={Second}}\n@misc{ENTRY,title={Later}}",
        ),
        ("later.bib", "@misc{entry,title={Other database}}"),
        ("unread.bib", "@misc{entry,title={Independent}}"),
    ];
    let found = parse(root, &files, "Database file #1: defs.bib\nDatabase file #2: refs.bib\nRepeated entry---line 1 of file refs.bib\nDatabase file #3: later.bib\n");
    let diagnostic = repeated(&found[&uri(root, "refs.bib")]);
    let related = diagnostic.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].location.uri, uri(root, "defs.bib"));
    assert_eq!(
        related[0].location.range,
        Range::new(Position::new(0, 6), Position::new(0, 11))
    );
}

#[test]
fn strings_literals_and_comments_never_supply_duplicate_entry_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@string{entry={Text}}\n@misc{other,title={entry}}\n% @misc{entry,title={No}}\n@misc{entry,title={Repeated}}";
    let found = parse(
        root,
        &[("refs.bib", source)],
        "Database file #1: refs.bib\nRepeated entry---line 4 of file refs.bib\n",
    );
    let diagnostic = repeated(&found[&uri(root, "refs.bib")]);
    assert_eq!(
        diagnostic.range,
        Range::new(Position::new(3, 6), Position::new(3, 11))
    );
    assert!(diagnostic.related_information.is_none());
}

#[test]
fn ambiguous_or_non_entry_lines_keep_the_original_line_diagnostic() {
    for source in [
        "@misc{entry,title={First}} @book{entry,title={Second}}",
        "@misc{entry,title={First}} @book{different,title={Second}}",
        "% entry",
        "@string{entry={Value}}",
        "@misc{entry with spaces,title={Value}}",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let found = parse(
            root,
            &[("refs.bib", source)],
            "Database file #1: refs.bib\nRepeated entry---line 1 of file refs.bib\n",
        );
        let diagnostic = repeated(&found[&uri(root, "refs.bib")]);
        assert_eq!(
            diagnostic.data.as_ref().unwrap()["unresolvedEntry"],
            true,
            "{source}"
        );
        assert!(diagnostic.related_information.is_none());
        assert_eq!(
            diagnostic.range.end.character,
            source.encode_utf16().count() as u32
        );
    }
}

#[test]
fn missing_sources_and_invalid_lines_never_borrow_a_fallback_key() {
    for (name, line) in [
        ("missing.bib", 1),
        ("refs.bib", 0),
        ("refs.bib", 400),
        ("main.aux", 1),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let log =
            format!("Database file #1: refs.bib\nRepeated entry---line {line} of file {name}\n");
        let found = parse(
            root,
            &[
                ("refs.bib", "@misc{entry,title={First}}"),
                ("main.aux", "@misc{entry,title={Unrelated}}"),
            ],
            &log,
        );
        let target = if name == "refs.bib" {
            "refs.bib"
        } else {
            "main.tex"
        };
        let diagnostic = repeated(&found[&uri(root, target)]);
        assert!(diagnostic.data.as_ref().unwrap().get("key").is_none());
        assert!(diagnostic.related_information.is_none());
        assert_eq!(
            diagnostic.data.as_ref().unwrap()[if name == "refs.bib" {
                "unresolvedLine"
            } else {
                "unresolvedSource"
            }],
            true
        );
    }
}

#[test]
fn repeated_entry_related_information_has_a_fixed_limit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@misc{entry,title={Text}}\n".repeat(33);
    let found = parse(
        root,
        &[("refs.bib", &source)],
        "Database file #1: refs.bib\nRepeated entry---line 33 of file refs.bib\n",
    );
    let diagnostic = repeated(&found[&uri(root, "refs.bib")]);
    assert_eq!(diagnostic.related_information.as_ref().unwrap().len(), 16);
    assert!(diagnostic
        .related_information
        .as_ref()
        .unwrap()
        .iter()
        .all(|related| related.location.range.start.line < 32));
}

#[test]
#[cfg(unix)]
fn aliases_keep_database_roles_and_related_locations_on_physical_files() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("physical.data"),
        "@misc{entry,title={First}}\n@misc{entry,title={Second}}",
    )
    .unwrap();
    symlink("physical.data", root.join("alias.bib")).unwrap();
    let found = parse(
        root,
        &[],
        "Database file #1: alias.bib\nRepeated entry---line 2 of file alias.bib\n",
    );
    let diagnostic = repeated(&found[&uri(root, "physical.data")]);
    assert_eq!(
        diagnostic.range,
        Range::new(Position::new(1, 6), Position::new(1, 11))
    );
    assert_eq!(
        diagnostic.related_information.as_ref().unwrap()[0]
            .location
            .uri,
        uri(root, "physical.data")
    );
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_duplicate_errors_preserve_multiline_keys_and_database_origins() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "@misc\n{Entry\n,title={Second}}\n";
    parse(
        root,
        &[
            ("defs.bib", "@misc{entry,title={First}}"),
            ("refs.bib", source),
        ],
        "",
    );
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{*}\n\\bibstyle{plain}\n\\bibdata{defs,refs}\n",
    )
    .unwrap();
    let result = std::process::Command::new("bibtex")
        .arg("main")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
    assert!(
        log.contains("Repeated entry---line 2 of file refs.bib"),
        "{log}"
    );
    let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
    let diagnostic = repeated(&found[&uri(root, "refs.bib")]);
    assert_eq!(
        diagnostic.range,
        Range::new(Position::new(1, 1), Position::new(1, 6))
    );
    let related = diagnostic.related_information.as_ref().unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].location.uri, uri(root, "defs.bib"));
}
