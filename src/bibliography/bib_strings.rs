//! BibTeX value structure and abbreviation references share one lexical scanner.
use crate::{
    parser::syntax::{self, Entry},
    project::workspace::{Document, Workspace},
};
use lsp_types::*;
use std::{collections::BTreeSet, ops::Range};

#[derive(Clone, Debug, Default)]
pub(crate) struct Index {
    pub definitions: Vec<Entry>,
    pub references: Vec<Entry>,
    /// Unexpanded expression, operand and literal-content ranges for selection.
    pub selection_spans: Vec<Range<usize>>,
    /// Literal operands in source order; punctuation cannot pair across them.
    pub literal_spans: Vec<Range<usize>>,
    pub number_spans: Vec<Range<usize>>,
    pub operator_spans: Vec<Range<usize>>,
    /// Complete, syntactically valid assignments available to compiler fixes.
    pub available_definitions: BTreeSet<usize>,
    empty_values: Vec<Range<usize>>,
}

const MONTHS: &[(&str, &str)] = &[
    ("jan", "January"),
    ("feb", "February"),
    ("mar", "March"),
    ("apr", "April"),
    ("may", "May"),
    ("jun", "June"),
    ("jul", "July"),
    ("aug", "August"),
    ("sep", "September"),
    ("oct", "October"),
    ("nov", "November"),
    ("dec", "December"),
];

pub(crate) fn builtin(name: &str) -> Option<&'static str> {
    MONTHS
        .iter()
        .find(|(month, _)| name.eq_ignore_ascii_case(month))
        .map(|(_, description)| *description)
}

pub(crate) fn identifier(name: &str) -> bool {
    !name.is_empty()
        && !name.as_bytes()[0].is_ascii_digit()
        && syntax::bib_identifier_end(name, 0) == name.len()
}

fn entry(source: &str, span: Range<usize>, detail: String) -> Entry {
    Entry {
        name: source[span.clone()].into(),
        span,
        detail,
        is_variable: false,
    }
}

/// Literals protect punctuation and percent signs; braces nest inside quotes.
fn literal_end(source: &str, start: usize, end: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let quoted = bytes[start] == b'"';
    let mut depth = usize::from(!quoted);
    let mut at = start + 1;
    while at < end {
        match bytes[at] {
            b'\\' => at += 1,
            b'{' => depth += 1,
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 && !quoted {
                    return Some(at + 1);
                }
            }
            b'"' if quoted && depth == 0 => return Some(at + 1),
            _ => {}
        }
        at += 1;
    }
    None
}

impl Index {
    pub fn declaration(&mut self, source: &str, start: usize, end: usize, closed: bool) {
        let start = syntax::skip_space(source, start).min(end);
        let name_end = syntax::bib_identifier_end(source, start).min(end);
        if !identifier(&source[start..name_end]) {
            return;
        }
        let assignment = syntax::skip_space(source, name_end);
        if assignment >= end || source.as_bytes()[assignment] != b'=' {
            return;
        }
        let value_start = assignment + 1;
        self.operator_spans.push(assignment..assignment + 1);
        let value_end = syntax::bib_value_end(source, value_start, end);
        // @string contains one assignment, with an optional trailing comma.
        if value_end < end && syntax::skip_space(source, value_end + 1) < end {
            return;
        }
        let mut words = source[value_start..value_end].split_whitespace();
        let mut characters = words
            .next()
            .unwrap_or("")
            .chars()
            .chain(words.flat_map(|word| std::iter::once(' ').chain(word.chars())));
        let mut preview: String = characters.by_ref().take(200).collect();
        if characters.next().is_some() {
            preview.push('…');
        }
        self.definitions.push(entry(
            source,
            start..name_end,
            format!("BibTeX string abbreviation = {preview}"),
        ));
        self.selection_spans.push(start..value_end);
        if self.expression(source, value_start, value_end) && closed {
            self.available_definitions.insert(start);
        }
    }

