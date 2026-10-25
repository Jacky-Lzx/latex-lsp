//! Native XDATA messages identify an owner and target, without a database.
//! Only unique raw literal references can supply a narrower source range.
use super::Context;
use crate::parser::syntax;
use lsp_types::*;
use std::path::Path;

pub(crate) struct Record<'a> {
    key: &'a str,
    parent: &'a str,
    section: u32,
    reason: &'static str,
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    let (body, section) = super::biber_relations::section(message)?;
    let (key, parent, reason) = if let Some(body) = body
        .strip_prefix("I didn't find a database entry for xdata entry '")
        .and_then(|body| body.strip_suffix("' - ignoring"))
    {
        let (parent, key) = body.rsplit_once("' in entry '")?;
        (key, parent, "missing-entry")
    } else {
        let body = body.strip_prefix("Entry '")?;
        let (body, reason) =
            if let Some(body) = body.strip_suffix("' which does not exist, not resolving") {
                (body, "missing-entry")
            } else {
                (
                    body.strip_suffix("' which is not an XDATA entry, not resolving")?,
                    "non-xdata-entry",
                )
            };
        let (key, parent) = body.rsplit_once("' references XDATA entry '")?;
        (key, parent, reason)
    };
    (syntax::is_static_citation_key(key) && syntax::is_static_citation_key(parent)).then_some(
        Record {
            key,
            parent,
            section,
            reason,
        },
    )
}

impl Context {
    pub(crate) fn biber_xdata_warning(
        &mut self,
        root: &Path,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data = serde_json::json!({
            "key":warning.key,"parent":warning.parent,"field":"xdata",
            "section":warning.section,"reason":warning.reason,"unresolvedSource":true
        });
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String("invalid-bibliography-xdata".into())),
            source: Some("latex-build".into()),
            message: format!("Biber: {}", message.trim()),
            ..Default::default()
        };
        if let Some((path, parsed)) = self.biber_field_source(warning.key) {
            if let Some((key, reference)) =
                parsed.biber_reference_ranges(warning.key, "xdata", warning.parent)
            {
                uri = Url::from_file_path(path).ok()?;
                diagnostic.range = reference.unwrap_or(key);
                data.as_object_mut()?.remove("unresolvedSource");
                if reference.is_none() {
                    data["unresolvedField"] = true.into();
                }
            }
        }
        diagnostic.data = Some(data);
        Some((uri, diagnostic))
    }
}
