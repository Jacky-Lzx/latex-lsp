use latex_lsp::compilation::build_log::{compilation_analysis, parse};
use lsp_types::*;
use serde_json::json;
use std::collections::BTreeMap;

fn warning(message: String) -> Diagnostic {
    Diagnostic {
        severity: Some(DiagnosticSeverity::WARNING),
        message,
        ..Default::default()
    }
}

fn shape(requested: &str, replacement: &str) -> Diagnostic {
    warning(format!("LaTeX Font Warning: Font shape `{requested}' undefined (Font) using `{replacement}' instead on input line 3."))
}

fn size(font: &str, requested: &str, replacement: &str) -> Diagnostic {
    warning(format!("LaTeX Font Warning: Font shape `{font}' in size <{requested}> not available (Font) size <{replacement}> substituted on input line 4."))
}

fn analyze(items: Vec<Diagnostic>) -> serde_json::Value {
    compilation_analysis(
        &[(
            Url::parse("file:///font-substitutions/main.tex").unwrap(),
            items,
        )]
        .into(),
        false,
    )
}

#[test]
fn font_analysis_separates_shape_and_size_substitutions_without_repeating_locations() {
    let main = Url::parse("file:///font-substitutions/main.tex").unwrap();
    let child = Url::parse("file:///font-substitutions/child.tex").unwrap();
    let items: BTreeMap<_, _> = [
        (
            main,
            vec![
                shape("OT1/missing/m/n", "OT1/cmr/m/n"),
                size("OT1/limited/m/n", "9.7", "10"),
            ],
        ),
        (
            child,
            vec![
                shape("OT1/missing/m/n", "OT1/cmr/m/n"),
                shape("TU/字体'/b/sc", "TU/lmr/b/n"),
                size("OT1/limited/m/n", "9.7", "10"),
            ],
        ),
    ]
    .into();
    let result = compilation_analysis(&items, false);
    assert_eq!(
        result["fontSubstitutions"],
        json!([
            {"requested":"OT1/missing/m/n", "replacement":"OT1/cmr/m/n"},
            {"requested":"TU/字体'/b/sc", "replacement":"TU/lmr/b/n"},
        ])
    );
    assert_eq!(
        result["fontSizeSubstitutions"],
        json!([
            {"font":"OT1/limited/m/n", "requestedSize":"9.7", "replacementSize":"10"},
        ])
    );
    assert_eq!(result["rerunRequired"], false);
    assert_eq!(result["truncated"], false);
}

#[test]
fn byte_wrapped_font_names_keep_their_identity_in_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "First\nSecond\nThird\n").unwrap();
    let requested = format!("OT1/missing{}/m/n", "x".repeat(80));
    let first = format!("LaTeX Font Warning: Font shape `{requested}' undefined");
    let wrapped = first
        .as_bytes()
        .chunks(79)
        .map(|part| std::str::from_utf8(part).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let log = format!("(./main.tex\n{wrapped}\n(Font)              using `OT1/cmr/m/n' instead on input line 3.\n\n)\n");
    let parsed = parse(&root, &log);
    let items = &parsed[&Url::from_file_path(&root).unwrap()];
    assert_eq!(items.len(), 1, "{parsed:?}");
    assert!(
        items[0].message.contains(&requested),
        "{}",
        items[0].message
    );
    assert_eq!(items[0].range.start.line, 2);
}

#[test]
fn incomplete_unrelated_or_non_warning_messages_do_not_invent_font_substitutions() {
    for message in [
        "LaTeX Font Warning: Some font shapes were not available, defaults substituted.",
        "LaTeX Font Warning: Size substitutions with differences (Font) up to 4.7pt have occurred.",
        "Package local Warning: Font shape `OT1/missing/m/n' undefined (Font) using `OT1/cmr/m/n' instead on input line 3.",
        "LaTeX Font Warning: Example: Font shape `OT1/missing/m/n' undefined (Font) using `OT1/cmr/m/n' instead on input line 3.",
        "LaTeX Font Warning: Font shape `OT1/missing/m/n' undefined",
        "LaTeX Font Warning: Font shape `OT1/missing/m/n' undefined (Font) using `OT1/cmr/m/n' instead on input line 3. Extra prose.",
        "LaTeX Font Warning: Font shape `OT1/missing/m/n' undefined (Font) using `OT1/cmr/m/n' instead on input line unknown.",
        "LaTeX Font Warning: Font shape `not-an-NFSS-identity' undefined (Font) using `OT1/cmr/m/n' instead on input line 3.",
        "LaTeX Font Warning: Font shape `OT1/cmr/m/n' in size <14..7> not available (Font) size <10> substituted on input line 3.",
    ] {
        let result = analyze(vec![warning(message.into())]);
        assert_eq!(result["fontSubstitutions"], json!([]), "{message}");
        assert_eq!(result["fontSizeSubstitutions"], json!([]), "{message}");
        assert_eq!(result["truncated"], false);
    }
    for severity in [
        None,
        Some(DiagnosticSeverity::ERROR),
        Some(DiagnosticSeverity::INFORMATION),
    ] {
        let mut item = shape("OT1/missing/m/n", "OT1/cmr/m/n");
        item.severity = severity;
        assert_eq!(analyze(vec![item])["fontSubstitutions"], json!([]));
    }
}

