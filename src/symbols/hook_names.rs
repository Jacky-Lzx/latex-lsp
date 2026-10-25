//! Literal kernel hook names, separate from code labels and reference keys.
use crate::{
    language::signature::CommandSignatures,
    parser::syntax,
    project::workspace::{Document, DocumentKind, Workspace},
};
use lsp_types::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range as Span,
};

const COMMANDS: &[&str] = &[
    "NewHook",
    "NewReversedHook",
    "NewHookWithArguments",
    "NewReversedHookWithArguments",
    "NewMirroredHookPair",
    "NewMirroredHookPairWithArguments",
    "AddToHook",
    "AddToHookWithArguments",
    "AddToHookNext",
    "AddToHookNextWithArguments",
    "RemoveFromHook",
    "UseHook",
    "UseOneTimeHook",
    "UseHookWithArguments",
    "UseOneTimeHookWithArguments",
    "ClearHookNext",
    "ActivateGenericHook",
    "DisableGenericHook",
    "ShowHook",
    "LogHook",
    "DeclareHookRule",
    "ClearHookRule",
    "IfHookEmptyTF",
    "IfHookEmptyT",
    "IfHookEmptyF",
    "IfHookExistsTF",
];

fn valid(name: &str) -> bool {
    syntax::is_static_key(name) && name != "." && !name.starts_with("./")
}

#[derive(Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    span: Span<usize>,
    declaration: bool,
    complete: bool,
}

fn field(source: &str, after: usize) -> Option<(Span<usize>, usize, bool)> {
    let at = syntax::skip_space(source, after);
    if source.as_bytes().get(at) == Some(&b'{') {
        return Some(syntax::group(source, at, b'{', b'}').map_or_else(
            || {
                let end = source[at + 1..]
                    .find(['\r', '\n'])
                    .map_or(source.len(), |len| at + 1 + len);
                (at + 1..end, source.len(), false)
            },
            |(argument, end)| {
                let argument = syntax::single_key_argument(source, argument);
                (argument.span, end, true)
            },
        ));
    }
    // An ungrouped m argument is one token. Only ASCII characters have the
    // same token boundary in all supported engines.
    let byte = *source.as_bytes().get(at)?;
    let span = if byte == b'\\' {
        syntax::control_word(source, at)?
    } else if byte.is_ascii() && !matches!(byte, b'{' | b'}' | b'%') {
        at..at + 1
    } else {
        return None;
    };
    let end = span.end;
    Some((span, end, true))
}

fn calls(doc: &Document, signatures: &CommandSignatures<'_>) -> BTreeMap<usize, Entry> {
    let source = &doc.text.source;
    let context = crate::symbols::named_references::Context::new(doc, signatures);
    let declarations: BTreeSet<_> = doc
        .syntax
        .definitions
        .iter()
        .chain(&doc.syntax.environments)
        .flat_map(|entry| [entry.span.start, entry.span.start.saturating_sub(1)])
        .collect();
    let empty = BTreeSet::new();
    let mut result = BTreeMap::new();
    for (index, command) in doc.syntax.commands.iter().enumerate() {
        if declarations.contains(&command.span.start)
            || context.operand(doc, command.span.start)
            || context.literal(command.span.start)
        {
            continue;
        }
        let (name, invocation) = signatures
            .named_invocation(source, &doc.syntax, index, &empty, &context.declarations)
            .unwrap_or((&command.name, command.span.clone()));
        let name = if COMMANDS.contains(&name) && signatures.command_unchanged(name) {
            name
        } else if let Some(name) = signatures
            .forwarded_builtin(name)
            .filter(|name| COMMANDS.contains(name))
        {
            name
        } else {
            continue;
        };
        let declaration = name.starts_with("New");
        let count = if name.starts_with("NewMirroredHookPair") {
            2
        } else {
            1
        };
        let mut after = invocation.end;
        for _ in 0..count {
            let Some((mut span, end, complete)) = field(source, after) else {
                break;
            };
            after = end;
            // Registration normalizes names; UseHook and the conditions use
            // the internal control sequence with literal whitespace.
            if !matches!(name, "UseHook" | "UseHookWithArguments")
                && !crate::language::signature::HOOK_CONDITIONS.contains(&name)
            {
                let raw = &source[span.clone()];
                span.start += raw.len() - raw.trim_start_matches([' ', '\t', '\r', '\n']).len();
                span.end = span.start
                    + source[span.clone()]
                        .trim_end_matches([' ', '\t', '\r', '\n'])
                        .len();
            }
            let value = &source[span.clone()];
            if valid(value) || value.is_empty() {
                result.insert(
                    span.start,
                    Entry {
                        name: value.into(),
                        span,
                        declaration,
                        complete,
                    },
                );
            }
            if !complete {
                break;
            }
        }
    }
    result
}

#[derive(Default)]
struct Index {
    entries: BTreeMap<Url, BTreeMap<usize, Entry>>,
}

