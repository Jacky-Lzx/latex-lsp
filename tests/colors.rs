use lsp_types::*;
use texquill::{compilation::build, language::colors, project::workspace::Document};

fn document(source: &str) -> Document {
    Document::new(
        &Url::parse("file:///colors/main.tex").unwrap(),
        source.into(),
        Some(7),
    )
}

fn color(red: f32, green: f32, blue: f32) -> Color {
    Color {
        red,
        green,
        blue,
        alpha: 1.0,
    }
}

fn apply(doc: &Document, presentations: Vec<ColorPresentation>) -> String {
    let mut edits: Vec<_> = presentations
        .into_iter()
        .flat_map(|presentation| {
            presentation
                .text_edit
                .into_iter()
                .chain(presentation.additional_text_edits.into_iter().flatten())
        })
        .collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    assert!(edits
        .windows(2)
        .all(|pair| pair[1].range.end <= pair[0].range.start));
    doc.text
        .changed(
            edits
                .into_iter()
                .map(|edit| TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                })
                .collect(),
        )
        .unwrap()
        .source
}

#[test]
fn literal_models_in_all_supported_commands_have_precise_utf16_ranges() {
    for command in ["definecolor", "color", "textcolor", "colorbox", "pagecolor"] {
        for (model, value, expected) in [
            ("rgb", "+.25, .5, 1.", color(0.25, 0.5, 1.0)),
            ("RGB", "0, 128, 255", color(0.0, 128.0 / 255.0, 1.0)),
            ("HTML", "aB10ff", color(171.0 / 255.0, 16.0 / 255.0, 1.0)),
            ("gray", ".5", color(0.5, 0.5, 0.5)),
            ("cmyk", ".25,.5,.75,.125", color(0.625, 0.375, 0.125)),
        ] {
            let model = if command == "definecolor" {
                format!("{{paint}}% keep😀\r\n{{{model}}}")
            } else {
                format!("[{model}]")
            };
            let source = format!(
                "% emoji😀\r\n\\{command}% before model\r\n{model}% before spec\r\n{{  {value} \t}}% keep\r\n{{Body}}"
            );
            let doc = document(&source);
            let found = colors::document_colors(&doc);
            assert_eq!(found.len(), 1, "{source}");
            assert_eq!(found[0].color, expected, "{source}");
            let start = source.find(value).unwrap();
            assert_eq!(found[0].range, doc.text.range(start, start + value.len()));
        }
    }
}

#[test]
fn comments_urls_verbatim_and_non_latex_documents_do_not_create_color_ranges() {
    let source = "% \\color[rgb]{1,0,0}\r\n\\verb|\\color[gray]{.5}|\n\\begin{verbatim}\n\\definecolor{paint}{HTML}{FF0000}\n\\end{verbatim}\n\\url{https://example.com/\\color[rgb]{1,0,0}}\n";
    assert!(colors::document_colors(&document(source)).is_empty());
    for extension in ["bib", "bst"] {
        let doc = Document::new(
            &Url::parse(&format!("file:///colors/main.{extension}")).unwrap(),
            r"\color[rgb]{1,0,0}".into(),
            Some(7),
        );
        assert!(colors::document_colors(&doc).is_empty());
    }
}

