//! Missing Biber relations retain their native owner, field and refsection.
use super::Context;
use crate::parser::syntax;
use lsp_types::*;
use std::path::Path;

pub(crate) struct Record<'a> {
    key: &'a str,
    parent: &'a str,
    field: &'static str,
    section: u32,
}

pub(super) fn section(message: &str) -> Option<(&str, u32)> {
    let (body, section) = message.strip_suffix(')')?.rsplit_once(" (section ")?;
    if section.is_empty() || !section.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((body, section.parse().ok()?))
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    let (body, section) = section(message)?;
    let (kind, body) = body
        .strip_prefix("I didn't find a database entry for ")?
        .split_once(" '")?;
    let field = match kind {
        "crossref" => "crossref",
        "xref" => "xref",
        "related entry" => "related",
        "static set member" => "entryset",
        _ => return None,
    };
    let (parent, key) = body
        .strip_suffix("' - ignoring")?
        .rsplit_once("' in entry '")?;
    (syntax::is_static_citation_key(key) && syntax::is_static_citation_key(parent)).then_some(
        Record {
            key,
            parent,
            field,
            section,
        },
    )
}

impl Context {
    pub(crate) fn biber_relation_warning(
        &mut self,
        root: &Path,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data = serde_json::json!({
            "key":warning.key,"parent":warning.parent,"field":warning.field,
            "section":warning.section,"unresolvedSource":true
        });
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String(
                "missing-bibliography-relation".into(),
            )),
            source: Some("latex-build".into()),
            message: format!("Biber: {}", message.trim()),
            ..Default::default()
        };
        if let Some((path, parsed)) = self.biber_field_source(warning.key) {
            if let Some((key, reference)) =
                parsed.biber_reference_ranges(warning.key, warning.field, warning.parent)
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
