//! Conservative structural actions and deterministic source fixes.
use crate::{
    features, syntax,
    workspace::{Document, DocumentKind, Workspace},
};
use std::collections::BTreeMap;
use tower_lsp::lsp_types::*;

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
    let fixes = crate::environment_diagnostics::fixes(ws, uri, doc);
    let mut unique: Vec<&syntax::QuickFix> = Vec::new();
    for fix in &fixes {
        if fix.diagnostic.source.as_deref() != Some("latex-lsp") {
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
fn selection(doc: &Document, range: Range) -> Option<(usize, usize, Option<usize>)> {
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
    let begin = doc.syntax.commands.iter().find(|cmd| {
        cmd.name == "begin"
            && cmd
                .argument
                .as_ref()
                .is_some_and(|arg| arg.value == "document")
    });
    let insertion = begin.map(|cmd| cmd.span.start);
    if let Some(begin) = begin {
        if start < begin.argument.as_ref()?.outer_span.end {
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
    let mut depth = 0i32;
    let mut math = false;
    let mut environments = Vec::new();
    let mut at = 0;
    let bytes = source.as_bytes();
    while at < start {
        if let Some(ignored) = doc.syntax.ignored.iter().find(|span| span.contains(&at)) {
            at = ignored.end;
            continue;
        }
        if bytes[at] == b'\\' {
            if let Some(cmd) = doc.syntax.commands.iter().find(|cmd| cmd.span.start == at) {
                if matches!(
                    cmd.name.as_str(),
                    "catcode" | "ExplSyntaxOn" | "verb" | "lstinline" | "mintinline"
                ) {
                    return None;
                }
                if matches!(cmd.name.as_str(), "(" | "[" | ")" | "]") {
                    math = matches!(cmd.name.as_str(), "(" | "[");
                }
                if matches!(cmd.name.as_str(), "begin" | "end") {
                    if let Some(arg) = &cmd.argument {
                        if cmd.name == "begin" {
                            environments.push(arg.value.as_str());
                        } else if environments.pop() != Some(arg.value.as_str()) {
                            return None;
                        }
                    }
                }
                if cmd.span.end > start {
                    return None;
                }
                at = cmd.span.end;
                continue;
            }
            // Control symbols are not all retained in the command index.
            let next = source[at + 1..].chars().next()?;
            if matches!(next, '(' | '[') {
                math = true;
            }
            if matches!(next, ')' | ']') {
                math = false;
            }
            at += 1 + next.len_utf8();
            continue;
        }
        match bytes[at] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'$' => math = !math,
            _ => {}
        }
        if depth < 0 {
            return None;
        }
        at += 1;
    }
    if (begin.is_some() && !environments.contains(&"document"))
        || at != start
        || depth != 0
        || math
        || environments.iter().any(|name| *name != "document")
    {
        return None;
    }
    // Independently balance the selected text, ignoring control symbols.
    let mut at = start;
    while at < end {
        if bytes[at] == b'\\' {
            if let Some(cmd) = doc.syntax.commands.iter().find(|cmd| cmd.span.start == at) {
                if cmd.span.end > end {
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
    if !context.diagnostics.is_empty() {
        return result;
    }
    let Some((start, end, insertion)) = selection(doc, range) else {
        return result;
    };
    let selected = &doc.text.source[start..end];
    if allows(context, &CodeActionKind::REFACTOR_REWRITE) {
        for (title, replacement) in [
            ("Wrap selection in \\emph", format!("\\emph{{{selected}}}")),
            (
                "Wrap selection in \\textbf",
                format!("\\textbf{{{selected}}}"),
            ),
            (
                "Wrap selection in quote environment",
                format!(
                    "\\begin{{quote}}{}{selected}{}\\end{{quote}}",
                    doc.text.newline(),
                    doc.text.newline()
                ),
            ),
        ] {
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
    if allows(context, &CodeActionKind::REFACTOR_EXTRACT) {
        if let Some(insertion) = insertion {
            let scope = ws.symbol_scope(uri);
            let used = |name: &str| {
                ws.documents
                    .iter()
                    .filter(|(uri, _)| scope.contains(*uri))
                    .any(|(_, doc)| doc.syntax.commands.iter().any(|cmd| cmd.name == name))
            };
            let name = (0..26)
                .map(|index| {
                    if index == 0 {
                        "extractedcommand".into()
                    } else {
                        format!("extractedcommand{}", char::from(b'a' + index))
                    }
                })
                .find(|name| !used(name));
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
            apply_edits: true,
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
