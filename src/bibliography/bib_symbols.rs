//! Bibliography outlines share construct and field boundaries with the parser.
use crate::{parser::syntax, project::workspace::Document};
use lsp_types::*;

#[allow(deprecated)]
pub(crate) fn symbols(doc: &Document) -> Vec<DocumentSymbol> {
    let index = &doc.syntax.bibliography_completion;
    let mut entries: Vec<_> = [
        (&doc.syntax.citations, SymbolKind::KEY),
        (
            &doc.syntax.bibliography_strings.definitions,
            SymbolKind::CONSTANT,
        ),
    ]
    .into_iter()
    .flat_map(|(entries, kind)| entries.iter().map(move |entry| (entry, kind)))
    .collect();
    entries.sort_by_key(|(entry, _)| entry.span.start);
    // Fields and constructs occur in source order. Advance once through the
    // field index instead of scanning every field for each bibliography key.
    let mut fields = index
        .entries
        .iter()
        .flat_map(|entry| &entry.fields)
        .peekable();
    // Alias lists can contain many names on one long line. Convert their
    // source-ordered ranges once instead of rescanning each line prefix.
    let mut byte = 0;
    let mut position = Position::new(0, 0);
    let mut at = |target| {
        for character in doc.text.source[byte..target].chars() {
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
    let mut aliases = doc
        .syntax
        .bibliography_aliases
        .iter()
        .map(|alias| {
            let range = Range::new(at(alias.span.start), at(alias.span.end));
            (alias, range)
        })
        .peekable();
    entries
        .into_iter()
        .map(|(entry, kind)| {
            let owner = index
                .types
                .partition_point(|ty| ty.span.start <= entry.span.start)
                .checked_sub(1)
                .and_then(|at| index.types[at].entry_span.as_ref())
                .filter(|span| span.start <= entry.span.start && entry.span.end <= span.end);
            let span = syntax::trimmed_span(&doc.text.source, owner.unwrap_or(&entry.span).clone());
            while fields
                .peek()
                .is_some_and(|field| field.span.start < span.start)
            {
                fields.next();
            }
            let mut children = Vec::new();
            while fields
                .peek()
                .is_some_and(|field| field.span.start < span.end)
            {
                let field = fields.next().unwrap();
                if kind != SymbolKind::KEY || field.range.end > span.end {
                    continue;
                }
                while aliases
                    .peek()
                    .is_some_and(|(alias, _)| alias.span.start < field.range.start)
                {
                    aliases.next();
                }
                let mut alias_symbols = Vec::new();
                while aliases
                    .peek()
                    .is_some_and(|(alias, _)| alias.span.start < field.range.end)
                {
                    let (alias, range) = aliases.next().unwrap();
                    if alias.span.end > field.range.end {
                        continue;
                    }
                    alias_symbols.push(DocumentSymbol {
                        name: alias.name.clone(),
                        detail: Some(alias.detail.clone()),
                        kind: SymbolKind::KEY,
                        tags: None,
                        deprecated: None,
                        range,
                        selection_range: range,
                        children: None,
                    });
                }
                children.push(DocumentSymbol {
                    name: doc.text.source[field.span.clone()].into(),
                    detail: None,
                    kind: SymbolKind::PROPERTY,
                    tags: None,
                    deprecated: None,
                    range: doc.text.range(field.range.start, field.range.end),
                    selection_range: doc.text.range(field.span.start, field.span.end),
                    children: (!alias_symbols.is_empty()).then_some(alias_symbols),
                });
            }
            DocumentSymbol {
                name: entry.name.clone(),
                detail: Some(entry.detail.clone()),
                kind,
                tags: None,
                deprecated: None,
                range: doc.text.range(span.start, span.end),
                selection_range: doc.text.range(entry.span.start, entry.span.end),
                children: (!children.is_empty()).then_some(children),
            }
        })
        .collect()
}