impl Index {
    fn new(ws: &Workspace, origin: &Url) -> Self {
        if !ws.index_truncated.is_empty()
            || !ws.documents.values().any(|doc| {
                doc.kind == DocumentKind::Latex
                    && COMMANDS.iter().any(|name| doc.text.source.contains(name))
            })
        {
            return Self::default();
        }
        let contexts = crate::symbols::environment_calls::project_contexts(ws);
        let mut connected = ws.symbol_scope(origin);
        loop {
            let before = connected.len();
            for scope in contexts.keys() {
                if !scope.is_disjoint(&connected) {
                    connected.extend(scope.iter().cloned());
                }
            }
            if before == connected.len() {
                break;
            }
        }
        let mut interpretations = BTreeMap::<Url, Vec<BTreeMap<usize, Entry>>>::new();
        let (mut bytes, mut commands, mut definitions, mut entries) =
            (0usize, 0usize, 0usize, 0usize);
        for (scope, consumers) in contexts {
            if scope.is_disjoint(&connected) {
                continue;
            }
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
            if bytes > 32 * 1024 * 1024 || commands > 100_000 || definitions > 8192 {
                return Self::default();
            }
            let signatures = CommandSignatures::new(
                docs.iter()
                    .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
            );
            for uri in consumers {
                let Some(doc) = ws
                    .documents
                    .get(&uri)
                    .filter(|doc| doc.kind == DocumentKind::Latex)
                else {
                    continue;
                };
                let calls = calls(doc, &signatures);
                entries = entries.saturating_add(calls.len());
                if entries > 100_000 {
                    return Self::default();
                }
                interpretations.entry(uri).or_default().push(calls);
            }
        }
        let mut result = Self::default();
        for (uri, interpretations) in interpretations {
            let Some(first) = interpretations.first() else {
                continue;
            };
            let entries = first
                .iter()
                .filter(|(at, entry)| {
                    interpretations
                        .iter()
                        .all(|other| other.get(at) == Some(entry))
                })
                .map(|(at, entry)| (*at, entry.clone()))
                .collect();
            result.entries.insert(uri, entries);
        }
        result
    }

    fn target(&self, uri: &Url, offset: usize, complete: bool) -> Option<&Entry> {
        self.entries.get(uri)?.values().find(|entry| {
            (!complete || entry.complete && valid(&entry.name))
                && entry.span.start <= offset
                && offset <= entry.span.end
        })
    }

    fn entries<'a>(
        &'a self,
        scope: &'a BTreeSet<Url>,
    ) -> impl Iterator<Item = (&'a Url, &'a Entry)> {
        self.entries
            .iter()
            .filter(|(uri, _)| scope.contains(*uri))
            .flat_map(|(uri, entries)| {
                entries
                    .values()
                    .filter(|entry| entry.complete && valid(&entry.name))
                    .map(move |entry| (uri, entry))
            })
    }

    fn location(ws: &Workspace, uri: &Url, entry: &Entry) -> Location {
        Location {
            uri: uri.clone(),
            range: ws.documents[uri]
                .text
                .range(entry.span.start, entry.span.end),
        }
    }

    fn definitions(&self, ws: &Workspace, uri: &Url, name: &str) -> Vec<Location> {
        self.entries(&ws.symbol_scope(uri))
            .filter(|(_, entry)| entry.declaration && entry.name == name)
            .map(|(uri, entry)| Self::location(ws, uri, entry))
            .collect()
    }
}

pub(crate) fn definitions(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    let index = Index::new(ws, uri);
    let entry = index.target(uri, offset, true)?;
    Some(index.definitions(ws, uri, &entry.name))
}

pub(crate) fn references(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    include: bool,
) -> Option<Vec<Location>> {
    let index = Index::new(ws, uri);
    let entry = index.target(uri, offset, true)?;
    let mut scope = ws.symbol_scope(uri);
    let definitions = index.definitions(ws, uri, &entry.name);
    if definitions.len() == 1 {
        for project in ws
            .symbol_regions()
            .into_values()
            .filter(|project| project.contains(&definitions[0].uri))
        {
            scope.extend(project);
        }
    }
    Some(
        index
            .entries(&scope)
            .filter(|(_, other)| other.name == entry.name && (include || !other.declaration))
            .map(|(uri, entry)| Index::location(ws, uri, entry))
            .collect(),
    )
}

pub(crate) fn hover(ws: &Workspace, uri: &Url, offset: usize) -> Option<Hover> {
    let index = Index::new(ws, uri);
    let entry = index.target(uri, offset, true)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::PlainText,
            value: format!("{} (LaTeX kernel hook)", entry.name),
        }),
        range: Some(Index::location(ws, uri, entry).range),
    })
}

pub(crate) fn completion(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<CompletionItem>> {
    let index = Index::new(ws, uri);
    let target = index.target(uri, offset, false)?;
    if target.declaration {
        return Some(vec![]);
    }
    let doc = ws.documents.get(uri)?;
    let prefix = &doc.text.source[target.span.start..offset];
    let names: BTreeSet<_> = index
        .entries(&ws.symbol_scope(uri))
        .filter(|(_, entry)| entry.declaration && entry.name.starts_with(prefix))
        .map(|(_, entry)| entry.name.clone())
        .collect();
    Some(
        names
            .into_iter()
            .take(256)
            .take(ws.completion_budget())
            .map(|name| CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::EVENT),
                detail: Some("LaTeX kernel hook".into()),
                filter_text: Some(name.clone()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: Index::location(ws, uri, target).range,
                    new_text: name,
                })),
                ..Default::default()
            })
            .collect(),
    )
}
