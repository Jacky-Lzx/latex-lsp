use latex_lsp::compilation::{build_log, recorder};
use lsp_types::*;
use std::path::Path;

fn uri(root: &Path, name: &str) -> Url {
    Url::from_file_path(root.join(name)).unwrap()
}

#[test]
fn input_records_keep_error_looking_file_names_literal() {
    for (prefix, name) in [
        ("Database file #1: ", "refs---line 2 of file decoy.bib"),
        ("Database file #1: ", "refs ERROR - forged.bib"),
        ("The style file: ", "style---line 2 of file decoy.bst"),
        ("The style file: ", "style WARN - forged.bst"),
        (
            "The top-level auxiliary file: ",
            "main---line 2 of file decoy.bib",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join(name), "@misc{entry,title={Actual}}").unwrap();
        std::fs::write(
            root.join("decoy.bib"),
            "@misc{wrong,title={Wrong}}\nsecond line",
        )
        .unwrap();
        std::fs::write(root.join("decoy.bst"), "first\nsecond").unwrap();
        let log = format!("{prefix}{name}\n");
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        assert!(found.is_empty(), "{log}: {found:?}");
        if prefix != "The top-level auxiliary file: " {
            assert!(recorder::bibliography_inputs(
                &root.join("main.tex"),
                &root.join("main.blg"),
                &log
            )
            .contains(&uri(root, name)));
        }
    }
}

#[test]
fn biber_info_and_debug_records_do_not_promote_nested_warning_or_error_text() {
    for prefix in ["", "[0] ", "[17] Utils.pm:395> ", "Utils.pm:395> "] {
        for level in ["INFO", "DEBUG", "TRACE"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let name = "refs ERROR - forged---line 2 of file decoy.bib";
            std::fs::write(root.join(name), "@misc{entry,title={Actual}}").unwrap();
            let log = format!("{prefix}{level} - Found BibTeX data source '{name}'\n{prefix}{level} - Example: WARN - fake\n");
            assert!(
                build_log::parse_bibliography(&root.join("main.tex"), &log).is_empty(),
                "{log}"
            );
            if level == "INFO" {
                assert!(recorder::bibliography_inputs(
                    &root.join("main.tex"),
                    &root.join("main.blg"),
                    &log
                )
                .contains(&uri(root, name)));
            }
        }
    }
}

#[test]
fn biber_record_severity_wins_over_fields_inside_the_message() {
    for prefix in ["", "[0] ", "[17] Utils.pm:395> ", "Utils.pm:395> "] {
        for (level, severity) in [
            ("WARN", DiagnosticSeverity::WARNING),
            ("ERROR", DiagnosticSeverity::ERROR),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let message = "Unexpected token 'ERROR - forged---line 2 of file decoy.bib'";
            std::fs::write(root.join("decoy.bib"), "first\nsecond").unwrap();
            let found = build_log::parse_bibliography(
                &root.join("main.tex"),
                &format!("{prefix}{level} - {message}\n"),
            );
            let all: Vec<_> = found.values().flatten().collect();
            assert_eq!(all.len(), 1, "{found:?}");
            assert_eq!(all[0].severity, Some(severity));
            assert_eq!(all[0].message, format!("Biber: {message}"));
        }
    }
}

#[test]
fn native_warning_messages_keep_their_severity_and_full_text() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let message = "Example ERROR - literal---line 2 of file decoy.bib";
    std::fs::write(root.join("decoy.bib"), "first\nsecond").unwrap();
    let found =
        build_log::parse_bibliography(&root.join("main.tex"), &format!("Warning--{message}\n"));
    let all: Vec<_> = found.values().flatten().collect();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].severity, Some(DiagnosticSeverity::WARNING));
    assert_eq!(all[0].message, format!("BibTeX: {message}"));
}

