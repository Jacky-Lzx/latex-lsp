use crate::{
    syntax::{self, Argument, Command},
    workspace::{Document, DocumentKind},
};
use std::ops::Range as Span;
use tower_lsp::lsp_types::*;

#[derive(Clone, Copy, PartialEq)]
enum Model {
    Rgb,
    ByteRgb,
    Html,
    Gray,
}

impl Model {
    fn name(self) -> &'static str {
        match self {
            Self::Rgb => "rgb",
            Self::ByteRgb => "RGB",
            Self::Html => "HTML",
            Self::Gray => "gray",
        }
    }
}

struct Literal {
    model: Model,
    // An empty span inserts an independent model before an inherited color.
    model_span: Span<usize>,
    inherited_background: Option<usize>,
    value_span: Span<usize>,
    color: Color,
}

fn trim(value: &str) -> &str {
    value.trim_matches([' ', '\t', '\r', '\n'])
}

fn fraction(value: &str) -> Option<f32> {
    let value = trim(value);
    // Rust also accepts exponents, NaN and infinity; TeX decimal literals do not.
    if !value.bytes().any(|ch| ch.is_ascii_digit())
        || !value
            .bytes()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, b'+' | b'-' | b'.'))
    {
        return None;
    }
    let value: f32 = value.parse().ok()?;
    (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
}

fn byte(value: &str) -> Option<f32> {
    let value = trim(value);
    if !value.bytes().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(f32::from(value.parse::<u8>().ok()?) / 255.0)
}

fn triple(value: &str, parse: fn(&str) -> Option<f32>) -> Option<[f32; 3]> {
    let mut values = value.split(',');
    let result = [
        parse(values.next()?)?,
        parse(values.next()?)?,
        parse(values.next()?)?,
    ];
    values.next().is_none().then_some(result)
}

fn literal(source: &str, command: &Command) -> Option<Literal> {
    let mut at = syntax::skip_space(source, command.span.end);
    let (model, after) = match command.name.as_str() {
        "definecolor" => {
            if source.as_bytes().get(at) == Some(&b'[') {
                let (kind, after) = syntax::bracket_argument(source, at)?;
                // PostScript definitions display the model's white value, not
                // the supplied components. Unknown types can use custom drivers.
                if !matches!(kind.value.as_str(), "" | "named") {
                    return None;
                }
                at = syntax::skip_space(source, after);
            }
            let (name, after) = syntax::group(source, at, b'{', b'}')?;
            if !syntax::is_static_key(&name.value) {
                return None;
            }
            at = syntax::skip_space(source, after);
            if source.as_bytes().get(at) == Some(&b'[') {
                let (prefix, after) = syntax::bracket_argument(source, at)?;
                if !prefix.value.is_empty() && !syntax::is_static_key(&prefix.value) {
                    return None;
                }
                at = syntax::skip_space(source, after);
            }
            syntax::group(source, at, b'{', b'}')?
        }
        "color" | "textcolor" | "colorbox" | "pagecolor" => syntax::bracket_argument(source, at)?,
        _ => return None,
    };
    let (argument, _) = syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?;
    literal_arguments(&model, &argument)
}

fn literal_arguments(model: &Argument, argument: &Argument) -> Option<Literal> {
    // Models are exact names. Do not expand macros, model lists or expressions.
    let model_kind = match model.value.as_str() {
        "rgb" => Model::Rgb,
        "RGB" => Model::ByteRgb,
        "HTML" => Model::Html,
        "gray" => Model::Gray,
        _ => return None,
    };
    let value = trim(&argument.value);
    let [red, green, blue] = match model_kind {
        Model::Rgb => triple(value, fraction)?,
        Model::ByteRgb => triple(value, byte)?,
        Model::Html => {
            if value.len() != 6 || !value.bytes().all(|ch| ch.is_ascii_hexdigit()) {
                return None;
            }
            let packed = u32::from_str_radix(value, 16).ok()?;
            [16, 8, 0].map(|shift| ((packed >> shift) & 255) as f32 / 255.0)
        }
        Model::Gray => [fraction(value)?; 3],
    };
    let start = argument.span.start + argument.value.len()
        - argument
            .value
            .trim_start_matches([' ', '\t', '\r', '\n'])
            .len();
    Some(Literal {
        model: model_kind,
        model_span: model.span.clone(),
        inherited_background: None,
        value_span: start..start + value.len(),
        color: Color {
            red,
            green,
            blue,
            alpha: 1.0,
        },
    })
}

