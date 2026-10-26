use lsp_types::{NumberOrString, Url};
use texquill::compilation::build_log;

#[test]
fn continued_source_names_outgrow_existing_and_unavailable_prefix_files() {
    for available in [false, true] {
        for existing_prefix in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let prefix = format!("{}中文.tex", "x".repeat(65));
            let name = format!("{prefix}(extra)中文.tex");
            let child = root.join(&name);
            std::fs::write(&main, "\\bad").unwrap();
            if available {
                std::fs::write(&child, "\\bad").unwrap();
            }
            if existing_prefix {
                std::fs::write(root.join(&prefix), "\\bad").unwrap();
            }
            for entry in [format!("./{prefix}\n(extra)中文.tex"), format!("./{name}")] {
                let log = format!(
                    "(./main.tex\n({entry}\n! Undefined control sequence.\nl.1 \\bad\n)\n)\n"
                );
                let diagnostics = build_log::parse(&main, &log);
                assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
                if available {
                    let error = &diagnostics[&Url::from_file_path(&child).unwrap()][0];
                    assert_eq!(
                        error.code,
                        Some(NumberOrString::String("undefined-control-sequence".into()))
                    );
                    assert_eq!(error.range.start.character, 0);
                    assert_eq!(error.range.end.character, 4);
                } else {
                    let error = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
                    assert_eq!(error.code, None);
                    assert_eq!(
                        error.data,
                        Some(serde_json::json!({"unresolvedSource":true}))
                    );
                }
            }
        }
    }
    // File metadata after a complete source name is not part of its path.
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let name = format!("{}.tex", "x".repeat(72));
    let child = directory.path().join(&name);
    std::fs::write(&main, "Root.").unwrap();
    std::fs::write(&child, "\\bad").unwrap();
    let log = format!("(./main.tex\n(./{name}\nFile: other.tex 2026\n! Undefined control sequence.\nl.1 \\bad\n)\n)\n");
    let diagnostics = build_log::parse(&main, &log);
    assert_eq!(
        diagnostics[&Url::from_file_path(&child).unwrap()][0].code,
        Some(NumberOrString::String("undefined-control-sequence".into()))
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn real_tex_print_boundaries_do_not_reassign_long_names_to_short_existing_sources() {
    for existing_prefix in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let prefix = format!("{}.tex", "x".repeat(72));
        let name = format!("{prefix}(extra).tex");
        let child = root.join(&name);
        std::fs::write(
            &main,
            format!(
                "\\documentclass{{article}}\\begin{{document}}\\input{{{name}}}\\end{{document}}\n"
            ),
        )
        .unwrap();
        std::fs::write(&child, "\\bad\n").unwrap();
        if existing_prefix {
            std::fs::write(root.join(&prefix), "\\bad\n").unwrap();
        }
        let output = std::process::Command::new("pdflatex")
            .current_dir(root)
            .env("max_print_line", "79")
            .args(["-interaction=nonstopmode", "-halt-on-error", "main.tex"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let log = std::fs::read_to_string(main.with_extension("log")).unwrap();
        assert!(log.contains(&format!("(./{prefix}\n(extra).tex")), "{log}");
        let diagnostics = build_log::parse(&main, &log);
        assert!(
            !diagnostics.contains_key(&Url::from_file_path(root.join(&prefix)).unwrap()),
            "{diagnostics:?}"
        );
        let errors = &diagnostics[&Url::from_file_path(&child).unwrap()];
        assert!(
            errors
                .iter()
                .any(|d| d.message == "Undefined control sequence."
                    && d.code == Some(NumberOrString::String("undefined-control-sequence".into()))),
            "{diagnostics:?}"
        );
        std::fs::remove_file(&child).unwrap();
        let after = build_log::parse(&main, &log);
        assert!(
            !after.contains_key(&Url::from_file_path(root.join(&prefix)).unwrap()),
            "{after:?}"
        );
        let fallback = &after[&Url::from_file_path(&main).unwrap()];
        assert!(
            fallback
                .iter()
                .any(|d| d.message == "Undefined control sequence."
                    && d.code.is_none()
                    && d.data == Some(serde_json::json!({"unresolvedSource":true}))),
            "{after:?}"
        );
    }
}
