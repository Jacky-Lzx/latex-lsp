use latex_lsp::{build_log, text::Text};
use serde_json::json;
use std::collections::BTreeMap;
use tower_lsp::lsp_types::*;

fn message(character: char) -> String {
    format!("LaTeX Error: Unicode character {character} (U+{:04X})               not set up for use with LaTeX.", character as u32)
}
fn analyze(messages: Vec<Diagnostic>) -> serde_json::Value {
    build_log::compilation_analysis(
        &[(
            Url::parse("file:///unicode-errors/main.tex").unwrap(),
            messages,
        )]
        .into(),
        false,
    )
}
fn error(character: char) -> Diagnostic {
    Diagnostic {
        severity: Some(DiagnosticSeverity::ERROR),
        message: message(character),
        ..Default::default()
    }
}
fn summary(characters: impl IntoIterator<Item = char>) -> serde_json::Value {
    let ordered: std::collections::BTreeSet<_> = characters.into_iter().collect();
    json!(ordered.into_iter().map(|character| json!({"character":character.to_string(),"codepoint":format!("U+{:04X}",character as u32)})).collect::<Vec<_>>())
}

#[test]
fn unsupported_unicode_errors_preserve_each_direct_utf16_range_and_codepoint() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let uri = Url::from_file_path(&main).unwrap();
    let source = "Header\r\nBefore 中 and 😀 and 中 after.\r\n";
    std::fs::write(&main, source).unwrap();
    let text = Text::new(source.into());
    for numbered in [false, true] {
        let mut log = "(./main.tex\n".to_string();
        for (at, character) in source
            .char_indices()
            .filter(|(_, ch)| matches!(ch, '中' | '😀'))
        {
            let header = if numbered { "./main.tex:2:" } else { "!" };
            let end = at + character.len_utf8();
            let gap = if numbered && character.len_utf16() == 2 {
                "\n"
            } else {
                ""
            };
            log.push_str(&format!("{header} LaTeX Error: Unicode character {character} (U+{:04X})\n{gap}               not set up for use with LaTeX.\n\nSee the LaTeX manual or LaTeX Companion for explanation.\nType  H <return>  for immediate help.\n ...\n\nl.2 {}\n                 after.\n\n",character as u32,&source[source.find("Before").unwrap()..end]));
        }
        log.push_str(")\n");
        let parsed = build_log::parse(&main, &log);
        let items = &parsed[&uri];
        assert_eq!(items.len(), 3, "{parsed:?}");
        for (item, (at, character)) in items.iter().zip(
            source
                .char_indices()
                .filter(|(_, ch)| matches!(ch, '中' | '😀')),
        ) {
            assert_eq!(item.range, text.range(at, at + character.len_utf8()));
            assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR));
            assert_eq!(
                item.code,
                Some(NumberOrString::String(
                    "unsupported-unicode-character".into()
                ))
            );
            assert_eq!(
                item.data,
                Some(
                    json!({"character":character.to_string(),"codepoint":format!("U+{:04X}",character as u32)})
                )
            );
        }
        let result = build_log::compilation_analysis(&parsed, false);
        assert_eq!(
            result["unsupportedUnicodeCharacters"],
            summary(['中', '😀'])
        );
        assert_eq!(result["missingCharacters"], json!([]));
        assert_eq!(result["truncated"], false);
    }
}

#[test]
fn unsupported_unicode_summaries_are_strict_deduplicated_and_separate_from_missing_glyphs() {
    let mut diagnostics = vec![error('中'), error('😀'), error('中')];
    diagnostics.push(Diagnostic {
        severity: Some(DiagnosticSeverity::WARNING),
        message: "Missing character: There is no 中 (U+4E2D) in font nullfont!".into(),
        ..Default::default()
    });
    let result = analyze(diagnostics);
    assert_eq!(
        result["unsupportedUnicodeCharacters"],
        summary(['中', '😀'])
    );
    assert_eq!(result["missingCharacters"][0]["character"], "中");
    assert_eq!(result["missingCharacters"][0]["fonts"], json!(["nullfont"]));
    for invalid in [
        "LaTeX Error: Unicode character 中 (U+4E2E) not set up for use with LaTeX.",
        "LaTeX Error: Unicode character 中中 (U+4E2D) not set up for use with LaTeX.",
        "LaTeX Error: Unicode character 中 (U+D800) not set up for use with LaTeX.",
        "LaTeX Error: Unicode character 中 (U+110000) not set up for use with LaTeX.",
        "LaTeX Error: Unicode character 中 (U+4E2D) not set up for use with LaTeX. More text.",
        "LaTeX Error: Unicode character 中 (U+4E2D)",
        "Package custom Error: Unicode character 中 (U+4E2D) not set up for use with LaTeX.",
    ] {
        assert_eq!(
            analyze(vec![Diagnostic {
                message: invalid.into(),
                ..error('中')
            }])["unsupportedUnicodeCharacters"],
            json!([]),
            "{invalid}"
        );
    }
    for severity in [
        None,
        Some(DiagnosticSeverity::WARNING),
        Some(DiagnosticSeverity::INFORMATION),
    ] {
        assert_eq!(
            analyze(vec![Diagnostic {
                severity,
                ..error('中')
            }])["unsupportedUnicodeCharacters"],
            json!([])
        );
    }
}

