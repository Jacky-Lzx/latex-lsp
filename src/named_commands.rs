//! Static control sequence names consumed by etoolbox's named interfaces.
use crate::{syntax, workspace::Workspace};
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
    argument(source, start, true)?.0
}

fn argument(source: &str, start: usize, named: bool) -> Option<(Option<Range<usize>>, usize)> {
    if let Some((argument, end)) = syntax::group(source, start, b'{', b'}') {
        let span = syntax::trimmed_span(source, argument.span);
        if named {
            let valid = syntax::is_static_key(&source[span.clone()]);
            Some((valid.then_some(span), end))
        } else {
            (syntax::control_word(source, span.start)? == span).then_some((None, end))
        }
    } else if !named {
        Some((None, syntax::control_word(source, start)?.end))
    } else {
        None
    }
}

pub(crate) fn spans(source: &str, command: &syntax::Command) -> Vec<Range<usize>> {
    let mut result = Vec::new();
    let mut start = syntax::skip_space(source, command.span.end);
    for &named in arguments(&command.name) {
        let Some((span, end)) = argument(source, start, named) else {
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
                    if let Some(span) = syntax::active_static_argument(source, start, b'{', cursor)
                    {
                        return Some(span);
                    }
                }
                let (_, end) = argument(source, start, named)?;
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
    let prefix = &source[span.start..cursor];
    Some(
        names
            .into_iter()
            .filter(|(name, _)| name.starts_with(prefix) && syntax::is_static_key(name))
            .map(|(name, detail)| CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(detail),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: doc.text.range(span.start, span.end),
                    new_text: name,
                })),
                ..Default::default()
            })
            .collect(),
    )
}
