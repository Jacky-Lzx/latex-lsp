//! Require complete, executable identities before editing PDF destination keys.
use crate::{
    anchors,
    named_references::Context,
    signature::{literal_copy_target, source_alias, CommandSignatures},
    syntax,
    workspace::{Document, DocumentKind, Workspace},
};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet},
};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::Url,
};

const COMMANDS: &[&str] = &[
    "hypertarget",
    "hyperlink",
    "hyperdef",
    "hyperref",
    "pdfbookmark",
];
type Potential = BTreeMap<String, BTreeSet<&'static str>>;

/// Follow copies as possible readers even when the public command has since
/// changed. Possibility only blocks edits; it never installs a symbol meaning.
fn possible_readers(docs: &[&Document]) -> Result<Potential> {
    let mut users = BTreeMap::<String, Vec<String>>::new();
    for doc in docs {
        for entry in &doc.syntax.definitions {
            let target = source_alias(&doc.text.source, &doc.syntax, entry).or_else(|| {
                literal_copy_target(&doc.text.source, &doc.syntax, entry, false)
                    .map(|(target, _)| target)
            });
            if let Some(target) = target {
                users.entry(target).or_default().push(entry.name.clone());
            }
        }
    }
    let mut result = Potential::new();
    let mut pending: Vec<_> = COMMANDS
        .iter()
        .map(|&name| (name.to_owned(), name))
        .collect();
    let mut work = 0usize;
    let mut bytes = 0usize;
    while let Some((name, builtin)) = pending.pop() {
        work += 1;
        bytes += name.len();
        if work > 100_000 || bytes > 32 * 1024 * 1024 {
            return Err(Error::invalid_params(
                "PDF destination rename exceeds the alias analysis budget",
            ));
        }
        if !result.entry(name.clone()).or_default().insert(builtin) {
            continue;
        }
        pending.extend(
            users
                .get(&name)
                .into_iter()
                .flatten()
                .map(|user| (user.clone(), builtin)),
        );
    }
    Ok(result)
}

pub(crate) fn validate(workspace: &Workspace, scope: &BTreeSet<Url>, name: &str) -> Result<()> {
    let docs: Vec<_> = workspace
        .documents
        .iter()
        .filter(|(uri, doc)| scope.contains(*uri) && doc.kind == DocumentKind::Latex)
        .map(|(_, doc)| doc)
        .collect();
    if docs.iter().map(|doc| doc.text.source.len()).sum::<usize>() > 16 * 1024 * 1024
        || docs
            .iter()
            .map(|doc| doc.syntax.commands.len())
            .sum::<usize>()
            > 100_000
        || docs
            .iter()
            .map(|doc| doc.syntax.definitions.len())
            .sum::<usize>()
            > 4096
    {
        return Err(Error::invalid_params(
            "PDF destination rename exceeds the source analysis budget",
        ));
    }
    let possible = possible_readers(&docs)?;
    let signatures = CommandSignatures::new(
        docs.iter()
            .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
    );
    let empty = BTreeSet::new();
    for doc in docs {
        let source = &doc.text.source;
        let context = Context::new(doc, &signatures);
        let declaration_operands = OnceCell::new();
        for (index, lexical) in doc.syntax.commands.iter().enumerate() {
            if context.literal(lexical.span.start) {
                continue;
            }
            let (called, span) = signatures
                .named_invocation(source, &doc.syntax, index, &empty, &context.declarations)
                .unwrap_or((&lexical.name, lexical.span.clone()));
            for &builtin in possible.get(called).into_iter().flatten() {
                let command = syntax::Command {
                    name: builtin.into(),
                    argument: syntax::group(
                        source,
                        syntax::skip_space(source, span.end),
                        b'{',
                        b'}',
                    )
                    .map(|(argument, _)| argument),
                    span: span.clone(),
                };
                let Some((declaration, entry)) = anchors::entry(source, &command) else {
                    continue;
                };
                if entry.name != name {
                    continue;
                }
                // These calls have no certified occurrence in the editable
                // destination index. Omitting them would break a real link.
                if lexical.name != builtin || span != lexical.span {
                    return Err(Error::invalid_params("Rename is incomplete: a PDF destination is used through an unverified indirect command"));
                }
                let operand = if declaration {
                    context.operand_with(
                        doc,
                        lexical.span.start,
                        declaration_operands.get_or_init(|| {
                            signatures.declaration_argument_tokens(source, &doc.syntax)
                        }),
                    )
                } else {
                    context.operand(doc, lexical.span.start)
                };
                if operand || !signatures.command_unchanged(builtin) {
                    return Err(Error::invalid_params("Cannot rename a PDF destination interpreted as parameter data or by a redefined command"));
                }
            }
        }
    }
    Ok(())
}
