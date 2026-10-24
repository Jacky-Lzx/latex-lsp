//! Conservative certification of IDS namespaces independent of dynamic sets.
use crate::{
    syntax,
    workspace::{DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::Url;

/// Intersecting aliases require Biber's alias-loading phase, not the ordinary
/// first-primary-request model. Certify only static, unique, disjoint owners.
/// Return charged work so callers retain their complete analysis budgets.
pub(crate) fn independent(
    ws: &Workspace,
    resources: &BTreeSet<Url>,
    sources: &BTreeSet<&str>,
) -> Option<usize> {
    let mut work = 0usize;
    let mut bytes = 0usize;
    let mut has_ids = false;
    for uri in resources {
        let doc = ws.documents.get(uri)?;
        for field in doc
            .syntax
            .bibliography_completion
            .entries
            .iter()
            .flat_map(|entry| &entry.fields)
        {
            work += 16;
            if work > 32 * 1024 * 1024 {
                return None;
            }
            has_ids |= field.name == "ids";
        }
    }
    if !has_ids {
        return Some(work);
    }
    let mut names = BTreeMap::<&str, usize>::new();
    for uri in resources {
        let doc = ws.documents.get(uri)?;
        bytes += doc.text.source.len();
        if bytes > 16 * 1024 * 1024 || doc.kind != DocumentKind::Bibtex {
            return None;
        }
        for entry in doc
            .syntax
            .citations
            .iter()
            .chain(&doc.syntax.bibliography_aliases)
        {
            work += entry.name.len() + 96;
            if work > 32 * 1024 * 1024 || names.len() >= 100_000 {
                return None;
            }
            *names.entry(&entry.name).or_default() += 1;
        }
    }
    for uri in resources {
        let doc = ws.documents.get(uri)?;
        work += doc.syntax.bibliography_entries.len() * 64
            + doc.syntax.bibliography_completion.types.len() * 64;
        if work > 32 * 1024 * 1024 {
            return None;
        }
        let mut owners = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        let closed: BTreeSet<_> = doc
            .syntax
            .bibliography_entries
            .iter()
            .filter(|(_, comment)| !comment)
            .map(|(span, _)| (span.start, span.end))
            .collect();
        let types = &doc.syntax.bibliography_completion.types;
        for field in doc
            .syntax
            .bibliography_completion
            .entries
            .iter()
            .flat_map(|entry| &entry.fields)
            .filter(|field| field.name == "ids")
        {
            let index = types
                .partition_point(|owner| owner.span.start < field.span.start)
                .checked_sub(1)?;
            let owner = types.get(index)?;
            let body = owner.entry_span.as_ref()?;
            let primary = &doc.text.source[owner.key.as_ref()?.clone()];
            if field.range.end > body.end
                || !closed.contains(&(body.start, body.end))
                || !owners.insert(body.start)
                || !field.assigned
                || !field.value_valid
                || doc.text.source[owner.span.clone()].eq_ignore_ascii_case("set")
                || !syntax::is_static_citation_key(primary)
                || sources.contains(primary)
                || names.get(primary) != Some(&1)
            {
                return None;
            }
            let assignment = syntax::skip_space(&doc.text.source, field.span.end);
            let start = syntax::skip_space(&doc.text.source, assignment + 1);
            let literal =
                syntax::bib_literal_mode(&doc.text.source, start, field.range.end, false)?;
            if literal.value.contains(['\\', '{', '}', '%', '#']) {
                return None;
            }
            let mut offset = literal.span.start;
            for part in literal.value.split(',') {
                let span = syntax::trimmed_span(&doc.text.source, offset..offset + part.len());
                let name = &doc.text.source[span.clone()];
                work += name.len() + 96;
                if work > 32 * 1024 * 1024
                    || name.is_empty()
                    || !syntax::is_static_citation_key(name)
                    || sources.contains(name)
                    || names.get(name) != Some(&1)
                    || !aliases.insert((span.start, span.end))
                {
                    return None;
                }
                offset += part.len() + 1;
            }
        }
        if aliases.len() != doc.syntax.bibliography_aliases.len()
            || doc
                .syntax
                .bibliography_aliases
                .iter()
                .any(|alias| !aliases.contains(&(alias.span.start, alias.span.end)))
        {
            return None;
        }
    }
    Some(work)
}

/// Keep the alias identity while previewing its uniquely certified owner.
pub(crate) fn detail(doc: &crate::workspace::Document, entry: &syntax::Entry) -> String {
    let owners = &doc.syntax.bibliography_completion.types;
    let primary = owners
        .partition_point(|owner| owner.span.start < entry.span.start)
        .checked_sub(1)
        .and_then(|index| owners.get(index))
        .and_then(|owner| owner.key.as_ref());
    if let Some(primary) = primary.filter(|primary| **primary != entry.span) {
        let index = doc
            .syntax
            .citations
            .partition_point(|candidate| candidate.span.start < primary.start);
        if let Some(owner) = doc
            .syntax
            .citations
            .get(index)
            .filter(|owner| owner.span == *primary)
        {
            return format!("{}\n{}", entry.detail, owner.detail);
        }
    }
    entry.detail.clone()
}
