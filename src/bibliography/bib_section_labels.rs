//! Project-consistent names of statically registered biblatex resources.
use crate::rpc::{Error, Result};
use crate::{
    bibliography::bib_section_files::{self, LabelBinding},
    language::signature::CommandSignatures,
    parser::syntax::{self, Argument, Command},
    project::workspace::{Document, DocumentKind, Workspace},
    symbols::named_references::Context,
};
use lsp_types::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

struct Query {
    command: usize,
    item: Range<usize>,
    selection: bool,
}

fn query(doc: &Document, signatures: &CommandSignatures<'_>, offset: usize) -> Option<Query> {
    let source = &doc.text.source;
    let context = Context::new(doc, signatures);
    let positions = signatures.source_command_positions(source, &doc.syntax);
    for (index, lexical) in doc.syntax.commands.iter().enumerate() {
        if !positions.contains(&lexical.span.start)
            || context.literal(lexical.span.start)
            || context.operand(doc, lexical.span.start)
        {
            continue;
        }
        let (name, span) = signatures
            .named_invocation(
                source,
                &doc.syntax,
                index,
                &BTreeSet::new(),
                &context.declarations,
            )
            .unwrap_or((&lexical.name, lexical.span.clone()));
        let name = signatures.forwarded_builtin(name).unwrap_or(name);
        if !signatures.command_unchanged(name) {
            continue;
        }
        if crate::bibliography::bib_resources::references_file(name) {
            let label = bib_section_files::resource_label(source, span.end)
                .ok()
                .flatten();
            if let Some(label) =
                label.filter(|label| label.span.start <= offset && offset <= label.span.end)
            {
                return Some(Query {
                    command: lexical.span.start,
                    item: label.span,
                    selection: false,
                });
            }
            continue;
        }
        let end = if matches!(name, "newrefsection" | "refsection") {
            span.end
        } else if name == "begin" && signatures.command_unchanged("refsection") {
            let Some(environment) = syntax::environment_name_argument(source, span.end)
                .filter(|argument| argument.value == "refsection")
            else {
                continue;
            };
            environment.outer_span.end
        } else {
            continue;
        };
        let at = syntax::skip_space(source, end);
        // Execution proof below checks when forwarding aliases become active;
        // a simple imported reader need not have changed the lexical catcodes.
        if source.as_bytes().get(at) != Some(&b'[')
            || crate::project::resource_context::ambiguous_header(&doc.syntax, at)
        {
            continue;
        }
        let content = syntax::verbatim_file_bracket(source, at)
            .map_or(at + 1..source.len(), |(argument, _)| argument.span);
        for segment in syntax::verbatim_list_segments(source, content) {
            let item = syntax::trimmed_span(source, segment);
            if item.start <= offset
                && offset <= item.end
                && !source[item.clone()]
                    .contains(['\\', '{', '}', '%', '#', '$', '&', '^', '~', '[', ']'])
            {
                return Some(Query {
                    command: lexical.span.start,
                    item,
                    selection: true,
                });
            }
        }
    }
    None
}

struct Target {
    query: Query,
    bindings: std::collections::BTreeMap<String, LabelBinding>,
}
fn target(ws: &Workspace, uri: &Url, offset: usize) -> Option<Target> {
    let doc = ws
        .documents
        .get(uri)
        .filter(|doc| doc.kind == DocumentKind::Latex)?;
    // Static option values contain no commands. Most editor requests can
    // therefore avoid constructing any project signatures at all.
    let before = doc
        .syntax
        .commands
        .partition_point(|command| command.span.start <= offset);
    let command = before
        .checked_sub(1)
        .and_then(|index| doc.syntax.commands.get(index))?;
    if !doc.text.source.get(command.span.end..offset)?.contains('[') {
        return None;
    }
    if !ws.index_truncated.is_empty()
        || !ws
            .documents
            .values()
            .any(|doc| doc.kind == DocumentKind::Latex && doc.text.source.contains("label"))
    {
        return None;
    }
    let (_, projects) = crate::bibliography::bib_section_keys::section_projects(ws, uri)?;
    let mut result: Option<Target> = None;
    for project in projects {
        let query = query(doc, &project.signatures, offset)?;
        let bindings = crate::bibliography::bib_section_keys::label_bindings(
            ws,
            &project.root,
            &project.docs,
            &project.signatures,
            uri,
            query.command,
        )?;
        if let Some(previous) = result.as_mut() {
            if previous.query.item != query.item || previous.query.selection != query.selection {
                return None;
            }
            previous
                .bindings
                .retain(|name, binding| bindings.get(name) == Some(binding));
        } else {
            result = Some(Target { query, bindings });
        }
    }
    result
}

