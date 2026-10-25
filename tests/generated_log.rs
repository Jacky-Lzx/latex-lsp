use latex_lsp::{build, build_log, config::BuildConfig};
use lsp_types::{DiagnosticSeverity, Url};
use std::sync::atomic::AtomicBool;

#[test]
fn source_and_generated_files_keep_numbered_and_traditional_error_locations_and_file_stack_boundaries(
) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "First.\nSecond.\n").unwrap();
    let output = root.join("build (草稿)");
    std::fs::create_dir(&output).unwrap();
    for extension in [
        "aux", "bbl", "toc", "lof", "lot", "out", "nav", "snm", "vrb", "bbx", "cbx", "lbx", "dbx",
        "fd", "ldf",
    ] {
        let path = output.join(format!("main.{extension}"));
        std::fs::write(&path, "First.\n😀 \\bad\n").unwrap();
        let generated = Url::from_file_path(&path).unwrap();
        for quoted in [false, true] {
            let file = format!("./build (草稿)/main.{extension}");
            let file = if quoted { format!("\"{file}\"") } else { file };
            let log = format!("(./main.tex\n({file}\n! Undefined control sequence.\nl.2 \\bad\n)\nPackage main Warning: After generated input on input line 2.\n)\n");
            let diagnostics = build_log::parse(&main, &log);
            let errors = diagnostics
                .get(&generated)
                .unwrap_or_else(|| panic!("{extension}, quoted={quoted}: {diagnostics:?}"));
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0].range.start.line, 1);
            assert_eq!(errors[0].range.end.character, 7);
            assert_eq!(errors[0].severity, Some(DiagnosticSeverity::ERROR));
            let main_uri = Url::from_file_path(&main).unwrap();
            assert!(diagnostics[&main_uri]
                .iter()
                .all(|entry| entry.message.contains("After generated input")));
            let numbered =
                build_log::parse(&main, &format!("{file}:2: Undefined control sequence.\n"));
            assert_eq!(numbered[&generated][0].range.start.line, 1);
        }
    }
}

#[test]
fn generated_file_announcements_end_wrapped_warnings_and_recover_wrapped_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "Main.\n").unwrap();
    let name = format!("{} bibliography.bbl", "long-章节-".repeat(12));
    let generated = root.join(&name);
    std::fs::write(&generated, "First.\nSecond.\n").unwrap();
    let split = name.char_indices().nth(50).unwrap().0;
    let log = format!("(./main.tex\nPackage main Warning: Unfinished message\n(./{}\n{}\nPackage generated Warning: Inside bibliography on input line 2.\n)\n)\n", &name[..split], &name[split..]);
    let diagnostics = build_log::parse(&main, &log);
    let main_uri = Url::from_file_path(main).unwrap();
    assert_eq!(diagnostics[&main_uri].len(), 1);
    assert_eq!(
        diagnostics[&main_uri][0].message,
        "Package main Warning: Unfinished message"
    );
    let generated_uri = Url::from_file_path(generated).unwrap();
    assert_eq!(diagnostics[&generated_uri][0].range.start.line, 1);
}

#[test]
fn package_warning_continuations_with_source_and_auxiliary_names_remain_a_single_message() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Main.\nSecond.\n").unwrap();
    // A package continuation is not a file announcement merely because it
    // mentions a source or auxiliary suffix.
    for extension in [
        "tex", "bbl", "aux", "toc", "bbx", "cbx", "lbx", "dbx", "fd", "ldf",
    ] {
        let log = format!("(./main.tex\nPackage demo Warning: Continuing message\n(demo) check main.{extension} on input line 2.\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let main_uri = Url::from_file_path(&main).unwrap();
        assert_eq!(diagnostics[&main_uri].len(), 1);
        assert!(diagnostics[&main_uri][0]
            .message
            .contains(&format!("check main.{extension}")));
        assert_eq!(diagnostics[&main_uri][0].range.start.line, 1);
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_corrupt_sources_and_generated_files_point_to_the_loaded_file() {
    for file_line in [false, true] {
        for (extension, body) in [
            ("aux", "Text."),
            ("bbl", "Text.\\bibliography{references}"),
            ("toc", "\\tableofcontents Text."),
            ("bbx", ""),
            ("cbx", ""),
            ("lbx", ""),
            ("dbx", ""),
            ("fd", ""),
            ("ldf", ""),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let mut arguments = vec![
                "-interaction=nonstopmode".into(),
                "-halt-on-error".into(),
                "-no-shell-escape".into(),
                "-recorder".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ];
            if file_line {
                arguments.insert(0, "-file-line-error".into());
            }
            let config = BuildConfig {
                executable: "pdflatex".into(),
                arguments: Some(arguments),
                ..Default::default()
            };
            let path = config.artifact_path(&main, extension).unwrap();
            let body = if body.is_empty() {
                format!(
                    "Text.\\input{{{}}}",
                    path.strip_prefix(root).unwrap().display()
                )
            } else {
                body.into()
            };
            std::fs::write(
                &main,
                format!(
                    "\\documentclass{{article}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n"
                ),
            )
            .unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "\\relax\n\\badGeneratedCommand\n").unwrap();
            let outcome = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(
                outcome.status, "error",
                "{extension}, numbered={file_line}: {}",
                outcome.output
            );
            assert!(outcome.pdf.is_none());
            let uri = Url::from_file_path(&path).unwrap();
            assert_eq!(
                outcome.inputs.as_ref().unwrap().contains(&uri),
                matches!(extension, "bbx" | "cbx" | "lbx" | "dbx" | "fd" | "ldf"),
                "{extension}, numbered={file_line}: {:?}",
                outcome.inputs
            );
            let error = outcome
                .diagnostics
                .get(&uri)
                .and_then(|entries| {
                    entries
                        .iter()
                        .find(|entry| entry.message.contains("Undefined control sequence"))
                })
                .unwrap_or_else(|| {
                    panic!("{extension}: {:?}\n{}", outcome.diagnostics, outcome.output)
                });
            if std::fs::read_to_string(&path).unwrap().lines().count() < 2 {
                // LaTeX may truncate a .toc after reporting an error in its old
                // contents. Preserve the original line while returning a valid
                // location in the file which is now on disk.
                assert_eq!(error.range.start.line, 0);
                assert_eq!(error.data.as_ref().unwrap()["reportedLine"], 2);
                assert!(error.message.contains("reported line 2"));
            } else {
                assert_eq!(error.range.start.line, 1);
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_generated_bibliography_warnings_keep_their_path_without_indexing_artifacts_as_sources() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\nText.\\bibliography{references}\n\\end{document}\n").unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let path = config.artifact_path(&main, "bbl").unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "\\relax\n\\PackageWarning{generated}{Bibliography warning}\n",
    )
    .unwrap();
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    let uri = Url::from_file_path(&path).unwrap();
    assert!(
        outcome.diagnostics.get(&uri).is_some_and(|entries| entries
            .iter()
            .any(|entry| entry.message.contains("Bibliography warning")
                && entry.range.start.line == 1)),
        "{:?}",
        outcome.diagnostics
    );
    assert!(!outcome.inputs.unwrap().contains(&uri));
    assert!(!latex_lsp::workspace::is_source(&path));
}
