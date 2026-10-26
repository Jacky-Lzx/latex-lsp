//! Conservative, project-scoped references that do not require a compiler run.
use crate::{
    parser::syntax,
    project::workspace::{DocumentKind, Workspace},
};
use lsp_types::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

fn occurrences(
    ws: &Workspace,
    uri: &Url,
    resources: Option<&crate::bibliography::bib_section_keys::ResourceMap>,
    sources: Option<&crate::bibliography::bib_section_references::SourceStatuses>,
) -> Vec<(bool, Range<usize>)> {
    occurrences_for(ws, uri, false, false, resources, sources)
}

pub(crate) fn hint_occurrences(ws: &Workspace, uri: &Url) -> Vec<(bool, Range<usize>)> {
    occurrences_for(ws, uri, true, false, None, None)
}

pub(crate) fn section_hint_occurrences(ws: &Workspace, uri: &Url) -> Vec<(bool, Range<usize>)> {
    occurrences_for(ws, uri, true, true, None, None)
}

fn occurrences_for(
    ws: &Workspace,
    uri: &Url,
    printing: bool,
    section_hints: bool,
    resources: Option<&crate::bibliography::bib_section_keys::ResourceMap>,
    sources: Option<&crate::bibliography::bib_section_references::SourceStatuses>,
) -> Vec<(bool, Range<usize>)> {
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
    let signatures = crate::symbols::commands::CommandSignatures::new(
        docs.iter()
            .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
    );
    let selected_resources =
        crate::bibliography::bib_sections::selects_resources(&docs, &signatures);
    let source = &doc.text.source;
    let operands = signatures.editing_argument_tokens(source, &doc.syntax);
    let mut opaque = crate::symbols::commands::definition_data_spans(source, &doc.syntax);
    opaque.extend(crate::symbols::expl3::math_data_spans(doc));
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
        let resolved = crate::symbols::reference_calls::named_command(
            source,
            &doc.syntax,
            index,
            &signatures,
            &operands,
            &declarations,
        );
        let command = resolved.as_ref().unwrap_or(command);
        if !signatures.command_unchanged(&command.name) || printing && command.name == "nocite" {
            continue;
        }
        result.extend(
            crate::symbols::reference_calls::keys(source, command, true)
                .into_iter()
                .filter(|(citation, span)| {
                    !*citation
                        || !selected_resources
                        || printing && section_hints
                        || !printing
                            && sources.map_or_else(
                                || {
                                    resources.is_some_and(|resources| {
                                        resources.contains_key(&(span.start, span.end))
                                    })
                                },
                                |sources| sources.contains_key(&(span.start, span.end)),
                            )
                }),
        );
    }
    if !printing {
        if let Some(sources) = sources {
            result.extend(sources.keys().map(|&(start, end)| (true, start..end)));
        }
    }
    result.sort_by_key(|(citation, span)| (span.start, span.end, *citation));
    result.dedup();
    result
}

fn citation_contexts(
    ws: &Workspace,
    uri: &Url,
) -> (
    Option<crate::bibliography::bib_section_keys::ResourceMap>,
    Option<crate::bibliography::bib_section_references::SourceStatuses>,
) {
    if !ws.settings(uri).diagnostics.undefined_citations {
        return (None, None);
    }
    let sources = crate::bibliography::bib_section_references::source_statuses(ws, uri);
    let resources = if sources.is_none() {
        crate::bibliography::bib_section_keys::occurrence_resources(ws, uri)
    } else {
        None
    };
    (resources, sources)
}

