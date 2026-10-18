use std::collections::BTreeSet;
use std::ops::Range as Span;

use tower_lsp::lsp_types::*;

use crate::{
    catalog::{COMMANDS, ENVIRONMENTS},
    syntax::{self, Command, Entry},
    workspace::{Document, Workspace},
};

#[derive(Clone, Copy, PartialEq)]
enum TargetKind {
    Label,
    Citation,
    Command,
    Environment,
}

struct Target<'a> {
    kind: TargetKind,
    name: &'a str,
    span: Span<usize>,
}

fn target(doc: &Document, offset: usize) -> Option<Target<'_>> {
    if doc.syntax.is_ignored(offset) {
        return None;
    }
    for command in &doc.syntax.commands {
        if let Some(arg) = &command.argument {
            let kind = if syntax::is_reference(&command.name) || command.name == "label" {
                Some(TargetKind::Label)
            } else if syntax::is_citation(&command.name) || command.name == "bibitem" {
                Some(TargetKind::Citation)
            } else if matches!(command.name.as_str(), "begin" | "end")
                || syntax::declares_environment(&command.name)
            {
                Some(TargetKind::Environment)
            } else {
                None
            };
            if let Some(kind) = kind {
                if arg.span.start <= offset && offset <= arg.span.end {
                    let local = offset - arg.span.start;
                    let start = arg.value[..local].rfind(',').map(|i| i + 1).unwrap_or(0);
                    let end = arg.value[local..]
                        .find(',')
                        .map(|i| local + i)
                        .unwrap_or(arg.value.len());
                    let span = syntax::trimmed_span(
                        &doc.text.source,
                        arg.span.start + start..arg.span.start + end,
                    );
                    return Some(Target {
                        kind,
                        name: &doc.text.source[span.clone()],
                        span,
                    });
                }
            }
        }
        if command.span.start <= offset && offset <= command.span.end {
            return Some(Target {
                kind: TargetKind::Command,
                name: &command.name,
                span: command.span.clone(),
            });
        }
    }
    for entry in doc
        .syntax
        .citations
        .iter()
        .chain(&doc.syntax.citation_references)
    {
        if entry.span.start <= offset && offset <= entry.span.end {
            return Some(Target {
                kind: TargetKind::Citation,
                name: &entry.name,
                span: entry.span.clone(),
            });
        }
    }
    None
}

fn entries(doc: &Document, kind: TargetKind) -> &[Entry] {
    match kind {
        TargetKind::Label => &doc.syntax.labels,
        TargetKind::Citation => &doc.syntax.citations,
        TargetKind::Command => &doc.syntax.definitions,
        TargetKind::Environment => &doc.syntax.environments,
    }
}

fn rename_target<'a>(
    workspace: &'a Workspace,
    uri: &Url,
    position: Position,
) -> tower_lsp::jsonrpc::Result<Option<Target<'a>>> {
    let Some(doc) = workspace.documents.get(uri) else {
        return Ok(None);
    };
    let Some(target) = doc
        .text
        .offset(position)
        .and_then(|offset| target(doc, offset))
    else {
        return Ok(None);
    };
    let declarations = workspace
        .documents
        .values()
        .flat_map(|doc| entries(doc, target.kind))
        .filter(|entry| entry.name == target.name)
        .count();
    if declarations == 0 {
        return Ok(None);
    }
    if declarations != 1 {
        return Err(tower_lsp::jsonrpc::Error::invalid_params(
            "Rename is ambiguous: the name has multiple definitions in this workspace",
        ));
    }
    Ok(Some(target))
}

pub fn prepare_rename(
    workspace: &Workspace,
    uri: &Url,
    position: Position,
) -> tower_lsp::jsonrpc::Result<Option<PrepareRenameResponse>> {
    let Some(target) = rename_target(workspace, uri, position)? else {
        return Ok(None);
    };
    let doc = &workspace.documents[uri];
    let start = target.span.start + usize::from(target.kind == TargetKind::Command);
    Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
        range: doc.text.range(start, target.span.end),
        placeholder: target.name.into(),
    }))
}

