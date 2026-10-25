//! Locate literal compiler-reported characters and repair TeX specials.
use crate::{
    language::signature,
    parser::syntax::{QuickFix, Syntax},
    project::workspace::Document,
    text::Text,
};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, TextEdit,
};

pub(crate) const EXTRA_ALIGNMENT_TAB: &str = r"Extra alignment tab has been changed to \cr.";

pub(crate) fn diagnostic_code(message: &str) -> &'static str {
    match message {
        EXTRA_ALIGNMENT_TAB => "extra-alignment-tab",
        "Double subscript." => "double-subscript",
        "Double superscript." => "double-superscript",
        _ if extra_closing_brace(message) => "extra-closing-brace",
        _ => "unescaped-special-character",
    }
}

fn extra_closing_brace(message: &str) -> bool {
    matches!(
        message,
        "Too many }'s."
            | "Extra }, or forgotten $."
            | r"Extra }, or forgotten \endgroup."
            | r"Extra }, or forgotten \right."
    )
}

pub(crate) fn error_character(message: &str, context: &str) -> Option<char> {
    match message {
        "Double subscript." => return Some('_'),
        "Double superscript." => return Some('^'),
        _ => {}
    }
    if extra_closing_brace(message) {
        return Some('}');
    }
    if matches!(
        message,
        "Misplaced alignment tab character &." | EXTRA_ALIGNMENT_TAB
    ) {
        return Some('&');
    }
    if message == "Missing $ inserted." {
        return context
            .trim_end()
            .chars()
            .next_back()
            .filter(|character| matches!(character, '_' | '^'));
    }
    let mode = message
        .strip_prefix("You can't use `macro parameter character #' in ")?
        .strip_suffix(" mode.")?;
    matches!(
        mode,
        "horizontal" | "vertical" | "math" | "restricted horizontal" | "internal vertical"
    )
    .then_some('#')
}

pub(crate) fn source_range(
    text: &Text,
    parsed: &Syntax,
    line: u32,
    context: &str,
    character: char,
    signatures: Option<&signature::CommandSignatures<'_>>,
) -> Option<Range> {
    let prefix = context.trim_end();
    if prefix.len() > 4096 || !prefix.ends_with(character) {
        return None;
    }
    let start = text.offset(Position::new(line, 0))?;
    let value = text.source.get(start..)?.lines().next()?;
    let end = if let Some(suffix) = prefix.strip_prefix("...") {
        // TeX may shorten the echo in the middle of a UTF-8 scalar.
        let suffix = suffix.trim_start_matches('\u{fffd}');
        let offset = value.find(suffix)?;
        if suffix.is_empty() || value.rfind(suffix) != Some(offset) {
            return None;
        }
        start + offset + suffix.len()
    } else {
        if !value.starts_with(prefix) {
            return None;
        }
        start + prefix.len()
    };
    let at = end.checked_sub(character.len_utf8())?;
    let backslashes = text.source.as_bytes()[..at]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    if backslashes % 2 == 1
        || parsed.is_ignored(at)
        || parsed
            .commands
            .iter()
            .any(|command| command.span.contains(&at))
        || signature::definition_body_spans(&text.source, parsed)
            .iter()
            .any(|body| body.contains(&at))
    {
        return None;
    }
    if matches!(character, '&' | '_' | '^' | '#') {
        let consumed = |signatures: &signature::CommandSignatures<'_>| {
            if character == '#' {
                // Unknown primitives can execute their contents (e.g. hbox).
                // Preserve hash diagnostics unless a declaration consumes it.
                signatures.declared_literal_is_argument(&text.source, parsed, at)
            } else {
                signatures.literal_is_argument(&text.source, parsed, at)
            }
        };
        let argument = match signatures {
            Some(signatures) => consumed(signatures),
            None => consumed(&signature::CommandSignatures::new([(
                text.source.as_str(),
                parsed,
            )])),
        };
        if argument {
            return None;
        }
    }
    Some(text.range(at, end))
}

pub(crate) fn fix(
    doc: &Document,
    diagnostic: &Diagnostic,
    signatures: Option<&signature::CommandSignatures<'_>>,
) -> Option<QuickFix> {
    if diagnostic.severity != Some(DiagnosticSeverity::ERROR) {
        return None;
    }
    let data = diagnostic.data.as_ref()?;
    let context = data["sourceContext"].as_str()?;
    let character = error_character(&diagnostic.message, context)?;
    if diagnostic.code
        != Some(NumberOrString::String(
            diagnostic_code(&diagnostic.message).into(),
        ))
        || data["character"].as_str()? != character.to_string()
    {
        return None;
    }
    if source_range(
        &doc.text,
        &doc.syntax,
        diagnostic.range.start.line,
        context,
        character,
        signatures,
    )? != diagnostic.range
    {
        return None;
    }
    let double_script = matches!(
        diagnostic_code(&diagnostic.message),
        "double-subscript" | "double-superscript"
    );
    let replacement = if double_script {
        format!("{{}}{character}")
    } else {
        match character {
            '}' => String::new(),
            '^' => "\\textasciicircum{}".into(),
            _ => format!("\\{character}"),
        }
    };
    let title = if double_script {
        format!("Insert empty base before '{character}'")
    } else if character == '}' {
        "Remove extra '}'".into()
    } else {
        format!("Escape '{character}' as '{replacement}'")
    };
    Some(QuickFix {
        title,
        diagnostic: diagnostic.clone(),
        edit: TextEdit {
            range: diagnostic.range,
            new_text: replacement,
        },
        additional_edits: vec![],
    })
}
