//! Complete, bounded occurrence graphs for bibliography keys in reference sections.
use crate::{
    bib_section_keys::{self, KeyGraph},
    named_references::Index,
    signature::CommandSignatures,
    syntax,
    workspace::{Document, DocumentKind, Workspace},
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::Arc,
};
use tower_lsp::{
    jsonrpc,
    lsp_types::{Location, PrepareRenameResponse, TextEdit, Url, WorkspaceEdit},
};

type Result<T> = std::result::Result<T, ()>;
type ResourceBindings = BTreeMap<Url, Result<Option<Identity>>>;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    uri: Url,
    start: usize,
    end: usize,
}
#[derive(Clone)]
struct Occurrence {
    name: String,
    field: String,
    span: Range<usize>,
}
struct Database {
    declarations: BTreeMap<String, Vec<Identity>>,
    references: Vec<Occurrence>,
}
fn relation(name: &str) -> bool {
    matches!(name, "crossref" | "xref" | "xdata" | "related" | "entryset")
}
impl Database {
    fn new(uri: &Url, doc: &Document) -> Result<Self> {
        if !doc.syntax.diagnostics.is_empty() {
            return Err(());
        }
        let mut declarations = BTreeMap::<String, Vec<Identity>>::new();
        let mut stored = 0usize;
        for entry in doc
            .syntax
            .citations
            .iter()
            .chain(&doc.syntax.bibliography_aliases)
        {
            stored += entry.name.len() + uri.as_str().len() + 96;
            if stored > 32 * 1024 * 1024 || !syntax::is_static_citation_key(&entry.name) {
                return Err(());
            }
            declarations
                .entry(entry.name.clone())
                .or_default()
                .push(Identity {
                    uri: uri.clone(),
                    start: entry.span.start,
                    end: entry.span.end,
                });
        }
        let mut references = Vec::new();
        let mut fields = doc
            .syntax
            .bibliography_completion
            .entries
            .iter()
            .flat_map(|entry| &entry.fields)
            .peekable();
        let mut examined = 0usize;
        let mut closed = doc.syntax.bibliography_entries.iter().peekable();
        for owner in &doc.syntax.bibliography_completion.types {
            if owner.key.is_none() {
                continue;
            }
            let body = owner.entry_span.as_ref().ok_or(())?;
            while closed
                .peek()
                .is_some_and(|(span, _)| span.start < body.start)
            {
                closed.next();
            }
            if !closed
                .peek()
                .is_some_and(|(span, comment)| span == body && !comment)
            {
                return Err(());
            }
            while fields
                .peek()
                .is_some_and(|field| field.span.start < body.start)
            {
                fields.next();
            }
            let mut seen = BTreeSet::new();
            while fields
                .peek()
                .is_some_and(|field| field.span.start < body.end)
            {
                let field = fields.next().ok_or(())?;
                examined += 1;
                if examined > 100_000 {
                    return Err(());
                }
                if !relation(&field.name) && field.name != "ids" {
                    continue;
                }
                if !seen.insert(field.name.as_str()) || !field.assigned || !field.value_valid {
                    return Err(());
                }
                let assignment = syntax::skip_space(&doc.text.source, field.span.end);
                let start = syntax::skip_space(&doc.text.source, assignment + 1);
                let argument =
                    syntax::bib_literal_mode(&doc.text.source, start, field.range.end, false)
                        .ok_or(())?;
                if argument.value.contains(['\\', '{', '}', '%', '#']) {
                    return Err(());
                }
                let multiple = matches!(
                    field.name.as_str(),
                    "xdata" | "related" | "entryset" | "ids"
                );
                let mut offset = argument.span.start;
                if !multiple && argument.value.contains(',') {
                    return Err(());
                }
                for member in argument.value.split(',') {
                    let span =
                        syntax::trimmed_span(&doc.text.source, offset..offset + member.len());
                    let key = &doc.text.source[span.clone()];
                    if !key.is_empty() && !syntax::is_static_citation_key(key) {
                        return Err(());
                    }
                    if field.name != "ids" && !key.is_empty() {
                        stored += key.len() + field.name.len() + 64;
                        if stored > 32 * 1024 * 1024 || references.len() >= 100_000 {
                            return Err(());
                        }
                        references.push(Occurrence {
                            name: key.into(),
                            field: field.name.clone(),
                            span,
                        });
                    }
                    offset += member.len() + 1;
                }
            }
        }
        Ok(Self {
            declarations,
            references,
        })
    }
}

// Resource allocations can be shared, but each source execution section has
// its own namespace and must have a distinct binding-cache identity.
#[derive(Default)]
struct Namespace {
    declared: BTreeMap<String, Identity>,
    first: BTreeMap<String, Option<Identity>>,
    wildcard: bool,
}
struct View {
    resources: Arc<BTreeSet<Url>>,
    namespace: Option<Namespace>,
}
impl std::ops::Deref for View {
    type Target = BTreeSet<Url>;
    fn deref(&self) -> &Self::Target {
        &self.resources
    }
}
type OccurrenceKey = (Url, usize, usize);
struct Graph {
    sections: Vec<Arc<View>>,
    occurrences: BTreeMap<OccurrenceKey, Vec<Arc<View>>>,
    sources: Option<BTreeSet<Identity>>,
    nonloading: BTreeSet<OccurrenceKey>,
    members: BTreeMap<OccurrenceKey, Identity>,
    work: usize,
}
impl Graph {
    fn resources(graph: KeyGraph) -> Result<Self> {
        let views: BTreeMap<_, _> = graph
            .sections
            .into_iter()
            .map(|resources| {
                (
                    resources.clone(),
                    Arc::new(View {
                        resources,
                        namespace: None,
                    }),
                )
            })
            .collect();
        let occurrences = graph
            .occurrences
            .into_iter()
            .map(|(key, resources)| {
                let visits = resources
                    .iter()
                    .map(|resources| views.get(resources).cloned().ok_or(()))
                    .collect::<Result<Vec<_>>>()?;
                Ok((key, visits))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            sections: views.into_values().collect(),
            occurrences,
            sources: None,
            nonloading: BTreeSet::new(),
            members: BTreeMap::new(),
            work: 0,
        })
    }
    fn source(trace: bib_section_keys::SourceTrace) -> Result<Self> {
        use bib_section_keys::SourceEvent;
        let names: BTreeSet<_> = trace
            .events
            .iter()
            .filter_map(|event| match event {
                SourceEvent::Set(_, set) => Some(set.key.name.as_str()),
                _ => None,
            })
            .collect();
        let mut namespaces = BTreeMap::<u32, Namespace>::new();
        let mut sources = BTreeSet::new();
        let mut members = BTreeMap::new();
        let mut visits = BTreeMap::<OccurrenceKey, BTreeSet<u32>>::new();
        let mut nonloading = BTreeSet::new();
        for event in &trace.events {
            match event {
                SourceEvent::Set(section, set) => {
                    if set
                        .members
                        .iter()
                        .any(|key| names.contains(key.name.as_str()))
                    {
                        return Err(());
                    }
                    let identity = Identity {
                        uri: set.key.uri.clone(),
                        start: set.key.span.start,
                        end: set.key.span.end,
                    };
                    if namespaces
                        .entry(*section)
                        .or_default()
                        .declared
                        .insert(set.key.name.clone(), identity.clone())
                        .is_some_and(|previous| previous != identity)
                    {
                        return Err(());
                    }
                    sources.insert(identity.clone());
                    for key in &set.members {
                        members.insert(
                            (key.uri.clone(), key.span.start, key.span.end),
                            identity.clone(),
                        );
                        visits
                            .entry((key.uri.clone(), key.span.start, key.span.end))
                            .or_default()
                            .insert(*section);
                    }
                }
                SourceEvent::Query(section, key) => {
                    let occurrence = (key.uri.clone(), key.span.start, key.span.end);
                    nonloading.insert(occurrence.clone());
                    visits.entry(occurrence).or_default().insert(*section);
                }
                SourceEvent::Request(section, key, wildcard) => {
                    let state = namespaces.entry(*section).or_default();
                    if *wildcard {
                        state.wildcard = true;
                    } else {
                        if names.contains(key.name.as_str()) && !state.first.contains_key(&key.name)
                        {
                            state
                                .first
                                .insert(key.name.clone(), state.declared.get(&key.name).cloned());
                        }
                        visits
                            .entry((key.uri.clone(), key.span.start, key.span.end))
                            .or_default()
                            .insert(*section);
                    }
                }
            }
        }
        let views: BTreeMap<_, _> = trace
            .resources
            .into_iter()
            .map(|(section, resources)| {
                (
                    section,
                    Arc::new(View {
                        resources,
                        namespace: Some(namespaces.remove(&section).unwrap_or_default()),
                    }),
                )
            })
            .collect();
        let occurrences = visits
            .into_iter()
            .map(|(key, sections)| {
                let visits = sections
                    .into_iter()
                    .map(|section| views.get(&section).cloned().ok_or(()))
                    .collect::<Result<Vec<_>>>()?;
                Ok((key, visits))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            sections: views.into_values().collect(),
            occurrences,
            sources: Some(sources),
            nonloading,
            members,
            work: trace.work,
        })
    }
}

