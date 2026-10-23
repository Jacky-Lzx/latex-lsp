//! A fatal inheritance error can use only the history immediately before it.
use super::Context;
use crate::syntax;
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::*;

pub(crate) struct Record<'a> {
    key: &'a str,
    parent: &'a str,
    inheritance: &'static str,
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    let (body, inheritance) =
        if let Some(body) = message.strip_prefix("Circular XDATA inheritance between '") {
            (body, "xdata")
        } else {
            (
                message.strip_prefix("Circular inheritance between '")?,
                "crossref",
            )
        };
    let body = body.strip_suffix('\'')?;
    let mut parts = body.split("'<->'");
    let (parent, key) = (parts.next()?, parts.next()?);
    (parts.next().is_none()
        && syntax::is_static_citation_key(parent)
        && syntax::is_static_citation_key(key))
    .then_some(Record {
        key,
        parent,
        inheritance,
    })
}

impl Context {
    /// Call only for a recognized cycle record, before recording its fatal
    /// level. The copy belongs to that one message and cannot revive history.
    pub(crate) fn biber_cycle_history(&self) -> Option<Vec<PathBuf>> {
        self.biber_sources.inputs().map(<[_]>::to_vec)
    }

    pub(crate) fn biber_cycle_diagnostic(
        &mut self,
        root: &Path,
        warning: Record<'_>,
        inputs: Option<&[PathBuf]>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data = serde_json::json!({"key":warning.key,"parent":warning.parent,"inheritance":warning.inheritance,"unresolvedSource":true});
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String(
                "cyclic-bibliography-inheritance".into(),
            )),
            source: Some("latex-build".into()),
            message: format!("Biber: {}", message.trim()),
            ..Default::default()
        };
        let documents = inputs.and_then(|inputs| {
            inputs
                .iter()
                .map(|path| self.document(path).map(|parsed| (path, parsed)))
                .collect::<Option<Vec<_>>>()
        });
        if let Some(documents) = documents.filter(|documents| !documents.is_empty()) {
            let targets: Vec<_> = documents
                .iter()
                .flat_map(|(path, parsed)| {
                    parsed
                        .biber_locations()
                        .key(warning.key)
                        .iter()
                        .map(move |range| (*path, parsed, *range))
                })
                .take(16)
                .collect();
            let selected = targets.len() == 1;
            if let [(path, parsed, key)] = targets.as_slice() {
                uri = Url::from_file_path(path).ok()?;
                diagnostic.range = *key;
                if warning.inheritance == "crossref" {
                    let reference = parsed
                        .biber_reference_ranges(warning.key, "crossref", warning.parent)
                        .and_then(|(_, reference)| reference);
                    if let Some(reference) = reference {
                        diagnostic.range = reference;
                    } else {
                        data["unresolvedField"] = true.into();
                    }
                }
                data.as_object_mut()?.remove("unresolvedSource");
            }
            let parents = documents.iter().flat_map(|(path, parsed)| {
                parsed
                    .biber_locations()
                    .key(warning.parent)
                    .iter()
                    .map(move |range| (*path, *range))
            });
            let mut related = Vec::new();
            for (path, range) in parents.chain(
                targets
                    .iter()
                    .filter(|_| !selected)
                    .map(|(path, _, range)| (*path, *range)),
            ) {
                let location = Location::new(Url::from_file_path(path).ok()?, range);
                // The owner key and a precise crossref may both be useful for
                // self-cycles; omit only the exact primary diagnostic range.
                if location == Location::new(uri.clone(), diagnostic.range)
                    || related
                        .iter()
                        .any(|item: &DiagnosticRelatedInformation| item.location == location)
                {
                    continue;
                }
                related.push(DiagnosticRelatedInformation {
                    location,
                    message: "Bibliography entry in Biber's reported inheritance cycle".into(),
                });
                if related.len() == 16 {
                    break;
                }
            }
            if !related.is_empty() {
                diagnostic.related_information = Some(related);
            }
        }
        diagnostic.data = Some(data);
        Some((uri, diagnostic))
    }
}
