//! Possible flow of source parameters into literal hook data.
use crate::{
    language::signature::{self, CommandShape, CommandSignatures},
    parser::syntax,
    project::workspace::{Document, DocumentKind, Workspace},
    symbols::named_references::LiteralContext,
};
use lsp_types::Url;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

pub(super) type Data = BTreeMap<Url, Vec<Range<usize>>>;
pub(super) type Native = BTreeMap<Url, Vec<(Range<usize>, usize)>>;
type Needs = BTreeMap<String, BTreeSet<(u16, usize)>>;

fn item(source: &str, template: &str, after: usize) -> Option<(usize, usize, u16)> {
    let at = syntax::skip_space(source, after);
    if source.as_bytes().get(at) == Some(&b'{') {
        return syntax::group(source, at, b'{', b'}').map(|(_, end)| (at, end, 0));
    }
    if source.as_bytes().get(at) == Some(&b'#')
        && template.as_bytes().get(at) == Some(&b'.')
        && source
            .as_bytes()
            .get(at + 1)
            .is_some_and(|b| (b'1'..=b'9').contains(b))
    {
        return Some((at, at + 2, 1 << (source.as_bytes()[at + 1] - b'1')));
    }
    syntax::required_token(source, at).map(|value| (at, at + value.len(), 0))
}

fn normalise_native(
    owners: &[Owner<'_>],
    native: Native,
    templates: &BTreeMap<Url, String>,
    work: &mut usize,
) -> Option<(Data, Needs)> {
    let mut data = Data::new();
    let mut needs = Needs::new();
    for (uri, spans) in native {
        for (span, arity) in spans {
            charge(work, owners.len().saturating_mul(8))?;
            let owner = owners
                .iter()
                .filter(|owner| {
                    *owner.uri == uri
                        && owner.body.start <= span.start
                        && span.start <= owner.body.end
                })
                .min_by_key(|owner| owner.body.len());
            let Some(owner) = owner else {
                data.entry(uri.clone()).or_default().push(span);
                continue;
            };
            let source = &owner.doc.text.source;
            let template = templates.get(&uri).map_or(source.as_str(), String::as_str);
            let mut after = span.start;
            let mut deficit = 0;
            let mut mask = 0;
            for _ in 0..arity {
                charge(work, 64)?;
                if syntax::skip_space(source, after) >= owner.body.end {
                    deficit += 1;
                    continue;
                }
                let (at, end, bit) = item(source, template, after)?;
                if at >= owner.body.end || bit != 0 {
                    deficit += 1;
                }
                mask |= bit;
                after = end;
            }
            after = after.min(owner.body.end);
            while deficit > 0 && syntax::skip_space(source, after) < owner.body.end {
                charge(work, 64)?;
                let (_, end, bit) = item(source, template, after)?;
                mask |= bit;
                if bit == 0 {
                    deficit -= 1;
                }
                after = end.min(owner.body.end);
            }
            data.entry(uri.clone()).or_default().push(span.start..after);
            if deficit > 0 {
                needs
                    .entry(owner.entry.name.clone())
                    .or_default()
                    .insert((mask, deficit.min(9)));
            }
        }
    }
    Some((data, needs))
}

fn contents(source: &str, span: Range<usize>) -> Range<usize> {
    let at = syntax::skip_space(source, span.start);
    let bracket = source.as_bytes().get(at) == Some(&b'[');
    let group = if bracket {
        syntax::bracket_argument(source, at)
    } else {
        syntax::group(source, at, b'{', b'}')
    };
    group
        .filter(|(_, end)| *end == span.end)
        .map_or(span, |(argument, _)| argument.span)
}

fn input_bound(
    source: &str,
    template: &str,
    span: Range<usize>,
    work: &mut usize,
) -> Option<(usize, u16)> {
    let span = contents(source, span);
    let mut at = span.start;
    let mut count = 0;
    let mut mask = 0;
    while syntax::skip_space(source, at) < span.end {
        charge(work, 64)?;
        let (_, end, bit) = item(source, template, at)?;
        if bit == 0 {
            count = (count + 1).min(9);
        }
        mask |= bit;
        at = end;
    }
    Some((count, mask))
}

fn charge(work: &mut usize, bytes: usize) -> Option<()> {
    *work = work.saturating_add(bytes);
    (*work <= 32 * 1024 * 1024).then_some(())
}

fn parameters(source: &str, span: Range<usize>) -> Vec<(usize, u16)> {
    let mut at = span.start;
    let mut result = Vec::new();
    while at < span.end {
        match source.as_bytes()[at] {
            b'%' => {
                at = source[at..]
                    .find('\n')
                    .map_or(source.len(), |len| at + len + 1)
            }
            b'\\' => at += syntax::token(source, at).map_or(1, str::len),
            b'#' if source
                .as_bytes()
                .get(at + 1)
                .is_some_and(|byte| (b'1'..=b'9').contains(byte)) =>
            {
                result.push((at, 1 << (source.as_bytes()[at + 1] - b'1')));
                at += 2;
            }
            _ => at += source[at..].chars().next().unwrap().len_utf8(),
        }
    }
    result
}

/// A template placeholder represents its substituted token list, rather than
/// two separate character arguments. Keep byte coordinates and word boundaries.
pub(super) fn template(doc: &Document, work: &mut usize) -> Option<Option<String>> {
    if !doc.text.source.contains('#') {
        return Some(None);
    }
    let mut slots = BTreeSet::new();
    for entry in &doc.syntax.definitions {
        if let Some(body) = signature::command_definition_body(&doc.text.source, &doc.syntax, entry)
        {
            charge(work, body.span.len())?;
            slots.extend(
                parameters(&doc.text.source, body.span)
                    .into_iter()
                    .map(|(at, _)| at),
            );
        }
    }
    if slots.is_empty() {
        return Some(None);
    }
    let mut source = doc.text.source.as_bytes().to_vec();
    for at in slots {
        source[at] = b'.';
        source[at + 1] = b' ';
    }
    Some(Some(
        String::from_utf8(source).expect("ASCII placeholder substitution preserves UTF-8"),
    ))
}

struct Owner<'a> {
    uri: &'a Url,
    doc: &'a Document,
    entry: &'a syntax::Entry,
    body: Range<usize>,
    header: Range<usize>,
    parameters: Vec<(usize, u16)>,
    defaults: Option<Vec<(usize, Range<usize>)>>,
}

