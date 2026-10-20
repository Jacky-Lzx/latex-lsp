use latex_lsp::{build, config::BuildConfig, workspace::Workspace};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

#[test]
fn invalid_reported_lines_never_borrow_names_or_ranges_from_clamped_source_lines() {
    use latex_lsp::{build_log, text::Text};
    use tower_lsp::lsp_types::{Position, Range};
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    let cases = [
        (r"😀 \ref{missing}", "LaTeX Warning: Reference `missing' on page 1 undefined on input line {line}.", DiagnosticSeverity::WARNING),
        (r"😀 \cite{missing}", "Package natbib Warning: Citation `missing' on page 1 undefined on input line {line}.", DiagnosticSeverity::WARNING),
        (r"😀 \hyperlink{missing}{Text}", "pdfTeX warning (dest): name{missing} has been referenced but does not exist, replaced by a fixed one", DiagnosticSeverity::WARNING),
        (r"😀 \hypertarget{missing}{Text}", "pdfTeX warning (ext4): destination with the same identifier (name{missing}) has been already used, duplicate ignored", DiagnosticSeverity::WARNING),
        (r"😀 \include{missing}", "Included file `missing.tex` not found", DiagnosticSeverity::WARNING),
        ("😀 Paragraph", r"Overfull \hbox (12pt too wide) in paragraph at lines 1--1", DiagnosticSeverity::WARNING),
        (r"😀 \bad{x}", "Undefined control sequence.", DiagnosticSeverity::ERROR),
    ];
    for (source, message, severity) in cases {
        for path in [&main, &child] {
            std::fs::write(path, source).unwrap();
        }
        let end = source.encode_utf16().count() as u32;
        for name in ["main.tex", "child.tex"] {
            let path = directory.path().join(name);
            for number in [0, 2, 999, u32::MAX] {
                let message = message.replace("{line}", &number.to_string());
                let log = format!("./{name}:{number}: {message}\nl.{number} 😀 \\bad\n");
                let diagnostics = build_log::parse(&main, &log);
                let items = &diagnostics[&Url::from_file_path(&path).unwrap()];
                assert_eq!(items.len(), 1, "{log}");
                let item = &items[0];
                assert_eq!(item.message, message);
                assert_eq!(
                    item.range,
                    Range::new(Position::new(0, 0), Position::new(0, end))
                );
                assert_eq!(item.severity, Some(severity));
                assert_eq!(item.code, None, "{log}");
                assert_eq!(
                    item.data,
                    Some(serde_json::json!({"unresolvedLine":true,"reportedLine":number})),
                    "{log}"
                );
                assert!(Text::new(source.into()).offset(item.range.end).is_some());
            }
        }
    }
}

#[test]
fn valid_and_zero_line_warnings_both_survive_parsing_and_publication() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "Paragraph";
    std::fs::write(&main, source).unwrap();
    let message = r"Overfull \hbox (12pt too wide) in paragraph at lines 1--1";
    for numbers in [[0, 1], [1, 0]] {
        let log = numbers
            .map(|number| format!("./main.tex:{number}: {message}\n"))
            .concat();
        let mut workspace = Workspace::default();
        workspace.open(uri.clone(), source.into(), 1);
        workspace
            .build_diagnostics
            .insert(uri.clone(), latex_lsp::build_log::parse(&main, &log));
        let diagnostics = workspace.diagnostics(&uri);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        assert_eq!(
            diagnostics
                .iter()
                .filter(|item| item.code.is_some())
                .count(),
            1
        );
        assert_eq!(
            diagnostics
                .iter()
                .filter(|item| item
                    .data
                    .as_ref()
                    .is_some_and(|data| data["unresolvedLine"] == true))
                .count(),
            1
        );
    }
}

#[test]
fn unresolved_numbered_sources_do_not_borrow_warning_ranges_or_codes_from_the_root() {
    use latex_lsp::{build_log, text::Text};
    use tower_lsp::lsp_types::{NumberOrString, Position, Range};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let child = root.join("child.tex");
    std::fs::create_dir(root.join("folder.tex")).unwrap();
    for (source, message, code) in [
        (r"Prefix \ref{missing}", "LaTeX Warning: Reference `missing' on page 1 undefined on input line 1.", None),
        (r"Prefix \cite{missing}", "Package natbib Warning: Citation `missing' on page 1 undefined on input line 1.", None),
        (r"Prefix \hyperlink{missing}{Text}", "pdfTeX warning (dest): name{missing} has been referenced but does not exist, replaced by a fixed one", Some("undefined-pdf-destination")),
        (r"Prefix \hyperlink{missing}{Text}", "warning  (pdf backend): unreferenced destination with name 'missing'", Some("undefined-pdf-destination")),
        (r"Prefix \hypertarget{missing}{Text}", "pdfTeX warning (ext4): destination with the same identifier (name{missing}) has been already used, duplicate ignored", Some("duplicate-pdf-destination")),
        (r"Prefix \include{missing}", "Included file `missing.tex` not found", Some("missing-include")),
        ("First paragraph line.\nSecond paragraph line.", r"Overfull \hbox (12pt too wide) in paragraph at lines 1--2", Some("overfull-box")),
        ("Paragraph.", "Missing character: There is no A (U+0041) in font nullfont!", None),
    ] {
        std::fs::write(&main, format!("{source}\n")).unwrap();
        std::fs::write(&child, format!("{source}\n")).unwrap();
        let text = Text::new(source.into());
        let expected = if let Some(start) = source.find("missing") {
            text.range(start, start + "missing".len())
        } else {
            text.range(0, source.len())
        };
        let mut severity = None;
        for (name, path) in [("./main.tex", &main), ("./child.tex", &child)] {
            let log = format!("(./main.tex\n{name}:1: {message}\n)\n");
            let diagnostics = build_log::parse(&main, &log);
            let item = &diagnostics[&Url::from_file_path(path).unwrap()][0];
            assert_eq!(item.range, expected, "{name}: {message}");
            assert_eq!(item.severity, Some(DiagnosticSeverity::WARNING), "{message}");
            assert_eq!(item.code, code.map(|code| NumberOrString::String(code.into())));
            severity = item.severity;
        }
        for name in ["./gone.tex", "./folder.tex"] {
            let log = format!("(./main.tex\n{name}:1: {message}\n)\n");
            let diagnostics = build_log::parse(&main, &log);
            let item = &diagnostics[&Url::from_file_path(&main).unwrap()][0];
            assert_eq!(item.message, message);
            assert_eq!(item.severity, severity);
            assert_eq!(item.code, None, "{name}: {item:?}");
            assert_eq!(item.data, Some(serde_json::json!({"unresolvedSource":true})), "{name}: {item:?}");
            let line_end = source.lines().next().unwrap().encode_utf16().count() as u32;
            assert_eq!(item.range, Range::new(Position::new(0, 0), Position::new(0, line_end)), "{name}: {message}");
        }
    }
}

