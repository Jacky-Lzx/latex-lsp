//! Citation resource provenance for statically ordered reference sections.
//!
//! Citation references and renames still need the identity of every database
//! declaration and bibliography relation. Resource labels have a separate graph.
use crate::{
    bib_section_files::Registry,
    named_references::Context,
    signature::CommandSignatures,
    syntax::{self, Command},
    workspace::{Document, DocumentKind, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::Arc,
};
use tower_lsp::lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind, Url};

#[derive(Clone, Copy)]
enum Purpose {
    Key,
    Completion,
}

const STATE: &[&str] = &[
    "newrefsection",
    "refsection",
    "endrefsection",
    "begin",
    "end",
    "addbibresource",
    "addglobalbib",
    "addsectionbib",
];
fn loader(name: &str) -> bool {
    matches!(name, "input" | "include" | "subfile") || syntax::loader_extension(name).is_some()
}
fn unsafe_control(name: &str) -> bool {
    name.starts_with("if")
        || name.starts_with("every")
        || matches!(
            name,
            "else"
                | "or"
                | "fi"
                | "unless"
                | "loop"
                | "repeat"
                | "whiledo"
                | "foreach"
                | "expandafter"
                | "noexpand"
                | "expanded"
                | "unexpanded"
                | "aftergroup"
                | "afterassignment"
                | "futurelet"
                | "let"
                | "edef"
                | "xdef"
                | "csedef"
                | "csxdef"
                | "cslet"
                | "csletcs"
                | "csundef"
                | "undef"
                | "catcode"
                | "makeatletter"
                | "globaldefs"
                | "count"
                | "countdef"
                | "advance"
                | "multiply"
                | "divide"
                | "scantokens"
                | "endlinechar"
                | "escapechar"
                | "ExplSyntaxOn"
                | "includeonly"
                | "InputIfFileExists"
                | "IfFileExists"
                | "import"
                | "subimport"
                | "bibliography"
                | "nobibliography"
        )
}

