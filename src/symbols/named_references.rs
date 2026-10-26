//! Label/citation keys whose meanings agree in every consuming project.
use crate::rpc as jsonrpc;
use crate::{
    parser::syntax,
    project::workspace::{Document, DocumentKind, Workspace},
    symbols::commands::CommandSignatures,
    symbols::reference_calls,
};
use lsp_types::Url;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

#[derive(Clone, PartialEq)]
struct Key {
    citation: bool,
    span: Range<usize>,
    name: String,
    operand: bool,
}
type Calls = BTreeMap<usize, Key>;
#[derive(Default)]
pub(crate) struct Index {
    certain: BTreeMap<Url, Calls>,
    ambiguous: Vec<(Url, bool, String)>,
    incomplete: bool,
    section_resources: BTreeSet<Url>,
}
fn merged(mut spans: Vec<Range<usize>>) -> Vec<Range<usize>> {
    spans.sort_unstable_by_key(|span| span.start);
    let mut result: Vec<Range<usize>> = Vec::new();
    for span in spans {
        if let Some(last) = result.last_mut().filter(|last| span.start <= last.end) {
            last.end = last.end.max(span.end);
        } else {
            result.push(span);
        }
    }
    result
}
fn contains(spans: &[Range<usize>], at: usize) -> bool {
    spans
        .partition_point(|span| span.start <= at)
        .checked_sub(1)
        .is_some_and(|index| spans[index].contains(&at))
}
/// Source facts for readers that do not inspect executable operand identities.
pub(crate) struct LiteralContext {
    pub(crate) declarations: BTreeSet<(usize, usize)>,
    literal: Vec<Range<usize>>,
}
impl LiteralContext {
    pub(crate) fn new(doc: &Document) -> Self {
        let declarations = doc
            .syntax
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        let literal = merged(
            doc.syntax
                .ignored
                .iter()
                .chain(&doc.syntax.literal_conversions)
                .chain(&doc.syntax.literal_tests)
                .cloned()
                .collect(),
        );
        Self {
            declarations,
            literal,
        }
    }
    pub(crate) fn literal(&self, at: usize) -> bool {
        contains(&self.literal, at)
    }
}

pub(crate) struct Context {
    operands: BTreeSet<usize>,
    pub(crate) declarations: BTreeSet<(usize, usize)>,
    declaration_data: Vec<Range<usize>>,
    bodies: Vec<Range<usize>>,
    opaque: Vec<Range<usize>>,
    literal: Vec<Range<usize>>,
}
impl Context {
    pub(crate) fn new(doc: &Document, signatures: &CommandSignatures<'_>) -> Self {
        let source = &doc.text.source;
        let operands = signatures.editing_argument_tokens(source, &doc.syntax);
        let LiteralContext {
            declarations,
            literal,
        } = LiteralContext::new(doc);
        let declaration_data = merged(crate::symbols::expl3::math_data_spans(doc));
        let bodies = merged(crate::symbols::expl3::occurrences(doc).bodies);
        let mut opaque = literal.clone();
        opaque.extend(doc.syntax.ambiguous_string_tests.iter().cloned());
        let opaque = merged(opaque);
        Self {
            operands,
            declarations,
            declaration_data,
            bodies,
            opaque,
            literal,
        }
    }
    pub(crate) fn literal(&self, at: usize) -> bool {
        contains(&self.literal, at)
    }
    pub(crate) fn operand(&self, doc: &Document, at: usize) -> bool {
        self.operand_with(doc, at, &self.operands)
    }
    pub(crate) fn operand_with(
        &self,
        doc: &Document,
        at: usize,
        operands: &BTreeSet<usize>,
    ) -> bool {
        operands.contains(&at)
            || doc.syntax.control_operands.contains(&at)
            || contains(&self.opaque, at)
            || contains(&self.declaration_data, at) && !contains(&self.bodies, at)
    }
}

