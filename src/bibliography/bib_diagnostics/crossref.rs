//! Native crossref errors and nested-reference warnings have two records but
//! no file or line number. Each kind consumes only its own continuation.
use super::{Context, Parsed};
use lsp_types::*;
use std::path::Path;

const MAX_ENTRIES: usize = 16_384;

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Missing,
    Nested,
}

impl Kind {
    fn header(self) -> &'static str {
        match self {
            Self::Missing => "A bad cross reference---entry \"",
            Self::Nested => "you've nested cross references--entry \"",
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Missing => "\", which doesn't exist",
            Self::Nested => "\", which also refers to something",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Missing => "bad-bibliography-crossref",
            Self::Nested => "nested-bibliography-crossref",
        }
    }

    fn severity(self) -> DiagnosticSeverity {
        match self {
            Self::Missing => DiagnosticSeverity::ERROR,
            Self::Nested => DiagnosticSeverity::WARNING,
        }
    }

    pub(crate) fn parent(self, line: &str) -> Option<&str> {
        let parent = line
            .strip_prefix("refers to entry \"")?
            .strip_suffix(self.suffix())?;
        // Empty literal values also produce native missing-parent errors.
        (!parent.chars().any(char::is_control)).then_some(parent)
    }

    fn message(self, child: &str, parent: Option<&str>) -> String {
        let mut message = format!("BibTeX: {}{child}\"", self.header());
        if let Some(parent) = parent {
            message.push_str(&format!(" refers to entry \"{parent}{}", self.suffix()));
        }
        message
    }
}

fn key(value: &str) -> Option<&str> {
    (!value.is_empty() && !value.chars().any(char::is_control)).then_some(value)
}

pub(crate) fn record(line: &str) -> Option<(Kind, &str)> {
    let (kind, line) = if let Some(line) = line.strip_prefix("Warning--") {
        (Kind::Nested, line)
    } else {
        (Kind::Missing, line)
    };
    let child = key(line.strip_prefix(kind.header())?.strip_suffix('"')?)?;
    Some((kind, child))
}

impl Context {
    pub(crate) fn crossref(
        &mut self,
        root: &Path,
        kind: Kind,
        child: &str,
        parent: Option<&str>,
    ) -> Option<(Url, Diagnostic)> {
        let mut diagnostic = Diagnostic {
            severity: Some(kind.severity()),
            source: Some("latex-build".into()),
            code: Some(NumberOrString::String(kind.code().into())),
            message: kind.message(child, parent),
            data: Some(serde_json::json!({"key":child,"parent":parent,"unresolvedSource":true})),
            ..Default::default()
        };
        let root_uri = Url::from_file_path(root).ok()?;
        let fallback = (root_uri, diagnostic.clone());
        let Some(parent) = parent.filter(|_| self.complete) else {
            return Some(fallback);
        };
        // Read only databases encountered before this record. Missing inputs,
        // incomplete histories and duplicate keys cannot identify the owner.
        let inputs = self.inputs.clone();
        let mut databases = Vec::new();
        let mut examined = 0usize;
        let mut selected = None;
        for (path, database) in inputs {
            if !database {
                continue;
            }
            let Some(parsed) = self.document(&path) else {
                return Some(fallback);
            };
            examined = examined.saturating_add(parsed.syntax.citations.len());
            if examined > MAX_ENTRIES {
                return Some(fallback);
            }
            for entry in &parsed.syntax.citations {
                if entry.name.eq_ignore_ascii_case(child) {
                    if selected.is_some() || entry.name != child {
                        return Some(fallback);
                    }
                    selected = Some((databases.len(), entry.span.clone()));
                }
            }
            databases.push((path, parsed));
        }
        let Some((index, key_span)) = selected else {
            return Some(fallback);
        };
        let (path, parsed) = &databases[index];
        let Some((body, _)) = parsed
            .syntax
            .bibliography_entries
            .iter()
            .find(|(span, comment)| {
                !comment && span.start < key_span.start && key_span.end < span.end
            })
        else {
            return Some(fallback);
        };
        // Count every crossref field, including macro/concatenated operands.
        // A second unindexed dynamic value must not make a literal look unique.
        let count = parsed
            .syntax
            .bibliography_completion
            .entries
            .iter()
            .flat_map(|entry| &entry.fields)
            .filter(|field| field.name == "crossref" && body.contains(&field.span.start))
            .count();
        if count != 1 {
            return Some(fallback);
        }
        let mut fields = parsed.syntax.citation_fields.iter().filter(|field| {
            field.name == "crossref"
                && body.contains(&field.argument.span.start)
                && field
                    .argument
                    .value
                    .trim_matches(|ch: char| ch.is_ascii_whitespace())
                    == parent
        });
        let Some(field) = fields.next().filter(|_| fields.next().is_none()) else {
            return Some(fallback);
        };
        let Ok(uri) = Url::from_file_path(path) else {
            return Some(fallback);
        };
        let value = &field.argument.value;
        let start = field.argument.span.start + value.len()
            - value
                .trim_start_matches(|ch: char| ch.is_ascii_whitespace())
                .len();
        let span = if parent.is_empty() {
            field.argument.outer_span.clone()
        } else {
            start..start + parent.len()
        };
        diagnostic.range = parsed.text.range(span.start, span.end);
        diagnostic.data = Some(serde_json::json!({"key":child,"parent":parent}));
        let mut related = Vec::new();
        for (other_index, (path, parsed)) in databases.iter().enumerate() {
            related_parents(&mut related, path, parsed, kind, parent, |offset| {
                other_index < index || other_index == index && offset < key_span.start
            });
        }
        if !related.is_empty() {
            diagnostic.related_information = Some(related);
        }
        Some((uri, diagnostic))
    }
}

fn related_parents(
    related: &mut Vec<DiagnosticRelatedInformation>,
    path: &Path,
    parsed: &Parsed,
    kind: Kind,
    parent: &str,
    earlier: impl Fn(usize) -> bool,
) {
    for entry in &parsed.syntax.citations {
        if related.len() >= 16 {
            break;
        }
        if !entry.name.eq_ignore_ascii_case(parent) {
            continue;
        }
        let Ok(uri) = Url::from_file_path(path) else {
            continue;
        };
        let message = match kind {
            Kind::Nested => "Parent entry in the nested crossref chain reported by BibTeX",
            Kind::Missing if earlier(entry.span.start) =>
                "Parent entry declared before the child; BibTeX can skip an uncited parent in this order",
            Kind::Missing =>
                "Matching parent entry; BibTeX did not resolve this crossref in the reported run",
        };
        related.push(DiagnosticRelatedInformation {
            location: Location::new(uri, parsed.text.range(entry.span.start, entry.span.end)),
            message: message.into(),
        });
    }
}