#[derive(Clone)]
struct Query {
    span: Range<usize>,
    command: usize,
    call: Command,
}
fn query(
    doc: &Document,
    signatures: &CommandSignatures<'_>,
    offset: usize,
    purpose: Purpose,
) -> Option<Query> {
    let context = Context::new(doc, signatures);
    let source = &doc.text.source;
    for (index, lexical) in doc.syntax.commands.iter().enumerate() {
        if context.operand(doc, lexical.span.start) || context.literal(lexical.span.start) {
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
        // Forwarded citation names require load-position meaning snapshots.
        if !syntax::is_citation(name) || !signatures.command_unchanged(name) {
            continue;
        }
        let command = crate::reference_calls::resolved_command(source, name, span);
        let span = match purpose {
            Purpose::Key => crate::reference_calls::keys(source, &command, false)
                .into_iter()
                .find(|(citation, span)| *citation && span.start <= offset && offset <= span.end)
                .map(|(_, span)| span),
            Purpose::Completion => completion_span(doc, &command, offset).map(|(span, _)| span),
        };
        if let Some(span) = span {
            return Some(Query {
                span,
                command: lexical.span.start,
                call: command,
            });
        }
    }
    None
}

fn completion_span(
    doc: &Document,
    command: &Command,
    offset: usize,
) -> Option<(Range<usize>, Vec<Range<usize>>)> {
    if !syntax::is_citation(&command.name) {
        return None;
    }
    if syntax::is_single_citation(&command.name) {
        crate::features::completion_name_argument(&doc.text.source, command, offset)
    } else {
        crate::citation_search::search_span(&doc.text.source, command, offset)
            .map(|span| (span, Vec::new()))
    }
}

fn citation_candidate(
    doc: &Document,
    signatures: &CommandSignatures<'_>,
    offset: usize,
    purpose: Purpose,
) -> bool {
    let context = Context::new(doc, signatures);
    doc.syntax
        .commands
        .iter()
        .enumerate()
        .any(|(index, lexical)| {
            if context.literal(lexical.span.start) {
                return false;
            }
            let command = crate::reference_calls::named_command(
                &doc.text.source,
                &doc.syntax,
                index,
                signatures,
                &BTreeSet::new(),
                &context.declarations,
            )
            .or_else(|| {
                syntax::is_citation(&lexical.name).then(|| {
                    crate::reference_calls::resolved_command(
                        &doc.text.source,
                        &lexical.name,
                        lexical.span.clone(),
                    )
                })
            });
            command.is_some_and(|command| match purpose {
                Purpose::Key => crate::reference_calls::keys(&doc.text.source, &command, false)
                    .iter()
                    .any(|(citation, span)| {
                        *citation && span.start <= offset && offset <= span.end
                    }),
                Purpose::Completion => completion_span(doc, &command, offset).is_some(),
            })
        })
}

#[derive(Clone)]
pub(crate) struct ResourceChoice {
    pub uri: Url,
    pub span: Range<usize>,
    pub name: String,
}

pub(crate) struct LabelGraph {
    pub bindings: BTreeMap<String, crate::bib_section_files::LabelBinding>,
    pub names: BTreeSet<String>,
    pub choices: Vec<ResourceChoice>,
}

struct LabelProof {
    command: Option<usize>,
    seen: bool,
    aliases: BTreeMap<String, String>,
    environments: Vec<String>,
    choices: Option<Vec<ResourceChoice>>,
}

struct Analysis<'a, 'b> {
    ws: &'a Workspace,
    root: &'a Url,
    signatures: &'b CommandSignatures<'a>,
    registry: Registry,
    origin: &'a Url,
    query: Option<Query>,
    // Label lookup proves execution without resolving unfinished selections.
    labels: Option<LabelProof>,
    resource_sets: BTreeSet<Arc<BTreeSet<Url>>>,
    collect_keys: bool,
    all_keys: Option<GraphKeys>,
    trace: Option<SourceTrace>,
    keys: BTreeMap<(usize, usize), Option<Arc<BTreeSet<Url>>>>,
    active: BTreeSet<Url>,
    answer: Option<BTreeSet<Url>>,
    visited: BTreeSet<Url>,
    stack: BTreeSet<Url>,
    visits: usize,
    commands: usize,
    work: usize,
    document: bool,
    ended: bool,
}
impl Analysis<'_, '_> {
    // Share one retained resource set across all keys in that section.
    fn intern_resources(&mut self) -> Option<Arc<BTreeSet<Url>>> {
        if let Some(resources) = self.resource_sets.get(&self.active) {
            return Some(resources.clone());
        }
        self.work += self
            .active
            .iter()
            .map(|uri| uri.as_str().len() + std::mem::size_of::<Url>())
            .sum::<usize>();
        if self.work > 32 * 1024 * 1024 {
            return None;
        }
        let resources = Arc::new(self.active.clone());
        self.resource_sets.insert(resources.clone());
        Some(resources)
    }
    fn walk(&mut self, uri: &Url) -> Option<()> {
        self.visits += 1;
        if self.visits > 1024 || self.stack.len() >= 32 || !self.stack.insert(uri.clone()) {
            return None;
        }
        let doc = self
            .ws
            .documents
            .get(uri)
            .filter(|doc| doc.kind == DocumentKind::Latex)?;
        self.visited.insert(uri.clone());
        let source = &doc.text.source;
        let context = Context::new(doc, self.signatures);
        let positions = self
            .signatures
            .source_command_positions(source, &doc.syntax);
        let alias_declarations: BTreeMap<_, _> = if self.labels.is_some() {
            doc.syntax
                .definitions
                .iter()
                .filter_map(|entry| {
                    let builtin = self.signatures.forwarded_builtin(&entry.name)?;
                    if !STATE.contains(&builtin) && !loader(builtin) {
                        return None;
                    }
                    let target = crate::signature::source_alias(source, &doc.syntax, entry)?;
                    let body =
                        crate::signature::command_definition_body(source, &doc.syntax, entry)?;
                    let at = crate::signature::command_declaration_start(&doc.syntax, entry)?;
                    Some((at, (entry.name.clone(), target, body.span)))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let forwarding_bodies: Vec<_> = alias_declarations
            .values()
            .map(|(_, _, body)| body)
            .collect();
        for (index, lexical) in doc.syntax.commands.iter().enumerate() {
            if self.ended {
                break;
            }
            self.commands += 1;
            if self.commands > 100_000 {
                return None;
            }
            if let Some((name, target, _)) = alias_declarations.get(&lexical.span.start) {
                let labels = self.labels.as_mut()?;
                self.work += source.len();
                if self.work > 32 * 1024 * 1024 {
                    return None;
                }
                if positions.contains(&lexical.span.start)
                    && !context.literal(lexical.span.start)
                    && !doc.syntax.control_operands.contains(&lexical.span.start)
                    && labels.environments.is_empty()
                    && !crate::resource_context::grouped(source, &doc.syntax, lexical.span.start)
                {
                    labels.aliases.insert(name.clone(), target.clone());
                }
            }
            if context.literal(lexical.span.start)
                || self.labels.is_some()
                    && context
                        .declarations
                        .contains(&(lexical.span.start, lexical.span.end))
                || forwarding_bodies
                    .iter()
                    .any(|body| body.contains(&lexical.span.start))
            {
                continue;
            }
            if unsafe_control(&lexical.name) {
                return None;
            }
            if lexical.name == "endcsname" {
                continue;
            }
            let (name, span) = self
                .signatures
                .named_invocation(
                    source,
                    &doc.syntax,
                    index,
                    &BTreeSet::new(),
                    &context.declarations,
                )
                .unwrap_or((&lexical.name, lexical.span.clone()));
            let name = if let Some(labels) = self.labels.as_ref() {
                if let Some(builtin) = self
                    .signatures
                    .forwarded_builtin(name)
                    .filter(|builtin| STATE.contains(builtin) || loader(builtin))
                {
                    let mut current = name;
                    let mut confirmed = false;
                    for _ in 0..64 {
                        if current == builtin {
                            confirmed = true;
                            break;
                        }
                        current = labels.aliases.get(current)?;
                    }
                    if !confirmed {
                        return None;
                    }
                    builtin
                } else {
                    name
                }
            } else {
                name
            };
            if matches!(name, "csuse" | "csname") {
                return None;
            }
            if unsafe_control(name)
                || name.contains("refsection") && !STATE.contains(&name) && name != "therefsection"
            {
                return None;
            }
            if matches!(
                name,
                "setcounter"
                    | "addtocounter"
                    | "stepcounter"
                    | "refstepcounter"
                    | "defcounter"
                    | "counterwithin"
                    | "counterwithout"
                    | "@addtoreset"
                    | "@removefromreset"
            ) {
                let (counter, _) =
                    syntax::group(source, syntax::skip_space(source, span.end), b'{', b'}')?;
                if !syntax::is_static_key(&counter.value) || counter.value == "refsection" {
                    return None;
                }
            }
            let environment = if matches!(name, "begin" | "end") {
                Some(syntax::environment_name_argument(source, span.end)?)
            } else {
                None
            };
            if let Some(labels) = self.labels.as_mut().filter(|_| {
                positions.contains(&lexical.span.start) && !context.operand(doc, lexical.span.start)
            }) {
                if let Some(environment) =
                    environment.as_ref().filter(|env| env.value != "document")
                {
                    if name == "begin" {
                        if labels.environments.len() >= 64 {
                            return None;
                        }
                        labels.environments.push(environment.value.clone());
                    } else if labels.environments.pop().as_deref()
                        != Some(environment.value.as_str())
                    {
                        return None;
                    }
                }
            }
            let relevant = (STATE.contains(&name)
                && environment
                    .as_ref()
                    .is_none_or(|env| matches!(env.value.as_str(), "document" | "refsection")))
                || loader(name);
            if relevant {
                self.work += source.len() + doc.syntax.commands.len();
                if self.work > 32 * 1024 * 1024 {
                    return None;
                }
            }
            if relevant
                && (!self.signatures.command_unchanged(name)
                    || !positions.contains(&lexical.span.start)
                    || context.operand(doc, lexical.span.start)
                    || self.signatures.resource_is_tokenized(
                        source,
                        &doc.syntax,
                        lexical.span.start,
                    ))
            {
                // Deferred code and ordinary macro parameters can change the
                // state later; textual ordering cannot prove when they run.
                return None;
            }
            if uri == self.origin
                && self
                    .labels
                    .as_ref()
                    .is_some_and(|labels| labels.command == Some(lexical.span.start))
            {
                if !relevant {
                    return None;
                }
                self.labels.as_mut().unwrap().seen = true;
            }
            if self.trace.is_some() {
                let declaration = self
                    .trace
                    .as_ref()?
                    .declarations
                    .get(&(uri.clone(), lexical.span.start))
                    .cloned();
                if let Some(declaration) = declaration {
                    // The declaration is shared, but processing its member list
                    // on every execution still consumes the analysis budget.
                    self.work += declaration.key.uri.as_str().len()
                        + declaration.key.name.len()
                        + 96
                        + declaration
                            .members
                            .iter()
                            .map(|member| member.uri.as_str().len() + member.name.len() + 96)
                            .sum::<usize>();
                    let trace = self.trace.as_mut()?;
                    if self.work > 32 * 1024 * 1024 || trace.events.len() >= 100_000 {
                        return None;
                    }
                    trace
                        .events
                        .push(SourceEvent::Set(trace.current, declaration));
                } else if syntax::is_citation(name) {
                    if !self.document
                        || !positions.contains(&lexical.span.start)
                        || context.operand(doc, lexical.span.start)
                        || !self.signatures.command_unchanged(name)
                        || !syntax::warns_on_undefined_citation(name)
                    {
                        return None;
                    }
                    let command =
                        crate::reference_calls::resolved_command(source, name, span.clone());
                    let fields = if let Some(style) = crate::citations::style(name) {
                        let arguments = crate::citations::arguments(source, span.end, style);
                        if arguments.fields.iter().any(|field| !field.complete) {
                            return None;
                        }
                        arguments
                            .keys()
                            .map(|argument| argument.span.clone())
                            .collect::<Vec<_>>()
                    } else {
                        vec![command.argument.as_ref()?.span.clone()]
                    };
                    if fields.is_empty() {
                        return None;
                    }
                    for field in fields {
                        let keys = source_key_list(uri, source, field)?;
                        if keys.iter().any(|key| key.name == "*")
                            && (name != "nocite" || keys.len() != 1)
                        {
                            return None;
                        }
                        for key in keys {
                            self.work += key.uri.as_str().len() + key.name.len() + 96;
                            let trace = self.trace.as_mut()?;
                            if self.work > 32 * 1024 * 1024 || trace.events.len() >= 100_000 {
                                return None;
                            }
                            let wildcard = key.name == "*";
                            trace
                                .events
                                .push(SourceEvent::Request(trace.current, key, wildcard));
                        }
                    }
                } else if self
                    .signatures
                    .forwarded_builtin(name)
                    .is_some_and(syntax::is_citation)
                {
                    return None;
                }
            }
            if (self.collect_keys && uri == self.origin || self.all_keys.is_some())
                && self.document
                && positions.contains(&lexical.span.start)
                && !context.operand(doc, lexical.span.start)
                && syntax::is_citation(name)
                && self.signatures.command_unchanged(name)
            {
                let command = crate::reference_calls::resolved_command(source, name, span.clone());
                let resources = self.intern_resources()?;
                for (citation, key) in crate::reference_calls::keys(source, &command, true) {
                    if !citation {
                        continue;
                    }
                    let key = (key.start, key.end);
                    if let Some(all_keys) = &mut self.all_keys {
                        let visits = all_keys.entry((uri.clone(), key.0, key.1)).or_default();
                        if visits.insert(resources.clone()) {
                            self.work += uri.as_str().len() + 128;
                            if self.work > 32 * 1024 * 1024 || all_keys.len() > 100_000 {
                                return None;
                            }
                        }
                    }
                    if self.collect_keys && uri == self.origin {
                        if let Some(previous) = self.keys.get_mut(&key) {
                            if previous
                                .as_ref()
                                .is_some_and(|previous| !Arc::ptr_eq(previous, &resources))
                            {
                                *previous = None;
                            }
                        } else {
                            if self.keys.len() >= 100_000 {
                                return None;
                            }
                            self.keys.insert(key, Some(resources.clone()));
                        }
                    }
                }
            }
            if uri == self.origin
                && self
                    .query
                    .as_ref()
                    .is_some_and(|query| lexical.span.start == query.command)
            {
                if !self.document || !positions.contains(&lexical.span.start) {
                    return None;
                }
                if self
                    .answer
                    .as_ref()
                    .is_some_and(|previous| *previous != self.active)
                {
                    return None;
                }
                self.answer = Some(self.active.clone());
            }
            if !relevant {
                continue;
            }
            let command = Command {
                name: name.into(),
                span: span.clone(),
                argument: if span == lexical.span {
                    lexical.argument.clone()
                } else {
                    syntax::source_input_argument(source, name, span.end)
                },
            };
            if crate::bib_resources::references_file(name) {
                if self.document
                    || crate::resource_context::grouped(source, &doc.syntax, span.start)
                {
                    return None;
                }
            } else if matches!(name, "begin" | "end") {
                let environment = syntax::environment_name_argument(source, span.end)?;
                if environment.value == "document" {
                    if name == "begin" {
                        self.document = true;
                    } else {
                        self.ended = true;
                    }
                } else if environment.value == "refsection" {
                    if name == "begin" {
                        self.select(uri, doc, environment.outer_span.end)?;
                    } else if self.labels.is_none() {
                        self.active = self.registry.resources(self.root, None)?;
                        if let Some(trace) = self.trace.as_mut() {
                            trace.current = 0;
                        }
                    }
                }
            } else if matches!(name, "newrefsection" | "refsection") {
                self.select(uri, doc, span.end)?;
            } else if name == "endrefsection" {
                if self.labels.is_none() {
                    self.active = self.registry.resources(self.root, None)?;
                    if let Some(trace) = self.trace.as_mut() {
                        trace.current = 0;
                    }
                }
            } else if loader(name) {
                if self.labels.is_some()
                    && crate::resource_context::grouped(source, &doc.syntax, span.start)
                {
                    return None;
                }
                let command = doc.syntax.potential_file_command(source, &command);
                let static_name = if syntax::file_list(name) {
                    syntax::complete_file_list_spans(source, &command).is_some()
                } else {
                    syntax::file_argument(source, &command)
                        .is_some_and(|argument| syntax::is_literal_file_path(&argument.value))
                };
                if !static_name {
                    return None;
                }
                let dependencies = self
                    .ws
                    .literal_command_dependencies(uri, self.root, &command);
                if matches!(name, "input" | "include" | "subfile") && dependencies.is_empty() {
                    return None;
                }
                for (dependency, kind) in dependencies {
                    if kind == DocumentKind::Latex {
                        self.walk(&dependency)?;
                    }
                }
            }
        }
        self.stack.remove(uri);
        Some(())
    }
    fn select(&mut self, uri: &Url, doc: &Document, end: usize) -> Option<()> {
        if !self.document {
            return None;
        }
        let source = &doc.text.source;
        let at = syntax::skip_space(source, end);
        if crate::resource_context::ambiguous_header(&doc.syntax, at) {
            return None;
        }
        let selected = if source.as_bytes().get(at) == Some(&b'[') {
            let raw = doc.syntax.raw_resource_brackets.contains(&at);
            let parsed = if raw {
                syntax::verbatim_file_bracket(source, at)
            } else {
                syntax::bracket_argument(source, at)
            };
            let argument = if let Some((argument, _)) = parsed {
                argument
            } else if self
                .labels
                .as_ref()
                .is_some_and(|labels| labels.choices.is_none())
            {
                syntax::Argument {
                    value: source[at + 1..].into(),
                    span: at + 1..source.len(),
                    outer_span: at..source.len(),
                }
            } else {
                return None;
            };

            let segments = if raw {
                syntax::verbatim_list_segments(source, argument.span)
            } else {
                syntax::key_list_segments(source, argument.span)
            };
            let mut items = Vec::new();
            for span in segments {
                let span = syntax::trimmed_span(source, span);
                let value = &source[span.clone()];
                if value.is_empty() {
                    continue;
                }
                let supported = if self.labels.is_some() {
                    crate::bib_section_files::literal_item(value)
                } else {
                    self.registry.is_label(value) || crate::bib_section_files::editable_item(value)
                };
                if !supported {
                    return None;
                }
                if let Some(choices) = self
                    .labels
                    .as_mut()
                    .and_then(|labels| labels.choices.as_mut())
                {
                    self.work +=
                        value.len() + uri.as_str().len() + std::mem::size_of::<ResourceChoice>();
                    if self.work > 32 * 1024 * 1024 || choices.len() >= 100_000 {
                        return None;
                    }
                    choices.push(ResourceChoice {
                        uri: uri.clone(),
                        span,
                        name: value.into(),
                    });
                }
                items.push(value.to_owned());
            }
            Some(items)
        } else {
            None
        };
        if self.labels.is_some() {
            return Some(());
        }
        self.active = self.registry.resources(self.root, selected.as_deref())?;
        if self.query.is_none() {
            let resources = self.intern_resources()?;
            if let Some(trace) = self.trace.as_mut() {
                self.work += 64;
                if self.work > 32 * 1024 * 1024 || trace.next >= 100_000 {
                    return None;
                }
                trace.next += 1;
                trace.current = trace.next;
                trace.resources.insert(trace.current, resources);
            }
        }
        Some(())
    }
}

pub(crate) fn signature_budget(docs: &[(Url, &Document)]) -> bool {
    docs.iter()
        .map(|(_, doc)| doc.syntax.commands.len())
        .sum::<usize>()
        <= 100_000
        && docs
            .iter()
            .map(|(_, doc)| doc.syntax.definitions.len() + doc.syntax.environments.len())
            .sum::<usize>()
            <= 4096
}

// All resource consumers share the same execution and path checks. A failed
// proof suppresses cross-library diagnostics rather than asserting a union.
fn valid_sources(docs: &[(Url, &Document)], signatures: &CommandSignatures<'_>) -> bool {
    !STATE.iter().any(|name| !signatures.command_unchanged(name))
        && docs.iter().all(|(_, doc)| {
            if doc
                .syntax
                .contextual_resource_readers
                .values()
                .flatten()
                .any(|reader| reader.opaque)
            {
                return false;
            }
            let compact: String = doc
                .text
                .source
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect();
            !compact.contains("refsection=") && !compact.contains("^^")
        })
}

// Dynamic sets and handwritten bibliography items share the key namespace.
// A resource-only model cannot certify their section-dependent identities.
fn database_only(docs: &[(Url, &Document)], signatures: &CommandSignatures<'_>) -> bool {
    docs.iter().all(|(_, doc)| {
        if !doc.syntax.citations.is_empty() {
            return false;
        }
        let context = Context::new(doc, signatures);
        !doc.syntax
            .commands
            .iter()
            .enumerate()
            .any(|(index, command)| {
                if context.literal(command.span.start) {
                    return false;
                }
                let (name, _) = signatures
                    .named_invocation(
                        &doc.text.source,
                        &doc.syntax,
                        index,
                        &BTreeSet::new(),
                        &context.declarations,
                    )
                    .unwrap_or((&command.name, command.span.clone()));
                ["defbibentryset", "bibitem"]
                    .iter()
                    .any(|builtin| name == *builtin || signatures.invokes_builtin(name, builtin))
            })
    })
}

pub(crate) enum Coexistence {
    Union,
    Unknown,
    Sections(BTreeSet<Arc<BTreeSet<Url>>>),
}
impl Coexistence {
    pub(crate) fn permits(&self, a: &Url, b: &Url) -> bool {
        match self {
            Self::Union => true,
            Self::Unknown => false,
            Self::Sections(sections) => sections
                .iter()
                .any(|resources| resources.contains(a) && resources.contains(b)),
        }
    }
}

pub(crate) fn coexistence(ws: &Workspace, root: &Url, scope: &BTreeSet<Url>) -> Coexistence {
    let Some(docs) = crate::source_inputs::bounded_latex_sources(ws, scope) else {
        return Coexistence::Unknown;
    };
    if !signature_budget(&docs) {
        return Coexistence::Unknown;
    }
    let signatures = CommandSignatures::new(
        docs.iter()
            .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
    );
    let selected = crate::bib_sections::selects_resources(
        &docs.iter().map(|(_, doc)| *doc).collect::<Vec<_>>(),
        &signatures,
    );
    let modern = docs.iter().any(|(_, doc)| {
        let context = Context::new(doc, &signatures);
        doc.syntax
            .commands
            .iter()
            .enumerate()
            .any(|(index, command)| {
                if context.literal(command.span.start) {
                    return false;
                }
                let (name, _) = signatures
                    .named_invocation(
                        &doc.text.source,
                        &doc.syntax,
                        index,
                        &BTreeSet::new(),
                        &context.declarations,
                    )
                    .unwrap_or((&command.name, command.span.clone()));
                ["addbibresource", "addglobalbib", "addsectionbib"]
                    .iter()
                    .any(|builtin| signatures.invokes_builtin(name, builtin))
            })
    }) || selected;
    if !modern {
        return Coexistence::Union;
    }
    if selected && (!database_only(&docs, &signatures) || !static_databases(ws, root)) {
        return Coexistence::Unknown;
    }
    let sections = (|| {
        if !ws.index_truncated.is_empty() || !valid_sources(&docs, &signatures) {
            return None;
        }
        let registry = Registry::new(root, &docs, &signatures);
        let active = registry.resources(root, None)?;
        let mut analysis = Analysis {
            ws,
            root,
            signatures: &signatures,
            registry,
            origin: root,
            query: None,
            labels: None,
            resource_sets: BTreeSet::from([Arc::new(active.clone())]),
            collect_keys: false,
            all_keys: None,
            trace: None,
            keys: BTreeMap::new(),
            active,
            answer: None,
            visited: BTreeSet::new(),
            stack: BTreeSet::new(),
            visits: 0,
            commands: 0,
            work: 0,
            document: false,
            ended: false,
        };
        analysis.walk(root)?;
        if docs.iter().any(|(uri, _)| !analysis.visited.contains(uri)) {
            return None;
        }
        Some(analysis.resource_sets)
    })();
    sections.map_or(Coexistence::Unknown, Coexistence::Sections)
}

struct Target {
    span: Range<usize>,
    resources: BTreeSet<Url>,
    call: Command,
}
// If a resource analysis limit is reached, do not fall back to a union
// target that the ordinary key index would otherwise expose.
fn incomplete(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    purpose: Purpose,
) -> Option<Option<Target>> {
    let supported = match purpose {
        Purpose::Key => crate::named_references::Index::new(ws, uri)
            .target(uri, offset)
            .is_some_and(|(citation, _)| citation),
        Purpose::Completion => {
            crate::named_references::completion_context(ws, uri, offset, |command| {
                if !syntax::is_citation(&command.name) {
                    return None;
                }
                let doc = ws.documents.get(uri)?;
                completion_span(doc, command, offset).map(|(span, _)| span.start..offset)
            })
            .is_some()
        }
    };
    supported.then_some(None)
}
pub(crate) struct Project<'a> {
    pub root: Url,
    pub docs: Vec<(Url, &'a Document)>,
    pub signatures: CommandSignatures<'a>,
}
pub(crate) fn section_projects<'a>(
    ws: &'a Workspace,
    uri: &Url,
) -> Option<(bool, Vec<Project<'a>>)> {
    let mut roots: BTreeSet<_> = ws.source_roots(uri).into_iter().collect();
    // Opened roots consume shared files even before their own path is saved.
    for candidate in ws
        .documents
        .keys()
        .filter(|candidate| ws.is_root(candidate))
    {
        if ws.project_documents(candidate).contains(uri) {
            roots.insert(candidate.clone());
        }
        if roots.len() > 64 {
            return None;
        }
    }
    let mut projects = Vec::new();
    let mut selected = false;
    let mut bytes = 0usize;
    for root in roots {
        let mut scope = ws.project_documents(&root);
        scope.insert(root.clone());
        let docs = crate::source_inputs::bounded_latex_sources(ws, &scope)?;
        if !signature_budget(&docs) {
            return None;
        }
        bytes += docs
            .iter()
            .map(|(_, doc)| doc.text.source.len())
            .sum::<usize>();
        if bytes > 16 * 1024 * 1024 {
            return None;
        }
        let signatures = CommandSignatures::new(
            docs.iter()
                .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
        );
        selected |= crate::bib_sections::selects_resources(
            &docs.iter().map(|(_, doc)| *doc).collect::<Vec<_>>(),
            &signatures,
        );
        projects.push(Project {
            root,
            docs,
            signatures,
        });
    }
    Some((selected, projects))
}

/// None leaves ordinary projects to the existing symbol model; Some(None)
/// owns a section-dependent citation whose provenance cannot be certified.
fn target(ws: &Workspace, uri: &Url, offset: usize, purpose: Purpose) -> Option<Option<Target>> {
    let doc = ws
        .documents
        .get(uri)
        .filter(|doc| doc.kind == DocumentKind::Latex)?;
    if !ws
        .documents
        .values()
        .any(|doc| doc.kind == DocumentKind::Latex && doc.text.source.contains("refsection"))
        || !ws.index_truncated.is_empty()
    {
        return None;
    }
    let Some((selected, projects)) = section_projects(ws, uri) else {
        return incomplete(ws, uri, offset, purpose);
    };
    if !selected
        || !projects
            .iter()
            .any(|project| citation_candidate(doc, &project.signatures, offset, purpose))
    {
        return None;
    }
    Some((|| {
        let mut result: Option<Target> = None;
        for Project {
            root,
            docs,
            signatures,
        } in &projects
        {
            let query = query(doc, signatures, offset, purpose)?;
            if !valid_sources(docs, signatures)
                || !database_only(docs, signatures)
                || !static_databases(ws, root)
            {
                return None;
            }
            let registry = Registry::new(root, docs, signatures);
            let active = registry.resources(root, None)?;
            let mut analysis = Analysis {
                ws,
                root,
                signatures,
                registry,
                origin: uri,
                query: Some(query.clone()),
                labels: None,
                resource_sets: BTreeSet::new(),
                collect_keys: false,
                all_keys: None,
                trace: None,
                keys: BTreeMap::new(),
                active,
                answer: None,
                visited: BTreeSet::new(),
                stack: BTreeSet::new(),
                visits: 0,
                commands: 0,
                work: 0,
                document: false,
                ended: false,
            };
            analysis.walk(root)?;
            // A potential input that was never reached must not contribute a
            // resource registration from the project's discovery graph.
            if docs.iter().any(|(uri, _)| !analysis.visited.contains(uri)) {
                return None;
            }
            let resources = analysis.answer?;
            if resources.iter().any(|uri| {
                !ws.documents
                    .get(uri)
                    .is_some_and(|doc| doc.kind == DocumentKind::Bibtex)
            }) {
                return None;
            }
            if result.as_ref().is_some_and(|previous| {
                previous.span != query.span
                    || previous.resources != resources
                    || previous.call.name != query.call.name
                    || previous.call.span != query.call.span
            }) {
                return None;
            }
            result = Some(Target {
                span: query.span,
                resources,
                call: query.call,
            });
        }
        result
    })())
}

pub(crate) fn definitions(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    let Some(target) = target(ws, uri, offset, Purpose::Key)? else {
        return Some(Vec::new());
    };
    let name = &ws.documents[uri].text.source[target.span];
    Some(
        target
            .resources
            .iter()
            .flat_map(|uri| {
                let doc = &ws.documents[uri];
                doc.syntax
                    .citations
                    .iter()
                    .chain(&doc.syntax.bibliography_aliases)
                    .filter(move |entry| entry.name == name)
                    .map(move |entry| Location {
                        uri: uri.clone(),
                        range: doc.text.range(entry.span.start, entry.span.end),
                    })
            })
            .collect(),
    )
}
pub(crate) fn hover(ws: &Workspace, uri: &Url, offset: usize) -> Option<Option<Hover>> {
    let target = target(ws, uri, offset, Purpose::Key)?;
    Some(target.and_then(|target| {
        let source = &ws.documents[uri];
        let name = &source.text.source[target.span.clone()];
        let mut entries = target.resources.iter().flat_map(|uri| {
            let doc = &ws.documents[uri];
            doc.syntax
                .citations
                .iter()
                .chain(&doc.syntax.bibliography_aliases)
                .filter(move |entry| entry.name == name)
                .map(move |entry| (uri, doc, entry))
        });
        let (entry_uri, doc, entry) = entries.next()?;
        if entries.next().is_some() {
            return None;
        }
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::PlainText,
                value: format!(
                    "{}\n\n{}\n{}:{}",
                    entry.name,
                    entry.detail,
                    entry_uri.path(),
                    doc.text.position(entry.span.start).line + 1
                ),
            }),
            range: Some(source.text.range(target.span.start, target.span.end)),
        })
    }))
}