type Potential = BTreeMap<String, BTreeSet<String>>;
type Uncertain = Vec<(bool, String)>;
fn calls(
    doc: &Document,
    signatures: &CommandSignatures<'_>,
    potential: &Potential,
) -> Option<(Calls, Uncertain)> {
    let source = &doc.text.source;
    let context = Context::new(doc, signatures);
    let declaration_operands = (!doc.syntax.labels.is_empty())
        .then(|| signatures.declaration_argument_tokens(source, &doc.syntax));
    let empty = BTreeSet::new();
    let mut result = BTreeMap::new();
    let mut uncertain = Vec::new();
    for (index, lexical) in doc.syntax.commands.iter().enumerate() {
        if context.literal(lexical.span.start) {
            continue;
        }
        let command = reference_calls::named_command(
            source,
            &doc.syntax,
            index,
            signatures,
            &empty,
            &context.declarations,
        )
        .or_else(|| direct_reference(&lexical.name).then(|| lexical.clone()))
        .filter(|command| signatures.command_unchanged(&command.name));
        let Some(command) = command else {
            let (name, span) = signatures
                .named_invocation(source, &doc.syntax, index, &empty, &context.declarations)
                .unwrap_or((&lexical.name, lexical.span.clone()));
            for possible in potential.get(name).into_iter().flatten() {
                let command = reference_calls::resolved_command(source, possible, span.clone());
                for (citation, key) in reference_calls::keys(source, &command, false) {
                    if result.len() + uncertain.len() >= 20_000 {
                        return None;
                    }
                    uncertain.push((citation, source[key].to_owned()));
                }
            }
            continue;
        };
        let at = lexical.span.start;
        let operand = context.operand(doc, at);
        for (citation, span) in reference_calls::keys(source, &command, false) {
            if result.len() + uncertain.len() >= 20_000 {
                return None;
            }
            result.insert(
                span.start,
                Key {
                    citation,
                    name: source[span.clone()].into(),
                    span,
                    operand,
                },
            );
        }
    }
    // Lexical declarations may be displayed or discarded as another macro's
    // argument. They still appear in the structural declaration index, but
    // cannot justify rewriting the displayed key. Definition bodies retain
    // their existing deferred editing semantics through Context::operand.
    for entry in &doc.syntax.labels {
        let before = doc
            .syntax
            .commands
            .partition_point(|command| command.span.end <= entry.span.start);
        let command = before
            .checked_sub(1)
            .and_then(|index| doc.syntax.commands.get(index));
        let proven = command.is_some_and(|command| {
            let argument = if command.name == "label" {
                command.argument.clone()
            } else if command.name == "hyperdef" {
                syntax::separate_label_argument(source, command)
            } else {
                None
            };
            argument.is_some_and(|argument| {
                argument.span.start <= entry.span.start && entry.span.end <= argument.span.end
            }) && signatures.command_unchanged(&command.name)
                && !declaration_operands
                    .as_ref()
                    .is_some_and(|operands| context.operand_with(doc, command.span.start, operands))
        });
        if !proven {
            if result.len() + uncertain.len() >= 20_000 {
                return None;
            }
            uncertain.push((false, entry.name.clone()));
        }
    }
    Some((result, uncertain))
}
fn direct_reference(name: &str) -> bool {
    syntax::is_reference(name)
        || syntax::has_separate_label_argument(name)
        || syntax::is_citation(name)
}
// Track only paths containing a captured copy. Plain forwarding paths keep
// their existing confirmed semantics. Captures may predate a redefinition and
// therefore remain possible references even when the source is now ordinary.
fn copy_candidates(docs: &[&Document], work: &mut usize) -> Option<Potential> {
    let mut users = BTreeMap::<String, Vec<(String, bool)>>::new();
    let mut has_copy = false;
    for doc in docs {
        for entry in &doc.syntax.definitions {
            let edge = crate::symbols::commands::literal_copy_target(
                &doc.text.source,
                &doc.syntax,
                entry,
                false,
            )
            .map(|(target, _)| (target, true))
            .or_else(|| {
                crate::symbols::commands::source_alias(&doc.text.source, &doc.syntax, entry)
                    .map(|target| (target, false))
            });
            if let Some((target, copied)) = edge {
                has_copy |= copied;
                users
                    .entry(target)
                    .or_default()
                    .push((entry.name.clone(), copied));
            }
        }
    }
    let mut result = Potential::new();
    if !has_copy {
        return Some(result);
    }
    for builtin in users.keys().filter(|name| direct_reference(name)) {
        let mut pending = vec![(builtin.clone(), false)];
        let mut seen = BTreeSet::new();
        while let Some((name, copied)) = pending.pop() {
            *work = work.saturating_add(1);
            if *work > 100_000 {
                return None;
            }
            if !seen.insert((name.clone(), copied)) {
                continue;
            }
            if copied {
                result
                    .entry(name.clone())
                    .or_default()
                    .insert(builtin.clone());
            }
            pending.extend(
                users
                    .get(&name)
                    .into_iter()
                    .flatten()
                    .map(|(name, edge)| (name.clone(), copied || *edge)),
            );
        }
    }
    Some(result)
}
fn candidate_names(workspace: &Workspace) -> BTreeSet<&str> {
    workspace
        .documents
        .values()
        .flat_map(|doc| {
            doc.syntax
                .definitions
                .iter()
                .chain(&doc.syntax.environments)
        })
        .map(|entry| entry.name.as_str())
        .collect()
}
fn has_calls(doc: &Document, names: &BTreeSet<&str>) -> bool {
    doc.kind == DocumentKind::Latex
        && (!doc.syntax.labels.is_empty()
            || doc.syntax.commands.iter().any(|command| {
                matches!(command.name.as_str(), "csuse" | "csname")
                    || direct_reference(&command.name)
                    || names.contains(command.name.as_str())
            }))
}

