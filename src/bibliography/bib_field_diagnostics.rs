//! Duplicate assignments can be checked without evaluating bibliography values.
use crate::project::workspace::{DocumentKind, Workspace};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::*;

const CODE: &str = "repeated-bibliography-field";

pub(crate) fn diagnostics(workspace: &Workspace, uri: &Url) -> Vec<Diagnostic> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    if doc.kind != DocumentKind::Bibtex
        || !workspace
            .settings(uri)
            .diagnostics
            .duplicate_bibliography_fields
    {
        return vec![];
    }
    let reported: BTreeSet<_> = workspace
        .build_diagnostics
        .values()
        .filter_map(|project| project.get(uri))
        .flatten()
        .filter(|item| {
            item.source.as_deref() == Some("latex-build")
                && item.code == Some(NumberOrString::String(CODE.into()))
                && item.data.as_ref().is_some_and(|data| {
                    data.get("unresolvedField").is_none() && data.get("unresolvedSource").is_none()
                })
        })
        .map(|item| (item.range.start, item.range.end))
        .collect();
    let source = &doc.text.source;
    let mut byte = 0;
    let mut position = Position::new(0, 0);
    // A single long bibliography line may contain thousands of fields. Walk
    // source positions once rather than rescanning that line for every name.
    let mut at = |target| {
        for character in source[byte..target].chars() {
            if character == '\n' {
                position.line += 1;
                position.character = 0;
            } else {
                position.character += character.len_utf16() as u32;
            }
        }
        byte = target;
        position
    };
    let mut result = Vec::new();
    for entry in &doc.syntax.bibliography_completion.entries {
        let mut seen = BTreeMap::new();
        for field in &entry.fields {
            if !field.assigned || !field.value_valid {
                continue;
            }
            let range = Range::new(at(field.span.start), at(field.span.end));
            let Some(earlier) = seen.get(&field.name).copied() else {
                seen.insert(field.name.clone(), range);
                continue;
            };
            if reported.contains(&(range.start, range.end)) {
                continue;
            }
            result.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String(CODE.into())),
                source: Some("latex-lsp".into()),
                message: format!(
                    "Repeated bibliography field `{}`",
                    &source[field.span.clone()]
                ),
                related_information: Some(vec![DiagnosticRelatedInformation {
                    location: Location::new(uri.clone(), earlier),
                    message: format!(
                        "First assignment to `{}` in this bibliography entry",
                        field.name
                    ),
                }]),
                data: Some(serde_json::json!({"field":field.name})),
                ..Default::default()
            });
        }
    }
    result
}