#[test]
fn unsupported_unicode_summary_budgets_keep_existing_characters_and_diagnostic_truncation() {
    let mut diagnostics: Vec<_> = (0x4e00..0x4e80)
        .map(|value| error(char::from_u32(value).unwrap()))
        .collect();
    assert_eq!(
        analyze(diagnostics.clone())["unsupportedUnicodeCharacters"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    diagnostics.push(error('一'));
    assert_eq!(analyze(diagnostics.clone())["truncated"], false);
    diagnostics.push(error('😀'));
    let result = analyze(diagnostics);
    assert_eq!(
        result["unsupportedUnicodeCharacters"]
            .as_array()
            .unwrap()
            .len(),
        128
    );
    assert_eq!(result["truncated"], true);
    assert_eq!(
        build_log::compilation_analysis(&BTreeMap::new(), true)["truncated"],
        true
    );
}

#[test]
fn unicode_error_metadata_keeps_conservative_ranges_when_literal_sources_are_not_confirmed() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    for (source, prefix, frame, precise) in [
        ("Long prefix unique 中 after.", "...unique 中", "", true),
        ("Before 中 middle Before 中", "...Before 中", "", false),
        ("Before 中 after.", "Wrong 中", "", false),
        (r"\def\bad{Before 中}", r"\def\bad{Before 中", "", false),
        (r"\verb|Before 中|", r"\verb|Before 中", "", false),
        ("% Before 中", "% Before 中", "", false),
        (
            "Before 中 after.",
            "Before 中",
            "\\bad ->Before 中\n",
            false,
        ),
        (r"\textbf{Before 中}", r"\textbf{Before 中}", "", false),
    ] {
        std::fs::write(&main, source).unwrap();
        let log = format!(
            "(./main.tex\n! {}\n{frame}l.1 {prefix}\n\n)\n",
            message('中')
        );
        let parsed = build_log::parse(&main, &log);
        let item = &parsed[&Url::from_file_path(&main).unwrap()][0];
        assert_eq!(
            item.code,
            Some(NumberOrString::String(
                "unsupported-unicode-character".into()
            ))
        );
        assert_eq!(
            item.range.end.character - item.range.start.character == 1,
            precise,
            "{log}: {item:?}"
        );
        assert_eq!(
            build_log::compilation_analysis(&parsed, false)["unsupportedUnicodeCharacters"],
            summary(['中'])
        );
    }
    std::fs::write(&main, "Before 中 after.\n").unwrap();
    for location in ["./gone.tex:1", "./main.tex:0", "./main.tex:900"] {
        let number = location.rsplit(':').next().unwrap();
        let log = format!(
            "(./main.tex\n{location}: {}\nl.{number} Before 中\n\n)\n",
            message('中')
        );
        let parsed = build_log::parse(&main, &log);
        let item = &parsed[&Url::from_file_path(&main).unwrap()][0];
        assert!(item.code.is_none(), "{item:?}");
        assert_eq!(
            build_log::compilation_analysis(&parsed, false)["unsupportedUnicodeCharacters"],
            summary(['中'])
        );
    }
}

#[test]
fn generated_file_line_notes_do_not_hide_unsupported_unicode_summary_entries() {
    let mut diagnostic = error('😀');
    diagnostic
        .message
        .push_str(" (reported line 900 is no longer present in the generated file)");
    diagnostic.data = Some(json!({"reportedLine":900,"unresolvedLine":true}));
    assert_eq!(
        analyze(vec![diagnostic])["unsupportedUnicodeCharacters"],
        summary(['😀'])
    );
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex and lualatex"]
fn real_unicode_input_errors_preserve_child_ranges_and_clear_after_explicit_mappings() {
    use std::os::unix::fs::PermissionsExt;
    let source = "Before 中 and 😀 and 中.\nJoined 👩\u{200d}💻.\nAccent é.\n% ignored 中 😀\n";
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            for numbered in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let main = root.join("main.tex");
                let child = root.join("child.tex");
                let compiler = root.join(engine);
                std::fs::write(&compiler,format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
                std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
                let mut arguments = vec![
                    "-interaction=nonstopmode".into(),
                    "-recorder".into(),
                    "-output-directory=%o".into(),
                    "%f".into(),
                ];
                if numbered {
                    arguments.insert(0, "-file-line-error".into());
                }
                let config = latex_lsp::config::BuildConfig {
                    executable: compiler.to_string_lossy().into(),
                    arguments: Some(arguments),
                    ..Default::default()
                };
                let header = "\\documentclass{article}\n";
                let body = "\\begin{document}\n\\input{child}\n\\end{document}\n";
                std::fs::write(&main, format!("{header}{body}")).unwrap();
                std::fs::write(&child, source).unwrap();
                let result = latex_lsp::build::run(&main, &config, &Default::default());
                let analysis = result.analysis.as_ref().unwrap();
                if engine != "pdflatex" {
                    assert_eq!(
                        result.status, "success",
                        "{engine}/{width}/{numbered}: {}",
                        result.output
                    );
                    assert_eq!(analysis["unsupportedUnicodeCharacters"], json!([]));
                    continue;
                }
                assert_eq!(
                    result.status, "error",
                    "{engine}/{width}/{numbered}: {}",
                    result.output
                );
                assert_eq!(
                    analysis["unsupportedUnicodeCharacters"],
                    summary(['中', '😀', '👩', '\u{200d}', '💻']),
                    "{engine}/{width}/{numbered}: {:?}\n{}",
                    result.diagnostics,
                    result.output
                );
                let uri = Url::from_file_path(&child).unwrap();
                let items: Vec<_> = result.diagnostics[&uri]
                    .iter()
                    .filter(|item| {
                        item.code
                            == Some(NumberOrString::String(
                                "unsupported-unicode-character".into(),
                            ))
                    })
                    .collect();
                assert_eq!(
                    items.len(),
                    6,
                    "{engine}/{width}/{numbered}: {:?}\n{}",
                    result.diagnostics,
                    result.output
                );
                let text = Text::new(source.into());
                for item in items {
                    let at = text.offset(item.range.start).unwrap();
                    let character = source[at..].chars().next().unwrap();
                    assert_eq!(item.range, text.range(at, at + character.len_utf8()));
                    assert_eq!(
                        item.data.as_ref().unwrap()["character"],
                        character.to_string()
                    );
                    assert_eq!(
                        item.data.as_ref().unwrap()["codepoint"],
                        format!("U+{:04X}", character as u32)
                    );
                }
                let mappings="\\DeclareUnicodeCharacter{4E2D}{Z}\n\\DeclareUnicodeCharacter{1F600}{G}\n\\DeclareUnicodeCharacter{1F469}{W}\n\\DeclareUnicodeCharacter{200D}{}\n\\DeclareUnicodeCharacter{1F4BB}{C}\n";
                std::fs::write(&main, format!("{header}{mappings}{body}")).unwrap();
                let repaired = latex_lsp::build::run(&main, &config, &Default::default());
                assert_eq!(
                    repaired.status, "success",
                    "{engine}/{width}/{numbered}: {}",
                    repaired.output
                );
                assert_eq!(
                    repaired.analysis.unwrap()["unsupportedUnicodeCharacters"],
                    json!([])
                );
                assert!(repaired
                    .diagnostics
                    .values()
                    .flatten()
                    .all(|item| item.severity != Some(DiagnosticSeverity::ERROR)));
                assert_eq!(std::fs::read_to_string(&child).unwrap(), source);
            }
        }
    }
}

