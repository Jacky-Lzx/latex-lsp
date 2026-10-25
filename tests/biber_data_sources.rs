use latex_lsp::{
    compilation::{build_log, recorder},
    text::Text,
};
use lsp_types::*;
use std::path::Path;

fn uri(path: &Path) -> Url {
    Url::from_file_path(path).unwrap()
}

#[test]
fn biber_database_inputs_preserve_nonstandard_suffixes_and_literal_names() {
    for name in [
        "refs.data",
        "references",
        "refs.json",
        "refs.tex",
        "refs.bst",
        "中文 refs.data",
        "author's refs,part.data",
        "refs.bib': copy.data",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(root.join(name), "@book{alpha,title={Alpha}}\n").unwrap();
        let log = format!("INFO - Found BibTeX data source '{name}'\n");
        assert_eq!(
            recorder::bibliography_inputs(&root.join("main.tex"), &root.join("main.blg"), &log),
            [uri(&root.join(name))].into(),
            "{name}"
        );
    }
}

#[test]
fn logged_biber_databases_locate_entry_warnings_without_a_bib_suffix() {
    for name in [
        "refs.data",
        "references",
        "refs.tex",
        "refs.bst",
        "author's refs,part.data",
        "refs.bib': copy.data",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "% comment\n@book{alpha,title={Alpha}}\n";
        std::fs::write(root.join(name), source).unwrap();
        std::fs::write(root.join("refs.bib"), "@book{alpha,title={Wrong}}\n").unwrap();
        for message in [
            format!("Duplicate entry key: 'alpha' in file '{name}', skipping ..."),
            format!("Datamodel: Entry 'alpha' ({name}): Missing mandatory field 'author'"),
            format!("Missing field for entry 'alpha' (file {name})"),
        ] {
            let log = format!("INFO - Found BibTeX data source '{name}'\nWARN - {message}\n");
            let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
            let key = source.find("alpha").unwrap();
            assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:?}");
            let item = &diagnostics[&uri(&root.join(name))][0];
            assert_eq!(
                item.range,
                Text::new(source.into()).range(key, key + 5),
                "{message}"
            );
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
        }
    }
}

#[test]
fn biber_database_roles_require_complete_input_records() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("refs.data"), "@book{alpha,title={Alpha}}\n").unwrap();
    for log in [
        "WARN - Found BibTeX data source 'refs.data'\n",
        "ERROR - Found BibTeX data source 'refs.data'\n",
        "INFO - Example: Found BibTeX data source 'refs.data'\n",
        "INFO - Found BibTeX data source 'refs.data\n",
        "INFO - Found BibTeX data source 'refs.data' trailing text\n",
        "Database file #1: refs.data\n",
    ] {
        assert!(
            recorder::bibliography_inputs(&root.join("main.tex"), &root.join("main.blg"), log)
                .is_empty(),
            "{log}"
        );
    }
    let log = "INFO - Found BibTeX data source 'missing.data'\nWARN - Datamodel: Entry 'alpha' (missing.data): Missing author\n";
    let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), log);
    assert!(diagnostics.contains_key(&uri(&root.join("main.tex"))));
    assert!(!diagnostics.contains_key(&uri(&root.join("refs.data"))));
    for message in [
        "Missing field for entry 'alpha' in file refs.data.backup",
        "Missing field for entry 'alpha' in file missing.other",
        "Missing field for entry 'alpha' in file 'missing.data'",
    ] {
        let log = format!("INFO - Found BibTeX data source 'refs.data'\nWARN - {message}\n");
        let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
        assert!(
            diagnostics.contains_key(&uri(&root.join("main.tex"))),
            "{message}: {diagnostics:?}"
        );
        assert!(
            !diagnostics.contains_key(&uri(&root.join("refs.data"))),
            "{message}: {diagnostics:?}"
        );
    }
}

