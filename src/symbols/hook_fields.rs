//! Bounded hook fields with meanings shared by every consuming project.
use crate::{
    language::signature::CommandSignatures,
    project::workspace::{Document, DocumentKind, Workspace},
};
use lsp_types::Url;
use std::collections::BTreeMap;

pub(super) fn index<E: Clone + Eq>(
    ws: &Workspace,
    origin: &Url,
    roots: &[&str],
    read: impl Fn(&Document, &CommandSignatures<'_>) -> BTreeMap<usize, E>,
    selected: impl Fn(&E) -> bool,
) -> BTreeMap<Url, BTreeMap<usize, E>> {
    if !ws.index_truncated.is_empty()
        || !ws.documents.values().any(|doc| {
            doc.kind == DocumentKind::Latex
                && roots.iter().any(|name| doc.text.source.contains(name))
        })
    {
        return BTreeMap::new();
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
    let mut interpretations = BTreeMap::<Url, Vec<BTreeMap<usize, E>>>::new();
    let (mut bytes, mut commands, mut definitions, mut entries) = (0usize, 0usize, 0usize, 0usize);
    for (scope, consumers) in contexts {
        if scope.is_disjoint(&connected) {
            continue;
        }
        let docs: Vec<_> = scope
            .iter()
            .filter_map(|uri| ws.documents.get(uri))
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
        if bytes > 32 * 1024 * 1024 || commands > 100_000 || definitions > 8192 {
            return BTreeMap::new();
        }
        let signatures = CommandSignatures::new(
            docs.iter()
                .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
        );
        // All callers need an entry at this cursor. Inspect its document
        // first so an ordinary label or command query does not index hook
        // fields in every other source. A shared source must still match in
        // each consumer interpretation before any result can be returned.
        for uri in consumers
            .iter()
            .filter(|uri| *uri == origin)
            .chain(consumers.iter().filter(|uri| *uri != origin))
        {
            let Some(doc) = ws
                .documents
                .get(uri)
                .filter(|doc| doc.kind == DocumentKind::Latex)
            else {
                continue;
            };
            let calls = read(doc, &signatures);
            if uri == origin && !calls.values().any(&selected) {
                return BTreeMap::new();
            }
            entries = entries.saturating_add(calls.len());
            if entries > 100_000 {
                return BTreeMap::new();
            }
            interpretations.entry(uri.clone()).or_default().push(calls);
        }
    }
    let mut result = BTreeMap::new();
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
        result.insert(uri, entries);
    }
    result
}
