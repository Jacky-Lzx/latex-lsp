//! FancyVerb's literal filenames and conservative formatting settings.
use crate::{syntax, text::Text};
use std::ops::Range;
use tower_lsp::lsp_types::TextEdit;

pub(crate) fn is_input(name: &str) -> bool {
    matches!(name, "VerbatimInput" | "LVerbatimInput" | "BVerbatimInput")
}

pub(crate) fn is_environment(name: &str) -> bool {
    matches!(
        name.strip_suffix('*').unwrap_or(name),
        "Verbatim" | "LVerbatim" | "BVerbatim" | "SaveVerbatim" | "VerbatimOut"
    )
}

/// FancyVerb checks the first literal end marker on each line. A different
/// environment there makes the complete line data, including later markers.
pub(crate) fn environment_end(source: &str, body_start: usize, name: &str) -> Option<usize> {
    let ending = format!("\\end{{{name}}}");
    let mut offset = body_start;
    for line in source[body_start..].split_inclusive('\n') {
        if let Some(start) = line.find("\\end{") {
            if line[start..].starts_with(&ending) {
                return Some(offset + start + ending.len());
            }
        }
        offset += line.len();
    }
    None
}

fn safe_options(source: &str) -> bool {
    if source.contains(['\\', '%']) {
        return false;
    }
    syntax::key_list_segments(source, 0..source.len())
        .into_iter()
        .all(|span| {
            let value = source[span].trim();
            let (key, argument) = value.split_once('=').unwrap_or((value, ""));
            match key.trim() {
                "commandchars" => argument.trim() == "none",
                "" | "firstline" | "lastline" | "frame" | "framerule" | "framesep" | "label"
                | "labelposition" | "numbers" | "numbersep" | "firstnumber" | "stepnumber"
                | "numberblanklines" | "showspaces" | "showtabs" | "tabsize" | "obeytabs"
                | "fontsize" | "fontfamily" | "fontshape" | "fontseries" | "baselinestretch"
                | "xleftmargin" | "xrightmargin" | "resetmargins" | "samepage" | "gobble" => true,
                _ => false,
            }
        })
}

fn internal_command(name: &str) -> bool {
    name.starts_with("FV@") || name.starts_with("FVC@") || name.starts_with("KV@FV@")
}

pub(crate) fn safe_syntax(source: &str, parsed: &syntax::Syntax) -> bool {
    !parsed.definitions.iter().any(|definition| {
        is_input(&definition.name)
            || internal_command(&definition.name)
            || matches!(
                definition.name.as_str(),
                "fvset" | "detokenize" | "setkeys" | "define@key"
            )
    }) && parsed
        .commands
        .iter()
        .all(|command| match command.name.as_str() {
            name if internal_command(name) => false,
            "CustomVerbatimCommand" | "RecustomVerbatimCommand" => false,
            // Direct keyval calls can change execution settings or redefine
            // the meaning of an otherwise harmless formatting key.
            "setkeys" | "define@key" => command.argument.as_ref().is_some_and(|argument| {
                argument.value.split(',').all(|family| {
                    let family = family.trim();
                    syntax::is_static_key(family) && family != "FV"
                })
            }),
            "fvset" => command.argument.as_ref().is_some_and(|argument| {
                argument.outer_span.start == syntax::skip_space(source, command.span.end)
                    && safe_options(&argument.value)
            }),
            _ => true,
        })
}

pub(crate) fn safe_call(source: &str, command: &syntax::Command) -> bool {
    let mut start = syntax::skip_space(source, command.span.end);
    if source.as_bytes().get(start) == Some(&b'*') {
        start = syntax::skip_space(source, start + 1);
    }
    if source.as_bytes().get(start) != Some(&b'[') {
        return true;
    }
    syntax::bracket_argument(source, start).is_some_and(|(options, _)| safe_options(&options.value))
}