impl Index {
    pub(crate) fn new(workspace: &Workspace, origin: &Url) -> std::sync::Arc<Self> {
        super::query_cache::named(workspace, origin, || Self::build(workspace, origin, None))
    }

    pub(super) fn cache_bytes(&self) -> usize {
        self.certain
            .iter()
            .map(|(uri, calls)| {
                uri.as_str().len()
                    + 64
                    + calls
                        .values()
                        .map(|key| std::mem::size_of::<Key>() + key.name.capacity() + 64)
                        .sum::<usize>()
            })
            .sum::<usize>()
            + self.ambiguous.capacity() * std::mem::size_of::<(Url, bool, String)>()
            + self
                .ambiguous
                .iter()
                .map(|(uri, _, name)| uri.as_str().len() + name.capacity())
                .sum::<usize>()
            + self
                .section_resources
                .iter()
                .map(|uri| uri.as_str().len() + 64)
                .sum::<usize>()
    }

    /// Lexical evidence only: callers must independently prove every key's
    /// resource identity before returning references or edits.
    pub(crate) fn for_section_keys(
        workspace: &Workspace,
        origin: &Url,
        contexts: BTreeMap<BTreeSet<Url>, BTreeSet<Url>>,
    ) -> Self {
        Self::build(workspace, origin, Some(contexts))
    }