#[test]
fn trusted_and_unresolved_numbered_warnings_do_not_suppress_each_other() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let cases = [
        (
            r"\hyperlink{missing}{Text}",
            "pdfTeX warning (dest): name{missing} has been referenced but does not exist, replaced by a fixed one",
        ),
        (
            "Paragraph",
            r"Overfull \hbox (12pt too wide) in paragraph at lines 1--1",
        ),
    ];
    for (source, message) in cases {
        std::fs::write(&main, format!("{source}\n")).unwrap();
        for names in [["gone.tex", "main.tex"], ["main.tex", "gone.tex"]] {
            let log = names
                .map(|name| format!("./{name}:1: {message}\n"))
                .concat();
            let diagnostics = latex_lsp::build_log::parse(&main, &log);
            let items = &diagnostics[&Url::from_file_path(&main).unwrap()];
            assert_eq!(items.len(), 2, "{log}");
            assert_eq!(items.iter().filter(|item| item.code.is_some()).count(), 1);
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item
                        .data
                        .as_ref()
                        .is_some_and(|data| data["unresolvedSource"] == true))
                    .count(),
                1
            );
            let uri = Url::from_file_path(&main).unwrap();
            let mut workspace = Workspace::default();
            workspace.open(uri.clone(), source.into(), 1);
            workspace.build_diagnostics.insert(uri.clone(), diagnostics);
            assert_eq!(workspace.diagnostics(&uri).len(), 2);
        }
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex, lualatex and hyperref"]
fn real_pdfbookmark_levels_match_actual_targets_across_engines_and_duplicate_warnings() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "\\documentclass{article}\\usepackage{hyperref}\\begin{document}\n\\pdfbookmark[-3]{Lowest}{lowest}\\pdfbookmark{Default}{base}\\pdfbookmark[01]{Leading}{leading}\\pdfbookmark[-1]{Part}{part}\\pdfbookmark[+1]{Plus}{plus}\n\\hyperlink{base.0}{A}\\hyperlink{leading.01}{B}\\hyperlink{part.-1}{C}\\hyperlink{plus.+1}{D}\\hyperlink{lowest.-3}{E}\\end{document}\n";
        std::fs::write(&main, source).unwrap();
        let executable = if engine == "lualatex" {
            script(directory.path(), "TEXMFVAR=\"${0%/*}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec lualatex \"$@\"")
        } else {
            engine.into()
        };
        let config = BuildConfig {
            executable,
            arguments: Some(vec![
                "-interaction=nonstopmode".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ]),
            ..Default::default()
        };
        for _ in 0..2 {
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            assert!(
                !outcome
                    .diagnostics
                    .values()
                    .flatten()
                    .any(|warning| warning.code
                        == Some(tower_lsp::lsp_types::NumberOrString::String(
                            "undefined-pdf-destination".into()
                        ))),
                "{engine}: {:?}",
                outcome.diagnostics
            );
        }
        let bookmarks =
            std::fs::read_to_string(config.artifact_path(&main, "out").unwrap()).unwrap();
        for name in ["base.0", "leading.01", "part.-1", "plus.+1", "lowest.-3"] {
            assert!(
                bookmarks.contains(&format!("{{{name}}}")),
                "{engine}: {bookmarks}"
            );
        }
        if engine != "xelatex" {
            let duplicated = source.replace(
                "\\end{document}",
                "\\hypertarget{base.0}{Duplicate}\\end{document}",
            );
            std::fs::write(&main, &duplicated).unwrap();
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            let warnings: Vec<_> = outcome
                .diagnostics
                .values()
                .flatten()
                .filter(|warning| {
                    warning.code
                        == Some(tower_lsp::lsp_types::NumberOrString::String(
                            "duplicate-pdf-destination".into(),
                        ))
                })
                .collect();
            assert_eq!(warnings.len(), 2, "{engine}: {:?}", outcome.diagnostics);
            let doc = latex_lsp::workspace::Document::new(
                &Url::from_file_path(&main).unwrap(),
                duplicated,
                None,
            );
            let bookmark = doc
                .syntax
                .anchors
                .iter()
                .find(|entry| entry.name == "base.0")
                .unwrap();
            assert!(warnings
                .iter()
                .any(|warning| warning.range
                    == doc.text.range(bookmark.span.start, bookmark.span.end)));
        }
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn final_log_sources_distinguish_read_chapters_from_includeonly_recorder_probes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\\usepackage{hyperref}\\includeonly{active}\\begin{document}\\include{active}\\include{excluded}\\end{document}\n").unwrap();
    for name in ["active.tex", "excluded.tex"] {
        std::fs::write(root.join(name), "\\hyperlink{lost}{Text}\n").unwrap();
    }
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &Default::default());
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    let active = Url::from_file_path(root.join("active.tex")).unwrap();
    let excluded = Url::from_file_path(root.join("excluded.tex")).unwrap();
    assert!(outcome.inputs.as_ref().unwrap().contains(&active));
    assert!(outcome
        .diagnostic_sources
        .contains(&Url::from_file_path(&main).unwrap()));
    assert!(outcome.diagnostic_sources.contains(&active));
    assert!(!outcome.diagnostic_sources.contains(&excluded));
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, lualatex and hyperref"]
fn real_duplicate_destinations_include_automatic_collisions_and_long_compound_declarations() {
    for engine in ["pdflatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let names = [
            "shared".to_owned(),
            format!("section.{}", "x".repeat(95)),
            format!(
                "{}'suffix",
                "x".repeat(
                    80 - "warning  (pdf backend): ignoring duplicate destination with the name '"
                        .len()
                        - 1
                )
            ),
        ];
        let mut source = "\\documentclass{article}\\usepackage{hyperref}\\begin{document}\n\\hypertarget{section.1}{Explicit}\\section{Automatic}\n".to_owned();
        for name in &names {
            source.push_str(&format!("\\hypertarget{{{name}}}{{First}}\\hyperdef{{}}% keep\n{{{name}}}{{Second}}\\hyperlink{{{name}}}{{Link}}\n"));
        }
        source.push_str("\\end{document}\n");
        std::fs::write(&main, &source).unwrap();
        let executable = if engine == "lualatex" {
            script(directory.path(), "TEXMFVAR=\"${0%/*}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec lualatex \"$@\"")
        } else {
            engine.into()
        };
        let config = BuildConfig {
            executable,
            arguments: Some(vec![
                "-interaction=nonstopmode".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ]),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &Default::default());
        assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
        let doc = latex_lsp::workspace::Document::new(
            &Url::from_file_path(&main).unwrap(),
            source.clone(),
            None,
        );
        let warnings: Vec<_> = outcome
            .diagnostics
            .values()
            .flatten()
            .filter(|diagnostic| {
                diagnostic.code
                    == Some(tower_lsp::lsp_types::NumberOrString::String(
                        "duplicate-pdf-destination".into(),
                    ))
            })
            .collect();
        assert_eq!(
            warnings.len(),
            if engine == "pdflatex" { 5 } else { 7 },
            "{engine}: {:?}\n{}",
            outcome.diagnostics,
            outcome.output
        );
        for entry in &doc.syntax.anchors {
            // pdfTeX abbreviates long duplicate names with \ETC. instead of
            // printing their full identity. Such warnings must stay unlocated.
            if engine == "pdflatex" && entry.name == names[1] {
                continue;
            }
            let range = doc.text.range(entry.span.start, entry.span.end);
            let warning = warnings
                .iter()
                .find(|warning| warning.range == range)
                .unwrap_or_else(|| panic!("{engine}: {entry:?}\n{warnings:?}"));
            assert_eq!(
                warning.data,
                Some(serde_json::json!({"destination":entry.name}))
            );
            assert!(!warning.message.contains("shipout"));
        }
        if engine == "pdflatex" {
            assert!(outcome.diagnostics.values().flatten().any(|warning| warning
                .message
                .contains("\\ETC.")
                && warning.code.is_none()
                && warning.data.is_none()));
        }
        let mut repaired = source.replace("\\hypertarget{section.1}{Explicit}", "");
        for name in &names {
            repaired = repaired.replace(&format!("\\hypertarget{{{name}}}{{First}}"), "");
        }
        std::fs::write(&main, repaired).unwrap();
        for _ in 0..2 {
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            assert!(
                !outcome
                    .diagnostics
                    .values()
                    .flatten()
                    .any(
                        |diagnostic| diagnostic.message.contains("duplicate destination")
                            || diagnostic.message.contains("same identifier")
                    ),
                "{engine}: {:?}",
                outcome.diagnostics
            );
        }
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, lualatex and hyperref"]
fn real_long_destination_warnings_recover_names_and_wrapped_explanatory_words() {
    for engine in ["pdflatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let mut names: Vec<_> = [35, 90, 150]
            .into_iter()
            .map(|length| format!("section.{}", "x".repeat(length)))
            .collect();
        names.push(format!("{}'suffix", "x".repeat(19)));
        let mut source =
            "\\documentclass{article}\\usepackage{hyperref}\\begin{document}\n".to_owned();
        for name in &names {
            source.push_str(&format!("\\hyperlink{{{name}}}{{Link}}\n"));
        }
        source.push_str(&format!(
            "\\hyperref{{}}{{section}}% keep\n{{{}}}{{Compound}}\\end{{document}}\n",
            "x".repeat(150)
        ));
        std::fs::write(&main, &source).unwrap();
        let executable = if engine == "lualatex" {
            script(directory.path(), "TEXMFVAR=\"${0%/*}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec lualatex \"$@\"")
        } else {
            engine.into()
        };
        let config = BuildConfig {
            executable,
            arguments: Some(vec![
                "-interaction=nonstopmode".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ]),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &Default::default());
        assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
        let doc =
            latex_lsp::workspace::Document::new(&Url::from_file_path(&main).unwrap(), source, None);
        let warnings: Vec<_> = outcome
            .diagnostics
            .values()
            .flatten()
            .filter(|diagnostic| {
                diagnostic.code
                    == Some(tower_lsp::lsp_types::NumberOrString::String(
                        "undefined-pdf-destination".into(),
                    ))
            })
            .collect();
        assert_eq!(
            warnings.len(),
            5,
            "{engine}: {:?}\n{}",
            outcome.diagnostics,
            outcome.output
        );
        for entry in &doc.syntax.anchor_references {
            let range = doc.text.range(entry.span.start, entry.span.end);
            let warning = warnings
                .iter()
                .find(|warning| warning.range == range)
                .unwrap_or_else(|| panic!("{engine}: {entry:?}\n{warnings:?}"));
            assert_eq!(
                warning.data,
                Some(serde_json::json!({"destination":entry.name}))
            );
            assert!(!warning.message.contains("</"), "{warning:?}");
        }
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, lualatex and hyperref"]
fn real_destination_warnings_locate_links_and_clear_after_repair() {
    for engine in ["pdflatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = concat!(
            "\\documentclass{article}\\usepackage{hyperref}\n",
            "\\begin{document}\\section{Title}\\hyperdef{section}{intro}{Point}\\label{section.itnro}\n",
            "\\hyperlink{section.itnro}{Direct}\\hyperref{}{section}% keep\n{itnro}{Compound}\n",
            "\\hyperlink{section.1}{Automatic destination}\\end{document}\n",
        );
        std::fs::write(&main, source).unwrap();
        let executable = if engine == "lualatex" {
            script(directory.path(), "TEXMFVAR=\"${0%/*}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec lualatex \"$@\"")
        } else {
            engine.into()
        };
        let config = BuildConfig {
            executable,
            arguments: Some(vec![
                "-interaction=nonstopmode".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ]),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &Default::default());
        assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
        let document = latex_lsp::workspace::Document::new(
            &Url::from_file_path(&main).unwrap(),
            source.into(),
            None,
        );
        let warnings: Vec<_> = outcome
            .diagnostics
            .values()
            .flatten()
            .filter(|diagnostic| {
                diagnostic.code
                    == Some(tower_lsp::lsp_types::NumberOrString::String(
                        "undefined-pdf-destination".into(),
                    ))
            })
            .collect();
        assert_eq!(
            warnings.len(),
            2,
            "{engine}: {:?}\n{}",
            outcome.diagnostics,
            outcome.output
        );
        for (warning, entry) in warnings.iter().zip(&document.syntax.anchor_references) {
            assert_eq!(
                warning.range,
                document.text.range(entry.span.start, entry.span.end)
            );
            assert_eq!(warning.severity, Some(DiagnosticSeverity::WARNING));
            assert_eq!(
                warning.data,
                Some(serde_json::json!({"destination":"section.itnro"}))
            );
        }
        std::fs::write(
            &main,
            source
                .replace("\\hyperlink{section.itnro}", "\\hyperlink{section.intro}")
                .replace("{itnro}{Compound}", "{intro}{Compound}"),
        )
        .unwrap();
        for _ in 0..2 {
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            assert!(
                !outcome
                    .diagnostics
                    .values()
                    .flatten()
                    .any(
                        |diagnostic| diagnostic.message.contains("unreferenced destination")
                            || diagnostic.message.contains("has been referenced")
                    ),
                "{engine}: {:?}",
                outcome.diagnostics
            );
        }
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires latexmk, pdflatex, xelatex and lualatex"]
fn custom_job_names_locate_pdf_logs_recorder_inputs_and_synctex_for_all_tex_engines() {
    for executable in ["latexmk", "pdflatex", "xelatex", "lualatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        std::fs::write(
            &main,
            "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("chapter.tex"),
            "\\PackageWarning{example}{Job name warning}Chapter.",
        )
        .unwrap();
        let compiler = if executable == "lualatex" {
            let wrapper = script(root, "TEXMFVAR=\"${0%/*}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec lualatex \"$@\"");
            let named = root.join("lualatex");
            std::fs::rename(wrapper, &named).unwrap();
            named.to_string_lossy().into_owned()
        } else {
            executable.into()
        };
        let config = BuildConfig {
            executable: compiler,
            job_name: Some("论文 draft.v2".into()),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status, "success",
            "{executable}: {}",
            outcome.output
        );
        assert_eq!(outcome.pdf, config.artifact_path(&main, "pdf"));
        assert_eq!(outcome.log, config.artifact_path(&main, "log"));
        assert_eq!(outcome.pages, Some(1), "{executable}: {}", outcome.output);
        let child = Url::from_file_path(root.join("chapter.tex")).unwrap();
        assert!(outcome.inputs.unwrap().contains(&child));
        assert!(
            outcome.diagnostics[&child]
                .iter()
                .any(|diagnostic| diagnostic.message.contains("Job name warning")),
            "{:?}",
            outcome.diagnostics
        );
        let mut workspace = Workspace::default();
        workspace.config.build = config;
        workspace.add_root(root.into());
        assert!(
            latex_lsp::synctex::prepare(&workspace, &Url::from_file_path(&main).unwrap()).is_ok(),
            "{executable}"
        );
        assert!(!root.join("build/latex-lsp/main.pdf").exists());
    }
}

#[test]
#[ignore = "requires latexmk and BibTeX"]
fn bibliography_builds_with_custom_job_names_still_resolve_and_analyze_the_database() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\cite{example}\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}\n").unwrap();
    std::fs::write(
        root.join("references.bib"),
        "@book{example, title={Example}, year={2026}, publisher={P}}\n",
    )
    .unwrap();
    let config = BuildConfig {
        job_name: Some("论文 draft.v2".into()),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    assert!(outcome.pdf.unwrap().is_file());
    let bib = Url::from_file_path(root.join("references.bib")).unwrap();
    assert!(
        outcome.diagnostics[&bib]
            .iter()
            .any(|diagnostic| diagnostic.message.contains("empty author")),
        "{:?}",
        outcome.diagnostics
    );
    assert!(
        !outcome
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.message.contains("undefined")),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
#[cfg(unix)]
#[ignore = "requires latexmk, BibTeX, pdflatex and xelatex"]
fn real_bibtex_errors_follow_database_and_style_aliases_to_the_correct_file_and_line() {
    use latex_lsp::config::Engine;
    for engine in [Engine::PdfLatex, Engine::XeLatex] {
        for style_error in [false, true] {
            for extension in ["payload", "png", "tex"] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let physical = root.join(format!("physical.{extension}"));
                let (style, name, contents, message, line) = if style_error {
                    (
                        "chosen",
                        "chosen.bst",
                        "ENTRY {title} {} {}\nFUNCTION {broken} {notafunction}\nREAD\n",
                        "notafunction is an unknown function",
                        1,
                    )
                } else {
                    (
                        "plain",
                        "refs.bib",
                        "% database\n@book{entry,author={Writer},title={Title},year={2026}}\n@book{entry,author={Writer},title={Duplicate},year={2026}}\n",
                        "Repeated entry",
                        2,
                    )
                };
                std::fs::write(&physical, contents).unwrap();
                std::os::unix::fs::symlink(&physical, root.join(name)).unwrap();
                if style_error {
                    std::fs::write(
                        root.join("refs.bib"),
                        "@book{entry,author={Writer},title={Title},year={2026}}\n",
                    )
                    .unwrap();
                }
                std::fs::write(&main, format!("\\documentclass{{article}}\n\\begin{{document}}\n\\cite{{entry}}\\bibliographystyle{{{style}}}\\bibliography{{refs}}\n\\end{{document}}\n")).unwrap();
                let config = BuildConfig {
                    engine: Some(engine),
                    ..Default::default()
                };
                let outcome = build::run(&main, &config, &AtomicBool::new(false));
                assert_eq!(outcome.status, "error", "{}", outcome.output);
                let uri = Url::from_file_path(&physical).unwrap();
                assert!(
                    outcome
                        .diagnostics
                        .get(&uri)
                        .is_some_and(|items| items
                            .iter()
                            .any(|item| item.message == format!("BibTeX: {message}")
                                && item.range.start.line == line
                                && item.severity == Some(DiagnosticSeverity::ERROR))),
                    "{engine:?} style={style_error} extension={extension}: {:?}\n{}",
                    outcome.diagnostics,
                    outcome.output
                );
                assert!(!outcome
                    .diagnostics
                    .get(&Url::from_file_path(&main).unwrap())
                    .is_some_and(|items| items.iter().any(|item| item.message.contains(message))));
            }
        }
    }
}

#[test]
#[ignore = "requires latexmk, BibTeX, pdflatex and xelatex"]
fn real_bibtex_warnings_keep_literal_entry_keys_despite_namesake_files() {
    use latex_lsp::config::Engine;
    use tower_lsp::lsp_types::{Position, Range};
    for engine in [Engine::PdfLatex, Engine::XeLatex] {
        for key in ["'entry'", "'entry.bib'", "entry.bib"] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let bib = root.join("refs.bib");
            std::fs::write(root.join("entry.bib"), "@book{wrong,title={Decoy}}\n").unwrap();
            std::fs::write(&main, format!("\\documentclass{{article}}\n\\begin{{document}}\n\\cite{{{key}}}\\bibliographystyle{{plain}}\\bibliography{{refs}}\n\\end{{document}}\n")).unwrap();
            std::fs::write(
                &bib,
                format!("@book{{{key},title={{Example}},year={{2026}},publisher={{P}}}}\n"),
            )
            .unwrap();
            let config = BuildConfig {
                engine: Some(engine),
                ..Default::default()
            };
            let outcome = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(outcome.status, "success", "{}", outcome.output);
            let range = Range::new(
                Position::new(0, 6),
                Position::new(0, 6 + key.encode_utf16().count() as u32),
            );
            assert!(
                outcome
                    .diagnostics
                    .get(&Url::from_file_path(&bib).unwrap())
                    .is_some_and(|items| items
                        .iter()
                        .any(|item| item.message.contains("empty author and editor")
                            && item.range == range)),
                "{engine:?} {key}: {:?}",
                outcome.diagnostics
            );
        }
    }
}

#[test]
#[ignore = "requires latexmk, BibTeX, pdflatex and xelatex"]
fn real_bibtex_style_warning_text_keeps_the_actual_entry_location() {
    use latex_lsp::config::Engine;
    use tower_lsp::lsp_types::{Position, Range};
    for engine in [Engine::PdfLatex, Engine::XeLatex] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let bib = root.join("refs.bib");
        std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\cite{entry}\\bibliographystyle{local}\\bibliography{refs}\n\\end{document}\n").unwrap();
        std::fs::write(&bib, "% header\n@book{entry,title={Book}}\n").unwrap();
        std::fs::write(
            root.join("local.bst"),
            r#"ENTRY {title} {} {}
FUNCTION {begin.bib} { "\begin{thebibliography}{1}" write$ newline$ }
FUNCTION {book} {
  "line 700 value in " cite$ * warning$
  "\bibitem{" cite$ * "}" * write$ newline$
  title write$ newline$
}
FUNCTION {end.bib} { "\end{thebibliography}" write$ newline$ }
READ
EXECUTE {begin.bib}
ITERATE {call.type$}
EXECUTE {end.bib}
"#,
        )
        .unwrap();
        let config = BuildConfig {
            engine: Some(engine),
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(outcome.status, "success", "{}", outcome.output);
        let items = &outcome.diagnostics[&Url::from_file_path(&bib).unwrap()];
        assert!(
            items
                .iter()
                .any(|item| item.message == "BibTeX: line 700 value in entry"
                    && item.range == Range::new(Position::new(1, 6), Position::new(1, 11))
                    && item.data.is_none()),
            "{engine:?}: {items:?}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_missing_include_warnings_reconstruct_tex_wrapped_long_chapter_names() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let chapter = format!("chapters/{} (missing chapter)", "中".repeat(50));
    std::fs::write(&main, format!("\\documentclass{{article}}\n\\begin{{document}}\nIntroduction.\n\\include{{{chapter}}}\n\\end{{document}}\n")).unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    let source = Url::from_file_path(&main).unwrap();
    let diagnostics = outcome
        .diagnostics
        .get(&source)
        .unwrap_or_else(|| panic!("{}", outcome.output));
    let warning = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "missing-include".into(),
                ))
        })
        .unwrap_or_else(|| panic!("{diagnostics:?}\n{}", outcome.output));
    assert_eq!(
        warning.message,
        format!("Included file `{chapter}.tex` not found")
    );
    assert_eq!(warning.range.start.line, 3);
    assert_eq!(warning.range.start.character, 9);
    assert_eq!(
        warning.range.end.character as usize,
        9 + chapter.encode_utf16().count()
    );
}

