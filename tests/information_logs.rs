use latex_lsp::{build_log, text::Text};
use lsp_types::{DiagnosticSeverity, NumberOrString, Url};

fn assert_sources(info: &str) {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "\\unknownrootcommand\n").unwrap();
    std::fs::write(&child, "\\unknownchildcommand\n").unwrap();
    std::fs::write(
        directory.path().join("shadow.tex"),
        "\\unknownchildcommand\n",
    )
    .unwrap();
    let log = format!("(./main.tex\n(./child.tex\n{info}\nPackage local Warning: Child sentinel on input line 1.\n! Undefined control sequence.\nl.1 \\unknownchildcommand\n)\n! Undefined control sequence.\nl.1 \\unknownrootcommand\n)\n");
    let parsed = build_log::parse(&main, &log);
    assert_eq!(parsed.len(), 2, "{info}: {parsed:?}");
    for (path, name, count) in [
        (&main, "unknownrootcommand", 1),
        (&child, "unknownchildcommand", 2),
    ] {
        let items = &parsed[&Url::from_file_path(path).unwrap()];
        assert_eq!(items.len(), count, "{info}: {parsed:?}");
        let error = items
            .iter()
            .find(|item| item.severity == Some(DiagnosticSeverity::ERROR))
            .unwrap();
        assert_eq!(
            error.code,
            Some(NumberOrString::String("undefined-control-sequence".into())),
            "{info}: {error:?}"
        );
        let source = Text::new(std::fs::read_to_string(path).unwrap());
        assert_eq!(
            &source.source[source.offset(error.range.start).unwrap()
                ..source.offset(error.range.end).unwrap()],
            format!("\\{name}")
        );
    }
    assert_eq!(parsed.values().flatten().count(), 3, "{info}: {parsed:?}");
}

#[test]
fn information_prose_never_opens_or_closes_a_source() {
    for header in [
        "Package local Info:",
        "Class local Info:",
        "LaTeX Info:",
        "LaTeX Font Info:",
        "LaTeX Encoding Info:",
    ] {
        for prose in [
            "Example (shadow.tex",
            "Example (missing.tex",
            "Example )",
            "Example (shadow.tex)",
        ] {
            assert_sources(&format!("{header} {prose} on input line 1."));
        }
    }
}

