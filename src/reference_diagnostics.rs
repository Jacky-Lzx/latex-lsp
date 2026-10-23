//! Conservative, project-scoped references that do not require a compiler run.
use crate::{
    syntax,
    workspace::{Document, DocumentKind, Workspace},
};
use std::{collections::BTreeSet, ops::Range};
use tower_lsp::lsp_types::*;

pub(crate) fn occurrences(doc: &Document) -> Vec<(bool, Range<usize>)> {
    if doc.kind != DocumentKind::Latex {
        return vec![];
    }
    let source = &doc.text.source;
    let mut result = Vec::new();
    for command in &doc.syntax.commands {
        if doc.syntax.control_operands.contains(&command.span.start) {
            continue;
        }
        if syntax::warns_on_undefined_citation(&command.name) {
            if let Some(style) = crate::citations::style(&command.name) {
                for arg in crate::citations::arguments(source, command.span.end, style).keys() {
                    result.extend(
                        crate::citations::key_spans(source, arg, style)
                            .into_iter()
                            .map(|span| (true, span)),
                    );
                }
            } else if let Some(arg) = &command.argument {
                if syntax::is_single_citation(&command.name) {
                    if syntax::is_static_citation_key(&arg.value) {
                        result.push((true, arg.span.clone()));
                    }
                } else {
                    result.extend(
                        syntax::citation_list_spans(source, arg.span.clone())
                            .into_iter()
                            .map(|span| (true, span)),
                    );
                }
            }
        } else if syntax::warns_on_undefined_reference(&command.name)
            || syntax::has_separate_label_argument(&command.name)
        {
            if let Some(arg) = command
                .argument
                .as_ref()
                .filter(|_| !syntax::has_separate_label_argument(&command.name))
            {
                if syntax::is_single_reference(&command.name) {
                    if syntax::is_static_reference_key(&arg.value) {
                        result.push((false, arg.span.clone()));
                    }
                } else {
                    result.extend(
                        syntax::reference_list_spans(source, arg.span.clone())
                            .into_iter()
                            .map(|span| (false, span)),
                    );
                }
            }
            if let Some(arg) = syntax::separate_label_argument(source, command)
                .or_else(|| syntax::range_reference_end(source, command))
            {
                if syntax::is_static_reference_key(&arg.value) {
                    result.push((false, arg.span));
                }
            }
        }
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
    occurrences(doc)
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
        let candidates: BTreeSet<_> = ws
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