#[test]
#[ignore = "requires pdflatex"]
fn real_overfull_paragraph_diagnostics_cover_the_complete_reported_source_range() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
    )
    .unwrap();
    let source = "\\hsize=20pt\n  AReallyLongUnbreakableWord\n  AnotherUnbreakableWord\n\\par\n";
    std::fs::write(root.join("chapter.tex"), source).unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(root.join("chapter.tex")).unwrap()];
    let overfull = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code
                == Some(tower_lsp::lsp_types::NumberOrString::String(
                    "overfull-box".into(),
                ))
        })
        .collect::<Vec<_>>();
    assert!(!overfull.is_empty(), "{diagnostics:?}");
    for diagnostic in overfull {
        assert!(
            diagnostic.message.contains("at lines 2--4"),
            "{diagnostic:?}"
        );
        assert_eq!(
            diagnostic.range.start,
            tower_lsp::lsp_types::Position::new(1, 2)
        );
        assert_eq!(
            diagnostic.range.end,
            tower_lsp::lsp_types::Position::new(3, 4)
        );
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn missing_included_chapters_warn_even_when_tex_successfully_generates_a_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let child = root.join("child.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n\\input{child}\nContent.\n\\end{document}\n",
    )
    .unwrap();
    let source = "\\include{missing chapter}\n";
    std::fs::write(&child, source).unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    assert!(outcome.pdf.unwrap().is_file());
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(&child).unwrap()];
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(
        diagnostics[0].code,
        Some(tower_lsp::lsp_types::NumberOrString::String(
            "missing-include".into()
        ))
    );
    assert_eq!(
        diagnostics[0].range,
        tower_lsp::lsp_types::Range::new(
            tower_lsp::lsp_types::Position::new(0, 9),
            tower_lsp::lsp_types::Position::new(0, 24)
        )
    );
    assert!(diagnostics[0].message.contains("missing chapter.tex"));
    std::fs::write(root.join("missing chapter.tex"), "Restored chapter.\n").unwrap();
    let repaired = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(repaired.status, "success", "{}", repaired.output);
    assert!(!repaired
        .diagnostics
        .values()
        .flatten()
        .any(|diagnostic| diagnostic.code
            == Some(tower_lsp::lsp_types::NumberOrString::String(
                "missing-include".into()
            ))));
}

