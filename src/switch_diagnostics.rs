//! Verify etoolbox boolean and toggle operations without expanding TeX.
use crate::{signature::CommandSignatures, syntax};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Boolean,
    Toggle,
}

impl Kind {
    fn command(self, name: &str) -> bool {
        match self {
            Self::Toggle => syntax::toggle_reference(name),
            Self::Boolean => matches!(
                name,
                "setbool" | "booltrue" | "boolfalse" | "ifbool" | "notbool"
            ),
        }
    }
}

pub(crate) fn boolean_test(
    doc: &crate::workspace::Document,
    range: tower_lsp::lsp_types::Range,
) -> bool {
    doc.syntax.commands.iter().any(|command| {
        (matches!(command.name.as_str(), "ifbool" | "notbool")
            && command
                .argument
                .as_ref()
                .is_some_and(|name| doc.text.range(name.span.start, name.span.end) == range))
            || crate::conditionals::expression(&command.name)
                && crate::conditionals::expression_arguments(&doc.text.source, command, false)
                    .iter()
                    .any(|name| doc.text.range(name.span.start, name.span.end) == range)
    })
}

pub(crate) fn error_invocations_with_signatures(
    source: &str,
    parsed: &syntax::Syntax,
    signatures: &CommandSignatures<'_>,
    kind: Kind,
    expression: bool,
) -> Vec<(Range<usize>, Range<usize>)> {
    let operands = signatures.argument_tokens(source, parsed);
    let overridden: BTreeSet<_> = signatures.names().collect();
    if expression {
        return parsed
            .commands
            .iter()
            .filter(|command| {
                crate::conditionals::expression(&command.name)
                    && !operands.contains(&command.span.start)
                    && !overridden.contains(command.name.as_str())
            })
            .flat_map(|command| {
                let Some(argument) = command.argument.as_ref() else {
                    return vec![];
                };
                let Some(names) = expression_arguments(source, argument.span.clone(), &overridden)
                else {
                    return vec![];
                };
                let start = syntax::skip_space(source, command.span.end);
                let mut end = argument.outer_span.end;
                if matches!(command.name.as_str(), "whileboolexpr" | "unlessboolexpr") {
                    let Some((_, after)) =
                        syntax::group(source, syntax::skip_space(source, end), b'{', b'}')
                    else {
                        return vec![];
                    };
                    end = after;
                }
                let mut counts = BTreeMap::new();
                for (namespace, span) in &names {
                    if *namespace == kind {
                        *counts.entry(&source[span.clone()]).or_insert(0) += 1;
                    }
                }
                names
                    .into_iter()
                    .filter(|(namespace, span)| {
                        *namespace == kind && counts.get(&source[span.clone()]) == Some(&1)
                    })
                    .map(|(_, span)| (span, start..end))
                    .collect()
            })
            .collect();
    }
    parsed
        .commands
        .iter()
        .filter(|command| {
            kind.command(&command.name)
                && !operands.contains(&command.span.start)
                && !overridden.contains(command.name.as_str())
        })
        .filter_map(|command| {
            let start = syntax::skip_space(source, command.span.end);
            let (name, mut end) = syntax::group(source, start, b'{', b'}')?;
            if !syntax::is_static_key(&source[name.span.clone()]) {
                return None;
            }
            // The setters consume their value before reporting an error;
            // conditional tests consume their branches only after it.
            if matches!(command.name.as_str(), "settoggle" | "setbool") {
                end = syntax::group(source, syntax::skip_space(source, end), b'{', b'}')?.1;
            }
            Some((name.span, start..end))
        })
        .collect()
}

/// Validate the whole expression: a dynamic name or test can raise the same
/// error as a static operand without adding a user macro frame to the log.
fn expression_arguments(
    source: &str,
    span: Range<usize>,
    overridden: &BTreeSet<&str>,
) -> Option<Vec<(Kind, Range<usize>)>> {
    if span.len() > 16_384 {
        return None;
    }
    let source = &source[..span.end];
    let mut at = span.start;
    let mut depth = 0;
    let mut operand = true;
    let mut names = vec![];
    loop {
        at = syntax::skip_space(source, at);
        if at == span.end {
            break;
        }
        let rest = &source[at..];
        if operand {
            if let Some(end) = crate::conditionals::keyword_end(source, at, "not") {
                at = end;
                continue;
            }
            if rest.starts_with('(') {
                depth += 1;
                if depth > 128 {
                    return None;
                }
                at += 1;
                continue;
            }
            let (after, kind) = [
                ("bool", Some(Kind::Boolean)),
                ("togl", Some(Kind::Toggle)),
                ("test", None),
            ]
            .into_iter()
            .find_map(|(keyword, kind)| {
                crate::conditionals::keyword_end(source, at, keyword).map(|end| (end, kind))
            })?;
            let (argument, end) =
                syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?;
            if let Some(kind) = kind {
                if !syntax::is_static_key(&source[argument.span.clone()]) || names.len() >= 256 {
                    return None;
                }
                names.push((kind, argument.span));
            } else if !literal_test(source, argument.span, overridden) {
                return None;
            }
            at = end;
            operand = false;
        } else if rest.starts_with(')') && depth > 0 {
            depth -= 1;
            at += 1;
        } else {
            at = crate::conditionals::keyword_end(source, at, "and")
                .or_else(|| crate::conditionals::keyword_end(source, at, "or"))?;
            operand = true;
        }
    }
    (!operand && depth == 0).then_some(names)
}

fn literal_test(source: &str, span: Range<usize>, overridden: &BTreeSet<&str>) -> bool {
    let source = &source[..span.end];
    let Some(control) = syntax::control_word(source, syntax::skip_space(source, span.start)) else {
        return false;
    };
    let name = &source[control.start + 1..control.end];
    if overridden.contains(name) {
        return false;
    }
    let Some(count) = crate::string_tests::data_count(name) else {
        return false;
    };
    // These predicates do not expand their string arguments. Controls such
    // as input/ifbool are data here and cannot raise a sibling's name error.
    let mut at = control.end;
    for _ in 0..count {
        let Some((_, end)) = syntax::group(source, syntax::skip_space(source, at), b'{', b'}')
        else {
            return false;
        };
        at = end;
    }
    syntax::skip_space(source, at) == span.end
}
