//! Search bibliography previews without changing the inserted citation key.
use crate::{
    syntax,
    workspace::{DocumentKind, Workspace},
};
use std::collections::BTreeSet;
use tower_lsp::lsp_types::*;

fn score(name: &str, metadata: &str, query: &str) -> Option<u8> {
    if name.starts_with(query) {
        return Some(0);
    }
    let query = query.to_lowercase();
    let name = name.to_lowercase();
    if name.starts_with(&query) {
        return Some(1);
    }
    if name.contains(&query) {
        return Some(2);
    }
    let words: Vec<_> = query.split_whitespace().collect();
    let metadata = metadata.to_lowercase();
    if !words.is_empty()
        && words
            .iter()
            .all(|word| name.contains(word) || metadata.contains(word))
    {
        return Some(3);
    }
    let mut chars = name.chars();
    if query.chars().count() >= 3
        && query
            .chars()
            .all(|ch| chars.by_ref().any(|candidate| candidate == ch))
    {
        return Some(4);
    }
    if query.chars().count() >= 4 && crate::spelling_fixes::distance(&name, &query, 2).is_some() {
        return Some(5);
    }
    None
}

pub(crate) fn completion(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = ws.documents.get(uri)?;
    if doc.kind != DocumentKind::Latex {
        return None;
    }
    let source = &doc.text.source;
    for command in doc.syntax.commands.iter().rev().filter(|cmd| {
        syntax::is_citation(&cmd.name)
            && !syntax::is_single_citation(&cmd.name)
            && !doc.syntax.control_operands.contains(&cmd.span.start)
    }) {
        let args: Vec<_> = if let Some(style) = crate::citations::style(&command.name) {
            crate::citations::arguments(source, command.span.end, style)
                .fields
                .into_iter()
                .filter(|field| field.role == crate::citations::Role::Keys)
                .map(|field| field.argument)
                .collect()
        } else {
            let argument = command.argument.clone().or_else(|| {
                let start =
                    syntax::citation_argument_start(source, &command.name, command.span.end)?;
                (source.as_bytes().get(start) == Some(&b'{')).then(|| syntax::Argument {
                    value: source[start + 1..].into(),
                    span: start + 1..source.len(),
                    outer_span: start..source.len(),
                })
            });
            argument.into_iter().collect()
        };
        for arg in args {
            if !(arg.span.start <= offset && offset <= arg.span.end) {
                continue;
            }
            let multiple = !syntax::is_single_citation(&command.name);
            let segment = if multiple {
                syntax::key_list_segments(source, arg.span.clone())
                    .into_iter()
                    .find(|span| span.start <= offset && offset <= span.end)?
            } else {
                arg.span
            };
            // Comments and expansions retain the established literal-key editor.
            if source[segment.clone()].contains(['%', '\\', '{', '}']) {
                return None;
            }
            let mut span = syntax::trimmed_span(source, segment);
            span.start = span.start.min(offset);
            span.end = span.end.max(offset);
            if crate::named_references::completion_context(ws, uri, offset, |candidate| {
                (candidate.span == command.span).then_some(span.start..offset)
            })
            .is_none()
            {
                return Some(vec![]);
            }
            let query = source[span.start..offset].trim();
            if query.chars().count() > 128 {
                return Some(vec![]);
            }
            let (range, extra) = syntax::name_completion_ranges(&doc.text, span, offset)?;
            let main_start = doc.text.offset(range.start)?;
            let filter_prefix = &source[main_start..offset];
            let mut ranked = Vec::new();
            let mut seen = BTreeSet::new();
            for (entry_uri, doc) in &ws.documents {
                if !scope.contains(entry_uri) {
                    continue;
                }
                for entry in doc
                    .syntax
                    .citations
                    .iter()
                    .chain(&doc.syntax.bibliography_aliases)
                {
                    if !syntax::is_static_citation_key(&entry.name)
                        || !seen.insert(entry.name.clone())
                    {
                        continue;
                    }
                    let Some(rank) = score(&entry.name, &entry.detail, query) else {
                        continue;
                    };
                    ranked.push((
                        rank,
                        CompletionItem {
                            label: entry.name.clone(),
                            kind: Some(CompletionItemKind::REFERENCE),
                            detail: Some(format!("{} — {}", entry.detail, entry_uri.path())),
                            filter_text: Some(format!(
                                "{filter_prefix} {} {}",
                                entry.name, entry.detail
                            )),
                            sort_text: Some(format!("{rank:02}{}", entry.name)),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                range,
                                new_text: entry.name.clone(),
                            })),
                            additional_text_edits: (!extra.is_empty()).then(|| extra.clone()),
                            ..Default::default()
                        },
                    ));
                }
            }
            ranked.sort_by(|(a, ia), (b, ib)| a.cmp(b).then_with(|| ia.label.cmp(&ib.label)));
            return Some(ranked.into_iter().map(|(_, item)| item).collect());
        }
    }
    None
}
