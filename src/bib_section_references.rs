//! Complete, bounded occurrence graphs for database keys in reference sections.
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
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    uri: Url,
    start: usize,
    end: usize,
}
#[derive(Clone)]
struct Occurrence {
    name: String,
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
                        stored += key.len() + 64;
                        if stored > 32 * 1024 * 1024 || references.len() >= 100_000 {
                            return Err(());
                        }
                        references.push(Occurrence {
                            name: key.into(),
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

struct Project {
    scope: BTreeSet<Url>,
    graph: KeyGraph,
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
        let graph = bib_section_keys::key_graph(self.ws, root, &docs, &signatures).ok_or(())?;
        self.stored += graph
            .sections
            .iter()
            .flat_map(|view| view.iter())
            .map(|uri| uri.as_str().len() + 64)
            .sum::<usize>();
        self.stored += graph
            .occurrences
            .iter()
            .map(|((uri, _, _), visits)| uri.as_str().len() + 128 + visits.len() * 32)
            .sum::<usize>();
        if self.stored > 32 * 1024 * 1024 {
            return Err(());
        }
        let resources: BTreeSet<_> = graph
            .sections
            .iter()
            .flat_map(|view| view.iter())
            .cloned()
            .collect();
        for resource in resources {
            if self.databases.contains_key(&resource) {
                continue;
            }
            let doc = self.ws.documents.get(&resource).ok_or(())?;
            self.bytes += doc.text.source.len();
            if self.bytes > 16 * 1024 * 1024 {
                return Err(());
            }
            let database = Database::new(&resource, doc)?;
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
                .map(|reference| reference.name.len() + 64)
                .sum::<usize>();
            if self.stored > 32 * 1024 * 1024 {
                return Err(());
            }
            self.databases.insert(resource, database);
        }
        self.projects.insert(root.clone(), Project { scope, graph });
        Ok(())
    }
    fn bind<'b>(
        &self,
        name: &str,
        views: impl IntoIterator<Item = &'b Arc<BTreeSet<Url>>>,
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
                let mut found = None;
                for resource in view.iter() {
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
}

struct Symbol<'a> {
    declaration: Identity,
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
    if !ws
        .documents
        .values()
        .any(|doc| doc.kind == DocumentKind::Latex && doc.text.source.contains("refsection"))
    {
        return None;
    }
    // Ordinary projects retain the established symbol/editing model.
    let (selected, _) = bib_section_keys::section_projects(ws, uri)?;
    if !selected {
        return None;
    }
    let mut graphs = match Graphs::new(ws) {
        Ok(graphs) => graphs,
        Err(()) => return Some(Err(())),
    };
    let lexical = graphs.lexical(uri);
    let (name, span, declaration) = query_span(doc, offset, &lexical, uri)?;
    graphs.name = Some(name.clone());
    Some((|| {
        let mut initial = BTreeSet::new();
        let mut resolved = declaration;
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
                graphs.bind(
                    &name,
                    project
                        .graph
                        .sections
                        .iter()
                        .filter(|view| view.contains(uri)),
                )?
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
        if !graphs.databases.contains_key(&declaration.uri) {
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
            let resources: BTreeSet<_> = project
                .graph
                .sections
                .iter()
                .flat_map(|view| view.iter())
                .collect();
            for resource in resources {
                for reference in &graphs.databases[resource].references {
                    if reference.name != name {
                        continue;
                    }
                    let answer = graphs.bind(
                        &name,
                        project
                            .graph
                            .sections
                            .iter()
                            .filter(|view| view.contains(resource)),
                    )?;
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

fn validate_edit(symbol: &Symbol<'_>) -> Result<()> {
    let database = Database::new(
        &symbol.declaration.uri,
        &symbol.graphs.ws.documents[&symbol.declaration.uri],
    )?;
    if database.declarations.get(&symbol.name).map(Vec::as_slice)
        != Some(std::slice::from_ref(&symbol.declaration))
    {
        return Err(());
    }
    for project in symbol.graphs.projects.values() {
        crate::resource_context::validate_rename(symbol.graphs.ws, &project.scope, &symbol.name)
            .map_err(|_| ())?;
        for view in project
            .graph
            .sections
            .iter()
            .filter(|view| view.contains(&symbol.declaration.uri))
        {
            if symbol.graphs.bind(&symbol.name, std::iter::once(view))?
                != Some(symbol.declaration.clone())
            {
                return Err(());
            }
        }
    }
    Ok(())
}

pub(crate) fn prepare(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
) -> Option<jsonrpc::Result<Option<PrepareRenameResponse>>> {
    Some(
        symbol(ws, uri, offset)?
            .and_then(|symbol| {
                validate_edit(&symbol)?;
                Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: ws.documents[uri]
                        .text
                        .range(symbol.query.start, symbol.query.end),
                    placeholder: symbol.name,
                }))
            })
            .map_err(|_| edit_error()),
    )
}

pub(crate) fn rename(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    name: &str,
) -> Option<jsonrpc::Result<Option<WorkspaceEdit>>> {
    Some(symbol(ws, uri, offset)?.map_err(|_| edit_error()).and_then(|mut symbol| {
        validate_edit(&symbol).map_err(|_| edit_error())?;
        if !syntax::is_static_citation_key(name) || name == "*" {
            return Err(jsonrpc::Error::invalid_params("Invalid bibliography key: use a nonempty literal key without separators or TeX expansion"));
        }
        if name == symbol.name { return Ok(Some(WorkspaceEdit::default())); }
        let database = Database::new(&symbol.declaration.uri, &ws.documents[&symbol.declaration.uri]).map_err(|_| edit_error())?;
        if database.declarations.contains_key(name) {
            return Err(jsonrpc::Error::invalid_params("The bibliography key or IDS alias already exists in the declaration database"));
        }
        symbol.graphs.name = Some(name.into());
        symbol.graphs.bindings.borrow_mut().clear();
        let relation_owners: BTreeSet<_> = symbol.graphs.databases.iter().filter(|(_, database)| database.references.iter().any(|reference| reference.name == name)).map(|(uri, _)| uri).collect();
        for project in symbol.graphs.projects.values() {
            symbol.lexical.validate(&project.scope, true, name)?;
            crate::resource_context::validate_rename(ws, &project.scope, name)?;
            for view in project.graph.sections.iter().filter(|view| view.contains(&symbol.declaration.uri)) {
                if symbol.graphs.bind(name, std::iter::once(view)).map_err(|_| edit_error())?.is_some() {
                    return Err(jsonrpc::Error::invalid_params("The bibliography key or IDS alias already exists in a section consuming this declaration"));
                }
            }
            for source in project.scope.iter().filter(|uri| ws.documents.get(*uri).is_some_and(|doc| doc.kind == DocumentKind::Latex)) {
                for span in symbol.lexical.references(source, true, name) {
                    let views = project.graph.occurrences.get(&(source.clone(), span.start, span.end)).ok_or_else(edit_error)?;
                    if views.iter().any(|view| view.contains(&symbol.declaration.uri)) {
                        return Err(jsonrpc::Error::invalid_params("Renaming this bibliography key would capture an existing citation"));
                    }
                }
            }
            if project.graph.sections.iter().filter(|view| view.contains(&symbol.declaration.uri)).any(|view| view.iter().any(|resource| relation_owners.contains(resource))) {
                return Err(jsonrpc::Error::invalid_params("Renaming this bibliography key would capture an existing bibliography relation"));
            }
        }
        let mut changes = BTreeMap::new();
        for (uri, spans) in symbol.occurrences {
            let doc = &ws.documents[&uri];
            let mut edits: Vec<_> = spans.into_iter().map(|(start,end)| TextEdit {range: doc.text.range(start,end),new_text:name.into()}).collect();
            if name.contains('"') { crate::features::brace_quoted_citation_edits(doc, &mut edits); }
            changes.insert(uri, edits);
        }
        Ok(Some(crate::features::workspace_edit(ws, changes)))
    }))
}
