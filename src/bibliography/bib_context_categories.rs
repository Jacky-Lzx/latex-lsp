//! Category hints require the current source's global members to match AUX preload.
use crate::{
    parser::syntax,
    project::workspace::{Document, Workspace},
    symbols::commands::CommandSignatures,
    symbols::named_references::Context,
};
use lsp_types::Url;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn name(source: &str, argument: syntax::Argument) -> Option<String> {
    let value = syntax::single_key_argument(source, argument).value;
    syntax::is_static_reference_key(&value).then_some(value)
}

pub(crate) fn membership(source: &str, end: usize) -> Option<(String, std::ops::Range<usize>)> {
    let arguments = crate::bibliography::citations::arguments(
        source,
        end,
        crate::bibliography::citations::Style::Category,
    );
    if arguments.waiting.is_some()
        || arguments.fields.len() != 2
        || arguments.fields.iter().any(|field| !field.complete)
    {
        return None;
    }
    Some((
        name(source, arguments.fields[0].argument.clone())?,
        arguments.fields[1].argument.span.clone(),
    ))
}

pub(crate) fn certify(
    ws: &Workspace,
    root: &Url,
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
    members: &BTreeMap<String, BTreeSet<String>>,
    work: &mut usize,
) -> Option<BTreeMap<String, Vec<String>>> {
    // Do not silently skip potential updates in macro bodies, callbacks or
    // captured parameters. A stale empty AUX would otherwise falsely agree.
    for (_, doc) in docs {
        let source = &doc.text.source;
        *work += source.len() + doc.syntax.commands.len();
        if *work > 32 * 1024 * 1024 {
            return None;
        }
        let context = Context::new(doc, signatures);
        let positions = signatures.source_command_positions(source, &doc.syntax);
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            let (name, _) = signatures
                .named_invocation(
                    source,
                    &doc.syntax,
                    index,
                    &BTreeSet::new(),
                    &context.declarations,
                )
                .unwrap_or((&command.name, command.span.clone()));
            let canonical = signatures.forwarded_builtin(name).unwrap_or(name);
            if !matches!(canonical, "DeclareBibliographyCategory" | "addtocategory") {
                continue;
            }
            if doc
                .syntax
                .ignored
                .iter()
                .chain(&doc.syntax.literal_conversions)
                .any(|span| span.contains(&command.span.start))
            {
                continue;
            }
            *work += source.len() + doc.syntax.commands.len();
            if *work > 32 * 1024 * 1024
                || name != canonical
                || !signatures.command_unchanged(canonical)
                || !positions.contains(&command.span.start)
                || context.literal(command.span.start)
                || context.operand(doc, command.span.start)
                || signatures.resource_is_tokenized(source, &doc.syntax, command.span.start)
            {
                return None;
            }
        }
    }
    let (records, bytes) = crate::compilation::auxiliary::numeric_citation_records(ws, root)?;
    *work += bytes;
    let mut expected = BTreeSet::new();
    let mut by_key = BTreeMap::<String, Vec<String>>::new();
    for (category, keys) in members {
        for key in keys {
            *work += category.len() + key.len() + 128;
            if *work > 32 * 1024 * 1024 {
                return None;
            }
            expected.insert((category.clone(), key.clone()));
            by_key
                .entry(key.clone())
                .or_default()
                .push(category.clone());
        }
    }
    (records.categories.as_ref() == Some(&expected)).then_some(by_key)
}
