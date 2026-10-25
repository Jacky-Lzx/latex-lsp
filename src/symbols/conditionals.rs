//! Completion and expression operands for named booleans and etoolbox toggles.
use crate::{parser::syntax, project::workspace::Workspace};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::*;

#[derive(Clone, Copy, PartialEq)]
enum ArgumentKind {
    Assignable,
    Test,
    Value,
    Toggle,
}

pub(crate) fn expression(name: &str) -> bool {
    matches!(
        name,
        "ifboolexpr" | "ifboolexpe" | "whileboolexpr" | "unlessboolexpr"
    )
}

/// TeX removes a comment's endline and the next row's indentation, but
/// ordinary spaces and blank rows must not join two parts of a keyword.
fn skip_keyword_comments(source: &str, mut at: usize) -> usize {
    while source.as_bytes().get(at) == Some(&b'%') {
        at = source[at..]
            .find('\n')
            .map_or(source.len(), |end| at + end + 1);
        while matches!(source.as_bytes().get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
    }
    at
}

pub(crate) fn keyword_end(source: &str, mut at: usize, keyword: &str) -> Option<usize> {
    for byte in keyword.bytes() {
        at = skip_keyword_comments(source, at);
        if source.as_bytes().get(at) != Some(&byte) {
            return None;
        }
        at += 1;
    }
    Some(at)
}

fn operand_token(token: &str) -> Option<bool> {
    let mut at = 0;
    loop {
        for (keyword, toggle) in [("bool", false), ("togl", true)] {
            if keyword_end(token, at, keyword)
                .is_some_and(|end| skip_keyword_comments(token, end) == token.len())
            {
                return Some(toggle);
            }
        }
        at = ["not", "and", "or"]
            .into_iter()
            .find_map(|keyword| keyword_end(token, at, keyword))?;
    }
}

fn operand_starts(
    source: &str,
    command: &syntax::Command,
    cursor: Option<usize>,
) -> Vec<(usize, bool)> {
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
                    i = skip_keyword_comments(&source[..end], i);
                    if i == end {
                        break;
                    }
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
                if let Some(toggle) = operand_token(&source[start..i]) {
                    let argument = syntax::skip_space(source, i);
                    if source.as_bytes().get(argument) == Some(&b'{') && argument < end {
                        result.push((argument, toggle));
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
    toggle: bool,
) -> Vec<syntax::Argument> {
    operand_starts(source, command, None)
        .into_iter()
        .filter(|(_, operand_toggle)| *operand_toggle == toggle)
        .filter_map(|(start, _)| {
            syntax::group(source, start, b'{', b'}').map(|(argument, _)| argument)
        })
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
        .filter(|command| {
            command.span.end <= cursor && !doc.syntax.control_operands.contains(&command.span.start)
        })
        .take(128)
        .filter(|command| {
            syntax::boolean_reference(&command.name)
                || syntax::toggle_reference(&command.name)
                || expression(&command.name)
        })
        .find_map(|command| {
            if expression(&command.name) {
                return operand_starts(source, command, Some(cursor))
                    .into_iter()
                    .rev()
                    .find_map(|(start, toggle)| {
                        syntax::active_static_argument(source, start, b'{', cursor).map(|span| {
                            (
                                span,
                                if toggle {
                                    ArgumentKind::Toggle
                                } else {
                                    ArgumentKind::Test
                                },
                            )
                        })
                    });
            }
            let start = syntax::skip_space(source, command.span.end);
            if let Some(span) = syntax::active_static_argument(source, start, b'{', cursor) {
                let kind = if syntax::toggle_reference(&command.name) {
                    ArgumentKind::Toggle
                } else if matches!(command.name.as_str(), "boolean" | "ifbool" | "notbool") {
                    ArgumentKind::Test
                } else {
                    ArgumentKind::Assignable
                };
                return Some((span, kind));
            }
            if !matches!(
                command.name.as_str(),
                "setboolean" | "setbool" | "settoggle"
            ) {
                return None;
            }
            let (_, end) = syntax::group(source, start, b'{', b'}')?;
            syntax::active_static_argument(source, syntax::skip_space(source, end), b'{', cursor)
                .map(|span| (span, ArgumentKind::Value))
        })?;
    let mut candidates = BTreeMap::new();
    // Primitive iftrue/iffalse have no setters. Offer them for tests and
    // values, while assignments require an explicitly declared boolean.
    if matches!(kind, ArgumentKind::Test | ArgumentKind::Value) {
        candidates.insert("true".to_owned(), "True boolean value".to_owned());
        candidates.insert("false".to_owned(), "False boolean value".to_owned());
    }
    if kind != ArgumentKind::Value {
        for (uri, doc) in &workspace.documents {
            if !scope.contains(uri) {
                continue;
            }
            let entries = if kind == ArgumentKind::Toggle {
                &doc.syntax.toggles
            } else {
                &doc.syntax.conditionals
            };
            for entry in entries {
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