#[test]
fn information_byte_wraps_and_owned_continuations_keep_the_source() {
    for width in [60, 79] {
        for (header, continuation) in [
            ("Package local Info: ", "(local) "),
            ("Class local Info: ", "(local) "),
            ("LaTeX Info: ", "            "),
            ("LaTeX Font Info: ", "(Font) "),
            ("LaTeX Encoding Info: ", "(Encoding) "),
        ] {
            for body in ["(shadow.tex", ")"] {
                let first = format!(
                    "{header}{}{body} on input line 1.",
                    "a".repeat(width - header.len())
                );
                let second = format!(
                    "{continuation}{}{body} on input line 1.",
                    "b".repeat(width - continuation.len())
                );
                let wrapped = [first, second]
                    .into_iter()
                    .map(|line| {
                        line.as_bytes()
                            .chunks(width)
                            .map(|chunk| std::str::from_utf8(chunk).unwrap())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                assert_sources(&wrapped);
            }
        }
    }
}

#[test]
fn owned_information_text_can_look_like_diagnostics_or_resource_output() {
    for (header, continuation) in [
        ("Package local Info:", "(local) "),
        ("Class local Info:", "(local) "),
        ("LaTeX Info:", "            "),
        ("LaTeX Font Info:", "(Font) "),
        ("LaTeX Encoding Info:", "(Encoding) "),
    ] {
        for body in [
            "<example (shadow.tex",
            "Here is how much (shadow.tex",
            "Package local Warning: Example (shadow.tex",
            "Class local Warning: Example (shadow.tex",
            "LaTeX Warning: Example (shadow.tex",
            "Package local Info: Example (shadow.tex",
            "Output written on (shadow.tex",
            "l.1 (shadow.tex",
            "./shadow.tex:1: Undefined control sequence.",
        ] {
            assert_sources(&format!(
                "{header} First sentence.\n{continuation}{body} on input line 1."
            ));
        }
        assert_sources(&format!("{header} First sentence.\n{continuation}(shadow.tex)\n{continuation}Example ) on input line 1."));
    }
}

#[test]
fn long_owned_information_sequences_and_no_line_checksums_are_not_sources() {
    assert_sources("LaTeX Info: First sentence.\n            \n            Example (shadow.tex on input line 1.");
    for (header, continuation) in [
        ("Package local Info:", "(local) "),
        ("Class local Info:", "(local) "),
        ("LaTeX Info:", "            "),
        ("LaTeX Font Info:", "(Font) "),
        ("LaTeX Encoding Info:", "(Encoding) "),
    ] {
        assert_sources(&format!(
            "{header} First sentence.\n{}",
            format!("{continuation}Example (shadow.tex\n").repeat(160)
        ));
    }
    for width in [60, 79] {
        let header = "Package rerunfilecheck Info: Checksums for `";
        let first = format!("{header}{}(shadow.tex':", "a".repeat(width - header.len()));
        let wrapped = first
            .as_bytes()
            .chunks(width)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert_sources(&format!(
            "{wrapped}\n(rerunfilecheck) Before: <no file>\n(rerunfilecheck) After: 12345.\n"
        ));
    }
}

#[test]
fn incomplete_information_preserves_real_file_entries_and_warning_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    std::fs::write(&child, "Child text.\n").unwrap();
    for width in [60, 79] {
        let prefix = "Package local Info: ";
        let info = format!("{prefix}{}", "a".repeat(width - prefix.len()));
        let log = format!("(./main.tex\nPackage local Warning: Unfinished warning\n{info}\n(./child.tex\nPackage local Warning: Child sentinel on input line 1.\n)\nPackage local Warning: Root sentinel on input line 1.\n)\n");
        let parsed = build_log::parse(&main, &log);
        assert_eq!(parsed.len(), 2, "{width}: {parsed:?}");
        let child_items = &parsed[&Url::from_file_path(&child).unwrap()];
        assert_eq!(child_items.len(), 1, "{width}: {parsed:?}");
        assert!(child_items[0].message.contains("Child sentinel"));
        let root_items = &parsed[&Url::from_file_path(&main).unwrap()];
        assert_eq!(root_items.len(), 2, "{width}: {parsed:?}");
        assert!(root_items
            .iter()
            .any(|item| item.message == "Package local Warning: Unfinished warning"));
        assert!(root_items
            .iter()
            .any(|item| item.message.contains("Root sentinel")));
    }
}

#[test]
fn information_lookahead_keeps_byte_wrapped_file_entries() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    for width in [60, 79] {
        let filename = format!("{}.tex", "a".repeat(width - 4));
        let child = directory.path().join(&filename);
        std::fs::write(&child, "Child text.\n").unwrap();
        let prefix = "Package local Info: ";
        let info = format!("{prefix}{}", "x".repeat(width - prefix.len()));
        let entry = format!("(./{filename}");
        let wrapped = entry
            .as_bytes()
            .chunks(width)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = build_log::parse(&main, &format!("(./main.tex\n{info}\n{wrapped}\nPackage local Warning: Child sentinel on input line 1.\n)\nPackage local Warning: Root sentinel on input line 1.\n)\n"));
        assert_eq!(parsed.len(), 2, "{width}: {parsed:?}");
        assert_eq!(parsed[&Url::from_file_path(&child).unwrap()].len(), 1);
        assert!(parsed[&Url::from_file_path(&child).unwrap()][0]
            .message
            .contains("Child sentinel"));
        assert_eq!(parsed[&Url::from_file_path(&main).unwrap()].len(), 1);
    }
}

#[test]
fn information_lookahead_keeps_byte_wrapped_explicit_error_paths() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Root text.\n").unwrap();
    for width in [60, 79] {
        let filename = format!("{}.tex", "b".repeat(width - 3));
        let child = directory.path().join(&filename);
        std::fs::write(&child, "\\unknownlongcommand\n").unwrap();
        let prefix = "Package local Info: ";
        let info = format!("{prefix}{}", "x".repeat(width - prefix.len()));
        let error = format!("./{filename}:1: Undefined control sequence.");
        let wrapped = error
            .as_bytes()
            .chunks(width)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = build_log::parse(&main, &format!("(./main.tex\n{info}\n{wrapped}\nl.1 \\unknownlongcommand\nPackage local Warning: Root sentinel on input line 1.\n)\n"));
        assert_eq!(parsed.len(), 2, "{width}: {parsed:?}");
        let errors = &parsed[&Url::from_file_path(&child).unwrap()];
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].code,
            Some(NumberOrString::String("undefined-control-sequence".into()))
        );
    }
}

#[test]
fn information_never_hides_a_truncation_barrier() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "\\unknownrootcommand\n").unwrap();
    std::fs::write(&child, "\\unknownchildcommand\n").unwrap();
    let info = format!("Package local Info: {}", "a".repeat(60));
    let parsed = build_log::parse(&main, &format!("(./main.tex\n{info}\n[latex-lsp: middle of compiler output omitted]\n! Undefined control sequence.\nl.1 \\unknownrootcommand\n(./child.tex\n! Undefined control sequence.\nl.1 \\unknownchildcommand\n)\n"));
    assert_eq!(parsed.len(), 2, "{parsed:?}");
    let root_error = &parsed[&Url::from_file_path(&main).unwrap()][0];
    assert!(root_error.code.is_none());
    assert_eq!(root_error.data.as_ref().unwrap()["unresolvedSource"], true);
    let child_error = &parsed[&Url::from_file_path(&child).unwrap()][0];
    assert_eq!(
        child_error.code,
        Some(NumberOrString::String("undefined-control-sequence".into()))
    );
}