pub(crate) fn completion(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
) -> Option<Vec<tower_lsp::lsp_types::CompletionItem>> {
    let Some(target) = target(ws, uri, offset, Purpose::Completion)? else {
        return Some(Vec::new());
    };
    let doc = ws.documents.get(uri)?;
    let Some((_, removals)) = completion_span(doc, &target.call, offset) else {
        return Some(Vec::new());
    };
    let mut items =
        crate::citation_search::items(ws, doc, offset, target.span, &target.resources, true)
            .unwrap_or_default();
    if !removals.is_empty() {
        let removals: Vec<_> = removals
            .into_iter()
            .map(|span| tower_lsp::lsp_types::TextEdit {
                range: doc.text.range(span.start, span.end),
                new_text: String::new(),
            })
            .collect();
        for item in &mut items {
            item.additional_text_edits
                .get_or_insert_with(Vec::new)
                .extend(removals.iter().cloned());
        }
    }
    Some(items)
}

pub(crate) type ResourceMap = BTreeMap<(usize, usize), Arc<BTreeSet<Url>>>;
/// None uses ordinary project semantics. Some owns section-dependent calls;
/// missing spans are unproved and must not fall back to a bibliography union.
pub(crate) fn occurrence_resources(ws: &Workspace, uri: &Url) -> Option<ResourceMap> {
    let doc = ws.documents.get(uri)?;
    if doc.kind != DocumentKind::Latex
        || !ws
            .documents
            .values()
            .any(|doc| doc.kind == DocumentKind::Latex && doc.text.source.contains("refsection"))
    {
        return None;
    }
    let Some((selected, projects)) = section_projects(ws, uri) else {
        return Some(BTreeMap::new());
    };
    if !selected {
        return None;
    }
    Some(
        (|| {
            if !ws.index_truncated.is_empty() {
                return None;
            }
            let mut result: Option<ResourceMap> = None;
            for Project {
                root,
                docs,
                signatures,
            } in &projects
            {
                if !valid_sources(docs, signatures)
                    || !database_only(docs, signatures)
                    || !static_databases(ws, root)
                {
                    return None;
                }
                let registry = Registry::new(root, docs, signatures);
                let active = registry.resources(root, None)?;
                let mut analysis = Analysis {
                    ws,
                    root,
                    signatures,
                    registry,
                    origin: uri,
                    query: None,
                    labels: None,
                    resource_sets: BTreeSet::from([Arc::new(active.clone())]),
                    collect_keys: true,
                    all_keys: None,
                    trace: None,
                    keys: BTreeMap::new(),
                    active,
                    answer: None,
                    visited: BTreeSet::new(),
                    stack: BTreeSet::new(),
                    visits: 0,
                    commands: 0,
                    work: 0,
                    document: false,
                    ended: false,
                };
                analysis.walk(root)?;
                if docs.iter().any(|(uri, _)| !analysis.visited.contains(uri))
                    || analysis.resource_sets.iter().any(|resources| {
                        resources.iter().any(|uri| {
                            !ws.documents
                                .get(uri)
                                .is_some_and(|doc| doc.kind == DocumentKind::Bibtex)
                        })
                    })
                {
                    return None;
                }
                let keys: ResourceMap = analysis
                    .keys
                    .into_iter()
                    .filter_map(|(span, resources)| resources.map(|resources| (span, resources)))
                    .collect();
                if let Some(previous) = &mut result {
                    let mut agreement = BTreeMap::new();
                    previous.retain(|span, resources| {
                        keys.get(span).is_some_and(|candidate| {
                            let identity = (
                                Arc::as_ptr(resources) as usize,
                                Arc::as_ptr(candidate) as usize,
                            );
                            *agreement
                                .entry(identity)
                                .or_insert_with(|| resources.as_ref() == candidate.as_ref())
                        })
                    });
                } else {
                    result = Some(keys);
                }
            }
            result
        })()
        .unwrap_or_default(),
    )
}

