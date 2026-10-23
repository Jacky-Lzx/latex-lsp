//! Biber key warnings name a current database and may involve earlier inputs.
//! Canonical matching retains case distinctions and ambiguous declarations.
use super::{Context, Parsed};
use crate::{recorder, recorder::BibliographyPaths, syntax};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use tower_lsp::lsp_types::*;

const MAX_RELATED: usize = 16;

const CASE_CODE: &str = "bibliography-key-case-mismatch";
const DUPLICATE_CODE: &str = "duplicate-bibliography-key";

/// Biber's BibTeX reader canonically decomposes keys in native messages.
/// This is backend-specific: general bibliography navigation keeps raw keys.
pub(super) fn key_name(name: &str) -> String {
    icu_normalizer::DecomposingNormalizerBorrowed::new_nfd()
        .normalize(name)
        .into_owned()
}

/// Biber stores entry types and data-model field identifiers using Perl's fc.
/// Keep this separate from case-sensitive keys and raw input field spellings.
pub(super) fn model_name(name: &str) -> String {
    let decomposed = key_name(name);
    key_name(&icu_casemap::CaseMapper::new().fold_string(&decomposed))
}

pub(crate) struct Record<'a> {
    key: &'a str,
    other: Option<&'a str>,
    /// Native quoting delimits the complete filename, including apostrophes.
    file: &'a str,
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    if let Some(body) = message.strip_prefix("Duplicate entry key: '") {
        let (key, tail) = body.split_once("' in file '")?;
        let file = tail
            .strip_suffix(", skipping ...")
            .unwrap_or(tail)
            .strip_suffix('\'')?;
        return (syntax::is_static_citation_key(key)
            && !file.is_empty()
            && !file.chars().any(char::is_control))
        .then_some(Record {
            key,
            other: None,
            file,
        });
    }
    let body = message.strip_prefix("Possible typo (case mismatch) between datasource keys: '")?;
    let (key, tail) = body.split_once("' and '")?;
    let (other, file) = tail.split_once("' in file ")?;
    let name = file.strip_prefix('\'')?.strip_suffix('\'')?;
    (key != other
        && syntax::is_static_citation_key(key)
        && syntax::is_static_citation_key(other)
        && !name.is_empty()
        && !name.chars().any(char::is_control))
    .then_some(Record {
        key,
        other: Some(other),
        file: name,
    })
}

pub(super) struct Sources {
    inputs: Vec<PathBuf>,
    complete: bool,
}

impl Sources {
    pub(super) fn inputs(&self) -> Option<&[PathBuf]> {
        (self.complete && !self.inputs.is_empty()).then_some(&self.inputs)
    }

    pub(super) fn new(log: &str) -> Self {
        Self {
            inputs: Vec::new(),
            complete: !crate::output::truncated(log),
        }
    }

    fn record(&mut self, paths: &BibliographyPaths<'_>, line: &str) {
        let Some((level, message)) = recorder::biber_record(line) else {
            return;
        };
        if level == "ERROR" {
            // These exact name failures skip one entry, then reading continues.
            // Other recovery can leave editor-indexed entries unread by Biber.
            self.complete = false;
            return;
        }
        let Some(name) = recorder::bibliography_database_source(line) else {
            if level == "INFO" && message.starts_with("Found BibTeX data source ") {
                self.complete = false;
            }
            return;
        };
        if name != name.trim().trim_matches('"') {
            self.complete = false;
            return;
        }
        let Some(path) = paths.resolve(name) else {
            self.complete = false;
            return;
        };
        if self.inputs.contains(&path) {
            return;
        }
        if self.inputs.len() >= super::MAX_INPUTS {
            self.complete = false;
            return;
        }
        self.inputs.push(path);
    }
}

/// Retain at most the related-location limit for each canonical, case-sensitive key.
/// The document and entry count are bounded by Context's source cache.
pub(super) struct Locations {
    pub(super) first_line: Range,
    keys: std::collections::BTreeMap<String, Vec<Range>>,
}

impl Locations {
    fn new(parsed: &Parsed) -> Self {
        let source = &parsed.text.source;
        let end = source.find('\n').unwrap_or(source.len());
        let end = end - usize::from(source[..end].ends_with('\r'));
        let mut keys = std::collections::BTreeMap::<String, Vec<Range>>::new();
        // Citation declarations are emitted in source order. Convert positions
        // with one forward traversal, including intervening Unicode values.
        let mut byte = 0;
        let mut position = Position::new(0, 0);
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
        for entry in &parsed.syntax.citations {
            let locations = keys.entry(key_name(&entry.name)).or_default();
            if locations.len() < MAX_RELATED {
                locations.push(Range::new(at(entry.span.start), at(entry.span.end)));
            }
        }
        Self {
            first_line: parsed.text.range(0, end),
            keys,
        }
    }