#[test]
fn malformed_dynamic_named_and_unsupported_colors_are_not_editable_literals() {
    for source in [
        r"\color{red}",
        r"\color[rgb]{red!50!blue}",
        r"\color[cmy]{0,1,1}",
        r"\color[rGb]{1,0,0}",
        r"\color[rgb]{1,0}",
        r"\color[rgb]{1,0,0,0}",
        r"\color[rgb]{1,,0}",
        r"\color[rgb]{NaN,0,0}",
        r"\color[rgb]{inf,0,0}",
        r"\color[rgb]{1e0,0,0}",
        r"\color[rgb]{-0.1,0,0}",
        r"\color[rgb]{1.1,0,0}",
        r"\color[rgb]{\redpart,0,0}",
        r"\color[rgb]{{1},0,0}",
        "\\color[rgb]{1,% note\n0,0}",
        r"\color[rgb]{1,0,0",
        r"\color[rgb][gray]{1,0,0}",
        r"\color*[rgb]{1,0,0}",
        r"\color[gray]{}",
        r"\color[gray]{.5,.5}",
        r"\color[RGB]{256,0,0}",
        r"\color[RGB]{255.0,0,0}",
        r"\color[HTML]{#FF0000}",
        r"\color[HTML]{FFF}",
        r"\color[HTML]{FF00GG}",
        r"\definecolor[ps]{paint}{rgb}{1,0,0}",
        r"\definecolor{\paint}{rgb}{1,0,0}",
        r"\definecolor{paint}{rgb/HTML}{1,0,0/FF0000}",
        r"\definecolor*{paint}{rgb}{1,0,0}",
    ] {
        assert!(
            colors::document_colors(&document(source)).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn definition_options_keep_exact_color_ranges_and_survive_picker_edits() {
    for kind in ["", "[]", "[named]"] {
        for prefix in ["", "[]", "[prefix]", "[prefix[literal]"] {
            for (model, value, expected) in [
                ("rgb", "1,0,0", color(1.0, 0.0, 0.0)),
                ("RGB", "255,0,0", color(1.0, 0.0, 0.0)),
                ("HTML", "FF0000", color(1.0, 0.0, 0.0)),
                ("gray", ".5", color(0.5, 0.5, 0.5)),
                ("cmyk", "0,1,1,0", color(1.0, 0.0, 0.0)),
            ] {
                let source = format!("% emoji😀\r\n\\definecolor{kind}% before name\r\n{{paint}}% before prefix\r\n{prefix}% before model\r\n{{{model}}}% before value\r\n{{ {value} }}% tail\r\n\\textcolor{{paint}}{{Body}}\r\n");
                let doc = document(&source);
                let found = colors::document_colors(&doc);
                assert_eq!(found.len(), 1, "{source}");
                assert_eq!(found[0].color, expected);
                let start = source.find(value).unwrap();
                assert_eq!(found[0].range, doc.text.range(start, start + value.len()));
                let presentations =
                    colors::presentations(&doc, found[0].range, color(0.0, 1.0, 0.0));
                assert_eq!(presentations.len(), 1);
                let replacement = match model {
                    "RGB" => "0,255,0",
                    "HTML" => "00FF00",
                    "cmyk" => "1,0,1,0",
                    _ => "0,1,0",
                };
                let expected = source
                    .replace(value, replacement)
                    .replace("{gray}", "{rgb}");
                let updated = apply(&doc, presentations);
                assert_eq!(updated, expected);
                assert_eq!(
                    colors::document_colors(&document(&updated))[0].color,
                    color(0.0, 1.0, 0.0)
                );
            }
        }
    }
}

#[test]
fn unknown_dynamic_or_malformed_definition_options_do_not_offer_picker_edits() {
    for arguments in [
        r"[ps]{paint}{rgb}{1,0,0}",
        r"[custom]{paint}{rgb}{1,0,0}",
        r"[ named ]{paint}{rgb}{1,0,0}",
        r"[\kind]{paint}{rgb}{1,0,0}",
        r"[named][extra]{paint}{rgb}{1,0,0}",
        r"[named{paint}{rgb}{1,0,0}",
        r"{paint}[prefix{rgb}{1,0,0}",
        r"{paint}[\prefix]{rgb}{1,0,0}",
        r"{paint}[{prefix]}]{rgb}{1,0,0}",
        r"{paint}[prefix][extra]{rgb}{1,0,0}",
        r"[named]{\paint}[prefix]{rgb}{1,0,0}",
        r"[named]{paint}[prefix]{rgb}{\component,0,0}",
        r"[named]{paint}[prefix]{rgb}{1,0,0",
        "[named% uncertain\n]{paint}{rgb}{1,0,0}",
        "{paint}[prefix% uncertain\n]{rgb}{1,0,0}",
    ] {
        let source = format!("\\definecolor{arguments}");
        let doc = document(&source);
        assert!(colors::document_colors(&doc).is_empty(), "{source}");
        let at = source.find("1,0,0").unwrap_or(0);
        assert!(colors::presentations(
            &doc,
            doc.text.range(at, (at + 5).min(source.len())),
            color(0.0, 1.0, 0.0)
        )
        .is_empty());
    }
}

#[test]
fn picker_edits_preserve_model_syntax_whitespace_comments_names_and_body() {
    for command in ["definecolor", "color", "textcolor", "colorbox", "pagecolor"] {
        for (model, original, replacement) in [
            ("rgb", "1,0,0", "0,1,0"),
            ("RGB", "255,0,0", "0,255,0"),
            ("HTML", "FF0000", "00FF00"),
            ("gray", ".5", "0,1,0"),
            ("cmyk", "0,1,1,0", "1,0,1,0"),
        ] {
            let model_argument = if command == "definecolor" {
                format!("{{paint}}{{{model}}}")
            } else {
                format!("[{model}]")
            };
            let source = format!("% emoji😀\r\n\\{command}{model_argument}% keep\r\n{{ {original} }}{{Body}}% tail\r\n");
            let doc = document(&source);
            let found = colors::document_colors(&doc);
            let presentations = colors::presentations(&doc, found[0].range, color(0.0, 1.0, 0.0));
            assert_eq!(presentations.len(), 1);
            assert_eq!(
                presentations[0].text_edit.as_ref().unwrap().range,
                found[0].range
            );
            assert_eq!(
                presentations[0]
                    .additional_text_edits
                    .as_ref()
                    .map_or(0, Vec::len),
                usize::from(model == "gray")
            );
            let updated = apply(&doc, presentations);
            let mut expected = source.replace(original, replacement);
            if model == "gray" {
                expected = expected.replace("gray", "rgb");
            }
            assert_eq!(updated, expected);
            assert_eq!(
                colors::document_colors(&document(&updated))[0].color,
                color(0.0, 1.0, 0.0)
            );
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and xcolor"]
fn optional_color_definitions_keep_real_components_after_picker_edits() {
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let mut source = String::from("\\documentclass{article}\n\\usepackage{xcolor}\n");
        let mut names = Vec::new();
        for kind in ["", "[]", "[named]"] {
            for prefix in ["", "[]", "[prefix]", "[prefix[literal]"] {
                for (model, value) in [
                    ("rgb", "1,0,0"),
                    ("RGB", "255,0,0"),
                    ("HTML", "FF0000"),
                    ("gray", ".5"),
                ] {
                    let name = format!("paint{}", names.len());
                    source.push_str(&format!(
                        "\\definecolor{kind}{{{name}}}{prefix}{{{model}}}{{{value}}}\n"
                    ));
                    names.push(name);
                }
            }
        }
        source.push_str("\\definecolor[ps]{postscript}[pre]{rgb}{1,0,0}\n\\begin{document}\n");
        for name in &names {
            source.push_str(&format!("\\textcolor{{{name}}}{{Paint}}\\extractcolorspec{{{name}}}\\tone\\typeout{{COLOR:{name}:\\meaning\\tone}}\n"));
        }
        source.push_str("\\extractcolorspec{postscript}\\tone\\typeout{POSTSCRIPT:\\meaning\\tone}\n\\end{document}\n");
        let doc = document(&source);
        let found = colors::document_colors(&doc);
        assert_eq!(found.len(), names.len());
        let presentations = found
            .iter()
            .flat_map(|item| colors::presentations(&doc, item.range, color(0.0, 1.0, 0.0)))
            .collect();
        for (index, source) in [source, apply(&doc, presentations)].into_iter().enumerate() {
            std::fs::write(&main, source).unwrap();
            let config = texquill::config::BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
            assert!(outcome.output.contains("POSTSCRIPT:macro:->{rgb}{1,1,1}"));
            if index == 1 {
                for name in &names {
                    assert!(
                        outcome
                            .output
                            .contains(&format!("COLOR:{name}:macro:->{{rgb}}{{0,1,0}}")),
                        "{engine}: {}",
                        outcome.output
                    );
                }
            }
        }
    }
}

#[test]
fn grayscale_remains_gray_and_decimal_edits_do_not_use_tex_incompatible_exponents() {
    let doc = document(r"\definecolor{paint}{gray}{.5}");
    let range = colors::document_colors(&doc)[0].range;
    let presentations = colors::presentations(&doc, range, color(0.25, 0.25, 0.25));
    assert!(presentations[0].additional_text_edits.is_none());
    assert_eq!(
        apply(&doc, presentations),
        r"\definecolor{paint}{gray}{0.25}"
    );
    let doc = document(r"\color[rgb]{1,0,0}");
    let range = colors::document_colors(&doc)[0].range;
    let updated = apply(
        &doc,
        colors::presentations(&doc, range, color(0.000001, 0.1234567, -0.0)),
    );
    assert_eq!(updated, r"\color[rgb]{0.000001,0.123457,0}");
}

#[test]
fn presentations_reject_unrelated_partial_and_invalid_ranges_and_unsupported_channels() {
    let doc = document("% emoji😀\r\n\\textcolor[rgb]{1,0,0}{Body}");
    let range = colors::document_colors(&doc)[0].range;
    let mut partial = range;
    partial.start.character += 1;
    for range in [
        Range::default(),
        partial,
        Range::new(range.start, range.start),
        Range::new(range.end, range.start),
        Range::new(Position::new(99, 0), Position::new(99, 2)),
    ] {
        assert!(colors::presentations(&doc, range, color(0.0, 1.0, 0.0)).is_empty());
    }
    for invalid in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        for component in 0..4 {
            let mut values = [0.0, 1.0, 0.0, 1.0];
            values[component] = invalid;
            let [red, green, blue, alpha] = values;
            assert!(colors::presentations(
                &doc,
                range,
                Color {
                    red,
                    green,
                    blue,
                    alpha,
                }
            )
            .is_empty());
        }
    }
    let mut transparent = color(0.0, 1.0, 0.0);
    transparent.alpha = 0.5;
    assert!(colors::presentations(&doc, range, transparent).is_empty());
}

#[test]
fn byte_and_html_presentations_round_components_to_the_nearest_byte() {
    for (source, expected) in [
        (r"\color[RGB]{0,0,0}", r"\color[RGB]{26,128,255}"),
        (r"\color[HTML]{000000}", r"\color[HTML]{1A80FF}"),
    ] {
        let doc = document(source);
        let range = colors::document_colors(&doc)[0].range;
        assert_eq!(
            apply(
                &doc,
                colors::presentations(&doc, range, color(0.1, 0.5, 1.0))
            ),
            expected
        );
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and xcolor"]
fn applied_picker_edits_remain_compilable_with_pdftex_and_xetex() {
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = "\\documentclass{article}\r\n\\usepackage{xcolor}\r\n% emoji😀\r\n\\definecolor{paint}{HTML}{FF0000}\r\n\\begin{document}\r\n\\textcolor{paint}{Paint}\r\n\\textcolor[RGB]{255,0,0}{Text}\r\n\\begingroup\\color[rgb]{1,0,0}Color\\endgroup\r\n\\colorbox[gray]{.5}{Box}\r\n\\pagecolor[rgb]{1,0,0}\r\n\\end{document}\r\n";
        let main = root.join("main.tex");
        let doc = document(source);
        let mut presentations = Vec::new();
        let found = colors::document_colors(&doc);
        assert_eq!(found.len(), 5);
        for found in found {
            presentations.extend(colors::presentations(
                &doc,
                found.range,
                color(0.0, 1.0, 0.0),
            ));
        }
        for source in [source.into(), apply(&doc, presentations)] {
            std::fs::write(&main, &source).unwrap();
            let config = texquill::config::BuildConfig {
                executable: engine.into(),
                ..Default::default()
            };
            let outcome = build::run(&main, &config, &Default::default());
            assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
        }
    }
}

#[test]
fn cmyk_previews_use_xcolor_black_addition_and_clip_saturated_channels() {
    for (value, expected) in [
        ("0,0,0,0", color(1.0, 1.0, 1.0)),
        ("0,0,0,1", color(0.0, 0.0, 0.0)),
        ("1,1,1,1", color(0.0, 0.0, 0.0)),
        (".25,.5,.75,.125", color(0.625, 0.375, 0.125)),
        ("1,0,.25,.5", color(0.0, 0.5, 0.25)),
        ("+.5, -0, .25, 0.", color(0.5, 1.0, 0.75)),
    ] {
        let doc = document(&format!("\\textcolor[cmyk]{{{value}}}{{Paint}}"));
        let found = colors::document_colors(&doc);
        assert_eq!(found.len(), 1, "{value}");
        assert_eq!(found[0].color, expected, "{value}");
    }
}

#[test]
fn malformed_cmyk_components_never_offer_color_ranges_or_picker_edits() {
    for value in [
        "",
        "0,1,1",
        "0,1,1,0,0",
        "0,,1,0",
        "0,1,1,",
        "0,1,1,NaN",
        "0,1,1,inf",
        "0,1,1,1e0",
        "0,1,1,-.1",
        "0,1,1,1.1",
        "-.1,1,1,0",
        "0,1.1,1,0",
        "0,1,-1,0",
        "0,1,1,\\blackpart",
        "{0},1,1,0",
        "0,1,% comment\n1,0",
        "0,1,1,0!50!white",
    ] {
        let source = format!("\\color[cmyk]{{{value}}}");
        let doc = document(&source);
        assert!(colors::document_colors(&doc).is_empty(), "{value}");
        let start = source.find('{').unwrap() + 1;
        assert!(
            colors::presentations(
                &doc,
                doc.text.range(start, start + value.len()),
                color(0.0, 1.0, 0.0)
            )
            .is_empty(),
            "{value}"
        );
    }
}

#[test]
fn unchanged_cmyk_picker_requests_preserve_black_separation_and_clipped_components() {
    for (value, normalized) in [
        (".2, .4, .8, .1", "0.2,0.4,0.8,0.1"),
        ("1,1,1,1", "1,1,1,1"),
        ("0,0,0,.5", "0,0,0,0.5"),
    ] {
        let source = format!("% 😀\r\n\\textcolor[cmyk]% keep\r\n{{ {value} }}{{Paint}}\r\n");
        let doc = document(&source);
        let original = colors::document_colors(&doc)[0].clone();
        let presentations = colors::presentations(&doc, original.range, original.color);
        assert_eq!(presentations.len(), 1);
        assert!(presentations[0].additional_text_edits.is_none());
        assert_eq!(presentations[0].label, format!("cmyk {{{normalized}}}"));
        assert_eq!(
            apply(&doc, presentations),
            source.replace(value, normalized)
        );
    }
}

#[test]
fn cmyk_picker_edits_round_trip_rgb_without_changing_the_color_model() {
    let doc = document(r"\definecolor{paint}{cmyk}{0,1,1,0}");
    let range = colors::document_colors(&doc)[0].range;
    for red in [0.0, 0.125, 0.5, 0.875, 1.0] {
        for green in [0.0, 0.125, 0.5, 0.875, 1.0] {
            for blue in [0.0, 0.125, 0.5, 0.875, 1.0] {
                let requested = color(red, green, blue);
                let presentations = colors::presentations(&doc, range, requested);
                assert_eq!(presentations.len(), 1);
                assert!(presentations[0].additional_text_edits.is_none());
                let updated = apply(&doc, presentations);
                assert!(updated.contains("{cmyk}"));
                let actual = colors::document_colors(&document(&updated))[0].color;
                for (actual, expected) in [
                    (actual.red, red),
                    (actual.green, green),
                    (actual.blue, blue),
                ] {
                    assert!((actual - expected).abs() < 0.000002, "{updated}");
                }
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and xcolor"]
fn cmyk_preview_and_picker_values_match_real_xcolor_conversions() {
    for engine in ["pdflatex", "xelatex"] {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        for value in [".2,.4,.8,.1", "1,0,.25,.5", "1,1,1,1", "0,0,0,0"] {
            let source = format!("\\documentclass{{article}}\n\\usepackage{{xcolor}}\n\\definecolor[named]{{paint}}[pre]{{cmyk}}{{{value}}}\n\\begin{{document}}\n\\textcolor{{paint}}{{Paint}}\n\\extractcolorspec{{paint}}\\extracted\n\\expandafter\\convertcolorspec\\extracted{{rgb}}\\converted\n\\typeout{{CMYKRGB:\\converted}}\n\\end{{document}}\n");
            let doc = document(&source);
            let found = colors::document_colors(&doc)[0].clone();
            for requested in [found.color, color(0.25, 0.5, 0.75), color(0.0, 0.0, 0.0)] {
                let updated = apply(&doc, colors::presentations(&doc, found.range, requested));
                std::fs::write(&main, &updated).unwrap();
                let config = texquill::config::BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                };
                let outcome = build::run(&main, &config, &Default::default());
                assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
                let actual: Vec<f32> = outcome
                    .output
                    .lines()
                    .find_map(|line| line.strip_prefix("CMYKRGB:"))
                    .unwrap()
                    .split(',')
                    .map(|value| value.parse().unwrap())
                    .collect();
                assert_eq!(actual.len(), 3);
                for (actual, expected) in
                    actual
                        .into_iter()
                        .zip([requested.red, requested.green, requested.blue])
                {
                    assert!(
                        (actual - expected).abs() < 0.00005,
                        "{engine} {value} => {updated}\n{}",
                        outcome.output
                    );
                }
            }
        }
    }
}