#[test]
#[ignore = "requires latexmk and pdflatex"]
fn nested_chapter_includes_prepare_auxiliary_directories_with_default_and_direct_compilers() {
    for executable in ["latexmk", "pdflatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir_all(root.join("chapters/deep")).unwrap();
        std::fs::write(root.join("main.tex"),"\\documentclass{article}\n\\begin{document}\n\\input{chapters/plan}\n\\end{document}\n").unwrap();
        std::fs::write(
            root.join("chapters/plan.tex"),
            "\\include{chapters/deep/part}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("chapters/deep/part.tex"),
            "\\typeout{NESTED INCLUDE OK}Chapter.\n",
        )
        .unwrap();
        let config = BuildConfig {
            executable: executable.into(),
            job_name: Some("paper.v2".into()),
            ..Default::default()
        };
        let outcome = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status, "success",
            "{executable}: {}",
            outcome.output
        );
        assert!(outcome.output.contains("NESTED INCLUDE OK"));
        assert!(root
            .join("build/latex-lsp/chapters/deep/part.aux")
            .is_file());
        assert!(!root.join("chapters/deep/part.aux").exists());
        assert!(outcome.pdf.unwrap().ends_with("paper.v2.pdf"));
    }
}

#[test]
#[cfg(unix)]
fn auxiliary_directory_preparation_skips_comments_macros_and_paths_outside_the_output_directory() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("main.tex"),"\\documentclass{article}\n\\include{chapters/deep/part}\n\\include{../escape/part}\n\\include{generated/\\unknown}\n% \\include{commented/part}\n\\verb|\\include{raw/part}|\n").unwrap();
    let wrapper = script(
        root,
        "[ -d \"$1/chapters/deep\" ] || exit 13\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"",
    );
    let compiler = root.join("pdflatex");
    std::fs::rename(wrapper, &compiler).unwrap();
    let config = BuildConfig {
        executable: compiler.to_string_lossy().into_owned(),
        arguments: Some(vec!["%o".into()]),
        ..Default::default()
    };
    let outcome = build::run(&root.join("main.tex"), &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    for name in [
        "build/escape",
        "build/latex-lsp/generated",
        "build/latex-lsp/commented",
        "build/latex-lsp/raw",
    ] {
        assert!(!root.join(name).exists(), "{name}");
    }
}

#[test]
fn engine_directives_use_the_root_comment_and_respect_explicit_configuration() {
    use latex_lsp::{config::Engine, workspace::Document};
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    std::fs::write(
        &main,
        "% !TeX program = xelatex\n\\documentclass{article}\n",
    )
    .unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    assert_eq!(
        workspace.build_config(&uri).effective_engine(),
        Some(Engine::XeLatex)
    );
    workspace.config.build.engine = Some(Engine::LuaLatex);
    assert_eq!(
        workspace.build_config(&uri).effective_engine(),
        Some(Engine::LuaLatex)
    );
    workspace.config.build.engine = None;
    workspace.config.build.executable = "pdflatex".into();
    assert_eq!(
        workspace.build_config(&uri).effective_engine(),
        Some(Engine::PdfLatex)
    );
    workspace.config.build.executable = "latexmk".into();
    for (source, expected) in [
        ("% !TeX TS-program = \"LuaLaTeX\"\n\\documentclass{article}", Engine::LuaLatex),
        ("\\begin{verbatim}\n% !TeX program = xelatex\n\\end{verbatim}\n\\documentclass{article}", Engine::PdfLatex),
        ("% !TeX program = sh -c anything\n\\documentclass{article}", Engine::PdfLatex),
    ] {
        workspace.documents.insert(uri.clone(), Document::new(&uri, source.into(), Some(1)));
        assert_eq!(workspace.build_config(&uri).effective_engine(), Some(expected));
    }
}

#[test]
#[cfg(unix)]
fn unchanged_pdf_is_accepted_only_when_it_is_not_older_than_the_source() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let output = directory.path().join("build/latex-lsp");
    std::fs::create_dir_all(&output).unwrap();
    let pdf = output.join("main.pdf");
    std::fs::write(&main, "Original\n").unwrap();
    std::fs::write(&pdf, "%PDF-1.4\n").unwrap();
    let now = std::time::SystemTime::now();
    std::fs::File::open(&main)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(now - Duration::from_secs(120)))
        .unwrap();
    std::fs::File::open(&pdf)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(now - Duration::from_secs(60)))
        .unwrap();
    let config = BuildConfig {
        executable: "/bin/sh".into(),
        arguments: Some(vec!["-c".into(), "exit 0".into()]),
        ..Default::default()
    };
    let fresh = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(fresh.status, "success", "{}", fresh.message);
    assert!(!fresh.pdf_updated);
    std::fs::write(&main, "Changed before compilation\n").unwrap();
    let outdated = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outdated.status, "error");
    assert!(outdated.pdf.is_none());
    assert!(outdated.message.contains("outdated PDF"));
    assert!(outdated
        .diagnostics
        .values()
        .flatten()
        .any(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR)));
    assert!(
        pdf.is_file(),
        "the previous PDF should remain available on disk"
    );
}

