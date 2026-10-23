//! Static semantic highlighting, with non-overlapping, single-line UTF-16 tokens.
use crate::{syntax, workspace::Document};
use std::{collections::BTreeSet, ops::Range as Span};
use tower_lsp::{jsonrpc, lsp_types::*};

const MACRO: u32 = 0;
const VARIABLE: u32 = 1;
const TYPE: u32 = 2;
const KEY: u32 = 3;
const STRING: u32 = 4;
const COMMENT: u32 = 5;
const PROPERTY: u32 = 6;
const NUMBER: u32 = 7;
const OPERATOR: u32 = 8;
const DECLARATION: u32 = 1;

pub fn options() -> SemanticTokensOptions {
    SemanticTokensOptions {
        legend: SemanticTokensLegend {
            token_types: vec![
                SemanticTokenType::MACRO,
                SemanticTokenType::VARIABLE,
                SemanticTokenType::TYPE,
                SemanticTokenType::ENUM_MEMBER,
                SemanticTokenType::STRING,
                SemanticTokenType::COMMENT,
                SemanticTokenType::PROPERTY,
                SemanticTokenType::NUMBER,
                SemanticTokenType::OPERATOR,
            ],
            token_modifiers: vec![SemanticTokenModifier::DECLARATION],
        },
        range: Some(true),
        full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
        ..Default::default()
    }
}

struct Token {
    span: Span<usize>,
    kind: u32,
    modifiers: u32,
    priority: u8,
}

fn push(tokens: &mut Vec<Token>, span: Span<usize>, kind: u32, declaration: bool) {
    if !span.is_empty() {
        tokens.push(Token {
            span,
            kind,
            modifiers: u32::from(declaration) * DECLARATION,
            priority: if declaration { 30 } else { 20 },
        });
    }
}

fn literal(tokens: &mut Vec<Token>, source: &str, span: Span<usize>, kind: u32) {
    if syntax::is_static_key(&source[span.clone()]) {
        push(tokens, span, kind, false);
    }
}

fn reference(tokens: &mut Vec<Token>, source: &str, span: Span<usize>) {
    if syntax::is_static_reference_key(&source[span.clone()]) {
        push(tokens, span, KEY, false);
    }
}

fn citation(tokens: &mut Vec<Token>, source: &str, span: Span<usize>) {
    if syntax::is_static_citation_key(&source[span.clone()]) {
        push(tokens, span, KEY, false);
    }
}