struct Project {
    scope: BTreeSet<Url>,
    graph: Graph,
    resources: BTreeSet<Url>,
}
struct Graphs<'a> {
    ws: &'a Workspace,
    regions: BTreeMap<Url, BTreeSet<Url>>,
    projects: BTreeMap<Url, Project>,
    databases: BTreeMap<Url, Database>,
    bytes: usize,
    stored: usize,
    name: Option<String>,
    // Bindings belong to `name`; changing a rename candidate clears this cache.
    bindings: RefCell<BTreeMap<usize, Result<Option<Identity>>>>,
    work: Cell<usize>,
    database_bindings: RefCell<BTreeMap<usize, Result<Option<Identity>>>>,
    relation_bindings: RefCell<BTreeMap<Url, ResourceBindings>>,
}
impl<'a> Graphs<'a> {
    fn new(ws: &'a Workspace) -> Result<Self> {
        if !ws.index_truncated.is_empty() {
            return Err(());
        }
        let mut regions = ws.saved_projects();
        for root in ws.documents.keys().filter(|root| ws.is_root(root)) {
            regions
                .entry(root.clone())
                .or_insert_with(|| ws.project_documents(root));
        }
        let memberships: usize = regions.values().map(BTreeSet::len).sum();
        let stored: usize = regions
            .values()
            .flatten()
            .map(|uri| uri.as_str().len() + 64)
            .sum();
        if memberships > 100_000 || stored > 32 * 1024 * 1024 {
            return Err(());
        }
        Ok(Self {
            ws,
            regions,
            projects: BTreeMap::new(),
            databases: BTreeMap::new(),
            bytes: 0,
            stored,
            name: None,
            bindings: RefCell::new(BTreeMap::new()),
            work: Cell::new(0),
            database_bindings: RefCell::new(BTreeMap::new()),
            relation_bindings: RefCell::new(BTreeMap::new()),
        })
    }
    fn consumers(&self, uri: &Url) -> Vec<Url> {
        self.ws.source_roots_in(uri, &self.regions)
    }
    fn lexical(&self, origin: &Url) -> Index {
        let mut contexts = BTreeMap::<BTreeSet<Url>, BTreeSet<Url>>::new();
        for scope in self.regions.values() {
            contexts
                .entry(scope.clone())
                .or_default()
                .extend(scope.iter().cloned());
        }
        Index::for_section_keys(self.ws, origin, contexts)
    }
    fn ensure(&mut self, root: &Url) -> Result<()> {
        if self.projects.contains_key(root) {
            return Ok(());
        }
        if self.projects.len() >= 64 {
            return Err(());
        }
        let mut scope = self
            .regions
            .get(root)
            .cloned()
            .unwrap_or_else(|| self.ws.project_documents(root));
        scope.insert(root.clone());
        let docs = crate::source_inputs::bounded_latex_sources(self.ws, &scope).ok_or(())?;
        self.bytes += docs
            .iter()
            .map(|(_, doc)| doc.text.source.len())
            .sum::<usize>();
        if self.bytes > 16 * 1024 * 1024 || !bib_section_keys::signature_budget(&docs) {
            return Err(());
        }
        let signatures = CommandSignatures::new(
            docs.iter()
                .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
        );
        let graph = if bib_section_keys::has_source_sets(&docs, &signatures) {
            Graph::source(
                bib_section_keys::source_trace(self.ws, root, &docs, &signatures).ok_or(())?,
            )?
        } else {
            Graph::resources(
                bib_section_keys::key_graph(self.ws, root, &docs, &signatures).ok_or(())?,
            )?
        };
        self.work.set(self.work.get() + graph.work);
        self.stored += graph.work;
        if self.work.get() > 32 * 1024 * 1024 {
            return Err(());
        }
        let mut allocations = BTreeSet::new();
        self.stored += graph
            .sections
            .iter()
            .filter(|view| allocations.insert(Arc::as_ptr(&view.resources) as usize))
            .flat_map(|view| view.iter())
            .map(|uri| uri.as_str().len() + 64)
            .sum::<usize>();
        self.stored += graph
            .occurrences
            .iter()
            .map(|((uri, _, _), visits)| uri.as_str().len() + 128 + visits.len() * 32)
            .sum::<usize>();
        self.stored += graph
            .nonloading
            .iter()
            .map(|(uri, _, _)| uri.as_str().len() + 96)
            .sum::<usize>();
        self.stored += graph
            .members
            .iter()
            .map(|((uri, _, _), owner)| uri.as_str().len() + owner.uri.as_str().len() + 192)
            .sum::<usize>();
        if self.stored > 32 * 1024 * 1024 {
            return Err(());
        }
        allocations.clear();
        let resources: BTreeSet<_> = graph
            .sections
            .iter()
            .filter(|view| allocations.insert(Arc::as_ptr(&view.resources) as usize))
            .flat_map(|view| view.iter())
            .cloned()
            .collect();
        self.stored += resources
            .iter()
            .map(|uri| uri.as_str().len() + 64)
            .sum::<usize>();
        if self.stored > 32 * 1024 * 1024 {
            return Err(());
        }
        if graph.sources.is_some() {
            let names = graph
                .sections
                .iter()
                .filter_map(|view| view.namespace.as_ref())
                .flat_map(|state| state.declared.keys().map(String::as_str))
                .collect();
            let members = graph
                .members
                .keys()
                .map(|(uri, start, end)| &self.ws.documents[uri].text.source[*start..*end])
                .collect::<BTreeSet<_>>();
            self.work.set(self.work.get() + members.len() * 32);
            if self.work.get() > 32 * 1024 * 1024 {
                return Err(());
            }
            let wildcard = graph
                .sections
                .iter()
                .any(|view| view.namespace.as_ref().is_some_and(|state| state.wildcard));
            allocations.clear();
            for view in graph
                .sections
                .iter()
                .filter(|view| allocations.insert(Arc::as_ptr(&view.resources) as usize))
            {
                self.work.set(
                    self.work.get()
                        + crate::bib_source_aliases::certify_reads(
                            self.ws,
                            &view.resources,
                            &names,
                            &members,
                            wildcard,
                        )
                        .ok_or(())?,
                );
                if self.work.get() > 32 * 1024 * 1024 {
                    return Err(());
                }
            }
            if self.work.get() > 32 * 1024 * 1024 {
                return Err(());
            }
        }
        for resource in &resources {
            if self.databases.contains_key(resource) {
                continue;
            }
            let doc = self.ws.documents.get(resource).ok_or(())?;
            self.bytes += doc.text.source.len();
            if self.bytes > 16 * 1024 * 1024 {
                return Err(());
            }
            let database = Database::new(resource, doc)?;
            self.stored += database
                .declarations
                .iter()
                .map(|(name, identities)| {
                    name.len()
                        + identities
                            .iter()
                            .map(|id| id.uri.as_str().len() + 96)
                            .sum::<usize>()
                })
                .sum::<usize>();
            self.stored += database
                .references
                .iter()
                .map(|reference| reference.name.len() + reference.field.len() + 64)
                .sum::<usize>();
            if self.stored > 32 * 1024 * 1024 {
                return Err(());
            }
            self.databases.insert(resource.clone(), database);
        }
        if let (true, Some(name)) = (graph.sources.is_some(), self.name.as_deref()) {
            for resource in &resources {
                if self.databases[resource]
                    .declarations
                    .get(name)
                    .is_some_and(|identities| identities.len() != 1)
                {
                    return Err(());
                }
            }
        }
        self.projects.insert(
            root.clone(),
            Project {
                scope,
                graph,
                resources,
            },
        );
        Ok(())
    }
    fn bind<'b>(
        &self,
        name: &str,
        views: impl IntoIterator<Item = &'b Arc<View>>,
    ) -> Result<Option<Identity>> {
        if self.name.as_deref() != Some(name) {
            return Err(());
        }
        let mut answer: Option<Option<Identity>> = None;
        for view in views {
            let address = Arc::as_ptr(view) as usize;
            let cached = self.bindings.borrow().get(&address).cloned();
            let found = if let Some(cached) = cached {
                cached?
            } else {
                let namespace = view.namespace.as_ref();
                if namespace.is_some_and(|state| {
                    state.wildcard
                        && state.declared.contains_key(name)
                        && state.first.get(name) == Some(&None)
                }) {
                    return Err(());
                }
                let source = namespace.and_then(|state| match state.first.get(name) {
                    Some(first) => first.clone(),
                    None => state.declared.get(name).cloned(),
                });
                let found = if let Some(source) = source {
                    Some(source)
                } else {
                    self.database_binding(name, &view.resources)?
                };
                self.work.set(
                    self.work.get()
                        + name.len()
                        + found
                            .as_ref()
                            .map_or(0, |identity| identity.uri.as_str().len())
                        + 160,
                );
                if self.work.get() > 32 * 1024 * 1024 {
                    return Err(());
                }
                self.bindings
                    .borrow_mut()
                    .insert(address, Ok(found.clone()));
                found
            };
            if answer.as_ref().is_some_and(|answer| *answer != found) {
                return Err(());
            }
            answer = Some(found);
        }
        answer.ok_or(())
    }
    // Ordinary database lookup is shared by numeric namespaces with the same
    // resource allocation; the final source binding remains per namespace.
    fn database_binding(
        &self,
        name: &str,
        resources: &Arc<BTreeSet<Url>>,
    ) -> Result<Option<Identity>> {
        let address = Arc::as_ptr(resources) as usize;
        if let Some(found) = self.database_bindings.borrow().get(&address).cloned() {
            return found;
        }
        let mut found = None;
        for resource in resources.iter() {
            self.work
                .set(self.work.get() + name.len() + resource.as_str().len() + 64);
            if self.work.get() > 32 * 1024 * 1024 {
                return Err(());
            }
            if let Some(identities) = self
                .databases
                .get(resource)
                .ok_or(())?
                .declarations
                .get(name)
            {
                for identity in identities {
                    if found.is_some() {
                        return Err(());
                    }
                    found = Some(identity.clone());
                }
            }
        }
        self.database_bindings
            .borrow_mut()
            .insert(address, Ok(found.clone()));
        Ok(found)
    }
    // Fold all numeric bindings once per root/name. Equal resource allocations
    // share membership scanning, while differing source identities stay visible.
    fn relation_binding(&self, root: &Url, owner: &Url) -> Result<Option<Identity>> {
        if let Some(found) = self
            .relation_bindings
            .borrow()
            .get(root)
            .and_then(|bindings| bindings.get(owner))
            .cloned()
        {
            return found;
        }
        let name = self.name.as_deref().ok_or(())?;
        let project = self.projects.get(root).ok_or(())?;
        type Answers = BTreeSet<Result<Option<Identity>>>;
        let mut allocations = BTreeMap::<usize, (&Arc<BTreeSet<Url>>, Answers)>::new();
        for view in &project.graph.sections {
            let answer = self.bind(name, std::iter::once(view)).and_then(|found| {
                if found.is_some()
                    && view.namespace.is_some()
                    && self.database_binding(name, &view.resources)?.is_none()
                {
                    return Err(());
                }
                Ok(found)
            });
            allocations
                .entry(Arc::as_ptr(&view.resources) as usize)
                .or_insert_with(|| (&view.resources, BTreeSet::new()))
                .1
                .insert(answer);
        }
        let mut bindings = BTreeMap::<Url, Result<Option<Identity>>>::new();
        for (_, (resources, answers)) in allocations {
            let answer = if answers.len() == 1 {
                answers.into_iter().next().ok_or(())?
            } else {
                Err(())
            };
            for resource in resources.iter() {
                self.work.set(
                    self.work.get()
                        + resource.as_str().len()
                        + answer
                            .as_ref()
                            .ok()
                            .and_then(Option::as_ref)
                            .map_or(0, |identity| identity.uri.as_str().len())
                        + 160,
                );
                if self.work.get() > 32 * 1024 * 1024 {
                    return Err(());
                }
                bindings
                    .entry(resource.clone())
                    .and_modify(|previous| {
                        if *previous != answer {
                            *previous = Err(());
                        }
                    })
                    .or_insert_with(|| answer.clone());
            }
        }
        let found = bindings.get(owner).cloned().ok_or(())?;
        self.relation_bindings
            .borrow_mut()
            .insert(root.clone(), bindings);
        found
    }
}