    pub fn preamble(&mut self, source: &str, start: usize, end: usize) {
        let value_end = syntax::bib_value_end(source, start, end);
        if value_end == end || syntax::skip_space(source, value_end + 1) >= end {
            self.expression(source, start, value_end);
        }
    }

    pub fn expression(&mut self, source: &str, mut at: usize, end: usize) -> bool {
        let bytes = source.as_bytes();
        let before = self.references.len();
        let expression_selection = self.selection_spans.len();
        let span = syntax::trimmed_span(source, syntax::skip_space(source, at).min(end)..end);
        if !span.is_empty() {
            self.selection_spans.push(span);
        }
        loop {
            let gap = at;
            at = syntax::skip_space(source, at).min(end);
            if at == end {
                self.empty_values.push(gap..end);
                return false;
            }
            let operand_start = at;
            at = match bytes[at] {
                b'{' | b'"' => {
                    let Some(after) = literal_end(source, at, end) else {
                        // Trailing blanks belong to an unfinished literal,
                        // rather than to the gap following a complete value.
                        self.selection_spans[expression_selection].end = end;
                        self.literal_spans.push(at..end);
                        self.selection_spans.extend([at..end, at + 1..end]);
                        // Earlier concatenation operands remain real references
                        // while the next literal is still being typed.
                        return false;
                    };
                    self.literal_spans.push(at..after);
                    self.selection_spans.push(at + 1..after - 1);
                    after
                }
                b'0'..=b'9' => {
                    let length = bytes[at..end]
                        .iter()
                        .take_while(|b| b.is_ascii_digit())
                        .count();
                    self.number_spans.push(at..at + length);
                    at + length
                }
                _ => {
                    let after = syntax::bib_identifier_end(source, at).min(end);
                    if !identifier(&source[at..after]) {
                        self.references.truncate(before);
                        return false;
                    }
                    self.references.push(entry(
                        source,
                        at..after,
                        "BibTeX string reference".into(),
                    ));
                    after
                }
            };
            self.selection_spans.push(operand_start..at);
            at = syntax::skip_space(source, at).min(end);
            if at == end {
                return true;
            }
            if bytes[at] != b'#' {
                // Do not rename identifiers in malformed multi-token operands.
                self.references.truncate(before);
                return false;
            }
            self.operator_spans.push(at..at + 1);
            at += 1;
        }
    }
}

pub(crate) fn completion(
    workspace: &Workspace,
    doc: &Document,
    offset: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let index = &doc.syntax.bibliography_strings;
    let span = if let Some(reference) = index
        .references
        .iter()
        .find(|entry| entry.span.start <= offset && offset <= entry.span.end)
    {
        reference.span.clone()
    } else {
        let gap = index
            .empty_values
            .iter()
            .find(|gap| gap.start <= offset && offset <= gap.end)?;
        let line_start = doc.text.source[..offset].rfind('\n').map_or(0, |at| at + 1);
        if doc.text.source[gap.start.max(line_start)..offset].contains('%') {
            return None;
        }
        offset..offset
    };
    let prefix = doc.text.source[span.start..offset].to_ascii_lowercase();
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for (uri, other) in &workspace.documents {
        if !scope.contains(uri) {
            continue;
        }
        for entry in &other.syntax.bibliography_strings.definitions {
            if seen.insert(entry.name.to_ascii_lowercase()) {
                candidates.push((
                    entry.name.clone(),
                    format!("{} — {}", entry.detail, crate::language::display::path(uri)),
                ));
            }
        }
    }
    for &(month, description) in MONTHS {
        if seen.insert(month.into()) {
            candidates.push((
                month.into(),
                format!("Common BibTeX month abbreviation: {description}"),
            ));
        }
    }
    Some(
        candidates
            .into_iter()
            .filter(|(name, _)| name.to_ascii_lowercase().starts_with(&prefix))
            .take(workspace.completion_budget())
            .map(|(name, detail)| CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::CONSTANT),
                detail: Some(detail),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: doc.text.range(span.start, span.end),
                    new_text: name,
                })),
                ..Default::default()
            })
            .collect(),
    )
}
