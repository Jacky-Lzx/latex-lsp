//! Recoverable name errors skip an entry without losing the reader's inputs.
//! Names may be decoded or supplied by custom model fields; mark only the key.
use super::Context;
use crate::parser::syntax;
use lsp_types::*;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
enum Reason {
    Commas,
    Consecutive,
    Empty,
}

impl Reason {
    fn code(self) -> &'static str {
        match self {
            Self::Commas => "too-many-commas",
            Self::Consecutive => "consecutive-commas",
            Self::Empty => "empty-name",
        }
    }
}

pub(crate) struct Record<'a> {
    key: &'a str,
    name: Option<&'a str>,
    reason: Reason,
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    if let Some(body) = message.strip_prefix("Name in key '") {
        let (key, other) = body
            .strip_suffix('\'')?
            .rsplit_once("' is empty (probably consecutive 'and'): skipping entry '")?;
        return (key == other && syntax::is_static_citation_key(key)).then_some(Record {
            key,
            name: None,
            reason: Reason::Empty,
        });
    }
    let body = message.strip_prefix("Name \"")?.strip_suffix('\'')?;
    for (separator, reason) in [
        ("\" has too many commas, skipping entry '", Reason::Commas),
        (
            "\" is malformed (consecutive commas): skipping entry '",
            Reason::Consecutive,
        ),
    ] {
        if let Some((name, key)) = body.rsplit_once(separator) {
            if !name.is_empty()
                && !name.chars().any(char::is_control)
                && syntax::is_static_citation_key(key)
            {
                return Some(Record {
                    key,
                    name: Some(name),
                    reason,
                });
            }
        }
    }
    None
}

impl Context {
    fn biber_name_location(&mut self, key: &str) -> Option<(PathBuf, Range)> {
        let inputs = self.biber_sources.inputs()?.to_vec();
        let mut selected = None;
        for path in inputs {
            let parsed = self.document(&path)?;
            match parsed.biber_locations().key(key) {
                [] => {}
                [range] => {
                    if selected.is_some() {
                        return None;
                    }
                    selected = Some((path, *range));
                }
                _ => return None,
            }
        }
        selected
    }

    pub(crate) fn biber_name_warning(
        &mut self,
        root: &Path,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data = serde_json::json!({"key":warning.key,"reason":warning.reason.code(),"unresolvedSource":true});
        if let Some(name) = warning.name {
            data["name"] = name.into();
        }
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String("malformed-bibliography-name".into())),
            source: Some("latex-build".into()),
            message: format!("Biber: {}", message.trim()),
            ..Default::default()
        };
        if let Some((path, range)) = self.biber_name_location(warning.key) {
            uri = Url::from_file_path(path).ok()?;
            diagnostic.range = range;
            data.as_object_mut()?.remove("unresolvedSource");
        }
        diagnostic.data = Some(data);
        Some((uri, diagnostic))
    }
}
