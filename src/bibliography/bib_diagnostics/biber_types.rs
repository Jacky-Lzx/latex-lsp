//! The active Biber data model decides whether a type is valid. Source maps
//! may change it, so highlight the raw type only when its full fold matches.
use super::{biber_keys, Context};
use crate::{bibliography::bib_strings, compilation::recorder::BibliographyPaths, parser::syntax};
use lsp_types::*;
use std::path::Path;

pub(crate) struct Record<'a> {
    key: &'a str,
    file: &'a str,
    kind: &'a str,
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    let body = message
        .strip_prefix("Datamodel: Entry '")?
        .strip_suffix("' - defaulting to 'misc'")?;
    let (key, body) = body.split_once("' (")?;
    let (file, kind) = body.rsplit_once("): Invalid entry type '")?;
    (syntax::is_static_citation_key(key)
        && bib_strings::identifier(kind)
        && !file.is_empty()
        && !file.chars().any(char::is_control))
    .then_some(Record { key, file, kind })
}

impl Context {
    pub(crate) fn biber_type_warning(
        &mut self,
        root: &Path,
        paths: &BibliographyPaths<'_>,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data =
            serde_json::json!({"key":warning.key,"entryType":warning.kind,"unresolvedSource":true});
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String(
                "invalid-bibliography-entry-type".into(),
            )),
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
            diagnostic.range = if let Some(key) = locations.unique(warning.key) {
                parsed
                    .biber_type_range(warning.key, warning.kind)
                    .unwrap_or_else(|| {
                        data["unresolvedType"] = true.into();
                        key
                    })
            } else {
                data["unresolvedEntry"] = true.into();
                locations.first_line
            };
        }
        diagnostic.data = Some(data);
        Some((uri, diagnostic))
    }
}