pub fn rename(
    workspace: &Workspace,
    uri: &Url,
    position: Position,
    new_name: &str,
) -> tower_lsp::jsonrpc::Result<Option<WorkspaceEdit>> {
    let Some(target) = rename_target(workspace, uri, position)? else {
        return Ok(None);
    };
    let new_name = if target.kind == TargetKind::Command {
        new_name.strip_prefix('\\').unwrap_or(new_name)
    } else {
        new_name
    };
    let valid = !new_name.is_empty()
        && !(target.kind == TargetKind::Citation && new_name == "*")
        && if target.kind == TargetKind::Command {
            new_name
                .bytes()
                .all(|b| b.is_ascii_alphabetic() || b == b'@')
        } else {
            !new_name.chars().any(|ch| {
                ch.is_whitespace()
                    || matches!(
                        ch,
                        '\\' | '{'
                            | '}'
                            | ','
                            | '%'
                            | '#'
                            | '$'
                            | '&'
                            | '^'
                            | '~'
                            | '"'
                            | '('
                            | ')'
                            | '='
                    )
            })
        };
    if !valid {
        return Err(tower_lsp::jsonrpc::Error::invalid_params(
            "Invalid name: use a single TeX command name or a reference key without separators",
        ));
    }
    if new_name == target.name {
        return Ok(Some(WorkspaceEdit::default()));
    }
    let conflicts =
        workspace.documents.values().any(|doc| {
            entries(doc, target.kind)
                .iter()
                .any(|entry| entry.name == new_name)
                || target.kind == TargetKind::Environment
                    && doc.syntax.definitions.iter().any(|entry| {
                        entry.name == new_name || entry.name == format!("end{new_name}")
                    })
                || target.kind == TargetKind::Command
                    && doc.syntax.environments.iter().any(|entry| {
                        entry.name == new_name || format!("end{}", entry.name) == new_name
                    })
        });
    let builtin = match target.kind {
        TargetKind::Command | TargetKind::Environment => {
            COMMANDS.iter().any(|(name, _, _)| *name == new_name)
                || ENVIRONMENTS
                    .iter()
                    .any(|name| *name == new_name || format!("end{name}") == new_name)
        }
        _ => false,
    };
    if conflicts || builtin {
        return Err(tower_lsp::jsonrpc::Error::invalid_params(format!(
            "The name `{new_name}` is already defined"
        )));
    }
    let replacement = if target.kind == TargetKind::Command {
        format!("\\{new_name}")
    } else {
        new_name.into()
    };
    let mut changes: std::collections::BTreeMap<Url, Vec<TextEdit>> =
        std::collections::BTreeMap::new();
    for location in references(workspace, uri, position, true) {
        changes.entry(location.uri).or_default().push(TextEdit {
            range: location.range,
            new_text: replacement.clone(),
        });
    }
    for edits in changes.values_mut() {
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        edits.dedup_by(|a, b| a.range == b.range);
    }
    Ok(Some(workspace_edit(workspace, changes)))
}

pub fn workspace_edit(
    workspace: &Workspace,
    changes: std::collections::BTreeMap<Url, Vec<TextEdit>>,
) -> WorkspaceEdit {
    if workspace.versioned_edits {
        WorkspaceEdit {
            document_changes: Some(DocumentChanges::Edits(
                changes
                    .into_iter()
                    .map(|(uri, edits)| {
                        let version = workspace.documents.get(&uri).and_then(|doc| doc.version);
                        TextDocumentEdit {
                            text_document: OptionalVersionedTextDocumentIdentifier { uri, version },
                            edits: edits.into_iter().map(OneOf::Left).collect(),
                        }
                    })
                    .collect(),
            )),
            ..Default::default()
        }
    } else {
        WorkspaceEdit {
            changes: Some(changes.into_iter().collect()),
            ..Default::default()
        }
    }
}

