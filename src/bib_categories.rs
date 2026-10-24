//! Project-consistent biblatex category names, independent of bibliography keys.
use crate::{
    signature::CommandSignatures,
    syntax,
    workspace::{Document, DocumentKind, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range as Span,
};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::*,
};

const COMMANDS: &[&str] = &[
    "DeclareBibliographyCategory",
    "addtocategory",
    "assignrefcontextcats",
    "printbibliography",
    "ifentrycategory",
    "ifcategory",
    "defbibfilter",
];
fn valid(name: &str) -> bool {
    syntax::is_static_key(name) && !name.contains(['[', ']'])
}
#[derive(Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    span: Span<usize>,
    declaration: bool,
    complete: bool,
}
#[derive(Clone)]
struct Field {
    span: Span<usize>,
    complete: bool,
}
fn group(source: &str, at: usize, open: u8) -> Option<(Field, usize)> {
    let at = syntax::skip_space(source, at);
    if source.as_bytes().get(at) != Some(&open) {
        return None;
    }
    let parsed = if open == b'[' {
        syntax::bracket_argument(source, at)
    } else {
        syntax::group(source, at, b'{', b'}')
    };
    Some(parsed.map_or_else(
        || {
            (
                Field {
                    span: at + 1..source.len(),
                    complete: false,
                },
                source.len(),
            )
        },
        |(arg, end)| {
            (
                Field {
                    span: arg.span,
                    complete: true,
                },
                end,
            )
        },
    ))
}
fn options(source: &str, field: Field) -> (Vec<Field>, bool) {
    let mut fields = vec![];
    let mut dynamic = false;
    for mut segment in syntax::key_list_segments(source, field.span) {
        segment.start = syntax::skip_space(&source[..segment.end], segment.start);
        let text = &source[segment.clone()];
        let Some(equal) = text.find('=') else {
            dynamic |= text.contains('\\');
            continue;
        };
        let key = text[..equal].trim();
        if !matches!(key, "category" | "notcategory") {
            dynamic |= key.contains(['\\', '%']);
            continue;
        }
        let start = syntax::skip_space(&source[..segment.end], segment.start + equal + 1);
        let mut span = syntax::trimmed_span(source, start..segment.end);
        if let Some((arg, end)) = syntax::group(source, span.start, b'{', b'}') {
            if end <= span.end && syntax::skip_space(&source[..span.end], end) == span.end {
                span = arg.span;
            }
        } else if source.as_bytes().get(span.start) == Some(&b'{') && !field.complete {
            span.start += 1;
        }
        if field.complete {
            let names = syntax::key_list_spans(source, span.clone());
            if names.len() == 1 {
                span = names[0].clone();
            }
        }
        dynamic |= !field.complete || !valid(&source[span.clone()]);
        fields.push(Field {
            span,
            complete: field.complete,
        });
    }
    (fields, dynamic)
}
// Parse the documented boolean filter grammar. Values remain literal tokens;
// unsupported expressions are never grounds for returning a partial rename.
fn filter(source: &str, field: Field) -> (Vec<Field>, bool) {
    let mut tokens = vec![];
    let mut at = field.span.start;
    while at < field.span.end {
        if tokens.len() >= 100_000 {
            return (vec![], true);
        }
        let ch = source[at..].chars().next().unwrap();
        if ch.is_whitespace() {
            at += ch.len_utf8();
            continue;
        }
        if matches!(ch, '(' | ')' | '=') {
            tokens.push(at..at + 1);
            at += 1;
            continue;
        }
        let start = at;
        while at < field.span.end {
            let ch = source[at..].chars().next().unwrap();
            if ch.is_whitespace() || matches!(ch, '(' | ')' | '=') {
                break;
            }
            at += ch.len_utf8();
        }
        tokens.push(start..at);
    }
    let mut result = vec![];
    let mut term = true;
    let mut depth = 0usize;
    let mut index = 0;
    while index < tokens.len() {
        let token = &source[tokens[index].clone()];
        if term {
            if token == "not" {
                index += 1;
                continue;
            }
            if token == "(" {
                depth += 1;
                index += 1;
                continue;
            }
            let Some(equal) = tokens.get(index + 1) else {
                return (result, true);
            };
            let Some(value) = tokens.get(index + 2) else {
                return (result, true);
            };
            if !matches!(
                token,
                "type" | "subtype" | "keyword" | "category" | "segment" | "section"
            ) || &source[equal.clone()] != "="
                || !valid(&source[value.clone()])
            {
                return (result, true);
            }
            if token == "category" {
                result.push(Field {
                    span: value.clone(),
                    complete: field.complete,
                });
            }
            index += 3;
            term = false;
        } else if matches!(token, "and" | "or") {
            term = true;
            index += 1;
        } else if token == ")" && depth > 0 {
            depth -= 1;
            index += 1;
        } else {
            return (result, true);
        }
    }
    (result, !field.complete || term || depth != 0)
}
fn fields(source: &str, name: &str, end: usize) -> (Vec<Field>, bool) {
    if name == "assignrefcontextcats" {
        let mut at = syntax::skip_space(source, end);
        if source.as_bytes().get(at) == Some(&b'*') {
            at += 1;
        }
        // Both native forms require [options], including an empty bracket.
        let Some((options, after)) = group(source, at, b'[') else {
            return (vec![], true);
        };
        if !options.complete {
            return (vec![], true);
        }
        let Some((list, _)) = group(source, after, b'{') else {
            return (vec![], true);
        };
        // Bound CSV allocation before building the project-consistent index.
        if source[list.span.clone()]
            .bytes()
            .filter(|byte| *byte == b',')
            .take(20_001)
            .count()
            > 20_000
        {
            return (vec![], true);
        }
        let fields = syntax::key_list_segments(source, list.span)
            .into_iter()
            .map(|span| {
                let at = syntax::skip_space(&source[..span.end], span.start);
                let names = syntax::key_list_spans(source, span.clone());
                Field {
                    span: if names.len() == 1 {
                        names[0].clone()
                    } else {
                        syntax::trimmed_span(source, at..span.end)
                    },
                    complete: list.complete,
                }
            })
            .collect();
        return (fields, !list.complete);
    }
    if name == "printbibliography" {
        return group(source, end, b'[')
            .map_or((vec![], false), |(field, _)| options(source, field));
    }
    let Some((first, after)) = group(source, end, b'{') else {
        return (vec![], true);
    };
    if matches!(name, "ifentrycategory" | "defbibfilter") {
        if !first.complete {
            return (vec![], true);
        }
        let Some((second, _)) = group(source, after, b'{') else {
            return (vec![], true);
        };
        if name == "defbibfilter" {
            return filter(source, second);
        }
        return (vec![second], false);
    }
    (vec![first], false)
}
#[derive(Default)]
struct Calls {
    entries: BTreeMap<usize, (Entry, bool)>,
    dynamic: bool,
}
fn calls(
    doc: &Document,
    signatures: &CommandSignatures<'_>,
    potential: &BTreeMap<String, BTreeSet<String>>,
) -> Calls {
    let source = &doc.text.source;
    let context = crate::named_references::Context::new(doc, signatures);
    let empty = BTreeSet::new();
    let targets: BTreeSet<_> = doc
        .syntax
        .definitions
        .iter()
        .chain(&doc.syntax.environments)
        .flat_map(|entry| [entry.span.start, entry.span.start.saturating_sub(1)])
        .collect();
    let alias_bodies = signatures.builtin_alias_body_positions(source, &doc.syntax, COMMANDS);
    let mut result = Calls::default();
    for (index, lexical) in doc.syntax.commands.iter().enumerate() {
        let at = lexical.span.start;
        if targets.contains(&at) || context.literal(at) {
            continue;
        }
        let (name, span) = signatures
            .named_invocation(source, &doc.syntax, index, &empty, &context.declarations)
            .unwrap_or((&lexical.name, lexical.span.clone()));
        let known = if COMMANDS.contains(&name) {
            signatures.command_unchanged(name).then_some(name)
        } else {
            signatures
                .forwarded_builtin(name)
                .filter(|name| COMMANDS.contains(name))
        };
        let candidates: Vec<_> = if let Some(name) = known {
            vec![(name, false)]
        } else if !COMMANDS.contains(&name) {
            potential
                .get(name)
                .into_iter()
                .flatten()
                .map(|name| (name.as_str(), true))
                .collect()
        } else {
            vec![]
        };
        for (name, uncertain) in candidates {
            let (fields, dynamic) = fields(source, name, span.end);
            if fields.is_empty() && context.operand(doc, at) {
                continue;
            }
            // The primitive in a confirmed forwarding body reads its
            // arguments at the later source invocation.
            if fields.is_empty() && alias_bodies.contains(&at) {
                continue;
            }
            result.dynamic |= dynamic;
            for mut field in fields {
                if field.complete && source[field.span.clone()].contains('%') {
                    let argument = syntax::Argument {
                        value: source[field.span.clone()].into(),
                        span: field.span.clone(),
                        outer_span: field.span.clone(),
                    };
                    field.span = syntax::single_key_argument(source, argument).span;
                }
                if !field.complete {
                    // A partial key ends before the next TeX token or whitespace;
                    // accepting completion must not erase following source code.
                    let length = source[field.span.clone()]
                        .char_indices()
                        .find_map(|(at, ch)| {
                            let mut buffer = [0u8; 4];
                            (!valid(ch.encode_utf8(&mut buffer))).then_some(at)
                        })
                        .unwrap_or(field.span.len());
                    field.span.end = field.span.start + length;
                    if field.span.is_empty() && field.span.start < source.len() {
                        continue;
                    }
                }
                let name_value = &source[field.span.clone()];
                if !valid(name_value)
                    && !(name == "assignrefcontextcats" && field.complete && name_value.is_empty())
                {
                    // Empty and partial names still provide completion context.
                    result.dynamic = true;
                }
                let entry = Entry {
                    name: name_value.into(),
                    span: field.span,
                    declaration: name == "DeclareBibliographyCategory",
                    complete: field.complete,
                };
                let operand = uncertain || context.operand(doc, at);
                result.entries.insert(entry.span.start, (entry, operand));
            }
        }
    }
    result
}
#[derive(Default)]
struct Index {
    certain: BTreeMap<Url, BTreeMap<usize, Entry>>,
    ambiguous: Vec<(Url, String)>,
    dynamic: BTreeSet<Url>,
    incomplete: bool,
}
impl Index {
    fn new(ws: &Workspace, origin: &Url) -> Self {
        if !ws.documents.values().any(|doc| {
            doc.kind == DocumentKind::Latex
                && doc.syntax.commands.iter().any(|cmd| {
                    COMMANDS.contains(&cmd.name.as_str())
                        || cmd.name == "csuse"
                            && cmd
                                .argument
                                .as_ref()
                                .is_some_and(|a| COMMANDS.contains(&a.value.as_str()))
                        || cmd.name == "csname"
                })
        }) {
            return Self::default();
        }
        if !ws.index_truncated.is_empty() {
            return Self {
                incomplete: true,
                ..Self::default()
            };
        }
        let contexts = crate::environment_calls::project_contexts(ws);
        let mut connected = ws.symbol_scope(origin);
        loop {
            let before = connected.len();
            for scope in contexts
                .keys()
                .filter(|scope| !scope.is_disjoint(&connected))
                .cloned()
                .collect::<Vec<_>>()
            {
                connected.extend(scope);
            }
            if before == connected.len() {
                break;
            }
        }
        let mut interpretations = BTreeMap::<Url, Vec<Calls>>::new();
        let (mut bytes, mut commands, mut definitions, mut entries) =
            (0usize, 0usize, 0usize, 0usize);
        for (scope, consumers) in contexts
            .into_iter()
            .filter(|(scope, _)| !scope.is_disjoint(&connected))
        {
            let docs: Vec<_> = scope
                .iter()
                .filter_map(|uri| ws.documents.get(uri))
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
                    ..Self::default()
                };
            }
            let signatures = CommandSignatures::new(
                docs.iter()
                    .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
            );
            let mut users = BTreeMap::<String, BTreeSet<String>>::new();
            for doc in &docs {
                for entry in &doc.syntax.definitions {
                    let target =
                        crate::signature::source_alias(&doc.text.source, &doc.syntax, entry)
                            .or_else(|| {
                                crate::signature::literal_copy_target(
                                    &doc.text.source,
                                    &doc.syntax,
                                    entry,
                                    false,
                                )
                                .map(|(target, _)| target)
                            });
                    if let Some(target) = target {
                        users.entry(target).or_default().insert(entry.name.clone());
                    }
                }
            }
            let mut potential = BTreeMap::<String, BTreeSet<String>>::new();
            for builtin in COMMANDS {
                let mut pending = vec![(*builtin).to_owned()];
                let mut seen = BTreeSet::new();
                while let Some(name) = pending.pop() {
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    potential
                        .entry(name.clone())
                        .or_default()
                        .insert((*builtin).to_owned());
                    pending.extend(users.get(&name).into_iter().flatten().cloned());
                }
            }

