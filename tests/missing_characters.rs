use latex_lsp::{build_log, text::Text};
use lsp_types::{DiagnosticSeverity, NumberOrString, Url};

#[test]
fn numbered_missing_character_errors_preserve_severity_and_utf16_character_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    let source = "Header\r\nBefore 中 and 😀 after.\r\n";
    std::fs::write(&main, source).unwrap();
    let log = "(./main.tex\n./main.tex:2: Missing character: There is no 中 (U+4E2D) in font [lmroman10-regu\nlar]:mapping=tex-text;.\nl.2 Before 中 \n             and 😀 after.\n\n./main.tex:2: Missing character: There is no 😀 (U+1F600) in font [lmroman10-reg\nular]:mapping=tex-text;.\nl.2 Before 中 and 😀 \n                   after.\n)\n";
    let items = build_log::parse(&main, log);
    let items = &items[&Url::from_file_path(main).unwrap()];
    assert_eq!(items.len(), 2, "{items:?}");
    let text = Text::new(source.into());
    for (glyph, codepoint) in [('中', "U+4E2D"), ('😀', "U+1F600")] {
        let item = items
            .iter()
            .find(|item| item.message.contains(codepoint))
            .unwrap();
        assert_eq!(item.severity, Some(DiagnosticSeverity::ERROR), "{item:?}");
        assert_eq!(
            item.code,
            Some(NumberOrString::String("missing-character".into()))
        );
        let at = source.find(glyph).unwrap();
        assert_eq!(item.range, text.range(at, at + glyph.len_utf8()));
        assert_eq!(
            item.data,
            Some(serde_json::json!({"character":glyph.to_string(), "codepoint":codepoint}))
        );
    }
}

fn parse(source: &str, log: &str) -> Vec<lsp_types::Diagnostic> {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    build_log::parse(&main, log)
        .remove(&Url::from_file_path(main).unwrap())
        .unwrap_or_default()
}

const GLYPH: &str = "Missing character: There is no 中 (U+4E2D) in font nullfont!";

#[test]
fn warning_and_error_forms_remain_distinct_even_at_the_same_location() {
    for log in [
        format!("(./main.tex\n{GLYPH}\n./main.tex:1: {GLYPH}\n{GLYPH}\n)\n"),
        format!("(./main.tex\n{GLYPH}\n./main.tex:1: {GLYPH}\n)\n"),
        format!("(./main.tex\n./main.tex:1: {GLYPH}\n{GLYPH}\n)\n"),
        format!("(./main.tex\n{GLYPH}\n! {GLYPH}\nl.1 \\emit\n)\n"),
    ] {
        let items = parse(r"\emit", &log);
        assert_eq!(items.len(), 2, "{items:?}");
        for severity in [DiagnosticSeverity::WARNING, DiagnosticSeverity::ERROR] {
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item.severity == Some(severity))
                    .count(),
                1,
                "{items:?}"
            );
        }
        assert!(items
            .iter()
            .all(|item| item.code == Some(NumberOrString::String("missing-character".into()))));
    }
}

#[test]
fn numbered_glyph_errors_are_not_downgraded_by_warning_like_font_names() {
    let message = GLYPH.replace("nullfont", "Warning:font");
    let items = parse(
        "Before 中",
        &format!("./main.tex:1: {message}\nl.1 Before 中 \n"),
    );
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].severity, Some(DiagnosticSeverity::ERROR));
}

#[test]
fn traditional_errors_and_unique_shortened_contexts_locate_direct_glyphs() {
    for (source, context) in [
        ("Before 中 after", "Before 中 "),
        ("Long prefix suffix 中 after", "...suffix 中 "),
    ] {
        let items = parse(
            source,
            &format!("(./main.tex\n! {GLYPH}\nl.1 {context}\n)\n"),
        );
        assert_eq!(items.len(), 1);
        let at = source.find('中').unwrap();
        assert_eq!(
            items[0].range,
            Text::new(source.into()).range(at, at + '中'.len_utf8())
        );
        assert_eq!(items[0].severity, Some(DiagnosticSeverity::ERROR));
    }
}

