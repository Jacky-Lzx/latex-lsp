//! Source-confirmed right delimiter repairs; never guess a visible delimiter.
use crate::{
    signature::CommandSignatures,
    syntax::{self, QuickFix, Syntax},
    text::Text,
    workspace::Document,
};
use std::ops::Range as Span;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, TextEdit};

pub(crate) const ERROR: &str = r"Extra \right.";
pub(crate) const CODE: &str = "unmatched-right-delimiter";
pub(crate) const MISSING_ERROR: &str = r"Missing \right. inserted.";
pub(crate) const MISSING_CODE: &str = "missing-right-delimiter";

pub(crate) struct Invocation {
    pub command: Span<usize>,
    pub delimiter: Span<usize>,
}

fn named_delimiter(name: &str) -> bool {
    matches!(
        name,
        "langle"
            | "rangle"
            | "lbrace"
            | "rbrace"
            | "lfloor"
            | "rfloor"
            | "lceil"
            | "rceil"
            | "vert"
            | "Vert"
            | "backslash"
            | "uparrow"
            | "downarrow"
            | "updownarrow"
            | "Uparrow"
            | "Downarrow"
            | "Updownarrow"
            | "lgroup"
            | "rgroup"
            | "lmoustache"
            | "rmoustache"
    )
}

