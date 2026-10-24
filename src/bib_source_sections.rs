//! Certified read-only identities for literal source sets in selected sections.
use crate::{
    bib_section_keys::{self, SourceEvent, SourceKey, SourceSet, SourceTrace},
    named_references::Index,
    workspace::{DocumentKind, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::Arc,
};
use tower_lsp::lsp_types::*;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    uri: Url,
    start: usize,
    end: usize,
}
impl Identity {
    fn source(key: &SourceKey) -> Self {
        Self {
            uri: key.uri.clone(),
            start: key.span.start,
            end: key.span.end,
        }
    }
    fn location(&self, ws: &Workspace) -> Location {
        Location::new(
            self.uri.clone(),
            ws.documents[&self.uri].text.range(self.start, self.end),
        )
    }
}
type Result<T> = std::result::Result<T, ()>;
#[derive(Default)]
struct Databases {
    matches: BTreeMap<Url, Option<Identity>>,
    bytes: usize,
    work: usize,
}
impl Databases {
    fn prepare(&mut self, ws: &Workspace, uri: &Url, name: &str) -> Result<()> {
        if self.matches.contains_key(uri) {
            return Ok(());
        }
        let doc = ws.documents.get(uri).ok_or(())?;
        self.bytes += doc.text.source.len();
        if self.bytes > 16 * 1024 * 1024
            || doc.kind != DocumentKind::Bibtex
            || !doc.syntax.diagnostics.is_empty()
            || !doc.syntax.bibliography_aliases.is_empty()
        {
            return Err(());
        }
        let mut matched = doc
            .syntax
            .citations
            .iter()
            .filter(|entry| entry.name == name);
        let found = matched.next().map(|entry| Identity {
            uri: uri.clone(),
            start: entry.span.start,
            end: entry.span.end,
        });
        if matched.next().is_some() {
            return Err(());
        }
        self.work += uri.as_str().len() * 2 + 160;
        if self.work > 32 * 1024 * 1024 {
            return Err(());
        }
        self.matches.insert(uri.clone(), found);
        Ok(())
    }
    fn bind(&self, view: &BTreeSet<Url>) -> Result<Option<Identity>> {
        let mut answer = None;
        for uri in view {
            if let Some(found) = self.matches.get(uri).ok_or(())? {
                if answer.is_some() {
                    return Err(());
                }
                answer = Some(found.clone());
            }
        }
        Ok(answer)
    }
}
#[derive(Default)]
struct State {
    declared: Option<Arc<SourceSet>>,
    first: Option<Option<Identity>>,
    wildcard: bool,
    early: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum QueryKind {
    Declaration,
    Source,
    Relation,
}
struct Answer {
    span: Range<usize>,
    identity: Identity,
    sets: BTreeMap<Identity, Arc<SourceSet>>,
}

fn resolve(
    trace: &SourceTrace,
    databases: &Databases,
    uri: &Url,
    query: &Range<usize>,
    name: &str,
    kind: QueryKind,
) -> Result<(Identity, BTreeMap<Identity, Arc<SourceSet>>)> {
    let mut states = BTreeMap::<u32, State>::new();
    let mut declarations = BTreeMap::<(u32, &str), &SourceSet>::new();
    let mut sets = BTreeMap::new();
    let names: BTreeSet<_> = trace
        .events
        .iter()
        .filter_map(|event| match event {
            SourceEvent::Set(_, set) => Some(set.key.name.as_str()),
            _ => None,
        })
        .collect();
    let mut views = if kind == QueryKind::Relation {
        trace
            .resources
            .iter()
            .filter(|(_, resources)| resources.contains(uri))
            .map(|(section, _)| *section)
            .collect()
    } else {
        BTreeSet::new()
    };
    let mut own = None;
    let mut bindings = BTreeMap::<usize, Option<Identity>>::new();
    let mut binding = |section: u32| -> Result<Option<Identity>> {
        let resources = trace.resources.get(&section).ok_or(())?;
        let address = Arc::as_ptr(resources) as usize;
        if let Some(answer) = bindings.get(&address) {
            return Ok(answer.clone());
        }
        let answer = databases.bind(resources)?;
        bindings.insert(address, answer.clone());
        Ok(answer)
    };
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
                if declarations
                    .insert((*section, &set.key.name), set)
                    .is_some_and(|previous| previous.key != set.key)
                {
                    return Err(());
                }
                let identity = Identity::source(&set.key);
                if kind == QueryKind::Declaration && set.key.uri == *uri && set.key.span == *query {
                    own = Some(identity.clone());
                }
                if kind == QueryKind::Source
                    && set
                        .members
                        .iter()
                        .any(|key| key.uri == *uri && key.span == *query && key.name == name)
                {
                    views.insert(*section);
                }
                if set.key.name == name {
                    states.entry(*section).or_default().declared = Some(set.clone());
                }
                sets.insert(identity, set.clone());
            }
            SourceEvent::Request(section, key, wildcard) => {
                let state = states.entry(*section).or_default();
                if *wildcard {
                    state.wildcard = true;
                    continue;
                }
                if key.name != name {
                    continue;
                }
                if key.uri == *uri && key.span == *query {
                    views.insert(*section);
                }
                if state.first.is_none() {
                    state.early = state.declared.is_none();
                    state.first = Some(if let Some(set) = &state.declared {
                        Some(Identity::source(&set.key))
                    } else {
                        binding(*section)?
                    });
                }
            }
        }
    }
    if kind == QueryKind::Declaration {
        return Ok((own.ok_or(())?, sets));
    }
    let mut answer = None;
    for section in views {
        // Relations cannot use a source-only key as a database parent. The
        // backend checks that namespace before materializing dynamic sets.
        if kind == QueryKind::Relation && binding(section)?.is_none() {
            return Err(());
        }
        let state = states.entry(section).or_default();
        // Native behavior for explicit requests combined with later wildcard
        // loading needs a separate proof; keep that combination protected.
        if state.early && state.wildcard && state.declared.is_some() {
            return Err(());
        }
        let found = if let Some(found) = &state.first {
            found.clone()
        } else if let Some(set) = &state.declared {
            Some(Identity::source(&set.key))
        } else {
            binding(section)?
        }
        .ok_or(())?;
        if answer.as_ref().is_some_and(|answer| *answer != found) {
            return Err(());
        }
        answer = Some(found);
    }
    Ok((answer.ok_or(())?, sets))
}

