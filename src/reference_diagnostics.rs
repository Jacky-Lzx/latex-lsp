//! Conservative, project-scoped references that do not require a compiler run.
use crate::{
    syntax,
    workspace::{DocumentKind, Workspace},
};
use std::{collections::BTreeSet, ops::Range};
use tower_lsp::lsp_types::*;

pub(crate) fn occurrences(ws: &Workspace, uri: &Url) -> Vec<(bool, Range<usize>)> {
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    if doc.kind != DocumentKind::Latex || !ws.index_truncated.is_empty() {
        return vec![];
    }
    let scope = ws.symbol_scope(uri);
    let docs: Vec<_> = scope
        .iter()
        .filter_map(|uri| ws.documents.get(uri))
        .filter(|doc| doc.kind == DocumentKind::Latex)
        .collect();
    if docs.iter().map(|doc| doc.text.source.len()).sum::<usize>() > 16 * 1024 * 1024
        || docs
            .iter()
            .map(|doc| doc.syntax.commands.len())
            .sum::<usize>()
            > 100_000
        || docs
            .iter()
            .map(|doc| doc.syntax.definitions.len() + doc.syntax.environments.len())
            .sum::<usize>()
            > 4096
    {
        return vec![];
    }
    let signatures = crate::signature::CommandSignatures::new(
        docs.iter()
            .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
    );
    let source = &doc.text.source;
    let operands = signatures.argument_tokens(source, &doc.syntax);
    let mut opaque = crate::signature::definition_data_spans(source, &doc.syntax);
    opaque.extend(crate::expl3::math_data_spans(doc));
    opaque.extend(doc.syntax.ignored.iter().cloned());
    opaque.extend(doc.syntax.literal_conversions.iter().cloned());
    opaque.extend(doc.syntax.literal_tests.iter().cloned());
    opaque.extend(doc.syntax.ambiguous_string_tests.iter().cloned());
    opaque.sort_unstable_by_key(|span| span.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for span in opaque {
        if let Some(last) = merged.last_mut().filter(|last| span.start <= last.end) {
            last.end = last.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    let declarations = doc
        .syntax
        .definitions
        .iter()
        .map(|entry| (entry.span.start, entry.span.end))
        .collect();
    let mut opaque = 0;
    let mut result = Vec::new();
    for (index, command) in doc.syntax.commands.iter().enumerate() {
        let at = command.span.start;
        while merged.get(opaque).is_some_and(|span| span.end <= at) {
            opaque += 1;
        }
        if doc.syntax.control_operands.contains(&at)
            || operands.contains(&at)
            || merged.get(opaque).is_some_and(|span| span.contains(&at))
        {
            continue;
        }
        let resolved = crate::reference_calls::named_command(
            source,
            &doc.syntax,
            index,
            &signatures,
            &operands,
            &declarations,
        );
        let command = resolved.as_ref().unwrap_or(command);
        if !signatures.command_unchanged(&command.name) {
            continue;
        }
        result.extend(crate::reference_calls::keys(source, command, true));
    }
    result.sort_by_key(|(citation, span)| (span.start, span.end, *citation));
    result.dedup();
    result
}

pub(crate) fn diagnostics(ws: &Workspace, uri: &Url) -> Vec<Diagnostic> {
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    let settings = ws.settings(uri);
    let config = &settings.diagnostics;
    if !config.undefined_references && !config.undefined_citations {
        return vec![];
    }
    let scope = ws.symbol_scope(uri);
    let mut labels = BTreeSet::new();
    let mut citations = BTreeSet::new();
    for (uri, doc) in &ws.documents {
        if !scope.contains(uri) {
            continue;
        }
        labels.extend(doc.syntax.labels.iter().map(|entry| entry.name.as_str()));
        citations.extend(
            doc.syntax
                .citations
                .iter()
                .chain(&doc.syntax.bibliography_aliases)
                .map(|entry| entry.name.as_str()),
        );
    }
    let auxiliary = crate::auxiliary::labels(ws, uri);
    labels.extend(
        auxiliary
            .iter()
            .filter(|(_, info)| info.external)
            .map(|(name, _)| name.as_str()),
    );
    occurrences(ws, uri)
        .into_iter()
        .filter_map(|(citation, span)| {
            let name = &doc.text.source[span.clone()];
            let (enabled, known, kind) = if citation {
                (config.undefined_citations, &citations, "citation")
            } else {
                (config.undefined_references, &labels, "reference")
            };
            if !enabled || name == "*" || known.contains(name) {
                return None;
            }
            // An identical compiler-confirmed warning already carries more context.
            let range = doc.text.range(span.start, span.end);
            if ws
                .build_diagnostics
                .values()
                .filter_map(|project| project.get(uri))
                .flatten()
                .any(|diag| {
                    diag.range == range
                        && crate::build_log::undefined_reference_key(&diag.message)
                            .is_some_and(|(is_cite, _)| is_cite == citation)
                })
            {
                return None;
            }
            Some(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("latex-lsp".into()),
                code: Some(NumberOrString::String(format!("undefined-{kind}"))),
                message: format!("Undefined {kind} `{name}`"),
                data: Some(serde_json::json!({"key":name,"citation":citation})),
                ..Default::default()
            })
        })
        .collect()
}

pub(crate) fn fixes(
    ws: &Workspace,
    uri: &Url,
    range: tower_lsp::lsp_types::Range,
    requested: &[Diagnostic],
) -> Vec<syntax::QuickFix> {
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    let scope = ws.symbol_scope(uri);
    let auxiliary = crate::auxiliary::labels(ws, uri);
    let mut result = Vec::new();
    for diagnostic in diagnostics(ws, uri)
        .into_iter()
        .filter(|diag| {
            diag.range.start <= range.end
                && range.start <= diag.range.end
                && (requested.is_empty() || requested.contains(diag))
        })
        .take(64)
    {
        let Some(data) = &diagnostic.data else {
            continue;
        };
        let Some(key) = data["key"].as_str() else {
            continue;
        };
        let citation = data["citation"].as_bool().unwrap_or(false);
        let length = key.chars().count();
        if !(3..=128).contains(&length) {
            continue;
        }
        let mut candidates: BTreeSet<_> = ws
            .documents
            .iter()
            .filter(|(uri, _)| scope.contains(*uri))
            .flat_map(|(_, doc)| {
                if citation {
                    doc.syntax
                        .citations
                        .iter()
                        .chain(&doc.syntax.bibliography_aliases)
                        .collect::<Vec<_>>()
                } else {
                    doc.syntax.labels.iter().collect()
                }
            })
            .take(4096)
            .map(|entry| entry.name.as_str())
            .collect();
        if !citation {
            candidates.extend(
                auxiliary
                    .iter()
                    .filter(|(_, info)| info.external)
                    .map(|(name, _)| name.as_str()),
            );
        }
        let mut ranked: Vec<_> = candidates
            .into_iter()
            .filter_map(|name| {
                crate::spelling_fixes::distance(key, name, if length < 8 { 1 } else { 2 })
                    .map(|score| (score, name))
            })
            .collect();
        ranked.sort();
        for (_, name) in ranked.into_iter().take(5) {
            let new_text = if citation {
                name.into()
            } else {
                syntax::label_replacement_text(
                    &doc.text,
                    &doc.syntax.commands,
                    diagnostic.range,
                    name,
                )
            };
            result.push(syntax::QuickFix {
                title: format!(
                    "Replace with {} `{name}`",
                    if citation { "citation" } else { "label" }
                ),
                diagnostic: diagnostic.clone(),
                edit: TextEdit {
                    range: diagnostic.range,
                    new_text,
                },
                additional_edits: vec![],
            });
        }
    }
    result
}
