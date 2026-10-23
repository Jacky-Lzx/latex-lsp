//! Detect projects whose reference sections select independent databases.
use crate::{
    named_references::Context,
    signature::{literal_copy_target, source_alias, CommandSignatures},
    syntax,
    workspace::Document,
};
use std::collections::{BTreeMap, BTreeSet};

/// Until resource provenance is resolved per section, the project's union of
/// bibliography entries cannot certify a citation target or a complete rename.
/// Captured readers remain possible even after their original is redefined.
pub(crate) fn selects_resources(docs: &[&Document], signatures: &CommandSignatures<'_>) -> bool {
    let mut users = BTreeMap::<String, Vec<(String, bool)>>::new();
    for doc in docs {
        for entry in &doc.syntax.definitions {
            let edge = literal_copy_target(&doc.text.source, &doc.syntax, entry, false)
                .map(|(target, _)| (target, true))
                .or_else(|| {
                    source_alias(&doc.text.source, &doc.syntax, entry).map(|target| (target, false))
                });
            if let Some((target, copied)) = edge {
                users
                    .entry(target)
                    .or_default()
                    .push((entry.name.clone(), copied));
            }
        }
    }
    let mut captured = BTreeMap::<String, BTreeSet<&str>>::new();
    let mut work = 0usize;
    for builtin in ["newrefsection", "refsection", "begin"] {
        let mut pending = vec![(builtin.to_owned(), false)];
        let mut seen = BTreeSet::new();
        while let Some((name, copied)) = pending.pop() {
            work += 1;
            if work > 100_000 {
                return true;
            }
            if !seen.insert((name.clone(), copied)) {
                continue;
            }
            if copied {
                captured.entry(name.clone()).or_default().insert(builtin);
            }
            pending.extend(
                users
                    .get(&name)
                    .into_iter()
                    .flatten()
                    .map(|(name, copy)| (name.clone(), copied || *copy)),
            );
        }
    }
    for doc in docs {
        let source = &doc.text.source;
        let context = std::cell::OnceCell::new();
        let declarations = doc
            .syntax
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        let empty = BTreeSet::new();
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            let (name, span) = signatures
                .named_invocation(source, &doc.syntax, index, &empty, &declarations)
                .unwrap_or((&command.name, command.span.clone()));
            let possible = |builtin| {
                signatures.invokes_builtin(name, builtin)
                    || captured
                        .get(name)
                        .is_some_and(|names| names.contains(builtin))
            };
            let end = if possible("newrefsection") || possible("refsection") {
                span.end
            } else if possible("begin") && signatures.command_unchanged("refsection") {
                let Some(environment) = syntax::environment_name_argument(source, span.end)
                    .filter(|environment| environment.value == "refsection")
                else {
                    continue;
                };
                environment.outer_span.end
            } else {
                continue;
            };
            let at = syntax::skip_space(source, end);
            if source.as_bytes().get(at) != Some(&b'[') {
                continue;
            }
            if context
                .get_or_init(|| Context::new(doc, signatures))
                .operand(doc, command.span.start)
            {
                continue;
            }
            let argument = if doc.syntax.raw_resource_brackets.contains(&at) {
                syntax::verbatim_file_bracket(source, at)
            } else {
                syntax::bracket_argument(source, at)
            };
            let Some((argument, _)) = argument else {
                return true;
            };
            // Ordinary comments have already been identified by tokenization;
            // a raw resource percent must remain a filename character.
            if source[argument.span.clone()]
                .char_indices()
                .any(|(offset, ch)| {
                    let at = argument.span.start + offset;
                    let comment = doc
                        .syntax
                        .comments
                        .partition_point(|comment| comment.end <= at);
                    !ch.is_whitespace()
                        && !doc
                            .syntax
                            .comments
                            .get(comment)
                            .is_some_and(|comment| comment.contains(&at))
                })
            {
                return true;
            }
        }
    }
    false
}
