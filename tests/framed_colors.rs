use lsp_types::*;
use texquill::{
    compilation::build,
    language::{colors, features, signature},
    project::workspace::{Document, Workspace},
    text::Text,
};

fn document(source: &str) -> Document {
    Document::new(
        &Url::parse("file:///framed-colors/main.tex").unwrap(),
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
fn apply(doc: &Document, presentation: ColorPresentation) -> String {
    let mut edits: Vec<_> = presentation
        .text_edit
        .into_iter()
        .chain(presentation.additional_text_edits.into_iter().flatten())
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
fn pick(doc: &Document, index: usize, value: Color) -> String {
    let found = colors::document_colors(doc);
    let mut presentations = colors::presentations(doc, found[index].range, value);
    assert_eq!(presentations.len(), 1);
    apply(doc, presentations.remove(0))
}

#[test]
fn shared_models_report_two_precise_colors_and_preserve_the_other_specification() {
    for (model, frame, background, expected_frame, expected_background) in [
        (
            "cmyk",
            "0,1,1,0",
            ".25,.5,.75,.125",
            color(1.0, 0.0, 0.0),
            color(0.625, 0.375, 0.125),
        ),
        (
            "rgb",
            "1,0,0",
            "0,0,1",
            color(1.0, 0.0, 0.0),
            color(0.0, 0.0, 1.0),
        ),
        (
            "RGB",
            "255,0,0",
            "0,0,255",
            color(1.0, 0.0, 0.0),
            color(0.0, 0.0, 1.0),
        ),
        (
            "HTML",
            "FF0000",
            "0000FF",
            color(1.0, 0.0, 0.0),
            color(0.0, 0.0, 1.0),
        ),
        (
            "gray",
            ".5",
            ".25",
            color(0.5, 0.5, 0.5),
            color(0.25, 0.25, 0.25),
        ),
    ] {
        let source = format!("% emoji😀\r\n\\fcolorbox[{model}]% frame\r\n{{ {frame} }}% background\r\n{{ {background} }}{{Body}}% tail\r\n");
        let doc = document(&source);
        let found = colors::document_colors(&doc);
        assert_eq!(found.len(), 2, "{source}");
        for (index, value, expected) in [
            (0, frame, expected_frame),
            (1, background, expected_background),
        ] {
            assert_eq!(found[index].color, expected);
            let at = source.find(value).unwrap();
            assert_eq!(found[index].range, doc.text.range(at, at + value.len()));
            let updated = pick(&doc, index, color(0.0, 1.0, 0.0));
            let updated_colors = colors::document_colors(&document(&updated));
            assert_eq!(updated_colors.len(), 2);
            assert_eq!(updated_colors[index].color, color(0.0, 1.0, 0.0));
            assert_eq!(updated_colors[1 - index].color, found[1 - index].color);
            assert!(updated.ends_with("{Body}% tail\r\n"));
            assert_eq!(updated.matches('%').count(), source.matches('%').count());
            assert_eq!(
                updated.matches("\r\n").count(),
                source.matches("\r\n").count()
            );
        }
    }
}

#[test]
fn explicit_background_models_are_independent_and_named_frames_do_not_hide_literal_backgrounds() {
    for (arguments, expected) in [
        (
            r"[rgb]{1,0,0}[RGB]{0,0,255}{Body}",
            vec![color(1.0, 0.0, 0.0), color(0.0, 0.0, 1.0)],
        ),
        (
            r"[gray]{.5}[HTML]{0000FF}{Body}",
            vec![color(0.5, 0.5, 0.5), color(0.0, 0.0, 1.0)],
        ),
        (r"{red}[rgb]{0,0,1}{Body}", vec![color(0.0, 0.0, 1.0)]),
        (
            r"[cmyk]{0,1,1,0}[rgb]{0,0,1}{Body}",
            vec![color(1.0, 0.0, 0.0), color(0.0, 0.0, 1.0)],
        ),
        (
            r"[cmy]{0,1,1}[rgb]{0,0,1}{Body}",
            vec![color(0.0, 0.0, 1.0)],
        ),
        (
            r"[rgb]{\frame,0,0}[HTML]{0000FF}{Body}",
            vec![color(0.0, 0.0, 1.0)],
        ),
        (
            r"[rgb]{1,0,0}[named]{blue}{Body}",
            vec![color(1.0, 0.0, 0.0)],
        ),
        (r"[rgb]{1,0,0}[]{blue}{Body}", vec![color(1.0, 0.0, 0.0)]),
    ] {
        let source = format!("\\fcolorbox{arguments}");
        let found = colors::document_colors(&document(&source));
        assert_eq!(
            found.iter().map(|item| item.color).collect::<Vec<_>>(),
            expected,
            "{source}"
        );
        for index in 0..found.len() {
            let updated = pick(&document(&source), index, color(0.0, 1.0, 0.0));
            let changed = colors::document_colors(&document(&updated));
            assert_eq!(changed[index].color, color(0.0, 1.0, 0.0));
            for other in 0..found.len() {
                if other != index {
                    assert_eq!(changed[other].color, found[other].color);
                }
            }
        }
    }
}

#[test]
fn shared_gray_conversions_insert_only_the_model_needed_to_preserve_the_other_color() {
    let source = "% emoji😀\r\n\\fcolorbox[gray]{ .5 }% keep ] {\r\n  { .25 }{Body}";
    let doc = document(source);
    assert_eq!(
        pick(&doc, 0, color(0.0, 1.0, 0.0)),
        "% emoji😀\r\n\\fcolorbox[rgb]{ 0,1,0 }% keep ] {\r\n  [gray]{ .25 }{Body}"
    );
    assert_eq!(
        pick(&doc, 1, color(0.0, 1.0, 0.0)),
        "% emoji😀\r\n\\fcolorbox[gray]{ .5 }% keep ] {\r\n  [rgb]{ 0,1,0 }{Body}"
    );
    let found = colors::document_colors(&doc);
    for index in 0..2 {
        let presentations =
            colors::presentations(&doc, found[index].range, color(0.75, 0.75, 0.75));
        assert!(presentations[0].additional_text_edits.is_none());
        let updated = apply(&doc, presentations.into_iter().next().unwrap());
        assert!(!updated.contains("[rgb]"));
        assert_eq!(
            colors::document_colors(&document(&updated))[1 - index].color,
            found[1 - index].color
        );
    }
    let first = document(&pick(&doc, 0, color(0.0, 1.0, 0.0)));
    let second = pick(&first, 1, color(0.0, 0.0, 1.0));
    assert!(second.contains("\\fcolorbox[rgb]{ 0,1,0 }"));
    assert!(second.contains("[rgb]{ 0,0,1 }{Body}"));
    let first = document(&pick(&doc, 1, color(0.0, 1.0, 0.0)));
    let second = pick(&first, 0, color(0.0, 0.0, 1.0));
    assert!(second.contains("\\fcolorbox[rgb]{ 0,0,1 }"));
    assert!(second.contains("[rgb]{ 0,1,0 }{Body}"));
}

#[test]
fn malformed_framed_calls_and_ignored_text_do_not_create_literal_colors() {
    for source in [
        r"\fcolorbox{red}{blue}{Body}",
        r"\fcolorbox*[rgb]{1,0,0}{0,0,1}{Body}",
        r"\fcolorbox[rgb][extra]{1,0,0}{0,0,1}{Body}",
        r"\fcolorbox[rgb]{1,0,0}[rgb][extra]{0,0,1}{Body}",
        r"\fcolorbox[rgb]{1,0,0}",
        r"\fcolorbox[rgb]{1,0,0}{0,0,1",
        r"\fcolorbox[\model]{1,0,0}{0,0,1}{Body}",
        r"\fcolorbox[rgb]{NaN,0,0}{0,0,inf}{Body}",
        r"\fcolorbox[gray]{1.1}{-.5}{Body}",
        r"\fcolorbox[RGB]{256,0,0}{0,0,256}{Body}",
        r"\fcolorbox[HTML]{FFF}{#0000FF}{Body}",
        r"% \fcolorbox[rgb]{1,0,0}{0,0,1}{Body}",
        r"\verb|\fcolorbox[rgb]{1,0,0}{0,0,1}{Body}|",
        "\\begin{verbatim}\n\\fcolorbox[rgb]{1,0,0}{0,0,1}{Body}\n\\end{verbatim}",
    ] {
        assert!(
            colors::document_colors(&document(source)).is_empty(),
            "{source}"
        );
    }
    let doc = document(r"\fcolorbox[gray]{.5}{.25}{Body}");
    assert!(colors::presentations(&doc, doc.text.range(0, 10), color(0.0, 1.0, 0.0)).is_empty());
    let uri = Url::parse("file:///framed-colors/main.bib").unwrap();
    assert!(colors::document_colors(&Document::new(&uri, doc.text.source, None)).is_empty());
}

#[test]
fn framed_box_completion_and_hover_preserve_existing_optional_models_and_body() {
    let uri = Url::parse("file:///framed-colors/main.tex").unwrap();
    let mut workspace = Workspace {
        client: texquill::project::state::ClientCapabilities {
            snippets: true,
            ..Default::default()
        },
        ..Workspace::default()
    };
    for (source, expected, snippet) in [
        (
            "😀 \\fcolorbo",
            "😀 \\fcolorbox{${1:frame color}}{${2:background color}}{${3:text}}",
            true,
        ),
        (
            "😀 \\fcolorboBAD[gray]{.5}[rgb]{0,0,1}{Body}",
            "😀 \\fcolorbox[gray]{.5}[rgb]{0,0,1}{Body}",
            false,
        ),
        (
            "😀 \\fcolorboBAD% keep\r\n{red}{blue}{Body}",
            "😀 \\fcolorbox% keep\r\n{red}{blue}{Body}",
            false,
        ),
    ] {
        workspace.open(uri.clone(), source.into(), 7);
        let offset = source.find("fcolorbo").unwrap() + "fcolorbo".len();
        let item =
            features::completion(&workspace, &uri, Text::new(source.into()).position(offset))
                .into_iter()
                .find(|item| item.label == "fcolorbox")
                .unwrap();
        assert_eq!(
            item.insert_text_format,
            Some(if snippet {
                InsertTextFormat::SNIPPET
            } else {
                InsertTextFormat::PLAIN_TEXT
            })
        );
        let Some(CompletionTextEdit::Edit(edit)) = item.text_edit else {
            panic!("missing edit")
        };
        let updated = Text::new(source.into())
            .changed(vec![TextDocumentContentChangeEvent {
                range: Some(edit.range),
                range_length: None,
                text: edit.new_text,
            }])
            .unwrap();
        assert_eq!(updated.source, expected);
    }
    workspace.open(uri.clone(), r"\fcolorbox{red}{blue}{Body}".into(), 7);
    let HoverContents::Markup(markup) = features::hover(&workspace, &uri, Position::new(0, 5))
        .unwrap()
        .contents
    else {
        panic!("missing docs")
    };
    assert!(markup.value.contains("background") && markup.value.contains("Requires"));
}

#[test]
fn framed_box_signatures_distinguish_both_models_colors_and_text_with_tex_bracket_rules() {
    let uri = Url::parse("file:///framed-colors/main.tex").unwrap();
    let mut workspace = Workspace::default();
    for (marked, active) in [
        (r"\fcolorbox[gr¦ay]{.5}[rgb]{0,0,1}{Body}", 0),
        (r"\fcolorbox[gray]{.¦5}[rgb]{0,0,1}{Body}", 1),
        (r"\fcolorbox[gray]{.5}[r¦gb]{0,0,1}{Body}", 2),
        (r"\fcolorbox[gray]{.5}[rgb]{0,0,¦1}{Body}", 3),
        (r"\fcolorbox[gray]{.5}[rgb]{0,0,1}{Bo¦dy}", 4),
        (r"\fcolorbox{re¦d}{blue}{Body}", 1),
        (r"\fcolorbox{red}{bl¦ue}{Body}", 3),
        (
            r"\fcolorbox[gray [literal]{.5}[rgb [literal]{0,0,1}{Bo¦dy}",
            4,
        ),
        (r"\fcolorbox[{gray]}]{.5}[{rgb]}]{0,0,1}{Bo¦dy}", 4),
        (
            "\\fcolorbox[gray% fake ]\r\n]{.5}% keep\r\n[rgb% fake ]\r\n]{0,0,1}{Bo¦dy}",
            4,
        ),
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(uri.clone(), source.clone(), 7);
        let help = signature::help(&workspace, &uri, Text::new(source).position(offset))
            .unwrap_or_else(|| panic!("{marked}"));
        assert_eq!(help.active_parameter, Some(active), "{marked}");
        assert_eq!(
            help.signatures[0].label,
            r"\fcolorbox[frame model]{frame color}[background model]{background color}{text}"
        );
    }
    for marked in [
        r"\fcolorbox*[rgb]{1,0,0}{0,0,¦1}{Body}",
        r"\fcolorbox[rgb][extra]{1,0,0}{0,0,¦1}{Body}",
        r"\fcolorbox[rgb]{1,0,0}[rgb][extra]{0,0,¦1}{Body}",
        r"\fcolorbox{red}{blue}{Body}After¦",
    ] {
        let offset = marked.find('¦').unwrap();
        let source = marked.replace('¦', "");
        workspace.open(uri.clone(), source.clone(), 7);
        assert!(
            signature::help(&workspace, &uri, Text::new(source).position(offset)).is_none(),
            "{marked}"
        );
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and xcolor"]
fn real_framed_gray_conversions_preserve_the_other_driver_color() {
    for engine in ["pdflatex", "xelatex"] {
        for first in 0..2 {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let main = root.join("main.tex");
            let source = r"\documentclass{article}\usepackage{xcolor}\makeatletter\let\savedsetcolor\set@color\def\set@color{\typeout{DRIVER:\current@color}\savedsetcolor}\makeatother\begin{document}\fcolorbox[gray]{.5}{.25}{\phantom{Paint}}\end{document}";
            let doc = document(source);
            let updated = pick(&doc, first, color(0.0, 1.0, 0.0));
            let final_source = pick(&document(&updated), 1 - first, color(0.0, 0.0, 1.0));
            for (index, source) in [updated, final_source].into_iter().enumerate() {
                std::fs::write(&main, source).unwrap();
                let config = texquill::config::BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                };
                let outcome = build::run(&main, &config, &Default::default());
                assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
                let green = if engine == "pdflatex" {
                    "DRIVER:0 1 0 rg 0 1 0 RG"
                } else {
                    "DRIVER:rgb 0 1 0"
                };
                assert!(outcome.output.contains(green), "{}", outcome.output);
                let other = if index == 1 {
                    if engine == "pdflatex" {
                        "DRIVER:0 0 1 rg 0 0 1 RG"
                    } else {
                        "DRIVER:rgb 0 0 1"
                    }
                } else {
                    match (engine, first) {
                        ("pdflatex", 0) => "DRIVER:.25 g .25 G",
                        ("pdflatex", _) => "DRIVER:.5 g .5 G",
                        (_, 0) => "DRIVER:gray .25",
                        _ => "DRIVER:gray .5",
                    }
                };
                assert!(outcome.output.contains(other), "{}", outcome.output);
            }
        }
    }
}

#[test]
#[ignore = "requires pdflatex, xelatex and xcolor"]
fn real_shared_cmyk_picker_edits_preserve_the_other_driver_separation() {
    for engine in ["pdflatex", "xelatex"] {
        for first in 0..2 {
            let directory = tempfile::tempdir().unwrap();
            let main = directory.path().join("main.tex");
            let source = r"\documentclass{article}\usepackage{xcolor}\makeatletter\let\savedsetcolor\set@color\def\set@color{\typeout{DRIVER:\current@color}\savedsetcolor}\makeatother\begin{document}\fcolorbox[cmyk]{.2,.4,.8,.1}{.25,.5,.75,.125}{Paint}\end{document}";
            let updated = pick(&document(source), first, color(0.0, 1.0, 0.0));
            let final_source = pick(&document(&updated), 1 - first, color(0.0, 0.0, 1.0));
            for (stage, source) in [updated, final_source].into_iter().enumerate() {
                assert!(source.contains("\\fcolorbox[cmyk]"));
                assert!(!source.contains("[rgb]"));
                std::fs::write(&main, source).unwrap();
                let config = texquill::config::BuildConfig {
                    executable: engine.into(),
                    ..Default::default()
                };
                let outcome = build::run(&main, &config, &Default::default());
                assert_eq!(outcome.status, "success", "{engine}: {}", outcome.output);
                let separations: Vec<Vec<f32>> = outcome
                    .output
                    .lines()
                    .filter_map(|line| line.strip_prefix("DRIVER:"))
                    .map(|line| {
                        line.split_whitespace()
                            .filter_map(|part| part.parse().ok())
                            .take(4)
                            .collect::<Vec<_>>()
                    })
                    .filter(|values| values.len() == 4)
                    .collect();
                assert!(
                    separations.contains(&vec![1.0, 0.0, 1.0, 0.0]),
                    "{engine}: {}",
                    outcome.output
                );
                let other = if stage == 1 {
                    vec![1.0, 1.0, 0.0, 0.0]
                } else if first == 0 {
                    vec![0.25, 0.5, 0.75, 0.125]
                } else {
                    vec![0.2, 0.4, 0.8, 0.1]
                };
                assert!(separations.contains(&other), "{engine}: {}", outcome.output);
            }
        }
    }
}