struct Symbol<'a> {
    declaration: Identity,
    prerequisites: BTreeSet<Identity>,
    occurrences: BTreeMap<Url, BTreeSet<(usize, usize)>>,
    name: String,
    query: Range<usize>,
    graphs: Graphs<'a>,
    lexical: Index,
}

fn query_span(
    doc: &Document,
    offset: usize,
    lexical: &Index,
    uri: &Url,
) -> Option<(String, Range<usize>, Option<Identity>)> {
    if doc.kind == DocumentKind::Bibtex {
        if let Some(entry) = doc
            .syntax
            .citations
            .iter()
            .chain(&doc.syntax.bibliography_aliases)
            .find(|entry| entry.span.start <= offset && offset <= entry.span.end)
        {
            return Some((
                entry.name.clone(),
                entry.span.clone(),
                Some(Identity {
                    uri: uri.clone(),
                    start: entry.span.start,
                    end: entry.span.end,
                }),
            ));
        }
        let entry = doc
            .syntax
            .citation_references
            .iter()
            .find(|entry| entry.span.start <= offset && offset <= entry.span.end)?;
        return Some((entry.name.clone(), entry.span.clone(), None));
    }
    if doc.kind != DocumentKind::Latex {
        return None;
    }
    let (citation, span) = lexical.target(uri, offset)?;
    citation.then(|| (doc.text.source[span.clone()].to_string(), span, None))
}
fn symbol<'a>(ws: &'a Workspace, uri: &Url, offset: usize) -> Option<Result<Symbol<'a>>> {
    let doc = ws.documents.get(uri)?;
    if !bib_section_keys::needs_namespace_proof(ws)
        && !ws.documents.values().any(|doc| {
            doc.kind == DocumentKind::Latex
                && doc.syntax.commands.iter().any(|command| {
                    matches!(command.name.as_str(), "defbibentryset" | "csuse" | "csname")
                })
        })
    {
        return None;
    }
    // Loose snippets retain the established symbol/editing model.
    let (selected, projects) = match bib_section_keys::source_projects(ws, uri) {
        Some(projects) => projects,
        None if bib_section_keys::unproved_citation_query(ws, uri, offset) => return Some(Err(())),
        None => return None,
    };
    if !selected {
        return None;
    }
    let mut graphs = match Graphs::new(ws) {
        Ok(graphs) => graphs,
        Err(()) => return Some(Err(())),
    };
    let lexical = graphs.lexical(uri);
    // Source declarations are not citation operands in the lexical index.
    // Own their queries too, so a failed source-namespace proof cannot fall
    // back to the generic same-name union of database and source entries.
    let source_declaration = (doc.kind == DocumentKind::Latex
        && lexical.target(uri, offset).is_none())
    .then(|| {
        doc.syntax
            .citations
            .iter()
            .find(|entry| entry.span.start <= offset && offset <= entry.span.end)
            .map(|entry| (entry.name.clone(), entry.span.clone()))
            .or_else(|| {
                projects.iter().find_map(|project| {
                    bib_section_keys::source_set_key(doc, &project.signatures, offset)
                })
            })
    })
    .flatten();
    let (name, span, declaration) = if let Some((name, span)) = source_declaration {
        let identity = Identity {
            uri: uri.clone(),
            start: span.start,
            end: span.end,
        };
        (name, span, Some(identity))
    } else {
        query_span(doc, offset, &lexical, uri)?
    };
    graphs.name = Some(name.clone());
    Some((|| {
        let mut initial = BTreeSet::new();
        let mut resolved = declaration;
        let own_declaration = resolved.is_some();
        for root in graphs.consumers(uri) {
            graphs.ensure(&root)?;
            initial.insert(root.clone());
            if resolved.is_some()
                && doc.kind == DocumentKind::Bibtex
                && doc
                    .syntax
                    .citations
                    .iter()
                    .chain(&doc.syntax.bibliography_aliases)
                    .any(|entry| entry.span == span)
            {
                continue;
            }
            let project = &graphs.projects[&root];
            if own_declaration
                && project.graph.sources.as_ref().is_some_and(|sources| {
                    resolved
                        .as_ref()
                        .is_some_and(|identity| sources.contains(identity))
                })
            {
                continue;
            }
            let answer = if doc.kind == DocumentKind::Latex {
                let views = project
                    .graph
                    .occurrences
                    .get(&(uri.clone(), span.start, span.end))
                    .ok_or(())?;
                graphs.bind(&name, views)?
            } else {
                let database = graphs.databases.get(uri).ok_or(())?;
                if !database
                    .references
                    .iter()
                    .any(|reference| reference.span == span && reference.name == name)
                {
                    return Err(());
                }
                graphs.relation_binding(&root, uri)?
            }
            .ok_or(())?;
            if resolved
                .as_ref()
                .is_some_and(|previous| *previous != answer)
            {
                return Err(());
            }
            resolved = Some(answer);
        }
        let declaration = resolved.ok_or(())?;
        // A configured root can name a project without consuming this database.
        // Do not certify a declaration-only graph that omits its own relations.
        if !graphs.databases.contains_key(&declaration.uri)
            && !graphs.projects.values().any(|project| {
                project
                    .graph
                    .sources
                    .as_ref()
                    .is_some_and(|sources| sources.contains(&declaration))
            })
        {
            return Err(());
        }

        let mut pending: BTreeSet<_> = graphs.consumers(&declaration.uri).into_iter().collect();
        pending.extend(initial);
        let mut checked = BTreeSet::new();
        let mut assignments = BTreeMap::<(Url, usize, usize), BTreeSet<Option<Identity>>>::new();
        let mut occurrences = BTreeMap::<Url, BTreeSet<(usize, usize)>>::new();
        let mut affected = BTreeSet::from([declaration.uri.clone()]);
        while let Some(root) = pending.pop_first() {
            if !checked.insert(root.clone()) {
                continue;
            }
            graphs.ensure(&root)?;
            let project = &graphs.projects[&root];
            lexical
                .validate(&project.scope, true, &name)
                .map_err(|_| ())?;
            let mut choices = Vec::new();
            for source in project.scope.iter().filter(|source| {
                ws.documents
                    .get(*source)
                    .is_some_and(|doc| doc.kind == DocumentKind::Latex)
            }) {
                for span in lexical.references(source, true, &name) {
                    let views = project
                        .graph
                        .occurrences
                        .get(&(source.clone(), span.start, span.end))
                        .ok_or(())?;
                    choices.push((
                        (source.clone(), span.start, span.end),
                        graphs.bind(&name, views)?,
                    ));
                }
            }
            for resource in &project.resources {
                for reference in &graphs.databases[resource].references {
                    if reference.name != name {
                        continue;
                    }
                    let answer = graphs.relation_binding(&root, resource)?;
                    choices.push((
                        (resource.clone(), reference.span.start, reference.span.end),
                        answer,
                    ));
                }
            }
            for (key, identity) in choices {
                if assignments.len() >= 100_000 && !assignments.contains_key(&key) {
                    return Err(());
                }
                let answers = assignments.entry(key.clone()).or_default();
                if answers.insert(identity.clone()) {
                    graphs.stored += key.0.as_str().len()
                        + identity.as_ref().map_or(0, |id| id.uri.as_str().len())
                        + 192;
                    if graphs.stored > 32 * 1024 * 1024 {
                        return Err(());
                    }
                }
                if answers.contains(&Some(declaration.clone())) && answers.len() != 1 {
                    return Err(());
                }
                if identity == Some(declaration.clone()) {
                    occurrences
                        .entry(key.0.clone())
                        .or_default()
                        .insert((key.1, key.2));
                    if affected.insert(key.0.clone()) {
                        pending.extend(graphs.consumers(&key.0));
                    }
                }
            }
        }
        occurrences
            .entry(declaration.uri.clone())
            .or_default()
            .insert((declaration.start, declaration.end));
        if !occurrences
            .get(uri)
            .is_some_and(|spans| spans.contains(&(span.start, span.end)))
        {
            return Err(());
        }
        Ok(Symbol {
            declaration,
            prerequisites: BTreeSet::new(),
            occurrences,
            name,
            query: span,
            graphs,
            lexical,
        })
    })())
}

