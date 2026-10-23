//! Label/citation keys whose meanings agree in every consuming project.
use crate::{
    reference_calls,
    signature::CommandSignatures,
    syntax,
    workspace::{Document, DocumentKind, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use tower_lsp::{jsonrpc, lsp_types::Url};

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
        let declarations = doc
            .syntax
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        let declaration_data = merged(crate::expl3::math_data_spans(doc));
        let bodies = merged(crate::expl3::occurrences(doc).bodies);
        let literal: Vec<_> = doc
            .syntax
            .ignored
            .iter()
            .chain(&doc.syntax.literal_conversions)
            .chain(&doc.syntax.literal_tests)
            .cloned()
            .collect();
        let literal = merged(literal);
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
        self.operands.contains(&at)
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
            let edge =
                crate::signature::literal_copy_target(&doc.text.source, &doc.syntax, entry, false)
                    .map(|(target, _)| (target, true))
                    .or_else(|| {
                        crate::signature::source_alias(&doc.text.source, &doc.syntax, entry)
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
        && doc.syntax.commands.iter().any(|command| {
            matches!(command.name.as_str(), "csuse" | "csname")
                || direct_reference(&command.name)
                || names.contains(command.name.as_str())
        })
}

impl Index {
    pub(crate) fn new(workspace: &Workspace, origin: &Url) -> Self {
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
        let contexts = crate::environment_calls::project_contexts(workspace);
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
            let signatures = CommandSignatures::new(
                docs.iter()
                    .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
            );
            let Some(potential) = copy_candidates(&docs, &mut copy_work) else {
                return Self {
                    incomplete: true,
                    ..Default::default()
                };
            };
            let selected_resources = crate::bib_sections::selects_resources(&docs, &signatures);
            if selected_resources {
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
                if selected_resources {
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
    prefix: impl Fn(&crate::syntax::Command) -> Option<Range<usize>>,
) -> Option<(crate::syntax::Command, Range<usize>)> {
    let candidates = candidate_names(workspace);
    let doc = workspace
        .documents
        .get(uri)
        .filter(|doc| has_calls(doc, &candidates))?;
    if !workspace.index_truncated.is_empty() {
        return None;
    }
    let mut selected: Option<(crate::syntax::Command, Range<usize>)> = None;
    let mut bytes = 0usize;
    let mut commands = 0usize;
    let mut definitions = 0usize;
    for (scope, consumers) in crate::environment_calls::project_contexts(workspace) {
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
        let selected_resources = crate::bib_sections::selects_resources(&docs, &signatures);
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
