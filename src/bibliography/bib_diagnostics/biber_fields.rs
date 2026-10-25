//! Native Biber field validation supplies a database or relies on prior inputs.
//! Source fields remain unexpanded; inherited and ambiguous fields use the key.
use super::{biber_keys, Context, Parsed};
use crate::{bibliography::bib_strings, compilation::recorder::BibliographyPaths, parser::syntax};
use biber_keys::key_name;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use tower_lsp::lsp_types::*;

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Name,
    Value,
    Overwritten,
    SetName,
    SetMissing,
}

impl Kind {
    fn code(self) -> &'static str {
        match self {
            Self::Name => "invalid-bibliography-field",
            Self::Value => "invalid-bibliography-field-value",
            Self::Overwritten => "overwritten-bibliography-field",
            Self::SetName | Self::SetMissing => "invalid-bibliography-set",
        }
    }
}

pub(crate) struct Record<'a> {
    key: &'a str,
    source: Source<'a>,
    field: &'a str,
    kind: Kind,
    spelling: Spelling,
}

#[derive(Clone, Copy)]
enum Spelling {
    Input,
    Model,
}

enum Source<'a> {
    Named(Option<&'a str>),
    History,
}

fn set(message: &str) -> Option<Record<'_>> {
    let (key, field, kind) = if let Some(key) = message
        .strip_prefix("Set entry '")
        .and_then(|body| body.strip_suffix("' has no entryset field, ignoring"))
    {
        (key, "entryset", Kind::SetMissing)
    } else {
        let (body, field) = if let Some(body) = message.strip_prefix(
            "Field 'crossref' is no longer needed in set entries in Biber - ignoring in entry '",
        ) {
            (body, "crossref")
        } else {
            (
                message.strip_prefix("Field 'entryset' is no longer needed in set member entries in Biber - ignoring in entry '")?,
                "entryset",
            )
        };
        (body.strip_suffix('\'')?, field, Kind::SetName)
    };
    syntax::is_static_citation_key(key).then_some(Record {
        key,
        source: Source::History,
        field,
        kind,
        spelling: Spelling::Model,
    })
}

fn overwritten(message: &str) -> Option<Record<'_>> {
    let (body, key) = message
        .strip_prefix("Overwriting field '")?
        .strip_suffix('\'')?
        .rsplit_once(" for entry '")?;
    let field = match body {
        "year' with year value from field 'date'" => "year",
        "month' with month value from field 'date'" => "month",
        _ => return None,
    };
    syntax::is_static_citation_key(key).then_some(Record {
        key,
        source: Source::History,
        field,
        kind: Kind::Overwritten,
        spelling: Spelling::Model,
    })
}

fn unqualified(message: &str) -> Option<Record<'_>> {
    if let Some(body) = message.strip_prefix("Range field '") {
        let (field, key) = body
            .strip_suffix("' is malformed, falling back to literal")?
            .rsplit_once("' in entry '")?;
        return (bib_strings::identifier(field) && syntax::is_static_citation_key(key)).then_some(
            Record {
                key,
                source: Source::History,
                field,
                kind: Kind::Value,
                spelling: Spelling::Input,
            },
        );
    }
    let (body, field) = if let Some(body) = message.strip_prefix("legacy ") {
        let (field, body) = body.split_once(" field '")?;
        if !matches!(field, "year" | "month") {
            return None;
        }
        (
            body.strip_suffix("' is not an integer - this will probably not sort properly.")?,
            field,
        )
    } else {
        (
            message.strip_prefix("ISBN '")?.strip_suffix(
                "' is invalid - run biber with '--validate_datamodel' for details.",
            )?,
            "isbn",
        )
    };
    // Values are unescaped native text and can contain apparent entry fields.
    // Only the rightmost delimiter immediately before the fixed suffix is a key.
    let (value, key) = body.rsplit_once("' in entry '")?;
    (syntax::is_static_citation_key(key) && !value.chars().any(char::is_control)).then_some(
        Record {
            key,
            source: Source::History,
            field,
            kind: Kind::Value,
            spelling: Spelling::Model,
        },
    )
}

fn identifier_suffix<'a>(body: &'a str, separator: &str) -> Option<(&'a str, &'a str)> {
    let (before, identifier) = body.strip_suffix('\'')?.rsplit_once(separator)?;
    bib_strings::identifier(identifier).then_some((before, identifier))
}