/// Works for incomplete groups too, so typing `\ref{` can offer completions.
fn argument_context(doc: &Document, offset: usize) -> Option<(&Command, Span<usize>)> {
    let source = &doc.text.source;
    let command = doc
        .syntax
        .commands
        .iter()
        .rev()
        .find(|cmd| cmd.span.end <= offset)?;
    let mut tail = &source[command.span.end..offset];
    tail = tail.strip_prefix('*').unwrap_or(tail).trim_start();
    while tail.starts_with('[') {
        tail = tail.get(tail.find(']')? + 1..)?.trim_start();
    }
    let value = tail.strip_prefix('{')?;
    if value.contains(['{', '}', '\n', '%']) {
        return None;
    }
    let key = value.rsplit(',').next()?.trim_start();
    Some((command, offset - key.len()..offset))
}

pub fn completion(workspace: &Workspace, uri: &Url, position: Position) -> Vec<CompletionItem> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let Some(offset) = doc.text.offset(position) else {
        return vec![];
    };
    if doc.bibtex {
        return bibliography_completion(workspace, doc, offset);
    }
    if doc.syntax.is_ignored(offset.saturating_sub(1)) {
        return vec![];
    }
    if let Some((command, span)) = argument_context(doc, offset) {
        if syntax::file_extensions(&command.name).is_some() {
            return crate::path_completion::complete(workspace, uri, &command.name, span);
        }
        let prefix = &doc.text.source[span.clone()];
        // Replace the rest of this key too when completing inside an existing
        // reference/environment; separators and following arguments stay intact.
        let suffix_length = doc.text.source[offset..]
            .chars()
            .take_while(|ch| !ch.is_whitespace() && !matches!(ch, '{' | '}' | ',' | '\\' | '%'))
            .map(char::len_utf8)
            .sum::<usize>();
        let replacement_range = doc.text.range(span.start, offset + suffix_length);
        let kind = if syntax::is_reference(&command.name) {
            Some(TargetKind::Label)
        } else if syntax::is_citation(&command.name) {
            Some(TargetKind::Citation)
        } else {
            None
        };
        let mut candidates = Vec::new();
        if let Some(kind) = kind {
            for (entry_uri, entry_doc) in &workspace.documents {
                for entry in entries(entry_doc, kind) {
                    candidates.push((
                        entry.name.clone(),
                        format!("{} — {}", entry.detail, entry_uri.path()),
                    ));
                }
            }
        } else if matches!(command.name.as_str(), "begin" | "end") {
            candidates.extend(
                ENVIRONMENTS
                    .iter()
                    .map(|&name| (name.into(), "LaTeX environment".into())),
            );
            candidates.extend(
                workspace
                    .documents
                    .values()
                    .flat_map(|doc| &doc.syntax.commands)
                    .filter(|cmd| cmd.name == "begin" || syntax::declares_environment(&cmd.name))
                    .filter_map(|cmd| cmd.argument.as_ref())
                    .map(|arg| (arg.value.clone(), "Document environment".into())),
            );
        } else {
            return vec![];
        }
        let mut seen = BTreeSet::new();
        return candidates
            .into_iter()
            .filter(|(name, _)| name.starts_with(prefix) && seen.insert(name.clone()))
            .map(|(name, detail)| CompletionItem {
                label: name.clone(),
                detail: Some(detail),
                kind: Some(CompletionItemKind::REFERENCE),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: replacement_range,
                    new_text: name,
                })),
                ..CompletionItem::default()
            })
            .collect();
    }
    let line_start = doc.text.source[..offset]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let before = &doc.text.source[line_start..offset];
    let Some(slash) = before.rfind('\\') else {
        return vec![];
    };
    // An escaped backslash is a line break, not a command prefix.
    let slash_run = before[..=slash]
        .bytes()
        .rev()
        .take_while(|&b| b == b'\\')
        .count();
    if slash_run % 2 == 0 {
        return vec![];
    }
    let prefix = &before[slash + 1..];
    if !prefix.bytes().all(|b| b.is_ascii_alphabetic() || b == b'@') {
        return vec![];
    }
    let command_end = offset
        + doc.text.source[offset..]
            .bytes()
            .take_while(|b| b.is_ascii_alphabetic() || *b == b'@')
            .count();
    let range = doc.text.range(line_start + slash + 1, command_end);
    // A command already followed by arguments only needs its name replaced.
    let after_command = syntax::skip_space(&doc.text.source, command_end);
    let use_snippets =
        workspace.snippets && !doc.text.source[after_command..].starts_with(['{', '[', '*']);
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    // User definitions take precedence over built-in documentation and snippets.
    for entry in workspace
        .documents
        .values()
        .flat_map(|doc| &doc.syntax.definitions)
    {
        if entry.name.starts_with(prefix) && seen.insert(entry.name.clone()) {
            let signature = crate::signature::completion(workspace, &entry.name);
            let snippet = signature
                .as_ref()
                .and_then(|(_, snippet)| snippet.as_ref())
                .filter(|_| use_snippets);
            result.push(CompletionItem {
                label: entry.name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(
                    signature
                        .as_ref()
                        .map(|(label, _)| label.clone())
                        .unwrap_or_else(|| entry.detail.clone()),
                ),
                documentation: Some(Documentation::String(entry.detail.clone())),
                insert_text_format: Some(if snippet.is_some() {
                    InsertTextFormat::SNIPPET
                } else {
                    InsertTextFormat::PLAIN_TEXT
                }),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: snippet.cloned().unwrap_or_else(|| entry.name.clone()),
                })),
                ..CompletionItem::default()
            });
        }
    }
    for &(name, snippet, documentation) in COMMANDS {
        if name.starts_with(prefix) && seen.insert(name.into()) {
            result.push(CompletionItem {
                label: name.into(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(format!("\\{name}")),
                documentation: Some(Documentation::String(documentation.into())),
                insert_text_format: Some(if use_snippets {
                    InsertTextFormat::SNIPPET
                } else {
                    InsertTextFormat::PLAIN_TEXT
                }),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    // Literal TeX backslashes must survive the snippet parser,
                    // including the one immediately before a placeholder.
                    new_text: if use_snippets {
                        snippet.replace('\\', "\\\\")
                    } else {
                        name.into()
                    },
                })),
                ..CompletionItem::default()
            });
        }
    }
    result
}

