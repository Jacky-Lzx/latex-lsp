//! Static control sequence names consumed by etoolbox's named interfaces.
use crate::{
    parser::syntax,
    project::workspace::{Document, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use tower_lsp::lsp_types::*;

/// Whether each argument contains a literal name, rather than a command token.
fn arguments(name: &str) -> &'static [bool] {
    match name {
        "letcs" => &[false, true],
        "csletcs" | "ifcsequal" | "ifcsstrequal" => &[true, true],
        name if declares(name) => &[true],
        "csuse" | "csmeaning" | "ifcsdef" | "ifcsundef" | "ifcsvoid" | "ifcsmacro"
        | "ifcsparam" | "ifcsprotected" | "ifcsltxprotect" | "ifcscounter" | "ifcslength"
        | "ifcsdimen" | "ifcsstring" | "csundef" | "csgundef" | "csappto" | "cseappto"
        | "csgappto" | "csxappto" | "cspreto" | "csepreto" | "csgpreto" | "csxpreto" => &[true],
        _ => &[],
    }
}

/// Whether the undefinition interface reads a name or an unexpanded control.
pub(crate) fn undefines(name: &str) -> Option<bool> {
    match name {
        "undef" | "gundef" => Some(false),
        "csundef" | "csgundef" => Some(true),
        _ => None,
    }
}

pub(crate) fn undefinition(source: &str, command: &syntax::Command) -> Option<syntax::Entry> {
    let named = undefines(&command.name)?;
    let at = syntax::skip_space(source, command.span.end);
    let grouped = syntax::group(source, at, b'{', b'}');
    let span = if named {
        let argument = grouped?.0;
        syntax::is_static_key(&source[argument.span.clone()]).then_some(argument.span)?
    } else if let Some((argument, _)) = grouped {
        let bounded = &source[..argument.span.end];
        let word = syntax::control_word(bounded, syntax::skip_space(bounded, argument.span.start))?;
        (syntax::skip_space(bounded, word.end) == argument.span.end).then_some(word)?
    } else {
        syntax::control_word(source, at)?
    };
    Some(syntax::Entry {
        name: source[span.start + usize::from(!named)..span.end].into(),
        span,
        detail: format!("Undefined with \\{}", command.name),
        is_variable: false,
    })
}

pub(crate) fn declares(name: &str) -> bool {
    matches!(
        name,
        "csdef" | "csgdef" | "csedef" | "csxdef" | "cslet" | "csletcs"
    )
}

pub(crate) fn declaration(source: &str, command: &syntax::Command) -> Option<Range<usize>> {
    if !declares(&command.name) {
        return None;
    }
    // Inspect the destination itself. A dynamic destination must never make
    // csletcs's static source appear to be a declaration instead.
    let start = syntax::skip_space(source, command.span.end);
    let (argument, _) = syntax::group(source, start, b'{', b'}')?;
    let span = argument.span;
    syntax::is_static_key(&source[span.clone()]).then_some(span)
}

fn error_argument(source: &str, start: usize) -> Option<syntax::Argument> {
    if let Some((argument, _)) = syntax::group(source, start, b'{', b'}') {
        return Some(argument);
    }
    let span = syntax::control_word(source, start)?;
    Some(syntax::Argument {
        value: source[span.clone()].into(),
        span: span.clone(),
        outer_span: span,
    })
}

/// Attribute a generic frame to one exact macro operand of a known interface.
/// Other named operands must be static; control-token operands are never blamed.
pub(crate) fn name_error_invocation(
    source: &str,
    command: &syntax::Command,
    frame: &str,
) -> Option<(Range<usize>, Range<usize>)> {
    let operands = arguments(&command.name);
    if operands.is_empty() {
        return None;
    }
    let start = syntax::skip_space(source, command.span.end);
    let mut end = start;
    let mut selected = None;
    // cslet also consumes its source control token before expanding the name.
    for (index, named) in operands
        .iter()
        .copied()
        .chain((command.name == "cslet").then_some(false))
        .enumerate()
    {
        let argument = error_argument(source, syntax::skip_space(source, end))?;
        end = argument.outer_span.end;
        let word = syntax::control_word(&argument.value, 0)
            .is_some_and(|span| span == (0..argument.value.len()));
        if !named {
            if !word {
                return None;
            }
        } else if word {
            // The copy destination uses an internal let/csname frame rather
            // than the simple source-name frame verified by this matcher.
            if command.name == "csletcs" && index == 0
                || selected.is_some()
                || argument.value != frame
            {
                return None;
            }
            selected = Some(argument.span);
        } else if !syntax::is_static_key(&argument.value) {
            return None;
        }
    }
    Some((selected?, start..end))
}

/// Respect the explicit expansion before letcs's destination. Without it,
/// csname itself is the token argument; never scan ahead to guess a name.
pub(crate) fn control_argument_end(
    doc: &Document,
    command: &syntax::Command,
    start: usize,
) -> Option<usize> {
    let source = &doc.text.source;
    let word = syntax::control_word(source, start)?;
    if command.name != "letcs" || source[word.clone()] != *"\\csname" {
        return Some(word.end);
    }
    let commands = &doc.syntax.commands;
    let index = commands
        .binary_search_by_key(&command.span.start, |item| item.span.start)
        .ok()?;
    let expanded = index
        .checked_sub(1)
        .and_then(|index| commands.get(index))
        .is_some_and(|previous| {
            previous.name == "expandafter"
                && syntax::skip_space(source, previous.span.end) == command.span.start
        });
    if !expanded {
        return Some(word.end);
    }
    if syntax::skip_space(source, command.span.end) != start {
        return None;
    }
    syntax::literal_csname(source, commands, index + 1)?;
    Some(commands.get(index + 2)?.span.end)
}

fn argument(
    doc: &Document,
    command: &syntax::Command,
    start: usize,
    named: bool,
) -> Option<(Option<Range<usize>>, usize)> {
    let source = &doc.text.source;
    if let Some((argument, end)) = syntax::group(source, start, b'{', b'}') {
        if named {
            let span = argument.span;
            let valid = syntax::is_static_key(&source[span.clone()]);
            Some((valid.then_some(span), end))
        } else {
            let span = syntax::trimmed_span(source, argument.span);
            (syntax::control_word(source, span.start)? == span).then_some((None, end))
        }
    } else if !named {
        Some((None, control_argument_end(doc, command, start)?))
    } else {
        None
    }
}

pub(crate) fn spans(doc: &Document, command: &syntax::Command) -> Vec<Range<usize>> {
    if doc.syntax.control_operands.contains(&command.span.start) {
        return vec![];
    }
    let source = &doc.text.source;
    let mut result = Vec::new();
    let mut start = syntax::skip_space(source, command.span.end);
    for &named in arguments(&command.name) {
        let Some((span, end)) = argument(doc, command, start, named) else {
            break;
        };
        if let Some(span) = span {
            result.push(span);
        }
        start = syntax::skip_space(source, end);
    }
    result
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &Url,
    cursor: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = workspace.documents.get(uri)?;
    let source = &doc.text.source;
    let span = doc
        .syntax
        .commands
        .iter()
        .rev()
        .filter(|command| {
            command.span.end <= cursor && !doc.syntax.control_operands.contains(&command.span.start)
        })
        .take(128)
        .find_map(|command| {
            let mut start = syntax::skip_space(source, command.span.end);
            for (index, &named) in arguments(&command.name).iter().enumerate() {
                if named && !(index == 0 && declares(&command.name)) {
                    if let Some(span) = syntax::active_name_argument(source, start, b'{', cursor) {
                        return Some(span);
                    }
                }
                let (_, end) = argument(doc, command, start, named)?;
                start = syntax::skip_space(source, end);
            }
            None
        })?;
    let mut names = BTreeMap::new();
    for (name, _, detail) in crate::project::catalog::COMMANDS {
        names.insert((*name).to_owned(), (*detail).to_owned());
    }
    for name in crate::project::catalog::PRIMITIVES {
        names
            .entry((*name).to_owned())
            .or_insert_with(|| "TeX primitive".into());
    }
    for (uri, doc) in &workspace.documents {
        if !scope.contains(uri) {
            continue;
        }
        for entry in &doc.syntax.definitions {
            names.insert(
                entry.name.clone(),
                format!("{} — {}", entry.detail, uri.path()),
            );
        }
        for name in crate::symbols::generated::commands(doc) {
            names
                .entry(name)
                .or_insert_with(|| format!("Generated command — {}", uri.path()));
        }
    }
    let prefix = source[span.start..cursor].trim_start();
    let (range, additional_edits) = syntax::name_completion_ranges(&doc.text, span, cursor)?;
    Some(
        names
            .into_iter()
            .filter(|(name, _)| name.starts_with(prefix) && syntax::is_static_key(name))
            .map(|(name, detail)| CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(detail),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: name,
                })),
                additional_text_edits: (!additional_edits.is_empty())
                    .then(|| additional_edits.clone()),
                ..Default::default()
            })
            .collect(),
    )
}