/// Delimit from the fixed native suffix, preserving punctuation in filenames.
fn datamodel(body: &str) -> Option<(&str, &str, Kind, Spelling)> {
    if let Some(body) = body.strip_suffix("' invalid in data model - ignoring") {
        let (file, field) = body.rsplit_once("): Field '")?;
        return Some((file, field, Kind::Name, Spelling::Input));
    }
    if let Some((body, _)) = identifier_suffix(body, "' for entrytype '") {
        let (file, field) = body.rsplit_once("): Invalid field '")?;
        return Some((file, field, Kind::Name, Spelling::Model));
    }
    for prefix in [
        "): Invalid ISBN in value of field '",
        "): Invalid ISSN in value of field '",
        "): Invalid ISMN in value of field '",
        "): Invalid value (pattern match fails) for field '",
    ] {
        if let Some((file, field)) = body.strip_suffix('\'').and_then(|b| b.rsplit_once(prefix)) {
            return Some((file, field, Kind::Value, Spelling::Model));
        }
    }
    let body = body.strip_suffix("' - ignoring field")?;
    let (body, constraint) = body.rsplit_once("' must be ")?;
    let valid = if let Some(datatype) = constraint.strip_prefix("datatype '") {
        bib_strings::identifier(datatype)
    } else {
        constraint.strip_prefix('\'').is_some_and(|value| {
            value
                .strip_prefix(">=")
                .or_else(|| value.strip_prefix("<="))
                .filter(|number| !number.chars().any(char::is_whitespace))
                .and_then(|number| number.parse::<f64>().ok())
                .is_some_and(f64::is_finite)
        })
    };
    let (file, field) = body.rsplit_once("): Invalid value of field '")?;
    valid.then_some((file, field, Kind::Value, Spelling::Model))
}

fn date(body: &str) -> Option<(Option<&str>, &str, Kind)> {
    let body = body.strip_suffix("' - ignoring")?;
    for separator in ["' of date field '", "' of end date field '"] {
        if let Some((body, field)) = body.rsplit_once(separator) {
            if !bib_strings::identifier(field) {
                continue;
            }
            let (file, value) = body.rsplit_once("): Invalid format '")?;
            // Both the filename and the invalid value are native unescaped
            // text. Multiple boundaries cannot identify the source reliably.
            let file = (!file.contains("): Invalid format '")).then_some(file);
            return (!value.chars().any(char::is_control)).then_some((file, field, Kind::Value));
        }
    }
    None
}

pub(crate) fn record(message: &str) -> Option<Record<'_>> {
    if let Some(record) = set(message)
        .or_else(|| overwritten(message))
        .or_else(|| unqualified(message))
    {
        return Some(record);
    }
    let (body, datamodel_message) = if let Some(body) = message.strip_prefix("Datamodel: Entry '") {
        (body, true)
    } else {
        (message.strip_prefix("Entry '")?, false)
    };
    let (key, body) = body.split_once("' (")?;
    let (file, field, kind, spelling) = if datamodel_message {
        datamodel(body).map(|(file, field, kind, spelling)| (Some(file), field, kind, spelling))
    } else {
        date(body).map(|(file, field, kind)| (file, field, kind, Spelling::Input))
    }?;
    (syntax::is_static_citation_key(key)
        && bib_strings::identifier(field)
        && file.is_none_or(|file| !file.is_empty() && !file.chars().any(char::is_control)))
    .then_some(Record {
        key,
        source: Source::Named(file),
        field,
        kind,
        spelling,
    })
}

// Native unknown-field and date messages interpolate the input spelling.
// Biber folds ASCII on that composed spelling, before NFD log normalization.
fn field_name(name: &str) -> String {
    let composed = icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(name);
    key_name(&composed.to_ascii_lowercase())
}

#[derive(Clone, Copy)]
struct Field {
    name: Range,
    value: Range,
}

struct Entry {
    key: Range,
    kind: Option<SourceType>,
    fields: BTreeMap<String, Option<Field>>,
    model_fields: BTreeMap<String, Option<Field>>,
    relations: BTreeMap<String, BTreeMap<String, Option<Range>>>,
}

struct SourceType {
    name: String,
    range: Range,
}

/// Built once per bounded source, with normalized ambiguities explicit.
pub(super) struct Locations {
    first_line: Range,
    entries: BTreeMap<String, Entry>,
    ambiguous: BTreeSet<String>,
}