#[test]
fn external_configured_root_uris_round_trip_without_rebasing_or_masking_missing_roots() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let folder = root.join("workspace/a/b/c");
    std::fs::create_dir_all(&folder).unwrap();
    let name = "主文档 main.tex";
    let main_path = root.join("workspace/a").join(name);
    let wrong_path = root.join(name);
    for path in [
        &main_path,
        &wrong_path,
        &root.join("workspace/a/unrelated.tex"),
    ] {
        std::fs::write(path, "\\documentclass{article}").unwrap();
    }
    std::fs::write(folder.join("chapter.tex"), "Chapter.").unwrap();
    let source = Url::from_file_path(folder.join("chapter.tex")).unwrap();
    let main = Url::from_file_path(&main_path).unwrap();
    let wrong = Url::from_file_path(&wrong_path).unwrap();
    let unrelated = Url::from_file_path(root.join("workspace/a/unrelated.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(folder);
    workspace.config.build.root_file = Some(format!("../../{name}"));
    assert_eq!(workspace.root_document(&source).unwrap(), main);
    assert_eq!(workspace.root_document(&main).unwrap(), main);
    workspace.add_root(root.join("workspace/a"));
    assert_eq!(workspace.root_document(&source).unwrap(), main);
    assert_eq!(workspace.root_document(&main).unwrap(), main);
    assert_eq!(workspace.root_document(&wrong).unwrap(), wrong);
    assert_eq!(workspace.root_document(&unrelated).unwrap(), wrong);
    std::fs::remove_file(&main_path).unwrap();
    assert!(
        workspace.root_document(&main).is_err(),
        "A removed configured root must not resolve to an existing unrelated file"
    );
    assert!(workspace.root_document(&source).is_err());
}

#[test]
fn root_detection_ignores_deleted_indexed_roots_without_waiting_for_a_file_notification() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a.tex", "b.ltx"] {
        std::fs::write(root.join(name), "\\documentclass{article}\\input{chapter}").unwrap();
    }
    std::fs::write(root.join("chapter.tex"), "Chapter.").unwrap();
    let source = Url::from_file_path(root.join("chapter.tex")).unwrap();
    let first = Url::from_file_path(root.join("a.tex")).unwrap();
    let second = Url::from_file_path(root.join("b.ltx")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert!(workspace.root_document(&source).is_err());
    std::fs::remove_file(root.join("a.tex")).unwrap();
    assert!(workspace.documents.contains_key(&first));
    assert_eq!(workspace.root_document(&source).unwrap(), second);
    assert!(
        workspace.root_document(&first).is_err(),
        "Direct builds of a deleted main must still report the missing root"
    );
    std::fs::create_dir(root.join("a.tex")).unwrap();
    assert_eq!(workspace.root_document(&source).unwrap(), second);
    workspace.config.build.root_file = Some("a.tex".into());
    assert!(
        workspace.root_document(&source).is_err(),
        "An explicit missing root must not fall back to another project"
    );
}

#[test]
fn package_class_and_support_file_document_examples_do_not_become_compilation_roots() {
    for extension in ["sty", "cls", "def", "clo"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(
            root.join("main.tex"),
            format!("\\documentclass{{article}}\\input{{support.{extension}}}"),
        )
        .unwrap();
        std::fs::write(
            root.join(format!("support.{extension}")),
            "\\newcommand{\\beginLater}{\\begin{document}}\\input{chapter}",
        )
        .unwrap();
        std::fs::write(root.join("chapter.tex"), "Chapter.").unwrap();
        let main = Url::from_file_path(root.join("main.tex")).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        for name in ["chapter.tex".to_owned(), format!("support.{extension}")] {
            let source = Url::from_file_path(root.join(name)).unwrap();
            assert_eq!(
                workspace.root_document(&source).unwrap(),
                main,
                "{extension}"
            );
        }
        workspace.config.build.root_file = Some(format!("support.{extension}"));
        assert!(workspace.root_document(&main).is_err());
    }
}

#[test]
fn root_detection_follows_includes_magic_comments_and_explicit_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(
        root.join("main.tex"),
        r"\documentclass{article}\input{chapter}",
    )
    .unwrap();
    std::fs::write(root.join("chapter.tex"), "Chapter").unwrap();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let chapter = Url::from_file_path(root.join("chapter.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    assert_eq!(workspace.root_document(&chapter).unwrap(), main);
    std::fs::write(
        root.join("other.tex"),
        r"\documentclass{article}\input{chapter}",
    )
    .unwrap();
    let other = Url::from_file_path(root.join("other.tex")).unwrap();
    workspace.reload(&other);
    assert!(workspace.root_document(&chapter).is_err());
    std::fs::write(root.join("chapter.tex"), "% !TeX root = main.tex\nChapter").unwrap();
    workspace.reload(&chapter);
    assert_eq!(workspace.root_document(&chapter).unwrap(), main);
    workspace.config.build.root_file = Some("other.tex".into());
    assert_eq!(workspace.root_document(&chapter).unwrap(), other);
    workspace.config.build.root_file = None;
    std::fs::write(root.join("main.tex"), "% !TeX root = chapter.tex\n").unwrap();
    workspace.reload(&main);
    assert!(workspace
        .root_document(&chapter)
        .unwrap_err()
        .contains("Cyclic"));
}

#[test]
fn root_magic_comments_inside_verbatim_are_ignored() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let other = directory.path().join("other.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{verbatim}\n% !TeX root = other.tex\n\\end{verbatim}\n",
    )
    .unwrap();
    std::fs::write(&other, "Other").unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(directory.path().into());
    let uri = Url::from_file_path(&main).unwrap();
    assert_eq!(workspace.root_document(&uri).unwrap(), uri);
    assert!(latex_lsp::navigation::document_links(&workspace, &uri).is_empty());
}

#[test]
fn project_refresh_follows_updated_children_packages_and_root_relative_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join("sections")).unwrap();
    for (name, source) in [
        (
            "main.tex",
            "\\documentclass{local}\\usepackage{macros}\\input{sections/child}",
        ),
        ("local.cls", "\\LoadClass{article}"),
        ("macros.sty", "\\RequirePackage{other}"),
        ("other.sty", "\\newcommand{\\hello}{Hello}"),
        ("sections/child.tex", "Chapter"),
        ("nested.v1.tex", "\\bibliography{refs\\_v1}"),
        ("refs_v1.bib", "@book{example,title={A}}"),
    ] {
        std::fs::write(root.join(name), source).unwrap();
    }
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let mut workspace = Workspace::default();
    workspace.add_root(root.into());
    // Introduce a new include in a closed file after it has been indexed.
    std::fs::write(root.join("sections/child.tex"), "\\input{nested.v1}").unwrap();
    workspace.refresh_project(&main);
    let project = workspace.project_documents(&main);
    assert_eq!(project.len(), 7);
    for name in [
        "local.cls",
        "macros.sty",
        "other.sty",
        "nested.v1.tex",
        "refs_v1.bib",
    ] {
        let uri = Url::from_file_path(root.join(name)).unwrap();
        assert!(project.contains(&uri), "missing {name}");
        assert_eq!(workspace.root_document(&uri).unwrap(), main);
    }
}

#[test]
fn missing_compiler_produces_an_actionable_error_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, r"\documentclass{article}").unwrap();
    let config = BuildConfig {
        executable: directory
            .path()
            .join("missing-compiler")
            .to_string_lossy()
            .into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error");
    assert!(outcome.message.contains("Cannot start"));
    assert_eq!(
        outcome.diagnostics[&Url::from_file_path(main).unwrap()][0].severity,
        Some(DiagnosticSeverity::ERROR)
    );
}

#[cfg(unix)]
fn script(directory: &std::path::Path, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join("fake compiler");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into()
}

#[test]
#[cfg(unix)]
fn bibliography_log_errors_are_reported_even_when_tex_produced_a_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("references.bib");
    std::fs::write(&main, "\\bibliography{references}\n").unwrap();
    std::fs::write(
        &bib,
        "@book{example, title={One}}\n@book{example, title={Two}}\n",
    )
    .unwrap();
    let executable = script(directory.path(), "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'Repeated entry---line 2 of file references.bib\\n' > \"$1/main.blg\"\n");
    let mut config = BuildConfig {
        executable,
        arguments: Some(vec!["%o".into()]),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error");
    assert!(outcome.pdf.is_none());
    assert!(outcome.message.contains("Repeated entry"));
    assert_eq!(
        outcome.diagnostics[&Url::from_file_path(bib).unwrap()][0]
            .range
            .start
            .line,
        1
    );
    // A later compiler invocation that does not touch the old .blg must ignore it.
    config.executable = "/bin/sh".into();
    config.arguments = Some(vec!["-c".into(), "exit 0".into()]);
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success");
    assert!(outcome.diagnostics.is_empty());
}

