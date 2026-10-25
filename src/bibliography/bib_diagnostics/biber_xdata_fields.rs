//! Granular XDATA warnings name the referring field, not a raw list item.
//! The reader retains input field spelling even when the model folds it.
use super::{biber_relations, Context};
use crate::{bibliography::bib_strings, parser::syntax};
use lsp_types::*;
use std::path::Path;

pub(crate) struct Record<'a> {
    key: &'a str,
    field: &'a str,
    section: u32,
    reason: &'static str,
    parent: Option<&'a str>,
    source_field: Option<&'a str>,
    source_position: Option<&'a str>,
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    let (body, section) = biber_relations::section(message)?;
    if let Some(body) = body.strip_prefix("Entry '") {
        let (key, field) = body
            .strip_suffix("' that contains no source field")?
            .rsplit_once("' has XDATA reference from field '")?;
        return (syntax::is_static_citation_key(key) && bib_strings::identifier(field)).then_some(
            Record {
                key,
                field,
                section,
                reason: "unspecified-source-field",
                parent: None,
                source_field: None,
                source_position: None,
            },
        );
    }
    let (field, body) = body.strip_prefix("Field '")?.split_once("' in entry '")?;
    let (body, separator, reason) =
        if let Some(body) = body.strip_suffix(" and this field does not exist, not resolving") {
            (body, "' references XDATA field '", "missing-source-field")
        } else if let Some(body) = body.strip_suffix(" are not the same types, not resolving") {
            (
                body,
                "' which xdata references field '",
                "incompatible-field-types",
            )
        } else {
            (
                body.strip_suffix(" and this position does not exist, not resolving")?,
                "' references field '",
                "missing-position",
            )
        };
    let (key, target) = body.rsplit_once(separator)?;
    let target = target.strip_suffix('\'')?;
    let (source_field, parent, source_position) = if reason == "missing-position" {
        let (body, parent) = target.rsplit_once(" in entry '")?;
        let (source_field, position) = body.rsplit_once("' position ")?;
        if position.is_empty() || !position.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        (source_field, parent, Some(position))
    } else {
        let (source_field, parent) = target.rsplit_once("' in entry '")?;
        (source_field, parent, None)
    };
    (bib_strings::identifier(field)
        && bib_strings::identifier(source_field)
        && syntax::is_static_citation_key(key)
        && syntax::is_static_citation_key(parent))
    .then_some(Record {
        key,
        field,
        section,
        reason,
        parent: Some(parent),
        source_field: Some(source_field),
        source_position,
    })
}

impl Context {
    pub(crate) fn biber_xdata_field_warning(
        &mut self,
        root: &Path,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data = serde_json::json!({"key":warning.key,"field":warning.field,
            "section":warning.section,"reason":warning.reason,"unresolvedSource":true});
        if let Some(parent) = warning.parent {
            data["parent"] = parent.into();
        }
        if let Some(field) = warning.source_field {
            data["sourceField"] = field.into();
        }
        if let Some(position) = warning.source_position {
            data["sourcePosition"] = position.into();
        }
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String("invalid-bibliography-xdata".into())),
            source: Some("latex-build".into()),
            message: format!("Biber: {}", message.trim()),
            ..Default::default()
        };
        if let Some((path, parsed)) = self.biber_field_source(warning.key) {
            if let Some((key, value)) = parsed.biber_input_field_ranges(warning.key, warning.field)
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