fn framed_colors(source: &str, command: &Command) -> Option<[Option<Literal>; 2]> {
    let mut at = syntax::skip_space(source, command.span.end);
    let frame_model = if source.as_bytes().get(at) == Some(&b'[') {
        let (model, after) = syntax::bracket_argument(source, at)?;
        at = syntax::skip_space(source, after);
        Some(model)
    } else {
        None
    };
    let (frame, after) = syntax::group(source, at, b'{', b'}')?;
    at = syntax::skip_space(source, after);
    let background_model = if source.as_bytes().get(at) == Some(&b'[') {
        let (model, after) = syntax::bracket_argument(source, at)?;
        at = syntax::skip_space(source, after);
        Some(model)
    } else {
        None
    };
    let (background, _) = syntax::group(source, at, b'{', b'}')?;
    let mut frame_color = frame_model
        .as_ref()
        .and_then(|model| literal_arguments(model, &frame));
    let mut background_color = background_model
        .as_ref()
        .or(frame_model.as_ref())
        .and_then(|model| literal_arguments(model, &background));
    if background_model.is_none() {
        // The background inherits the frame model. Either picker must leave
        // the other color unchanged when converting gray to rgb.
        if let Some(frame) = &mut frame_color {
            frame.inherited_background = Some(background.outer_span.start);
        }
        if let Some(color) = &mut background_color {
            color.model_span = background.outer_span.start..background.outer_span.start;
        }
    }
    Some([frame_color, background_color])
}

fn literals(doc: &Document) -> impl Iterator<Item = Literal> + '_ {
    doc.syntax
        .commands
        .iter()
        .filter(|_| doc.kind == DocumentKind::Latex)
        .flat_map(|command| {
            if command.name == "fcolorbox" {
                framed_colors(&doc.text.source, command).unwrap_or_default()
            } else {
                [literal(&doc.text.source, command), None]
            }
        })
        .flatten()
}

pub fn document_colors(doc: &Document) -> Vec<ColorInformation> {
    literals(doc)
        .map(|literal| ColorInformation {
            range: doc
                .text
                .range(literal.value_span.start, literal.value_span.end),
            color: literal.color,
        })
        .collect()
}

fn decimal(value: f32) -> String {
    let rendered = format!("{value:.6}");
    let rendered = rendered.trim_end_matches('0').trim_end_matches('.');
    if rendered == "-0" {
        "0".into()
    } else {
        rendered.into()
    }
}

pub fn presentations(doc: &Document, range: Range, color: Color) -> Vec<ColorPresentation> {
    let channels = [color.red, color.green, color.blue];
    if color.alpha != 1.0
        || channels
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return vec![];
    }
    let Some(literal) = literals(doc).find(|literal| {
        doc.text
            .range(literal.value_span.start, literal.value_span.end)
            == range
    }) else {
        return vec![];
    };
    let model = if literal.model == Model::Gray
        && (color.red != color.green || color.green != color.blue)
    {
        Model::Rgb
    } else {
        literal.model
    };
    let value = match model {
        Model::Rgb => channels.map(decimal).join(","),
        Model::ByteRgb => channels
            .map(|value| ((value * 255.0).round() as u8).to_string())
            .join(","),
        Model::Html => channels
            .map(|value| format!("{:02X}", (value * 255.0).round() as u8))
            .join(""),
        Model::Gray => decimal(color.red),
    };
    vec![ColorPresentation {
        label: format!("{} {{{value}}}", model.name()),
        text_edit: Some(TextEdit {
            range,
            new_text: value,
        }),
        additional_text_edits: (model != literal.model).then(|| {
            let mut edits = vec![TextEdit {
                range: doc
                    .text
                    .range(literal.model_span.start, literal.model_span.end),
                new_text: if literal.model_span.is_empty() {
                    format!("[{}]", model.name())
                } else {
                    model.name().into()
                },
            }];
            if let Some(at) = literal.inherited_background {
                edits.push(TextEdit {
                    range: doc.text.range(at, at),
                    new_text: format!("[{}]", literal.model.name()),
                });
            }
            edits
        }),
    }]
}
