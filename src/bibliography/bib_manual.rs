//! Native handwritten bibliography entries do not define biblatex data keys.
use crate::{
    parser::syntax, project::workspace::Document, symbols::commands::CommandSignatures,
    symbols::named_references::Context,
};
use lsp_types::Url;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn needed(docs: &[&Document], signatures: &CommandSignatures<'_>) -> bool {
    if !docs.iter().any(|doc| doc.text.source.contains("bibitem")) {
        return false;
    }
    let mut backend = false;
    let mut manual = false;
    for doc in docs {
        let context = Context::new(doc, signatures);
        let positions = signatures.source_command_positions(&doc.text.source, &doc.syntax);
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            if context.literal(command.span.start) {
                continue;
            }
            let (name, span) = signatures
                .named_invocation(
                    &doc.text.source,
                    &doc.syntax,
                    index,
                    &BTreeSet::new(),
                    &context.declarations,
                )
                .unwrap_or((&command.name, command.span.clone()));
            manual |= name == "bibitem" || signatures.invokes_builtin(name, "bibitem");
            let name = signatures.forwarded_builtin(name).unwrap_or(name);
            if !positions.contains(&command.span.start)
                || context.operand(doc, command.span.start)
                || !signatures.command_unchanged(name)
            {
                continue;
            }
            backend |= matches!(name, "addbibresource" | "addglobalbib" | "addsectionbib");
            if syntax::loader_extension(name) == Some("sty") {
                let resolved = syntax::Command {
                    name: name.into(),
                    span,
                    argument: command.argument.clone(),
                };
                backend |=
                    syntax::file_argument(&doc.text.source, &resolved).is_some_and(|argument| {
                        argument
                            .value
                            .split(',')
                            .any(|name| syntax::loader_name(name) == "biblatex")
                    });
            }
        }
    }
    backend && manual
}

pub(crate) fn items(
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
) -> Option<BTreeMap<Url, BTreeSet<(usize, usize)>>> {
    if !signatures.command_unchanged("bibitem")
        || !needed(
            &docs.iter().map(|(_, doc)| *doc).collect::<Vec<_>>(),
            signatures,
        )
    {
        return None;
    }
    let mut result = BTreeMap::new();
    let mut stored = 0usize;
    for (uri, doc) in docs {
        let source = &doc.text.source;
        let context = Context::new(doc, signatures);
        let positions = signatures.source_command_positions(source, &doc.syntax);
        let mut spans = BTreeSet::new();
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            if context.literal(command.span.start) {
                continue;
            }
            let (name, span) = signatures
                .named_invocation(
                    source,
                    &doc.syntax,
                    index,
                    &BTreeSet::new(),
                    &context.declarations,
                )
                .unwrap_or((&command.name, command.span.clone()));
            if name != "bibitem" {
                if signatures.invokes_builtin(name, "bibitem") {
                    return None;
                }
                continue;
            }
            if !positions.contains(&command.span.start) && !context.operand(doc, command.span.start)
            {
                return None;
            }
            let mut at = syntax::skip_space(source, span.end);
            if source.as_bytes().get(at) == Some(&b'[') {
                at = syntax::skip_space(source, syntax::optional_note_end(source, at)?);
            }
            let (argument, _) = syntax::group(source, at, b'{', b'}')?;
            let key = syntax::single_citation_argument(source, argument);
            if !syntax::is_static_citation_key(&key.value) || key.value == "*" {
                return None;
            }
            stored += uri.as_str().len() + key.value.len() + 96;
            if stored > 32 * 1024 * 1024 || spans.len() >= 100_000 {
                return None;
            }
            spans.insert((key.span.start, key.span.end));
        }
        result.insert(uri.clone(), spans);
    }
    Some(result)
}
