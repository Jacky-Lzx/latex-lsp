//! Certified biblatex text initialization for passive source entry queries.
use crate::{
    named_references::Context,
    signature::CommandSignatures,
    syntax::{self, Command},
    workspace::Document,
};
use std::{collections::BTreeSet, ops::Range};
use tower_lsp::lsp_types::Url;

pub(crate) fn native_initializer(name: &str) -> bool {
    matches!(
        name,
        "mkbibparens" | "mkbibbrackets" | "parentext" | "brackettext"
    )
}

pub(crate) fn initialized_call_argument(
    source: &str,
    command: &Command,
    call: &Range<usize>,
    work: &mut usize,
) -> Option<Option<Range<usize>>> {
    *work += 64;
    if *work > 32 * 1024 * 1024 {
        return None;
    }
    if *call == command.span {
        let start = syntax::skip_space(source, call.end);
        Some(
            command
                .argument
                .as_ref()
                .filter(|argument| {
                    argument.outer_span.start == start
                        && source.as_bytes().get(start) == Some(&b'{')
                })
                .map(|argument| argument.span.clone()),
        )
    } else {
        initialized_text_argument(source, call.end, work)
    }
}

// Extra declaration scope analysis is needed only for entrydata actually
// enclosed by a forward. An unused forward or one containing ordinary cites
// must not restrict previously supported section and context syntax.
pub(crate) fn uses_initializer_forwards(
    docs: &[(Url, &Document)],
    signatures: &CommandSignatures<'_>,
    work: &mut usize,
) -> Option<bool> {
    if !docs.iter().any(|(_, doc)| {
        doc.syntax.definitions.iter().any(|entry| {
            signatures
                .forwarded_builtin(&entry.name)
                .is_some_and(native_initializer)
        })
    }) {
        return Some(false);
    }
    for (_, doc) in docs {
        let source = &doc.text.source;
        *work += source.len() + doc.syntax.commands.len() * 64;
        if *work > 32 * 1024 * 1024 {
            return None;
        }
        let context = Context::new(doc, signatures);
        let positions = signatures.source_command_positions(source, &doc.syntax);
        let initialized = initialized_entrydata_spans(doc, signatures, &positions, &context, work)?;
        let empty = BTreeSet::new();
        let mut queries = Vec::new();
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            if !matches!(command.name.as_str(), "entrydata" | "csuse" | "csname")
                || !positions.contains(&command.span.start)
                || context.literal(command.span.start)
                || context.operand(doc, command.span.start)
            {
                continue;
            }
            let (name, _) = signatures
                .named_invocation(source, &doc.syntax, index, &empty, &context.declarations)
                .unwrap_or((&command.name, command.span.clone()));
            let already_initialized = initialized
                .partition_point(|span| span.start <= command.span.start)
                .checked_sub(1)
                .is_some_and(|index| initialized[index].contains(&command.span.start));
            if name == "entrydata" && signatures.command_unchanged(name) && !already_initialized {
                queries.push(command.span.start);
            }
        }
        if queries.is_empty() {
            continue;
        }
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            let (name, call) = signatures
                .named_invocation(source, &doc.syntax, index, &empty, &context.declarations)
                .unwrap_or((&command.name, command.span.clone()));
            if !signatures
                .forwarded_builtin(name)
                .is_some_and(native_initializer)
                || !positions.contains(&command.span.start)
                || context.literal(command.span.start)
                || context.operand(doc, command.span.start)
            {
                continue;
            }
            let Some(argument) = initialized_call_argument(source, command, &call, work)? else {
                continue;
            };
            if queries
                .get(queries.partition_point(|at| *at < argument.start))
                .is_some_and(|at| *at < argument.end)
            {
                return Some(true);
            }
        }
    }
    Some(false)
}

// csuse's parsed argument is the command name, not the text. Bound the new
// group scan and copy before reading its contents; exhaustion revokes the trace.
fn initialized_text_argument(
    source: &str,
    after: usize,
    work: &mut usize,
) -> Option<Option<Range<usize>>> {
    let mut limit = source.len().min(after + (32 * 1024 * 1024 - *work));
    while !source.is_char_boundary(limit) {
        limit -= 1;
    }
    let start = syntax::skip_space(&source[..limit], after);
    *work += start - after;
    if source.as_bytes().get(start) != Some(&b'{') {
        return (start != limit || limit == source.len()).then_some(None);
    }
    if let Some((argument, end)) = syntax::group(&source[..limit], start, b'{', b'}') {
        *work += end - start;
        Some(Some(argument.span))
    } else {
        *work += limit - start;
        (limit == source.len()).then_some(None)
    }
}

// These native text wrappers initialize biblatex's internal commands before
// executing their grouped text. Build their union once per visit; queries do
// not rescan nested group contents. Ordinary document entrydata stays
// unproved, as do captured/redefined wrappers and parameters consumed as data.
pub(crate) fn initialized_entrydata_spans(
    doc: &Document,
    signatures: &CommandSignatures<'_>,
    positions: &BTreeSet<usize>,
    context: &Context,
    work: &mut usize,
) -> Option<Vec<Range<usize>>> {
    let mut spans = Vec::new();
    let source = &doc.text.source;
    for (index, command) in doc.syntax.commands.iter().enumerate() {
        if !matches!(
            command.name.as_str(),
            "mkbibparens" | "mkbibbrackets" | "parentext" | "brackettext" | "csuse" | "csname"
        ) {
            continue;
        }
        *work += 64;
        if *work > 32 * 1024 * 1024 {
            return None;
        }
        if !positions.contains(&command.span.start) {
            continue;
        }
        if matches!(command.name.as_str(), "csuse" | "csname") {
            let Some((name, call)) = signatures.named_invocation(
                source,
                &doc.syntax,
                index,
                &BTreeSet::new(),
                &context.declarations,
            ) else {
                continue;
            };
            if !native_initializer(name) || !signatures.command_unchanged(name) {
                continue;
            }
            if let Some(span) = initialized_text_argument(source, call.end, work)? {
                spans.push(span);
            }
            continue;
        }
        if !signatures.command_unchanged(&command.name) {
            continue;
        }
        let Some(argument) = &command.argument else {
            continue;
        };
        let start = syntax::skip_space(&doc.text.source, command.span.end);
        if argument.outer_span.start == start
            && doc.text.source.as_bytes().get(start) == Some(&b'{')
        {
            spans.push(argument.span.clone());
        }
    }
    spans.sort_unstable_by_key(|span| span.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for span in spans {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| span.start <= previous.end)
        {
            previous.end = previous.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    Some(merged)
}

#[cfg(test)]
mod initializer_tests {
    use super::*;

    #[test]
    fn named_initializer_budget_exhaustion_never_returns_partial_ranges() {
        let source = r"\mkbibparens{Text}\csuse{mkbibparens}{😀\entrydata{collection}{Text}}";
        let doc = Document::new(
            &Url::parse("file:///budget.tex").unwrap(),
            source.into(),
            None,
        );
        let signatures = CommandSignatures::new([(source, &doc.syntax)]);
        let positions = signatures.source_command_positions(source, &doc.syntax);
        let context = Context::new(&doc, &signatures);
        // After the two command costs, the scan limit lies inside the emoji.
        let mut work = 32 * 1024 * 1024 - 128 - 2;
        assert!(
            initialized_entrydata_spans(&doc, &signatures, &positions, &context, &mut work)
                .is_none()
        );
        assert!(work <= 32 * 1024 * 1024);
    }
}