pub(crate) fn diagnostics(ws: &Workspace, uri: &Url) -> Vec<Diagnostic> {
    if !ws
        .documents
        .get(uri)
        .is_some_and(|doc| doc.kind == DocumentKind::Latex)
    {
        return vec![];
    }
    let (resources, sources) = citation_contexts(ws, uri);
    diagnostics_for(ws, uri, resources.as_ref(), sources.as_ref())
}
fn diagnostics_for(
    ws: &Workspace,
    uri: &Url,
    resources: Option<&crate::bibliography::bib_section_keys::ResourceMap>,
    sources: Option<&crate::bibliography::bib_section_references::SourceStatuses>,
) -> Vec<Diagnostic> {
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    let settings = ws.settings(uri);
    let config = &settings.diagnostics;
    if !config.undefined_references && !config.undefined_citations {
        return vec![];
    }
    let occurrences = occurrences(ws, uri, resources, sources);
    if occurrences.is_empty() {
        return vec![];
    }
    let scope = ws.symbol_scope(uri);
    let declarations = crate::symbols::label_declarations::Index::new(ws, uri);
    let mut labels = BTreeSet::new();
    let mut citations = BTreeSet::new();
    let mut citation_files = BTreeMap::<&str, BTreeSet<&Url>>::new();
    for (uri, doc) in &ws.documents {
        if !scope.contains(uri) {
            continue;
        }
        labels.extend(
            doc.syntax
                .labels
                .iter()
                .filter(|entry| declarations.contains(uri, entry))
                .map(|entry| entry.name.as_str()),
        );
        let names = doc
            .syntax
            .citations
            .iter()
            .chain(&doc.syntax.bibliography_aliases)
            .map(|entry| entry.name.as_str());
        if resources.is_some() {
            for name in names {
                citation_files.entry(name).or_default().insert(uri);
            }
        } else {
            citations.extend(names);
        }
    }
    let auxiliary = crate::compilation::auxiliary::labels(ws, uri);
    labels.extend(
        auxiliary
            .iter()
            .filter(|(_, info)| info.external)
            .map(|(name, _)| name.as_str()),
    );
    occurrences
        .into_iter()
        .filter_map(|(citation, span)| {
            let name = &doc.text.source[span.clone()];
            let (enabled, known, kind) = if citation {
                let known = if let Some(sources) = sources {
                    *sources.get(&(span.start, span.end))?
                } else if let Some(resources) = resources {
                    let files = resources.get(&(span.start, span.end))?;
                    citation_files.get(name).is_some_and(|definitions| {
                        definitions.iter().any(|uri| files.contains(*uri))
                    })
                } else {
                    citations.contains(name)
                };
                (config.undefined_citations, known, "citation")
            } else {
                (
                    config.undefined_references,
                    labels.contains(name),
                    "reference",
                )
            };
            if !enabled || name == "*" || known {
                return None;
            }
            // An identical compiler-confirmed warning already carries more context.
            let range = doc.text.range(span.start, span.end);
            if ws
                .project
                .build_diagnostics
                .values()
                .filter_map(|project| project.get(uri))
                .flatten()
                .any(|diag| {
                    diag.range == range
                        && crate::compilation::build_log::undefined_reference_key(&diag.message)
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
    range: lsp_types::Range,
    requested: &[Diagnostic],
) -> Vec<syntax::QuickFix> {
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    let scope = ws.symbol_scope(uri);
    let declarations = crate::symbols::label_declarations::Index::new(ws, uri);
    let auxiliary = crate::compilation::auxiliary::labels(ws, uri);
    let mut result = Vec::new();
    let (section_resources, sources) = citation_contexts(ws, uri);
    for diagnostic in diagnostics_for(ws, uri, section_resources.as_ref(), sources.as_ref())
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
        if citation && sources.is_some() {
            result.extend(crate::bibliography::bib_source_citations::repairs(
                ws,
                uri,
                &diagnostic,
            ));
            continue;
        }
        let citation_scope = if citation {
            if let Some(resources) = &section_resources {
                let (Some(start), Some(end)) = (
                    doc.text.offset(diagnostic.range.start),
                    doc.text.offset(diagnostic.range.end),
                ) else {
                    continue;
                };
                let Some(resources) = resources.get(&(start, end)) else {
                    continue;
                };
                Some(resources)
            } else {
                None
            }
        } else {
            None
        };
        let length = key.chars().count();
        if !(3..=128).contains(&length) {
            continue;
        }
        let mut candidates: BTreeSet<_> = ws
            .documents
            .iter()
            .filter(|(uri, _)| {
                scope.contains(*uri)
                    && citation_scope.is_none_or(|resources| resources.contains(*uri))
            })
            .flat_map(|(uri, doc)| {
                if citation {
                    doc.syntax
                        .citations
                        .iter()
                        .chain(&doc.syntax.bibliography_aliases)
                        .collect::<Vec<_>>()
                } else {
                    doc.syntax
                        .labels
                        .iter()
                        .filter(|entry| declarations.contains(uri, entry))
                        .collect()
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
                crate::editing::spelling_fixes::distance(key, name, if length < 8 { 1 } else { 2 })
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
