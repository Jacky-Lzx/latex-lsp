//! Static IDS certification for source bibliography namespaces.
use crate::{
    syntax,
    workspace::{DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::Url;

/// Certify unique static IDS, their source owners and indirect member uses.
/// Alias names cannot themselves shadow source sets. Complete edits and
/// candidate fields additionally validate their own execution dependencies.
pub(crate) fn certify(
    ws: &Workspace,
    resources: &BTreeSet<Url>,
    sources: &BTreeSet<&str>,
    members: &BTreeSet<&str>,
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
                    && doc.text.source[owner.span.clone()].eq_ignore_ascii_case("xdata")
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
                    || sources.contains(primary) && members.contains(name)
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

/// Resource-only identities cannot model Biber's global alias cache. Require
/// each consumed alias to have a local declaration. Other allocations can
/// keep independent aliases until one is imported through the global cache.
pub(crate) fn closed_resources(
    ws: &Workspace,
    views: &BTreeSet<std::sync::Arc<BTreeSet<Url>>>,
    uses: &BTreeMap<std::sync::Arc<BTreeSet<Url>>, BTreeSet<&str>>,
) -> Option<usize> {
    let mut resources = BTreeSet::new();
    let mut work = 0usize;
    for view in views {
        work += view
            .iter()
            .map(|uri| uri.as_str().len() + 64)
            .sum::<usize>();
        if work > 32 * 1024 * 1024 {
            return None;
        }
        resources.extend(view.iter());
    }
    let mut aliases = BTreeMap::new();
    for uri in resources {
        let doc = ws.documents.get(uri)?;
        for alias in &doc.syntax.bibliography_aliases {
            let primary = owner(doc, alias)?;
            work += alias.name.len() + primary.len() + 96;
            if work > 32 * 1024 * 1024 || aliases.len() >= 100_000 {
                return None;
            }
            if aliases
                .insert(alias.name.as_str(), primary)
                .is_some_and(|previous| previous != primary)
            {
                return None;
            }
        }
    }
    if aliases.is_empty() {
        return Some(work);
    }
    for view in views {
        let mut visible = BTreeSet::new();
        for uri in view.iter() {
            let doc = ws.documents.get(uri)?;
            for alias in &doc.syntax.bibliography_aliases {
                work += alias.name.len() + 64;
                if work > 32 * 1024 * 1024 {
                    return None;
                }
                visible.insert(alias.name.as_str());
            }
        }
        if uses
            .get(view)
            .is_some_and(|names| names.iter().any(|name| !visible.contains(name)))
        {
            return None;
        }
        for uri in view.iter() {
            for reference in &ws.documents.get(uri)?.syntax.citation_references {
                work += reference.name.len() + 64;
                if work > 32 * 1024 * 1024 {
                    return None;
                }
                if aliases.contains_key(reference.name.as_str())
                    && !visible.contains(reference.name.as_str())
                {
                    return None;
                }
            }
        }
    }
    Some(work)
}

#[derive(Clone, Copy)]
pub(crate) struct PrimaryRename<'a> {
    pub uri: &'a Url,
    pub from: &'a str,
    pub to: &'a str,
}
fn primary_spelling<'a>(uri: &Url, name: &'a str, rename: Option<PrimaryRename<'a>>) -> &'a str {
    rename
        .filter(|rename| rename.uri == uri && rename.from == name)
        .map_or(name, |rename| rename.to)
}

/// Each database is parsed once per Biber run. Later files replace equal
/// primary spellings in GLOBALDS, which cached IDS requests then import.
pub(crate) fn cached_owners(
    ws: &Workspace,
    sections: &[std::sync::Arc<BTreeSet<Url>>],
    uses: &BTreeMap<usize, BTreeSet<&str>>,
    aliases: &BTreeSet<&str>,
    processed: &BTreeSet<usize>,
    rename: Option<PrimaryRename<'_>>,
) -> Option<usize> {
    let mut loaded = BTreeSet::new();
    let mut removed = BTreeSet::new();
    let mut ambiguous = BTreeSet::new();
    let mut owners = BTreeMap::new();
    let mut keys = BTreeMap::<&Url, BTreeMap<&str, usize>>::new();
    let mut work = 0usize;
    for (section, resources) in sections.iter().enumerate() {
        if !processed.contains(&section) {
            continue;
        }
        let mut parsed = BTreeSet::new();
        for uri in resources.iter() {
            work += uri.as_str().len() + 64;
            if work > 32 * 1024 * 1024 {
                return None;
            }
            if loaded.insert(uri) {
                for entry in &ws.documents.get(uri)?.syntax.citations {
                    work += entry.name.len() + 192;
                    if work > 32 * 1024 * 1024 || owners.len() >= 100_000 {
                        return None;
                    }
                    let primary = primary_spelling(uri, &entry.name, rename);
                    owners.insert(primary, uri);
                    parsed.insert(primary);
                    *keys.entry(uri).or_default().entry(primary).or_default() += 1;
                    // Real entries remove equal aliases even when uncited.
                    // A later re-registration is not yet a certified recovery.
                    if aliases.contains(primary) {
                        removed.insert(primary);
                    }
                }
            }
        }
        // Parsing a new file alongside an equal real key has an uncertain
        // provider order. Keep that uncertainty in later cached requests.
        for primary in parsed {
            let mut matches = 0usize;
            for resource in resources.iter() {
                work += primary.len() + 64;
                if work > 32 * 1024 * 1024 {
                    return None;
                }
                matches += keys
                    .get(resource)
                    .and_then(|keys| keys.get(primary))
                    .copied()
                    .unwrap_or(0);
            }
            if matches != 1 {
                ambiguous.insert(primary);
            }
        }
        let mut relations = BTreeSet::new();
        for uri in resources.iter() {
            for reference in &ws.documents.get(uri)?.syntax.citation_references {
                work += reference.name.len() + 64;
                if work > 32 * 1024 * 1024 {
                    return None;
                }
                relations.insert(reference.name.as_str());
            }
        }
        for uri in resources.iter() {
            let doc = ws.documents.get(uri)?;
            for alias in &doc.syntax.bibliography_aliases {
                work += alias.name.len() + 64;
                if work > 32 * 1024 * 1024 {
                    return None;
                }
                if !uses
                    .get(&section)
                    .is_some_and(|names| names.contains(alias.name.as_str()))
                    && !relations.contains(alias.name.as_str())
                {
                    continue;
                }
                let primary = primary_spelling(uri, owner(doc, alias)?, rename);
                if removed.contains(alias.name.as_str())
                    || ambiguous.contains(primary)
                    || owners.get(primary) != Some(&uri)
                {
                    return None;
                }
                let mut matches = 0usize;
                for resource in resources.iter() {
                    work += primary.len() + 64;
                    if work > 32 * 1024 * 1024 {
                        return None;
                    }
                    matches += keys
                        .get(resource)
                        .and_then(|keys| keys.get(primary))
                        .copied()
                        .unwrap_or(0);
                }
                if matches != 1 {
                    return None;
                }
            }
        }
    }
    Some(work)
}

/// Static primary spelling for a certified alias declaration.
pub(crate) fn owner<'a>(
    doc: &'a crate::workspace::Document,
    entry: &syntax::Entry,
) -> Option<&'a str> {
    let types = &doc.syntax.bibliography_completion.types;
    let index = types
        .partition_point(|owner| owner.span.start < entry.span.start)
        .checked_sub(1)?;
    let primary = types.get(index)?.key.as_ref()?;
    (*primary != entry.span).then(|| &doc.text.source[primary.clone()])
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