#[test]
#[cfg(unix)]
fn arguments_are_passed_without_shell_interpolation_and_dotted_names_work() {
    let directory = tempfile::Builder::new()
        .prefix("latex lsp 中文 ")
        .tempdir()
        .unwrap();
    let main = directory.path().join("main.v1.tex");
    std::fs::write(&main, "source\nsource\n").unwrap();
    let executable = script(directory.path(), "[ \"$1\" = 'literal;$(false)' ] || exit 99\nprintf '%%PDF-1.4\n' > \"$2/main.v1.pdf\"\nprintf '%s:2: LaTeX Warning: example warning.\n' \"$3\"");
    let config = BuildConfig {
        executable,
        arguments: Some(vec!["literal;$(false)".into(), "%o".into(), "%f".into()]),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(outcome.pdf.unwrap().ends_with("main.v1.pdf"));
    assert_eq!(
        outcome.diagnostics[&Url::from_file_path(main).unwrap()][0]
            .range
            .start
            .line,
        1
    );
    assert!(!directory.path().join("false").exists());
}

#[test]
fn builds_cancelled_before_start_preserve_artifacts_and_do_not_create_output_directories() {
    for existing in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "source").unwrap();
        let config = BuildConfig {
            // Cancellation must take precedence over trying an unavailable tool.
            executable: directory
                .path()
                .join("missing-compiler")
                .to_string_lossy()
                .into(),
            output_directory: "out/reports".into(),
            job_name: Some("résumé.1".into()),
            ..Default::default()
        };
        let output = directory.path().join(&config.output_directory);
        let extensions = ["log", "pdf", "fls", "blg", "aux", "synctex.gz"];
        if existing {
            std::fs::create_dir_all(&output).unwrap();
            for extension in extensions {
                std::fs::write(
                    config.artifact_path(&main, extension).unwrap(),
                    "previous build",
                )
                .unwrap();
            }
        }
        let outcome = build::run(&main, &config, &AtomicBool::new(true));
        assert_eq!(outcome.status, "cancelled", "{}", outcome.message);
        assert_eq!(outcome.message, "Compilation cancelled");
        assert_eq!(outcome.exit_code, None);
        assert!(outcome.diagnostics.is_empty());
        assert!(outcome.diagnostic_sources.is_empty());
        assert!(!outcome.diagnostics_truncated);
        assert!(outcome.pdf.is_none());
        assert!(outcome.log.is_none());
        assert!(outcome.pages.is_none());
        assert!(outcome.inputs.is_none());
        assert!(outcome.bibliography_inputs.is_none());
        assert!(!outcome.pdf_updated);
        assert!(outcome.output.is_empty());
        assert_eq!(output.exists(), existing);
        if existing {
            for extension in extensions {
                assert_eq!(
                    std::fs::read_to_string(config.artifact_path(&main, extension).unwrap())
                        .unwrap(),
                    "previous build"
                );
            }
        }
    }
}

#[test]
#[cfg(unix)]
fn cancellation_and_timeout_terminate_the_compiler_process_group() {
    for cancelled in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "source").unwrap();
        let executable = script(
            directory.path(),
            "sleep 30 &\necho $! > \"$1/child.pid\"\nwait",
        );
        let config = BuildConfig {
            executable,
            arguments: Some(vec!["%o".into()]),
            timeout_seconds: 1,
            ..Default::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let started = Instant::now();
        let worker = std::thread::spawn(move || build::run(&main, &config, &worker_cancel));
        let pid_file = directory.path().join("build/latex-lsp/child.pid");
        while !pid_file.is_file() {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        if cancelled {
            cancel.store(true, Ordering::Relaxed);
        }
        let outcome = worker.join().unwrap();
        assert_eq!(
            outcome.status,
            if cancelled { "cancelled" } else { "timeout" }
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        #[cfg(target_os = "linux")]
        {
            let pid = std::fs::read_to_string(pid_file).unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            // SIGKILL delivery and the kernel's final process-state update are
            // asynchronous even after the parent has been reaped.
            while let Ok(state) = std::fs::read_to_string(format!("/proc/{}/stat", pid.trim())) {
                if state
                    .split(')')
                    .nth(1)
                    .unwrap()
                    .trim_start()
                    .starts_with('Z')
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "compiler descendant was not terminated: {state}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn real_hyperref_urls_with_percent_escapes_compile_without_syntax_errors() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "\\documentclass{article}\n\\usepackage{hyperref}\n\\begin{document}\n\\url{https://example.com/a%20b?x=some_value}\n\\href{https://example.com/a%20b#section}{\\textbf{Link}}\n\\end{document}\n";
    std::fs::write(&main, source).unwrap();
    let uri = Url::from_file_path(&main).unwrap();
    let document = latex_lsp::workspace::Document::new(&uri, source.into(), Some(1));
    assert!(latex_lsp::features::diagnostics(&document).is_empty());
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
}

#[test]
#[ignore = "requires latexmk, BibTeX and a TeX installation"]
fn real_bibliography_supports_configured_hidden_and_external_output_directories() {
    let directory = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main,"\\documentclass{article}\n\\begin{document}\n\\cite{example}\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}").unwrap();
    std::fs::write(
        directory.path().join("references.bib"),
        "@book{example,author={A},title={Example},year={2025},publisher={P}}",
    )
    .unwrap();
    for output_directory in [
        ".custom-output".into(),
        external.path().to_string_lossy().into_owned(),
    ] {
        let config = BuildConfig {
            output_directory,
            ..Default::default()
        };
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(
            outcome.status, "success",
            "{}\n{}",
            outcome.message, outcome.output
        );
        assert!(outcome.pdf.unwrap().is_file());
    }
}

#[test]
#[ignore = "requires latexmk and BibTeX"]
fn real_bibtex_directory_changes_resolve_shadowed_database_inputs_and_warning_locations() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\cite{example}\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n").unwrap();
    std::fs::write(
        root.join("refs.bib"),
        "@book{example,title={Root},year={2026},publisher={P}}\n",
    )
    .unwrap();
    let output = root.join(".custom-output");
    std::fs::create_dir(&output).unwrap();
    let bib = output.join("refs.bib");
    std::fs::write(
        &bib,
        "% selected in BibTeX cwd\n\n@book{example,title={Output},year={2026},publisher={P}}\n",
    )
    .unwrap();
    let config = BuildConfig {
        output_directory: ".custom-output".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "success", "{}", outcome.output);
    assert!(std::fs::read_to_string(output.join("main.bbl"))
        .unwrap()
        .contains("Root"));
    let bib = Url::from_file_path(root.join("refs.bib")).unwrap();
    assert!(outcome.bibliography_inputs.unwrap().contains(&bib));
    assert!(
        outcome.diagnostics[&bib]
            .iter()
            .any(|diagnostic| diagnostic.message.contains("empty author")
                && diagnostic.range.start.line == 0),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
#[ignore = "requires XeLaTeX and a TeX installation"]
fn real_missing_glyphs_and_font_substitutions_are_warnings_even_with_a_valid_pdf() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\fontfamily{missingfont}\\selectfont\nHello \\char\"1F600.\n\\end{document}\n").unwrap();
    let config = BuildConfig {
        executable: "xelatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(outcome.pdf.unwrap().is_file());
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(&main).unwrap()];
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.starts_with("LaTeX Font Warning:")),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.iter().any(
            |diagnostic| diagnostic.message.starts_with("Missing character:")
                && diagnostic.message.contains("U+1F600")
        ),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING)),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "requires latexmk, BibTeX and a TeX installation"]
fn real_bibtex_failure_points_to_the_bib_file() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let bib = directory.path().join("references.bib");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\cite{example}\n\\bibliographystyle{plain}\n\\bibliography{references}\n\\end{document}\n").unwrap();
    std::fs::write(&bib, "@book{example, author={A}, title={One}, year={2025}, publisher={P}}\n@book{example, title={Two}}\n").unwrap();
    let outcome = build::run(&main, &BuildConfig::default(), &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "error",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(outcome.pdf.is_none());
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(bib).unwrap()];
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == 1
                && diagnostic.message.contains("Repeated entry")),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn real_latexmk_produces_pdf_and_uses_only_the_final_pass_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.v1.tex");
    std::fs::write(&main, "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\nSee~\\ref{sec:x}.\n\\end{document}\n").unwrap();
    std::fs::write(
        directory.path().join("chapter.tex"),
        r"\section{Hello}\label{sec:x}",
    )
    .unwrap();
    let outcome = build::run(&main, &BuildConfig::default(), &AtomicBool::new(false));
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    assert!(outcome.pdf.unwrap().is_file());
    assert!(outcome.log.unwrap().is_file());
    assert!(
        outcome
            .diagnostics
            .values()
            .flatten()
            .all(|diagnostic| !diagnostic.message.contains("undefined")),
        "{:?}",
        outcome.diagnostics
    );
}