#[test]
fn unicode_error_continuations_do_not_cross_multiple_blank_lines_or_other_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Before 中 after.\n").unwrap();
    for boundary in [
        "\n\n\n",
        "\n\n! Other error.\n",
        "\n\n(./unavailable.tex\n",
        "\n\n<argument> unsafe\n",
    ] {
        let log=format!("(./main.tex\n! LaTeX Error: Unicode character 中 (U+4E2D){boundary}               not set up for use with LaTeX.\nl.1 Before 中\n\n)\n");
        let parsed = build_log::parse(&main, &log);
        assert_eq!(
            build_log::compilation_analysis(&parsed, false)["unsupportedUnicodeCharacters"],
            json!([]),
            "{log}: {parsed:?}"
        );
    }
}

#[test]
fn oversized_unicode_continuations_respect_the_error_message_budget() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, "Before 中 after.\n").unwrap();
    let log=format!("(./main.tex\n! LaTeX Error: Unicode character 中 (U+4E2D)\n\n{}not set up for use with LaTeX.\nl.1 Before 中\n\n)\n"," ".repeat(5000));
    let parsed = build_log::parse(&main, &log);
    assert_eq!(
        build_log::compilation_analysis(&parsed, false)["unsupportedUnicodeCharacters"],
        json!([])
    );
}
