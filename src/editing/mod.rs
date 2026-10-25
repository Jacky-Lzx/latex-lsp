//! Source edits, refactorings, quick fixes and file operations.

pub(crate) mod anchor_edits;
pub(crate) mod code_action_resolve;
pub(crate) mod file_fixes;
pub mod file_operations;
pub(crate) mod image_fixes;
pub(crate) mod refactor;
pub(crate) mod spelling_fixes;

use crate::{
    language::{features, navigation},
    parser::syntax,
    project::workspace::{Document, Workspace},
};
use lsp_types::*;

pub fn code_actions(
    workspace: &Workspace,
    uri: &Url,
    range: Range,
    context: &CodeActionContext,
) -> CodeActionResponse {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let mut actions = Vec::new();
    if workspace.code_action_literals || workspace.apply_edits {
        actions.extend(crate::editing::refactor::actions(workspace, uri, range, context).into_iter().map(|action| {
            if workspace.code_action_literals { CodeActionOrCommand::CodeAction(action) }
            else { CodeActionOrCommand::Command(Command {
                title: action.title.clone(), command: "latex.applyCodeAction".into(),
                arguments: Some(vec![serde_json::json!({"uri":uri,"version":doc.version,"range":range,"kind":action.kind,"title":action.title})]),
            }) }
        }));
    }
    if !crate::editing::refactor::allows(context, &CodeActionKind::QUICKFIX) {
        return actions;
    }
    actions.extend(rebuild_actions(workspace, uri, range, context));
    if !workspace.code_action_literals && !workspace.apply_edits {
        return actions;
    }
    let mut fixes = crate::diagnostics::environment_diagnostics::fixes(workspace, uri, doc);
    fixes.extend(spelling_fixes::fixes(
        workspace,
        uri,
        range,
        &context.diagnostics,
    ));
    actions.extend(fixes
        .iter()
        .filter(|fix| {
            let diagnostic = &fix.diagnostic;
            diagnostic.range.start <= range.end
                && range.start <= diagnostic.range.end
                && (context.diagnostics.is_empty()
                    || context.diagnostics.iter().any(|reported| {
                        reported.range == diagnostic.range
                            && reported.message == diagnostic.message
                            && reported.source == diagnostic.source
                    }))
        })
        .map(|fix| {
            if !workspace.code_action_literals {
                return CodeActionOrCommand::Command(Command {
                    title: fix.title.clone(),
                    command: "latex.applyQuickFix".into(),
                    arguments: Some(vec![serde_json::json!({"uri":uri,"version":doc.version,"diagnostic":fix.diagnostic,"replacement":fix.edit.new_text})]),
                });
            }
            CodeActionOrCommand::CodeAction(CodeAction {
                title: fix.title.clone(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![fix.diagnostic.clone()]),
                is_preferred: Some(fix.diagnostic.source.as_deref() == Some("latex-lsp")),
                edit: Some(features::workspace_edit(
                    workspace,
                    std::collections::BTreeMap::from([(uri.clone(), fix.edits())]),
                )),
                ..Default::default()
            })
        })
    );
    actions
}

// Explicit stages must match this root and warning. The default latexmk cycle
// can also satisfy bibliography requests without explicit stages.
fn rebuild_warning(
    root: &Url,
    config: &crate::config::BuildConfig,
    diagnostic: &Diagnostic,
) -> bool {
    crate::compilation::build_log::rerun_required(diagnostic)
        && (crate::compilation::build_log::required_index_tool(diagnostic).is_none()
            || root.to_file_path().is_ok_and(|root| {
                crate::compilation::index_processing::handles_warning(&root, config, diagnostic)
            }))
        && crate::compilation::build_log::required_bibliography_tool(diagnostic).is_none_or(
            |required| {
                config.rebuilds_bibliography(required)
                    && (config.bibliography_tool.is_none()
                        || root.to_file_path().is_ok_and(|path| {
                            crate::compilation::build_log::required_bibliography_job(diagnostic)
                                .as_deref()
                                == config.job_name(&path).and_then(|name| name.to_str())
                        }))
            },
        )
}