impl Locations {
    fn new(parsed: &Parsed) -> Self {
        let source = &parsed.text.source;
        let end = source.find('\n').unwrap_or(source.len());
        let end = end - usize::from(source[..end].ends_with('\r'));
        let mut counts = BTreeMap::<String, bool>::new();
        for entry in &parsed.syntax.citations {
            counts
                .entry(key_name(&entry.name))
                .and_modify(|unique| *unique = false)
                .or_insert(true);
        }
        let mut fields = parsed
            .syntax
            .bibliography_completion
            .entries
            .iter()
            .flat_map(|entry| &entry.fields)
            .peekable();
        let mut closed = parsed.syntax.bibliography_entries.iter().peekable();
        let mut references = parsed.syntax.citation_references.iter().peekable();
        let mut entries = BTreeMap::new();
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
        for owner in &parsed.syntax.bibliography_completion.types {
            let Some(body) = &owner.entry_span else {
                continue;
            };
            while closed
                .peek()
                .is_some_and(|(span, _)| span.start < body.start)
            {
                closed.next();
            }
            while fields
                .peek()
                .is_some_and(|field| field.span.start < body.start)
            {
                fields.next();
            }
            let key = owner
                .key
                .as_ref()
                .map(|span| key_name(&source[span.clone()]));
            let selected = key
                .as_ref()
                .is_some_and(|key| counts.get(key) == Some(&true));
            let complete = closed
                .peek()
                .is_some_and(|(span, comment)| span == body && !comment);
            let kind =
                (selected && complete && bib_strings::identifier(&source[owner.span.clone()]))
                    .then(|| SourceType {
                        name: biber_keys::model_name(&source[owner.span.clone()]),
                        range: Range::new(at(owner.span.start), at(owner.span.end)),
                    });
            let key_range = owner
                .key
                .as_ref()
                .filter(|_| selected)
                .map(|span| Range::new(at(span.start), at(span.end)));
            let mut names = BTreeMap::new();
            let mut model_names = BTreeMap::new();
            let mut relations = BTreeMap::<String, BTreeMap<String, Option<Range>>>::new();
            while fields
                .peek()
                .is_some_and(|field| field.span.start < body.end)
            {
                let field = fields.next().unwrap();
                if !selected || !complete {
                    continue;
                }
                let input_name = field_name(&source[field.span.clone()]);
                let location = (field.assigned && field.value_valid && field.range.end <= body.end)
                    .then(|| {
                        let assignment = syntax::skip_space(source, field.span.end);
                        let start = syntax::skip_space(source, assignment + 1);
                        let end = field.range.end;
                        let value = syntax::bib_literal_mode(source, start, end, false)
                            .map(|literal| {
                                if literal.span.is_empty() {
                                    literal.outer_span
                                } else {
                                    literal.span
                                }
                            })
                            .unwrap_or(start..end);
                        let name = Range::new(at(field.span.start), at(field.span.end));
                        let start = at(value.start);
                        while references
                            .peek()
                            .is_some_and(|reference| reference.span.start < value.start)
                        {
                            references.next();
                        }
                        if matches!(
                            input_name.as_str(),
                            "crossref" | "xref" | "xdata" | "related" | "entryset"
                        ) {
                            let targets = relations.entry(input_name.clone()).or_default();
                            // Convert literal list items before advancing to
                            // the value's end; the converter only moves forward.
                            while references.peek().is_some_and(|reference| {
                                reference.span.start < value.end && reference.span.end <= value.end
                            }) {
                                let reference = references.next().unwrap();
                                let range =
                                    Range::new(at(reference.span.start), at(reference.span.end));
                                targets
                                    .entry(key_name(&reference.name))
                                    .and_modify(|entry| *entry = None)
                                    .or_insert(Some(range));
                            }
                        }
                        Field {
                            name,
                            value: Range::new(start, at(value.end)),
                        }
                    });
                names
                    .entry(input_name)
                    .and_modify(|entry| *entry = None)
                    .or_insert(location);
                // Convert the raw location once; a model fold can join input
                // spellings that remain distinct in unknown-field messages.
                model_names
                    .entry(biber_keys::model_name(&source[field.span.clone()]))
                    .and_modify(|entry| *entry = None)
                    .or_insert(location);
            }
            if let Some(key_range) = key_range {
                entries.insert(
                    key.unwrap(),
                    Entry {
                        key: key_range,
                        kind,
                        fields: names,
                        model_fields: model_names,
                        relations,
                    },
                );
            }
        }
        Self {
            first_line: parsed.text.range(0, end),
            entries,
            ambiguous: counts
                .into_iter()
                .filter_map(|(key, unique)| (!unique).then_some(key))
                .collect(),
        }
    }
}

impl Parsed {
    pub(super) fn biber_model_field_ranges(
        &self,
        key: &str,
        field: &str,
    ) -> Option<(Range, Option<Range>)> {
        let entry = self.biber_field_locations().entries.get(&key_name(key))?;
        let value = entry
            .model_fields
            .get(&biber_keys::model_name(field))
            .copied()
            .flatten()
            .map(|field| field.value);
        Some((entry.key, value))
    }

    pub(super) fn biber_input_field_ranges(
        &self,
        key: &str,
        field: &str,
    ) -> Option<(Range, Option<Range>)> {
        let entry = self.biber_field_locations().entries.get(&key_name(key))?;
        let value = entry
            .fields
            .get(&field_name(field))
            .copied()
            .flatten()
            .map(|field| field.value);
        Some((entry.key, value))
    }

