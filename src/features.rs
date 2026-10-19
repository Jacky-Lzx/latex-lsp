use std::collections::BTreeSet;
use std::ops::Range as Span;

use tower_lsp::lsp_types::*;

use crate::{
    catalog::{COMMANDS, ENVIRONMENTS},
    syntax::{self, Command, Entry},
    workspace::{Document, DocumentKind, Workspace},
};

#[derive(Clone, Copy, PartialEq)]
enum TargetKind {
    Label,
    Anchor,
    Citation,
    Command,
    Environment,
    Counter,
    Conditional,
    Toggle,
}

impl TargetKind {
    fn generated_kind(self) -> Option<crate::generated::Kind> {
        match self {
            Self::Environment => Some(crate::generated::Kind::Environment),
            Self::Counter => Some(crate::generated::Kind::Counter),
            Self::Conditional => Some(crate::generated::Kind::Conditional),
            Self::Toggle => Some(crate::generated::Kind::Toggle),
            _ => None,
        }
    }
}

struct Target<'a> {
    kind: TargetKind,
    name: &'a str,
    span: Span<usize>,
    control_word: bool,
}

fn raw_target(doc: &Document, offset: usize) -> Option<Target<'_>> {
    if doc.syntax.is_ignored(offset) {
        return None;
    }
    if let Some(entry) = doc
        .syntax
        .anchors
        .iter()
        .chain(&doc.syntax.anchor_references)
        .find(|entry| entry.span.start <= offset && offset <= entry.span.end)
    {
        return Some(Target {
            kind: TargetKind::Anchor,
            name: &entry.name,
            span: entry.span.clone(),
            control_word: false,
        });
    }
    for command in &doc.syntax.commands {
        if let Some(argument) = syntax::environment_copy_source(&doc.text.source, command) {
            if argument.span.start <= offset && offset <= argument.span.end {
                if !syntax::is_static_key(&argument.value) {
                    return None;
                }
                return Some(Target {
                    kind: TargetKind::Environment,
                    name: &doc.text.source[argument.span.clone()],
                    span: argument.span,
                    control_word: false,
                });
            }
        }
        if let Some(span) = crate::named_commands::spans(doc, command)
            .into_iter()
            .find(|span| span.start <= offset && offset <= span.end)
        {
            return Some(Target {
                kind: TargetKind::Command,
                name: &doc.text.source[span.clone()],
                span,
                control_word: false,
            });
        }
        if let Some(argument) = syntax::separate_label_argument(&doc.text.source, command) {
            if argument.span.start <= offset && offset <= argument.span.end {
                if !syntax::is_static_key(&argument.value) {
                    return None;
                }
                return Some(Target {
                    kind: TargetKind::Label,
                    name: &doc.text.source[argument.span.clone()],
                    span: argument.span,
                    control_word: false,
                });
            }
        }
        if let Some(arg) = &command.argument {
            let kind = if syntax::is_reference(&command.name) || command.name == "label" {
                Some(TargetKind::Label)
            } else if syntax::is_citation(&command.name) || command.name == "bibitem" {
                Some(TargetKind::Citation)
            } else if matches!(command.name.as_str(), "begin" | "end")
                || syntax::declares_environment(&command.name)
            {
                Some(TargetKind::Environment)
            } else if matches!(command.name.as_str(), "newcounter" | "newaliascnt") {
                Some(TargetKind::Counter)
            } else {
                None
            };
            if let Some(kind) = kind {
                if arg.span.start <= offset && offset <= arg.span.end {
                    if (matches!(kind, TargetKind::Environment | TargetKind::Counter)
                        || kind == TargetKind::Label
                            && (command.name == "label"
                                || syntax::is_single_reference(&command.name))
                        || kind == TargetKind::Citation
                            && syntax::is_single_citation(&command.name))
                        && !syntax::is_static_key(&arg.value)
                    {
                        return None;
                    }
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
                        control_word: false,
                    });
                }
            }
        }
        if command.span.start <= offset && offset <= command.span.end {
            return Some(Target {
                kind: TargetKind::Command,
                name: &command.name,
                span: command.span.clone(),
                control_word: true,
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
                control_word: false,
            });
        }
    }
    None
}

