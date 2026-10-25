//! Extended-name warnings identify the model field, not a raw name token.
use super::Context;
use crate::{bibliography::bib_strings, parser::syntax};
use lsp_types::*;
use std::path::Path;

pub(crate) struct Record<'a> {
    key: &'a str,
    field: &'a str,
    namepart: &'a str,
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    let body = message
        .strip_prefix("Invalid namepart '")?
        .strip_suffix("', ignoring")?;
    let (body, key) = body.rsplit_once("' in entry '")?;
    let (namepart, field) = body.rsplit_once("' found in extended name format name '")?;
    (syntax::is_static_citation_key(key)
        && bib_strings::identifier(field)
        && !namepart.is_empty()
        && !namepart.chars().any(char::is_control))
    .then_some(Record {
        key,
        field,
        namepart,
    })
}

impl Context {
    pub(crate) fn biber_namepart_warning(
        &mut self,
        root: &Path,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data = serde_json::json!({
            "key":warning.key,"field":warning.field,"namepart":warning.namepart,
            "unresolvedSource":true
        });
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String(
                "invalid-bibliography-namepart".into(),
            )),
            source: Some("latex-build".into()),
            message: format!("Biber: {}", message.trim()),
            ..Default::default()
        };
        if let Some((path, parsed)) = self.biber_field_source(warning.key) {
            if let Some((key, value)) = parsed.biber_model_field_ranges(warning.key, warning.field)
            {
                uri = Url::from_file_path(path).ok()?;
                diagnostic.range = value.unwrap_or(key);
                data.as_object_mut()?.remove("unresolvedSource");
                if value.is_none() {
                    data["unresolvedField"] = true.into();
                }
            }
        }
        diagnostic.data = Some(data);
        Some((uri, diagnostic))
    }
}