fn bibliography_completion(
    workspace: &Workspace,
    doc: &Document,
    offset: usize,
) -> Vec<CompletionItem> {
    let Some(field) = doc
        .syntax
        .citation_fields
        .iter()
        .find(|field| field.argument.span.start <= offset && offset <= field.argument.span.end)
    else {
        return vec![];
    };
    let value = &doc.text.source[field.argument.span.start..offset];
    let key = if matches!(field.name.as_str(), "xdata" | "related" | "entryset") {
        value.rsplit(',').next().unwrap_or("")
    } else {
        value
    };
    let key = key.trim_start();
    let suffix = &doc.text.source[offset..field.argument.span.end];
    let remaining = suffix.find(',').unwrap_or(suffix.len());
    let remaining = suffix[..remaining].trim_end().len();
    let range = doc.text.range(offset - key.len(), offset + remaining);
    let mut seen = BTreeSet::new();
    workspace
        .documents
        .iter()
        .flat_map(|(uri, doc)| doc.syntax.citations.iter().map(move |entry| (uri, entry)))
        .filter(|(_, entry)| entry.name.starts_with(key) && seen.insert(entry.name.clone()))
        .map(|(uri, entry)| CompletionItem {
            label: entry.name.clone(),
            detail: Some(format!("{} — {}", entry.detail, uri.path())),
            kind: Some(CompletionItemKind::REFERENCE),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: entry.name.clone(),
            })),
            ..Default::default()
        })
        .collect()
}

pub fn definitions(workspace: &Workspace, uri: &Url, position: Position) -> Vec<Location> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let Some(offset) = doc.text.offset(position) else {
        return vec![];
    };
    for link in crate::navigation::document_links(workspace, uri) {
        if link.range.start <= position && position <= link.range.end {
            if let Some(uri) = link.target.filter(|uri| uri.scheme() == "file") {
                return vec![Location {
                    uri,
                    range: Range::default(),
                }];
            }
        }
    }
    if doc.syntax.is_ignored(offset) {
        return vec![];
    }
    let Some(target) = target(doc, offset) else {
        return vec![];
    };
    workspace
        .documents
        .iter()
        .flat_map(|(uri, doc)| {
            entries(doc, target.kind)
                .iter()
                .filter(|entry| entry.name == target.name)
                .map(|entry| Location {
                    uri: uri.clone(),
                    range: doc.text.range(entry.span.start, entry.span.end),
                })
        })
        .collect()
}

