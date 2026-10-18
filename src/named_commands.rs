//! Static control sequence names consumed by etoolbox's named interfaces.
use crate::{syntax, workspace::Workspace};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use tower_lsp::lsp_types::*;

fn count(name: &str) -> usize {
    match name {
        "csuse" | "csmeaning" | "ifcsdef" | "ifcsundef" | "ifcsvoid" | "ifcsmacro"
        | "ifcsparam" | "ifcsprotected" | "ifcsltxprotect" | "ifcsstring" => 1,
        "ifcsequal" | "ifcsstrequal" => 2,
        _ => 0,
    }
}

pub(crate) fn spans(source: &str, command: &syntax::Command) -> Vec<Range<usize>> {
    let mut result = Vec::new();
    let mut start = syntax::skip_space(source, command.span.end);
    for _ in 0..count(&command.name) {
        let Some((argument, end)) = syntax::group(source, start, b'{', b'}') else {
            break;
        };
        let span = syntax::trimmed_span(source, argument.span);
        if syntax::is_static_key(&source[span.clone()]) {
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
            for _ in 0..count(&command.name) {
                if let Some(span) = syntax::active_static_argument(source, start, b'{', cursor) {
                    return Some(span);
                }
                let (_, end) = syntax::group(source, start, b'{', b'}')?;
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