    fn build(
        workspace: &Workspace,
        origin: &Url,
        contexts: Option<BTreeMap<BTreeSet<Url>, BTreeSet<Url>>>,
    ) -> Self {
        let section_keys = contexts.is_some();

        if !workspace.index_truncated.is_empty() {
            return Self {
                incomplete: true,
                ..Default::default()
            };
        }
        let candidates = candidate_names(workspace);
        if !workspace
            .documents
            .values()
            .any(|doc| has_calls(doc, &candidates))
        {
            return Self::default();
        }
        let contexts = contexts
            .unwrap_or_else(|| crate::symbols::environment_calls::project_contexts(workspace));
        let mut connected = workspace.symbol_scope(origin);
        loop {
            let before = connected.len();
            let relevant: Vec<_> = contexts
                .keys()
                .filter(|scope| !scope.is_disjoint(&connected))
                .collect();
            for scope in relevant {
                connected.extend(scope.iter().cloned());
            }
            if before == connected.len() {
                break;
            }
        }
        let mut interpretations = BTreeMap::<Url, Vec<Calls>>::new();
        let mut bytes = 0usize;
        let mut commands = 0usize;
        let mut definitions = 0usize;
        let mut keys = 0usize;
        let mut copy_work = 0usize;
        let mut uncertain = Vec::new();
        let mut section_resources = BTreeSet::new();
        for (scope, consumers) in contexts
            .into_iter()
            .filter(|(scope, _)| !scope.is_disjoint(&connected))
        {
            let docs: Vec<_> = scope
                .iter()
                .filter_map(|uri| workspace.documents.get(uri))
                .filter(|doc| doc.kind == DocumentKind::Latex)
                .collect();
            bytes =
                bytes.saturating_add(docs.iter().map(|doc| doc.text.source.len()).sum::<usize>());
            commands = commands.saturating_add(
                docs.iter()
                    .map(|doc| doc.syntax.commands.len())
                    .sum::<usize>(),
            );
            definitions = definitions.saturating_add(
                docs.iter()
                    .map(|doc| doc.syntax.definitions.len() + doc.syntax.environments.len())
                    .sum::<usize>(),
            );
            if bytes > 32 * 1024 * 1024 || commands > 200_000 || definitions > 8192 {
                return Self {
                    incomplete: true,
                    ..Default::default()
                };
            }
            let signatures = super::query_cache::project_signatures(workspace, &scope);
            let Some(potential) = copy_candidates(&docs, &mut copy_work) else {
                return Self {
                    incomplete: true,
                    ..Default::default()
                };
            };
            let selected_resources =
                crate::bibliography::bib_sections::selects_resources(&docs, &signatures);
            if selected_resources && !section_keys {
                section_resources.extend(consumers.iter().cloned());
            }
            for uri in consumers {
                let Some(doc) = workspace
                    .documents
                    .get(&uri)
                    .filter(|doc| has_calls(doc, &candidates))
                else {
                    continue;
                };
                let Some((mut calls, possible)) = calls(doc, &signatures, &potential) else {
                    return Self {
                        incomplete: true,
                        ..Default::default()
                    };
                };
                if selected_resources && !section_keys {
                    for key in calls.values_mut().filter(|key| key.citation) {
                        key.operand = true;
                    }
                }
                keys = keys.saturating_add(calls.len() + possible.len());
                if keys > 100_000 {
                    return Self {
                        incomplete: true,
                        ..Default::default()
                    };
                }
                uncertain.extend(
                    possible
                        .into_iter()
                        .map(|(citation, name)| (uri.clone(), citation, name)),
                );
                interpretations.entry(uri).or_default().push(calls);
            }
        }
        let mut result = Self {
            ambiguous: uncertain,
            section_resources,
            ..Default::default()
        };
        for (uri, interpretations) in interpretations {
            let starts: BTreeSet<_> = interpretations
                .iter()
                .flat_map(|calls| calls.keys().copied())
                .collect();
            for start in starts {
                let candidates: Vec<_> = interpretations
                    .iter()
                    .filter_map(|calls| calls.get(&start))
                    .collect();
                let first = candidates[0];
                if candidates.len() == interpretations.len()
                    && candidates.iter().all(|key| !key.operand && *key == first)
                {
                    result
                        .certain
                        .entry(uri.clone())
                        .or_default()
                        .insert(start, first.clone());
                } else {
                    result.ambiguous.extend(
                        candidates
                            .into_iter()
                            .map(|key| (uri.clone(), key.citation, key.name.clone())),
                    );
                }
            }
        }
        result
    }
    pub(crate) fn target(&self, uri: &Url, offset: usize) -> Option<(bool, Range<usize>)> {
        self.certain
            .get(uri)?
            .values()
            .find(|key| key.span.start <= offset && offset <= key.span.end)
            .map(|key| (key.citation, key.span.clone()))
    }
    pub(crate) fn references<'a>(
        &'a self,
        uri: &Url,
        citation: bool,
        name: &'a str,
    ) -> impl Iterator<Item = Range<usize>> + 'a {
        self.certain
            .get(uri)
            .into_iter()
            .flat_map(|calls| calls.values())
            .filter(move |key| key.citation == citation && key.name == name)
            .map(|key| key.span.clone())
    }
    pub(crate) fn validate(
        &self,
        scope: &BTreeSet<Url>,
        citation: bool,
        name: &str,
    ) -> jsonrpc::Result<()> {
        if citation && !scope.is_disjoint(&self.section_resources) {
            return Err(jsonrpc::Error::invalid_params("Bibliography resources differ between reference sections; a complete citation rename cannot be confirmed"));
        }
        if self.incomplete {
            return Err(jsonrpc::Error::invalid_params("Reference analysis is incomplete or exceeds its budget; reindex or narrow the project before renaming"));
        }
        if self
            .ambiguous
            .iter()
            .any(|(uri, kind, key)| scope.contains(uri) && *kind == citation && key == name)
        {
            return Err(jsonrpc::Error::invalid_params("Rename is ambiguous: a reference is used as data, has an unconfirmed captured meaning, or has different meanings in its projects"));
        }
        Ok(())
    }
}
pub(crate) fn may_contain_key(workspace: &Workspace, doc: &Document, offset: usize) -> bool {
    // Literal reference keys cannot overlap a control sequence's own name.
    // Keep the end boundary available for an adjacent single-token argument.
    if doc
        .syntax
        .commands
        .iter()
        .any(|command| command.span.start <= offset && offset < command.span.end)
    {
        return false;
    }
    let names = candidate_names(workspace);
    doc.syntax.commands.iter().any(|command| {
        (matches!(command.name.as_str(), "csuse" | "csname")
            || names.contains(command.name.as_str()))
            && command.span.start < offset
    })
}

