//! Native ignored-field warnings report the delimiter after the value, which
//! may occur several lines after the field name. Repeated same-line records
//! can be paired in source order only when the complete group is observed.
use super::{Context, Parsed};
use crate::{
    bibliography::{bib_completion::Field, bib_strings},
    compilation::recorder::BibliographyPaths,
    parser::syntax,
};
use std::{collections::BTreeMap, path::Path, sync::Arc};
use tower_lsp::lsp_types::*;

const CODE: &str = "repeated-bibliography-field";
type Key = (String, u32, String, String);

pub(crate) fn record(line: &str) -> Option<(&str, &str)> {
    let body = line
        .strip_prefix("Warning--I'm ignoring ")?
        .strip_suffix("\" field")?;
    let (key, field) = body.rsplit_once("'s extra \"")?;
    (syntax::is_static_citation_key(key) && bib_strings::identifier(field)).then_some((key, field))
}

#[derive(Default)]
pub(super) struct Warnings {
    groups: BTreeMap<Key, Group>,
}

#[derive(Default)]
struct Group {
    total: usize,
    used: usize,
    locations: Option<(Arc<Parsed>, Locations)>,
}

struct Occurrence {
    group: Key,
    at: usize,
}

struct Locations {
    line: Range,
    valid: bool,
    assignments: Option<Assignments>,
}

struct Assignments {
    earlier: Range,
    earlier_name: String,
    duplicates: Vec<Range>,
}

fn group(key: &str, field: &str, location: (u32, &str)) -> Key {
    (
        location.1.into(),
        location.0,
        key.to_ascii_lowercase(),
        field.to_ascii_lowercase(),
    )
}

impl Warnings {
    pub(super) fn new(log: &str) -> Self {
        let mut result = Self::default();
        let mut count = 0;
        let mut bytes = 0;
        for (line, next) in log.lines().zip(log.lines().skip(1)) {
            let Some((key, field)) = record(line) else {
                continue;
            };
            let Some(location) = super::position(next) else {
                continue;
            };
            count += 1;
            bytes += key.len() + field.len() + location.1.len();
            if count > 16_384 || bytes > super::MAX_TOTAL_SIZE {
                return Self::default();
            }
            result
                .groups
                .entry(group(key, field, location))
                .or_default()
                .total += 1;
        }
        result
    }

    fn next(
        &mut self,
        key: &str,
        field: &str,
        location: Option<(u32, &str)>,
    ) -> Option<Occurrence> {
        let key = group(key, field, location?);
        let state = self.groups.get_mut(&key)?;
        let at = state.used;
        state.used += 1;
        Some(Occurrence { group: key, at })
    }
}

struct Matches<'a> {
    earlier: &'a Field,
    duplicates: Vec<&'a Field>,
}

fn matches<'a>(parsed: &'a Parsed, key: &str, name: &str, line: u32) -> Option<Matches<'a>> {
    let source = &parsed.text.source;
    // Matching a reported line needs byte boundaries, not UTF-16 columns.
    // Avoid rescanning an entire long line for every candidate delimiter.
    let line_start = parsed.text.offset(Position::new(line, 0))?;
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |at| line_start + at);
    let mut result = None;
    // Fields are ordered independently of types. Advance through them once,
    // including over unmatched owners, rather than rescanning for each key.
    let mut fields = parsed
        .syntax
        .bibliography_completion
        .entries
        .iter()
        .flat_map(|entry| &entry.fields)
        .peekable();
    for owner in &parsed.syntax.bibliography_completion.types {
        let Some(body) = &owner.entry_span else {
            continue;
        };
        while fields
            .peek()
            .is_some_and(|field| field.span.start < body.start)
        {
            fields.next();
        }
        let selected = owner
            .key
            .as_ref()
            .is_some_and(|span| source[span.clone()].eq_ignore_ascii_case(key));
        let mut earlier = None;
        let mut duplicates = Vec::new();
        let mut blocked = false;
        while fields
            .peek()
            .is_some_and(|field| field.span.start < body.end)
        {
            let field = fields.next().unwrap();
            if !selected {
                continue;
            }
            if !field.assigned || !field.value_valid {
                blocked = true;
                continue;
            }
            if blocked || !field.name.eq_ignore_ascii_case(name) {
                continue;
            }
            if earlier.is_none() {
                earlier = Some(field);
                continue;
            }
            let delimiter = syntax::skip_space(source, field.range.end);
            if delimiter < body.end
                && matches!(source.as_bytes().get(delimiter), Some(b',' | b'}' | b')'))
                && (line_start..line_end).contains(&delimiter)
            {
                duplicates.push(field);
            }
        }
        if !duplicates.is_empty() {
            if result.is_some() {
                return None;
            }
            result = Some(Matches {
                earlier: earlier?,
                duplicates,
            });
        }
    }
    result
}

