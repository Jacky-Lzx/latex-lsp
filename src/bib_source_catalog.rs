//! Bounded candidate identities for relations and TeX in source-set projects.
use crate::{
    bib_section_keys::{self, SourceCompletion, SourceEvent, SourceSet, SourceTrace},
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
#[derive(Default, Clone)]
struct State {
    declared: BTreeMap<String, Arc<SourceSet>>,
    first: BTreeMap<String, Option<Arc<SourceSet>>>,
    wildcard: bool,
}
pub(crate) struct Candidate {
    pub uri: Url,
    pub detail: String,
}
#[derive(Default)]
pub(crate) struct Catalog {
    contexts: usize,
    all: BTreeSet<Url>,
    common: Option<BTreeSet<Url>>,
    loaded: BTreeSet<Url>,
    alias_resources: BTreeSet<Arc<BTreeSet<Url>>>,
    overrides: BTreeSet<String>,
    source_names: BTreeSet<String>,
    source_members: BTreeSet<String>,
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
        for name in &names {
            if self.source_names.insert((*name).to_string()) {
                self.charge(name.len() + 64)?;
            }
        }
        for member in trace
            .events
            .iter()
            .filter_map(|event| match event {
                SourceEvent::Set(_, set) => Some(&set.members),
                _ => None,
            })
            .flatten()
        {
            if self.source_members.insert(member.name.clone()) {
                self.charge(member.name.len() + 64)?;
            }
        }
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
                if self.alias_resources.insert(resources.clone()) {
                    self.charge(
                        resources
                            .iter()
                            .map(|uri| uri.as_str().len() + 64)
                            .sum::<usize>()
                            + 64,
                    )?;
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
    pub(crate) fn trace_completion(&mut self, completion: &SourceCompletion) -> Result<()> {
        let trace = &completion.trace;
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
        for name in &names {
            if self.source_names.insert((*name).to_string()) {
                self.charge(name.len() + 64)?;
            }
        }
        for member in trace
            .events
            .iter()
            .filter_map(|event| match event {
                SourceEvent::Set(_, set) => Some(&set.members),
                _ => None,
            })
            .flatten()
        {
            if self.source_members.insert(member.name.clone()) {
                self.charge(member.name.len() + 64)?;
            }
        }
        let mut first_probes = BTreeMap::<u32, usize>::new();
        for &(section, event) in &trace.probes {
            first_probes
                .entry(section)
                .and_modify(|first| *first = (*first).min(event))
                .or_insert(event);
        }
        let mut by_event = BTreeMap::<usize, Vec<u32>>::new();
        for (section, event) in first_probes {
            by_event.entry(event).or_default().push(section);
        }
        let mut states = BTreeMap::<u32, State>::new();
        let mut snapshots = BTreeMap::<u32, State>::new();
        for index in 0..=trace.events.len() {
            if let Some(sections) = by_event.get(&index) {
                for section in sections {
                    let state = states.entry(*section).or_default();
                    self.work += state
                        .declared
                        .keys()
                        .chain(state.first.keys())
                        .map(|name| name.len() + 192)
                        .sum::<usize>()
                        + 128;
                    if self.work > LIMIT {
                        return Err(());
                    }
                    snapshots.insert(*section, state.clone());
                }
            }
            let Some(event) = trace.events.get(index) else {
                break;
            };
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
        if completion.member {
            for name in &names {
                if self.blocked.insert((*name).into()) {
                    self.charge(name.len() + 64)?;
                }
            }
        }
        let mut seen = BTreeSet::new();
        for resources in trace.resources.values() {
            if seen.insert(Arc::as_ptr(resources) as usize) {
                self.work += resources
                    .iter()
                    .map(|uri| uri.as_str().len() + 96)
                    .sum::<usize>();
                if self.work > LIMIT {
                    return Err(());
                }
                if self.alias_resources.insert(resources.clone()) {
                    self.charge(
                        resources
                            .iter()
                            .map(|uri| uri.as_str().len() + 64)
                            .sum::<usize>()
                            + 64,
                    )?;
                }
                for uri in resources.iter() {
                    if self.loaded.insert(uri.clone()) {
                        self.charge(uri.as_str().len() + 96)?;
                    }
                }
            }
        }
        seen.clear();
        for (section, before) in snapshots {
            let resources = trace.resources.get(&section).ok_or(())?;
            self.contexts += 1;
            if self.contexts > 100_000 {
                return Err(());
            }
            if seen.insert(Arc::as_ptr(resources) as usize) {
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
            if completion.member {
                continue;
            }
            let Some(final_state) = states.get(&section) else {
                continue;
            };
            for name in final_state.declared.keys() {
                let set = match before.first.get(name) {
                    Some(first) => first.as_ref(),
                    None => before.declared.get(name),
                };
                let Some(set) = set else {
                    if final_state.wildcard && self.blocked.insert(name.clone()) {
                        self.charge(name.len() + 64)?;
                    }
                    continue;
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
    pub(crate) fn candidates(
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
        let source_names = self.source_names.iter().map(String::as_str).collect();
        let source_members = self.source_members.iter().map(String::as_str).collect();
        for resources in &self.alias_resources {
            self.work +=
                crate::bib_source_aliases::certify(ws, resources, &source_names, &source_members)
                    .ok_or(())?;
            if self.work > LIMIT {
                return Err(());
            }
        }
        if self.work > LIMIT {
            return Err(());
        }
        for uri in loaded {
            let doc = ws.documents.get(&uri).ok_or(())?;
            bytes += doc.text.source.len();
            if bytes > 16 * 1024 * 1024
                || doc.kind != DocumentKind::Bibtex
                || (uri != *origin && !doc.syntax.diagnostics.is_empty())
            {
                return Err(());
            }
            let mut seen = BTreeSet::new();
            for entry in doc
                .syntax
                .citations
                .iter()
                .chain(&doc.syntax.bibliography_aliases)
            {
                if !syntax::is_static_citation_key(&entry.name) {
                    continue;
                }
                if !seen.insert(entry.name.as_str()) && self.blocked.insert(entry.name.clone()) {
                    self.charge(entry.name.len() + 64)?;
                }
                if !self.all.contains(&uri) {
                    continue;
                }
                let owner = crate::bib_source_aliases::owner(doc, entry);
                // A prospective alias member can materialize a source owner,
                // creating a nested set even if no existing member does so.
                if owner.is_some_and(|owner| self.source_names.contains(owner)) && field == "member"
                    || owner.is_some_and(|owner| self.overrides.contains(owner))
                        && matches!(field, "xdata" | "related" | "entryset")
                {
                    if self.blocked.insert(entry.name.clone()) {
                        self.charge(entry.name.len() + 64)?;
                    }
                    continue;
                }
                let detail =
                    if let Some(owner) = owner.filter(|owner| self.source_names.contains(*owner)) {
                        format!("Bibliography alias of {owner}")
                    } else {
                        crate::bib_source_aliases::detail(doc, entry)
                    };
                self.charge(entry.name.len() + detail.len() + uri.as_str().len() + 128)?;
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
                            detail,
                        }),
                    );
                }
            }
        }
        // Biber resolves crossref/xref parents in the database before it
        // materializes dynamic sets. A source-only key is not a valid parent.
        // XDATA and the related/nested-set transformations cannot consume an
        // active dynamic set as an ordinary parent either. TeX citation keys
        // can request a source-only set; member completion excludes all sets.
        self.sources.retain(|_, (set, _)| {
            field == "cite"
                || (matches!(field, "crossref" | "xref")
                    && base.get(&set.key.name).is_some_and(Option::is_some))
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
    let (selected, projects) = match bib_section_keys::source_projects(ws, uri) {
        Some(projects) => projects,
        None => return Some(Vec::new()),
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
