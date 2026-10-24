//! Bounded candidate identities for relations in source-set projects.
use crate::{
    bib_section_keys::{self, SourceEvent, SourceSet, SourceTrace},
    syntax,
    workspace::{DocumentKind, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tower_lsp::lsp_types::*;
type Identity = (Url, usize, usize);
type Result<T> = std::result::Result<T, ()>;
const LIMIT: usize = 32 * 1024 * 1024;
#[derive(Default)]
struct State {
    declared: BTreeMap<String, Arc<SourceSet>>,
    first: BTreeMap<String, Option<Arc<SourceSet>>>,
    wildcard: bool,
}
struct Candidate {
    uri: Url,
    detail: String,
}
#[derive(Default)]
struct Catalog {
    contexts: usize,
    all: BTreeSet<Url>,
    common: Option<BTreeSet<Url>>,
    loaded: BTreeSet<Url>,
    overrides: BTreeSet<String>,
    blocked: BTreeSet<String>,
    sources: BTreeMap<Identity, (Arc<SourceSet>, usize)>,
    work: usize,
    stored: usize,
}
impl Catalog {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.stored += bytes;
        if self.stored > LIMIT {
            Err(())
        } else {
            Ok(())
        }
    }
    fn trace(&mut self, trace: &SourceTrace, origin: &Url) -> Result<()> {
        self.work += trace.work;
        if self.work > LIMIT {
            return Err(());
        }
        let names: BTreeSet<_> = trace
            .events
            .iter()
            .filter_map(|event| match event {
                SourceEvent::Set(_, set) => Some(set.key.name.as_str()),
                _ => None,
            })
            .collect();
        let mut states = BTreeMap::<u32, State>::new();
        for event in &trace.events {
            match event {
                SourceEvent::Set(section, set) => {
                    if set
                        .members
                        .iter()
                        .any(|member| names.contains(member.name.as_str()))
                    {
                        return Err(());
                    }
                    let previous = states
                        .entry(*section)
                        .or_default()
                        .declared
                        .insert(set.key.name.clone(), set.clone());
                    if previous.is_some_and(|previous| previous.key != set.key) {
                        return Err(());
                    }
                }
                SourceEvent::Request(section, key, wildcard) => {
                    let state = states.entry(*section).or_default();
                    if *wildcard {
                        state.wildcard = true;
                    } else if names.contains(key.name.as_str())
                        && !state.first.contains_key(&key.name)
                    {
                        state
                            .first
                            .insert(key.name.clone(), state.declared.get(&key.name).cloned());
                    }
                }
            }
        }
        let mut seen = BTreeSet::new();
        let mut relevant = BTreeSet::new();
        for (section, resources) in &trace.resources {
            let address = Arc::as_ptr(resources) as usize;
            if seen.insert(address) {
                self.work += resources
                    .iter()
                    .map(|uri| uri.as_str().len() + 96)
                    .sum::<usize>();
                if self.work > LIMIT {
                    return Err(());
                }
                for uri in resources.iter() {
                    if self.loaded.insert(uri.clone()) {
                        self.charge(uri.as_str().len() + 96)?;
                    }
                }
            }
            if !resources.contains(origin) {
                continue;
            }
            self.contexts += 1;
            if self.contexts > 100_000 {
                return Err(());
            }
            // Numeric namespaces remain independent even when they share one
            // allocation. Base database membership needs scanning only once.
            if relevant.insert(address) {
                self.work += resources.len() * 96
                    + self.common.as_ref().map_or(0, |common| common.len() * 96);
                if self.work > LIMIT {
                    return Err(());
                }
                for uri in resources.iter() {
                    if self.all.insert(uri.clone()) {
                        self.charge(2 * (uri.as_str().len() + 96))?;
                    }
                }
                if let Some(common) = self.common.as_mut() {
                    common.retain(|uri| resources.contains(uri));
                } else {
                    self.common = Some(resources.as_ref().clone());
                }
            }
            let Some(state) = states.get(section) else {
                continue;
            };
            for (name, final_set) in &state.declared {
                let set = match state.first.get(name) {
                    Some(None) => {
                        if state.wildcard && self.blocked.insert(name.clone()) {
                            self.charge(name.len() + 64)?;
                        }
                        continue;
                    }
                    Some(Some(first)) => first,
                    None => final_set,
                };
                if self.overrides.insert(name.clone()) {
                    self.charge(name.len() + 64)?;
                }
                let id = (set.key.uri.clone(), set.key.span.start, set.key.span.end);
                if !self.sources.contains_key(&id) {
                    self.charge(id.0.as_str().len() + 128)?;
                    self.sources.insert(id.clone(), (set.clone(), 0));
                }
                self.sources.get_mut(&id).ok_or(())?.1 += 1;
            }
        }
        Ok(())
    }
    fn candidates(
        mut self,
        ws: &Workspace,
        origin: &Url,
        field: &str,
    ) -> Result<BTreeMap<String, Candidate>> {
        if self.contexts == 0 {
            return Err(());
        }
        let common = self.common.take().ok_or(())?;
        let mut base = BTreeMap::<String, Option<Candidate>>::new();
        let mut bytes = 0usize;
        let loaded = std::mem::take(&mut self.loaded);
        for uri in loaded {
            let doc = ws.documents.get(&uri).ok_or(())?;
            bytes += doc.text.source.len();
            if bytes > 16 * 1024 * 1024
                || doc.kind != DocumentKind::Bibtex
                || (uri != *origin && !doc.syntax.diagnostics.is_empty())
            {
                return Err(());
            }
            for field in doc
                .syntax
                .bibliography_completion
                .entries
                .iter()
                .flat_map(|entry| &entry.fields)
            {
                self.work += 16;
                if self.work > LIMIT || field.name == "ids" {
                    return Err(());
                }
            }
            let mut seen = BTreeSet::new();
            for entry in &doc.syntax.citations {
                if !syntax::is_static_citation_key(&entry.name) {
                    continue;
                }
                if !seen.insert(entry.name.as_str()) && self.blocked.insert(entry.name.clone()) {
                    self.charge(entry.name.len() + 64)?;
                }
                if !self.all.contains(&uri) {
                    continue;
                }
                self.charge(entry.name.len() + entry.detail.len() + uri.as_str().len() + 128)?;
                if !common.contains(&uri)
                    || base.contains_key(&entry.name)
                    || (field == "xdata"
                        && !crate::bib_completion::is_xdata_entry(doc, &entry.span))
                {
                    base.insert(entry.name.clone(), None);
                } else {
                    base.insert(
                        entry.name.clone(),
                        Some(Candidate {
                            uri: uri.clone(),
                            detail: entry.detail.clone(),
                        }),
                    );
                }
            }
        }
        // Biber resolves crossref/xref parents in the database before it
        // materializes dynamic sets. A source-only key is not a valid parent.
        // XDATA and the related/nested-set transformations cannot consume an
        // active dynamic set as an ordinary parent either.
        self.sources.retain(|_, (set, _)| {
            matches!(field, "crossref" | "xref")
                && base.get(&set.key.name).is_some_and(Option::is_some)
        });
        let mut answer: BTreeMap<_, _> = base
            .into_iter()
            .filter_map(|(name, candidate)| {
                if self.overrides.contains(&name) || self.blocked.contains(&name) {
                    return None;
                }
                Some((name, candidate?))
            })
            .collect();
        for (_, (set, count)) in std::mem::take(&mut self.sources) {
            if count != self.contexts || self.blocked.contains(&set.key.name) {
                continue;
            }
            self.charge(
                set.key.name.len()
                    + set.key.uri.as_str().len()
                    + 160
                    + set
                        .members
                        .iter()
                        .take(16)
                        .map(|member| member.name.len() + 2)
                        .sum::<usize>(),
            )?;
            let mut members = set
                .members
                .iter()
                .take(16)
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if set.members.len() > 16 {
                members.push_str(", …");
            }
            answer.insert(
                set.key.name.clone(),
                Candidate {
                    uri: set.key.uri.clone(),
                    detail: format!("Dynamic bibliography set: {members}"),
                },
            );
        }
        Ok(answer)
    }
}
pub(crate) fn completion(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<CompletionItem>> {
    let doc = ws
        .documents
        .get(uri)
        .filter(|doc| doc.kind == DocumentKind::Bibtex)?;
    let field =
        doc.syntax.citation_fields.iter().find(|field| {
            field.argument.span.start <= offset && offset <= field.argument.span.end
        })?;
    if !ws.documents.values().any(|doc| {
        doc.kind == DocumentKind::Latex
            && doc.syntax.commands.iter().any(|command| {
                matches!(command.name.as_str(), "defbibentryset" | "csuse" | "csname")
            })
    }) {
        return None;
    }
    let (selected, projects) = match bib_section_keys::section_projects(ws, uri) {
        Some(projects) => projects,
        None if field.name == "xdata" => return Some(Vec::new()),
        None => return None,
    };
    if (!selected && field.name != "xdata")
        || !projects
            .iter()
            .any(|project| bib_section_keys::has_source_sets(&project.docs, &project.signatures))
    {
        return None;
    }
    Some(
        (|| {
            if !ws.index_truncated.is_empty()
                || !crate::bib_section_symbols::valid_relation_completion_field(doc, field)
            {
                return None;
            }
            let mut catalog = Catalog::default();
            for project in projects {
                let trace = bib_section_keys::source_trace(
                    ws,
                    &project.root,
                    &project.docs,
                    &project.signatures,
                )?;
                catalog.trace(&trace, uri).ok()?;
            }
            let candidates = catalog.candidates(ws, uri, &field.name).ok()?;
            Some(crate::features::bibliography_relation_edits(
                doc,
                offset,
                candidates.iter().map(|(name, candidate)| {
                    (&candidate.uri, name.as_str(), candidate.detail.as_str())
                }),
                ws.settings(uri).indexing.completion_limit.saturating_add(1),
            ))
        })()
        .unwrap_or_default(),
    )
}
