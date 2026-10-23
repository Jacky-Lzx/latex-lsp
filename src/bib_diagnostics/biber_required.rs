//! Required fields are defined by Biber's active data model, including groups
//! where any one field suffices. No missing value can be inferred from the log.
use super::{biber_keys, Context};
use crate::{bib_strings, recorder::BibliographyPaths, syntax};
use std::path::Path;
use tower_lsp::lsp_types::*;

enum Fields<'a> {
    Single(&'a str),
    OneOf(&'a str),
}

pub(crate) struct Record<'a> {
    key: &'a str,
    file: &'a str,
    fields: Fields<'a>,
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    let body = message.strip_prefix("Datamodel: Entry '")?;
    let (key, body) = body.split_once("' (")?;
    let (file, fields) = if let Some((file, field)) = body
        .strip_suffix('\'')
        .and_then(|body| body.rsplit_once("): Missing mandatory field '"))
    {
        bib_strings::identifier(field).then_some((file, Fields::Single(field)))?
    } else {
        let (file, fields) = body
            .strip_suffix("' must be defined")?
            .rsplit_once("): Missing mandatory field - one of '")?;
        fields
            .split(", ")
            .all(bib_strings::identifier)
            .then_some((file, Fields::OneOf(fields)))?
    };
    (syntax::is_static_citation_key(key) && !file.is_empty() && !file.chars().any(char::is_control))
        .then_some(Record { key, file, fields })
}

impl Context {
    pub(crate) fn biber_required_warning(
        &mut self,
        root: &Path,
        paths: &BibliographyPaths<'_>,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data = serde_json::json!({"key":warning.key,"unresolvedSource":true});
        match warning.fields {
            Fields::Single(field) => data["field"] = field.into(),
            Fields::OneOf(fields) => {
                data["fields"] = fields.split(", ").collect::<Vec<_>>().into();
            }
        }
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String("missing-bibliography-field".into())),
            source: Some("latex-build".into()),
            message: format!("Biber: {}", message.trim()),
            ..Default::default()
        };
        if let Some((path, parsed)) = biber_keys::source(paths, warning.file)
            .and_then(|path| self.document(&path).map(|parsed| (path, parsed)))
        {
            uri = Url::from_file_path(path).ok()?;
            data.as_object_mut()?.remove("unresolvedSource");
            let locations = parsed.biber_locations();
            diagnostic.range = if let Some(range) = locations.unique(warning.key) {
                range
            } else {
                data["unresolvedEntry"] = true.into();
                locations.first_line
            };
        }
        diagnostic.data = Some(data);
        Some((uri, diagnostic))
    }
}