pub(crate) fn definitions(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    let target = target(ws, uri, offset)?;
    let name = &ws.documents[uri].text.source[target.query.item];
    let binding = target.bindings.get(name)?;
    let doc = ws.documents.get(&binding.uri)?;
    Some(vec![Location::new(
        binding.uri.clone(),
        doc.text.range(binding.span.start, binding.span.end),
    )])
}

pub(crate) fn hover(ws: &Workspace, uri: &Url, offset: usize) -> Option<Hover> {
    let target = target(ws, uri, offset)?;
    let doc = &ws.documents[uri];
    let name = &doc.text.source[target.query.item.clone()];
    let binding = target.bindings.get(name)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::PlainText,
            value: format!(
                "{name}\n\nbiblatex resource: {}\n{}:{}",
                binding.filename,
                binding.uri.path(),
                ws.documents[&binding.uri]
                    .text
                    .position(binding.span.start)
                    .line
                    + 1
            ),
        }),
        range: Some(
            doc.text
                .range(target.query.item.start, target.query.item.end),
        ),
    })
}

pub(crate) fn completion(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<CompletionItem>> {
    let target = target(ws, uri, offset)?;
    if !target.query.selection {
        return None;
    }
    let doc = &ws.documents[uri];
    let prefix = &doc.text.source[target.query.item.start..offset];
    let range = doc
        .text
        .range(target.query.item.start, target.query.item.end);
    let mut items: Vec<_> = target
        .bindings
        .iter()
        .filter(|(name, _)| {
            name.starts_with(prefix)
                && syntax::is_static_key(name)
                && bib_section_files::literal_item(name)
                && !name.contains(['[', ']'])
        })
        .map(|(name, binding)| CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::REFERENCE),
            detail: Some(format!("biblatex resource: {}", binding.filename)),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: name.clone(),
            })),
            ..Default::default()
        })
        .collect();
    // Retain literal file suggestions with their existing insertion checks.
    let command = Command {
        name: bib_section_files::INFERRED.into(),
        span: target.query.command..target.query.command,
        argument: Some(Argument {
            value: doc.text.source[target.query.item.clone()].into(),
            span: target.query.item.clone(),
            outer_span: target.query.item.clone(),
        }),
    };
    items.extend(crate::language::path_completion::complete_source(
        ws,
        uri,
        &command,
        target.query.item.start..offset,
    ));
    Some(items)
}

fn unconfirmed() -> Error {
    Error::invalid_params(
        "The complete resource label identity cannot be confirmed in every consuming project",
    )
}

struct Graphs<'a> {
    ws: &'a Workspace,
    regions: BTreeMap<Url, BTreeSet<Url>>,
    projects: BTreeMap<Url, crate::bibliography::bib_section_keys::LabelGraph>,
    bytes: usize,
    stored: usize,
}
impl<'a> Graphs<'a> {
    fn new(ws: &'a Workspace) -> Result<Self> {
        let mut regions = ws.saved_projects();
        // Unsaved roots are consumers as soon as their buffers are opened.
        for root in ws.documents.keys().filter(|uri| ws.is_root(uri)) {
            regions
                .entry(root.clone())
                .or_insert_with(|| ws.project_documents(root));
        }
        if regions.values().map(BTreeSet::len).sum::<usize>() > 100_000 {
            return Err(unconfirmed());
        }
        Ok(Self {
            ws,
            regions,
            projects: BTreeMap::new(),
            bytes: 0,
            stored: 0,
        })
    }

    fn consumers(&self, uri: &Url) -> Vec<Url> {
        self.ws.source_roots_in(uri, &self.regions)
    }

    fn ensure(&mut self, root: &Url) -> Result<()> {
        if self.projects.contains_key(root) {
            return Ok(());
        }
        if self.projects.len() >= 64 {
            return Err(unconfirmed());
        }
        let mut scope = self
            .regions
            .get(root)
            .cloned()
            .unwrap_or_else(|| self.ws.project_documents(root));
        scope.insert(root.clone());
        let docs = crate::project::source_inputs::bounded_latex_sources(self.ws, &scope)
            .ok_or_else(unconfirmed)?;
        self.bytes += docs
            .iter()
            .map(|(_, doc)| doc.text.source.len())
            .sum::<usize>();
        if self.bytes > 16 * 1024 * 1024
            || !crate::bibliography::bib_section_keys::signature_budget(&docs)
        {
            return Err(unconfirmed());
        }
        let signatures = CommandSignatures::new(
            docs.iter()
                .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
        );
        let graph =
            crate::bibliography::bib_section_keys::label_graph(self.ws, root, &docs, &signatures)
                .ok_or_else(unconfirmed)?;
        self.stored += graph
            .choices
            .iter()
            .map(|choice| {
                choice.uri.as_str().len() + choice.name.len() + std::mem::size_of_val(choice)
            })
            .sum::<usize>();
        self.stored += graph
            .bindings
            .iter()
            .map(|(name, binding)| {
                name.len()
                    + binding.uri.as_str().len()
                    + binding.filename.len()
                    + std::mem::size_of_val(binding)
            })
            .sum::<usize>();
        self.stored += graph
            .names
            .iter()
            .map(|name| name.len() + std::mem::size_of::<String>())
            .sum::<usize>();
        if self.stored > 32 * 1024 * 1024 {
            return Err(unconfirmed());
        }
        self.projects.insert(root.clone(), graph);
        Ok(())
    }
}