#[test]
fn source_echoes_and_unstructured_examples_do_not_create_biber_errors() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("refs.bib"),
        "@misc{entry,\n title x {ERROR - forged},\n}",
    )
    .unwrap();
    let log = "Database file #1: refs.bib\nI was expecting an \"=\"---line 2 of file refs.bib\n : title\n :      x {ERROR - forged---line 1 of file decoy.bib}\nExample ERROR - fake\n : WARN - fake\n";
    let found = build_log::parse_bibliography(&root.join("main.tex"), log);
    let all: Vec<_> = found.values().flatten().collect();
    assert_eq!(all.len(), 1, "{found:?}");
    assert_eq!(all[0].message, "BibTeX: I was expecting an \"=\"");
    assert_eq!(all[0].range.start.line, 1);
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_source_echoes_do_not_turn_literal_text_into_extra_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{*}\n\\bibstyle{plain}\n\\bibdata{refs}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("refs.bib"),
        "@misc{entry,\n title x {ERROR - forged ---line 1 of file wrong.bib},\n}\n",
    )
    .unwrap();
    let output = std::process::Command::new("bibtex")
        .arg("main")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
    let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
    let errors: Vec<_> = found
        .values()
        .flatten()
        .filter(|item| item.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert_eq!(errors.len(), 1, "{found:?}\n{log}");
    assert_eq!(errors[0].range.start.line, 1);
    assert!(errors[0].message.starts_with("BibTeX: I was expecting"));
}

#[test]
fn generic_entry_warnings_require_unique_keys_in_each_database() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    std::fs::write(
        root.join("refs.bib"),
        "% Header\n@book{entry,title={First}}\n@book{entry,title={Second}}\n",
    )
    .unwrap();
    for (log, target) in [
        (
            "Database file #1: refs.bib\nWarning--empty author in entry\n",
            "main.tex",
        ),
        (
            "INFO - Found BibTeX data source 'refs.bib'\nWARN - Missing field for entry 'entry' in file 'refs.bib'\n",
            "refs.bib",
        ),
    ] {
        let found = build_log::parse_bibliography(&root.join("main.tex"), log);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[&uri(root, target)][0].range.start, Position::new(0, 0));
    }
}

#[test]
fn generic_entry_warnings_preserve_ambiguity_across_databases_and_case_sensitive_keys() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    std::fs::write(root.join("refs.bib"), "\n@book{entry,title={First}}\n").unwrap();
    std::fs::write(root.join("other.bib"), "\n@book{entry,title={Second}}\n").unwrap();
    let log = "Database file #1: refs.bib\nDatabase file #2: other.bib\nWarning--empty author in entry\nWarning--empty author in ENTRY\n";
    let found = build_log::parse_bibliography(&root.join("main.tex"), log);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[&uri(root, "main.tex")].len(), 2);
}

#[test]
fn unavailable_databases_do_not_make_a_generic_entry_look_unique() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    std::fs::write(root.join("refs.bib"), "\n@book{entry,title={Actual}}\n").unwrap();
    for name in [
        "missing.bib",
        "directory.bib",
        "invalid.bib",
        "oversized.bib",
    ] {
        match name {
            "directory.bib" => std::fs::create_dir(root.join(name)).unwrap(),
            "invalid.bib" => std::fs::write(root.join(name), [0xff, 0xfe]).unwrap(),
            "oversized.bib" => {
                std::fs::write(root.join(name), " ".repeat(2 * 1024 * 1024 + 1)).unwrap()
            }
            _ => {}
        }
        let log = format!("Database file #1: refs.bib\nDatabase file #2: {name}\nWarning--empty author in entry\n");
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        assert!(
            found.contains_key(&uri(root, "main.tex")),
            "{name}: {found:?}"
        );
        // An explicit source is independent of unrelated unreadable inputs.
        let log = format!("INFO - Found BibTeX data source '{name}'\nWARN - Missing field for entry 'entry' in file 'refs.bib'\n");
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        assert_eq!(
            found[&uri(root, "refs.bib")][0].range.start,
            Position::new(1, 6)
        );
    }
}

