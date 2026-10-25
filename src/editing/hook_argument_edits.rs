//! Hook parameters are tokens, not necessarily executable source references.
use crate::{
    language::signature::{self, CommandSignatures},
    parser::syntax,
    project::workspace::{Document, DocumentKind, Workspace},
    rpc::{Error, Result},
};
use lsp_types::{Location, Url};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

const USES: &[&str] = &[
    "UseHook",
    "UseOneTimeHook",
    "UseHookWithArguments",
    "UseOneTimeHookWithArguments",
];
const DECLARATIONS: &[&str] = &[
    "NewHook",
    "NewReversedHook",
    "NewMirroredHookPair",
    "NewHookWithArguments",
    "NewReversedHookWithArguments",
    "NewMirroredHookPairWithArguments",
];

fn error() -> Error {
    Error::invalid_params("Cannot rename a reference passed as hook argument data without a confirmed executable identity")
}

fn argument(source: &str, after: usize) -> Option<(Range<usize>, usize)> {
    let at = syntax::skip_space(source, after);
    if source.as_bytes().get(at) == Some(&b'{') {
        return Some(
            syntax::group(source, at, b'{', b'}')
                .map_or((at + 1..source.len(), source.len()), |(argument, end)| {
                    (argument.span, end)
                }),
        );
    }
    let value = syntax::required_token(source, at)?;
    Some((at..at + value.len(), at + value.len()))
}

fn count(value: &str) -> usize {
    // The native interface supports at most nine parameters. Expressions or
    // parameter tokens do not justify selecting a smaller data boundary.
    value
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|&n| n <= 9)
        .unwrap_or(9)
}

fn static_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 512
        && name != "."
        && !name.starts_with("./")
        && !name.contains(['\\', '{', '}', '%', '#', '$', '&', '^', '~'])
        && !name
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\r' | '\n'))
}

struct Call {
    name: &'static str,
    after: usize,
}

fn calls(
    doc: &Document,
    signatures: &CommandSignatures<'_>,
    possible: &signature::StoredCodeAliases,
) -> Vec<Call> {
    let context = crate::symbols::named_references::Context::new(doc, signatures);
    let empty = BTreeSet::new();
    let mut result = Vec::new();
    for (index, command) in doc.syntax.commands.iter().enumerate() {
        if context.literal(command.span.start)
            || doc.syntax.control_operands.contains(&command.span.start)
            || context
                .declarations
                .contains(&(command.span.start, command.span.end))
        {
            continue;
        }
        let (name, invocation) = signatures
            .named_invocation(
                &doc.text.source,
                &doc.syntax,
                index,
                &empty,
                &context.declarations,
            )
            .unwrap_or((&command.name, command.span.clone()));
        for &candidate in possible.get(name).into_iter().flatten() {
            // A unique ordinary user definition is not the native interface.
            // Copies stay possible readers even after their source changes.
            if !signatures.invokes_builtin(name, candidate)
                && signatures.has_custom_signature(name, candidate)
            {
                continue;
            }
            result.push(Call {
                name: candidate,
                after: invocation.end,
            });
        }
    }
    result
}