pub(crate) fn references(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    include: bool,
) -> Option<Vec<Location>> {
    Some(
        symbol(ws, uri, offset)?
            .map(|symbol| {
                symbol
                    .occurrences
                    .iter()
                    .flat_map(|(uri, spans)| {
                        spans.iter().map(|&(start, end)| {
                            Location::new(uri.clone(), ws.documents[uri].text.range(start, end))
                        })
                    })
                    .filter(|location| {
                        include
                            || *location
                                != Location::new(
                                    symbol.declaration.uri.clone(),
                                    ws.documents[&symbol.declaration.uri]
                                        .text
                                        .range(symbol.declaration.start, symbol.declaration.end),
                                )
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}

fn edit_error() -> jsonrpc::Error {
    jsonrpc::Error::invalid_params("The bibliography key has an incomplete or ambiguous declaration identity across its consumers")
}

fn is_source(symbol: &Symbol<'_>) -> bool {
    symbol.graphs.projects.values().any(|project| {
        project
            .graph
            .sources
            .as_ref()
            .is_some_and(|sources| sources.contains(&symbol.declaration))
    })
}
fn is_alias(symbol: &Symbol<'_>) -> bool {
    symbol.graphs.ws.documents[&symbol.declaration.uri]
        .syntax
        .bibliography_aliases
        .iter()
        .any(|alias| alias.span == (symbol.declaration.start..symbol.declaration.end))
}
fn consumes(view: &View, declaration: &Identity, name: &str, source: bool) -> bool {
    if source {
        view.namespace
            .as_ref()
            .is_some_and(|state| state.declared.get(name) == Some(declaration))
    } else {
        view.contains(&declaration.uri)
    }
}
fn validate_edit(symbol: &Symbol<'_>) -> Result<()> {
    let source = is_source(symbol);
    if source {
        // A final source identity alone cannot certify the raw prerequisite.
        if symbol.prerequisites.is_empty()
            && symbol
                .occurrences
                .keys()
                .any(|uri| symbol.graphs.ws.documents[uri].kind == DocumentKind::Bibtex)
        {
            return Err(());
        }
    } else {
        let database = Database::new(
            &symbol.declaration.uri,
            &symbol.graphs.ws.documents[&symbol.declaration.uri],
        )?;
        if database.declarations.get(&symbol.name).map(Vec::as_slice)
            != Some(std::slice::from_ref(&symbol.declaration))
        {
            return Err(());
        }
    }
    for project in symbol.graphs.projects.values() {
        crate::resource_context::validate_rename(symbol.graphs.ws, &project.scope, &symbol.name)
            .map_err(|_| ())?;
        if source {
            for view in project
                .graph
                .sections
                .iter()
                .filter(|view| consumes(view, &symbol.declaration, &symbol.name, true))
            {
                // An early book request can suppress the old source set.
                // Giving it a fresh name would materialize an additional set.
                if symbol.graphs.bind(&symbol.name, std::iter::once(view))?
                    != Some(symbol.declaration.clone())
                {
                    return Err(());
                }
                if view.namespace.as_ref().is_some_and(|state| state.wildcard)
                    && symbol
                        .graphs
                        .database_binding(&symbol.name, &view.resources)?
                        .is_some()
                {
                    // Moving the source shadow would expose the old raw entry
                    // to nocite{*}, adding a bibliography entry after the edit.
                    return Err(());
                }
            }
        } else {
            for view in project
                .graph
                .sections
                .iter()
                .filter(|view| consumes(view, &symbol.declaration, &symbol.name, false))
            {
                // Moving a raw key can remove the first request that suppressed
                // a same-name source declaration, materializing an extra set.
                if view
                    .namespace
                    .as_ref()
                    .is_some_and(|state| state.declared.contains_key(&symbol.name))
                    || symbol.graphs.bind(&symbol.name, std::iter::once(view))?
                        != Some(symbol.declaration.clone())
                {
                    return Err(());
                }
            }
        }
    }
    Ok(())
}

// A source edit can move raw prerequisites only when every consumer shadows
// those headers with this exact physical source declaration.
fn source_prerequisites(target: &Symbol<'_>) -> Result<BTreeSet<Identity>> {
    let mut has_ids_owner = false;
    for uri in target.graphs.databases.keys() {
        let doc = &target.graphs.ws.documents[uri];
        for alias in &doc.syntax.bibliography_aliases {
            target
                .graphs
                .work
                .set(target.graphs.work.get() + alias.name.len() + 64);
            if target.graphs.work.get() > 32 * 1024 * 1024 {
                return Err(());
            }
            has_ids_owner |=
                crate::bib_source_aliases::owner(doc, alias) == Some(target.name.as_str());
        }
    }
    if !has_ids_owner
        && !target
            .occurrences
            .keys()
            .any(|uri| target.graphs.ws.documents[uri].kind == DocumentKind::Bibtex)
    {
        return Ok(BTreeSet::new());
    }
    let mut prerequisites = BTreeSet::new();
    for project in target.graphs.projects.values() {
        for view in project
            .graph
            .sections
            .iter()
            .filter(|view| consumes(view, &target.declaration, &target.name, true))
        {
            let found = target
                .graphs
                .database_binding(&target.name, &view.resources)?
                .ok_or(())?;
            target
                .graphs
                .work
                .set(target.graphs.work.get() + found.uri.as_str().len() + 96);
            if target.graphs.work.get() > 32 * 1024 * 1024
                || prerequisites.len() >= 64 && !prerequisites.contains(&found)
            {
                return Err(());
            }
            prerequisites.insert(found);
        }
    }
    if prerequisites.is_empty() {
        return Err(());
    }
    let mut aliases = BTreeSet::new();
    for prerequisite in &prerequisites {
        let database = target.graphs.databases.get(&prerequisite.uri).ok_or(())?;
        if database.declarations.get(&target.name).map(Vec::as_slice)
            != Some(std::slice::from_ref(prerequisite))
        {
            return Err(());
        }
        let consumers = target.graphs.consumers(&prerequisite.uri);
        if consumers.is_empty() {
            return Err(());
        }
        for root in consumers {
            let project = target.graphs.projects.get(&root).ok_or(())?;
            if !project.resources.contains(&prerequisite.uri) {
                return Err(());
            }
            for view in project
                .graph
                .sections
                .iter()
                .filter(|view| view.contains(&prerequisite.uri))
            {
                target.graphs.work.set(target.graphs.work.get() + 96);
                if target.graphs.work.get() > 32 * 1024 * 1024
                    || target.graphs.bind(&target.name, std::iter::once(view))?
                        != Some(target.declaration.clone())
                    || target
                        .graphs
                        .database_binding(&target.name, &view.resources)?
                        != Some(prerequisite.clone())
                {
                    return Err(());
                }
            }
        }
        if prerequisites.len() > 1 {
            let doc = &target.graphs.ws.documents[&prerequisite.uri];
            for alias in &doc.syntax.bibliography_aliases {
                target
                    .graphs
                    .work
                    .set(target.graphs.work.get() + alias.name.len() + 96);
                if target.graphs.work.get() > 32 * 1024 * 1024 {
                    return Err(());
                }
                if crate::bib_source_aliases::owner(doc, alias) == Some(target.name.as_str()) {
                    aliases.insert(alias.name.as_str());
                }
            }
            // A raw parent need not own IDS. Every consumer above must bind
            // to this source, and edit_symbol separately certifies all raw
            // crossref/xref occurrences before coupling the header edits.
        }
    }
    if prerequisites.len() > 1 {
        for (uri, database) in &target.graphs.databases {
            let doc = &target.graphs.ws.documents[uri];
            for alias in &doc.syntax.bibliography_aliases {
                target
                    .graphs
                    .work
                    .set(target.graphs.work.get() + alias.name.len() + 64);
                if target.graphs.work.get() > 32 * 1024 * 1024 {
                    return Err(());
                }
                if aliases.contains(alias.name.as_str()) {
                    let owner = crate::bib_source_aliases::owner(doc, alias).ok_or(())?;
                    let entries = database.declarations.get(owner).ok_or(())?;
                    if owner != target.name
                        || entries.len() != 1
                        || !prerequisites.contains(&entries[0])
                    {
                        return Err(());
                    }
                }
            }
        }
    }
    Ok(prerequisites)
}

// Couple only proved backend prerequisites with no independent book consumers.
fn edit_symbol<'a>(mut target: Symbol<'a>, shared: &BTreeSet<Identity>) -> Result<Symbol<'a>> {
    // An alias-only edit preserves its globally certified primary spelling.
    // Raw owner edits certify each actual resource allocation independently.
    let alias_key = is_alias(&target);
    let source_key = is_source(&target);
    let prerequisites = if source_key {
        source_prerequisites(&target)?
    } else {
        BTreeSet::new()
    };
    for project in target
        .graphs
        .projects
        .values()
        .filter(|project| !alias_key && project.graph.sources.is_some())
    {
        let names = project
            .graph
            .sections
            .iter()
            .filter_map(|view| view.namespace.as_ref())
            .flat_map(|state| state.declared.keys().map(String::as_str))
            .collect();
        let members = project
            .graph
            .members
            .keys()
            .map(|(uri, start, end)| &target.graphs.ws.documents[uri].text.source[*start..*end])
            .collect();
        let certify = |resources: &BTreeSet<Url>| -> Result<()> {
            target.graphs.work.set(
                target.graphs.work.get()
                    + crate::bib_source_aliases::certify(
                        target.graphs.ws,
                        resources,
                        &names,
                        &members,
                    )
                    .ok_or(())?,
            );
            (target.graphs.work.get() <= 32 * 1024 * 1024)
                .then_some(())
                .ok_or(())
        };
        if source_key && prerequisites.len() <= 1 {
            // Preserve the original union proof for single prerequisites.
            certify(&project.resources)?;
        } else {
            // Equal IDS in disjoint sections are not duplicate declarations.
            // The complete raw owner closure below couples their edits.
            let mut allocations = BTreeSet::new();
            for view in project
                .graph
                .sections
                .iter()
                .filter(|view| allocations.insert(Arc::as_ptr(&view.resources) as usize))
            {
                certify(&view.resources)?;
            }
        }
    }

    // Changing a raw IDS owner would make equal global alias spellings in
    // another resource point to a different primary, even across sections.
    if !is_source(&target) {
        let doc = &target.graphs.ws.documents[&target.declaration.uri];
        let aliases: BTreeSet<_> = doc
            .syntax
            .bibliography_aliases
            .iter()
            .filter(|alias| {
                crate::bib_source_aliases::owner(doc, alias) == Some(target.name.as_str())
            })
            .map(|alias| alias.name.as_str())
            .collect();
        if !aliases.is_empty() {
            for (uri, _) in target
                .graphs
                .databases
                .iter()
                .filter(|(uri, _)| **uri != target.declaration.uri)
            {
                for alias in &target.graphs.ws.documents[uri].syntax.bibliography_aliases {
                    target
                        .graphs
                        .work
                        .set(target.graphs.work.get() + alias.name.len() + 64);
                    if target.graphs.work.get() > 32 * 1024 * 1024 {
                        return Err(());
                    }
                    if aliases.contains(alias.name.as_str()) {
                        let owner = crate::bib_source_aliases::owner(
                            &target.graphs.ws.documents[uri],
                            alias,
                        )
                        .ok_or(())?;
                        let entries = target.graphs.databases[uri]
                            .declarations
                            .get(owner)
                            .ok_or(())?;
                        if entries.len() != 1 || !shared.contains(&entries[0]) {
                            return Err(());
                        }
                    }
                }
            }
        }
    }
    if !prerequisites.is_empty() {
        for (uri, spans) in &target.occurrences {
            if target.graphs.ws.documents[uri].kind != DocumentKind::Bibtex {
                continue;
            }
            let database = target.graphs.databases.get(uri).ok_or(())?;
            let mut checked = 0usize;
            for reference in &database.references {
                target.graphs.work.set(
                    target.graphs.work.get() + reference.name.len() + reference.field.len() + 64,
                );
                if target.graphs.work.get() > 32 * 1024 * 1024 {
                    return Err(());
                }
                if spans.contains(&(reference.span.start, reference.span.end)) {
                    if reference.name != target.name
                        || !matches!(reference.field.as_str(), "crossref" | "xref")
                    {
                        return Err(());
                    }
                    checked += 1;
                }
            }
            if checked != spans.len() {
                return Err(());
            }
        }
        for prerequisite in &prerequisites {
            target.graphs.stored += prerequisite.uri.as_str().len() * 2 + 192;
            if target.graphs.stored > 32 * 1024 * 1024 {
                return Err(());
            }
            target
                .occurrences
                .entry(prerequisite.uri.clone())
                .or_default()
                .insert((prerequisite.start, prerequisite.end));
        }
        target.prerequisites = prerequisites;
    }
    validate_edit(&target)?;
    Ok(target)
}

// A shared IDS mapping must keep one primary spelling throughout a run.
// Expand through every alias of each raw owner before validating any edit.
fn edit_symbols<'a>(target: Symbol<'a>) -> Result<Vec<Symbol<'a>>> {
    let ws = target.graphs.ws;
    let mut seen = BTreeSet::from([target.declaration.clone()]);
    let (mut work, mut stored, mut bytes) = (
        target.graphs.work.get(),
        target.graphs.stored,
        target.graphs.bytes,
    );
    if work > 32 * 1024 * 1024 || stored > 32 * 1024 * 1024 || bytes > 16 * 1024 * 1024 {
        return Err(());
    }
    let mut pending = vec![target];
    let mut symbols = Vec::new();
    while let Some(target) = pending.pop() {
        if !is_source(&target) && !is_alias(&target) {
            let doc = &ws.documents[&target.declaration.uri];
            let aliases: BTreeSet<_> = doc
                .syntax
                .bibliography_aliases
                .iter()
                .filter(|alias| {
                    crate::bib_source_aliases::owner(doc, alias) == Some(target.name.as_str())
                })
                .map(|alias| alias.name.as_str())
                .collect();
            for (uri, database) in &target.graphs.databases {
                for alias in &ws.documents[uri].syntax.bibliography_aliases {
                    work += alias.name.len() + 64;
                    if work > 32 * 1024 * 1024 {
                        return Err(());
                    }
                    if !aliases.contains(alias.name.as_str()) {
                        continue;
                    }
                    let owner =
                        crate::bib_source_aliases::owner(&ws.documents[uri], alias).ok_or(())?;
                    if owner != target.name {
                        return Err(());
                    }
                    let entries = database.declarations.get(owner).ok_or(())?;
                    if entries.len() != 1 {
                        return Err(());
                    }
                    let identity = &entries[0];
                    if seen.insert(identity.clone()) {
                        if seen.len() > 64 {
                            return Err(());
                        }
                        let sibling = symbol(ws, uri, identity.start).ok_or(())??;
                        // Charge the retained graph before enqueueing another
                        // owner, rather than after all siblings are allocated.
                        work += sibling.graphs.work.get();
                        stored += sibling.graphs.stored;
                        bytes += sibling.graphs.bytes;
                        if work > 32 * 1024 * 1024
                            || stored > 32 * 1024 * 1024
                            || bytes > 16 * 1024 * 1024
                        {
                            return Err(());
                        }
                        pending.push(sibling);
                    }
                }
            }
        }
        symbols.push(target);
    }
    if symbols.len() > 1
        && symbols.iter().any(|target| {
            is_source(target)
                || target.graphs.projects.values().any(|project| {
                    project.graph.sections.iter().any(|view| {
                        view.namespace
                            .as_ref()
                            .is_some_and(|state| state.declared.contains_key(&target.name))
                    })
                })
        })
    {
        // Multiple independently queried raw symbols cannot move a source
        // shadow as well. Source edits prove their complete prerequisites above.
        return Err(());
    }
    let mut edited = Vec::new();
    for target in symbols {
        let before = target.graphs.work.get();
        let target = edit_symbol(target, &seen)?;
        work += target.graphs.work.get() - before;
        if work > 32 * 1024 * 1024 {
            return Err(());
        }
        edited.push(target);
    }
    let charged: usize = edited.iter().map(|target| target.graphs.work.get()).sum();
    if let Some(first) = edited.first() {
        first
            .graphs
            .work
            .set(first.graphs.work.get() + work - charged);
    }
    Ok(edited)
}

pub(crate) fn prepare(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
) -> Option<jsonrpc::Result<Option<PrepareRenameResponse>>> {
    Some(
        symbol(ws, uri, offset)?
            .and_then(|symbol| {
                let range = ws.documents[uri]
                    .text
                    .range(symbol.query.start, symbol.query.end);
                let placeholder = symbol.name.clone();
                edit_symbols(symbol)?;
                Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range,
                    placeholder,
                }))
            })
            .map_err(|_| edit_error()),
    )
}