pub(crate) fn completion_context(
    workspace: &Workspace,
    uri: &Url,
    offset: usize,
    prefix: impl Fn(&crate::parser::syntax::Command) -> Option<Range<usize>>,
) -> Option<(crate::parser::syntax::Command, Range<usize>)> {
    let candidates = candidate_names(workspace);
    let doc = workspace
        .documents
        .get(uri)
        .filter(|doc| has_calls(doc, &candidates))?;
    // Typing an ordinary control word cannot be a reference-key prefix. Keep
    // reference/alias boundaries available for adjacent single-token keys.
    if doc.syntax.commands.iter().any(|command| {
        command.span.start < offset
            && offset <= command.span.end
            && !direct_reference(&command.name)
            && !candidates.contains(command.name.as_str())
            && !matches!(command.name.as_str(), "csuse" | "csname" | "endcsname")
    }) {
        return None;
    }
    if !workspace.index_truncated.is_empty() {
        return None;
    }
    let mut selected: Option<(crate::parser::syntax::Command, Range<usize>)> = None;
    let mut bytes = 0usize;
    let mut commands = 0usize;
    let mut definitions = 0usize;
    for (scope, consumers) in crate::symbols::environment_calls::project_contexts(workspace) {
        if !consumers.contains(uri) {
            continue;
        }
        let docs: Vec<_> = scope
            .iter()
            .filter_map(|uri| workspace.documents.get(uri))
            .filter(|doc| doc.kind == DocumentKind::Latex)
            .collect();
        bytes = bytes.saturating_add(docs.iter().map(|doc| doc.text.source.len()).sum::<usize>());
        commands = commands.saturating_add(
            docs.iter()
                .map(|doc| doc.syntax.commands.len())
                .sum::<usize>(),
        );
        definitions = definitions.saturating_add(
            docs.iter()
                .map(|doc| doc.syntax.definitions.len() + doc.syntax.environments.len())
                .sum::<usize>(),
        );
        if bytes > 32 * 1024 * 1024 || commands > 200_000 || definitions > 8192 {
            return None;
        }
        let signatures = CommandSignatures::new(
            docs.iter()
                .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
        );
        let selected_resources =
            crate::bibliography::bib_sections::selects_resources(&docs, &signatures);
        let context = Context::new(doc, &signatures);
        let candidate = doc
            .syntax
            .commands
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, command)| command.span.end <= offset)
            .find_map(|(index, lexical)| {
                if context.operand(doc, lexical.span.start) {
                    return None;
                }
                let command = reference_calls::named_command(
                    &doc.text.source,
                    &doc.syntax,
                    index,
                    &signatures,
                    &context.operands,
                    &context.declarations,
                )
                .or_else(|| direct_reference(&lexical.name).then(|| lexical.clone()))?;
                if !signatures.command_unchanged(&command.name)
                    || selected_resources && syntax::is_citation(&command.name)
                {
                    return None;
                }
                let span = prefix(&command)?;
                Some((command, span))
            })?;
        if selected.as_ref().is_some_and(|(command, span)| {
            command.name != candidate.0.name
                || command.span != candidate.0.span
                || *span != candidate.1
        }) {
            return None;
        }
        selected = Some(candidate);
    }
    selected
}
