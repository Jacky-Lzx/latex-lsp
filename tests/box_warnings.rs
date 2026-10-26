use lsp_types::{DiagnosticSeverity, NumberOrString, Url};
use texquill::{compilation::build_log, text::Text};

fn wrap(message: &str, width: usize, newline: &str) -> String {
    message
        .as_bytes()
        .chunks(width)
        .map(|part| std::str::from_utf8(part).unwrap())
        .collect::<Vec<_>>()
        .join(newline)
}

#[test]
fn wrapped_box_headers_preserve_complete_messages_source_ranges_and_following_warnings() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    let source = "% header\r\n  First 😀 paragraph line.\r\nLast paragraph line.\r\n";
    std::fs::write(&main, "Main.\n").unwrap();
    std::fs::write(&child, source).unwrap();
    for message in [
        "Overfull \\hbox (12.34567pt too wide) in paragraph at lines 2--3",
        "Underfull \\hbox (badness 10000) in paragraph at lines 2--3",
        "Overfull \\hbox (12.34567pt too wide) in alignment at lines 2--3",
        "Underfull \\vbox (badness 10000) detected at line 3",
    ] {
        for newline in ["\n", "\r\n"] {
            let log = format!("(./main.tex{newline}(./child.tex{newline}{}{newline}\\OT1/cmr/m/n/10 unrelated box contents{newline} []{newline}{newline}){newline}Package audit Warning: Parent warning on input line 1.{newline}){newline}", wrap(message, 60, newline));
            let diagnostics = build_log::parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&child).unwrap()];
            assert_eq!(items.len(), 1, "{log}: {diagnostics:?}");
            assert_eq!(items[0].message, message, "{log}");
            assert_eq!(items[0].severity, Some(DiagnosticSeverity::WARNING));
            let text = Text::new(source.into());
            let start = if message.contains("at lines") {
                source.find("First").unwrap()
            } else {
                source.find("Last").unwrap()
            };
            assert_eq!(
                items[0].range,
                text.range(start, source.trim_end().len()),
                "{log}"
            );
            assert_eq!(
                items[0].code,
                Some(NumberOrString::String(
                    if message.starts_with("Overfull") {
                        "overfull-box"
                    } else {
                        "underfull-box"
                    }
                    .into()
                ))
            );
            let parents = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(parents.len(), 1, "{log}: {diagnostics:?}");
            assert!(parents[0].message.contains("Parent warning"));
        }
    }
}

#[test]
fn numbered_box_headers_recover_each_print_width_split_without_including_box_dumps() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let source = "% header\n  First paragraph line.\nLast 😀 line.\n";
    std::fs::write(&main, source).unwrap();
    let text = Text::new(source.into());
    for (message, range) in [
        (
            "Overfull \\hbox (12.34567pt too wide) in paragraph at lines 2--3",
            text.range(source.find("First").unwrap(), source.trim_end().len()),
        ),
        (
            "Underfull \\hbox (badness 10000) in alignment at lines 2--3",
            text.range(source.find("First").unwrap(), source.trim_end().len()),
        ),
        (
            "Overfull \\vbox (19.0pt too high) detected at line 3",
            text.range(source.find("Last").unwrap(), source.trim_end().len()),
        ),
        (
            "Underfull \\vbox (badness 10000) has occurred while \\output is active",
            text.range(
                source.find("First").unwrap(),
                source.find("\nLast").unwrap(),
            ),
        ),
    ] {
        for width in [60, 79] {
            for length in 1..=width {
                let name = format!("{}.tex", "s".repeat(length));
                let path = root.join(&name);
                std::fs::write(&path, source).unwrap();
                let reported = if message.contains("detected at line 3") {
                    3
                } else {
                    2
                };
                let numbered = format!("./{name}:{reported}: {message}");
                let log = format!("{}\n\\OT1/cmr/m/n/10 unrelated box contents\n []\n\n./main.tex:3: LaTeX Warning: Following warning.\n", wrap(&numbered, width, "\n"));
                let diagnostics = build_log::parse(&main, &log);
                let item = &diagnostics
                    .get(&Url::from_file_path(&path).unwrap())
                    .unwrap_or_else(|| panic!("{log}: {diagnostics:?}"))[0];
                assert_eq!(item.message, message, "{log}");
                assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
                assert_eq!(item.range, range, "{log}: {item:?}");
                assert_eq!(diagnostics[&Url::from_file_path(&main).unwrap()].len(), 1);
                // An existing suffix file is intentionally an independent
                // diagnostic boundary. Keep these split-position cases isolated.
                std::fs::remove_file(&path).unwrap();
            }
        }
    }
}

#[test]
fn line_numbers_split_in_the_middle_are_not_accepted_as_complete_shorter_numbers() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "  Paragraph line.\n".repeat(1234);
    std::fs::write(&main, &source).unwrap();
    let text = Text::new(source.clone());
    for metric in ["1.0", "12.0", "123.0", "1234.0", "12345.0"] {
        let message =
            format!("Overfull \\hbox ({metric}pt too wide) in paragraph at lines 2--1234");
        let log = format!(
            "(./main.tex\n{}\n\\OT1/cmr/m/n/10 box contents\n []\n)\n",
            wrap(&message, 60, "\n")
        );
        let diagnostics = build_log::parse(&main, &log);
        let item = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert_eq!(item.message, message, "{log}");
        assert_eq!(
            item.range,
            text.range(source.find('\n').unwrap() + 3, source.trim_end().len()),
            "{log}"
        );
    }
}