fn rename_edits(
    ws: &Workspace,
    mut symbol: Symbol<'_>,
    name: &str,
) -> jsonrpc::Result<(BTreeMap<Url, Vec<TextEdit>>, usize)> {
    if !syntax::is_static_citation_key(name) || name == "*" {
        return Err(jsonrpc::Error::invalid_params("Invalid bibliography key: use a nonempty literal key without separators or TeX expansion"));
    }
    if name == symbol.name {
        return Ok((BTreeMap::new(), symbol.graphs.work.get()));
    }
    let source_key = is_source(&symbol);
    let alias_key = is_alias(&symbol);
    if !source_key
        && symbol.graphs.databases[&symbol.declaration.uri]
            .declarations
            .contains_key(name)
    {
        return Err(jsonrpc::Error::invalid_params(
            "The bibliography key or IDS alias already exists in the declaration database",
        ));
    }
    symbol.graphs.name = Some(name.into());
    symbol.graphs.bindings.borrow_mut().clear();
    symbol.graphs.database_bindings.borrow_mut().clear();
    let raws: Vec<_> = if source_key {
        symbol.prerequisites.iter().collect()
    } else if !alias_key && ws.documents[&symbol.declaration.uri].kind == DocumentKind::Bibtex {
        vec![&symbol.declaration]
    } else {
        Vec::new()
    };
    let coupled_source = source_key && raws.len() > 1;
    let mut owns_ids = false;
    for raw in &raws {
        for alias in &ws.documents[&raw.uri].syntax.bibliography_aliases {
            symbol
                .graphs
                .work
                .set(symbol.graphs.work.get() + alias.name.len() + 64);
            if symbol.graphs.work.get() > 32 * 1024 * 1024 {
                return Err(edit_error());
            }
            owns_ids |= crate::bib_source_aliases::owner(&ws.documents[&raw.uri], alias)
                == Some(symbol.name.as_str());
        }
    }
    symbol.graphs.relation_bindings.borrow_mut().clear();
    let relation_owners: BTreeSet<_> = symbol
        .graphs
        .databases
        .iter()
        .filter(|(_, database)| {
            database
                .references
                .iter()
                .any(|reference| reference.name == name)
        })
        .map(|(uri, _)| uri)
        .collect();
    for (root, project) in &symbol.graphs.projects {
        let mut check_cache = false;
        // A new real key can delete an earlier IDS mapping from Biber's
        // global cache, breaking later citations in other sections.
        for resource in &project.resources {
            if owns_ids {
                for entry in &ws.documents[resource].syntax.citations {
                    symbol
                        .graphs
                        .work
                        .set(symbol.graphs.work.get() + entry.name.len() + 64);
                    if symbol.graphs.work.get() > 32 * 1024 * 1024 {
                        return Err(edit_error());
                    }
                    if entry.name == name {
                        check_cache = true;
                    }
                }
            }
            for alias in &ws.documents[resource].syntax.bibliography_aliases {
                symbol
                    .graphs
                    .work
                    .set(symbol.graphs.work.get() + alias.name.len() + 64);
                if symbol.graphs.work.get() > 32 * 1024 * 1024 {
                    return Err(edit_error());
                }
                if alias.name == name {
                    return Err(jsonrpc::Error::invalid_params("The new bibliography key conflicts with a global IDS alias in this project"));
                }
                if !raws.is_empty()
                    && crate::bib_source_aliases::owner(&ws.documents[resource], alias)
                        == Some(name)
                {
                    check_cache = true;
                }
            }
        }
        if check_cache {
            let [raw] = raws.as_slice() else {
                // A one-file simulation cannot certify an atomic multi-owner
                // move against global cached data. Require a fresh spelling.
                return Err(edit_error());
            };
            let work = bib_section_keys::validate_primary_rename(
                ws,
                root,
                &project.scope,
                crate::bib_source_aliases::PrimaryRename {
                    uri: &raw.uri,
                    from: &symbol.name,
                    to: name,
                },
            )
            .ok_or_else(|| {
                jsonrpc::Error::invalid_params(
                    "The new primary spelling has an unproved global IDS cache owner",
                )
            })?;
            symbol.graphs.work.set(symbol.graphs.work.get() + work);
            if symbol.graphs.work.get() > 32 * 1024 * 1024 {
                return Err(edit_error());
            }
        }
        symbol.lexical.validate(&project.scope, true, name)?;
        crate::resource_context::validate_rename(ws, &project.scope, name)?;
        for view in project.graph.sections.iter().filter(|view| {
            alias_key
                || coupled_source
                || consumes(view, &symbol.declaration, &symbol.name, source_key)
        }) {
            if view
                .namespace
                .as_ref()
                .is_some_and(|state| state.declared.contains_key(name))
                || symbol
                    .graphs
                    .database_binding(name, &view.resources)
                    .map_err(|_| edit_error())?
                    .is_some()
            {
                return Err(jsonrpc::Error::invalid_params("The bibliography key or IDS alias already exists in a section consuming this declaration"));
            }
        }
        for source in project.scope.iter().filter(|uri| {
            ws.documents
                .get(*uri)
                .is_some_and(|doc| doc.kind == DocumentKind::Latex)
        }) {
            for span in symbol.lexical.references(source, true, name) {
                let views = project
                    .graph
                    .occurrences
                    .get(&(source.clone(), span.start, span.end))
                    .ok_or_else(edit_error)?;
                if alias_key
                    || coupled_source
                    || views
                        .iter()
                        .any(|view| consumes(view, &symbol.declaration, &symbol.name, source_key))
                {
                    return Err(jsonrpc::Error::invalid_params(
                        "Renaming this bibliography key would capture an existing citation",
                    ));
                }
            }
        }
        if project
            .graph
            .sections
            .iter()
            .filter(|view| {
                alias_key
                    || coupled_source
                    || consumes(view, &symbol.declaration, &symbol.name, source_key)
            })
            .any(|view| {
                view.iter()
                    .any(|resource| relation_owners.contains(resource))
            })
        {
            return Err(jsonrpc::Error::invalid_params(
                "Renaming this bibliography key would capture an existing bibliography relation",
            ));
        }
    }
    let mut changes = BTreeMap::new();
    for (uri, spans) in symbol.occurrences {
        let doc = &ws.documents[&uri];
        let mut edits: Vec<_> = spans
            .into_iter()
            .map(|(start, end)| TextEdit {
                range: doc.text.range(start, end),
                new_text: name.into(),
            })
            .collect();
        if name.contains('"') {
            crate::features::brace_quoted_citation_edits(doc, &mut edits);
        }
        changes.insert(uri, edits);
    }
    Ok((changes, symbol.graphs.work.get()))
}