            for uri in consumers {
                let Some(doc) = ws
                    .documents
                    .get(&uri)
                    .filter(|doc| doc.kind == DocumentKind::Latex)
                else {
                    continue;
                };
                let calls = calls(doc, &signatures, &potential);
                entries = entries.saturating_add(calls.entries.len());
                if calls.entries.len() > 20_000 || entries > 100_000 {
                    return Self {
                        incomplete: true,
                        ..Self::default()
                    };
                }
                interpretations.entry(uri).or_default().push(calls);
            }
        }
        let mut result = Self::default();
        for (uri, interpretations) in interpretations {
            if interpretations.iter().any(|calls| calls.dynamic) {
                result.dynamic.insert(uri.clone());
            }
            let starts: BTreeSet<_> = interpretations
                .iter()
                .flat_map(|calls| calls.entries.keys().copied())
                .collect();
            for start in starts {
                let candidates: Vec<_> = interpretations
                    .iter()
                    .filter_map(|calls| calls.entries.get(&start))
                    .collect();
                let first = candidates[0];
                if candidates.len() == interpretations.len()
                    && candidates
                        .iter()
                        .all(|candidate| !candidate.1 && candidate.0 == first.0)
                {
                    result
                        .certain
                        .entry(uri.clone())
                        .or_default()
                        .insert(start, first.0.clone());
                } else {
                    result.ambiguous.extend(
                        candidates
                            .into_iter()
                            .map(|(entry, _)| (uri.clone(), entry.name.clone())),
                    );
                }
            }
        }
        result
    }
    fn target(&self, uri: &Url, offset: usize, complete: bool) -> Option<&Entry> {
        self.certain.get(uri)?.values().find(|entry| {
            entry.span.start <= offset
                && offset <= entry.span.end
                && (!complete || entry.complete && valid(&entry.name))
        })
    }
    fn entries<'a>(
        &'a self,
        scope: &'a BTreeSet<Url>,
    ) -> impl Iterator<Item = (&'a Url, &'a Entry)> {
        self.certain
            .iter()
            .filter(move |(uri, _)| scope.contains(*uri))
            .flat_map(|(uri, entries)| entries.values().map(move |entry| (uri, entry)))
            .filter(|(_, entry)| entry.complete && valid(&entry.name))
    }
    fn location(ws: &Workspace, uri: &Url, entry: &Entry) -> Location {
        Location::new(
            uri.clone(),
            ws.documents[uri]
                .text
                .range(entry.span.start, entry.span.end),
        )
    }
    fn definitions(&self, ws: &Workspace, scope: &BTreeSet<Url>, entry: &Entry) -> Vec<Location> {
        self.entries(scope)
            .filter(|(_, other)| other.declaration && other.name == entry.name)
            .map(|(uri, entry)| Self::location(ws, uri, entry))
            .collect()
    }
    fn rename_scope(&self, ws: &Workspace, uri: &Url, entry: &Entry) -> Result<BTreeSet<Url>> {
        if self.incomplete {
            return Err(Error::invalid_params(
                "Category rename requires a complete project index",
            ));
        }
        let crate::workspace::SymbolProjects { mut scope, regions } = ws.symbol_projects(uri);
        let own = self.definitions(ws, &scope, entry);
        if own.len() != 1 {
            return Err(Error::invalid_params(
                "Category rename requires one unambiguous declaration",
            ));
        }
        loop {
            let before = scope.len();
            let shared: BTreeSet<_> = self
                .entries(&scope)
                .filter(|(_, other)| other.name == entry.name)
                .map(|(uri, _)| uri.clone())
                .collect();
            for project in regions
                .values()
                .filter(|project| !project.is_disjoint(&shared))
            {
                if self.definitions(ws, project, entry) != own {
                    return Err(Error::invalid_params(
                        "Shared category has different or missing declarations",
                    ));
                }
                scope.extend(project.iter().cloned());
            }
            if scope.len() == before {
                break;
            }
        }
        // Generated BBL code can reference the category from an execute field
        // or preamble. Check after expanding every shared consumer so an open,
        // unsaved root cannot silently lose its unindexed category operands.
        if scope
            .iter()
            .filter_map(|uri| ws.documents.get(uri))
            .any(crate::bib_section_keys::database_execution)
        {
            return Err(Error::invalid_params(
                "Category rename requires bibliography execution to be fully indexed",
            ));
        }
        if self.dynamic.iter().any(|uri| scope.contains(uri))
            || self
                .ambiguous
                .iter()
                .any(|(uri, name)| scope.contains(uri) && *name == entry.name)
        {
            return Err(Error::invalid_params("Category rename is ambiguous: dynamic names, operands or differing project meanings"));
        }
        Ok(scope)
    }
}

