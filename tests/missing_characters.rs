use latex_lsp::{build_log, text::Text};
use tower_lsp::lsp_types::{DiagnosticSeverity, NumberOrString, Url};

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

fn parse(source: &str, log: &str) -> Vec<tower_lsp::lsp_types::Diagnostic> {
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