    pub(super) fn key(&self, key: &str) -> &[Range] {
        self.keys
            .get(&key_name(key))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// A canonical collision remains ambiguous even when one raw spelling
    /// matches the reported key. The cap retains at least two candidates.
    pub(super) fn unique(&self, key: &str) -> Option<Range> {
        match self.key(key) {
            [range] => Some(*range),
            _ => None,
        }
    }
}

impl Parsed {
    pub(super) fn biber_locations(&self) -> &Locations {
        self.biber_locations.get_or_init(|| Locations::new(self))
    }
}

pub(super) fn source(paths: &BibliographyPaths<'_>, file: &str) -> Option<PathBuf> {
    (file == file.trim().trim_matches('"'))
        .then(|| match paths.recorded_bibliography_source(file) {
            Some(source) if source.selected => source.path,
            Some(source) => source.path.or_else(|| paths.resolve(file)),
            None => file
                .ends_with(".bib")
                .then(|| paths.resolve(file))
                .flatten(),
        })
        .flatten()
}

impl Context {
    pub(crate) fn biber_key_record(&mut self, paths: &BibliographyPaths<'_>, line: &str) {
        self.biber_sources.record(paths, line);
    }

    pub(crate) fn biber_key_warning(
        &mut self,
        root: &Path,
        paths: &BibliographyPaths<'_>,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data = serde_json::json!({"key":warning.key,"unresolvedSource":true});
        if let Some(other) = warning.other {
            data["otherKey"] = other.into();
        }
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            code: Some(NumberOrString::String(
                if warning.other.is_some() {
                    CASE_CODE
                } else {
                    DUPLICATE_CODE
                }
                .into(),
            )),
            source: Some("latex-build".into()),
            message: format!("Biber: {}", message.trim()),
            data: Some(data.clone()),
            ..Default::default()
        };
        // Preserve the exact native filename. General prose path parsing can
        // mistake quote/space fragments inside this delimited name for fields.
        let path = source(paths, warning.file);
        let Some(path) = path else {
            return Some((uri, diagnostic));
        };
        let Some(parsed) = self.document(&path) else {
            return Some((uri, diagnostic));
        };
        uri = Url::from_file_path(&path).ok()?;
        let locations = parsed.biber_locations();
        data.as_object_mut()?.remove("unresolvedSource");
        let primary_range = locations.unique(warning.key);
        let primary_path = path.clone();
        diagnostic.range = primary_range.unwrap_or(locations.first_line);
        if primary_range.is_none() {
            data["unresolvedEntry"] = true.into();
        }
        let inputs = if self.biber_sources.complete {
            self.biber_sources.inputs.clone()
        } else {
            Vec::new()
        };
        let mut seen = BTreeSet::new();
        let mut related = Vec::new();
        let related_key = warning.other.unwrap_or(warning.key);
        for path in std::iter::once(path).chain(inputs) {
            if !seen.insert(path.clone()) {
                continue;
            }
            let Some(parsed) = self.document(&path) else {
                continue;
            };
            let Ok(uri) = Url::from_file_path(&path) else {
                continue;
            };
            for range in parsed
                .biber_locations()
                .key(related_key)
                .iter()
                .take(MAX_RELATED - related.len())
            {
                // A unique current declaration is already the primary range.
                // Ambiguous declarations remain independent related candidates.
                if warning.other.is_none() && path == primary_path && primary_range == Some(*range)
                {
                    continue;
                }
                related.push(DiagnosticRelatedInformation {
                    location: Location::new(uri.clone(), *range),
                    message: if warning.other.is_some() {
                        format!("Matching bibliography key with Biber's other reported spelling `{related_key}`")
                    } else {
                        format!("Bibliography entry matching Biber's reported duplicate key `{related_key}`")
                    },
                });
            }
            if related.len() == MAX_RELATED {
                break;
            }
        }
        if !related.is_empty() {
            diagnostic.related_information = Some(related);
        } else if warning.other.is_some() {
            data["unresolvedOtherKey"] = true.into();
        }
        diagnostic.data = Some(data);
        Some((uri, diagnostic))
    }
}