fn applicable(ws: &Workspace, uri: &Url, offset: usize) -> bool {
    let Some(doc) = ws
        .documents
        .get(uri)
        .filter(|doc| doc.kind == DocumentKind::Latex)
    else {
        return false;
    };
    let names: BTreeSet<_> = ws
        .documents
        .values()
        .flat_map(|doc| {
            doc.syntax
                .definitions
                .iter()
                .chain(&doc.syntax.environments)
        })
        .map(|entry| entry.name.as_str())
        .collect();
    doc.syntax
        .commands
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, command)| command.span.end <= offset)
        .any(|(index, command)| {
            if COMMANDS.contains(&command.name.as_str()) {
                return fields(&doc.text.source, &command.name, command.span.end)
                    .0
                    .into_iter()
                    .any(|field| field.span.start <= offset && offset <= field.span.end);
            }
            if !matches!(command.name.as_str(), "csuse" | "csname")
                && !names.contains(command.name.as_str())
            {
                return false;
            }
            let mut end = if command.name == "csuse" {
                command
                    .argument
                    .as_ref()
                    .map_or(command.span.end, |a| a.outer_span.end)
            } else if command.name == "csname" {
                doc.syntax
                    .commands
                    .get(index + 1)
                    .filter(|next| next.name == "endcsname")
                    .map_or(command.span.end, |next| next.span.end)
            } else {
                command.span.end
            };
            end = syntax::skip_space(&doc.text.source, end);
            if doc.text.source.as_bytes().get(end) == Some(&b'*') {
                end += 1;
            }
            for _ in 0..2 {
                if let Some((field, after)) = group(&doc.text.source, end, b'{')
                    .or_else(|| group(&doc.text.source, end, b'['))
                {
                    if field.span.start <= offset && offset <= field.span.end {
                        return true;
                    }
                    end = after;
                } else {
                    break;
                }
            }
            false
        })
}

