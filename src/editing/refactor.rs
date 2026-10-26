//! Conservative structural actions and deterministic source fixes.
use crate::{
    language::features,
    parser::syntax,
    project::workspace::{Document, DocumentKind, Workspace},
};
use lsp_types::*;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn allows(context: &CodeActionContext, kind: &CodeActionKind) -> bool {
    context.only.as_ref().is_none_or(|only| {
        only.iter().any(|requested| {
            requested.as_str().is_empty()
                || requested == kind
                || kind
                    .as_str()
                    .strip_prefix(requested.as_str())
                    .is_some_and(|rest| rest.starts_with('.'))
        })
    })
}
fn action(
    ws: &Workspace,
    uri: &Url,
    title: &str,
    kind: CodeActionKind,
    edits: Vec<TextEdit>,
) -> CodeAction {
    CodeAction {
        title: title.into(),
        kind: Some(kind),
        edit: Some(features::workspace_edit(
            ws,
            BTreeMap::from([(uri.clone(), edits)]),
        )),
        ..Default::default()
    }
}
fn overlaps(a: Range, b: Range) -> bool {
    // Insertions at another edit's boundary are also ambiguous.
    a.start <= b.end && b.start <= a.end
}
fn fix_all(ws: &Workspace, uri: &Url, doc: &Document) -> Option<CodeAction> {
    let fixes = crate::diagnostics::environment_diagnostics::fixes(ws, uri, doc);
    let mut unique: Vec<&syntax::QuickFix> = Vec::new();
    for fix in &fixes {
        if fix.diagnostic.source.as_deref() != Some("texquill") {
            continue;
        }
        let edits = fix.edits();
        if edits.iter().enumerate().any(|(i, edit)| {
            edits[i + 1..]
                .iter()
                .any(|other| overlaps(edit.range, other.range))
        }) {
            continue;
        }
        if fixes
            .iter()
            .any(|other| other.diagnostic == fix.diagnostic && other.edits() != edits)
        {
            continue;
        }
        if !unique.iter().any(|other| other.edits() == edits) {
            unique.push(fix);
        }
    }
    let mut edits = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, fix) in unique.iter().enumerate() {
        let proposed = fix.edits();
        if unique.iter().enumerate().any(|(other_index, other)| {
            index != other_index
                && proposed.iter().any(|edit| {
                    other
                        .edits()
                        .iter()
                        .any(|other| overlaps(edit.range, other.range))
                })
        }) {
            continue;
        }
        edits.extend(proposed);
        diagnostics.push(fix.diagnostic.clone());
    }
    if edits.is_empty() || edits.len() > 256 {
        return None;
    }
    let mut result = action(
        ws,
        uri,
        "Fix all unambiguous LaTeX syntax errors",
        CodeActionKind::SOURCE_FIX_ALL,
        edits,
    );
    result.diagnostics = Some(diagnostics);
    Some(result)
}