#[test]
fn font_summary_budgets_keep_complete_identities_and_independent_categories() {
    let mut items: Vec<_> = (0..128)
        .map(|index| shape(&format!("OT1/family{index:03}/m/n"), "OT1/cmr/m/n"))
        .collect();
    items.extend((0..128).map(|index| size(&format!("OT1/size{index:03}/m/n"), "14.7", "10")));
    items.push(shape("OT1/family000/m/n", "OT1/cmr/m/n"));
    items.push(size("OT1/size000/m/n", "14.7", "10"));
    let result = analyze(items.clone());
    assert_eq!(result["fontSubstitutions"].as_array().unwrap().len(), 128);
    assert_eq!(
        result["fontSizeSubstitutions"].as_array().unwrap().len(),
        128
    );
    assert_eq!(result["truncated"], false);
    items.push(shape("OT1/overflow/m/n", "OT1/cmr/m/n"));
    items.push(size("OT1/overflow/m/n", "14.7", "10"));
    let result = analyze(items);
    assert_eq!(result["fontSubstitutions"].as_array().unwrap().len(), 128);
    assert_eq!(
        result["fontSizeSubstitutions"].as_array().unwrap().len(),
        128
    );
    assert_eq!(result["truncated"], true);
    let accepted = format!("OT1/{}/m/n", "字".repeat(248));
    assert_eq!(accepted.chars().count(), 256);
    let rejected = format!("OT1/{}/m/n", "字".repeat(249));
    let result = analyze(vec![
        shape(&accepted, "OT1/cmr/m/n"),
        shape(&rejected, "OT1/cmr/m/n"),
        size(&rejected, "14.7", "10"),
        size("OT1/cmr/m/n", &"1".repeat(129), "10"),
    ]);
    assert_eq!(
        result["fontSubstitutions"],
        json!([{"requested":accepted,"replacement":"OT1/cmr/m/n"}])
    );
    assert_eq!(result["fontSizeSubstitutions"], json!([]));
    assert_eq!(result["truncated"], true);
    assert_eq!(
        compilation_analysis(&BTreeMap::new(), true)["truncated"],
        true
    );
}

fn wrap(value: &str, width: usize) -> String {
    let mut result = String::new();
    let mut rest = value;
    while rest.len() > width {
        let mut end = width;
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        result.push_str(&rest[..end]);
        result.push('\n');
        rest = &rest[end..];
    }
    result.push_str(rest);
    result
}

#[test]
fn shape_size_unicode_and_parenthesized_font_names_survive_warning_byte_wraps() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "First\nSecond\nThird\nFourth\n").unwrap();
    let requested = format!("OT1/{}(Font)'family/m/n", "字".repeat(32));
    let replacement = format!("OT1/{}(fallback)/m/n", "x".repeat(90));
    for width in [60, 64, 79, 90] {
        for (first, continuation, field, expected) in [
            (
                format!("LaTeX Font Warning: Font shape `{requested}' undefined"),
                format!("(Font)              using `{replacement}' instead on input line 3."),
                "fontSubstitutions",
                json!([{"requested":requested,"replacement":replacement}]),
            ),
            (
                format!(
                    "LaTeX Font Warning: Font shape `{requested}' in size <14.7> not available"
                ),
                "(Font)              size <10> substituted on input line 4.".into(),
                "fontSizeSubstitutions",
                json!([{"font":requested,"requestedSize":"14.7","replacementSize":"10"}]),
            ),
        ] {
            let log = format!(
                "(./main.tex\n{}\n{}\n\n! Later error.\nl.1 First\n)\n",
                wrap(&first, width),
                wrap(&continuation, width)
            );
            let parsed = parse(&root, &log);
            assert_eq!(
                compilation_analysis(&parsed, false)[field],
                expected,
                "{width}: {log}\n{parsed:?}"
            );
            assert!(parsed
                .values()
                .flatten()
                .any(|item| item.message == "Later error."));
        }
    }
}

#[test]
fn size_angle_bracket_at_start_of_wrapped_line_is_not_a_font_resource() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    std::fs::write(&root, "First\nSecond\nThird\nFourth\n").unwrap();
    let first = "LaTeX Font Warning: Font shape `OT1/lsplimited/m/n' in size <14.7> not available";
    let wrapped = wrap(first, 60);
    assert!(wrapped.contains("\n<14.7>"), "{wrapped}");
    let log = format!(
        "(./main.tex\n{wrapped}\n(Font)              size <10> substituted on input line 4.\n\n)\n"
    );
    let parsed = parse(&root, &log);
    assert_eq!(
        compilation_analysis(&parsed, false)["fontSizeSubstitutions"],
        json!([{"font":"OT1/lsplimited/m/n","requestedSize":"14.7","replacementSize":"10"}]),
        "{parsed:?}"
    );
}

