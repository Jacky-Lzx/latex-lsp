use crate::{
    features, syntax,
    workspace::{resolve_file_path, resolve_path, root_directive, Document, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range as Span,
    path::PathBuf,
};
use tower_lsp::lsp_types::*;

pub fn workspace_symbols(workspace: &Workspace, query: &str) -> Vec<SymbolInformation> {
    let query = query.trim().to_lowercase();
    let mut result = Vec::new();
    for (uri, doc) in &workspace.documents {
        for symbol in features::flat_symbols(doc, uri) {
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
                    continue;
                }
                3
            };
            result.push((score, symbol));
        }
    }
    result.sort_by(|(a_score, a), (b_score, b)| {
        (a_score, &a.name, &a.location.uri, a.location.range.start).cmp(&(
            b_score,
            &b.name,
            &b.location.uri,
            b.location.range.start,
        ))
    });
    result
        .into_iter()
        .take(256)
        .map(|(_, symbol)| symbol)
        .collect()
}

fn fold(doc: &Document, span: Span<usize>, kind: FoldingRangeKind) -> Option<FoldingRange> {
    let start = doc.text.position(span.start);
    let end = doc.text.position(span.end);
    let end_line = if end.character == 0 {
        end.line.saturating_sub(1)
    } else {
        end.line
    };
    (end_line > start.line).then_some(FoldingRange {
        start_line: start.line,
        end_line,
        kind: Some(kind),
        ..Default::default()
    })
}

fn heading_folds(symbols: &[DocumentSymbol], result: &mut Vec<FoldingRange>) {
    for symbol in symbols {
        if symbol.kind == SymbolKind::NAMESPACE {
            let range = symbol.range;
            let end = range
                .end
                .line
                .saturating_sub(u32::from(range.end.character == 0));
            if end > range.start.line {
                result.push(FoldingRange {
                    start_line: range.start.line,
                    end_line: end,
                    kind: Some(FoldingRangeKind::Region),
                    ..Default::default()
                });
            }
        }
        if let Some(children) = &symbol.children {
            heading_folds(children, result);
        }
    }
}

pub fn folding_ranges(doc: &Document, limit: Option<u32>) -> Vec<FoldingRange> {
    let mut result = Vec::new();
    heading_folds(&features::symbols(doc), &mut result);
    let mut environments = Vec::new();
    for command in &doc.syntax.commands {
        let Some(argument) = &command.argument else {
            continue;
        };
        if command.name == "begin" {
            if let Some(raw) = doc
                .syntax
                .ignored
                .iter()
                .find(|span| span.start == argument.outer_span.end)
            {
                result.extend(fold(
                    doc,
                    command.span.start..raw.end,
                    FoldingRangeKind::Region,
                ));
            } else {
                environments.push((argument.value.as_str(), command.span.start));
            }
        } else if command.name == "end" {
            if let Some(index) = environments
                .iter()
                .rposition(|(name, _)| *name == argument.value)
            {
                result.extend(fold(
                    doc,
                    environments[index].1..argument.outer_span.end,
                    FoldingRangeKind::Region,
                ));
                environments.truncate(index);
            }
        }
    }
    let source = &doc.text.source;
    let bytes = source.as_bytes();
    let mut braces = Vec::new();
    let mut ignored = doc.syntax.ignored.iter().peekable();
    let mut i = 0;
    while i < bytes.len() {
        while ignored.peek().is_some_and(|span| span.end <= i) {
            ignored.next();
        }
        if let Some(span) = ignored
            .peek()
            .filter(|span| span.start <= i && i < span.end)
        {
            i = span.end;
            continue;
        }
        match bytes[i] {
            b'\\' => {
                i += 1;
                if i < bytes.len() {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
            }
            b'%' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => {
                braces.push(i);
                i += 1;
            }
            b'}' => {
                if let Some(start) = braces.pop() {
                    result.extend(fold(doc, start..i + 1, FoldingRangeKind::Region));
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    let comments: Vec<_> = doc
        .syntax
        .ignored
        .iter()
        .filter_map(|span| {
            if !source[span.clone()].starts_with('%') {
                return None;
            }
            let position = doc.text.position(span.start);
            let start = doc.text.offset(Position::new(position.line, 0))?;
            source[start..span.start]
                .trim()
                .is_empty()
                .then_some(position.line)
        })
        .collect();
    let mut run: Option<(u32, u32)> = None;
    for line in comments.into_iter().chain(std::iter::once(u32::MAX)) {
        if let Some((start, end)) = run {
            if line == end + 1 {
                run = Some((start, line));
                continue;
            }
            if end > start {
                result.push(FoldingRange {
                    start_line: start,
                    end_line: end,
                    kind: Some(FoldingRangeKind::Comment),
                    ..Default::default()
                });
            }
        }
        run = Some((line, line));
    }
    result.sort_by_key(|range| (range.start_line, std::cmp::Reverse(range.end_line)));
    result.dedup_by(|a, b| a.start_line == b.start_line && a.end_line == b.end_line);
    result.truncate(limit.unwrap_or(u32::MAX) as usize);
    result
}

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
        return crate::graphics::resolve_in(root, name, extensions, graphics);
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
    if !workspace.documents.contains_key(uri) {
        return vec![];
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
    let graphics = if doc
        .syntax
        .commands
        .iter()
        .any(|command| command.name == "includegraphics")
    {
        crate::graphics::directories_in(workspace, uri, root)
    } else {
        vec![]
    };
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
    let inputs = crate::source_inputs::Index::new(workspace, root);
    for call in inputs.calls.get(uri).into_iter().flatten() {
        if let (Some(argument), Some(target)) = (
            call.command.argument.as_ref(),
            crate::source_inputs::target(uri, root, call),
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
        .any(|command| crate::source_inputs::editable_file_command(&command.name))
        .then(|| {
            let mut scope = workspace.project_documents(root);
            scope.insert(uri.clone());
            crate::source_inputs::editable_file_positions(workspace, &scope)
        });
    for command in doc
        .syntax
        .commands
        .iter()
        .filter(|command| {
            !crate::source_inputs::editable_file_command(&command.name)
                || editable_sources
                    .as_ref()
                    .and_then(|sources| sources.get(uri))
                    .is_some_and(|positions| positions.contains(&command.span.start))
        })
        .chain(
            inputs
                .loaders
                .get(uri)
                .into_iter()
                .flatten()
                .chain(inputs.forwarders.get(uri).into_iter().flatten())
                .chain(inputs.resources.get(uri).into_iter().flatten())
                .chain(inputs.selectors.get(uri).into_iter().flatten())
                .chain(inputs.checks.get(uri).into_iter().flatten())
                .filter(|call| call.certain)
                .map(|call| &call.command),
        )
    {
        if syntax::code_input_command(&command.name)
            && !crate::code_inputs::supported(workspace, uri, root, command)
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
        let names = if let Some(spans) = syntax::file_list_spans(&doc.text.source, command) {
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
            let target = if extensions.is_empty() {
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
    let mut seen = BTreeSet::new();
    links.retain(|link| seen.insert((link.range.start, link.range.end)));
    links
}