impl Assignments {
    fn new(parsed: &Parsed, found: Matches<'_>) -> Self {
        let source = &parsed.text.source;
        // Names are already in source order. Map their positions in one pass,
        // including Unicode and line breaks in intervening values. Calling
        // Text::range for each name would rescan a long line for every field.
        let mut byte = found.earlier.span.start;
        let mut position = parsed.text.position(byte);
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
        let mut range = |field: &Field| Range::new(at(field.span.start), at(field.span.end));
        Self {
            earlier: range(found.earlier),
            earlier_name: source[found.earlier.span.clone()].into(),
            duplicates: found.duplicates.into_iter().map(range).collect(),
        }
    }
}

impl Locations {
    fn new(
        parsed: &Parsed,
        key: &str,
        field: &str,
        line: u32,
        total: Option<usize>,
    ) -> Option<Self> {
        let valid = line
            .checked_sub(1)
            .filter(|line| (*line as usize) < parsed.text.text_line_count());
        let selected = valid.unwrap_or_else(|| {
            line.saturating_sub(1)
                .min(parsed.text.text_line_count().saturating_sub(1) as u32)
        });
        let start = parsed.text.offset(Position::new(selected, 0))?;
        let end = parsed.text.source[start..]
            .find('\n')
            .map_or(parsed.text.source.len(), |length| start + length);
        let end = end - usize::from(parsed.text.source[..end].ends_with('\r'));
        let assignments = valid
            .and_then(|_| matches(parsed, key, field, selected))
            // Do not retain large candidate lists for incomplete or ambiguous
            // groups. Retained duplicate ranges are bounded by record counts.
            .filter(|found| found.duplicates.len() == 1 || total == Some(found.duplicates.len()))
            .map(|found| Assignments::new(parsed, found));
        Some(Self {
            line: parsed.text.range(start, end),
            valid: valid.is_some(),
            assignments,
        })
    }
}

impl Context {
    pub(crate) fn extra_field(
        &mut self,
        root: &Path,
        paths: &BibliographyPaths<'_>,
        key: &str,
        field: &str,
        location: Option<(u32, &str)>,
    ) -> Option<(Url, Diagnostic)> {
        let ordinal = self.field_warnings.next(key, field, location);
        let mut uri = Url::from_file_path(root).ok()?;
        let mut diagnostic = Diagnostic {
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String(CODE.into())),
            source: Some("latex-build".into()),
            message: format!("BibTeX: I'm ignoring {key}'s extra \"{field}\" field"),
            data: Some(serde_json::json!({"key":key,"field":field,"unresolvedSource":true})),
            ..Default::default()
        };
        let Some((path, line)) = location.and_then(|(line, name)| {
            let path = paths.resolve(name)?;
            self.is_database(name, &path).then_some((path, line))
        }) else {
            return Some((uri, diagnostic));
        };
        let Some(parsed) = self.document(&path) else {
            return Some((uri, diagnostic));
        };
        uri = Url::from_file_path(&path).ok()?;
        let uncached;
        let locations = if let Some(state) = ordinal
            .as_ref()
            .and_then(|occurrence| self.field_warnings.groups.get_mut(&occurrence.group))
        {
            if !state
                .locations
                .as_ref()
                .is_some_and(|(source, _)| Arc::ptr_eq(source, &parsed))
            {
                state.locations = Some((
                    parsed.clone(),
                    Locations::new(&parsed, key, field, line, Some(state.total))?,
                ));
            }
            &state.locations.as_ref()?.1
        } else {
            uncached = Locations::new(&parsed, key, field, line, None)?;
            &uncached
        };
        diagnostic.range = locations.line;
        diagnostic.data = Some(serde_json::json!({"key":key,"field":field,"unresolvedField":true}));
        if !locations.valid {
            diagnostic.data.as_mut()?["unresolvedLine"] = true.into();
            diagnostic.data.as_mut()?["reportedLine"] = line.into();
            return Some((uri, diagnostic));
        }
        if let Some(found) = &locations.assignments {
            let selected = if found.duplicates.len() == 1 {
                found.duplicates.first()
            } else if self.complete {
                ordinal
                    .as_ref()
                    .and_then(|occurrence| found.duplicates.get(occurrence.at))
            } else {
                None
            };
            if let Some(selected) = selected {
                diagnostic.range = *selected;
                diagnostic.data = Some(serde_json::json!({"key":key,"field":field}));
                diagnostic.related_information = Some(vec![DiagnosticRelatedInformation {
                    location: Location::new(uri.clone(), found.earlier),
                    message: format!(
                        "Earlier assignment to `{}` in this bibliography entry",
                        found.earlier_name
                    ),
                }]);
            }
        }
        Some((uri, diagnostic))
    }
}