/// Keep filename wrappers outside the editable span. Only the primitive
/// detokenize wrapper is understood, and project guards reject its redefinition.
pub(crate) fn filename(source: &str, content: Range<usize>, partial: bool) -> Option<Range<usize>> {
    if source[content.clone()].contains('%') {
        return None;
    }
    let mut span = syntax::trimmed_span(source, content);
    if source[span.clone()].starts_with("\\detokenize") {
        let open = syntax::skip_space(source, span.start + "\\detokenize".len());
        if source.as_bytes().get(open) != Some(&b'{') {
            return None;
        }
        if let Some((argument, after)) = syntax::group(source, open, b'{', b'}') {
            if after > span.end || !source[after..span.end].trim().is_empty() {
                return None;
            }
            span = argument.span;
        } else if partial {
            span = open + 1..span.end;
        } else {
            return None;
        }
    }
    let quoted = source.as_bytes().get(span.start) == Some(&b'"');
    if quoted {
        span.start += 1;
        if span.start < span.end && source.as_bytes().get(span.end - 1) == Some(&b'"') {
            span.end -= 1;
        } else if !partial {
            return None;
        }
    }
    let value = &source[span.clone()];
    if !(value.is_empty() && partial || syntax::is_literal_file_path(value))
        || !partial && !quoted && value.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some(span)
}

pub(crate) fn prefix(
    source: &str,
    command: &syntax::Command,
    cursor: usize,
) -> Option<Range<usize>> {
    let open = syntax::file_name_argument_start(source, &command.name, command.span.end)?;
    if source.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let content = syntax::group(source, open, b'{', b'}')
        .map_or(open + 1..source.len(), |(argument, _)| argument.span);
    let span = filename(source, content, true)?;
    (span.start <= cursor && cursor <= span.end).then_some(span.start..cursor)
}

/// Expanding the edit to the complete argument lets a rename introduce spaces
/// and UTF-8 safely for both pdfTeX and Unicode engines.
pub(crate) fn filename_edit(
    text: &Text,
    command: &syntax::Command,
    value: String,
) -> Option<TextEdit> {
    let argument = command.argument.as_ref()?;
    let old = filename(&text.source, argument.span.clone(), false)?;
    let wrapper = &text.source[argument.span.start..old.start];
    if !value.is_ascii() && !wrapper.contains("\\detokenize") {
        return Some(TextEdit {
            range: text.range(argument.span.start, argument.span.end),
            new_text: format!("\\detokenize{{\"{value}\"}}"),
        });
    }
    let quoted =
        old.start > argument.span.start && text.source.as_bytes().get(old.start - 1) == Some(&b'"');
    Some(TextEdit {
        range: text.range(old.start, old.end),
        new_text: if !quoted && value.chars().any(char::is_whitespace) {
            format!("\"{value}\"")
        } else {
            value
        },
    })
}

pub(crate) fn completion_edit(
    text: &Text,
    command: &syntax::Command,
    mut value: String,
    replaced_end: usize,
) -> Option<TextEdit> {
    let open = syntax::file_name_argument_start(&text.source, &command.name, command.span.end)?;
    if text.source.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let content = syntax::group(&text.source, open, b'{', b'}')
        .map_or(open + 1..text.source.len(), |(argument, _)| argument.span);
    let span = filename(&text.source, content.clone(), true)?;
    // Completing a directory in the middle of a path must keep the remaining
    // components inside the filename wrappers.
    value.push_str(text.source.get(replaced_end..span.end)?);
    let wrapper = &text.source[content.start..span.start];
    let quoted = text.source.as_bytes().get(span.start.saturating_sub(1)) == Some(&b'"');
    let value = if quoted || value.chars().any(char::is_whitespace) || !value.is_ascii() {
        format!("\"{value}\"")
    } else {
        value
    };
    let value = if wrapper.contains("\\detokenize") || !value.is_ascii() {
        format!("\\detokenize{{{value}}}")
    } else {
        value
    };
    Some(TextEdit {
        range: text.range(content.start, content.end),
        new_text: value,
    })
}