/// Rebuild the project that produced a current warning, even when the source
/// belongs to multiple roots. Commands do not need workspace/applyEdit support.
fn rebuild_actions(
    workspace: &Workspace,
    uri: &Url,
    range: Range,
    context: &CodeActionContext,
) -> CodeActionResponse {
    workspace
        .build_diagnostics
        .iter()
        .filter_map(|(root, project)| {
            let config = workspace.build_config(root);
            let diagnostics: Vec<_> = project
                .get(uri)?
                .iter()
                .filter(|diagnostic| {
                    diagnostic.source.as_deref() == Some("latex-build")
                        && rebuild_warning(root, &config, diagnostic)
                        && diagnostic.range.start <= range.end
                        && range.start <= diagnostic.range.end
                        && (context.diagnostics.is_empty()
                            || context.diagnostics.iter().any(|reported| {
                                reported.range == diagnostic.range
                                    && reported.message == diagnostic.message
                                    && reported.source == diagnostic.source
                            }))
                })
                .take(64)
                .cloned()
                .collect();
            if diagnostics.is_empty()
                || workspace.root_document(root).as_ref().ok() != Some(root)
                || config.rebuild_arguments().is_err()
            {
                return None;
            }
            let path = root.to_file_path().ok().filter(|path| path.is_file())?;
            let title = format!("Rebuild {}", path.display());
            let command = Command {
                title: title.clone(),
                command: "latex.rebuild".into(),
                arguments: Some(vec![serde_json::json!({
                    "uri":root,
                    "rebuildGuard":{
                        "uri":uri,
                        "version":workspace.documents.get(uri)?.version,
                        "diagnostics":diagnostics,
                    },
                })]),
            };
            Some(if workspace.code_action_literals {
                CodeActionOrCommand::CodeAction(CodeAction {
                    title,
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(diagnostics),
                    command: Some(command),
                    ..Default::default()
                })
            } else {
                CodeActionOrCommand::Command(command)
            })
        })
        .take(32)
        .collect()
}

