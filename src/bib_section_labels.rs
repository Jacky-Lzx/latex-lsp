//! Read-only names of statically registered biblatex resources.
use crate::{
    bib_section_files::{self, LabelBinding},
    named_references::Context,
    signature::CommandSignatures,
    syntax::{self, Argument, Command},
    workspace::{Document, DocumentKind, Workspace},
};
use std::{collections::BTreeSet, ops::Range};
use tower_lsp::lsp_types::*;

struct Query {
    command: usize,
    item: Range<usize>,
    selection: bool,
}

fn query(doc: &Document, signatures: &CommandSignatures<'_>, offset: usize) -> Option<Query> {
    let source = &doc.text.source;
    let context = Context::new(doc, signatures);
    let positions = signatures.source_command_positions(source, &doc.syntax);
    for (index, lexical) in doc.syntax.commands.iter().enumerate() {
        if !positions.contains(&lexical.span.start)
            || context.literal(lexical.span.start)
            || context.operand(doc, lexical.span.start)
        {
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
            .unwrap_or((&lexical.name, lexical.span.clone()));
        let name = signatures.forwarded_builtin(name).unwrap_or(name);
        if !signatures.command_unchanged(name) {
            continue;
        }
        if crate::bib_resources::references_file(name) {
            let label = bib_section_files::resource_label(source, span.end)
                .ok()
                .flatten();
            if let Some(label) =
                label.filter(|label| label.span.start <= offset && offset <= label.span.end)
            {
                return Some(Query {
                    command: lexical.span.start,
                    item: label.span,
                    selection: false,
                });
            }
            continue;
        }
        let end = if matches!(name, "newrefsection" | "refsection") {
            span.end
        } else if name == "begin" && signatures.command_unchanged("refsection") {
            let Some(environment) = syntax::environment_name_argument(source, span.end)
                .filter(|argument| argument.value == "refsection")
            else {
                continue;
            };
            environment.outer_span.end
        } else {
            continue;
        };
        let at = syntax::skip_space(source, end);
        // Execution proof below checks when forwarding aliases become active;
        // a simple imported reader need not have changed the lexical catcodes.
        if source.as_bytes().get(at) != Some(&b'[')
            || crate::resource_context::ambiguous_header(&doc.syntax, at)
        {
            continue;
        }
        let content = syntax::verbatim_file_bracket(source, at)
            .map_or(at + 1..source.len(), |(argument, _)| argument.span);
        for segment in syntax::verbatim_list_segments(source, content) {
            let item = syntax::trimmed_span(source, segment);
            if item.start <= offset
                && offset <= item.end
                && !source[item.clone()]
                    .contains(['\\', '{', '}', '%', '#', '$', '&', '^', '~', '[', ']'])
            {
                return Some(Query {
                    command: lexical.span.start,
                    item,
                    selection: true,
                });
            }
        }
    }
    None
}

struct Target {
    query: Query,
    bindings: std::collections::BTreeMap<String, LabelBinding>,
}
fn target(ws: &Workspace, uri: &Url, offset: usize) -> Option<Target> {
    let doc = ws
        .documents
        .get(uri)
        .filter(|doc| doc.kind == DocumentKind::Latex)?;
    // Static option values contain no commands. Most editor requests can
    // therefore avoid constructing any project signatures at all.
    let before = doc
        .syntax
        .commands
        .partition_point(|command| command.span.start <= offset);
    let command = before
        .checked_sub(1)
        .and_then(|index| doc.syntax.commands.get(index))?;
    if !doc.text.source.get(command.span.end..offset)?.contains('[') {
        return None;
    }
    if !ws.index_truncated.is_empty()
        || !ws
            .documents
            .values()
            .any(|doc| doc.kind == DocumentKind::Latex && doc.text.source.contains("label"))
    {
        return None;
    }
    let (_, projects) = crate::bib_section_keys::section_projects(ws, uri)?;
    let mut result: Option<Target> = None;
    for project in projects {
        let query = query(doc, &project.signatures, offset)?;
        let bindings = crate::bib_section_keys::label_bindings(
            ws,
            &project.root,
            &project.docs,
            &project.signatures,
            uri,
            query.command,
        )?;
        if let Some(previous) = result.as_mut() {
            if previous.query.item != query.item || previous.query.selection != query.selection {
                return None;
            }
            previous
                .bindings
                .retain(|name, binding| bindings.get(name) == Some(binding));
        } else {
            result = Some(Target { query, bindings });
        }
    }
    result
}

pub(crate) fn definitions(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    let target = target(ws, uri, offset)?;
    let name = &ws.documents[uri].text.source[target.query.item];
    let binding = target.bindings.get(name)?;
    let doc = ws.documents.get(&binding.uri)?;
    Some(vec![Location::new(
        binding.uri.clone(),
        doc.text.range(binding.span.start, binding.span.end),
    )])
}

pub(crate) fn hover(ws: &Workspace, uri: &Url, offset: usize) -> Option<Hover> {
    let target = target(ws, uri, offset)?;
    let doc = &ws.documents[uri];
    let name = &doc.text.source[target.query.item.clone()];
    let binding = target.bindings.get(name)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::PlainText,
            value: format!(
                "{name}\n\nbiblatex resource: {}\n{}:{}",
                binding.filename,
                binding.uri.path(),
                ws.documents[&binding.uri]
                    .text
                    .position(binding.span.start)
                    .line
                    + 1
            ),
        }),
        range: Some(
            doc.text
                .range(target.query.item.start, target.query.item.end),
        ),
    })
}

pub(crate) fn completion(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<CompletionItem>> {
    let target = target(ws, uri, offset)?;
    if !target.query.selection {
        return None;
    }
    let doc = &ws.documents[uri];
    let prefix = &doc.text.source[target.query.item.start..offset];
    let range = doc
        .text
        .range(target.query.item.start, target.query.item.end);
    let mut items: Vec<_> = target
        .bindings
        .iter()
        .filter(|(name, _)| {
            name.starts_with(prefix) && syntax::is_static_key(name) && !name.contains(['[', ']'])
        })
        .map(|(name, binding)| CompletionItem {
            label: name.clone(),
            kind: Some(CompletionItemKind::REFERENCE),
            detail: Some(format!("biblatex resource: {}", binding.filename)),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: name.clone(),
            })),
            ..Default::default()
        })
        .collect();
    // Retain literal file suggestions with their existing insertion checks.
    let command = Command {
        name: bib_section_files::INFERRED.into(),
        span: target.query.command..target.query.command,
        argument: Some(Argument {
            value: doc.text.source[target.query.item.clone()].into(),
            span: target.query.item.clone(),
            outer_span: target.query.item.clone(),
        }),
    };
    items.extend(crate::path_completion::complete_source(
        ws,
        uri,
        &command,
        target.query.item.start..offset,
    ));
    Some(items)
}
