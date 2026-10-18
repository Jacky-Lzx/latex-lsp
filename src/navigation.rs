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
                .find(|span| span.start == argument.span.end + 1)
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
                    environments[index].1..argument.span.end + 1,
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

fn file_link(
    uri: &Url,
    name: &str,
    extensions: &[&str],
    graphics: &[PathBuf],
    root: &Url,
    command: &str,
) -> Option<Url> {
    for origin in std::iter::once(root).chain(std::iter::once(uri).filter(|uri| *uri != root)) {
        if let Some(target) = resolve_file_path(origin, command, name, extensions) {
            return Some(target);
        }
    }
    for directory in graphics {
        for extension in extensions {
            if let Some(target) = crate::graphics::resolve(directory, name, extension) {
                return Some(target);
            }
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
    for command in &doc.syntax.commands {
        let Some(argument) = &command.argument else {
            continue;
        };
        let extensions = match syntax::inferred_file_extensions(&command.name) {
            Some(extensions) => extensions,
            None if matches!(command.name.as_str(), "url" | "href") => &[],
            None => continue,
        };
        let multiple = syntax::file_list(&command.name);
        let mut start = argument.span.start;
        for name in if multiple {
            argument.value.split(',').collect::<Vec<_>>()
        } else {
            vec![argument.value.as_str()]
        } {
            let span = syntax::trimmed_span(&doc.text.source, start..start + name.len());
            let value = unescape(name.trim());
            let target = if extensions.is_empty() {
                Url::parse(&value).ok().filter(|uri| {
                    matches!(uri.scheme(), "http" | "https" | "mailto" | "file" | "ftp")
                })
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
            start += name.len() + 1;
        }
    }
    let mut seen = BTreeSet::new();
    links.retain(|link| seen.insert((link.range.start, link.range.end)));
    links
}
