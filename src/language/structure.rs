//! Source-local presentation and list scopes for document symbols.
use crate::{
    parser::syntax::{self, Argument, Command},
    project::workspace::Document,
    symbols::commands::{definition_body_spans, CommandSignatures, EnvironmentBoundary},
};
use lsp_types::{DocumentSymbol, SymbolKind};
use std::ops::Range;

pub(crate) fn container_kind(name: &str) -> Option<SymbolKind> {
    Some(match name {
        "itemize" | "enumerate" | "description" => SymbolKind::ARRAY,
        "frame" | "block" | "alertblock" | "exampleblock" | "columns" | "column" | "figure"
        | "figure*" | "table" | "table*" | "theorem" | "lemma" | "proof" | "definition"
        | "example" | "corollary" => SymbolKind::OBJECT,
        _ => return None,
    })
}

pub(crate) fn container(symbol: &DocumentSymbol) -> bool {
    symbol.kind == SymbolKind::NAMESPACE
        || symbol.detail.as_deref().is_some_and(|detail| {
            container_kind(detail) == Some(symbol.kind)
                || detail == "item" && symbol.kind == SymbolKind::ENUM_MEMBER
        })
}

fn header(source: &str, end: usize) -> usize {
    let mut at = syntax::skip_space(source, end);
    loop {
        let parsed = match source.as_bytes().get(at) {
            Some(b'<') => syntax::group(source, at, b'<', b'>'),
            Some(b'[') => syntax::bracket_argument(source, at),
            _ => None,
        };
        let Some((_, end)) = parsed else { return at };
        at = syntax::skip_space(source, end);
    }
}

fn title(source: &str, end: usize) -> Option<Argument> {
    let at = syntax::skip_space(source, end);
    let end = if source.as_bytes().get(at) == Some(&b'*') {
        at + 1
    } else {
        end
    };
    syntax::group(source, header(source, end), b'{', b'}').map(|(arg, _)| arg)
}

fn summary(value: &str, fallback: &str) -> String {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return fallback.into();
    }
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(80).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

#[allow(deprecated)]
fn symbol(
    doc: &Document,
    name: String,
    detail: &str,
    kind: SymbolKind,
    span: Range<usize>,
    selection: Range<usize>,
) -> DocumentSymbol {
    DocumentSymbol {
        name,
        detail: Some(detail.into()),
        kind,
        tags: None,
        deprecated: None,
        range: doc.text.range(span.start, span.end),
        selection_range: doc.text.range(selection.start, selection.end),
        children: None,
    }
}

struct Item {
    start: usize,
    selection: Range<usize>,
    name: String,
}
struct Environment {
    name: String,
    start: usize,
    selection: Range<usize>,
    title: Option<String>,
    item: Option<Item>,
}
impl Environment {
    fn finish_item(&mut self, doc: &Document, end: usize, result: &mut Vec<DocumentSymbol>) {
        if let Some(item) = self.item.take() {
            result.push(symbol(
                doc,
                item.name,
                "item",
                SymbolKind::ENUM_MEMBER,
                item.start..end.max(item.selection.end),
                item.selection,
            ));
        }
    }
    fn finish(
        mut self,
        doc: &Document,
        body_end: usize,
        end: usize,
        result: &mut Vec<DocumentSymbol>,
    ) {
        self.finish_item(doc, body_end, result);
        if let Some(kind) = container_kind(&self.name) {
            result.push(symbol(
                doc,
                self.title.unwrap_or_else(|| self.name.clone()),
                &self.name,
                kind,
                self.start..end.max(self.selection.end),
                self.selection,
            ));
        }
    }
}

enum Event<'a> {
    Boundary(EnvironmentBoundary),
    Item(&'a Command),
    Title(&'a Command),
}
impl Event<'_> {
    fn start(&self) -> usize {
        match self {
            Self::Boundary(call) => call.span.start,
            Self::Item(command) | Self::Title(command) => command.span.start,
        }
    }
}