/// Only plain text and complete, known inline text commands at top level.
/// Reject partially selected arguments, formulae, comments and special catcodes.
fn selection(
    doc: &Document,
    range: Range,
    signatures: &crate::symbols::commands::CommandSignatures<'_>,
) -> Option<(usize, usize, Option<usize>)> {
    if doc.kind != DocumentKind::Latex {
        return None;
    }
    let start = doc.text.offset(range.start)?;
    let end = doc.text.offset(range.end)?;
    if start >= end || end - start > 65536 {
        return None;
    }
    let source = &doc.text.source;
    let selected = &source[start..end];
    if selected.trim().is_empty() || selected.contains(['%', '#', '$', '&', '^', '_']) {
        return None;
    }
    if doc
        .syntax
        .ignored
        .iter()
        .any(|span| span.start < end && start < span.end)
    {
        return None;
    }
    if !signatures.environment_primitives_unchanged()
        || ["(", ")", "[", "]"]
            .iter()
            .any(|name| !signatures.command_unchanged(name))
    {
        return None;
    }
    let operands = signatures.argument_tokens(source, &doc.syntax);
    let declarations = doc
        .syntax
        .definitions
        .iter()
        .map(|entry| (entry.span.start, entry.span.end))
        .collect();
    let boundaries = signatures.environment_boundaries(source, &doc.syntax);
    let boundaries: BTreeMap<_, _> = boundaries
        .iter()
        .filter(|boundary| !boundary.operand && !boundary.deferred)
        .map(|boundary| (boundary.span.start, boundary))
        .collect();
    let begin = boundaries.values().find(|boundary| {
        boundary.begin
            && boundary
                .argument
                .as_ref()
                .is_some_and(|arg| arg.value == "document")
    });
    let insertion = begin.map(|boundary| boundary.span.start);
    if let Some(begin) = begin {
        if start < begin.span.end {
            return None;
        }
    } else if doc
        .syntax
        .commands
        .iter()
        .any(|cmd| cmd.name == "documentclass")
    {
        return None;
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Math {
        Inline,
        Display,
    }
    let mut depth = 0i32;
    let mut math = None;
    let mut environments = Vec::new();
    let mut expl_mode = false;
    // begingroup/endgroup pairs cannot close a simple brace/bgroup frame.
    let mut group_modes = Vec::new();
    let deferred = crate::symbols::expl3::occurrences(doc).bodies;
    let mut opaque: Vec<_> = doc
        .syntax
        .ignored
        .iter()
        .chain(&doc.syntax.literal_conversions)
        .chain(&doc.syntax.literal_tests)
        .chain(&doc.syntax.ambiguous_string_tests)
        .cloned()
        .collect();
    opaque.extend(crate::symbols::expl3::math_data_spans(doc));
    if opaque
        .iter()
        .any(|span| span.start < end && start < span.end)
    {
        return None;
    }
    let mut at = 0;
    let bytes = source.as_bytes();
    let mut math_work = 0usize;
    while at < start {
        if let Some(body) = deferred.iter().find(|body| body.contains(&at)) {
            if body.end > start {
                return None;
            }
            at = body.end;
            continue;
        }
        if let Some(ignored) = opaque.iter().find(|span| span.contains(&at)) {
            at = ignored.end;
            continue;
        }
        if let Some(boundary) = boundaries.get(&at) {
            let argument = boundary.argument.as_ref()?;
            if boundary.span.end > start {
                return None;
            }
            if boundary.begin {
                environments.push(argument.value.as_str());
            } else if environments.pop() != Some(argument.value.as_str()) {
                return None;
            }
            at = boundary.span.end;
            continue;
        }
        if bytes[at] == b'\\' {
            let token = syntax::token(source, at)?;
            let mut next = at + token.len();
            if doc.syntax.control_operands.contains(&at) || operands.contains(&at) {
                at = next;
                continue;
            }
            let index = doc
                .syntax
                .commands
                .binary_search_by_key(&at, |cmd| cmd.span.start)
                .ok();
            let invocation = index.and_then(|index| {
                signatures.named_invocation(source, &doc.syntax, index, &operands, &declarations)
            });
            let name = if let Some((name, span)) = invocation {
                next = span.end;
                name
            } else {
                token.strip_prefix('\\')?
            };
            if next > start {
                return None;
            }
            if matches!(name, "catcode" | "verb" | "lstinline" | "mintinline") {
                return None;
            }
            if matches!(
                name,
                "ExplSyntaxOn" | "ProvidesExplPackage" | "ProvidesExplClass" | "ProvidesExplFile"
            ) {
                if !signatures.command_unchanged(name) {
                    return None;
                }
                expl_mode = true;
            }
            if name == "ExplSyntaxOff" {
                if !signatures.command_unchanged(name) {
                    return None;
                }
                expl_mode = false;
            }
            let invokes = |builtin| signatures.invokes_builtin(name, builtin);
            if invokes("begingroup") || invokes("bgroup") {
                depth += 1;
                group_modes.push((invokes("begingroup"), expl_mode));
            } else if invokes("endgroup") || invokes("egroup") {
                let (semi_simple, previous) = group_modes.pop()?;
                if semi_simple != invokes("endgroup") {
                    return None;
                }
                depth -= 1;
                expl_mode = previous;
            }
            if let Some(name) = ["(", ")", "[", "]"].into_iter().find(|name| invokes(name)) {
                let mode = if matches!(name, "(" | ")") {
                    Math::Inline
                } else {
                    Math::Display
                };
                if matches!(name, "(" | "[") {
                    if math.is_some() {
                        return None;
                    }
                    math = Some(mode);
                } else if math.take() != Some(mode) {
                    return None;
                }
            }
            at = next;
            continue;
        }
        if doc.syntax.punctuation_operands.contains(&at) {
            at += 1;
            continue;
        }
        match bytes[at] {
            b'{' => {
                depth += 1;
                group_modes.push((false, expl_mode));
            }
            b'}' => {
                let (semi_simple, previous) = group_modes.pop()?;
                if semi_simple {
                    return None;
                }
                depth -= 1;
                expl_mode = previous;
            }
            b'$' => {
                // Bound repeated argument queries for punctuation in large files.
                math_work = math_work.checked_add(doc.syntax.commands.len().max(1))?;
                if math_work > 4_000_000 {
                    return None;
                }
                if signatures.literal_is_argument(source, &doc.syntax, at) {
                    at += 1;
                    continue;
                }
                let width = if bytes.get(at + 1) == Some(&b'$') {
                    2
                } else {
                    1
                };
                let mode = if width == 2 {
                    Math::Display
                } else {
                    Math::Inline
                };
                if math == Some(mode) {
                    math = None;
                } else if math.is_some() {
                    return None;
                } else {
                    math = Some(mode);
                }
                at += width;
                continue;
            }
            _ => {}
        }
        at += 1;
    }
    if (begin.is_some() && !environments.contains(&"document"))
        || at != start
        || depth != 0
        || math.is_some()
        || expl_mode
        || environments.iter().any(|name| *name != "document")
    {
        return None;
    }
    // Independently balance the selected text, ignoring control symbols.
    let mut at = start;
    while at < end {
        if bytes[at] == b'\\' {
            if let Some(cmd) = doc.syntax.commands.iter().find(|cmd| cmd.span.start == at) {
                if cmd.span.end > end || !signatures.command_unchanged(&cmd.name) {
                    return None;
                }
                match cmd.name.as_str() {
                    "emph" | "textbf" | "textit" | "texttt" | "textrm" | "textsf"
                    | "textnormal" | "underline" => {
                        let arg = cmd.argument.as_ref()?;
                        if arg.outer_span.start != syntax::skip_space(source, cmd.span.end)
                            || arg.outer_span.end > end
                        {
                            return None;
                        }
                    }
                    "TeX" | "LaTeX" | "ldots" | "dots" => {}
                    _ => return None,
                }
                at = cmd.span.end;
                continue;
            }
            let next = source[at + 1..].chars().next()?;
            if !matches!(next, '{' | '}' | '%' | '&' | '#' | '$' | '_' | ' ') {
                return None;
            }
            at += 1 + next.len_utf8();
            continue;
        }
        match bytes[at] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return None;
        }
        at += 1;
    }
    (at == end && depth == 0).then_some((start, end, insertion))
}