#[derive(serde::Deserialize)]
pub(crate) struct RebuildGuard {
    pub uri: Url,
    pub version: Option<i32>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Recheck the menu's original source and warning under the build's workspace
/// lock. A cached editor action must not become an unqualified manual rebuild.
pub(crate) fn valid_rebuild_guard(workspace: &Workspace, root: &Url, guard: &RebuildGuard) -> bool {
    workspace
        .documents
        .get(&guard.uri)
        .is_some_and(|doc| doc.version == guard.version)
        && workspace
            .build_diagnostics
            .get(root)
            .and_then(|project| project.get(&guard.uri))
            .is_some_and(|current| {
                guard.diagnostics.iter().any(|diagnostic| {
                    diagnostic.source.as_deref() == Some("latex-build")
                        && rebuild_warning(root, &workspace.build_config(root), diagnostic)
                        && current.contains(diagnostic)
                })
            })
}

#[derive(serde::Deserialize)]
pub struct FixRequest {
    pub uri: Url,
    pub version: Option<i32>,
    pub diagnostic: Diagnostic,
    pub replacement: Option<String>,
}

pub fn resolve_fix(workspace: &Workspace, request: FixRequest) -> Option<WorkspaceEdit> {
    let doc = workspace.documents.get(&request.uri)?;
    if doc.version != request.version {
        return None;
    }
    let mut fixes =
        crate::diagnostics::environment_diagnostics::fixes(workspace, &request.uri, doc);
    fixes.extend(spelling_fixes::fixes(
        workspace,
        &request.uri,
        request.diagnostic.range,
        std::slice::from_ref(&request.diagnostic),
    ));
    let fix = fixes.iter().find(|fix| {
        fix.diagnostic.range == request.diagnostic.range
            && fix.diagnostic.message == request.diagnostic.message
            && request.diagnostic.source == fix.diagnostic.source
            && request.replacement.as_ref().map_or(
                fix.diagnostic.source.as_deref() == Some("latex-lsp"),
                |replacement| *replacement == fix.edit.new_text,
            )
    })?;
    Some(features::workspace_edit(
        workspace,
        std::collections::BTreeMap::from([(request.uri, fix.edits())]),
    ))
}

pub fn highlights(workspace: &Workspace, uri: &Url, position: Position) -> Vec<DocumentHighlight> {
    let definitions = features::definitions(workspace, uri, position);
    features::references(workspace, uri, position, true)
        .into_iter()
        .filter(|location| &location.uri == uri)
        .map(|location| DocumentHighlight {
            kind: Some(if definitions.contains(&location) {
                DocumentHighlightKind::WRITE
            } else {
                DocumentHighlightKind::READ
            }),
            range: location.range,
        })
        .collect()
}

fn symbol_spans(
    doc: &Document,
    symbols: &[DocumentSymbol],
    spans: &mut Vec<std::ops::Range<usize>>,
) {
    for symbol in symbols {
        for range in [symbol.range, symbol.selection_range] {
            if let (Some(start), Some(end)) =
                (doc.text.offset(range.start), doc.text.offset(range.end))
            {
                spans.push(start..end);
            }
        }
        if let Some(children) = &symbol.children {
            symbol_spans(doc, children, spans);
        }
    }
}

fn reference_selection_spans(doc: &Document, spans: &mut Vec<std::ops::Range<usize>>) {
    let source = &doc.text.source;
    if doc.kind == crate::project::workspace::DocumentKind::Bibtex {
        spans.extend(
            doc.syntax
                .citation_references
                .iter()
                .chain(&doc.syntax.bibliography_aliases)
                .map(|entry| entry.span.clone()),
        );
    }
    for argument in doc
        .syntax
        .citation_fields
        .iter()
        .map(|field| &field.argument)
        .chain(&doc.syntax.bibliography_alias_values)
    {
        spans.extend([argument.span.clone(), argument.outer_span.clone()]);
    }
    for command in &doc.syntax.commands {
        if doc.syntax.control_operands.contains(&command.span.start) {
            continue;
        }
        if let Some(style) = crate::bibliography::citations::style(&command.name) {
            let arguments =
                crate::bibliography::citations::arguments(source, command.span.end, style);
            for field in &arguments.fields {
                spans.extend([
                    field.argument.span.clone(),
                    field.argument.outer_span.clone(),
                ]);
            }
            for argument in arguments.keys() {
                spans.extend(crate::bibliography::citations::key_spans(
                    source, argument, style,
                ));
            }
            if let Some(last) = arguments.fields.last() {
                // The diagnostic end includes a lookahead token. A selection
                // ends at the final actual argument, before any following text.
                spans.push(command.span.start..last.argument.outer_span.end);
            }
        } else if let Some(argument) = &command.argument {
            if syntax::is_citation(&command.name) {
                if syntax::is_single_citation(&command.name) {
                    if syntax::is_static_citation_key(&argument.value) {
                        spans.push(argument.span.clone());
                    }
                } else {
                    spans.extend(syntax::citation_list_spans(source, argument.span.clone()));
                }
                if command.name == "defcitealias" {
                    if let Some((display, _)) = syntax::group(
                        source,
                        syntax::skip_space(source, argument.outer_span.end),
                        b'{',
                        b'}',
                    ) {
                        spans.extend([
                            display.span,
                            display.outer_span.clone(),
                            command.span.start..display.outer_span.end,
                        ]);
                    }
                }
            } else if syntax::is_reference(&command.name) {
                if syntax::is_single_reference(&command.name) {
                    if syntax::is_static_reference_key(&argument.value) {
                        spans.push(argument.span.clone());
                    }
                } else {
                    spans.extend(syntax::reference_list_spans(source, argument.span.clone()));
                }
            }
        }
        for argument in syntax::separate_label_argument(source, command)
            .into_iter()
            .chain(syntax::range_reference_end(source, command))
        {
            spans.extend([
                argument.span,
                argument.outer_span.clone(),
                command.span.start..argument.outer_span.end,
            ]);
        }
    }
}

/// Return strict containment chains, preserving the client's input order.
pub fn selection_ranges(doc: &Document, positions: &[Position]) -> Option<Vec<SelectionRange>> {
    let source = &doc.text.source;
    let mut spans = Vec::new();
    spans.push(0..source.len());
    for command in &doc.syntax.commands {
        spans.push(command.span.clone());
        if let Some(argument) = &command.argument {
            spans.push(argument.span.clone());
            spans.push(argument.outer_span.clone());
            spans.push(command.span.start..argument.outer_span.end);
        }
    }
    reference_selection_spans(doc, &mut spans);
    spans.extend(
        doc.syntax
            .bibliography_strings
            .selection_spans
            .iter()
            .cloned(),
    );
    spans.extend(
        doc.syntax
            .bibliography_completion
            .types
            .iter()
            .filter_map(|ty| ty.entry_span.clone()),
    );
    spans.extend(doc.syntax.ignored.iter().cloned());
    symbol_spans(doc, &features::symbols(doc), &mut spans);
    for fold in navigation::folding_ranges(doc, None) {
        let start = doc.text.offset(Position::new(fold.start_line, 0))?;
        let end = doc
            .text
            .offset(Position::new(fold.end_line + 1, 0))
            .unwrap_or(source.len());
        spans.push(start..end);
    }
    let mut groups = Vec::new();
    let mut i = 0;
    let mut ignored = doc.syntax.ignored.iter().peekable();
    let mut literals = doc
        .syntax
        .bibliography_strings
        .literal_spans
        .iter()
        .peekable();
    let mut literal_scope: Option<(usize, usize)> = None;
    while i < source.len() {
        // Unmatched punctuation in one literal cannot pair with a later value.
        if let Some((_, depth)) = literal_scope.filter(|(end, _)| *end <= i) {
            groups.truncate(depth);
            literal_scope = None;
        }
        while literals.peek().is_some_and(|span| span.end <= i) {
            literals.next();
        }
        if let Some(span) = literals.peek().filter(|span| span.start == i) {
            literal_scope = Some((span.end, groups.len()));
            literals.next();
        }
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
        match source.as_bytes()[i] {
            b'%' if doc.kind == crate::project::workspace::DocumentKind::Bibtex
                && literal_scope.is_none() =>
            {
                let start = i;
                while i < source.len() && source.as_bytes()[i] != b'\n' {
                    i += 1;
                }
                spans.push(start..i);
            }
            b'\\' => {
                i += 1;
                if i < source.len() {
                    i += source[i..].chars().next()?.len_utf8();
                }
            }
            b'{' | b'[' => {
                groups.push((source.as_bytes()[i], i));
                i += 1;
            }
            b'}' | b']' => {
                let opening = if source.as_bytes()[i] == b'}' {
                    b'{'
                } else {
                    b'['
                };
                if groups.last().is_some_and(|(kind, _)| *kind == opening) {
                    let (_, start) = groups.pop()?;
                    spans.push(start..i + 1);
                    spans.push(start + 1..i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    // Comment spans end immediately before LF and can therefore include CR.
    // LSP line columns exclude line endings, including the CR in CRLF.
    for span in &mut spans {
        if span.start < span.end && source[..span.end].ends_with('\r') {
            span.end -= 1;
        }
    }
    positions
        .iter()
        .map(|position| {
            let offset = doc.text.offset(*position)?;
            let line_start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
            let line_end = source[offset..]
                .find('\n')
                .map_or(source.len(), |index| offset + index);
            let line_end = if source[..line_end].ends_with('\r') {
                line_end - 1
            } else {
                line_end
            };
            let word = |ch: char| ch.is_alphanumeric() || matches!(ch, '_' | ':' | '-' | '@' | '.');
            let start = source[..offset]
                .char_indices()
                .rev()
                .take_while(|(_, ch)| word(*ch))
                .last()
                .map_or(offset, |(index, _)| index);
            let end = offset
                + source[offset..]
                    .chars()
                    .take_while(|ch| word(*ch))
                    .map(char::len_utf8)
                    .sum::<usize>();
            let mut candidates: Vec<_> = spans
                .iter()
                .filter(|span| span.start <= offset && offset <= span.end)
                .cloned()
                .collect();
            let line = line_start..line_end;
            // A partial line at the end of a multiline call can extend into
            // following prose. Do not let that crossing range displace the
            // enclosing argument or command from the containment chain.
            if candidates.iter().all(|span| {
                span.start <= line.start && line.end <= span.end
                    || line.start <= span.start && span.end <= line.end
            }) {
                candidates.push(line);
            }
            if start < end {
                candidates.push(start..end);
            }
            candidates.sort_by_key(|span| (span.len(), std::cmp::Reverse(span.start)));
            candidates.dedup();
            let mut inner = offset..offset;
            let mut chain = Vec::new();
            for span in candidates {
                if span.start <= inner.start
                    && inner.end <= span.end
                    && (chain.is_empty() || span != inner)
                {
                    inner = span.clone();
                    chain.push(doc.text.range(span.start, span.end));
                }
            }
            let mut parent = None;
            for range in chain.into_iter().rev() {
                parent = Some(Box::new(SelectionRange { range, parent }));
            }
            parent.map(|selection| *selection)
        })
        .collect()
}

/// Link a matched environment pair with only this document as context.
/// A local pair cannot cross an input or package-loading command.
/// Use `linked_editing_ranges_in_workspace` when project documents are available.
pub fn linked_editing_ranges(doc: &Document, position: Position) -> Option<LinkedEditingRanges> {
    if doc.kind != crate::project::workspace::DocumentKind::Latex
        || doc.text.offset(position).is_none()
    {
        return None;
    }
    let uri = Url::parse("untitled:linked-editing.tex").ok()?;
    let mut workspace = Workspace::default();
    workspace.documents.insert(uri.clone(), doc.clone());
    linked_editing_ranges_with_context(&workspace, &uri, position, true)
}

/// Link a pair only when all executions and consuming projects agree.
pub fn linked_editing_ranges_in_workspace(
    workspace: &Workspace,
    uri: &Url,
    position: Position,
) -> Option<LinkedEditingRanges> {
    linked_editing_ranges_with_context(workspace, uri, position, false)
}

fn linked_editing_ranges_with_context(
    workspace: &Workspace,
    uri: &Url,
    position: Position,
    local: bool,
) -> Option<LinkedEditingRanges> {
    let doc = workspace.documents.get(uri)?;
    if doc.kind != crate::project::workspace::DocumentKind::Latex {
        return None;
    }
    let cursor = doc.text.offset(position)?;
    // A linked name must be a braced argument after a literal or named
    // command. Cursor-move requests elsewhere need no project-wide pairing.
    // Do not resolve the command here: project aliases and comment-bounded
    // names still receive the full execution/context checks below.
    let name_argument = doc.syntax.commands.iter().any(|command| {
        [
            Some(command.span.end),
            command
                .argument
                .as_ref()
                .map(|argument| argument.outer_span.end),
        ]
        .into_iter()
        .flatten()
        .filter(|end| *end <= cursor)
        .any(|end| {
            let at = crate::parser::syntax::skip_space(&doc.text.source, end);
            if at >= cursor || doc.text.source.as_bytes().get(at) != Some(&b'{') {
                return false;
            }
            crate::parser::syntax::environment_name_argument(&doc.text.source, end)
                .or_else(|| {
                    crate::parser::syntax::group(&doc.text.source, at, b'{', b'}')
                        .filter(|(argument, _)| argument.span.is_empty())
                        .map(|(argument, _)| argument)
                })
                .is_some_and(|argument| {
                    argument.span.start <= cursor && cursor <= argument.span.end
                })
        })
    });
    // Raw closing markers live in the parser's ignored region rather than
    // the command index. Let the ordered walk certify those markers as well.
    if !name_argument && !doc.syntax.ignored.iter().any(|span| span.contains(&cursor)) {
        return None;
    }
    let pairs = if local {
        crate::symbols::project_environments::local_linked_pairs(workspace, uri)
    } else {
        crate::symbols::project_environments::linked_pairs(workspace, uri)
    };
    pairs
        .into_iter()
        .find(|pair| {
            pair.iter()
                .any(|(start, end)| *start <= cursor && cursor <= *end)
        })
        .map(|pair| LinkedEditingRanges {
            ranges: pair
                .iter()
                .map(|(start, end)| doc.text.range(*start, *end))
                .collect(),
            word_pattern: Some(r"[^\s{}\\%]*".into()),
        })
}
