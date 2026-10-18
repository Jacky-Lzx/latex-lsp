//! Literal PDF destinations are independent of LaTeX reference labels.
use crate::{
    syntax::{self, Argument, Command, Entry},
    workspace::Document,
};
use std::ops::Range;
use tower_lsp::lsp_types::{Range as LspRange, TextEdit};

struct Destination {
    name: String,
    span: Range<usize>,
    category: Argument,
    key: Argument,
}

fn compound(source: &str, command: &Command) -> Option<Destination> {
    let mut at = syntax::skip_space(source, command.span.end);
    match command.name.as_str() {
        "hyperdef" => {
            if source.as_bytes().get(at) == Some(&b'[') {
                at = syntax::skip_space(source, syntax::group(source, at, b'[', b']')?.1);
            }
        }
        "hyperref" => {
            let url = command.argument.as_ref()?;
            if source.as_bytes().get(at) != Some(&b'{')
                || url.outer_span.start != at
                || !url.value.is_empty()
            {
                return None;
            }
            at = syntax::skip_space(source, url.outer_span.end);
        }
        _ => return None,
    }
    let (category, after) = syntax::group(source, at, b'{', b'}')?;
    let (key, _) = syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?;
    if [&category, &key]
        .iter()
        .any(|argument| !argument.value.is_empty() && !syntax::is_static_key(&argument.value))
    {
        return None;
    }
    let name = if category.value.is_empty() {
        key.value.clone()
    } else {
        format!("{}.{}", category.value, key.value)
    };
    if name.is_empty() {
        return None;
    }
    Some(Destination {
        name,
        span: category.span.start..key.span.end,
        category,
        key,
    })
}

pub(crate) fn entry(source: &str, command: &Command) -> Option<(bool, Entry)> {
    let declaration = matches!(command.name.as_str(), "hypertarget" | "hyperdef");
    let (name, span) = if matches!(command.name.as_str(), "hypertarget" | "hyperlink") {
        let (argument, _) = syntax::group(
            source,
            syntax::skip_space(source, command.span.end),
            b'{',
            b'}',
        )?;
        if !syntax::is_static_key(&argument.value) {
            return None;
        }
        (argument.value, argument.span)
    } else {
        let destination = compound(source, command)?;
        (destination.name, destination.span)
    };
    Some((
        declaration,
        Entry {
            name,
            span,
            detail: format!("PDF destination — \\{}", command.name),
            is_variable: false,
        },
    ))
}

/// Preserve the source separators and comments when a destination uses two fields.
pub(crate) fn rename_edits(doc: &Document, range: LspRange, new_name: &str) -> Vec<TextEdit> {
    for command in &doc.syntax.commands {
        let Some(destination) = compound(&doc.text.source, command) else {
            continue;
        };
        if doc.text.range(destination.span.start, destination.span.end) != range {
            continue;
        }
        let prefix = format!("{}.", destination.category.value);
        let (category, key) =
            if !destination.category.value.is_empty() && new_name.starts_with(&prefix) {
                (
                    destination.category.value.as_str(),
                    &new_name[prefix.len()..],
                )
            } else {
                ("", new_name)
            };
        return [(&destination.category, category), (&destination.key, key)]
            .into_iter()
            .filter(|(argument, value)| argument.value != *value)
            .map(|(argument, value)| TextEdit {
                range: doc.text.range(argument.span.start, argument.span.end),
                new_text: value.into(),
            })
            .collect();
    }
    vec![TextEdit {
        range,
        new_text: new_name.into(),
    }]
}
