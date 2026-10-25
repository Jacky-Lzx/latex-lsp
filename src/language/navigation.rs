use crate::{
    parser::syntax,
    project::workspace::{
        resolve_file_path, resolve_path, root_directive, Document, DocumentKind, Workspace,
    },
};
use lsp_types::*;
use std::{
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    path::PathBuf,
};

struct RankedSymbol {
    score: u8,
    ordinal: usize,
    symbol: SymbolInformation,
}

impl RankedSymbol {
    fn key(&self) -> (u8, &str, &Url, Position, usize) {
        (
            self.score,
            &self.symbol.name,
            &self.symbol.location.uri,
            self.symbol.location.range.start,
            self.ordinal,
        )
    }
}

impl PartialEq for RankedSymbol {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}
impl Eq for RankedSymbol {}
impl PartialOrd for RankedSymbol {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RankedSymbol {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

#[allow(deprecated)]
pub fn workspace_symbols(workspace: &Workspace, query: &str) -> Vec<SymbolInformation> {
    let query = query.trim().to_lowercase();
    let mut result = BinaryHeap::<RankedSymbol>::new();
    let mut ordinal = 0;
    for (uri, doc) in &workspace.documents {
        // Release uncached outlines after each document, including when the
        // shared budget is full. Retain at most 256 ranked protocol results.
        let outline = super::symbol_cache::outline(doc);
        visit_symbols(&outline.symbols, None, &mut |symbol, container| {
            if doc.kind == DocumentKind::Bibtex && symbol.kind == SymbolKind::PROPERTY {
                return;
            }
            let name = symbol.name.to_lowercase();
            let score = if name == query {
                0
            } else if name.starts_with(&query) {
                1
            } else if name.contains(&query) {
                2
            } else {
                let mut characters = name.chars();
                if !query
                    .chars()
                    .all(|needle| characters.any(|character| character == needle))
                {
                    return;
                }
                3
            };
            // The ordinal preserves the old stable order for coincident symbols.
            ordinal += 1;
            let key = (
                score,
                symbol.name.as_str(),
                uri,
                symbol.selection_range.start,
                ordinal,
            );
            if result.len() == 256 {
                if result.peek().is_some_and(|worst| key >= worst.key()) {
                    return;
                }
                result.pop();
            }
            result.push(RankedSymbol {
                score,
                ordinal,
                symbol: SymbolInformation {
                    name: symbol.name.clone(),
                    kind: symbol.kind,
                    tags: symbol.tags.clone(),
                    deprecated: None,
                    location: Location {
                        uri: uri.clone(),
                        range: symbol.selection_range,
                    },
                    container_name: container.map(str::to_owned),
                },
            });
        });
    }
    result
        .into_sorted_vec()
        .into_iter()
        .map(|ranked| ranked.symbol)
        .collect()
}

fn visit_symbols<'a>(
    symbols: &'a [DocumentSymbol],
    container: Option<&'a str>,
    visit: &mut impl FnMut(&'a DocumentSymbol, Option<&'a str>),
) {
    for symbol in symbols {
        visit(symbol, container);
        if let Some(children) = &symbol.children {
            visit_symbols(children, Some(&symbol.name), visit);
        }
    }
}

pub use crate::language::folding::{folding_ranges, folding_ranges_for_client};

fn unescape(value: &str) -> String {
    let mut value = value.to_owned();
    for character in ['_', '%', '#', '&', '$'] {
        value = value.replace(&format!("\\{character}"), &character.to_string());
    }
    value
}

fn external_url(value: &str) -> Option<Url> {
    Url::parse(value)
        .ok()
        .filter(|uri| matches!(uri.scheme(), "http" | "https" | "mailto" | "file" | "ftp"))
}

/// The four-argument hyperref form appends its category and name as a destination.
/// Bracketed label links have a different argument layout and are handled by navigation.
fn hyperref_url_link(doc: &Document, command: &syntax::Command) -> Option<DocumentLink> {
    let source = &doc.text.source;
    let first = syntax::skip_space(source, command.span.end);
    let argument = command.argument.as_ref()?;
    if source.as_bytes().get(first) != Some(&b'{') || argument.outer_span.start != first {
        return None;
    }
    let (category, after) = syntax::group(
        source,
        syntax::skip_space(source, argument.outer_span.end),
        b'{',
        b'}',
    )?;
    let (name, _) = syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?;
    let category = category.value;
    let name = name.value;
    if [&category, &name]
        .iter()
        .any(|value| !value.is_empty() && !syntax::is_static_key(value))
    {
        return None;
    }
    let mut value = unescape(argument.value.trim());
    if value.contains(['\\', '{', '}']) {
        return None;
    }
    if !category.is_empty() || !name.is_empty() {
        value.push('#');
        if !category.is_empty() {
            value.push_str(&category);
            value.push('.');
        }
        value.push_str(&name);
    }
    let span = syntax::trimmed_span(source, argument.span.clone());
    Some(DocumentLink {
        range: doc.text.range(span.start, span.end),
        target: Some(external_url(&value)?),
        tooltip: None,
        data: None,
    })
}

fn file_link(
    uri: &Url,
    name: &str,
    extensions: &[&str],
    graphics: &[PathBuf],
    root: &Url,
    command: &str,
) -> Option<Url> {
    if command == "includegraphics" {
        return crate::project::graphics::resolve_in(root, name, extensions, graphics);
    }
    if syntax::code_input_command(command) {
        return resolve_file_path(root, command, name, extensions);
    }
    for origin in std::iter::once(root).chain(std::iter::once(uri).filter(|uri| *uri != root)) {
        if let Some(target) = resolve_file_path(origin, command, name, extensions) {
            return Some(target);
        }
    }
    None
}

pub fn document_links(workspace: &Workspace, uri: &Url) -> Vec<DocumentLink> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    if doc.kind != DocumentKind::Latex {
        return crate::bibliography::bib_links::document_links(doc);
    }
    if let Ok(root) = workspace.root_document(uri) {
        return document_links_in(workspace, uri, &root);
    }
    let roots = workspace.source_roots(uri);
    if roots.len() == 1 {
        return document_links_in(workspace, uri, &roots[0]);
    }
    let mut links = BTreeMap::<(Position, Position), (DocumentLink, usize, bool)>::new();
    for root in &roots {
        for link in document_links_in(workspace, uri, root) {
            links
                .entry((link.range.start, link.range.end))
                .and_modify(|(first, count, conflict)| {
                    *count += 1;
                    *conflict |= first.target != link.target;
                })
                .or_insert((link, 1, false));
        }
    }
    links
        .into_values()
        .filter_map(|(link, count, conflict)| (count == roots.len() && !conflict).then_some(link))
        .collect()
}

