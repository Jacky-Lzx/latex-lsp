//! Static control sequence names consumed by etoolbox's named interfaces.
use crate::{
    syntax,
    workspace::{Document, Workspace},
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
        | "ifcsparam" | "ifcsprotected" | "ifcsltxprotect" | "ifcsstring" | "csundef"
        | "csgundef" | "csappto" | "cseappto" | "csgappto" | "csxappto" | "cspreto"
        | "csepreto" | "csgpreto" | "csxpreto" => &[true],
        _ => &[],
    }
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
        .filter(|command| command.span.end <= cursor)
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
    for (name, _, detail) in crate::catalog::COMMANDS {
        names.insert((*name).to_owned(), (*detail).to_owned());
    }
    for name in crate::catalog::PRIMITIVES {
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
        for name in crate::generated::commands(doc) {
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