fn target<'a>(workspace: &'a Workspace, uri: &Url, offset: usize) -> Option<Target<'a>> {
    let doc = workspace.documents.get(uri)?;
    if doc.syntax.is_ignored(offset) {
        return None;
    }
    let raw = raw_target(doc, offset);
    if raw
        .as_ref()
        .is_some_and(|target| target.kind != TargetKind::Command)
    {
        return raw;
    }
    for (target_kind, generated_kind) in [
        (TargetKind::Environment, crate::generated::Kind::Environment),
        (TargetKind::Counter, crate::generated::Kind::Counter),
        (TargetKind::Conditional, crate::generated::Kind::Conditional),
        (TargetKind::Toggle, crate::generated::Kind::Toggle),
    ] {
        if let Some((name, span)) = crate::generated::target(
            workspace,
            &workspace.symbol_scope(uri),
            doc,
            offset,
            generated_kind,
        ) {
            return Some(Target {
                kind: target_kind,
                name,
                span,
                control_word: false,
            });
        }
    }
    for index in 0..doc.syntax.commands.len() {
        if let Some(span) = syntax::literal_csname(&doc.text.source, &doc.syntax.commands, index) {
            if span.start <= offset && offset <= span.end {
                return Some(Target {
                    kind: TargetKind::Command,
                    name: &doc.text.source[span.clone()],
                    span,
                    control_word: false,
                });
            }
        }
    }
    raw
}

fn entries(doc: &Document, kind: TargetKind) -> &[Entry] {
    match kind {
        TargetKind::Label => &doc.syntax.labels,
        TargetKind::Anchor => &doc.syntax.anchors,
        TargetKind::Citation => &doc.syntax.citations,
        TargetKind::Command => &doc.syntax.definitions,
        TargetKind::Environment => &doc.syntax.environments,
        TargetKind::Counter => &doc.syntax.counters,
        TargetKind::Conditional => &doc.syntax.conditionals,
        TargetKind::Toggle => &doc.syntax.toggles,
    }
}

fn rename_target<'a>(
    workspace: &'a Workspace,
    uri: &Url,
    position: Position,
) -> tower_lsp::jsonrpc::Result<Option<(Target<'a>, BTreeSet<Url>)>> {
    let Some(doc) = workspace.documents.get(uri) else {
        return Ok(None);
    };
    let Some(target) = doc
        .text
        .offset(position)
        .and_then(|offset| target(workspace, uri, offset))
    else {
        return Ok(None);
    };
    let crate::workspace::SymbolProjects { mut scope, regions } = workspace.symbol_projects(uri);
    let declarations = |scope: &BTreeSet<Url>| -> Vec<Url> {
        workspace
            .documents
            .iter()
            .filter(|(uri, _)| scope.contains(*uri))
            .flat_map(|(uri, doc)| {
                entries(doc, target.kind)
                    .iter()
                    .filter(|entry| entry.name == target.name)
                    .map(move |_| uri.clone())
            })
            .collect()
    };
    let definitions = declarations(&scope);
    if definitions.is_empty() {
        return Ok(None);
    }
    if definitions.len() != 1 {
        return Err(tower_lsp::jsonrpc::Error::invalid_params(
            "Rename is ambiguous: the name has multiple definitions in this project",
        ));
    }
    // Shared occurrences must resolve to this declaration in every consumer.
    // Otherwise changing the shared file would silently break another paper.
    let occurrences = all_references(workspace, uri, position, true);
    loop {
        let used: BTreeSet<_> = occurrences
            .iter()
            .filter(|location| scope.contains(&location.uri))
            .map(|location| location.uri.clone())
            .collect();
        let before = scope.len();
        for project in regions
            .values()
            .filter(|project| !project.is_disjoint(&used))
        {
            if !project.contains(&definitions[0]) {
                return Err(tower_lsp::jsonrpc::Error::invalid_params("Rename is ambiguous: a shared occurrence belongs to a project with a different or missing definition"));
            }
            scope.extend(project.iter().cloned());
        }
        if scope.len() == before {
            break;
        }
    }
    if declarations(&scope).len() != 1 {
        return Err(tower_lsp::jsonrpc::Error::invalid_params(
            "Rename is ambiguous: projects sharing this definition contain multiple definitions",
        ));
    }
    Ok(Some((target, scope)))
}