#[test]
fn biber_database_roles_require_the_actual_info_record_level() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("refs.tex"), "@book{entry,title={Actual}}").unwrap();
    for record in [
        "WARN - Example INFO - Found BibTeX data source 'refs.tex'",
        "ERROR - INFO - Found BibTeX data source 'refs.tex'",
        "INFO - Example: INFO - Found BibTeX data source 'refs.tex'",
        "[0] Utils.pm:395> WARN - INFO - Found BibTeX data source 'refs.tex'",
        " : INFO - Found BibTeX data source 'refs.tex'",
        "Example INFO - Found BibTeX data source 'refs.tex'",
        "[invalid] INFO - Found BibTeX data source 'refs.tex'",
    ] {
        assert!(
            recorder::bibliography_inputs(
                &root.join("main.tex"),
                &root.join("main.blg"),
                &format!("{record}\n")
            )
            .is_empty(),
            "{record}"
        );
    }
    for prefix in ["", "[0] ", "[17] Utils.pm:395> ", "Utils.pm:395> "] {
        let log = format!("{prefix}INFO - Found BibTeX data source 'refs.tex'\n");
        assert_eq!(
            recorder::bibliography_inputs(&root.join("main.tex"), &root.join("main.blg"), &log),
            [uri(&root.join("refs.tex"))].into()
        );
    }
}

#[test]
#[ignore = "requires biber"]
fn real_biber_data_sources_preserve_dependencies_and_warning_locations() {
    for name in [
        "refs.data",
        "references",
        "refs.bst",
        "author's refs,part.data",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "% Duplicate keys have no unambiguous declaration range\n@book{alpha,title={Alpha},year={2020}}\n@book{alpha,title={Duplicate}}\n";
        std::fs::write(root.join(name), source).unwrap();
        let output = std::process::Command::new("biber")
            .args(["--tool", "--output-file=processed.bib", name])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(std::fs::read_to_string(root.join(name)).unwrap(), source);
        let log_path = root.join(format!("{name}.blg"));
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(log.contains("Duplicate entry key:"), "{log}");
        assert_eq!(
            recorder::bibliography_inputs(&root.join("main.tex"), &log_path, &log),
            [uri(&root.join(name))].into()
        );
        let diagnostics =
            build_log::parse_bibliography_log(&root.join("main.tex"), &log_path, &log);
        assert_eq!(
            diagnostics[&uri(&root.join(name))][0].range,
            Text::new(source.into()).range(0, source.find('\n').unwrap())
        );
    }
}

#[test]
fn biber_duplicate_key_fields_preserve_apostrophes_and_do_not_supply_file_names() {
    for key in ["entry.bib", "O'Connor2026", "book(old)"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("% 😀\n@book{{{key},title={{Alpha}}}}\n");
        std::fs::write(root.join("refs.data"), &source).unwrap();
        std::fs::write(root.join("entry.bib"), "@book{unrelated}\n").unwrap();
        let log = format!("INFO - Found BibTeX data source 'refs.data'\nWARN - Duplicate entry key: '{key}' in file 'refs.data', skipping ...\n");
        let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let item = &diagnostics[&uri(&root.join("refs.data"))][0];
        let start = source.find(key).unwrap();
        assert_eq!(
            item.range,
            Text::new(source).range(start, start + key.len())
        );
    }
}

#[test]
#[cfg(unix)]
fn biber_database_aliases_keep_the_canonical_source_and_exact_key_range() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let source = "% header\n@book{alpha,title={Alpha}}\n";
    let target = root.join("payload.tex");
    std::fs::write(&target, source).unwrap();
    std::os::unix::fs::symlink(&target, root.join("refs.data")).unwrap();
    for printed in ["payload.tex".into(), target.to_str().unwrap().to_owned()] {
        let log = format!("INFO - Found BibTeX data source 'refs.data'\nWARN - Missing field for entry 'alpha' in file {printed}\n");
        assert_eq!(
            recorder::bibliography_inputs(&root.join("main.tex"), &root.join("main.blg"), &log),
            [uri(&target)].into()
        );
        let diagnostics = build_log::parse_bibliography(&root.join("main.tex"), &log);
        let start = source.find("alpha").unwrap();
        assert_eq!(
            diagnostics[&uri(&target)][0].range,
            Text::new(source.into()).range(start, start + 5)
        );
    }
}
