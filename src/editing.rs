use crate::{
    features, navigation, spelling_fixes, syntax,
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

pub fn code_actions(
    workspace: &Workspace,
    uri: &Url,
    range: Range,
    context: &CodeActionContext,
) -> CodeActionResponse {
    if !workspace.code_action_literals && !workspace.apply_edits {
        return vec![];
    }
    if context.only.as_ref().is_some_and(|kinds| {
        !kinds.iter().any(|kind| {
            let kind = kind.as_str();
            kind.is_empty() || kind == CodeActionKind::QUICKFIX.as_str()
        })
    }) {
        return vec![];
    }
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let mut fixes = crate::environment_diagnostics::fixes(workspace, uri, doc);
    fixes.extend(spelling_fixes::fixes(
        workspace,
        uri,
        range,
        &context.diagnostics,
    ));
    fixes
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
        .collect()
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
    let mut fixes = crate::environment_diagnostics::fixes(workspace, &request.uri, doc);
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
    while i < source.len() {
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
            candidates.push(line_start..line_end);
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

/// Link a single syntactically matched environment pair in this document.
pub fn linked_editing_ranges(doc: &Document, position: Position) -> Option<LinkedEditingRanges> {
    if doc.kind != crate::workspace::DocumentKind::Latex {
        return None;
    }
    let cursor = doc.text.offset(position)?;
    let source = &doc.text.source;
    let pair = |begin: &std::ops::Range<usize>, end: &std::ops::Range<usize>| {
        [begin, end]
            .iter()
            .any(|span| span.start <= cursor && cursor <= span.end)
            .then(|| LinkedEditingRanges {
                ranges: vec![
                    doc.text.range(begin.start, begin.end),
                    doc.text.range(end.start, end.end),
                ],
                word_pattern: Some(r"[^\s{}\\%]+".into()),
            })
    };
    let mut stack = Vec::new();
    for command in &doc.syntax.commands {
        if !matches!(command.name.as_str(), "begin" | "end") {
            continue;
        }
        let argument = command.argument.as_ref().filter(|argument| {
            argument.outer_span.start == syntax::skip_space(source, command.span.end)
                && syntax::is_static_key(&argument.value)
        });
        if command.name == "begin" {
            if let Some(begin) =
                argument.filter(|argument| syntax::is_verbatim_environment(&argument.value))
            {
                // Raw bodies contain no indexed end command. The parser's raw
                // region includes its exact literal terminator, if present.
                if let Some(raw) = doc
                    .syntax
                    .ignored
                    .iter()
                    .find(|raw| raw.start == begin.outer_span.end)
                {
                    let ending = format!("\\end{{{}}}", begin.value);
                    if let Some(start) = raw
                        .end
                        .checked_sub(ending.len())
                        .filter(|start| *start >= raw.start)
                    {
                        if source.get(start..raw.end) == Some(ending.as_str()) {
                            let end = start + "\\end{".len()..raw.end - 1;
                            if let Some(linked) = pair(&begin.span, &end) {
                                return Some(linked);
                            }
                        }
                    }
                }
                continue;
            }
            // Unknown argument shapes remain opaque until a closing command;
            // they must not borrow a known enclosing environment's name.
            stack.push(argument);
        } else {
            let Some(Some(begin)) = stack.pop() else {
                stack.clear();
                continue;
            };
            let Some(end) = argument.filter(|end| end.value == begin.value) else {
                stack.clear();
                continue;
            };
            if let Some(linked) = pair(&begin.span, &end.span) {
                return Some(linked);
            }
        }
    }
    None
}