#[test]
#[ignore = "requires pdflatex and a TeX installation"]
fn real_pdflatex_error_points_to_the_included_file_and_line() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let chapter = directory.path().join("chapter.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(&chapter, "Hello\n\\unknownLspCommand\n").unwrap();
    let config = BuildConfig {
        executable: "pdflatex".into(),
        ..Default::default()
    };
    let outcome = build::run(&main, &config, &AtomicBool::new(false));
    assert_eq!(outcome.status, "error", "{}", outcome.output);
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(chapter).unwrap()];
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == 1
                && diagnostic.message.contains("Undefined control sequence")),
        "{diagnostics:?}"
    );
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn real_definition_file_warnings_with_spaces_and_parentheses_keep_their_source_context() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let definition = directory.path().join("settings (old).def");
    let options = directory.path().join("layout options.clo");
    let long_name = format!("settings {}(old).def", "long-name-".repeat(12));
    let long_definition = directory.path().join(&long_name);
    let unicode_name = format!("settings {}(old).def", "中".repeat(50));
    let unicode_definition = directory.path().join(&unicode_name);
    std::fs::write(&main,format!("\\documentclass{{article}}\n\\begin{{document}}\n\\input{{settings (old).def}}\n\\input{{layout options.clo}}\n\\PackageWarning{{audit}}{{Root warning}}\n\\input{{{long_name}}}\n\\input{{{unicode_name}}}\nText.\n\\end{{document}}\n")).unwrap();
    std::fs::write(&definition,"% configuration\n\\PackageWarning{audit}{Closing ) marker}\n\\PackageWarning{audit}{Still in definition}\n").unwrap();
    std::fs::write(
        &options,
        "% class options\n\\PackageWarning{audit}{Class options warning}\n",
    )
    .unwrap();
    std::fs::write(
        &long_definition,
        "\\PackageWarning{audit}{Wrapped filename warning}\n",
    )
    .unwrap();
    std::fs::write(
        &unicode_definition,
        "\\PackageWarning{audit}{Unicode wrapped filename warning}\n",
    )
    .unwrap();
    let outcome = build::run(&main, &Default::default(), &Default::default());
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    let definition_diagnostics = &outcome.diagnostics[&Url::from_file_path(&definition).unwrap()];
    assert_eq!(definition_diagnostics.len(), 2, "{:?}", outcome.diagnostics);
    assert!(definition_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.range.start.line == 2
            && diagnostic.message.contains("Still in definition")));
    assert_eq!(
        outcome.diagnostics[&Url::from_file_path(&options).unwrap()][0]
            .range
            .start
            .line,
        1
    );
    assert!(outcome.diagnostics[&Url::from_file_path(&main).unwrap()]
        .iter()
        .any(|diagnostic| diagnostic.range.start.line == 4
            && diagnostic.message.contains("Root warning")));
    assert!(
        outcome.diagnostics[&Url::from_file_path(&long_definition).unwrap()][0]
            .message
            .contains("Wrapped filename warning")
    );
    assert!(
        outcome.diagnostics[&Url::from_file_path(&unicode_definition).unwrap()][0]
            .message
            .contains("Unicode wrapped filename warning")
    );
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn real_undefined_reference_and_citation_warnings_highlight_only_the_missing_keys() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let chapter = directory.path().join("chapter.tex");
    std::fs::write(
        &main,
        "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
    )
    .unwrap();
    let source = "See \\ref{missing} and \\cite{lost}.\n";
    std::fs::write(&chapter, source).unwrap();
    let outcome = build::run(&main, &Default::default(), &Default::default());
    assert_eq!(
        outcome.status, "success",
        "{}\n{}",
        outcome.message, outcome.output
    );
    let diagnostics = &outcome.diagnostics[&Url::from_file_path(&chapter).unwrap()];
    let text = latex_lsp::text::Text::new(source.into());
    let values: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("undefined"))
        .map(|diagnostic| {
            &source[text.offset(diagnostic.range.start).unwrap()
                ..text.offset(diagnostic.range.end).unwrap()]
        })
        .collect();
    assert_eq!(values, vec!["missing", "lost"], "{:?}", outcome.diagnostics);
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex, lualatex and hyperref"]
fn real_hyperref_warnings_select_bracketed_labels_and_clear_after_repair() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for file_line in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let chapter = directory.path().join("chapter.tex");
            std::fs::write(&main, "\\documentclass{article}\\usepackage{hyperref}\n\\begin{document}\\section{Title}\\label{sec:intro}\\input{chapter}\\end{document}\n").unwrap();
            let source = concat!(
                "\\hyperref[sec:itnro]{sec:itnro} \\ref{sec:itnro}\n",
                "\\hyperref\n[sec:itnro]{\nLink text\n}\n",
                "\\hyperref[{sec:itnro}]{Grouped link}\n",
                "\\hyperref{https://example.com/a%20b?x=some_value#section}{sec:itnro}{sec:itnro}{URL}\n",
            );
            std::fs::write(&chapter, source).unwrap();
            let document = latex_lsp::workspace::Document::new(
                &Url::from_file_path(&chapter).unwrap(),
                source.into(),
                None,
            );
            assert!(
                latex_lsp::features::diagnostics(&document).is_empty(),
                "{:?}",
                document.syntax.diagnostics
            );
            let mut arguments = vec![
                "-interaction=nonstopmode".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ];
            if file_line {
                arguments.insert(0, "-file-line-error".into());
            }
            let executable = if engine == "lualatex" {
                script(directory.path(), "TEXMFVAR=\"${0%/*}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec lualatex \"$@\"")
            } else {
                engine.into()
            };
            let config = latex_lsp::config::BuildConfig {
                executable,
                arguments: Some(arguments),
                ..Default::default()
            };
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            let uri = Url::from_file_path(&chapter).unwrap();
            let warnings: Vec<_> = outcome
                .diagnostics
                .get(&uri)
                .into_iter()
                .flatten()
                .filter(|diagnostic| diagnostic.message.contains("undefined"))
                .collect();
            assert_eq!(
                warnings.len(),
                4,
                "{engine}: {:?}\n{}",
                outcome.diagnostics,
                outcome.output
            );
            let text = latex_lsp::text::Text::new(source.into());
            for warning in warnings {
                assert_eq!(
                    &source[text.offset(warning.range.start).unwrap()
                        ..text.offset(warning.range.end).unwrap()],
                    "sec:itnro",
                    "{engine}: {warning:?}"
                );
                if warning.range.start.line == 0 {
                    assert_eq!(
                        warning.range.start.character,
                        if warning.message.contains("Hyper reference") {
                            10
                        } else {
                            text.position(source.find("\\ref{").unwrap() + 5).character
                        },
                        "{warning:?}"
                    );
                }
            }
            std::fs::write(
                &chapter,
                source
                    .replace("[sec:itnro]", "[sec:intro]")
                    .replace("[{sec:itnro}]", "[{sec:intro}]")
                    .replace("\\ref{sec:itnro}", "\\ref{sec:intro}"),
            )
            .unwrap();
            for _ in 0..2 {
                let repaired = build::run(&main, &config, &Default::default());
                assert_eq!(repaired.status, "success", "{}", repaired.output);
            }
            let repaired = build::run(&main, &config, &Default::default());
            assert!(
                !repaired
                    .diagnostics
                    .values()
                    .flatten()
                    .any(|diagnostic| diagnostic.message.contains("undefined")),
                "{engine}: {:?}",
                repaired.diagnostics
            );
        }
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex, lualatex, hyperref and refcount"]
fn real_capitalized_and_named_page_reference_warnings_distinguish_starred_calls_and_getters() {
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for file_line in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let child = root.join("chapter.tex");
            std::fs::write(&main, "\\documentclass{article}\\usepackage{hyperref}\\usepackage{refcount}\\begin{document}\\input{chapter}\\end{document}\n").unwrap();
            let source = concat!(
                "\\Ref{lost-upper}\n",
                "\\Ref * {lost-star}\n",
                "\\autopageref{lost-page}\n",
                "\\autopageref * {lost-page-star}\n",
                "\\Ref*{same} \\hyperref[same]{Link} \\getrefnumber{same}\n",
            );
            std::fs::write(&child, source).unwrap();
            let mut arguments = vec![
                "-interaction=nonstopmode".into(),
                "-output-directory=%o".into(),
                "%f".into(),
            ];
            if file_line {
                arguments.insert(0, "-file-line-error".into());
            }
            let executable = if engine == "lualatex" {
                script(root, "TEXMFVAR=\"${0%/*}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nexport TEXMFVAR TEXMFCACHE\nmkdir -p \"$TEXMFVAR\"\nexec lualatex \"$@\"")
            } else {
                engine.into()
            };
            let config = BuildConfig {
                executable,
                arguments: Some(arguments),
                ..Default::default()
            };
            let result = build::run(&main, &config, &Default::default());
            assert_eq!(result.status, "success", "{engine}: {}", result.output);
            let diagnostics: Vec<_> = result.diagnostics[&Url::from_file_path(&child).unwrap()]
                .iter()
                .filter(|diagnostic| diagnostic.message.contains("undefined"))
                .collect();
            assert_eq!(diagnostics.len(), 8, "{engine}: {diagnostics:?}");
            let text = latex_lsp::text::Text::new(source.into());
            for diagnostic in diagnostics {
                let value = &source[text.offset(diagnostic.range.start).unwrap()
                    ..text.offset(diagnostic.range.end).unwrap()];
                assert!(
                    matches!(
                        value,
                        "lost-upper" | "lost-star" | "lost-page" | "lost-page-star" | "same"
                    ),
                    "{engine}: {diagnostic:?}"
                );
                if diagnostic.range.start.line == 4 {
                    let line = source.lines().nth(4).unwrap();
                    let offset = if diagnostic.message.contains("Hyper reference") {
                        line.find("[same]").unwrap() + 1
                    } else {
                        line.find("{same}").unwrap() + 1
                    };
                    assert_eq!(
                        diagnostic.range.start.character, offset as u32,
                        "{engine}: {diagnostic:?}"
                    );
                }
            }
        }
    }
}