pub fn references(
    workspace: &Workspace,
    uri: &Url,
    position: Position,
    include_declaration: bool,
) -> Vec<Location> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let Some(target) = doc
        .text
        .offset(position)
        .and_then(|offset| target(doc, offset))
    else {
        return vec![];
    };
    let mut result = Vec::new();
    for (uri, doc) in &workspace.documents {
        if target.kind == TargetKind::Citation {
            result.extend(
                doc.syntax
                    .citation_references
                    .iter()
                    .filter(|entry| entry.name == target.name)
                    .map(|entry| Location {
                        uri: uri.clone(),
                        range: doc.text.range(entry.span.start, entry.span.end),
                    }),
            );
        }
        if include_declaration {
            result.extend(
                entries(doc, target.kind)
                    .iter()
                    .filter(|entry| entry.name == target.name)
                    .map(|entry| Location {
                        uri: uri.clone(),
                        range: doc.text.range(entry.span.start, entry.span.end),
                    }),
            );
        }
        for command in &doc.syntax.commands {
            if target.kind == TargetKind::Command {
                if command.name == target.name
                    && !doc
                        .syntax
                        .definitions
                        .iter()
                        .any(|entry| entry.span == command.span)
                {
                    // Macro names inside their declaration group are indexed as commands too.
                    let declaration = doc.syntax.definitions.iter().any(|entry| {
                        entry.span.start == command.span.start && entry.name == command.name
                    });
                    if !declaration {
                        result.push(Location {
                            uri: uri.clone(),
                            range: doc.text.range(command.span.start, command.span.end),
                        });
                    }
                }
                continue;
            }
            let matches = match target.kind {
                TargetKind::Label => syntax::is_reference(&command.name),
                TargetKind::Citation => syntax::is_citation(&command.name),
                TargetKind::Command => false,
                TargetKind::Environment => matches!(command.name.as_str(), "begin" | "end"),
            };
            if !matches {
                continue;
            }
            if let Some(arg) = &command.argument {
                let mut start = arg.span.start;
                for key in arg.value.split(',') {
                    if key.trim() == target.name {
                        let span = syntax::trimmed_span(&doc.text.source, start..start + key.len());
                        result.push(Location {
                            uri: uri.clone(),
                            range: doc.text.range(span.start, span.end),
                        });
                    }
                    start += key.len() + 1;
                }
            }
        }
    }
    result
}

pub fn hover(workspace: &Workspace, uri: &Url, position: Position) -> Option<Hover> {
    let doc = workspace.documents.get(uri)?;
    let target = target(doc, doc.text.offset(position)?)?;
    let mut value = None;
    for (entry_uri, entry_doc) in &workspace.documents {
        if let Some(entry) = entries(entry_doc, target.kind)
            .iter()
            .find(|entry| entry.name == target.name)
        {
            value = Some(format!(
                "{}\n\n{}\n{}:{}",
                entry.name,
                entry.detail,
                entry_uri.path(),
                entry_doc.text.position(entry.span.start).line + 1
            ));
            break;
        }
    }
    if value.is_none() && target.kind == TargetKind::Command {
        value = COMMANDS
            .iter()
            .find(|(name, _, _)| *name == target.name)
            .map(|(name, _, docs)| format!("\\{name}\n\n{docs}"));
    }
    if value.is_none()
        && target.kind == TargetKind::Environment
        && ENVIRONMENTS.contains(&target.name)
    {
        value = Some(format!(
            "\\begin{{{}}} ... \\end{{{}}}\n\nLaTeX environment",
            target.name, target.name
        ));
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::PlainText,
            value: value?,
        }),
        range: Some(doc.text.range(target.span.start, target.span.end)),
    })
}