#[test]
#[ignore = "requires pdfLaTeX, XeLaTeX and LuaLaTeX"]
fn real_information_messages_preserve_child_and_parent_error_locations() {
    let cache = tempfile::tempdir().unwrap();
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let main = directory.path().join("main.tex");
                let child = directory.path().join("child.tex");
                let main_source = "\\documentclass{article}\n\\makeatletter\n\\begin{document}Text.\n\\input{child}\n\\unknownrootcommand\n\\end{document}\n";
                let mut child_source = String::new();
                let mut expected = Vec::new();
                for command in [
                    "\\PackageInfo{local}",
                    "\\ClassInfo{local}",
                    "\\@latex@info",
                    "\\@font@info",
                    "\\@enc@info",
                ] {
                    for body in [
                        "Example (shadow.tex".into(),
                        "Example )".into(),
                        format!("{}(shadow.tex", "a".repeat(2 * width)),
                        "First sentence.\\MessageBreak Example (shadow.tex".into(),
                        "First sentence.\\MessageBreak\\MessageBreak Example (shadow.tex".into(),
                        "First sentence.\\MessageBreak <example (shadow.tex".into(),
                        "First sentence.\\MessageBreak Here is how much (shadow.tex".into(),
                        "First sentence.\\MessageBreak Package local Warning: Example (shadow.tex"
                            .into(),
                        "First sentence.\\MessageBreak ./shadow.tex:1: Undefined control sequence."
                            .into(),
                        "First sentence.\\MessageBreak (shadow.tex)\\MessageBreak Example )".into(),
                    ] {
                        let suffix = format!(
                            "{}{}",
                            char::from(b'a' + (expected.len() / 26) as u8),
                            char::from(b'a' + (expected.len() % 26) as u8)
                        );
                        child_source.push_str(&format!("{command}{{{body}}}\n\\PackageWarning{{local}}{{Child sentinel {suffix}}}\n"));
                        let line = child_source.lines().count() as u32;
                        let name = format!("\\unknownchild{suffix}");
                        child_source.push_str(&format!("{name}\\par\n"));
                        expected.push((line, name));
                    }
                }
                std::fs::write(&main, main_source).unwrap();
                std::fs::write(&child, &child_source).unwrap();
                std::fs::write(directory.path().join("shadow.tex"), &child_source).unwrap();
                let mut compiler = std::process::Command::new(engine);
                compiler
                    .current_dir(directory.path())
                    .env("TEXMFVAR", cache.path())
                    .env("TEXMFCACHE", cache.path())
                    .env("max_print_line", width.to_string())
                    .args(["-interaction=nonstopmode", "-no-shell-escape"]);
                if numbered {
                    compiler.arg("-file-line-error");
                }
                let output = compiler.arg("main.tex").output().unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(1),
                    "{engine}/{width}/{numbered}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
                let log = std::fs::read_to_string(directory.path().join("main.log")).unwrap();
                let parsed = build_log::parse(&main, &log);
                assert_eq!(parsed.len(), 2, "{engine}/{width}/{numbered}: {parsed:?}");
                let child_items = &parsed[&Url::from_file_path(&child).unwrap()];
                assert_eq!(
                    child_items.len(),
                    2 * expected.len(),
                    "{engine}/{width}/{numbered}: {parsed:?}"
                );
                let text = Text::new(child_source);
                for (line, name) in expected {
                    let error = child_items
                        .iter()
                        .find(|item| {
                            item.severity == Some(DiagnosticSeverity::ERROR)
                                && item.range.start.line == line
                        })
                        .unwrap_or_else(|| {
                            panic!("{engine}/{width}/{numbered}/{name}: {parsed:?}")
                        });
                    assert_eq!(
                        error.code,
                        Some(NumberOrString::String("undefined-control-sequence".into())),
                        "{engine}/{width}/{numbered}: {error:?}"
                    );
                    assert_eq!(
                        &text.source[text.offset(error.range.start).unwrap()
                            ..text.offset(error.range.end).unwrap()],
                        name,
                        "{engine}/{width}/{numbered}: {error:?}"
                    );
                    assert!(child_items
                        .iter()
                        .any(|item| item.severity == Some(DiagnosticSeverity::WARNING)
                            && item.range.start.line == line - 1));
                }
                let root_items = &parsed[&Url::from_file_path(&main).unwrap()];
                assert_eq!(
                    root_items.len(),
                    1,
                    "{engine}/{width}/{numbered}: {parsed:?}"
                );
                assert_eq!(root_items[0].range.start.line, 4);
                assert_eq!(
                    root_items[0].code,
                    Some(NumberOrString::String("undefined-control-sequence".into()))
                );
            }
        }
    }
}