/// Certify the registration dictionary separately from the selected resources.
/// Incomplete names can be completed without pretending they already resolve.
pub(crate) fn label_bindings(
    ws: &Workspace,
    root: &Url,
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
    uri: &Url,
    command: usize,
) -> Option<BTreeMap<String, crate::bib_section_files::LabelBinding>> {
    Some(analyze_labels(ws, root, docs, signatures, uri, Some(command), false)?.bindings)
}

pub(crate) fn label_graph(
    ws: &Workspace,
    root: &Url,
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
) -> Option<LabelGraph> {
    analyze_labels(ws, root, docs, signatures, root, None, true)
}

fn analyze_labels(
    ws: &Workspace,
    root: &Url,
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
    uri: &Url,
    command: Option<usize>,
    complete: bool,
) -> Option<LabelGraph> {
    if !valid_sources(docs, signatures) {
        return None;
    }
    let registry = Registry::new(root, docs, signatures);
    let mut analysis = Analysis {
        ws,
        root,
        signatures,
        registry,
        origin: uri,
        query: None,
        labels: Some(LabelProof {
            command,
            seen: false,
            aliases: BTreeMap::new(),
            environments: Vec::new(),
            choices: complete.then(Vec::new),
        }),
        resource_sets: BTreeSet::new(),
        collect_keys: false,
        all_keys: None,
        trace: None,
        keys: BTreeMap::new(),
        active: BTreeSet::new(),
        answer: None,
        visited: BTreeSet::new(),
        stack: BTreeSet::new(),
        visits: 0,
        commands: 0,
        work: 0,
        document: false,
        ended: false,
    };
    analysis.walk(root)?;
    let labels = analysis.labels?;
    if command.is_some() && !labels.seen
        || docs.iter().any(|(uri, _)| !analysis.visited.contains(uri))
    {
        return None;
    }
    Some(LabelGraph {
        bindings: analysis.registry.label_bindings()?,
        names: if complete {
            analysis.registry.label_names()
        } else {
            BTreeSet::new()
        },
        choices: labels.choices.unwrap_or_default(),
    })
}

