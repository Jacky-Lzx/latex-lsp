//! Repair a missing operand of a directly reported math delimiter primitive.
use crate::{
    language::signature::CommandSignatures,
    parser::syntax::{self, QuickFix, Syntax},
    project::workspace::Document,
    text::Text,
};
use std::ops::Range as Span;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, TextEdit};

pub(crate) const ERROR: &str = "Missing delimiter (. inserted).";
pub(crate) const CODE: &str = "missing-delimiter-operand";

/// TeX prints a single unexpanded token on the following recovery line.
/// Restrict to ASCII: pdfTeX's UTF-8 tokens and TeX's ^^ notation cannot be
/// matched to one source scalar with this evidence.
pub(crate) fn recovery_token(line: &str) -> Option<&str> {
    let token = line.trim();
    (!token.is_empty()
        && token.len() <= 128
        && token.is_ascii()
        && !token.bytes().any(|byte| byte.is_ascii_control())
        && syntax::token(token, 0) == Some(token))
    .then_some(token)
}

pub(crate) struct Invocation {
    pub command: Span<usize>,
    pub operand: Span<usize>,
}

pub(crate) fn invocation(
    text: &Text,
    parsed: &Syntax,
    line: u32,
    context: &str,
    pending: &str,
    signatures: Option<&CommandSignatures<'_>>,
) -> Option<Invocation> {
    if recovery_token(pending)? != pending {
        return None;
    }
    let end = crate::parser::right_delimiters::source_end(text, line, context)?;
    let source = &text.source;
    let (command, at) = parsed.commands.iter().find_map(|command| {
        if !matches!(command.name.as_str(), "left" | "right" | "middle") {
            return None;
        }
        let at = syntax::skip_space(source, command.span.end);
        (source.get(at..end) == Some(pending)
            && syntax::token(source, at) == Some(pending)
            && !source[at..].starts_with("^^"))
        .then_some((command, at))
    })?;
    if parsed.is_ignored(command.span.start)
        || parsed.is_ignored(at)
        || parsed.control_operands.contains(&command.span.start)
        || crate::language::signature::definition_body_spans(source, parsed)
            .iter()
            .any(|body| body.contains(&command.span.start))
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
    if !signatures.command_unchanged(&command.name)
        || signatures.literal_is_argument(source, parsed, command.span.start)
        || pending.strip_prefix('\\').is_some_and(|name| {
            !signatures.command_unchanged(name) || parsed.control_operands.contains(&at)
        })
    {
        return None;
    }
    Some(Invocation {
        command: command.span.clone(),
        operand: at..end,
    })
}

pub(crate) fn fix(
    doc: &Document,
    diagnostic: &Diagnostic,
    signatures: Option<&CommandSignatures<'_>>,
) -> Option<QuickFix> {
    if diagnostic.message != ERROR
        || diagnostic.severity != Some(DiagnosticSeverity::ERROR)
        || diagnostic.code != Some(NumberOrString::String(CODE.into()))
    {
        return None;
    }
    let data = diagnostic.data.as_ref()?;
    let invocation = invocation(
        &doc.text,
        &doc.syntax,
        u32::try_from(data["sourceLine"].as_u64()?).ok()?,
        data["sourceContext"].as_str()?,
        data["recoveryToken"].as_str()?,
        signatures,
    )?;
    if doc
        .text
        .range(invocation.command.start, invocation.command.end)
        != diagnostic.range
        || data["delimiterCommand"].as_str()? != &doc.text.source[invocation.command.clone()]
    {
        return None;
    }
    Some(QuickFix {
        title: format!(
            "Insert invisible delimiter after '{}'",
            &doc.text.source[invocation.command.clone()]
        ),
        diagnostic: diagnostic.clone(),
        edit: TextEdit {
            range: doc
                .text
                .range(invocation.command.end, invocation.command.end),
            new_text: ".".into(),
        },
        additional_edits: vec![],
    })
}