pub(crate) fn validate(
    workspace: &Workspace,
    scope: &BTreeSet<Url>,
    occurrences: &[Location],
) -> Result<()> {
    if !workspace.documents.iter().any(|(uri, doc)| {
        scope.contains(uri)
            && (USES.iter().any(|name| doc.text.source.contains(name))
                || doc.text.source.contains("HookWithArguments")
                || doc.text.source.contains("HookPairWithArguments"))
    }) {
        return Ok(());
    }
    let mut work = 0usize;
    for (project, consumers) in crate::symbols::environment_calls::project_contexts(workspace) {
        if project.is_disjoint(scope) {
            continue;
        }
        let docs: Vec<_> = project
            .iter()
            .filter_map(|uri| workspace.documents.get(uri))
            .filter(|doc| doc.kind == DocumentKind::Latex)
            .collect();
        work = work.saturating_add(
            docs.iter()
                .map(|doc| {
                    doc.text.source.len()
                        + doc.syntax.commands.len() * 64
                        + doc.syntax.definitions.len() * 64
                })
                .sum::<usize>(),
        );
        if work > 32 * 1024 * 1024 || !workspace.index_truncated.is_empty() {
            return Err(error());
        }
        let possible = signature::possible_builtin_aliases(
            docs.iter()
                .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
            USES.iter().chain(DECLARATIONS).copied(),
        )
        .ok_or_else(error)?;
        let signatures = CommandSignatures::new(
            docs.iter()
                .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
        );
        let mut arities = BTreeMap::<String, usize>::from([("label".into(), 1)]);
        let mut dynamic = false;
        let mut parsed = BTreeMap::new();
        for uri in &project {
            let Some(doc) = workspace
                .documents
                .get(uri)
                .filter(|doc| doc.kind == DocumentKind::Latex)
            else {
                continue;
            };
            let invocations = calls(doc, &signatures, &possible);
            for call in invocations
                .iter()
                .filter(|call| DECLARATIONS.contains(&call.name))
            {
                let mut after = call.after;
                let mut names = Vec::new();
                for _ in 0..if call.name.starts_with("NewMirrored") {
                    2
                } else {
                    1
                } {
                    let Some((span, end)) = argument(&doc.text.source, after) else {
                        dynamic = true;
                        break;
                    };
                    let name = doc.text.source[span].trim_matches([' ', '\t', '\r', '\n']);
                    if static_name(name) {
                        names.push(name.to_owned());
                    } else {
                        dynamic = true;
                    }
                    after = end;
                }
                let arity = if call.name.ends_with("WithArguments") {
                    argument(&doc.text.source, after)
                        .map_or(9, |(span, _)| count(&doc.text.source[span]))
                } else {
                    0
                };
                for name in names {
                    arities
                        .entry(name)
                        .and_modify(|n| *n = (*n).max(arity))
                        .or_insert(arity);
                }
            }
            parsed.insert(uri, invocations);
        }
        for uri in consumers.intersection(scope) {
            let Some(doc) = workspace.documents.get(uri) else {
                continue;
            };
            let Some(invocations) = parsed.get(&uri) else {
                continue;
            };
            let locations: Vec<_> = occurrences
                .iter()
                .filter(|location| location.uri == *uri)
                .filter_map(|location| doc.text.offset(location.range.start))
                .collect();
            if locations.is_empty() {
                continue;
            }
            for call in invocations.iter().filter(|call| USES.contains(&call.name)) {
                let Some((name, mut after)) = argument(&doc.text.source, call.after) else {
                    continue;
                };
                let raw = &doc.text.source[name];
                let name = if call.name.starts_with("UseOneTime") {
                    raw.trim_matches([' ', '\t', '\r', '\n'])
                } else {
                    raw
                };
                let mut n = if dynamic {
                    9
                } else if !static_name(name) {
                    arities.values().copied().max().unwrap_or(0)
                } else {
                    arities.get(name).copied().unwrap_or(0)
                };
                if call.name.ends_with("WithArguments") {
                    let Some((span, end)) = argument(&doc.text.source, after) else {
                        continue;
                    };
                    n = n.max(count(&doc.text.source[span]));
                    after = end;
                }
                let start = after;
                for _ in 0..n {
                    let Some((_, end)) = argument(&doc.text.source, after) else {
                        // Unknown tokenization cannot justify editing later
                        // source as if it were outside the data operands.
                        after = doc.text.source.len();
                        break;
                    };
                    after = end;
                }
                if after == start {
                    continue;
                }
                for &at in &locations {
                    // A hook may consume only the command token of a label or
                    // reference, leaving that token's apparent arguments as text.
                    let command = doc
                        .syntax
                        .commands
                        .partition_point(|command| command.span.start <= at)
                        .checked_sub(1)
                        .map(|index| &doc.syntax.commands[index]);
                    if (start..after).contains(&at)
                        || command
                            .is_some_and(|command| (start..after).contains(&command.span.start))
                    {
                        return Err(error());
                    }
                }
            }
        }
    }
    Ok(())
}