fn candidates(doc: &Document) -> Vec<Token> {
    let source = &doc.text.source;
    let mut tokens = Vec::new();
    let variables: BTreeSet<_> = doc
        .syntax
        .definitions
        .iter()
        .filter(|entry| entry.is_variable)
        .map(|entry| entry.name.as_str())
        .collect();
    for (index, command) in doc.syntax.commands.iter().enumerate() {
        if let Some(argument) = syntax::environment_copy_source(source, command) {
            literal(&mut tokens, source, argument.span, TYPE);
        }
        tokens.push(Token {
            span: command.span.clone(),
            kind: if variables.contains(command.name.as_str()) {
                VARIABLE
            } else {
                MACRO
            },
            modifiers: 0,
            priority: 10,
        });
        if doc.syntax.control_operands.contains(&command.span.start) {
            continue;
        }
        if let Some(argument) = &command.argument {
            if matches!(command.name.as_str(), "begin" | "end") {
                literal(&mut tokens, source, argument.span.clone(), TYPE);
            } else if syntax::is_reference(&command.name) {
                if syntax::is_single_reference(&command.name) {
                    reference(&mut tokens, source, argument.span.clone());
                } else {
                    for span in syntax::reference_list_spans(source, argument.span.clone()) {
                        reference(&mut tokens, source, span);
                    }
                }
            } else if syntax::is_citation(&command.name)
                && !crate::citations::is_structured(&command.name)
            {
                if syntax::is_single_citation(&command.name) {
                    citation(&mut tokens, source, argument.span.clone());
                } else {
                    for span in syntax::citation_list_spans(source, argument.span.clone()) {
                        citation(&mut tokens, source, span);
                    }
                }
            } else if syntax::boolean_reference(&command.name)
                || syntax::toggle_reference(&command.name)
            {
                literal(&mut tokens, source, argument.span.clone(), VARIABLE);
            }
        }
        if let Some(argument) = syntax::separate_label_argument(source, command)
            .or_else(|| syntax::range_reference_end(source, command))
        {
            reference(&mut tokens, source, argument.span);
        }
        for (declaration, span) in crate::anchors::name_spans(source, command) {
            push(&mut tokens, span, KEY, declaration);
        }
        for span in crate::named_commands::spans(doc, command) {
            push(&mut tokens, span, MACRO, false);
        }
        if let Some(span) = syntax::literal_csname(source, &doc.syntax.commands, index) {
            push(&mut tokens, span, MACRO, false);
        }
        for (span, _) in crate::counters::invocations(source, command) {
            push(&mut tokens, span, VARIABLE, false);
        }
        if command.name == "ifltxcounter" && source.as_bytes().get(command.span.end) != Some(&b'*')
        {
            if let Some(argument) = command
                .argument
                .as_ref()
                .filter(|argument| syntax::is_static_key(&argument.value))
            {
                push(&mut tokens, argument.span.clone(), VARIABLE, false);
            }
        }
    }
    for entry in &doc.syntax.definitions {
        push(
            &mut tokens,
            entry.span.clone(),
            if entry.is_variable { VARIABLE } else { MACRO },
            true,
        );
    }
    for (entries, kind) in [
        (&doc.syntax.environments, TYPE),
        (&doc.syntax.counters, VARIABLE),
        (&doc.syntax.conditionals, VARIABLE),
        (&doc.syntax.toggles, VARIABLE),
        (&doc.syntax.labels, KEY),
        (&doc.syntax.citations, KEY),
        (&doc.syntax.bibliography_aliases, KEY),
        (&doc.syntax.colors, VARIABLE),
    ] {
        for entry in entries {
            push(&mut tokens, entry.span.clone(), kind, true);
        }
    }
    for entry in &doc.syntax.citation_references {
        push(&mut tokens, entry.span.clone(), KEY, false);
    }
    for entry in &doc.syntax.bibliography_strings.definitions {
        push(&mut tokens, entry.span.clone(), VARIABLE, true);
    }
    for entry in &doc.syntax.bibliography_strings.references {
        push(&mut tokens, entry.span.clone(), VARIABLE, false);
    }
    for entry in &doc.syntax.bibliography_completion.types {
        if crate::bib_strings::identifier(&source[entry.span.clone()]) {
            push(&mut tokens, entry.span.clone(), TYPE, false);
        }
    }
    for entry in &doc.syntax.bibliography_completion.entries {
        for field in &entry.fields {
            push(&mut tokens, field.span.clone(), PROPERTY, false);
        }
    }
    let values = &doc.syntax.bibliography_strings;
    for span in &values.literal_spans {
        // Literal backgrounds yield to relationship keys and validated links.
        tokens.push(Token {
            span: span.clone(),
            kind: STRING,
            modifiers: 0,
            priority: 10,
        });
    }
    for (spans, kind) in [
        (&values.number_spans, NUMBER),
        (&values.operator_spans, OPERATOR),
    ] {
        for span in spans {
            push(&mut tokens, span.clone(), kind, false);
        }
    }
    for link in &doc.syntax.bibliography_links {
        push(&mut tokens, link.span.clone(), STRING, false);
    }
    for entry in &doc.syntax.color_references {
        push(&mut tokens, entry.span.clone(), VARIABLE, false);
    }
    let comments: BTreeSet<_> = doc
        .syntax
        .comments
        .iter()
        .map(|span| (span.start, span.end))
        .collect();
    for span in &doc.syntax.ignored {
        let comment = comments.contains(&(span.start, span.end));
        // The primitive verb command does not enter the ordinary command index.
        // Recover only their immediately preceding, explicit control word.
        let end = span.start - usize::from(source[..span.start].ends_with('*'));
        let start = source[..end]
            .rfind(|ch: char| !ch.is_ascii_alphabetic())
            .unwrap_or(end);
        if !comment
            && source.as_bytes().get(start) == Some(&b'\\')
            && &source[start + 1..end] == "verb"
        {
            push(&mut tokens, start..end, MACRO, false);
        }
        tokens.push(Token {
            span: span.clone(),
            kind: if comment { COMMENT } else { STRING },
            modifiers: 0,
            priority: 100,
        });
    }
    tokens
}