#[test]
fn incomplete_box_headers_do_not_consume_source_switches_other_diagnostics_or_truncation() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&main, "Main.\nSecond.\nThird.\n").unwrap();
    std::fs::write(&child, "Child.\nSecond child.\n").unwrap();
    let header = "Overfull \\hbox (12.34567pt too wide) in paragraph at lines 2";
    assert_eq!(header.len(), 60);
    for boundary in [
        "(./child.tex\n./child.tex:2: Child error.\n)\n",
        ")\n(./child.tex\n./child.tex:2: Child error.\n)\n",
        "[texquill: middle of compiler output omitted]\n(./child.tex\n./child.tex:2: Child error.\n)\n",
        "./child.tex:2: Child error.\n",
        "! Other error.\nl.3 Third.\n",
        "Package audit Warning: Independent warning on input line 3.\n",
        "\\OT1/cmr/m/n/10 box dump\n []\n",
        "l.3 Third.\n",
        "\n--3\n",
    ] {
        let log = format!("(./main.tex\n{header}\n{boundary}./main.tex:3: Final sentinel.\n)\n");
        let diagnostics = build_log::parse(&main, &log);
        let main_items = &diagnostics[&Url::from_file_path(&main).unwrap()];
        let item = main_items.iter().find(|item| item.message.starts_with("Overfull")).unwrap();
        assert_eq!(item.message, header, "{log}: {diagnostics:?}");
        assert!(main_items.iter().any(|item| item.message == "Final sentinel." && item.range.start.line == 2), "{log}: {diagnostics:?}");
        if boundary.contains("Child error") {
            assert_eq!(diagnostics[&Url::from_file_path(&child).unwrap()][0].message, "Child error.", "{log}");
        }
        if boundary.contains("Independent warning") {
            assert!(main_items.iter().any(|item| item.message.contains("Independent warning")));
        }
        if boundary.contains("Other error") {
            assert!(main_items.iter().any(|item| item.message == "Other error." && item.range.start.line == 2));
        }
    }
}

#[test]
fn wrapped_unknown_sources_keep_fallback_metadata_without_borrowing_parent_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Main.\nSecond.\nThird.\n").unwrap();
    let message = "Overfull \\hbox (12.34567pt too wide) in paragraph at lines 2--3";
    for opening in ["(./gone.tex\n".to_string(), String::new()] {
        let header = if opening.is_empty() {
            format!("./gone.tex:2: {message}")
        } else {
            message.into()
        };
        let log = format!(
            "(./main.tex\n{opening}{}\n []\n\n)\n",
            wrap(&header, 60, "\n")
        );
        let diagnostics = build_log::parse(&main, &log);
        let item = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
        assert_eq!(item.message, message, "{log}");
        assert_eq!(
            item.data,
            Some(serde_json::json!({"unresolvedSource":true}))
        );
        assert_eq!(item.code, None);
        assert_eq!(item.range.start.line, 1);
        assert_eq!(item.range.end.line, 1);
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_wrapped_box_warnings_keep_paragraph_line_ranges_across_engines() {
    use std::{fs, process::Command};
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let source = "\\documentclass{article}\n\\begin{document}\nBefore.\\par\n\\noindent\\hbox to 1pt{A very wide box}\\par\n\\vbox to 1pt{\\vskip 20pt}\\par\n{\\hsize=1pt\\noindent A long paragraph\nwith unbreakableword text.\\par}\n\\end{document}\n";
        fs::write(&main, source).unwrap();
        let output = Command::new(engine)
            .args([
                "-interaction=nonstopmode",
                "-halt-on-error",
                "-file-line-error",
                "main.tex",
            ])
            .env("max_print_line", "60")
            .env("TEXMFVAR", root.join("tex-cache"))
            .env("TEXMFCACHE", root.join("tex-cache"))
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{engine}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let log = fs::read_to_string(root.join("main.log")).unwrap();
        if engine == "pdflatex" {
            assert!(log.contains("6-\n-7") || log.contains("6\n--7"), "{log}");
        }
        let diagnostics = build_log::parse(&main, &log);
        let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
        let boxes: Vec<_> = items
            .iter()
            .filter(|d| d.message.starts_with("Overfull"))
            .collect();
        assert!(boxes.len() > 3, "{engine}: {items:?}");
        let text = Text::new(source.into());
        for item in boxes {
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING));
            if item.message.contains("in paragraph") {
                assert!(
                    item.message.ends_with("at lines 6--7"),
                    "{engine}: {item:?}"
                );
                let start = source.find("{\\hsize").unwrap();
                let end = source.find("\n\\end{document}").unwrap();
                assert_eq!(item.range, text.range(start, end), "{engine}: {item:?}");
            }
        }
    }
}
