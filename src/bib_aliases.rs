//! Static biblatex IDS declarations remain separate from database entry keys.
use crate::syntax::{self, Argument, Entry, Syntax};

pub(crate) fn index(source: &str, syntax: &Syntax) -> (Vec<Entry>, Vec<Argument>) {
    let mut fields = syntax
        .bibliography_completion
        .entries
        .iter()
        .flat_map(|entry| &entry.fields)
        .peekable();
    let mut closed = syntax.bibliography_entries.iter().peekable();
    let mut keys = syntax.citations.iter().peekable();
    let mut result = Vec::new();
    let mut values = Vec::new();
    for owner in &syntax.bibliography_completion.types {
        let (Some(body), Some(key)) = (&owner.entry_span, &owner.key) else {
            continue;
        };
        while fields
            .peek()
            .is_some_and(|field| field.span.start < body.start)
        {
            fields.next();
        }
        while closed
            .peek()
            .is_some_and(|(span, _)| span.start < body.start)
        {
            closed.next();
        }
        while keys
            .peek()
            .is_some_and(|entry| entry.span.start < key.start)
        {
            keys.next();
        }
        let primary = keys.peek().filter(|entry| entry.span == *key);
        let complete = closed
            .peek()
            .is_some_and(|(span, comment)| span == body && !comment);
        let mut selected = None;
        let mut count = 0;
        while fields
            .peek()
            .is_some_and(|field| field.span.start < body.end)
        {
            let field = fields.next().unwrap();
            // Biber reads IDS directly from Text::BibTeX, before model folding.
            if field.name.eq_ignore_ascii_case("ids") {
                selected = Some(field);
                count += 1;
            }
        }
        let Some((primary, field)) = primary.zip(selected).filter(|(primary, field)| {
            complete
                && count == 1
                && syntax::is_static_citation_key(&primary.name)
                && field.assigned
                && field.value_valid
                && field.range.end <= body.end
        }) else {
            continue;
        };
        let assignment = syntax::skip_space(source, field.span.end);
        let start = syntax::skip_space(source, assignment + 1);
        let Some(literal) = syntax::bib_literal_mode(source, start, field.range.end, false) else {
            continue;
        };
        if literal.value.contains(['\\', '{', '}', '%', '#']) {
            continue;
        }
        // Bound the repeated owner preview; IDs can contain many small aliases.
        let preview: String = primary.name.chars().take(80).collect();
        let detail = format!(
            "Biblatex alias for {preview}{}",
            if preview.len() < primary.name.len() {
                "…"
            } else {
                ""
            }
        );
        let mut offset = literal.span.start;
        let before = result.len();
        for name in literal.value.split(',') {
            let span = syntax::trimmed_span(source, offset..offset + name.len());
            if syntax::is_static_citation_key(&source[span.clone()]) {
                result.push(Entry {
                    name: source[span.clone()].into(),
                    span,
                    detail: detail.clone(),
                    is_variable: false,
                });
            }
            offset += name.len() + 1;
        }
        if result.len() > before {
            values.push(literal);
        }
    }
    (result, values)
}