/// Accept only a standard delimiter's own expansion; additional user frames
/// must still prevent attributing an error to a direct source call.
pub(crate) fn expansion_frame(line: &str) -> Option<&str> {
    let (name, code) = line
        .trim_end()
        .strip_prefix('\\')?
        .split_once(" ->\\delimiter \"")?;
    (named_delimiter(name)
        && !code.is_empty()
        && code.len() <= 8
        && code.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then_some(name)
}

fn source_end(text: &Text, line: u32, context: &str) -> Option<usize> {
    let prefix = context.trim_end();
    if prefix.is_empty() || prefix.len() > 4096 {
        return None;
    }
    let start = text.offset(Position::new(line, 0))?;
    let value = text.source.get(start..)?.lines().next()?;
    Some(if let Some(suffix) = prefix.strip_prefix("...") {
        let suffix = suffix.trim_start_matches('\u{fffd}');
        let at = value.find(suffix)?;
        if suffix.is_empty() || value.rfind(suffix) != Some(at) {
            return None;
        }
        start + at + suffix.len()
    } else {
        if !value.starts_with(prefix) {
            return None;
        }
        start + prefix.len()
    })
}

pub(crate) fn invocation(
    text: &Text,
    parsed: &Syntax,
    line: u32,
    context: &str,
    frame: Option<&str>,
    signatures: Option<&CommandSignatures<'_>>,
) -> Option<Invocation> {
    let end = source_end(text, line, context)?;
    let (command, delimiter) = parsed
        .commands
        .iter()
        .filter(|command| command.name == "right")
        .find_map(|command| {
            let at = syntax::skip_space(&text.source, command.span.end);
            let token = text.source.get(at..end)?;
            let valid = if let Some(name) = token.strip_prefix('\\') {
                named_delimiter(name) && frame == Some(name)
            } else {
                frame.is_none() && matches!(token, "(" | ")" | "[" | "]" | "/" | "|" | ".")
            };
            valid.then_some((command, at..end))
        })?;
    if parsed.is_ignored(command.span.start)
        || parsed.control_operands.contains(&command.span.start)
        || crate::signature::definition_body_spans(&text.source, parsed)
            .iter()
            .any(|body| body.contains(&command.span.start))
    {
        return None;
    }
    let local;
    let signatures = match signatures {
        Some(signatures) => signatures,
        None => {
            local = CommandSignatures::new([(text.source.as_str(), parsed)]);
            &local
        }
    };
    if !signatures.command_unchanged("right")
        || frame.is_some_and(|name| !signatures.command_unchanged(name))
        || signatures.literal_is_argument(&text.source, parsed, command.span.start)
    {
        return None;
    }
    Some(Invocation {
        command: command.span.clone(),
        delimiter,
    })
}

/// The engine's exact recovery frame confirms the missing delimiter. Only
/// repair at a directly echoed math/group terminator, never at end{document}
/// or another delayed recovery location.
pub(crate) fn missing_boundary(
    text: &Text,
    parsed: &Syntax,
    line: u32,
    context: &str,
    signatures: Option<&CommandSignatures<'_>>,
) -> Option<Span<usize>> {
    let end = source_end(text, line, context)?;
    let source = &text.source;
    let (start, command) = if source[..end].ends_with('$') {
        (end - 1, None)
    } else if source[..end].ends_with(r"\)") || source[..end].ends_with(r"\]") {
        (end - 2, Some(&source[end - 1..end]))
    } else {
        let command = parsed.commands.iter().rev().find(|command| {
            command.span.end == end && command.name == "endgroup"
                || command.name == "end"
                    && command.argument.as_ref().is_some_and(|argument| {
                        argument.outer_span.end == end
                            && matches!(
                                argument.value.as_str(),
                                "math" | "displaymath" | "equation"
                            )
                    })
        })?;
        (command.span.start, Some(command.name.as_str()))
    };
    let backslashes = source.as_bytes()[..start]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    if backslashes % 2 == 1
        || parsed.is_ignored(start)
        || parsed.control_operands.contains(&start)
        || crate::signature::definition_body_spans(source, parsed)
            .iter()
            .any(|body| body.contains(&start))
    {
        return None;
    }
    let local;
    let signatures = match signatures {
        Some(signatures) => signatures,
        None => {
            local = CommandSignatures::new([(source.as_str(), parsed)]);
            &local
        }
    };
    if !signatures.command_unchanged("right")
        || command.is_some_and(|name| !signatures.command_unchanged(name))
        || signatures.literal_is_argument(source, parsed, start)
        || parsed.commands.iter().any(|command| {
            syntax::skip_space(source, command.span.end) == start
                && ["string", "meaning", "noexpand"]
                    .iter()
                    .any(|name| signatures.invokes_builtin(&command.name, name))
        })
    {
        return None;
    }
    if command == Some("end") {
        let call = parsed
            .commands
            .iter()
            .find(|call| call.span.start == start)?;
        let name = &call.argument.as_ref()?.value;
        if !signatures.environment_primitives_unchanged()
            || !signatures.command_unchanged(name)
            || !signatures.command_unchanged(&format!("end{name}"))
        {
            return None;
        }
    }
    Some(start..end)
}

fn missing_fix(
    doc: &Document,
    diagnostic: &Diagnostic,
    signatures: Option<&CommandSignatures<'_>>,
) -> Option<QuickFix> {
    if diagnostic.message != MISSING_ERROR
        || diagnostic.severity != Some(DiagnosticSeverity::ERROR)
        || diagnostic.code != Some(NumberOrString::String(MISSING_CODE.into()))
    {
        return None;
    }
    let data = diagnostic.data.as_ref()?;
    let boundary = missing_boundary(
        &doc.text,
        &doc.syntax,
        u32::try_from(data["sourceLine"].as_u64()?).ok()?,
        data["sourceContext"].as_str()?,
        signatures,
    )?;
    if doc.text.range(boundary.start, boundary.end) != diagnostic.range
        || data["closingToken"].as_str()? != &doc.text.source[boundary.clone()]
    {
        return None;
    }
    Some(QuickFix {
        title: r"Insert invisible '\right.'".into(),
        diagnostic: diagnostic.clone(),
        edit: TextEdit {
            range: doc.text.range(boundary.start, boundary.start),
            new_text: r"\right.".into(),
        },
        additional_edits: vec![],
    })
}

pub(crate) fn fix(
    doc: &Document,
    diagnostic: &Diagnostic,
    signatures: Option<&CommandSignatures<'_>>,
) -> Option<QuickFix> {
    if diagnostic.message == MISSING_ERROR {
        return missing_fix(doc, diagnostic, signatures);
    }
    if diagnostic.message != ERROR
        || diagnostic.severity != Some(DiagnosticSeverity::ERROR)
        || diagnostic.code != Some(NumberOrString::String(CODE.into()))
    {
        return None;
    }
    let data = diagnostic.data.as_ref()?;
    let line = u32::try_from(data["sourceLine"].as_u64()?).ok()?;
    let invocation = invocation(
        &doc.text,
        &doc.syntax,
        line,
        data["sourceContext"].as_str()?,
        data["delimiterFrame"].as_str(),
        signatures,
    )?;
    let range = doc
        .text
        .range(invocation.command.start, invocation.command.end);
    let delimiter = &doc.text.source[invocation.delimiter.clone()];
    if range != diagnostic.range || data["delimiter"].as_str()? != delimiter {
        return None;
    }
    let invisible = delimiter == ".";
    Some(QuickFix {
        title: r"Remove unmatched '\right'".into(),
        diagnostic: diagnostic.clone(),
        edit: TextEdit {
            range,
            // An empty group also prevents adjacent control words merging
            // when the invisible delimiter is removed. Preserve comments.
            new_text: if invisible { "{}" } else { "" }.into(),
        },
        additional_edits: if invisible {
            vec![TextEdit {
                range: doc
                    .text
                    .range(invocation.delimiter.start, invocation.delimiter.end),
                new_text: String::new(),
            }]
        } else {
            vec![]
        },
    })
}