pub(crate) fn actions(
    ws: &Workspace,
    uri: &Url,
    range: Range,
    context: &CodeActionContext,
) -> Vec<CodeAction> {
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    let mut result = Vec::new();
    // Source actions are requested explicitly (e.g. codeActionsOnSave).
    if context.only.is_some() && allows(context, &CodeActionKind::SOURCE_FIX_ALL) {
        if let Some(action) = fix_all(ws, uri, doc) {
            result.push(action);
        }
    }
    if !context.diagnostics.is_empty()
        || !allows(context, &CodeActionKind::REFACTOR_REWRITE)
            && !allows(context, &CodeActionKind::REFACTOR_EXTRACT)
        || doc
            .text
            .offset(range.start)
            .zip(doc.text.offset(range.end))
            .is_none_or(|(start, end)| start >= end || end - start > 65536)
    {
        return result;
    }
    let scope = ws.symbol_scope(uri);
    let documents: Vec<_> = ws
        .documents
        .iter()
        .filter(|(uri, doc)| scope.contains(*uri) && doc.kind == DocumentKind::Latex)
        .map(|(_, doc)| doc)
        .collect();
    if !ws.index_truncated.is_empty()
        || documents
            .iter()
            .map(|doc| doc.text.source.len())
            .sum::<usize>()
            > 16 * 1024 * 1024
        || documents
            .iter()
            .map(|doc| doc.syntax.commands.len())
            .sum::<usize>()
            > 100_000
        || documents
            .iter()
            .map(|doc| doc.syntax.definitions.len() + doc.syntax.environments.len())
            .sum::<usize>()
            > 4096
    {
        return result;
    }
    let signatures = crate::symbols::commands::CommandSignatures::new(
        documents
            .iter()
            .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
    );
    let Some((start, end, insertion)) = selection(doc, range, &signatures) else {
        return result;
    };
    let anchor = syntax::skip_space(&doc.text.source, start);
    if signatures.is_parameter_token(&doc.text.source, &doc.syntax, start)
        || anchor < end
            && anchor != start
            && signatures.is_parameter_token(&doc.text.source, &doc.syntax, anchor)
    {
        return result;
    }
    let selected = &doc.text.source[start..end];
    if allows(context, &CodeActionKind::REFACTOR_REWRITE) {
        for (name, title, replacement) in [
            (
                "emph",
                "Wrap selection in \\emph",
                format!("\\emph{{{selected}}}"),
            ),
            (
                "textbf",
                "Wrap selection in \\textbf",
                format!("\\textbf{{{selected}}}"),
            ),
            (
                "quote",
                "Wrap selection in quote environment",
                format!(
                    "\\begin{{quote}}{}{selected}{}\\end{{quote}}",
                    doc.text.newline(),
                    doc.text.newline()
                ),
            ),
        ] {
            if !signatures.command_unchanged(name)
                || name == "quote" && !signatures.command_unchanged("endquote")
            {
                continue;
            }
            result.push(action(
                ws,
                uri,
                title,
                CodeActionKind::REFACTOR_REWRITE,
                vec![TextEdit {
                    range,
                    new_text: replacement,
                }],
            ));
        }
    }
    if allows(context, &CodeActionKind::REFACTOR_EXTRACT)
        && signatures.command_unchanged("newcommand")
    {
        if let Some(insertion) = insertion {
            let mut used = BTreeSet::new();
            for document in &documents {
                used.extend(
                    document
                        .syntax
                        .commands
                        .iter()
                        .map(|command| command.name.clone()),
                );
                used.extend(
                    document
                        .syntax
                        .definitions
                        .iter()
                        .map(|entry| entry.name.clone()),
                );
                used.extend(crate::symbols::generated::commands(document));
                for (index, command) in document.syntax.commands.iter().enumerate() {
                    for span in crate::symbols::named_commands::spans(document, command) {
                        used.insert(document.text.source[span].to_owned());
                    }
                    if let Some(span) = syntax::literal_csname(
                        &document.text.source,
                        &document.syntax.commands,
                        index,
                    ) {
                        used.insert(document.text.source[span].to_owned());
                    }
                }
                for call in
                    signatures.environment_invocations(&document.text.source, &document.syntax)
                {
                    used.insert(format!("end{}", call.argument.value));
                    used.insert(call.argument.value);
                }
            }
            let name = (0..26)
                .map(|index| {
                    if index == 0 {
                        "extractedcommand".into()
                    } else {
                        format!("extractedcommand{}", char::from(b'a' + index))
                    }
                })
                .find(|name| !used.contains(name));
            if let Some(name) = name {
                result.push(action(
                    ws,
                    uri,
                    "Extract selection into a new command",
                    CodeActionKind::REFACTOR_EXTRACT,
                    vec![
                        TextEdit {
                            range: doc.text.range(insertion, insertion),
                            new_text: format!(
                                "\\newcommand{{\\{name}}}{{{selected}}}{}",
                                doc.text.newline()
                            ),
                        },
                        TextEdit {
                            range,
                            new_text: format!("\\{name}{{}}"),
                        },
                    ],
                ));
            }
        }
    }
    result
}