pub(crate) fn rename(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    name: &str,
) -> Option<jsonrpc::Result<Option<WorkspaceEdit>>> {
    Some(
        symbol(ws, uri, offset)?
            .map_err(|_| edit_error())
            .and_then(|symbol| {
                let symbols = edit_symbols(symbol).map_err(|_| edit_error())?;
                if symbols[0].name == name {
                    return Ok(Some(WorkspaceEdit::default()));
                }
                if symbols.len() > 1 && symbols[0].name != name {
                    // Reusing an existing primary can replace the cached IDS provider.
                    // A fresh spelling leaves the certified loading order unchanged.
                    for symbol in &symbols {
                        for database in symbol.graphs.databases.values() {
                            if database.declarations.contains_key(name) {
                                return Err(edit_error());
                            }
                        }
                    }
                }
                let mut changes = BTreeMap::<Url, Vec<TextEdit>>::new();
                let mut work = 0usize;
                for symbol in symbols {
                    let (edits, used) = rename_edits(ws, symbol, name)?;
                    work += used;
                    if work > 32 * 1024 * 1024 {
                        return Err(edit_error());
                    }
                    for (uri, edits) in edits {
                        changes.entry(uri).or_default().extend(edits);
                    }
                }
                for edits in changes.values_mut() {
                    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
                    edits.dedup();
                    if edits
                        .windows(2)
                        .any(|pair| pair[0].range.end > pair[1].range.start)
                    {
                        return Err(edit_error());
                    }
                }
                Ok(Some(crate::features::workspace_edit(ws, changes)))
            }),
    )
}