pub(crate) fn symbols(doc: &Document) -> Vec<DocumentSymbol> {
    let source = &doc.text.source;
    if !doc
        .syntax
        .commands
        .iter()
        .any(|command| command.name == "begin")
    {
        return vec![];
    }
    let signatures = CommandSignatures::new([(source.as_str(), &doc.syntax)]);
    let bodies = definition_body_spans(source, &doc.syntax);
    let stored = signatures.source_argument_tokens(source, &doc.syntax);
    let mut events: Vec<_> = signatures
        .environment_boundaries(source, &doc.syntax)
        .into_iter()
        .filter(|call| !call.operand && !call.deferred)
        .map(Event::Boundary)
        .collect();
    for command in &doc.syntax.commands {
        if doc.syntax.control_operands.contains(&command.span.start)
            || stored.contains(&command.span.start)
            || bodies.iter().any(|span| span.contains(&command.span.start))
        {
            continue;
        }
        match command.name.as_str() {
            "item" if signatures.command_unchanged("item") => events.push(Event::Item(command)),
            "frametitle" if signatures.command_unchanged("frametitle") => {
                events.push(Event::Title(command))
            }
            _ => (),
        }
    }
    events.sort_by_key(Event::start);
    let mut stack: Vec<Environment> = Vec::new();
    let mut result = Vec::new();
    for (index, event) in events.iter().enumerate() {
        match event {
            Event::Boundary(call) => {
                let Some(argument) = &call.argument else {
                    continue;
                };
                if call.begin {
                    let heading = matches!(
                        argument.value.as_str(),
                        "frame" | "block" | "alertblock" | "exampleblock"
                    )
                    .then(|| title(source, call.span.end))
                    .flatten();
                    let (selection, heading) = heading
                        .map_or((argument.span.clone(), None), |arg| {
                            (arg.span, Some(summary(&arg.value, &argument.value)))
                        });
                    stack.push(Environment {
                        name: argument.value.clone(),
                        start: call.span.start,
                        selection,
                        title: heading,
                        item: None,
                    });
                } else if let Some(at) = stack.iter().rposition(|env| env.name == argument.value) {
                    while stack.len() > at + 1 {
                        stack.pop().unwrap().finish(
                            doc,
                            call.span.start,
                            call.span.start,
                            &mut result,
                        );
                    }
                    stack
                        .pop()
                        .unwrap()
                        .finish(doc, call.span.start, call.span.end, &mut result);
                }
            }
            Event::Title(command) => {
                if let Some(env) = stack.iter_mut().rev().find(|env| env.name == "frame") {
                    if env.title.is_none() {
                        if let Some(arg) = title(source, command.span.end) {
                            env.title = Some(summary(&arg.value, "frame"));
                            env.selection = arg.span;
                        }
                    }
                }
            }
            Event::Item(command) => {
                if let Some(env) = stack.iter_mut().rev().find(|env| {
                    matches!(env.name.as_str(), "itemize" | "enumerate" | "description")
                }) {
                    env.finish_item(doc, command.span.start, &mut result);
                    let next = events.get(index + 1).map_or(source.len(), Event::start);
                    let at = header(source, command.span.end).min(next);
                    // Bound previews even for a very long, single-line document.
                    let limit = source[at..next]
                        .char_indices()
                        .nth(400)
                        .map_or(next, |(offset, _)| at + offset);
                    let comment = doc
                        .syntax
                        .comments
                        .iter()
                        .find(|span| span.start >= at && span.start < limit)
                        .map_or(limit, |span| span.start);
                    env.item = Some(Item {
                        start: command.span.start,
                        selection: command.span.clone(),
                        name: summary(source[at..comment].lines().next().unwrap_or(""), "\\item"),
                    });
                }
            }
        }
    }
    while let Some(env) = stack.pop() {
        env.finish(doc, source.len(), source.len(), &mut result);
    }
    result
}
