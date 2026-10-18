use std::collections::BTreeSet;
use std::ops::Range as Span;

use tower_lsp::lsp_types::*;

use crate::{
    catalog::{COMMANDS, ENVIRONMENTS},
    syntax::{self, Command, Entry},
    workspace::{resolve_path, Document, Workspace},
};

#[derive(Clone, Copy, PartialEq)]
enum TargetKind {
    Label,
    Citation,
    Command,
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
    for entry in &doc.syntax.citations {
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
    if doc.bibtex || doc.syntax.is_ignored(offset.saturating_sub(1)) {
        return vec![];
    }
    if let Some((command, span)) = argument_context(doc, offset) {
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
                    .filter(|cmd| {
                        matches!(
                            cmd.name.as_str(),
                            "begin" | "newenvironment" | "renewenvironment"
                        )
                    })
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
    let use_snippets = workspace.snippets
        && !doc.text.source[command_end..]
            .trim_start()
            .starts_with(['{', '[', '*']);
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    // User definitions take precedence over built-in documentation and snippets.
    for entry in workspace
        .documents
        .values()
        .flat_map(|doc| &doc.syntax.definitions)
    {
        if entry.name.starts_with(prefix) && seen.insert(entry.name.clone()) {
            result.push(CompletionItem {
                label: entry.name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(entry.detail.clone()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: entry.name.clone(),
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
                    new_text: if use_snippets { snippet } else { name }.into(),
                })),
                ..CompletionItem::default()
            });
        }
    }
    result
}

pub fn definitions(workspace: &Workspace, uri: &Url, position: Position) -> Vec<Location> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let Some(offset) = doc.text.offset(position) else {
        return vec![];
    };
    if doc.syntax.is_ignored(offset) {
        return vec![];
    }
    // File inclusion commands jump to the file even when it is outside the index.
    for command in &doc.syntax.commands {
        if let Some(arg) = &command.argument {
            if arg.span.start <= offset && offset <= arg.span.end {
                let extension = match command.name.as_str() {
                    "input" | "include" | "subfile" => "tex",
                    "bibliography" | "addbibresource" => "bib",
                    _ => continue,
                };
                return arg
                    .value
                    .split(',')
                    .filter_map(|value| resolve_path(uri, value.trim(), extension))
                    .map(|uri| Location {
                        uri,
                        range: Range::default(),
                    })
                    .collect();
            }
        }
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