struct Symbol {
    name: String,
    span: Range<usize>,
    declaration: Location,
    occurrences: BTreeMap<Url, BTreeSet<(usize, usize)>>,
    projects: BTreeMap<Url, crate::bibliography::bib_section_keys::LabelGraph>,
}

fn symbol(ws: &Workspace, uri: &Url, offset: usize) -> Option<Result<Symbol>> {
    let target = target(ws, uri, offset)?;
    let name = ws.documents[uri].text.source[target.query.item.clone()].to_string();
    let binding = target.bindings.get(&name)?;
    Some((|| {
        let mut graphs = Graphs::new(ws)?;
        let mut pending: BTreeSet<_> = graphs.consumers(&binding.uri).into_iter().collect();
        let mut checked = BTreeSet::new();
        let mut sources = BTreeSet::from([binding.uri.clone()]);
        let mut occurrences = BTreeMap::<Url, BTreeSet<(usize, usize)>>::new();
        occurrences
            .entry(binding.uri.clone())
            .or_default()
            .insert((binding.span.start, binding.span.end));
        while let Some(root) = pending.pop_first() {
            if !checked.insert(root.clone()) {
                continue;
            }
            graphs.ensure(&root)?;
            let graph = &graphs.projects[&root];
            if graph.bindings.get(&name) != Some(binding) {
                return Err(unconfirmed());
            }
            let choices: Vec<_> = graph
                .choices
                .iter()
                .filter(|choice| choice.name == name)
                .cloned()
                .collect();
            for choice in choices {
                occurrences
                    .entry(choice.uri.clone())
                    .or_default()
                    .insert((choice.span.start, choice.span.end));
                // A shared use can have consumers which never load the
                // declaration. They must agree before this source is edited.
                if sources.insert(choice.uri.clone()) {
                    pending.extend(graphs.consumers(&choice.uri));
                }
            }
        }
        if !occurrences
            .get(uri)
            .is_some_and(|items| items.contains(&(target.query.item.start, target.query.item.end)))
        {
            return Err(unconfirmed());
        }
        let declaration = Location::new(
            binding.uri.clone(),
            ws.documents[&binding.uri]
                .text
                .range(binding.span.start, binding.span.end),
        );
        Ok(Symbol {
            name,
            span: target.query.item,
            declaration,
            occurrences,
            projects: graphs.projects,
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
                    .filter(|location| include || *location != symbol.declaration)
                    .collect()
            })
            .unwrap_or_default(),
    )
}

pub(crate) fn prepare(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
) -> Option<Result<Option<PrepareRenameResponse>>> {
    Some(symbol(ws, uri, offset)?.map(|symbol| {
        Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: ws.documents[uri]
                .text
                .range(symbol.span.start, symbol.span.end),
            placeholder: symbol.name,
        })
    }))
}

pub(crate) fn rename(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    name: &str,
) -> Option<Result<Option<WorkspaceEdit>>> {
    Some(symbol(ws, uri, offset)?.and_then(|symbol| {
        if !syntax::is_static_key(name) || !bib_section_files::literal_item(name) || name.contains(['[', ']']) {
            return Err(Error::invalid_params("Invalid bibliography resource label name"));
        }
        if name != symbol.name {
            for project in symbol.projects.values() {
                if project.names.contains(name) {
                    return Err(Error::invalid_params("The target bibliography resource label already exists"));
                }
                if project.choices.iter().any(|choice| choice.name == name) {
                    return Err(Error::invalid_params("Renaming this resource label would capture a literal bibliography filename selection"));
                }
            }
        }
        let changes = symbol.occurrences.into_iter().map(|(uri, spans)| {
            let edits = spans.into_iter().map(|(start, end)| TextEdit {
                range: ws.documents[&uri].text.range(start, end), new_text: name.into(),
            }).collect();
            (uri, edits)
        }).collect();
        Ok(Some(crate::language::features::workspace_edit(ws, changes)))
    }))
}