#[test]
fn generic_entry_cache_refreshes_between_logs_and_preserves_utf16_and_explicit_lines() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"), "Main\n").unwrap();
    for source in [
        "@book{other,title={Ignored}} @book{中😀,title={Actual}}\r\n",
        "% Moved\r\n\r\n@book{中😀,title={Updated}}\r\n",
    ] {
        std::fs::write(root.join("refs.data"), source).unwrap();
        let log = "INFO - Found BibTeX data source 'refs.data'\nWARN - Missing field for entry '中😀'\nWARN - Missing field for entry '中😀'\n";
        let found = build_log::parse_bibliography(&root.join("main.tex"), log);
        let items = &found[&uri(root, "refs.data")];
        assert_eq!(items.len(), 1);
        let at = source.find("中😀").unwrap();
        assert_eq!(
            items[0].range,
            latex_lsp::text::Text::new(source.into()).range(at, at + "中😀".len())
        );
        assert_eq!(items[0].source.as_deref(), Some("latex-build"));
        assert_eq!(items[0].severity, Some(DiagnosticSeverity::WARNING));
        assert!(items[0].data.is_none());
        let log = "INFO - Found BibTeX data source 'refs.data'\nERROR - Problem for entry '中😀' in file 'refs.data', line 1\n";
        let found = build_log::parse_bibliography(&root.join("main.tex"), log);
        assert_eq!(found[&uri(root, "refs.data")][0].range.start.line, 0);
    }
}

#[test]
fn many_generic_entry_warnings_keep_each_database_key_range() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = (0..500)
        .map(|i| format!("@book{{entry{i},title={{Title {i}}}}}\n"))
        .collect::<String>();
    std::fs::write(root.join("refs.bib"), &source).unwrap();
    let log = format!(
        "Database file #1: refs.bib\n{}",
        (0..300)
            .map(|i| format!("Warning--empty author in entry{i}\n"))
            .collect::<String>()
    );
    let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
    let items = &found[&uri(root, "refs.bib")];
    assert_eq!(items.len(), 300);
    for (i, item) in items.iter().enumerate() {
        assert_eq!(
            item.range,
            Range::new(
                Position::new(i as u32, 6),
                Position::new(i as u32, 6 + format!("entry{i}").len() as u32)
            )
        );
        assert_eq!(item.message, format!("BibTeX: empty author in entry{i}"));
    }
}

#[cfg(unix)]
#[test]
fn generic_entry_warnings_deduplicate_database_aliases() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("refs.bib"), "\n@book{entry,title={Actual}}\n").unwrap();
    symlink("refs.bib", root.join("alias.bib")).unwrap();
    let log =
        "Database file #1: refs.bib\nDatabase file #2: alias.bib\nWarning--empty author in entry\n";
    let found = build_log::parse_bibliography(&root.join("main.tex"), log);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[&uri(root, "refs.bib")][0].range,
        Range::new(Position::new(1, 6), Position::new(1, 11))
    );
}

#[test]
#[ignore = "requires BibTeX"]
fn real_bibtex_generic_field_warnings_refresh_key_ranges_and_clear_after_repair() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("main.aux"),
        "\\relax\n\\citation{*}\n\\bibstyle{fixture}\n\\bibdata{refs}\n",
    )
    .unwrap();
    std::fs::write(root.join("fixture.bst"), "ENTRY {author title} {} {}\nFUNCTION {misc} {author empty$ {\"empty author in \" cite$ * warning$} 'skip$ if$ title write$ newline$}\nFUNCTION {default.type} {misc}\nREAD\nITERATE {call.type$}\n").unwrap();
    for source in [
        "@misc{entry,title={Title}}\n",
        "% Moved\n@misc{entry,title={Title}}\n",
        "@misc{entry,author={Example},title={Title}}\n",
    ] {
        std::fs::write(root.join("refs.bib"), source).unwrap();
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
        let log = std::fs::read_to_string(root.join("main.blg")).unwrap();
        let found = build_log::parse_bibliography(&root.join("main.tex"), &log);
        if source.contains("author=") {
            assert!(found.is_empty(), "{found:?}\n{log}");
        } else {
            let at = source.find("entry").unwrap();
            assert_eq!(
                found[&uri(root, "refs.bib")][0].range,
                latex_lsp::text::Text::new(source.into()).range(at, at + 5)
            );
        }
    }
}