#[test]
#[cfg(unix)]
fn partial_bibliography_log_sets_discard_all_new_bindings_including_complete_sibling_logs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = root.join("main.tex");
    let physical = root.join("physical.bib");
    let alias = root.join("refs.bib");
    std::fs::write(&main, "Source\n").unwrap();
    std::fs::write(&physical, "@book{entry}\n").unwrap();
    std::os::unix::fs::symlink(&physical, &alias).unwrap();
    let executable = script(root, "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'Database file #1: refs.bib\\n' > \"$1/complete.blg\"\ncp partial-template \"$1/other.blg\"\n");
    let config = BuildConfig {
        executable,
        arguments: Some(vec!["%o".into()]),
        ..Default::default()
    };
    for partial in [false, true] {
        let contents = if partial {
            format!(
                "Database file #1: refs.bib\n{}\n",
                " ".repeat(9 * 1024 * 1024)
            )
        } else {
            "Bibliography completed\n".into()
        };
        std::fs::write(root.join("partial-template"), contents).unwrap();
        let outcome = build::run(&main, &config, &AtomicBool::new(false));
        assert_eq!(outcome.status, "success", "{}", outcome.message);
        assert_eq!(outcome.diagnostics_truncated, partial);
        if partial {
            assert!(outcome.bibliography_inputs.is_none());
            assert!(outcome.bibliography_bindings.is_empty());
        } else {
            assert!(outcome
                .bibliography_inputs
                .unwrap()
                .contains(&Url::from_file_path(&physical).unwrap()));
            assert_eq!(outcome.bibliography_bindings[&alias], physical);
        }
    }
}

#[test]
fn invalid_or_cancelled_rebuilds_preserve_previous_artifacts() {
    for existing in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(&main, "source").unwrap();
        let config = BuildConfig {
            executable: "unavailable-custom-compiler".into(),
            ..Default::default()
        };
        let output = directory.path().join(&config.output_directory);
        if existing {
            std::fs::create_dir_all(&output).unwrap();
            for extension in ["pdf", "log", "aux", "fls"] {
                std::fs::write(config.artifact_path(&main, extension).unwrap(), "previous")
                    .unwrap();
            }
        }
        for cancelled in [false, true] {
            let outcome = build::rebuild(&main, &config, &AtomicBool::new(cancelled));
            assert_eq!(
                outcome.status,
                if cancelled { "cancelled" } else { "error" }
            );
            assert!(outcome.pdf.is_none());
            assert!(outcome.log.is_none());
            assert_eq!(output.exists(), existing);
            if existing {
                for extension in ["pdf", "log", "aux", "fls"] {
                    assert_eq!(
                        std::fs::read_to_string(config.artifact_path(&main, extension).unwrap())
                            .unwrap(),
                        "previous"
                    );
                }
            }
        }
    }
}

#[test]
#[cfg(unix)]
fn custom_rebuild_invokes_only_explicit_arguments_and_parses_its_log() {
    let directory = tempfile::Builder::new()
        .prefix("force %o 中文 ")
        .tempdir()
        .unwrap();
    let main = directory.path().join("main%j.tex");
    std::fs::write(&main, "source\n").unwrap();
    let executable = script(
        directory.path(),
        r#"[ "$1" = 'force;$(false)' ] || exit 99
printf '%%PDF-1.4\n' > "$2/$4.pdf"
printf '%s:1: LaTeX Warning: rebuilt.\n' "$3" > "$2/$4.log"
"#,
    );
    let config = BuildConfig {
        executable,
        arguments: Some(vec!["normal".into()]),
        rebuild_arguments: Some(vec![
            "force;$(false)".into(),
            "%o".into(),
            "%f".into(),
            "%j".into(),
        ]),
        ..Default::default()
    };
    let outcome = build::rebuild(&main, &config, &Default::default());
    assert_eq!(outcome.status, "success", "{}", outcome.message);
    assert!(outcome.pdf.unwrap().ends_with("main%j.pdf"));
    assert!(outcome.diagnostics[&Url::from_file_path(main).unwrap()]
        .iter()
        .any(|item| item.message.contains("rebuilt")));
    assert_eq!(config.arguments(), vec!["normal"]);
}

#[test]
#[ignore = "requires latexmk, pdflatex and xelatex"]
fn real_forced_latexmk_rebuild_replaces_an_existing_pdf_after_a_noop() {
    for engine in [
        latex_lsp::config::Engine::PdfLatex,
        latex_lsp::config::Engine::XeLatex,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        std::fs::write(
            &main,
            "\\documentclass{article}\\begin{document}Expected.\\end{document}\n",
        )
        .unwrap();
        let config = BuildConfig {
            engine: Some(engine),
            job_name: Some("draft.v2".into()),
            ..Default::default()
        };
        let initial = build::run(&main, &config, &Default::default());
        assert_eq!(initial.status, "success", "{}", initial.output);
        let pdf = config.artifact_path(&main, "pdf").unwrap();
        let modified = std::fs::metadata(&pdf).unwrap().modified().unwrap();
        let mut corrupted = std::fs::read(&pdf).unwrap();
        let last = corrupted.last_mut().unwrap();
        *last = if *last == b' ' { b'\n' } else { b' ' };
        std::fs::write(&pdf, &corrupted).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&pdf)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        let normal = build::run(&main, &config, &Default::default());
        assert!(normal.output.contains("Nothing to do"), "{}", normal.output);
        assert_eq!(std::fs::read(&pdf).unwrap(), corrupted);
        let forced = build::rebuild(&main, &config, &Default::default());
        assert_eq!(forced.status, "success", "{}", forced.output);
        assert!(forced.output.contains("applying rule"), "{}", forced.output);
        assert_ne!(std::fs::read(&pdf).unwrap(), corrupted);
        assert!(forced.pdf_updated);
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and listings"]
fn real_listings_code_inputs_are_recorded_as_data_dependencies() {
    for engine in ["pdflatex", "xelatex"] {
        for (extension, code) in [
            ("rs", "fn main() { println!(\"Rust listing\"); }\n"),
            ("py", "print(\"Python listing\")\n"),
            ("cpp", "int main() { return 0; }\n"),
            ("js", "console.log(\"JS listing\");\n"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let code_path = root.join(format!("sample.{extension}"));
            std::fs::write(&code_path, code).unwrap();
            std::fs::write(&main, format!("\\documentclass{{article}}\\usepackage{{listings}}\\begin{{document}}\\lstinputlisting{{sample.{extension}}}\\end{{document}}\n")).unwrap();
            let config = BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(
                outcome.status, "success",
                "{engine}/{extension}: {}",
                outcome.output
            );
            let code_uri = Url::from_file_path(&code_path).unwrap();
            assert!(outcome.inputs.as_ref().unwrap().contains(&code_uri));
            assert!(outcome.binary_inputs.contains(&code_uri));
            assert!(outcome.pdf.is_some());
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and verbatim"]
fn real_verbatim_inputs_preserve_arbitrary_file_names_and_code_roles() {
    for engine in ["pdflatex", "xelatex"] {
        for (name, star) in [
            ("sample.rs", ""),
            ("sample.unknown", "*"),
            ("README", " *"),
            ("样例, one.rs", ""),
            ("example.tex", "*"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let code_path = root.join(name);
            std::fs::write(&code_path, "Before.\n\\label{code-only}\nAfter.\n").unwrap();
            std::fs::write(&main, format!("\\documentclass{{article}}\\usepackage{{verbatim}}\\begin{{document}}\\verbatiminput{star}{{{name}}}\\end{{document}}\n")).unwrap();
            let outcome = build::run(
                &main,
                &BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                },
                &Default::default(),
            );
            assert_eq!(
                outcome.status, "success",
                "{engine}/{name}: {}",
                outcome.output
            );
            let code_uri = Url::from_file_path(&code_path).unwrap();
            // Unknown and suffixless names are static data dependencies;
            // recorder classification intentionally accepts known types only.
            let recorder = std::fs::read_to_string(root.join("build/latex-lsp/main.fls")).unwrap();
            assert!(
                recorder
                    .lines()
                    .filter_map(|line| line.strip_prefix("INPUT "))
                    .any(|path| root.join(path).canonicalize().ok().as_ref() == Some(&code_path)),
                "{engine}/{name}: {recorder}"
            );
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let main_uri = Url::from_file_path(&main).unwrap();
            assert!(workspace.project_files(&main_uri).contains(&code_uri));
            assert!(!workspace.project_documents(&main_uri).contains(&code_uri));
            assert!(outcome.pdf.is_some());
        }
    }
}