    pub(super) fn biber_reference_ranges(
        &self,
        key: &str,
        field: &str,
        parent: &str,
    ) -> Option<(Range, Option<Range>)> {
        let entry = self.biber_field_locations().entries.get(&key_name(key))?;
        let reference = entry.fields.get(field).copied().flatten().and_then(|_| {
            entry
                .relations
                .get(field)?
                .get(&key_name(parent))
                .copied()
                .flatten()
        });
        Some((entry.key, reference))
    }

    pub(super) fn biber_type_range(&self, key: &str, reported: &str) -> Option<Range> {
        self.biber_field_locations()
            .entries
            .get(&key_name(key))?
            .kind
            .as_ref()
            .filter(|kind| kind.name == biber_keys::model_name(reported))
            .map(|kind| kind.range)
    }

    fn biber_field_locations(&self) -> &Locations {
        self.biber_field_locations
            .get_or_init(|| Locations::new(self))
    }
}

impl Context {
    /// An unqualified warning cannot select the latest database: every earlier
    /// input must be readable, and canonical key collisions remain ambiguous.
    pub(super) fn biber_field_source(&mut self, key: &str) -> Option<(PathBuf, Arc<Parsed>)> {
        let inputs = self.biber_sources.inputs()?.to_vec();
        let key = key_name(key);
        let mut selected = None;
        for path in inputs {
            let parsed = self.document(&path)?;
            let locations = parsed.biber_field_locations();
            if locations.ambiguous.contains(&key) {
                return None;
            }
            if locations.entries.contains_key(&key) {
                if selected.is_some() {
                    return None;
                }
                selected = Some((path, parsed));
            }
        }
        selected
    }

    pub(crate) fn biber_field_warning(
        &mut self,
        root: &Path,
        paths: &BibliographyPaths<'_>,
        warning: Record<'_>,
        message: &str,
        severity: DiagnosticSeverity,
    ) -> Option<(Url, Diagnostic)> {
        let mut uri = Url::from_file_path(root).ok()?;
        let mut data =
            serde_json::json!({"key":warning.key,"field":warning.field,"unresolvedSource":true});
        if matches!(warning.kind, Kind::Overwritten) {
            data["overwrittenBy"] = "date".into();
        }
        match warning.kind {
            Kind::SetName => data["reason"] = "obsolete-field".into(),
            Kind::SetMissing => data["reason"] = "missing-entryset".into(),
            _ => {}
        }
        let mut diagnostic = Diagnostic {
            severity: Some(severity),
            source: Some("latex-build".into()),
            code: Some(NumberOrString::String(warning.kind.code().into())),
            message: format!("Biber: {}", message.trim()),
            data: Some(data),
            ..Default::default()
        };
        let selected = match warning.source {
            Source::Named(file) => file
                .and_then(|file| biber_keys::source(paths, file))
                .and_then(|path| self.document(&path).map(|parsed| (path, parsed))),
            Source::History => self.biber_field_source(warning.key),
        };
        let Some((path, parsed)) = selected else {
            return Some((uri, diagnostic));
        };
        uri = Url::from_file_path(path).ok()?;
        let data = diagnostic.data.as_mut()?;
        data.as_object_mut()?.remove("unresolvedSource");
        let locations = parsed.biber_field_locations();
        if let Some(entry) = locations.entries.get(&key_name(warning.key)) {
            diagnostic.range = entry.key;
            // A missing-field report may describe source-mapped data, even
            // when the original source contains an ENTRYSET assignment.
            if matches!(warning.kind, Kind::SetMissing) {
                return Some((uri, diagnostic));
            }
            let field = match warning.spelling {
                Spelling::Input => entry.fields.get(&field_name(warning.field)),
                Spelling::Model => entry
                    .model_fields
                    .get(&biber_keys::model_name(warning.field)),
            };
            if let Some(Some(field)) = field {
                diagnostic.range = match warning.kind {
                    Kind::Name | Kind::SetName => field.name,
                    Kind::Value | Kind::Overwritten => field.value,
                    Kind::SetMissing => unreachable!(),
                };
            } else {
                data["unresolvedField"] = true.into();
            }
            if matches!(warning.kind, Kind::Overwritten) {
                // Source maps can create or rename DATE. Only link its unique,
                // complete raw assignment in the same entry, never a parent.
                if let Some(Some(date)) = entry.model_fields.get("date") {
                    diagnostic.related_information = Some(vec![DiagnosticRelatedInformation {
                        location: Location::new(uri.clone(), date.value),
                        message: format!(
                            "Biber uses this date field to overwrite `{}`",
                            warning.field
                        ),
                    }]);
                }
            }
        } else {
            diagnostic.range = locations.first_line;
            data["unresolvedEntry"] = true.into();
        }
        Some((uri, diagnostic))
    }
}