fn derive(
    owners: &[Owner<'_>],
    data: &Data,
    readers: &mut BTreeMap<String, u16>,
    work: &mut usize,
) -> Option<bool> {
    let mut changed = false;
    for owner in owners {
        let mut bits = 0;
        for span in data.get(owner.uri).into_iter().flatten() {
            charge(work, 64)?;
            if span.start >= owner.body.end || span.end <= owner.body.start {
                continue;
            }
            for &(at, bit) in &owner.parameters {
                charge(work, 8)?;
                if span.contains(&at) {
                    bits |= bit;
                }
            }
        }
        if bits != 0 {
            let entry = readers.entry(owner.entry.name.clone()).or_default();
            changed |= *entry | bits != *entry;
            *entry |= bits;
        }
    }
    Some(changed)
}

pub(super) fn extend(
    workspace: &Workspace,
    project: &BTreeSet<Url>,
    signatures: &CommandSignatures<'_>,
    native: Native,
    templates: &BTreeMap<Url, String>,
    work: &mut usize,
) -> Option<Data> {
    let docs: Vec<_> = project
        .iter()
        .filter_map(|uri| workspace.documents.get(uri).map(|doc| (uri, doc)))
        .filter(|(_, doc)| doc.kind == DocumentKind::Latex)
        .collect();
    let mut owners = Vec::new();
    let mut aliases = Vec::new();
    for &(uri, doc) in &docs {
        for entry in &doc.syntax.definitions {
            charge(work, 64)?;
            if let Some(target) = signature::source_alias(&doc.text.source, &doc.syntax, entry)
                .or_else(|| {
                    signature::literal_copy_target(&doc.text.source, &doc.syntax, entry, false)
                        .map(|(target, _)| target)
                })
            {
                aliases.push((entry.name.clone(), target));
            }
            if let Some(body) =
                signature::command_definition_body(&doc.text.source, &doc.syntax, entry)
            {
                charge(work, body.span.len())?;
                let parameters = parameters(&doc.text.source, body.span.clone());
                owners.push(Owner {
                    uri,
                    doc,
                    entry,
                    header: entry.span.end..body.outer_span.start,
                    body: body.span,
                    parameters,
                    defaults: signature::command_parameter_defaults(
                        &doc.text.source,
                        &doc.syntax,
                        entry,
                    ),
                });
            }
        }
    }
    let (native, mut needs) = normalise_native(&owners, native, templates, work)?;
    let mut readers = BTreeMap::<String, u16>::new();
    for name in needs.keys() {
        readers.insert(name.clone(), 0);
    }
    derive(&owners, &native, &mut readers, work)?;
    if readers.is_empty() {
        return Some(native);
    }
    let contexts: BTreeMap<_, _> = docs
        .iter()
        .map(|&(uri, doc)| (uri, LiteralContext::new(doc)))
        .collect();
    let empty = BTreeSet::new();
    let alias_bodies: BTreeSet<_> = owners
        .iter()
        .filter(|owner| {
            signature::source_alias(&owner.doc.text.source, &owner.doc.syntax, owner.entry)
                .is_some()
        })
        .map(|owner| {
            (
                owner.uri.clone(),
                syntax::skip_space(&owner.doc.text.source, owner.body.start),
            )
        })
        .collect();
    loop {
        let mut changed = false;
        for (alias, target) in &aliases {
            charge(work, 64)?;
            if let Some(&bits) = readers.get(target) {
                let entry = readers.entry(alias.clone()).or_default();
                changed |= *entry | bits != *entry;
                *entry |= bits;
            }
            if let Some(models) = needs.get(target).cloned() {
                for model in models {
                    changed |= needs.entry(alias.clone()).or_default().insert(model);
                }
            }
        }
        let mut data = native.clone();
        for owner in &owners {
            if let Some(&bits) = readers.get(&owner.entry.name) {
                match &owner.defaults {
                    Some(defaults) => {
                        for (slot, span) in defaults {
                            if bits & (1 << slot) != 0 {
                                data.entry(owner.uri.clone())
                                    .or_default()
                                    .push(span.clone());
                            }
                        }
                    }
                    None => data
                        .entry(owner.uri.clone())
                        .or_default()
                        .push(owner.header.clone()),
                }
            }
        }
        for &(uri, doc) in &docs {
            let context = &contexts[uri];
            let source = templates
                .get(uri)
                .map_or(doc.text.source.as_str(), String::as_str);
            for (index, command) in doc.syntax.commands.iter().enumerate() {
                charge(work, 64)?;
                // Only these lexical names can resolve to a current reader.
                // Keep static named calls until their target has been read.
                if !readers.contains_key(&command.name)
                    && !matches!(command.name.as_str(), "csuse" | "csname")
                {
                    continue;
                }
                if context.literal(command.span.start)
                    || alias_bodies.contains(&(uri.clone(), command.span.start))
                    || doc.syntax.control_operands.contains(&command.span.start)
                    || context
                        .declarations
                        .contains(&(command.span.start, command.span.end))
                {
                    continue;
                }
                let (name, invocation) = signatures
                    .named_invocation(
                        &doc.text.source,
                        &doc.syntax,
                        index,
                        &empty,
                        &context.declarations,
                    )
                    .unwrap_or((&command.name, command.span.clone()));
                let Some(&bits) = readers.get(name) else {
                    continue;
                };
                let spaces = !(command.name == "csuse" && invocation.end > command.span.end)
                    && syntax::control_word(source, command.span.start).is_some();
                let mut shapes = signatures
                    .parameter_spans(source, &doc.syntax, name, invocation.clone(), spaces)
                    .into_iter()
                    .collect::<Vec<_>>();
                if shapes.is_empty() {
                    // Copies may retain a formerly defined grammar. Inspect
                    // possible source prototypes without certifying expansion.
                    let mut pending = vec![name.to_owned()];
                    let mut seen = BTreeSet::new();
                    while let Some(name) = pending.pop() {
                        charge(work, name.len() + 64)?;
                        if !seen.insert(name.clone()) {
                            continue;
                        }
                        charge(work, owners.len().saturating_mul(8))?;
                        for owner in owners.iter().filter(|owner| owner.entry.name == name) {
                            if let Some(spans) = CommandShape::definition(
                                &owner.doc.text.source,
                                &owner.doc.syntax,
                                owner.entry,
                            )
                            .and_then(|shape| {
                                shape.parameter_spans(
                                    source,
                                    &doc.syntax,
                                    invocation.clone(),
                                    spaces,
                                )
                            }) {
                                shapes.push(spans);
                            }
                        }
                        charge(work, aliases.len().saturating_mul(8))?;
                        pending.extend(
                            aliases
                                .iter()
                                .filter(|(alias, _)| *alias == name)
                                .map(|(_, target)| target.clone()),
                        );
                    }
                }
                if shapes.is_empty() {
                    data.entry(uri.clone())
                        .or_default()
                        .push(invocation.end..source.len());
                } else {
                    for shape in shapes {
                        let spans = shape.spans;
                        charge(work, owners.len().saturating_mul(8))?;
                        let owner = owners
                            .iter()
                            .filter(|owner| {
                                owner.uri == uri && owner.body.contains(&invocation.start)
                            })
                            .min_by_key(|owner| owner.body.len());
                        if owner
                            .is_some_and(|owner| spans.iter().any(|span| span.end > owner.body.end))
                        {
                            return None;
                        }
                        for (slot, span) in spans.iter().enumerate() {
                            if bits & (1 << slot) != 0 {
                                data.entry(uri.clone()).or_default().push(span.clone());
                            }
                        }
                        for (mask, mut debt) in needs.get(name).cloned().into_iter().flatten() {
                            let mut outer_mask = 0;
                            for (slot, span) in spans.iter().enumerate() {
                                if mask & (1 << slot) == 0 {
                                    continue;
                                }
                                let (known, variables) =
                                    input_bound(&doc.text.source, source, span.clone(), work)?;
                                if shape.simple {
                                    debt = debt.saturating_sub(known);
                                }
                                outer_mask |= variables;
                            }
                            if debt == 0 {
                                continue;
                            }
                            let mut after = shape.end;
                            if after > 0
                                && doc.text.source.as_bytes().get(after - 1) == Some(&b'#')
                                && source.as_bytes().get(after - 1) == Some(&b'.')
                            {
                                after += 1;
                            }
                            let start = after;
                            let limit = owner.map_or(source.len(), |owner| owner.body.end);
                            while debt > 0 && syntax::skip_space(&doc.text.source, after) < limit {
                                charge(work, 64)?;
                                let (_, end, bit) = item(&doc.text.source, source, after)?;
                                if end > limit {
                                    return None;
                                }
                                outer_mask |= bit;
                                if bit == 0 {
                                    debt -= 1;
                                }
                                after = end;
                            }
                            data.entry(uri.clone()).or_default().push(start..after);
                            if debt > 0 {
                                if let Some(owner) = owner {
                                    changed |= needs
                                        .entry(owner.entry.name.clone())
                                        .or_default()
                                        .insert((outer_mask, debt));
                                    readers.entry(owner.entry.name.clone()).or_default();
                                }
                            }
                        }
                    }
                }
            }
        }
        changed |= derive(&owners, &data, &mut readers, work)?;
        if !changed {
            return Some(data);
        }
    }
}