pub(crate) type GraphKeys = BTreeMap<(Url, usize, usize), BTreeSet<Arc<BTreeSet<Url>>>>;
pub(crate) struct KeyGraph {
    pub sections: BTreeSet<Arc<BTreeSet<Url>>>,
    pub occurrences: GraphKeys,
}

/// Collect every source occurrence once per root, preserving all resource
/// views of a repeatedly loaded source for later declaration-identity checks.
pub(crate) fn key_graph(
    ws: &Workspace,
    root: &Url,
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
) -> Option<KeyGraph> {
    if !ws.index_truncated.is_empty()
        || !valid_sources(docs, signatures)
        || !database_only(docs, signatures)
        || !static_databases(ws, root)
    {
        return None;
    }
    let registry = Registry::new(root, docs, signatures);
    let active = registry.resources(root, None)?;
    let mut analysis = Analysis {
        ws,
        root,
        signatures,
        registry,
        origin: root,
        query: None,
        labels: None,
        resource_sets: BTreeSet::from([Arc::new(active.clone())]),
        collect_keys: false,
        all_keys: Some(BTreeMap::new()),
        trace: None,
        keys: BTreeMap::new(),
        active,
        answer: None,
        visited: BTreeSet::new(),
        stack: BTreeSet::new(),
        visits: 0,
        commands: 0,
        work: 0,
        document: false,
        ended: false,
    };
    analysis.walk(root)?;
    if docs.iter().any(|(uri, _)| !analysis.visited.contains(uri))
        || analysis.resource_sets.iter().any(|resources| {
            resources.iter().any(|uri| {
                !ws.documents
                    .get(uri)
                    .is_some_and(|doc| doc.kind == DocumentKind::Bibtex)
            })
        })
    {
        return None;
    }
    Some(KeyGraph {
        sections: analysis.resource_sets,
        occurrences: analysis.all_keys?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceKey {
    pub uri: Url,
    pub span: std::ops::Range<usize>,
    pub name: String,
}
#[derive(Clone, Debug)]
pub(crate) struct SourceSet {
    pub key: SourceKey,
    pub members: Vec<SourceKey>,
}
type SourceDeclarations = BTreeMap<(Url, usize), Arc<SourceSet>>;

#[derive(Clone, Debug)]
pub(crate) enum SourceEvent {
    Set(u32, Arc<SourceSet>),
    Request(u32, SourceKey, bool),
}
pub(crate) struct SourceTrace {
    pub work: usize,
    pub resources: BTreeMap<u32, Arc<BTreeSet<Url>>>,
    pub events: Vec<SourceEvent>,
    declarations: SourceDeclarations,
    current: u32,
    next: u32,
}

pub(crate) fn has_source_sets(
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
) -> bool {
    docs.iter().any(|(_, doc)| {
        let context = Context::new(doc, signatures);
        doc.syntax
            .commands
            .iter()
            .enumerate()
            .any(|(index, command)| {
                if context.literal(command.span.start) {
                    return false;
                }
                let (name, _) = signatures
                    .named_invocation(
                        &doc.text.source,
                        &doc.syntax,
                        index,
                        &BTreeSet::new(),
                        &context.declarations,
                    )
                    .unwrap_or((&command.name, command.span.clone()));
                name == "defbibentryset" || signatures.invokes_builtin(name, "defbibentryset")
            })
    })
}

fn source_key_list(
    uri: &Url,
    source: &str,
    span: std::ops::Range<usize>,
) -> Option<Vec<SourceKey>> {
    if source[span.clone()]
        .bytes()
        .filter(|byte| *byte == b',')
        .take(100_001)
        .count()
        > 100_000
    {
        return None;
    }
    let mut result = Vec::new();
    let mut stored = 0usize;
    for segment in syntax::key_list_segments(source, span) {
        let mut names = syntax::citation_list_spans(source, segment).into_iter();
        let span = names.next()?;
        if names.next().is_some() {
            return None;
        }
        stored += uri.as_str().len() + span.len() + 96;
        if stored > 32 * 1024 * 1024 || result.len() >= 100_000 {
            return None;
        }
        result.push(SourceKey {
            uri: uri.clone(),
            name: source[span.clone()].into(),
            span,
        });
    }
    (!result.is_empty()).then_some(result)
}

fn source_declarations(
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
) -> Option<(SourceDeclarations, usize)> {
    if !signatures.command_unchanged("defbibentryset") {
        return None;
    }
    let mut result = BTreeMap::new();
    let mut work = 0usize;
    for (uri, doc) in docs {
        let source = &doc.text.source;
        let context = Context::new(doc, signatures);
        let positions = signatures.source_command_positions(source, &doc.syntax);
        let mut declared = BTreeSet::new();
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            if context.literal(command.span.start) {
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
                .unwrap_or((&command.name, command.span.clone()));
            if name != "defbibentryset" {
                if signatures.invokes_builtin(name, "defbibentryset") {
                    return None;
                }
                continue;
            }
            if !positions.contains(&command.span.start) || context.operand(doc, command.span.start)
            {
                return None;
            }
            let arguments =
                crate::citations::arguments(source, span.end, crate::citations::Style::Set);
            if arguments.fields.len() != 2 || arguments.fields.iter().any(|field| !field.complete) {
                return None;
            }
            let key =
                syntax::single_citation_argument(source, arguments.fields[0].argument.clone());
            if !syntax::is_static_citation_key(&key.value) || key.value == "*" {
                return None;
            }
            let members = source_key_list(uri, source, arguments.fields[1].argument.span.clone())?;
            if members.iter().any(|member| member.name == "*") {
                return None;
            }
            work += 2 * uri.as_str().len()
                + key.value.len()
                + 192
                + members
                    .iter()
                    .map(|member| member.uri.as_str().len() + member.name.len() + 96)
                    .sum::<usize>();
            if work > 32 * 1024 * 1024 || result.len() >= 100_000 {
                return None;
            }
            declared.insert((key.span.start, key.span.end));
            result.insert(
                (uri.clone(), command.span.start),
                Arc::new(SourceSet {
                    key: SourceKey {
                        uri: uri.clone(),
                        span: key.span,
                        name: key.value,
                    },
                    members,
                }),
            );
        }
        // Any other source bibliography declaration has an unproved namespace.
        if doc
            .syntax
            .citations
            .iter()
            .any(|entry| !declared.contains(&(entry.span.start, entry.span.end)))
        {
            return None;
        }
    }
    Some((result, work))
}

pub(crate) fn source_trace(
    ws: &Workspace,
    root: &Url,
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
) -> Option<SourceTrace> {
    if !ws.index_truncated.is_empty()
        || !valid_sources(docs, signatures)
        || !static_databases(ws, root)
    {
        return None;
    }
    let (declarations, mut work) = source_declarations(docs, signatures)?;
    let registry = Registry::new(root, docs, signatures);
    let active = registry.resources(root, None)?;
    work += active
        .iter()
        .map(|uri| uri.as_str().len() + 96)
        .sum::<usize>();
    if work > 32 * 1024 * 1024 {
        return None;
    }
    let resources = Arc::new(active.clone());
    let trace = SourceTrace {
        work: 0,
        resources: BTreeMap::from([(0, resources.clone())]),
        events: Vec::new(),
        declarations,
        current: 0,
        next: 0,
    };
    let mut analysis = Analysis {
        ws,
        root,
        signatures,
        registry,
        origin: root,
        query: None,
        labels: None,
        resource_sets: BTreeSet::from([resources]),
        collect_keys: false,
        all_keys: None,
        trace: Some(trace),
        keys: BTreeMap::new(),
        active,
        answer: None,
        visited: BTreeSet::new(),
        stack: BTreeSet::new(),
        visits: 0,
        commands: 0,
        work,
        document: false,
        ended: false,
    };
    analysis.walk(root)?;
    if docs.iter().any(|(uri, _)| !analysis.visited.contains(uri))
        || analysis.resource_sets.iter().any(|view| {
            view.iter().any(|uri| {
                !ws.documents
                    .get(uri)
                    .is_some_and(|doc| doc.kind == DocumentKind::Bibtex)
            })
        })
    {
        return None;
    }
    let mut trace = analysis.trace?;
    trace.work = analysis.work;
    Some(trace)
}

// Recognize a potential declaration for read-only lookup. source_trace still
// proves its execution and every consuming project before returning a target.
pub(crate) fn source_set_key(
    doc: &Document,
    signatures: &CommandSignatures<'_>,
    offset: usize,
) -> Option<(String, std::ops::Range<usize>)> {
    let context = Context::new(doc, signatures);
    for (index, command) in doc.syntax.commands.iter().enumerate() {
        if context.literal(command.span.start) {
            continue;
        }
        let (name, span) = signatures
            .named_invocation(
                &doc.text.source,
                &doc.syntax,
                index,
                &BTreeSet::new(),
                &context.declarations,
            )
            .unwrap_or((&command.name, command.span.clone()));
        if name != "defbibentryset" {
            continue;
        }
        let arguments =
            crate::citations::arguments(&doc.text.source, span.end, crate::citations::Style::Set);
        let key = syntax::single_citation_argument(
            &doc.text.source,
            arguments.fields.first()?.argument.clone(),
        );
        if syntax::is_static_citation_key(&key.value)
            && key.span.start <= offset
            && offset <= key.span.end
        {
            return Some((key.value, key.span));
        }
    }
    None
}

// A bibliography preamble executes TeX from generated BBL input and can
// introduce section-specific sets before the first source citation. Until its
// effects are proved, no database-only graph can certify that namespace.
fn static_databases(ws: &Workspace, root: &Url) -> bool {
    let mut bytes = 0usize;
    let mut entries = 0usize;
    for uri in ws.project_documents(root) {
        let Some(doc) = ws.documents.get(&uri) else {
            return false;
        };
        if doc.kind != DocumentKind::Bibtex {
            continue;
        }
        bytes += doc.text.source.len();
        entries += doc.syntax.bibliography_completion.types.len();
        if bytes > 16 * 1024 * 1024
            || entries > 100_000
            || doc
                .syntax
                .bibliography_completion
                .types
                .iter()
                .any(|entry| doc.text.source[entry.span.clone()].eq_ignore_ascii_case("preamble"))
        {
            return false;
        }
    }
    true
}