pub(crate) fn document_links_in(workspace: &Workspace, uri: &Url, root: &Url) -> Vec<DocumentLink> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let mut links = Vec::new();
    let inputs = crate::project::source_inputs::Index::new(workspace, root);
    let images = crate::project::graphics::calls(workspace, root);
    let has_images = images
        .get(uri)
        .is_some_and(|calls| calls.iter().any(|call| call.certain));
    let graphic_directories =
        has_images.then(|| crate::project::graphics_settings::Index::directories(workspace, root));
    let graphic_extensions =
        has_images.then(|| crate::project::graphics_settings::Index::new(workspace, root));
    if let Some(directive) = root_directive(doc) {
        if let Some(target) = resolve_path(uri, &directive.value, "tex") {
            links.push(DocumentLink {
                range: doc.text.range(directive.span.start, directive.span.end),
                target: Some(target),
                tooltip: Some("TeX root document".into()),
                data: None,
            });
        }
    }
    for call in inputs.calls.get(uri).into_iter().flatten() {
        if let (Some(argument), Some(target)) = (
            call.command.argument.as_ref(),
            crate::project::source_inputs::target(uri, root, call),
        ) {
            links.push(DocumentLink {
                range: doc.text.range(argument.span.start, argument.span.end),
                target: Some(target),
                tooltip: None,
                data: None,
            });
        }
    }
    let editable_sources = doc
        .syntax
        .commands
        .iter()
        .any(|command| crate::project::source_inputs::editable_file_command(&command.name))
        .then(|| {
            let mut scope = workspace.project_documents(root);
            scope.insert(uri.clone());
            crate::project::source_inputs::editable_file_positions(workspace, &scope)
        });
    for command in doc
        .syntax
        .commands
        .iter()
        .filter(|command| {
            command.name != "includegraphics"
                && (!crate::project::source_inputs::editable_file_command(&command.name)
                    || editable_sources
                        .as_ref()
                        .and_then(|sources| sources.get(uri))
                        .is_some_and(|positions| positions.contains(&command.span.start)))
        })
        .chain(
            inputs
                .loaders
                .get(uri)
                .into_iter()
                .flatten()
                .chain(inputs.forwarders.get(uri).into_iter().flatten())
                .chain(inputs.resource_references.get(uri).into_iter().flatten())
                .chain(inputs.selectors.get(uri).into_iter().flatten())
                .chain(inputs.checks.get(uri).into_iter().flatten())
                .chain(images.get(uri).into_iter().flatten())
                .filter(|call| call.certain)
                .map(|call| &call.command),
        )
    {
        if syntax::code_input_command(&command.name)
            && !crate::project::code_inputs::supported(workspace, uri, root, command)
        {
            continue;
        }
        if command.name == "hyperref" {
            links.extend(hyperref_url_link(doc, command));
            continue;
        }
        let Some(argument) = syntax::file_argument(&doc.text.source, command) else {
            continue;
        };
        let extensions = match syntax::inferred_file_extensions(&command.name) {
            Some(extensions) => extensions,
            None if matches!(command.name.as_str(), "url" | "href") => &[],
            None => continue,
        };
        let graphics = if command.name == "includegraphics" {
            let Some(paths) = graphic_directories
                .as_ref()
                .and_then(|index| index.paths_at(root, uri, command.span.start, true))
            else {
                continue;
            };
            paths
        } else {
            vec![]
        };
        let image_extensions = if command.name == "includegraphics" {
            let Some(extensions) = graphic_extensions
                .as_ref()
                .and_then(|index| index.at(uri, command.span.start))
            else {
                continue;
            };
            Some(extensions)
        } else {
            None
        };
        let extensions = image_extensions.as_deref().unwrap_or(extensions);
        let names = if command.name == "includegraphics" {
            vec![(argument.span.clone(), argument.value.as_str())]
        } else if let Some(spans) = syntax::file_list_spans(&doc.text.source, command) {
            spans
                .into_iter()
                .map(|span| (span.clone(), &doc.text.source[span]))
                .collect::<Vec<_>>()
        } else {
            let values = if syntax::file_list(&command.name) {
                argument.value.split(',').collect::<Vec<_>>()
            } else {
                vec![argument.value.as_str()]
            };
            let mut start = argument.span.start;
            values
                .into_iter()
                .map(|name| {
                    let span = syntax::trimmed_span(&doc.text.source, start..start + name.len());
                    start += name.len() + 1;
                    (span, name)
                })
                .collect()
        };
        for (span, name) in names {
            let value = unescape(if command.name == "bibliographystyle" {
                name
            } else {
                name.trim()
            });
            let target = if matches!(command.name.as_str(), "url" | "href") {
                external_url(&value)
            } else {
                file_link(
                    uri,
                    &value,
                    extensions,
                    if command.name == "includegraphics" {
                        &graphics
                    } else {
                        &[]
                    },
                    root,
                    &command.name,
                )
                .or_else(|| workspace.distribution.resolve(&command.name, &value))
            };
            if let Some(target) = target {
                links.push(DocumentLink {
                    range: doc.text.range(span.start, span.end),
                    target: Some(target),
                    tooltip: None,
                    data: None,
                });
            }
        }
    }
    crate::project::imports::update_links(workspace, uri, root, &mut links);
    links.extend(crate::compilation::auxiliary::document_links(
        workspace, uri, root,
    ));
    let mut seen = BTreeSet::new();
    links.retain(|link| seen.insert((link.range.start, link.range.end)));
    links.sort_by_key(|link| (link.range.start, link.range.end));
    links
}
