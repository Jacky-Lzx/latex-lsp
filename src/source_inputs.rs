//! Root-specific discovery of static indirect source, package and class loads.
use crate::{
    signature::{CommandSignatures, SourceCall},
    syntax,
    workspace::{resolve_file_path, DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::Url;

const MAX_ROUNDS: usize = 32;
const MAX_SOURCES: usize = 1024;

#[derive(Default)]
pub(crate) struct Index {
    pub calls: BTreeMap<Url, Vec<SourceCall>>,
    pub loaders: BTreeMap<Url, Vec<SourceCall>>,
    pub forwarders: BTreeMap<Url, Vec<SourceCall>>,
    pub resources: BTreeMap<Url, Vec<SourceCall>>,
    pub incomplete: bool,
    literal: Option<BTreeMap<Url, Vec<Url>>>,
}

impl Index {
    pub fn new(workspace: &Workspace, root: &Url) -> Self {
        let mut index = Self::default();
        let mut scope = index.scope(workspace, root);
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_ROUNDS {
            if index.incomplete || !seen.insert(scope.clone()) {
                break;
            }
            let signatures = CommandSignatures::source_context(scope.iter().filter_map(|uri| {
                workspace.documents.get(uri).and_then(|doc| {
                    (doc.kind == DocumentKind::Latex)
                        .then_some((doc.text.source.as_str(), &doc.syntax))
                })
            }));
            let mut literal = BTreeMap::new();
            let mut loader_calls = BTreeMap::new();
            let mut forwarders = BTreeMap::new();
            let mut resources = BTreeMap::new();
            index.calls = scope
                .iter()
                .filter_map(|uri| {
                    let doc = workspace.documents.get(uri)?;
                    if doc.kind != DocumentKind::Latex {
                        return None;
                    }
                    let positions =
                        signatures.source_command_positions(&doc.text.source, &doc.syntax);
                    literal.insert(
                        uri.clone(),
                        workspace
                            .literal_source_dependencies_filtered(uri, root, |command| {
                                positions.contains(&command.span.start)
                            })
                            .into_iter()
                            .map(|(uri, _)| uri)
                            .collect(),
                    );
                    let loaders = signatures.indirect_loader_invocations(
                        &doc.text.source,
                        &doc.syntax,
                        &positions,
                    );
                    if !loaders.is_empty() {
                        loader_calls.insert(uri.clone(), loaders);
                    }
                    let options = signatures.indirect_option_invocations(
                        &doc.text.source,
                        &doc.syntax,
                        &positions,
                    );
                    if !options.is_empty() {
                        forwarders.insert(uri.clone(), options);
                    }
                    let bibliography = signatures.indirect_bibliography_invocations(
                        &doc.text.source,
                        &doc.syntax,
                        &positions,
                    );
                    if !bibliography.is_empty() {
                        resources.insert(uri.clone(), bibliography);
                    }
                    let calls = signatures.indirect_source_invocations(
                        &doc.text.source,
                        &doc.syntax,
                        &positions,
                    );
                    (!calls.is_empty()).then_some((uri.clone(), calls))
                })
                .collect();
            index.loaders = loader_calls;
            index.forwarders = forwarders;
            index.resources = resources;
            index.literal = Some(literal);
            let next = index.scope(workspace, root);
            if !index.incomplete && next == scope {
                return index;
            }
            scope = next;
        }
        // Definitions discovered through a call may invalidate that very call.
        // Do not select an arbitrary interpretation of an oscillating graph.
        Self {
            calls: BTreeMap::new(),
            loaders: BTreeMap::new(),
            forwarders: BTreeMap::new(),
            resources: BTreeMap::new(),
            incomplete: true,
            literal: None,
        }
    }

    pub fn resource_dependencies(
        &self,
        workspace: &Workspace,
        uri: &Url,
        root: &Url,
    ) -> Vec<(Url, DocumentKind)> {
        let Some(doc) = workspace.documents.get(uri) else {
            return vec![];
        };
        self.resources
            .get(uri)
            .into_iter()
            .flatten()
            .flat_map(|call| resource_targets(&doc.text.source, uri, root, call))
            .collect()
    }

    fn scope(&mut self, workspace: &Workspace, root: &Url) -> BTreeSet<Url> {
        let mut result = BTreeSet::new();
        let mut pending = vec![root.clone()];
        while let Some(uri) = pending.pop() {
            if result.contains(&uri) {
                continue;
            }
            if result.len() >= MAX_SOURCES {
                self.incomplete = true;
                break;
            }
            result.insert(uri.clone());
            if let Some(literal) = &self.literal {
                pending.extend(literal.get(&uri).into_iter().flatten().cloned());
            } else {
                pending.extend(
                    workspace
                        .literal_source_dependencies_with_kinds(&uri, root)
                        .into_iter()
                        .map(|(uri, _)| uri),
                );
            }
            pending.extend(self.dependencies(workspace, &uri, root));
        }
        result
    }

    pub fn dependencies(&self, workspace: &Workspace, uri: &Url, root: &Url) -> Vec<Url> {
        self.calls
            .get(uri)
            .into_iter()
            .flatten()
            .filter_map(|call| target(uri, root, call))
            .chain(
                self.loaders
                    .get(uri)
                    .into_iter()
                    .flatten()
                    .flat_map(|call| {
                        workspace.documents.get(uri).map_or_else(Vec::new, |doc| {
                            loader_targets(&doc.text.source, uri, root, call)
                        })
                    }),
            )
            .collect()
    }
}

pub(crate) fn target(uri: &Url, root: &Url, call: &SourceCall) -> Option<Url> {
    let argument = call.command.argument.as_ref()?;
    if !call.certain || !syntax::is_literal_file_path(&argument.value) {
        return None;
    }
    resolve_file_path(root, &call.command.name, &argument.value, &["tex"])
        .or_else(|| resolve_file_path(uri, &call.command.name, &argument.value, &["tex"]))
}

/// Resolve complete literal package lists or a single literal class name.
fn loader_targets(source: &str, uri: &Url, root: &Url, call: &SourceCall) -> Vec<Url> {
    if !call.certain {
        return vec![];
    }
    let Some(extension) = syntax::loader_extension(&call.command.name) else {
        return vec![];
    };
    let names = if syntax::file_list(&call.command.name) {
        syntax::complete_file_list_spans(source, &call.command)
            .into_iter()
            .flatten()
            .map(|span| source[span].to_owned())
            .collect::<Vec<_>>()
    } else {
        syntax::file_argument(source, &call.command)
            .filter(|argument| syntax::is_literal_file_path(&argument.value))
            .map(|argument| vec![argument.value.to_string()])
            .unwrap_or_default()
    };
    names
        .into_iter()
        .filter_map(|name| {
            resolve_file_path(root, &call.command.name, &name, &[extension])
                .or_else(|| resolve_file_path(uri, &call.command.name, &name, &[extension]))
        })
        .collect()
}

/// Keep resource roles even when an alias uses an unconventional suffix.
fn resource_targets(
    source: &str,
    uri: &Url,
    root: &Url,
    call: &SourceCall,
) -> Vec<(Url, DocumentKind)> {
    if !call.certain {
        return vec![];
    }
    let (extension, kind) = match call.command.name.as_str() {
        "bibliography" | "nobibliography" => ("bib", DocumentKind::Bibtex),
        "bibliographystyle" => ("bst", DocumentKind::BibtexStyle),
        _ => return vec![],
    };
    let names = if syntax::file_list(&call.command.name) {
        syntax::file_list_spans(source, &call.command)
            .unwrap_or_default()
            .into_iter()
            .map(|span| source[span].to_owned())
            .collect::<Vec<_>>()
    } else {
        syntax::file_argument(source, &call.command)
            .map(|argument| vec![argument.value.clone()])
            .unwrap_or_default()
    };
    names
        .into_iter()
        .filter_map(|name| {
            resolve_file_path(root, &call.command.name, &name, &[extension])
                .or_else(|| resolve_file_path(uri, &call.command.name, &name, &[extension]))
                .map(|uri| (uri, kind))
        })
        .collect()
}
