//! Static, project-scoped call hierarchy for uniquely declared user macros.
use crate::{
    signature::CommandSignatures,
    syntax,
    workspace::{Document, DocumentKind, Workspace},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    ops::Range as Span,
};
use tower_lsp::{
    jsonrpc::{Error, ErrorCode, Result},
    lsp_types::*,
};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    origin: Url,
    uri: Url,
    name: Option<String>,
    fingerprint: String,
}
struct Definition {
    uri: Url,
    name: String,
    declaration: Span<usize>,
    body: Span<usize>,
    selection: Range,
    eligible: bool,
    node: Option<usize>,
}
struct Index {
    nodes: Vec<CallHierarchyItem>,
    by_name: BTreeMap<String, usize>,
    edges: BTreeMap<(usize, usize), Vec<Range>>,
}
fn fingerprint(doc: &Document) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    doc.text.source.hash(&mut hash);
    doc.version.hash(&mut hash);
    format!("{:016x}", hash.finish())
}
fn item(
    origin: &Url,
    uri: &Url,
    doc: &Document,
    definition: Option<&Definition>,
    source_fingerprint: &str,
) -> CallHierarchyItem {
    let name = definition.map(|definition| definition.name.clone());
    CallHierarchyItem {
        name: name
            .as_ref()
            .map(|name| format!("\\{name}"))
            .unwrap_or_else(|| {
                uri.to_file_path()
                    .ok()
                    .and_then(|path| {
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .unwrap_or_else(|| uri.path().to_owned())
            }),
        kind: if definition.is_some() {
            SymbolKind::FUNCTION
        } else {
            SymbolKind::FILE
        },
        tags: None,
        detail: Some(uri.path().to_owned()),
        uri: uri.clone(),
        range: definition.map_or_else(
            || doc.text.range(0, doc.text.source.len()),
            |definition| {
                doc.text
                    .range(definition.declaration.start, definition.declaration.end)
            },
        ),
        selection_range: definition.map_or_else(Range::default, |definition| definition.selection),
        data: Some(
            serde_json::to_value(Identity {
                origin: origin.clone(),
                uri: uri.clone(),
                name,
                fingerprint: source_fingerprint.into(),
            })
            .unwrap(),
        ),
    }
}
fn budget() -> Error {
    crate::formatting::failure(
        "Call hierarchy exceeds the source or call budget; narrow the project scope",
    )
}
fn modified() -> Error {
    Error {
        code: ErrorCode::ServerError(-32801),
        message: "Call hierarchy item is outdated; prepare the hierarchy again".into(),
        data: None,
    }
}
fn declaration_start(doc: &Document, entry: &syntax::Entry) -> usize {
    let declaration = entry.detail.strip_prefix("Defined with \\");
    doc.syntax.commands[..doc
        .syntax
        .commands
        .partition_point(|command| command.span.start < entry.span.start)]
        .iter()
        .rev()
        .find(|command| Some(command.name.as_str()) == declaration)
        .map_or(entry.span.start, |command| command.span.start)
}
impl Index {
    fn new(ws: &Workspace, origin: &Url) -> Result<Self> {
        if !ws.index_truncated.is_empty() {
            return Err(crate::formatting::failure("Project indexing is incomplete; reindex or adjust indexing limits before requesting a call hierarchy"));
        }
        if !ws
            .documents
            .get(origin)
            .is_some_and(|doc| doc.kind == DocumentKind::Latex)
        {
            return Err(modified());
        }
        let scope = ws.symbol_scope(origin);
        let docs: Vec<_> = scope
            .iter()
            .filter_map(|uri| {
                ws.documents
                    .get(uri)
                    .filter(|doc| doc.kind == DocumentKind::Latex)
                    .map(|doc| (uri, doc))
            })
            .collect();
        if docs.len() > 1024
            || docs
                .iter()
                .map(|(_, doc)| doc.text.source.len())
                .sum::<usize>()
                > 16 * 1024 * 1024
            || docs
                .iter()
                .map(|(_, doc)| doc.syntax.commands.len())
                .sum::<usize>()
                > 100_000
            || docs
                .iter()
                .map(|(_, doc)| doc.syntax.definitions.len())
                .sum::<usize>()
                > 4096
        {
            return Err(budget());
        }
        let fingerprints: BTreeMap<_, _> = docs
            .iter()
            .map(|(uri, doc)| ((*uri).clone(), fingerprint(doc)))
            .collect();
        let signatures = CommandSignatures::new(
            docs.iter()
                .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
        );
        let mut counts = BTreeMap::<&str, usize>::new();
        for (_, doc) in &docs {
            for definition in &doc.syntax.definitions {
                *counts.entry(&definition.name).or_default() += 1;
            }
        }
        let mut definitions = Vec::new();
        for (uri, doc) in &docs {
            let bodies = crate::signature::definition_body_spans(&doc.text.source, &doc.syntax);
            for entry in &doc.syntax.definitions {
                let Some(body) =
                    crate::signature::command_definition_body(&doc.text.source, &doc.syntax, entry)
                else {
                    continue;
                };
                let eligible = counts.get(entry.name.as_str()) == Some(&1)
                    && !bodies.iter().any(|body| body.contains(&entry.span.start));
                definitions.push(Definition {
                    uri: (*uri).clone(),
                    name: entry.name.clone(),
                    declaration: declaration_start(doc, entry)..body.outer_span.end,
                    body: body.span,
                    selection: doc.text.range(entry.span.start, entry.span.end),
                    eligible,
                    node: None,
                });
            }
        }
        let mut index = Self {
            nodes: vec![],
            by_name: BTreeMap::new(),
            edges: BTreeMap::new(),
        };
        for definition in &mut definitions {
            if !definition.eligible {
                continue;
            }
            let doc = &ws.documents[&definition.uri];
            let node = index.nodes.len();
            index.nodes.push(item(
                origin,
                &definition.uri,
                doc,
                Some(definition),
                &fingerprints[&definition.uri],
            ));
            index.by_name.insert(definition.name.clone(), node);
            definition.node = Some(node);
        }
        let mut calls = 0usize;
        for (uri, doc) in docs {
            let file = index.nodes.len();
            index
                .nodes
                .push(item(origin, uri, doc, None, &fingerprints[uri]));
            let operands = signatures.argument_tokens(&doc.text.source, &doc.syntax);
            let eligible_data: BTreeSet<_> = definitions
                .iter()
                .filter(|definition| definition.uri == *uri && definition.eligible)
                .flat_map(|definition| {
                    [
                        (definition.declaration.start, definition.declaration.end),
                        (
                            doc.text.offset(definition.selection.start).unwrap(),
                            definition.declaration.end,
                        ),
                    ]
                })
                .collect();
            let mut data: Vec<_> =
                crate::signature::definition_data_spans(&doc.text.source, &doc.syntax)
                    .into_iter()
                    .filter(|span| !eligible_data.contains(&(span.start, span.end)))
                    .collect();
            data.extend(doc.syntax.ignored.iter().cloned());
            data.extend(doc.syntax.literal_conversions.iter().cloned());
            data.extend(doc.syntax.literal_tests.iter().cloned());
            data.extend(doc.syntax.ambiguous_string_tests.iter().cloned());
            data.extend(
                crate::expl3::math_data_spans(doc)
                    .into_iter()
                    .filter(|span| !eligible_data.contains(&(span.start, span.end))),
            );
            data.sort_unstable_by_key(|span| span.start);
            let mut merged: Vec<Span<usize>> = vec![];
            for span in data {
                if let Some(last) = merged.last_mut().filter(|last| span.start <= last.end) {
                    last.end = last.end.max(span.end);
                } else {
                    merged.push(span);
                }
            }
            let mut opaque = 0;
            let mut local: Vec<_> = definitions
                .iter()
                .filter(|definition| definition.uri == *uri)
                .collect();
            local.sort_unstable_by_key(|definition| definition.declaration.start);
            let mut next = 0;
            let mut stack: Vec<&Definition> = vec![];
            let mut noexpand = None;
            for command in &doc.syntax.commands {
                let at = command.span.start;
                while next < local.len() && local[next].declaration.start <= at {
                    while stack.last().is_some_and(|definition| {
                        definition.declaration.end <= local[next].declaration.start
                    }) {
                        stack.pop();
                    }
                    stack.push(local[next]);
                    next += 1;
                }
                while stack
                    .last()
                    .is_some_and(|definition| definition.declaration.end <= at)
                {
                    stack.pop();
                }
                while merged.get(opaque).is_some_and(|span| span.end <= at) {
                    opaque += 1;
                }
                if noexpand == Some(at) {
                    noexpand = None;
                    continue;
                }
                if operands.contains(&at)
                    || doc.syntax.control_operands.contains(&at)
                    || merged.get(opaque).is_some_and(|span| span.contains(&at))
                {
                    continue;
                }
                if command.name == "noexpand" {
                    noexpand = Some(syntax::skip_space(&doc.text.source, command.span.end));
                    continue;
                }
                let caller = if let Some(definition) = stack.last() {
                    if !definition.body.contains(&at) {
                        continue;
                    }
                    let Some(node) = definition.node else {
                        continue;
                    };
                    node
                } else {
                    file
                };
                let Some(&callee) = index.by_name.get(&command.name) else {
                    continue;
                };
                let range = doc.text.range(command.span.start, command.span.end);
                index.edges.entry((caller, callee)).or_default().push(range);
                calls += 1;
                if calls > 20_000 {
                    return Err(budget());
                }
            }
        }
        Ok(index)
    }
    fn validate(&self, item: &CallHierarchyItem) -> Result<usize> {
        self.nodes
            .iter()
            .position(|current| {
                current.uri == item.uri
                    && current.name == item.name
                    && current.kind == item.kind
                    && current.range == item.range
                    && current.selection_range == item.selection_range
                    && current.data == item.data
            })
            .ok_or_else(modified)
    }
}
fn identity(item: &CallHierarchyItem) -> Result<Identity> {
    serde_json::from_value(
        item.data
            .clone()
            .ok_or_else(|| Error::invalid_params("Missing call hierarchy identity"))?,
    )
    .map_err(|_| Error::invalid_params("Invalid call hierarchy identity"))
}
pub(crate) fn prepare(
    ws: &Workspace,
    uri: &Url,
    position: Position,
) -> Result<Option<Vec<CallHierarchyItem>>> {
    let Some(name) = crate::features::command_target(ws, uri, position) else {
        return Ok(None);
    };
    let index = Index::new(ws, uri)?;
    Ok(index
        .by_name
        .get(name)
        .map(|&node| vec![index.nodes[node].clone()]))
}
pub(crate) fn incoming(
    ws: &Workspace,
    item: &CallHierarchyItem,
) -> Result<Vec<CallHierarchyIncomingCall>> {
    let identity = identity(item)?;
    let index = Index::new(ws, &identity.origin)?;
    let node = index.validate(item)?;
    Ok(index
        .edges
        .iter()
        .filter(|((_, callee), _)| *callee == node)
        .map(|((caller, _), ranges)| CallHierarchyIncomingCall {
            from: index.nodes[*caller].clone(),
            from_ranges: ranges.clone(),
        })
        .collect())
}
pub(crate) fn outgoing(
    ws: &Workspace,
    item: &CallHierarchyItem,
) -> Result<Vec<CallHierarchyOutgoingCall>> {
    let identity = identity(item)?;
    let index = Index::new(ws, &identity.origin)?;
    let node = index.validate(item)?;
    Ok(index
        .edges
        .iter()
        .filter(|((caller, _), _)| *caller == node)
        .map(|((_, callee), ranges)| CallHierarchyOutgoingCall {
            to: index.nodes[*callee].clone(),
            from_ranges: ranges.clone(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(source: &str) -> (Workspace, Url) {
        let uri = Url::parse("untitled:hierarchy.tex").unwrap();
        let mut ws = Workspace::default();
        ws.config.distribution.enabled = false;
        ws.open(uri.clone(), source.into(), 1);
        (ws, uri)
    }
    fn prepared(ws: &Workspace, uri: &Url, name: &str) -> CallHierarchyItem {
        let doc = &ws.documents[uri];
        let at = doc.text.source.rfind(&format!("\\{name}")).unwrap();
        prepare(ws, uri, doc.text.position(at + 1))
            .unwrap()
            .unwrap()
            .remove(0)
    }
    fn source_range<'a>(ws: &'a Workspace, uri: &Url, range: Range) -> &'a str {
        let text = &ws.documents[uri].text;
        &text.source[text.offset(range.start).unwrap()..text.offset(range.end).unwrap()]
    }

    #[test]
    fn macro_and_file_callers_group_repeated_ranges_and_support_outgoing_navigation() {
        let (ws, uri) = setup(
            "\\newcommand{\\leaf}{}\n\\newcommand{\\caller}{\\leaf \\leaf}\n\\caller \\leaf\n",
        );
        let leaf = prepared(&ws, &uri, "leaf");
        let callers = incoming(&ws, &leaf).unwrap();
        assert_eq!(callers.len(), 2, "{callers:?}");
        let macro_call = callers
            .iter()
            .find(|call| call.from.name == "\\caller")
            .unwrap();
        assert_eq!(macro_call.from_ranges.len(), 2);
        for range in &macro_call.from_ranges {
            assert_eq!(source_range(&ws, &uri, *range), "\\leaf");
            assert!(
                macro_call.from.range.start <= range.start
                    && range.end <= macro_call.from.range.end
            );
        }
        let calls = outgoing(&ws, &macro_call.from).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to, leaf);
        assert_eq!(calls[0].from_ranges, macro_call.from_ranges);
        let file = callers
            .iter()
            .find(|call| call.from.kind == SymbolKind::FILE)
            .unwrap();
        let top_level = outgoing(&ws, &file.from).unwrap();
        assert_eq!(top_level.len(), 2);
        assert!(top_level.iter().all(|call| call.from_ranges.len() == 1));
        assert!(outgoing(&ws, &leaf).unwrap().is_empty());
    }

    #[test]
    fn recursive_edges_and_utf16_crlf_ranges_are_finite_and_exact() {
        let (ws, uri) =
            setup("% 😀\r\n\\newcommand{\\recursive}{😀 \\recursive}\r\n😀 \\recursive\r\n");
        let node = prepared(&ws, &uri, "recursive");
        let calls = outgoing(&ws, &node).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].to, node);
        assert_eq!(
            source_range(&ws, &uri, calls[0].from_ranges[0]),
            "\\recursive"
        );
        assert!(
            node.range.start <= node.selection_range.start
                && node.selection_range.end <= node.range.end
        );
        assert_eq!(incoming(&ws, &node).unwrap().len(), 2);
    }

    #[test]
    fn comments_literals_operands_macro_arguments_and_nested_declarations_are_not_calls() {
        let source = r"\newcommand{\leaf}{}
\newcommand{\discard}[1]{}
\newcommand{\outer}{\newcommand{\inner}{\leaf}}
\newenvironment{example}{\leaf}{\leaf}
% \leaf
\verb|\leaf|
\begin{verbatim}
\leaf
\end{verbatim}
\string\leaf \meaning\leaf \let\copy\leaf
\noexpand\leaf
\discard{\leaf}
\leaf
";
        let (ws, uri) = setup(source);
        let leaf = prepared(&ws, &uri, "leaf");
        let calls = incoming(&ws, &leaf).unwrap();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0].from.kind, SymbolKind::FILE);
        assert_eq!(calls[0].from_ranges.len(), 1, "{calls:?}");
        assert_eq!(
            calls[0].from_ranges[0].start,
            ws.documents[&uri]
                .text
                .position(source.rfind("\\leaf").unwrap())
        );
        let outer = prepared(&ws, &uri, "outer");
        assert!(outgoing(&ws, &outer).unwrap().is_empty());
        let inner_position = ws.documents[&uri]
            .text
            .position(source.find("\\inner").unwrap() + 1);
        assert!(prepare(&ws, &uri, inner_position).unwrap().is_none());
    }

    #[test]
    fn supported_definition_grammars_keep_headers_and_default_arguments_out_of_edges() {
        for declaration in [
            r"\newcommand{\caller}[1][{\leaf}]{\leaf}",
            r"\NewDocumentCommand{\caller}{O{\leaf}}{\leaf}",
            r"\def\caller#1\STOP{\leaf}",
            r"\csdef{caller}{\leaf}",
        ] {
            let (ws, uri) = setup(&format!(
                "\\newcommand{{\\leaf}}{{}}\n{declaration}\n\\caller\n"
            ));
            let caller = prepared(&ws, &uri, "caller");
            let edges = outgoing(&ws, &caller).unwrap();
            assert_eq!(edges.len(), 1, "{declaration}: {edges:?}");
            assert_eq!(edges[0].to.name, "\\leaf");
            assert_eq!(edges[0].from_ranges.len(), 1);
        }
    }

    #[test]
    fn project_scopes_keep_independent_definitions_and_top_level_sources_separate() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::write(
            root.join("main.tex"),
            "\\documentclass{article}\n\\input{macros}\n\\input{child}\n\\leaf\n",
        )
        .unwrap();
        std::fs::write(root.join("macros.tex"), "\\newcommand{\\leaf}{}\n").unwrap();
        std::fs::write(
            root.join("child.tex"),
            "\\newcommand{\\caller}{\\leaf}\n\\caller\n",
        )
        .unwrap();
        std::fs::write(
            root.join("other.tex"),
            "\\documentclass{article}\n\\newcommand{\\leaf}{}\n\\leaf\n",
        )
        .unwrap();
        let mut ws = Workspace::default();
        ws.add_root(root.into());
        let main = Url::from_file_path(root.join("main.tex")).unwrap();
        let leaf = prepared(&ws, &main, "leaf");
        let callers = incoming(&ws, &leaf).unwrap();
        assert_eq!(callers.len(), 2, "{callers:?}");
        assert!(callers
            .iter()
            .all(|call| !call.from.uri.path().ends_with("other.tex")));
        assert!(callers
            .iter()
            .any(|call| call.from.uri.path().ends_with("child.tex")));
    }

    #[test]
    fn ambiguous_incomplete_and_builtin_commands_do_not_gain_invented_nodes() {
        let (ws, uri) = setup("\\newcommand{\\leaf}{}\n\\renewcommand{\\leaf}{}\n\\newcommand{\\caller}{\\leaf}\n\\leaf \\caller \\textbf{body}\n");
        let pos = ws.documents[&uri]
            .text
            .position(ws.documents[&uri].text.source.rfind("\\leaf").unwrap() + 1);
        assert!(prepare(&ws, &uri, pos).unwrap().is_none());
        let caller = prepared(&ws, &uri, "caller");
        assert!(outgoing(&ws, &caller).unwrap().is_empty());
        let (ws, uri) = setup("\\newcommand{\\unfinished}{body\n\\textbf{body}");
        for needle in ["\\unfinished", "\\textbf"] {
            let pos = ws.documents[&uri]
                .text
                .position(ws.documents[&uri].text.source.find(needle).unwrap() + 1);
            assert!(prepare(&ws, &uri, pos).unwrap().is_none());
        }
    }

    #[test]
    fn stale_or_modified_items_and_missing_identity_are_rejected() {
        let (mut ws, uri) = setup("\\newcommand{\\leaf}{}\n\\leaf\n");
        let node = prepared(&ws, &uri, "leaf");
        let mut forged = node.clone();
        forged.name = "\\other".into();
        assert_eq!(
            incoming(&ws, &forged).unwrap_err().code,
            ErrorCode::ServerError(-32801)
        );
        forged.data = None;
        assert_eq!(
            outgoing(&ws, &forged).unwrap_err().code,
            ErrorCode::InvalidParams
        );
        ws.open(
            uri.clone(),
            "\\newcommand{\\leaf}{}\n\\leaf\n\\leaf\n".into(),
            2,
        );
        assert_eq!(
            incoming(&ws, &node).unwrap_err().code,
            ErrorCode::ServerError(-32801)
        );
        let current = prepared(&ws, &uri, "leaf");
        assert_eq!(incoming(&ws, &current).unwrap()[0].from_ranges.len(), 2);
        ws.documents.remove(&uri);
        assert_eq!(
            outgoing(&ws, &current).unwrap_err().code,
            ErrorCode::ServerError(-32801)
        );
    }

    #[test]
    fn incomplete_indexes_and_oversized_sources_return_errors_instead_of_partial_graphs() {
        let (mut ws, uri) = setup("\\newcommand{\\leaf}{}\n\\leaf\n");
        let node = prepared(&ws, &uri, "leaf");
        let large = Url::parse("untitled:large.tex").unwrap();
        ws.documents.insert(
            large,
            Document::new(
                &Url::parse("untitled:large.tex").unwrap(),
                format!("%{}", "x".repeat(16 * 1024 * 1024)),
                Some(1),
            ),
        );
        assert_eq!(
            incoming(&ws, &node).unwrap_err().code,
            ErrorCode::ServerError(-32803)
        );

        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("main.tex"),
            "\\newcommand{\\leaf}{}\n\\leaf\n",
        )
        .unwrap();
        std::fs::write(directory.path().join("other.tex"), "Other source\n").unwrap();
        let mut ws = Workspace::default();
        ws.config.indexing.max_files = 1;
        ws.add_root(directory.path().into());
        let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        ws.open(uri.clone(), "\\newcommand{\\leaf}{}\n\\leaf\n".into(), 1);
        let error = prepare(&ws, &uri, Position::new(1, 2)).unwrap_err();
        assert!(error.message.contains("indexing is incomplete"));
    }
}