#[test]
fn ambiguous_expanded_generated_ignored_or_mismatched_contexts_keep_line_ranges() {
    for (source, context) in [
        (r"中 \emit", "\\emit ->中\nl.1 中 \\emit"),
        (r"中 \emit", "l.1 中 \\emit"),
        (r#"\char"4E2D 中"#, "l.1 \\char\"4E2D"),
        ("Other 中 text", "l.1 Before 中 "),
        ("x 中 x 中", "l.1 ...x 中 "),
        ("% 中", "l.1 % 中 "),
        (r"\verb|中|", "l.1 \\verb|中 "),
        (r"中 \emit", "<recently read> 中\nl.1 中 \\emit"),
    ] {
        let items = parse(source, &format!("(./main.tex\n! {GLYPH}\n{context}\n)\n"));
        assert_eq!(items.len(), 1, "{source}: {items:?}");
        assert_eq!(
            items[0].range,
            Text::new(source.into()).range(0, source.len()),
            "{source}: {items:?}"
        );
        assert_eq!(items[0].severity, Some(DiagnosticSeverity::ERROR));
    }
}

#[test]
fn malformed_codepoints_and_legacy_glyph_printing_never_invent_character_identity() {
    for body in [
        "中 (U+1F600) in font nullfont!",
        "中 (U+XYZ1) in font nullfont!",
        "中 (U+D800) in font nullfont!",
        "中 (U+110000) in font nullfont!",
        "中 (U+4E2D) in font ",
        "中 (U+004E2D0) in font nullfont!",
        "^^@ in font nullfont!",
    ] {
        let items = parse(
            "Before 中 after",
            &format!("(./main.tex\n! Missing character: There is no {body}\nl.1 Before 中 \n)\n"),
        );
        assert_eq!(items.len(), 1, "{body}: {items:?}");
        assert_eq!(items[0].severity, Some(DiagnosticSeverity::ERROR));
        assert!(
            items[0].code.is_none() && items[0].data.is_none(),
            "{body}: {items:?}"
        );
        assert_eq!(
            items[0].range,
            Text::new("Before 中 after".into()).range(0, "Before 中 after".len())
        );
    }
}

#[test]
fn unresolved_sources_and_invalid_lines_keep_errors_without_borrowing_glyph_ranges() {
    for (path, line, data) in [
        (
            "./gone.tex",
            1,
            serde_json::json!({"unresolvedSource":true}),
        ),
        (
            "./main.tex",
            0,
            serde_json::json!({"unresolvedLine":true,"reportedLine":0}),
        ),
        (
            "./main.tex",
            99,
            serde_json::json!({"unresolvedLine":true,"reportedLine":99}),
        ),
    ] {
        let items = parse(
            "Before 中",
            &format!("{path}:{line}: {GLYPH}\nl.{line} Before 中 \n"),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(items[0].code, None);
        assert_eq!(items[0].data, Some(data));
        assert_eq!(
            items[0].range,
            Text::new("Before 中".into()).range(0, "Before 中".len())
        );
    }
}

#[test]
#[ignore = "requires xelatex"]
fn native_xelatex_missing_glyph_levels_preserve_warnings_errors_and_child_positions() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;
    for numbered in [false, true] {
        for level in [2, 3] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let child = directory.path().join("child.tex");
            let source = "Before 中 and 😀 after.\r\n";
            std::fs::write(&main, format!("\\documentclass{{article}}\n\\tracinglostchars={level}\n\\begin{{document}}\n\\input{{child}}\n\\end{{document}}\n")).unwrap();
            std::fs::write(&child, source).unwrap();
            let mut arguments: Vec<String> = [
                "-interaction=nonstopmode",
                "-recorder",
                "-output-directory=%o",
                "%f",
            ]
            .into_iter()
            .map(String::from)
            .collect();
            if numbered {
                arguments.insert(0, "-file-line-error".into());
            }
            let config = BuildConfig {
                executable: "xelatex".into(),
                arguments: Some(arguments),
                ..Default::default()
            };
            let outcome = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(
                outcome.status,
                if level == 3 { "error" } else { "success" },
                "{}",
                outcome.output
            );
            let items = &outcome.diagnostics[&Url::from_file_path(&child).unwrap()];
            let glyphs: Vec<_> = items
                .iter()
                .filter(|item| {
                    item.code == Some(NumberOrString::String("missing-character".into()))
                })
                .collect();
            assert_eq!(glyphs.len(), 2, "{numbered}/{level}: {items:?}");
            let summary = &outcome.analysis.as_ref().unwrap()["missingCharacters"];
            assert_eq!(summary.as_array().unwrap().len(), 2, "{summary}");
            for (glyph, codepoint) in [("中", "U+4E2D"), ("😀", "U+1F600")] {
                let entry = summary
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["character"] == glyph)
                    .unwrap();
                assert_eq!(entry["codepoint"], codepoint);
                assert_eq!(entry["severity"], if level == 3 { 1 } else { 2 });
                let fonts = entry["fonts"].as_array().unwrap();
                assert_eq!(fonts.len(), 1, "{summary}");
                let font = fonts[0].as_str().unwrap();
                assert_eq!(font, "[lmroman10-regular]:mapping=tex-text;", "{summary}");
            }
            let text = Text::new(source.into());
            for glyph in glyphs {
                assert_eq!(
                    glyph.severity,
                    Some(if level == 3 {
                        DiagnosticSeverity::ERROR
                    } else {
                        DiagnosticSeverity::WARNING
                    })
                );
                if level == 3 {
                    let character = glyph.data.as_ref().unwrap()["character"].as_str().unwrap();
                    let at = source.find(character).unwrap();
                    assert_eq!(
                        glyph.range,
                        text.range(at, at + character.len()),
                        "{glyph:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn shared_run_contexts_locate_all_direct_missing_characters_and_preserve_repetitions() {
    for source in ["中文中", "中😀中", "中中", "Before 中文😀 after."] {
        let run = source
            .strip_prefix("Before ")
            .unwrap_or(source)
            .split_whitespace()
            .next()
            .unwrap();
        let consumed = source.find(run).unwrap() + run.len();
        let log = run.chars().map(|glyph| format!("./main.tex:1: Missing character: There is no {glyph} (U+{:04X}) in font nullfont!\nl.1 {} \n\n", glyph as u32, &source[..consumed])).collect::<String>();
        let items = parse(source, &log);
        assert_eq!(items.len(), run.chars().count(), "{source}: {items:?}");
        let text = Text::new(source.into());
        for (offset, glyph) in source
            .char_indices()
            .filter(|(_, glyph)| *glyph == '中' || *glyph == '文' || *glyph == '😀')
        {
            assert!(
                items.iter().any(|item| item.range
                    == text.range(offset, offset + glyph.len_utf8())
                    && item.data.as_ref().unwrap()["character"] == glyph.to_string()),
                "{source}: missing offset {offset}: {items:?}"
            );
        }
    }
}

#[test]
fn glyph_runs_do_not_borrow_prior_words_or_macro_generated_characters() {
    let source = "中 and 中 after.";
    let items = parse(source, &format!("./main.tex:1: {GLYPH}\nl.1 中 and 中 \n"));
    let at = source.rfind('中').unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].range,
        Text::new(source.into()).range(at, at + '中'.len_utf8())
    );
    for (source, context) in [
        (r"\def\emit{中}\emit 中", r"\def\emit{中}\emit 中"),
        (r"\def\emit{中}\emit   中中", r"\def\emit{中}\emit   中中"),
        (r"\emit{中}中", r"\emit{中}中"),
        (r"\emit[中中]", r"\emit[中中]"),
        (r"\def\saved{Before 中中}", r"\def\saved{Before 中中"),
        (r"\begin{verbatim}中中", r"\begin{verbatim}中中"),
        (r"% 中文中", r"% 中文中"),
    ] {
        let items = parse(source, &format!("./main.tex:1: {GLYPH}\nl.1 {context}\n"));
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(
            items[0].range,
            Text::new(source.into()).range(0, source.len()),
            "{source}: {items:?}"
        );
    }
}

#[test]
fn repeated_shortened_glyph_contexts_expand_once_without_losing_source_positions() {
    let run = format!("{}😀", "中".repeat(128));
    let source = format!("Prefix {run} after.");
    let context = format!("...{}😀", "中".repeat(8));
    let log = run.chars().map(|glyph| format!("./main.tex:1: Missing character: There is no {glyph} (U+{:04X}) in font nullfont!\nl.1 {context}\n\n", glyph as u32)).collect::<String>();
    let items = parse(&source, &log);
    assert_eq!(items.len(), 129);
    let text = Text::new(source.clone());
    let expected: Vec<_> = source
        .char_indices()
        .filter(|(_, ch)| !ch.is_ascii())
        .map(|(at, ch)| text.range(at, at + ch.len_utf8()))
        .collect();
    assert_eq!(
        items.iter().map(|item| item.range).collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn different_glyph_contexts_and_fonts_retain_their_own_positions() {
    let source = "中中 and 中中";
    let mut log = String::new();
    for context in ["中中", source] {
        for font in ["nullfont", "otherfont"] {
            for _ in 0..2 {
                log.push_str(&format!(
                    "./main.tex:1: {}\nl.1 {context}\n\n",
                    GLYPH.replace("nullfont", font)
                ));
            }
        }
    }
    let items = parse(source, &log);
    assert_eq!(items.len(), 8, "{items:?}");
    let text = Text::new(source.into());
    for (at, _) in source.match_indices('中') {
        let range = text.range(at, at + '中'.len_utf8());
        assert_eq!(items.iter().filter(|item| item.range == range).count(), 2);
    }
}

#[test]
#[ignore = "requires xelatex"]
fn native_xelatex_shared_glyph_runs_keep_repetitions_and_conservative_macro_ranges() {
    use latex_lsp::{build, config::BuildConfig};
    use std::sync::atomic::AtomicBool;
    let long = format!("Before {}😀 after.", "中".repeat(24));
    for numbered in [false, true] {
        for (source, precise) in [
            ("中文中", true),
            ("中😀中", true),
            ("中中", true),
            ("Before 中文😀 after.", true),
            ("中 中", true),
            ("中 and 中 after.", true),
            (r"Word \label{x} 中文中", true),
            (long.as_str(), true),
            (r"\def\emit{中}\emit 中", false),
            (r"\textbf{中中}", false),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let child = directory.path().join("child.tex");
            std::fs::write(&main, "\\documentclass{article}\n\\tracinglostchars=3\n\\begin{document}\n\\input{child}\n\\end{document}\n").unwrap();
            std::fs::write(&child, source).unwrap();
            let mut arguments: Vec<String> = [
                "-interaction=nonstopmode",
                "-recorder",
                "-output-directory=%o",
                "%f",
            ]
            .into_iter()
            .map(String::from)
            .collect();
            if numbered {
                arguments.insert(0, "-file-line-error".into());
            }
            let config = BuildConfig {
                executable: "xelatex".into(),
                arguments: Some(arguments),
                ..Default::default()
            };
            let outcome = build::run(&main, &config, &AtomicBool::new(false));
            assert_eq!(
                outcome.status, "error",
                "{numbered}/{source}: {}",
                outcome.output
            );
            let items = &outcome.diagnostics[&Url::from_file_path(&child).unwrap()];
            let glyphs: Vec<_> = items
                .iter()
                .filter(|item| {
                    item.code == Some(NumberOrString::String("missing-character".into()))
                })
                .collect();
            assert!(!glyphs.is_empty(), "{numbered}/{source}: {items:?}");
            assert!(glyphs
                .iter()
                .all(|item| item.severity == Some(DiagnosticSeverity::ERROR)));
            let text = Text::new(source.into());
            if precise {
                let expected: Vec<_> = source
                    .char_indices()
                    .filter(|(_, ch)| !ch.is_ascii())
                    .map(|(at, ch)| text.range(at, at + ch.len_utf8()))
                    .collect();
                assert_eq!(
                    glyphs.iter().map(|item| item.range).collect::<Vec<_>>(),
                    expected,
                    "{numbered}/{source}: {items:?}\n{}",
                    outcome.output
                );
            } else {
                assert!(
                    glyphs
                        .iter()
                        .all(|item| item.range == text.range(0, source.len())),
                    "{numbered}/{source}: {items:?}"
                );
            }
        }
    }
}
