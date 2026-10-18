//! Completion for the named boolean APIs of ifthen and etoolbox.
use crate::{syntax, workspace::Workspace};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::*;

#[derive(Clone, Copy, PartialEq)]
enum ArgumentKind {
    Assignable,
    Test,
    Value,
}

fn expression(name: &str) -> bool {
    matches!(
        name,
        "ifboolexpr" | "ifboolexpe" | "whileboolexpr" | "unlessboolexpr"
    )
}

fn boolean_token(mut token: &str) -> bool {
    loop {
        if token == "bool" {
            return true;
        }
        let Some(rest) = token
            .strip_prefix("not")
            .or_else(|| token.strip_prefix("and"))
            .or_else(|| token.strip_prefix("or"))
        else {
            return false;
        };
        token = rest;
    }
}

fn operand_starts(source: &str, command: &syntax::Command, cursor: Option<usize>) -> Vec<usize> {
    if !expression(&command.name) {
        return vec![];
    }
    let (mut i, end) = if let Some(argument) = &command.argument {
        (
            argument.span.start,
            cursor.map_or(argument.span.end, |cursor| cursor.min(argument.span.end)),
        )
    } else if let Some(cursor) = cursor {
        let start = syntax::skip_space(source, command.span.end);
        if source.as_bytes().get(start) != Some(&b'{') || cursor <= start {
            return vec![];
        }
        (start + 1, cursor)
    } else {
        return vec![];
    };
    let mut result = Vec::new();
    while i < end {
        match source.as_bytes()[i] {
            b'%' => {
                i = source[i..end]
                    .find('\n')
                    .map_or(end, |offset| i + offset + 1);
            }
            b'\\' => {
                i += 1;
                if i < end {
                    let start = i;
                    while i < end
                        && (source.as_bytes()[i].is_ascii_alphabetic()
                            || source.as_bytes()[i] == b'@')
                    {
                        i += 1;
                    }
                    if i == start {
                        i += source[i..].chars().next().unwrap().len_utf8();
                    }
                }
            }
            b'{' => {
                i = syntax::group(source, i, b'{', b'}').map_or(end, |(_, after)| after.min(end));
            }
            _ => {
                let start = i;
                while i < end {
                    let ch = source[i..].chars().next().unwrap();
                    if !ch.is_alphanumeric() && !matches!(ch, '@' | '_' | '-') {
                        break;
                    }
                    i += ch.len_utf8();
                }
                if i == start {
                    i += source[i..].chars().next().unwrap().len_utf8();
                    continue;
                }
                if boolean_token(&source[start..i]) {
                    let argument = syntax::skip_space(source, i);
                    if source.as_bytes().get(argument) == Some(&b'{') && argument < end {
                        result.push(argument);
                        i = syntax::group(source, argument, b'{', b'}')
                            .map_or(end, |(_, after)| after.min(end));
                    }
                }
            }
        }
    }
    result
}

pub(crate) fn expression_arguments(
    source: &str,
    command: &syntax::Command,
) -> Vec<syntax::Argument> {
    operand_starts(source, command, None)
        .into_iter()
        .filter_map(|start| syntax::group(source, start, b'{', b'}').map(|(argument, _)| argument))
        .collect()
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &Url,
    cursor: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = workspace.documents.get(uri)?;
    let source = &doc.text.source;
    let (span, kind) = doc
        .syntax
        .commands
        .iter()
        .rev()
        .filter(|command| command.span.end <= cursor)
        .take(128)
        .filter(|command| syntax::boolean_reference(&command.name) || expression(&command.name))
        .find_map(|command| {
            if expression(&command.name) {
                return operand_starts(source, command, Some(cursor))
                    .into_iter()
                    .rev()
                    .find_map(|start| syntax::active_static_argument(source, start, b'{', cursor))
                    .map(|span| (span, ArgumentKind::Test));
            }
            let start = syntax::skip_space(source, command.span.end);
            if let Some(span) = syntax::active_static_argument(source, start, b'{', cursor) {
                let kind = if matches!(command.name.as_str(), "boolean" | "ifbool" | "notbool") {
                    ArgumentKind::Test
                } else {
                    ArgumentKind::Assignable
                };
                return Some((span, kind));
            }
            if !matches!(command.name.as_str(), "setboolean" | "setbool") {
                return None;
            }
            let (_, end) = syntax::group(source, start, b'{', b'}')?;
            syntax::active_static_argument(source, syntax::skip_space(source, end), b'{', cursor)
                .map(|span| (span, ArgumentKind::Value))
        })?;
    let mut candidates = BTreeMap::new();
    // Primitive iftrue/iffalse have no setters. Offer them for tests and
    // values, while assignments require an explicitly declared boolean.
    if kind != ArgumentKind::Assignable {
        candidates.insert("true".to_owned(), "True boolean value".to_owned());
        candidates.insert("false".to_owned(), "False boolean value".to_owned());
    }
    if kind != ArgumentKind::Value {
        for (uri, doc) in &workspace.documents {
            if !scope.contains(uri) {
                continue;
            }
            for entry in &doc.syntax.conditionals {
                candidates.insert(
                    entry.name.clone(),
                    format!("{} — {}", entry.detail, uri.path()),
                );
            }
        }
    }
    let prefix = &source[span.start..cursor];
    let range = doc.text.range(span.start, span.end);
    Some(
        candidates
            .into_iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, detail)| CompletionItem {
                label: name.clone(),
                detail: Some(detail),
                kind: Some(if kind == ArgumentKind::Value {
                    CompletionItemKind::VALUE
                } else {
                    CompletionItemKind::VARIABLE
                }),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: name,
                })),
                ..Default::default()
            })
            .collect(),
    )
}