/// Validate a relation's complete owner and literal field before read-only
/// source-aware resolution. Reuse the graph scanner's strict field rules.
pub(crate) fn certified_relation(
    uri: &Url,
    doc: &Document,
    offset: usize,
) -> Option<Option<(String, Range<usize>)>> {
    let candidate = doc
        .syntax
        .citation_references
        .iter()
        .find(|entry| entry.span.start <= offset && offset <= entry.span.end)?;
    Some((|| {
        if doc.kind != DocumentKind::Bibtex || doc.text.source.len() > 16 * 1024 * 1024 {
            return None;
        }
        let database = Database::new(uri, doc).ok()?;
        let reference = database
            .references
            .into_iter()
            .find(|reference| reference.span == candidate.span)?;
        Some((reference.name, reference.span))
    })())
}

/// Missing-key proofs for every physical citation in a source-set project.
/// None retains ordinary diagnostics; an empty owned map suppresses guesses.
pub(crate) type SourceStatuses = BTreeMap<(usize, usize), bool>;
pub(crate) fn source_statuses(ws: &Workspace, uri: &Url) -> Option<SourceStatuses> {
    let doc = ws.documents.get(uri)?;
    if doc.kind != DocumentKind::Latex
        || !ws.documents.values().any(|doc| {
            doc.kind == DocumentKind::Latex
                && doc.syntax.commands.iter().any(|command| {
                    matches!(command.name.as_str(), "defbibentryset" | "csuse" | "csname")
                })
        })
    {
        return None;
    }
    let (selected, projects) = match bib_section_keys::source_projects(ws, uri) {
        Some(projects) => projects,
        None => return Some(BTreeMap::new()),
    };
    if !selected
        || !projects
            .iter()
            .any(|project| bib_section_keys::has_source_sets(&project.docs, &project.signatures))
    {
        return None;
    }
    Some(
        (|| {
            let mut graphs = Graphs::new(ws).ok()?;
            for project in &projects {
                graphs.ensure(&project.root).ok()?;
            }
            let mut names = BTreeMap::<&str, BTreeSet<(usize, usize)>>::new();
            for project in graphs.projects.values() {
                for (source, start, end) in project.graph.occurrences.keys() {
                    if source == uri
                        && !project
                            .graph
                            .nonloading
                            .contains(&(source.clone(), *start, *end))
                    {
                        names
                            .entry(&doc.text.source[*start..*end])
                            .or_default()
                            .insert((*start, *end));
                    }
                }
            }
            let mut statuses = BTreeMap::new();
            for (name, spans) in names {
                graphs.name = Some(name.into());
                graphs.bindings.get_mut().clear();
                graphs.database_bindings.get_mut().clear();
                for (start, end) in spans {
                    let mut answer: Option<Option<Identity>> = None;
                    let mut proved = true;
                    for project in graphs.projects.values() {
                        let Some(views) = project.graph.occurrences.get(&(uri.clone(), start, end))
                        else {
                            proved = false;
                            break;
                        };
                        // A suppressed declaration must not report members
                        // that Biber never materializes. Check its final source
                        // identity without changing the current name cache.
                        if let Some(owner) = project.graph.members.get(&(uri.clone(), start, end)) {
                            let owner_name =
                                &ws.documents[&owner.uri].text.source[owner.start..owner.end];
                            if !views.iter().all(|view| {
                                view.namespace.as_ref().is_some_and(|state| {
                                    (match state.first.get(owner_name) {
                                        Some(first) => first.as_ref(),
                                        None => state.declared.get(owner_name),
                                    }) == Some(owner)
                                })
                            }) {
                                proved = false;
                                break;
                            }
                        }
                        let Ok(found) = graphs.bind(name, views) else {
                            proved = false;
                            break;
                        };
                        if answer.as_ref().is_some_and(|previous| *previous != found) {
                            proved = false;
                            break;
                        }
                        answer = Some(found);
                    }
                    if graphs.work.get() > 32 * 1024 * 1024 {
                        return None;
                    }
                    if let (true, Some(found)) = (proved, answer) {
                        statuses.insert((start, end), found.is_some());
                    }
                }
            }
            Some(statuses)
        })()
        .unwrap_or_default(),
    )
}
