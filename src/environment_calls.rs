//! Indirect environment name arguments interpreted in every project consuming a file.
use crate::{
    signature::CommandSignatures,
    syntax::{self, Argument},
    workspace::{Document, DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::{jsonrpc, lsp_types::Url};

type Calls = BTreeMap<usize, (Argument, bool)>;

#[derive(Default)]
pub(crate) struct Index {
    certain: BTreeMap<Url, BTreeMap<usize, Argument>>,
    ambiguous: Vec<(Url, BTreeSet<String>)>,
}

impl Index {
    pub(crate) fn new(workspace: &Workspace) -> Self {
        let mut contexts: BTreeMap<BTreeSet<Url>, BTreeSet<Url>> = BTreeMap::new();
        let mut covered = BTreeSet::new();
        for scope in workspace.saved_projects().into_values() {
            covered.extend(scope.iter().cloned());
            contexts.entry(scope.clone()).or_default().extend(scope);
        }
        for uri in workspace
            .documents
            .keys()
            .filter(|uri| !covered.contains(*uri))
        {
            contexts
                .entry(workspace.symbol_scope(uri))
                .or_default()
                .insert(uri.clone());
        }
        let mut interpretations: BTreeMap<Url, Vec<Calls>> = BTreeMap::new();
        for (scope, consumers) in contexts {
            let signatures = CommandSignatures::new(
                workspace
                    .documents
                    .iter()
                    .filter(|(uri, doc)| scope.contains(*uri) && doc.kind == DocumentKind::Latex)
                    .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
            );
            for uri in consumers {
                let Some(doc) = workspace
                    .documents
                    .get(&uri)
                    .filter(|doc| doc.kind == DocumentKind::Latex)
                else {
                    continue;
                };
                let calls = signatures
                    .environment_calls(&doc.text.source, &doc.syntax)
                    .into_iter()
                    .map(|(argument, operand)| (argument.span.start, (argument, operand)))
                    .collect();
                interpretations.entry(uri).or_default().push(calls);
            }
        }
        let mut index = Self::default();
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
                    && candidates.iter().all(|argument| {
                        !argument.1
                            && argument.0.span == first.0.span
                            && argument.0.value == first.0.value
                    })
                {
                    index
                        .certain
                        .entry(uri.clone())
                        .or_default()
                        .insert(start, first.0.clone());
                } else {
                    index.ambiguous.push((
                        uri.clone(),
                        candidates
                            .iter()
                            .map(|argument| argument.0.value.clone())
                            .collect(),
                    ));
                }
            }
        }
        index
    }

    pub(crate) fn references<'a>(
        &'a self,
        uri: &Url,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Argument> {
        self.certain
            .get(uri)
            .into_iter()
            .flat_map(|calls| calls.values())
            .filter(move |argument| argument.value == name)
    }

    pub(crate) fn target(&self, uri: &Url, offset: usize) -> Option<std::ops::Range<usize>> {
        self.certain
            .get(uri)?
            .values()
            .find(|argument| argument.span.start <= offset && offset <= argument.span.end)
            .map(|argument| argument.span.clone())
    }

    pub(crate) fn validate(&self, scope: &BTreeSet<Url>, name: &str) -> jsonrpc::Result<()> {
        if self
            .ambiguous
            .iter()
            .any(|(uri, names)| scope.contains(uri) && names.contains(name))
        {
            return Err(jsonrpc::Error::invalid_params(
                "Rename is ambiguous: an environment name is used as data or has different meanings in its projects",
            ));
        }
        Ok(())
    }
}

/// Avoid building a project index for ordinary text and command-name requests.
pub(crate) fn may_contain_name(doc: &Document, offset: usize) -> bool {
    doc.syntax.commands.iter().any(|command| {
        let Some(first) = &command.argument else {
            return false;
        };
        if first.span.start <= offset && offset <= first.span.end {
            return true;
        }
        command.name == "csuse"
            && syntax::group(
                &doc.text.source,
                syntax::skip_space(&doc.text.source, first.outer_span.end),
                b'{',
                b'}',
            )
            .is_some_and(|(argument, _)| {
                argument.span.start <= offset && offset <= argument.span.end
            })
    })
}