#[derive(serde::Deserialize)]
pub(crate) struct Request {
    pub uri: Url,
    pub version: Option<i32>,
    pub range: Range,
    pub kind: CodeActionKind,
    pub title: String,
}
pub(crate) fn resolve(ws: &Workspace, request: Request) -> Option<WorkspaceEdit> {
    let doc = ws.documents.get(&request.uri)?;
    if doc.version != request.version {
        return None;
    }
    actions(
        ws,
        &request.uri,
        request.range,
        &CodeActionContext {
            diagnostics: vec![],
            only: Some(vec![request.kind]),
            trigger_kind: None,
        },
    )
    .into_iter()
    .find(|action| action.title == request.title)?
    .edit
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_actions_recompute_edits_and_reject_stale_versions() {
        let uri = Url::parse("file:///refactor/main.tex").unwrap();
        let mut ws = Workspace {
            client: crate::project::state::ClientCapabilities {
                apply_edits: true,
                ..Default::default()
            },
            ..Default::default()
        };
        ws.documents
            .insert(uri.clone(), Document::new(&uri, "Hello".into(), Some(2)));
        let range = Range::new(Position::new(0, 0), Position::new(0, 5));
        let context = CodeActionContext {
            diagnostics: vec![],
            only: Some(vec![CodeActionKind::REFACTOR]),
            trigger_kind: None,
        };
        let actions = crate::editing::code_actions(&ws, &uri, range, &context);
        let CodeActionOrCommand::Command(command) = &actions[0] else {
            panic!()
        };
        assert_eq!(command.command, "latex.applyCodeAction");
        let request = command.arguments.as_ref().unwrap()[0].clone();
        assert!(resolve(&ws, serde_json::from_value(request.clone()).unwrap()).is_some());
        ws.documents
            .insert(uri.clone(), Document::new(&uri, "Changed".into(), Some(3)));
        assert!(resolve(&ws, serde_json::from_value(request.clone()).unwrap()).is_none());
        let mut forged = request;
        forged["version"] = 3.into();
        forged["title"] = "Arbitrary edit".into();
        assert!(resolve(&ws, serde_json::from_value(forged).unwrap()).is_none());
    }
}
