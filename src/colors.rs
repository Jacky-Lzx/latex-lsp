use crate::{
    syntax::{self, Command},
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
    model_span: Span<usize>,
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
    let at = syntax::skip_space(source, command.span.end);
    let (model, after) = match command.name.as_str() {
        "definecolor" => {
            let (name, after) = syntax::group(source, at, b'{', b'}')?;
            if !syntax::is_static_key(&name.value) {
                return None;
            }
            syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?
        }
        "color" | "textcolor" | "colorbox" | "pagecolor" => syntax::bracket_argument(source, at)?,
        _ => return None,
    };
    // Models are exact names. Do not expand macros, model lists or expressions.
    let model_kind = match model.value.as_str() {
        "rgb" => Model::Rgb,
        "RGB" => Model::ByteRgb,
        "HTML" => Model::Html,
        "gray" => Model::Gray,
        _ => return None,
    };
    let (argument, _) = syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?;
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
        model_span: model.span,
        value_span: start..start + value.len(),
        color: Color {
            red,
            green,
            blue,
            alpha: 1.0,
        },
    })
}

fn literals(doc: &Document) -> impl Iterator<Item = Literal> + '_ {
    doc.syntax
        .commands
        .iter()
        .filter(|_| doc.kind == DocumentKind::Latex)
        .filter_map(|command| literal(&doc.text.source, command))
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
            vec![TextEdit {
                range: doc
                    .text
                    .range(literal.model_span.start, literal.model_span.end),
                new_text: model.name().into(),
            }]
        }),
    }]
}