pub(crate) fn definitions(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    if !applicable(ws, uri, offset) {
        return None;
    }
    let index = Index::new(ws, uri);
    let entry = index.target(uri, offset, true)?;
    Some(index.definitions(ws, &ws.symbol_scope(uri), entry))
}
pub(crate) fn hover(ws: &Workspace, uri: &Url, offset: usize) -> Option<Hover> {
    if !applicable(ws, uri, offset) {
        return None;
    }
    let index = Index::new(ws, uri);
    let entry = index.target(uri, offset, true)?;
    let definitions = index.definitions(ws, &ws.symbol_scope(uri), entry);
    let locations = definitions
        .iter()
        .map(|location| location.uri.path())
        .collect::<Vec<_>>()
        .join("\n");
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::PlainText,
            value: format!("{} (biblatex category)\n{}", entry.name, locations),
        }),
        range: Some(Index::location(ws, uri, entry).range),
    })
}
pub(crate) fn references(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    include: bool,
) -> Option<Vec<Location>> {
    if !applicable(ws, uri, offset) {
        return None;
    }
    let index = Index::new(ws, uri);
    let entry = index.target(uri, offset, true)?;
    let scope = index
        .rename_scope(ws, uri, entry)
        .unwrap_or_else(|_| ws.symbol_scope(uri));
    Some(
        index
            .entries(&scope)
            .filter(|(_, other)| other.name == entry.name && (include || !other.declaration))
            .map(|(uri, entry)| Index::location(ws, uri, entry))
            .collect(),
    )
}
pub(crate) fn prepare(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
) -> Option<Result<Option<PrepareRenameResponse>>> {
    if !applicable(ws, uri, offset) {
        return None;
    }
    let index = Index::new(ws, uri);
    let entry = index.target(uri, offset, true)?;
    if index
        .definitions(ws, &ws.symbol_scope(uri), entry)
        .is_empty()
    {
        return Some(Ok(None));
    }

    Some(index.rename_scope(ws, uri, entry).map(|_| {
        Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: Index::location(ws, uri, entry).range,
            placeholder: entry.name.clone(),
        })
    }))
}
pub(crate) fn rename(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    name: &str,
) -> Option<Result<Option<WorkspaceEdit>>> {
    if !applicable(ws, uri, offset) {
        return None;
    }
    let index = Index::new(ws, uri);
    let entry = index.target(uri, offset, true)?;
    if index
        .definitions(ws, &ws.symbol_scope(uri), entry)
        .is_empty()
    {
        return Some(Ok(None));
    }

    Some((|| {
        if !valid(name) {
            return Err(Error::invalid_params("Invalid bibliography category name"));
        }
        let scope = index.rename_scope(ws, uri, entry)?;
        if name != entry.name
            && (index.entries(&scope).any(|(_, other)| other.name == name)
                || index
                    .ambiguous
                    .iter()
                    .any(|(uri, other)| scope.contains(uri) && other == name))
        {
            return Err(Error::invalid_params(
                "The target bibliography category already exists",
            ));
        }
        let mut changes = BTreeMap::<Url, Vec<TextEdit>>::new();
        for (uri, other) in index
            .entries(&scope)
            .filter(|(_, other)| other.name == entry.name)
        {
            changes.entry(uri.clone()).or_default().push(TextEdit {
                range: Index::location(ws, uri, other).range,
                new_text: name.into(),
            });
        }
        Ok(Some(crate::features::workspace_edit(ws, changes)))
    })())
}
pub(crate) fn completion(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<CompletionItem>> {
    if !applicable(ws, uri, offset) {
        return None;
    }
    let index = Index::new(ws, uri);
    let entry = index
        .target(uri, offset, false)
        .filter(|entry| !entry.declaration)?;
    let doc = ws.documents.get(uri)?;
    let prefix = &doc.text.source[entry.span.start..offset];
    if !entry.name.is_empty() && !valid(&entry.name) || !prefix.is_empty() && !valid(prefix) {
        return Some(vec![]);
    }
    let (range, extra) = syntax::name_completion_ranges(&doc.text, entry.span.clone(), offset)?;
    let mut seen = BTreeSet::new();
    Some(
        index
            .entries(&ws.symbol_scope(uri))
            .filter(|(_, other)| {
                other.declaration
                    && other.name.starts_with(prefix)
                    && seen.insert(other.name.clone())
            })
            .map(|(_, other)| CompletionItem {
                label: other.name.clone(),
                kind: Some(CompletionItemKind::REFERENCE),
                detail: Some("biblatex category".into()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: other.name.clone(),
                })),
                additional_text_edits: (!extra.is_empty()).then(|| extra.clone()),
                ..Default::default()
            })
            .collect(),
    )
}