pub fn diagnostics(doc: &Document) -> Vec<Diagnostic> {
    let mut diagnostics = doc.syntax.diagnostics.clone();
    for entries in [&doc.syntax.labels, &doc.syntax.citations] {
        let mut seen = BTreeSet::new();
        for entry in entries {
            if !seen.insert(&entry.name) {
                diagnostics.push(syntax::diagnostic(
                    &doc.text,
                    entry.span.clone(),
                    format!("Duplicate key `{}`", entry.name),
                ));
            }
        }
    }
    diagnostics.sort_by_key(|d| (d.range.start.line, d.range.start.character));
    diagnostics
}

#[allow(deprecated)]
pub fn symbols(doc: &Document) -> Vec<DocumentSymbol> {
    if doc.bibtex {
        return doc
            .syntax
            .citations
            .iter()
            .map(|entry| DocumentSymbol {
                name: entry.name.clone(),
                detail: Some(entry.detail.clone()),
                kind: SymbolKind::KEY,
                tags: None,
                deprecated: None,
                range: doc.text.range(entry.span.start, entry.span.end),
                selection_range: doc.text.range(entry.span.start, entry.span.end),
                children: None,
            })
            .collect();
    }
    let headings: Vec<_> = doc
        .syntax
        .commands
        .iter()
        .filter_map(|command| {
            Some((
                syntax::section_level(&command.name)?,
                command,
                command.argument.as_ref()?,
            ))
        })
        .collect();
    let mut result = Vec::new();
    for (index, (level, command, argument)) in headings.iter().enumerate() {
        let end = headings[index + 1..]
            .iter()
            .find(|(next_level, _, _)| next_level <= level)
            .map(|(_, command, _)| command.span.start)
            .unwrap_or(doc.text.source.len());
        result.push(DocumentSymbol {
            name: argument.value.clone(),
            detail: Some(command.name.clone()),
            kind: SymbolKind::NAMESPACE,
            tags: None,
            deprecated: None,
            range: doc.text.range(command.span.start, end),
            selection_range: doc.text.range(argument.span.start, argument.span.end),
            children: None,
        });
    }
    for (entries, kind) in [
        (&doc.syntax.labels, SymbolKind::KEY),
        (&doc.syntax.definitions, SymbolKind::FUNCTION),
        (&doc.syntax.environments, SymbolKind::CLASS),
    ] {
        result.extend(entries.iter().map(|entry| DocumentSymbol {
            name: entry.name.clone(),
            detail: Some(entry.detail.clone()),
            kind,
            tags: None,
            deprecated: None,
            range: doc.text.range(entry.span.start, entry.span.end),
            selection_range: doc.text.range(entry.span.start, entry.span.end),
            children: None,
        }));
    }
    result.sort_by_key(|symbol| (symbol.range.start.line, symbol.range.start.character));
    let mut nested = Vec::new();
    for symbol in result {
        insert_symbol(&mut nested, symbol);
    }
    nested
}

fn insert_symbol(symbols: &mut Vec<DocumentSymbol>, symbol: DocumentSymbol) {
    if let Some(parent) = symbols.last_mut() {
        if parent.kind == SymbolKind::NAMESPACE
            && parent.range.start <= symbol.range.start
            && symbol.range.end <= parent.range.end
        {
            insert_symbol(parent.children.get_or_insert_with(Vec::new), symbol);
            return;
        }
    }
    symbols.push(symbol);
}

/// Older clients require SymbolInformation instead of hierarchical DocumentSymbol.
pub fn flat_symbols(doc: &Document, uri: &Url) -> Vec<SymbolInformation> {
    let mut result = Vec::new();
    flatten_symbols(&symbols(doc), uri, None, &mut result);
    result
}

#[allow(deprecated)]
fn flatten_symbols(
    symbols: &[DocumentSymbol],
    uri: &Url,
    container: Option<&str>,
    result: &mut Vec<SymbolInformation>,
) {
    for symbol in symbols {
        result.push(SymbolInformation {
            name: symbol.name.clone(),
            kind: symbol.kind,
            tags: symbol.tags.clone(),
            deprecated: None,
            location: Location {
                uri: uri.clone(),
                range: symbol.selection_range,
            },
            container_name: container.map(str::to_owned),
        });
        if let Some(children) = &symbol.children {
            flatten_symbols(children, uri, Some(&symbol.name), result);
        }
    }
}