pub fn prepare_rename(
    workspace: &Workspace,
    uri: &Url,
    position: Position,
) -> tower_lsp::jsonrpc::Result<Option<PrepareRenameResponse>> {
    let Some((target, _)) = rename_target(workspace, uri, position)? else {
        return Ok(None);
    };
    let doc = &workspace.documents[uri];
    let start = target.span.start + usize::from(target.control_word);
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
    let Some((target, scope)) = rename_target(workspace, uri, position)? else {
        return Ok(None);
    };
    let new_name = if target.kind == TargetKind::Command {
        new_name.strip_prefix('\\').unwrap_or(new_name)
    } else {
        new_name
    };
    let valid = !new_name.is_empty()
        && !(target.kind == TargetKind::Citation && new_name == "*")
        && syntax::is_static_key(new_name);
    if !valid {
        return Err(tower_lsp::jsonrpc::Error::invalid_params(
            "Invalid name: use a single TeX command name or a reference key without separators",
        ));
    }
    if new_name == target.name {
        return Ok(Some(WorkspaceEdit::default()));
    }
    let conflicts = workspace
        .documents
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
        .any(|(_, doc)| {
            entries(doc, target.kind)
                .iter()
                .any(|entry| entry.name == new_name)
                || target.kind == TargetKind::Environment
                    && doc.syntax.definitions.iter().any(|entry| {
                        entry.name == new_name || entry.name == format!("end{new_name}")
                    })
                || target.kind == TargetKind::Command
                    && crate::generated::generated_command_exists(doc, new_name)
        });
    let builtin = match target.kind {
        TargetKind::Command | TargetKind::Environment => {
            COMMANDS.iter().any(|(name, _, _)| *name == new_name)
                || syntax::declares_register(new_name)
                || crate::catalog::PRIMITIVES.contains(&new_name)
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
    if let Some(kind) = target.kind.generated_kind() {
        if let Some(model) = crate::generated::in_scope(workspace, &scope, target.name, kind) {
            crate::generated::validate(workspace, &scope, target.name, new_name, model)?;
        }
    }
    let mut changes: std::collections::BTreeMap<Url, Vec<TextEdit>> =
        std::collections::BTreeMap::new();
    for location in all_references(workspace, uri, position, true)
        .into_iter()
        .filter(|location| scope.contains(&location.uri))
    {
        let document = &workspace.documents[&location.uri];
        let text = &document.text;
        if target.kind == TargetKind::Anchor {
            let edits = crate::anchors::rename_edits(document, location.range, new_name)?;
            changes.entry(location.uri).or_default().extend(edits);
            continue;
        }
        let control_word = target.kind == TargetKind::Command
            && text
                .offset(location.range.start)
                .is_some_and(|offset| text.source[offset..].starts_with('\\'));
        if control_word
            && !new_name
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic() || byte == b'@')
        {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(
                "Names used in direct TeX commands must contain only command letters or @",
            ));
        }
        changes.entry(location.uri).or_default().push(TextEdit {
            range: location.range,
            new_text: if control_word {
                format!("\\{new_name}")
            } else if target.kind == TargetKind::Label {
                syntax::label_replacement_text(
                    text,
                    &document.syntax.commands,
                    location.range,
                    new_name,
                )
            } else {
                new_name.into()
            },
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
fn completion_name_argument(source: &str, command: &Command, offset: usize) -> Option<Span<usize>> {
    let (start, open) = if syntax::declares_environment_copy(&command.name) {
        (
            syntax::environment_copy_source_start(source, command)?,
            b'{',
        )
    } else if syntax::has_separate_label_argument(&command.name) {
        syntax::separate_label_argument_start(source, command)?
    } else {
        let mut at = syntax::skip_space(source, command.span.end);
        if syntax::reference_accepts_star(&command.name) && source.as_bytes().get(at) == Some(&b'*')
        {
            at = syntax::skip_space(source, at + 1);
        }
        (at, b'{')
    };
    syntax::active_name_argument(source, start, open, offset)
}

fn argument_context(doc: &Document, offset: usize) -> Option<(&Command, Span<usize>)> {
    doc.syntax
        .commands
        .iter()
        .rev()
        .filter(|command| command.span.end <= offset)
        .find_map(|command| argument_span(doc, command, offset).map(|span| (command, span)))
}

fn argument_span(doc: &Document, command: &Command, offset: usize) -> Option<Span<usize>> {
    let source = &doc.text.source;
    if !(matches!(command.name.as_str(), "begin" | "end" | "hyperlink")
        || syntax::declares_environment_copy(&command.name)
        || syntax::is_reference(&command.name)
        || syntax::has_separate_label_argument(&command.name)
        || syntax::is_citation(&command.name)
        || syntax::file_extensions(&command.name).is_some())
    {
        return None;
    }
    // A completed inner call in an optional note must not hide the outer key.
    // Most calls also let us skip old, closed arguments without rescanning text.
    if !syntax::has_separate_label_argument(&command.name)
        && !syntax::declares_environment_copy(&command.name)
        && command
            .argument
            .as_ref()
            .is_some_and(|arg| arg.outer_span.end < offset)
    {
        return None;
    }
    if matches!(command.name.as_str(), "begin" | "end")
        || syntax::declares_environment_copy(&command.name)
        || syntax::is_single_reference(&command.name)
        || syntax::has_separate_label_argument(&command.name)
        || command.name == "hyperlink"
        || syntax::is_single_citation(&command.name)
    {
        let span = completion_name_argument(source, command, offset)?;
        let prefix = source[span.start..offset].trim_start();
        return Some(offset - prefix.len()..offset);
    }
    if command.name == "input" {
        if let Some(argument) = &command.argument {
            if argument.span.start <= offset && offset <= argument.span.end {
                let start = if source.as_bytes().get(argument.outer_span.start) == Some(&b'{') {
                    offset - source[argument.span.start..offset].trim_start().len()
                } else {
                    argument.span.start
                };
                return Some(start..offset);
            }
        }
    }
    let mut at = syntax::skip_space(source, command.span.end);
    if source.as_bytes().get(at) == Some(&b'*') {
        if syntax::loader_extension(&command.name).is_some()
            || syntax::is_citation(&command.name) && !syntax::citation_accepts_star(&command.name)
        {
            return None;
        }
        at = syntax::skip_space(source, at + 1);
    }
    while source.as_bytes().get(at) == Some(&b'[') {
        if syntax::loader_forwards_options(&command.name)
            || syntax::is_citation(&command.name) && !syntax::citation_accepts_notes(&command.name)
        {
            return None;
        }
        let (_, after) = syntax::group(source, at, b'[', b']')?;
        at = syntax::skip_space(source, after);
    }
    let value = source.get(at..offset)?.strip_prefix('{')?;
    if value.contains(['{', '}', '%'])
        || value.contains('\n') && !syntax::file_name_ignores_spaces(&command.name)
        || command.name == "bibliographystyle"
            && (value.contains([' ', '\t', '\r', '\n'])
                || command
                    .argument
                    .as_ref()
                    .is_some_and(|argument| argument.value.contains([' ', '\t', '\r', '\n'])))
    {
        return None;
    }
    let key = value.rsplit(',').next()?.trim_start();
    Some(offset - key.len()..offset)
}

pub fn completion(workspace: &Workspace, uri: &Url, position: Position) -> Vec<CompletionItem> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let Some(offset) = doc.text.offset(position) else {
        return vec![];
    };
    let scope = workspace.symbol_scope(uri);
    if doc.kind == DocumentKind::BibtexStyle {
        return vec![];
    }
    if doc.kind == DocumentKind::Bibtex {
        return bibliography_completion(workspace, doc, offset, &scope);
    }
    if doc.syntax.is_ignored(offset.saturating_sub(1)) {
        return vec![];
    }
    if let Some(items) = crate::counters::completion(workspace, uri, offset, &scope) {
        return items;
    }
    if let Some(items) = crate::conditionals::completion(workspace, uri, offset, &scope) {
        return items;
    }
    if let Some(items) = crate::named_commands::completion(workspace, uri, offset, &scope) {
        return items;
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
        let (replacement_range, additional_edits) =
            if matches!(command.name.as_str(), "begin" | "end")
                || syntax::declares_environment_copy(&command.name)
                || syntax::is_single_reference(&command.name)
                || syntax::has_separate_label_argument(&command.name)
                || command.name == "hyperlink"
                || syntax::is_single_citation(&command.name)
            {
                let Some(name) = completion_name_argument(&doc.text.source, command, offset) else {
                    return vec![];
                };
                let Some(edits) = syntax::name_completion_ranges(&doc.text, name, offset) else {
                    return vec![];
                };
                edits
            } else {
                (doc.text.range(span.start, offset + suffix_length), vec![])
            };
        let kind = if command.name == "hyperlink" {
            Some(TargetKind::Anchor)
        } else if syntax::is_reference(&command.name)
            || syntax::has_separate_label_argument(&command.name)
        {
            Some(TargetKind::Label)
        } else if syntax::is_citation(&command.name) {
            Some(TargetKind::Citation)
        } else {
            None
        };
        let mut candidates = Vec::new();
        if let Some(kind) = kind {
            for (entry_uri, entry_doc) in &workspace.documents {
                if !scope.contains(entry_uri) {
                    continue;
                }
                for entry in entries(entry_doc, kind) {
                    candidates.push((
                        entry.name.clone(),
                        format!("{} — {}", entry.detail, entry_uri.path()),
                    ));
                }
            }
        } else if matches!(command.name.as_str(), "begin" | "end")
            || syntax::declares_environment_copy(&command.name)
        {
            candidates.extend(
                ENVIRONMENTS
                    .iter()
                    .map(|&name| (name.into(), "LaTeX environment".into())),
            );
            candidates.extend(
                workspace
                    .documents
                    .iter()
                    .filter(|(uri, _)| scope.contains(*uri))
                    .flat_map(|(_, doc)| &doc.syntax.commands)
                    .filter(|cmd| cmd.name == "begin" || syntax::declares_environment(&cmd.name))
                    .filter_map(|cmd| cmd.argument.as_ref())
                    .filter(|argument| syntax::is_static_key(&argument.value))
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
                    new_text: syntax::label_name_text(&doc.text.source, command, &name),
                })),
                additional_text_edits: (!additional_edits.is_empty())
                    .then(|| additional_edits.clone()),
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
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
        .flat_map(|(_, doc)| &doc.syntax.definitions)
        .chain(
            workspace
                .documents
                .iter()
                .filter(|(uri, _)| scope.contains(*uri))
                .flat_map(|(_, doc)| &doc.syntax.environments),
        )
        .filter(|entry| {
            !entry.name.is_empty()
                && entry
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphabetic() || byte == b'@')
        })
    {
        if entry.name.starts_with(prefix) && seen.insert(entry.name.clone()) {
            let signature = crate::signature::completion(workspace, &entry.name, &scope);
            let snippet = signature
                .as_ref()
                .and_then(|(_, snippet)| snippet.as_ref())
                .filter(|_| use_snippets);
            result.push(CompletionItem {
                label: entry.name.clone(),
                kind: Some(if entry.is_variable {
                    CompletionItemKind::VARIABLE
                } else {
                    CompletionItemKind::FUNCTION
                }),
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
    for entry in workspace
        .documents
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
        .flat_map(|(_, doc)| &doc.syntax.conditionals)
    {
        for (index, name) in crate::generated::conditional_names(&entry.name)
            .into_iter()
            .enumerate()
        {
            if name
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic() || byte == b'@')
                && name.starts_with(prefix)
                && seen.insert(name.clone())
            {
                result.push(CompletionItem {
                    label: name.clone(),
                    kind: Some(if index == 0 {
                        CompletionItemKind::VARIABLE
                    } else {
                        CompletionItemKind::FUNCTION
                    }),
                    detail: Some(format!("Conditional `{}` — {}", entry.name, entry.detail)),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range,
                        new_text: name,
                    })),
                    ..Default::default()
                });
            }
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
    scope: &BTreeSet<Url>,
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
        .filter(|(uri, _)| scope.contains(*uri))
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
    let Some(target) = target(workspace, uri, offset) else {
        return vec![];
    };
    let scope = workspace.symbol_scope(uri);
    workspace
        .documents
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
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
        .and_then(|offset| target(workspace, uri, offset))
    else {
        return vec![];
    };
    let crate::workspace::SymbolProjects { mut scope, regions } = workspace.symbol_projects(uri);
    let definitions: Vec<_> = workspace
        .documents
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
        .flat_map(|(uri, doc)| {
            entries(doc, target.kind)
                .iter()
                .filter(|entry| entry.name == target.name)
                .map(move |_| uri)
        })
        .collect();
    if definitions.len() == 1 {
        for project in regions
            .values()
            .filter(|project| project.contains(definitions[0]))
        {
            scope.extend(project.iter().cloned());
        }
    }
    all_references(workspace, uri, position, include_declaration)
        .into_iter()
        .filter(|location| scope.contains(&location.uri))
        .collect()
}

fn all_references(
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
        .and_then(|offset| target(workspace, uri, offset))
    else {
        return vec![];
    };
    let generated = target.kind.generated_kind().and_then(|kind| {
        crate::generated::in_scope(workspace, &workspace.symbol_scope(uri), target.name, kind)
    });
    let mut result = Vec::new();
    for (uri, doc) in &workspace.documents {
        if let Some(model) = generated {
            result.extend(
                crate::generated::references(doc, target.name, model)
                    .into_iter()
                    .filter(|reference| {
                        !matches!(target.kind, TargetKind::Conditional | TargetKind::Toggle)
                            || !entries(doc, target.kind)
                                .iter()
                                .any(|entry| entry.span == reference.span)
                    })
                    .map(|reference| Location {
                        uri: uri.clone(),
                        range: doc.text.range(reference.span.start, reference.span.end),
                    }),
            );
        }
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
        if target.kind == TargetKind::Anchor {
            result.extend(
                doc.syntax
                    .anchor_references
                    .iter()
                    .filter(|entry| entry.name == target.name)
                    .map(|entry| Location {
                        uri: uri.clone(),
                        range: doc.text.range(entry.span.start, entry.span.end),
                    }),
            );
            continue;
        }
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            if target.kind == TargetKind::Environment && generated.is_none() {
                if let Some(argument) = syntax::environment_copy_source(&doc.text.source, command)
                    .filter(|argument| argument.value == target.name)
                {
                    result.push(Location {
                        uri: uri.clone(),
                        range: doc.text.range(argument.span.start, argument.span.end),
                    });
                }
            }
            if target.kind == TargetKind::Command {
                result.extend(
                    crate::named_commands::spans(doc, command)
                        .into_iter()
                        .filter(|span| doc.text.source[span.clone()] == *target.name)
                        .filter(|span| {
                            !doc.syntax
                                .definitions
                                .iter()
                                .any(|entry| entry.span == *span)
                        })
                        .map(|span| Location {
                            uri: uri.clone(),
                            range: doc.text.range(span.start, span.end),
                        }),
                );
                if let Some(span) =
                    syntax::literal_csname(&doc.text.source, &doc.syntax.commands, index)
                        .filter(|span| doc.text.source[span.clone()] == *target.name)
                        .filter(|span| {
                            !doc.syntax
                                .definitions
                                .iter()
                                .any(|entry| entry.span == *span)
                        })
                {
                    result.push(Location {
                        uri: uri.clone(),
                        range: doc.text.range(span.start, span.end),
                    });
                }
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
                TargetKind::Label => {
                    syntax::is_reference(&command.name)
                        || syntax::has_separate_label_argument(&command.name)
                }
                TargetKind::Citation => syntax::is_citation(&command.name),
                TargetKind::Command | TargetKind::Anchor => false,
                TargetKind::Counter | TargetKind::Conditional | TargetKind::Toggle => false,
                TargetKind::Environment => matches!(command.name.as_str(), "begin" | "end"),
            };
            if !matches {
                continue;
            }
            let separate_label = syntax::separate_label_argument(&doc.text.source, command);
            let argument = if syntax::has_separate_label_argument(&command.name) {
                separate_label.as_ref()
            } else {
                command.argument.as_ref()
            };
            if let Some(arg) = argument {
                if target.kind == TargetKind::Environment
                    || target.kind == TargetKind::Citation
                        && syntax::is_single_citation(&command.name)
                    || target.kind == TargetKind::Label
                        && (syntax::is_single_reference(&command.name)
                            || syntax::has_separate_label_argument(&command.name))
                {
                    if arg.value == target.name {
                        result.push(Location {
                            uri: uri.clone(),
                            range: doc.text.range(arg.span.start, arg.span.end),
                        });
                    }
                    continue;
                }
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
    let target = target(workspace, uri, doc.text.offset(position)?)?;
    let scope = workspace.symbol_scope(uri);
    let mut value = None;
    for (entry_uri, entry_doc) in &workspace.documents {
        if !scope.contains(entry_uri) {
            continue;
        }
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
    for entries in [
        &doc.syntax.labels,
        &doc.syntax.citations,
        &doc.syntax.anchors,
    ] {
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
    if doc.kind == DocumentKind::Bibtex {
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
        (&doc.syntax.anchors, SymbolKind::KEY),
        (&doc.syntax.definitions, SymbolKind::FUNCTION),
        (&doc.syntax.environments, SymbolKind::CLASS),
        (&doc.syntax.counters, SymbolKind::VARIABLE),
        (&doc.syntax.conditionals, SymbolKind::BOOLEAN),
        (&doc.syntax.toggles, SymbolKind::BOOLEAN),
    ] {
        result.extend(entries.iter().map(|entry| DocumentSymbol {
            name: entry.name.clone(),
            detail: Some(entry.detail.clone()),
            kind: if entry.is_variable {
                SymbolKind::VARIABLE
            } else {
                kind
            },
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