/// An interval sweep lets comments and literal regions override any embedded
/// command candidates. Declarations override command calls at the same span.
fn resolve(tokens: &[Token], requested: Span<usize>) -> Vec<(Span<usize>, usize)> {
    let mut events = Vec::with_capacity(tokens.len() * 2);
    for (id, token) in tokens.iter().enumerate() {
        let start = token.span.start.max(requested.start);
        let end = token.span.end.min(requested.end);
        if start < end {
            events.push((start, true, id));
            events.push((end, false, id));
        }
    }
    events.sort_unstable();
    let mut active = BTreeSet::new();
    let mut result: Vec<(Span<usize>, usize)> = Vec::new();
    let mut previous = requested.start;
    for (at, entering, id) in events {
        if previous < at {
            if let Some(&(_, current)) = active.last() {
                if let Some((last, _)) = result
                    .last_mut()
                    .filter(|(span, last_id)| span.end == previous && *last_id == current)
                {
                    last.end = at;
                } else {
                    result.push((previous..at, current));
                }
            }
        }
        if entering {
            active.insert((tokens[id].priority, id));
        } else {
            active.remove(&(tokens[id].priority, id));
        }
        previous = at;
    }
    result
}

fn advance(source: &str, from: usize, to: usize, position: &mut Position) {
    for ch in source[from..to].chars() {
        match ch {
            '\n' => {
                position.line += 1;
                position.character = 0;
            }
            _ => position.character += ch.len_utf16() as u32,
        }
    }
}

fn range_offset(doc: &Document, position: Position) -> Option<usize> {
    if let Some(offset) = doc.text.offset(position) {
        return Some(offset);
    }
    // Editors use (line count, 0) for a viewport ending at EOF, including
    // buffers without a final newline. Keep this normalization local to queries.
    if position.line as usize == doc.text.text_line_count() && position.character == 0 {
        return Some(doc.text.source.len());
    }
    let start = doc.text.offset(Position::new(position.line, 0))?;
    let tail = &doc.text.source[start..];
    let line = tail.split('\n').next()?;
    let line = line.strip_suffix('\r').unwrap_or(line);
    // LSP columns past the end of a line clamp to its length. A position
    // inside a surrogate pair remains invalid.
    (position.character as usize > line.encode_utf16().count()).then_some(start + line.len())
}

pub fn tokens(doc: &Document, range: Option<Range>) -> jsonrpc::Result<SemanticTokens> {
    let source = &doc.text.source;
    let requested = match range {
        Some(range) => {
            let start = range_offset(doc, range.start);
            let end = range_offset(doc, range.end);
            match (start, end) {
                (Some(start), Some(end)) if start <= end => start..end,
                _ => {
                    return Err(jsonrpc::Error::invalid_params(
                        "Invalid semantic token range",
                    ))
                }
            }
        }
        None => 0..source.len(),
    };
    if requested.is_empty() {
        return Ok(SemanticTokens {
            result_id: None,
            data: vec![],
        });
    }
    let candidates = candidates(doc);
    let mut data = Vec::new();
    let mut cursor = 0;
    let mut position = Position::new(0, 0);
    let mut previous = Position::new(0, 0);
    for (span, id) in resolve(&candidates, requested) {
        advance(source, cursor, span.start, &mut position);
        cursor = span.start;
        // Never require clients to support multiline or overlapping tokens.
        for line in source[span.clone()].split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            let content = content.strip_suffix('\r').unwrap_or(content);
            let length = content.encode_utf16().count() as u32;
            if length > 0 {
                data.push(SemanticToken {
                    delta_line: position.line - previous.line,
                    delta_start: if position.line == previous.line {
                        position.character - previous.character
                    } else {
                        position.character
                    },
                    length,
                    token_type: candidates[id].kind,
                    token_modifiers_bitset: candidates[id].modifiers,
                });
                previous = position;
            }
            advance(source, cursor, cursor + line.len(), &mut position);
            cursor += line.len();
        }
    }
    Ok(SemanticTokens {
        result_id: None,
        data,
    })
}
