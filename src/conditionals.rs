//! Completion for the named boolean APIs of ifthen and etoolbox.
use crate::{syntax, workspace::Workspace};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::*;

#[derive(Clone, Copy, PartialEq)]
enum ArgumentKind {
    Assignable,
    Test,
    Value,
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &Url,
    cursor: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = workspace.documents.get(uri)?;
    let source = &doc.text.source;
    let (span, kind) = doc
        .syntax
        .commands
        .iter()
        .rev()
        .filter(|command| command.span.end <= cursor)
        .take(128)
        .filter(|command| syntax::boolean_reference(&command.name))
        .find_map(|command| {
            let start = syntax::skip_space(source, command.span.end);
            if let Some(span) = syntax::active_static_argument(source, start, b'{', cursor) {
                let kind = if matches!(command.name.as_str(), "boolean" | "ifbool" | "notbool") {
                    ArgumentKind::Test
                } else {
                    ArgumentKind::Assignable
                };
                return Some((span, kind));
            }
            if !matches!(command.name.as_str(), "setboolean" | "setbool") {
                return None;
            }
            let (_, end) = syntax::group(source, start, b'{', b'}')?;
            syntax::active_static_argument(source, syntax::skip_space(source, end), b'{', cursor)
                .map(|span| (span, ArgumentKind::Value))
        })?;
    let mut candidates = BTreeMap::new();
    // Primitive iftrue/iffalse have no setters. Offer them for tests and
    // values, while assignments require an explicitly declared boolean.
    if kind != ArgumentKind::Assignable {
        candidates.insert("true".to_owned(), "True boolean value".to_owned());
        candidates.insert("false".to_owned(), "False boolean value".to_owned());
    }
    if kind != ArgumentKind::Value {
        for (uri, doc) in &workspace.documents {
            if !scope.contains(uri) {
                continue;
            }
            for entry in &doc.syntax.conditionals {
                candidates.insert(
                    entry.name.clone(),
                    format!("{} — {}", entry.detail, uri.path()),
                );
            }
        }
    }
    let prefix = &source[span.start..cursor];
    let range = doc.text.range(span.start, span.end);
    Some(
        candidates
            .into_iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, detail)| CompletionItem {
                label: name.clone(),
                detail: Some(detail),
                kind: Some(if kind == ArgumentKind::Value {
                    CompletionItemKind::VALUE
                } else {
                    CompletionItemKind::VARIABLE
                }),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: name,
                })),
                ..Default::default()
            })
            .collect(),
    )
}