#[test]
fn incomplete_font_warnings_preserve_following_files_and_numbered_errors() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("main.tex");
    let child = directory.path().join("child.tex");
    std::fs::write(&root, "Main\n").unwrap();
    std::fs::write(&child, "Child\n").unwrap();
    let partial = format!("LaTeX Font Warning: Font shape `OT1/{}", "x".repeat(80));
    for tail in [
        "(./child.tex\n! Child error.\nl.1 Child\n)\n",
        "./child.tex:1: Child error.\nl.1 Child\n",
    ] {
        let parsed = parse(&root, &format!("(./main.tex\n{partial}\n{tail})\n"));
        let child = &parsed[&Url::from_file_path(&child).unwrap()];
        assert_eq!(child.len(), 1, "{parsed:?}");
        assert_eq!(child[0].message, "Child error.");
        let result = compilation_analysis(&parsed, false);
        assert_eq!(result["fontSubstitutions"], json!([]));
        assert_eq!(result["fontSizeSubstitutions"], json!([]));
    }
}

#[test]
fn generated_file_line_explanations_do_not_change_font_substitution_identity() {
    let mut item = shape("OT1/missing/m/n", "OT1/cmr/m/n");
    item.message
        .push_str(" (reported line 900 is no longer present in the generated file)");
    item.data = Some(json!({"reportedLine":900,"unresolvedLine":true}));
    assert_eq!(
        analyze(vec![item])["fontSubstitutions"],
        json!([
            {"requested":"OT1/missing/m/n","replacement":"OT1/cmr/m/n"},
        ])
    );
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex, xelatex, lualatex and Computer Modern fonts"]
fn real_font_substitutions_preserve_names_sizes_and_clear_after_font_selection_is_repaired() {
    use std::os::unix::fs::PermissionsExt;
    for engine in ["pdflatex", "xelatex", "lualatex"] {
        for width in [60, 79] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let compiler = root.join(engine);
            std::fs::write(&compiler, format!("#!/bin/sh\nTEXMFVAR=\"${{0%/*}}/tex-cache\"\nTEXMFCACHE=\"$TEXMFVAR\"\nmax_print_line={width}\nexport TEXMFVAR TEXMFCACHE max_print_line\nmkdir -p \"$TEXMFVAR\"\nexec {engine} \"$@\"\n")).unwrap();
            std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
            let config = latex_lsp::config::BuildConfig {
                executable: compiler.to_string_lossy().into(),
                ..Default::default()
            };
            let family = format!("missingfamily{}", "x".repeat(65));
            let source = format!("\\documentclass{{article}}\n\\DeclareFontFamily{{OT1}}{{lsplimited}}{{}}\n\\DeclareFontShape{{OT1}}{{lsplimited}}{{m}}{{n}}{{<10> cmr10}}{{}}\n\\begin{{document}}\n{{\\fontencoding{{OT1}}\\fontfamily{{missingfont}}\\selectfont Missing family.}}\n{{\\fontencoding{{OT1}}\\fontfamily{{cmr}}\\fontseries{{bx}}\\fontshape{{sc}}\\selectfont Missing shape.}}\n{{\\fontencoding{{OT1}}\\fontfamily{{lsplimited}}\\fontsize{{14.7}}{{18}}\\selectfont Limited size.}}\n{{\\fontencoding{{OT1}}\\fontfamily{{{family}}}\\selectfont Long family.}}\n\\end{{document}}\n");
            std::fs::write(&main, &source).unwrap();
            let outcome = latex_lsp::compilation::build::run(&main, &config, &Default::default());
            assert_eq!(
                outcome.status, "success",
                "{engine}/{width}: {}",
                outcome.output
            );
            let analysis = outcome.analysis.unwrap();
            assert_eq!(
                analysis["fontSubstitutions"],
                json!([
                    {"requested":"OT1/cmr/bx/sc","replacement":"OT1/cmr/bx/n"},
                    {"requested":format!("OT1/{family}/m/n"),"replacement":"OT1/cmr/m/n"},
                    {"requested":"OT1/missingfont/m/n","replacement":"OT1/cmr/m/n"},
                ]),
                "{engine}/{width}: {:?}",
                outcome.diagnostics
            );
            assert_eq!(
                analysis["fontSizeSubstitutions"],
                json!([
                    {"font":"OT1/lsplimited/m/n","requestedSize":"14.7","replacementSize":"10"},
                ]),
                "{engine}/{width}: {:?}",
                outcome.diagnostics
            );
            assert_eq!(analysis["truncated"], false);
            let fixed = source
                .replace("missingfont", "cmr")
                .replace(&family, "cmr")
                .replace(r"\fontshape{sc}", r"\fontshape{n}")
                .replace(r"\fontsize{14.7}{18}", r"\fontsize{10}{18}");
            std::fs::write(&main, fixed).unwrap();
            let outcome = latex_lsp::compilation::build::run(&main, &config, &Default::default());
            assert_eq!(
                outcome.status, "success",
                "{engine}/{width}: {}",
                outcome.output
            );
            let analysis = outcome.analysis.unwrap();
            assert_eq!(analysis["fontSubstitutions"], json!([]), "{engine}/{width}");
            assert_eq!(
                analysis["fontSizeSubstitutions"],
                json!([]),
                "{engine}/{width}"
            );
        }
    }
}