fn target(ws: &Workspace, uri: &Url, offset: usize) -> Option<Option<Answer>> {
    let doc = ws.documents.get(uri)?;
    if !matches!(doc.kind, DocumentKind::Latex | DocumentKind::Bibtex) {
        return None;
    }
    // Database declarations remain lexical and need no execution analysis.
    if doc.kind == DocumentKind::Bibtex
        && !doc
            .syntax
            .citation_references
            .iter()
            .any(|entry| entry.span.start <= offset && offset <= entry.span.end)
    {
        return None;
    }
    if !ws.documents.values().any(|doc| {
        doc.kind == DocumentKind::Latex
            && doc.syntax.commands.iter().any(|command| {
                matches!(command.name.as_str(), "defbibentryset" | "csuse" | "csname")
            })
    }) {
        return None;
    }
    let (selected, projects) = bib_section_keys::section_projects(ws, uri)?;
    if !selected
        || !projects
            .iter()
            .any(|project| bib_section_keys::has_source_sets(&project.docs, &project.signatures))
    {
        return None;
    }
    let (name, span, kind) = if doc.kind == DocumentKind::Bibtex {
        let candidate = doc
            .syntax
            .citation_references
            .iter()
            .find(|entry| entry.span.start <= offset && offset <= entry.span.end)?;
        (
            candidate.name.clone(),
            candidate.span.clone(),
            QueryKind::Relation,
        )
    } else {
        let declaration = projects
            .iter()
            .find_map(|project| bib_section_keys::source_set_key(doc, &project.signatures, offset));
        if let Some((name, span)) = declaration {
            (name, span, QueryKind::Declaration)
        } else {
            let mut contexts = BTreeMap::<BTreeSet<Url>, BTreeSet<Url>>::new();
            for project in &projects {
                let scope = ws.project_documents(&project.root);
                contexts.entry(scope.clone()).or_default().extend(scope);
            }
            let lexical = Index::for_section_keys(ws, uri, contexts);
            let (citation, span) = lexical.target(uri, offset)?;
            if !citation {
                return None;
            }
            (
                doc.text.source[span.clone()].to_string(),
                span,
                QueryKind::Source,
            )
        }
    };
    Some((|| {
        if kind == QueryKind::Relation {
            crate::bib_section_references::certified_relation(uri, doc, offset)??;
        }
        let mut databases = Databases::default();
        let mut work = 0usize;
        let mut answer: Option<Answer> = None;
        for project in projects {
            let trace = bib_section_keys::source_trace(
                ws,
                &project.root,
                &project.docs,
                &project.signatures,
            )?;
            work += trace.work;
            if work > 32 * 1024 * 1024 {
                return None;
            }
            // Equal numeric sections share resource allocations. Inspect each
            // allocation once rather than expanding every section membership.
            let mut views = BTreeMap::new();
            for resources in trace.resources.values() {
                views
                    .entry(Arc::as_ptr(resources) as usize)
                    .or_insert(resources);
            }
            for resources in views.values() {
                for resource in resources.iter() {
                    databases.prepare(ws, resource, &name).ok()?;
                }
            }
            let (identity, sets) = resolve(&trace, &databases, uri, &span, &name, kind).ok()?;
            if let Some(previous) = answer.as_mut() {
                if previous.identity != identity {
                    return None;
                }
                previous.sets.extend(sets);
            } else {
                answer = Some(Answer {
                    span: span.clone(),
                    identity,
                    sets,
                });
            }
        }
        answer
    })())
}

pub(crate) fn definitions(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    Some(
        target(ws, uri, offset)?.map_or_else(Vec::new, |answer| vec![answer.identity.location(ws)]),
    )
}
pub(crate) fn hover(ws: &Workspace, uri: &Url, offset: usize) -> Option<Option<Hover>> {
    Some(target(ws, uri, offset)?.and_then(|answer| {
        let doc = ws.documents.get(&answer.identity.uri)?;
        let name = &doc.text.source[answer.identity.start..answer.identity.end];
        let detail = if let Some(set) = answer.sets.get(&answer.identity) {
            let mut names = set
                .members
                .iter()
                .take(16)
                .map(|key| key.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if set.members.len() > 16 {
                names.push_str(", …");
            }
            format!("Dynamic bibliography set: {names}")
        } else {
            doc.syntax
                .citations
                .iter()
                .find(|entry| entry.span == (answer.identity.start..answer.identity.end))?
                .detail
                .clone()
        };
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::PlainText,
                value: format!(
                    "{name}\n\n{detail}\n{}:{}",
                    answer.identity.uri.path(),
                    doc.text.position(answer.identity.start).line + 1
                ),
            }),
            range: Some(
                ws.documents[uri]
                    .text
                    .range(answer.span.start, answer.span.end),
            ),
        })
    }))
}
