//! Syntax-aware folding, with UTF-16 columns negotiated at the LSP boundary.
use crate::{
    language::features,
    parser::syntax,
    project::workspace::{Document, DocumentKind},
};
use std::ops::Range;
use tower_lsp::lsp_types::*;

fn fold(doc: &Document, span: Range<usize>, kind: FoldingRangeKind) -> Option<FoldingRange> {
    let start = doc.text.position(span.start);
    let end_offset = span.end - usize::from(doc.text.source[..span.end].ends_with('\r'));
    let mut end = doc.text.position(end_offset);
    // A section ends at the next heading's line start. Keep that heading out
    // of the fold, including when the preceding line uses CRLF.
    if end.character == 0 && end.line > start.line {
        end.line -= 1;
        let line_start = doc.text.offset(Position::new(end.line, 0))?;
        let line = doc.text.source[line_start..].split('\n').next()?;
        end.character = line.trim_end_matches('\r').encode_utf16().count() as u32;
    }
    (end.line > start.line).then_some(FoldingRange {
        start_line: start.line,
        start_character: Some(start.character),
        end_line: end.line,
        end_character: Some(end.character),
        kind: Some(kind),
        collapsed_text: None,
    })
}

fn heading_folds(doc: &Document, symbols: &[DocumentSymbol], result: &mut Vec<FoldingRange>) {
    for symbol in symbols {
        if symbol.kind == SymbolKind::NAMESPACE {
            if let (Some(start), Some(end)) = (
                doc.text.offset(symbol.range.start),
                doc.text.offset(symbol.range.end),
            ) {
                result.extend(fold(doc, start..end, FoldingRangeKind::Region));
            }
        }
        if let Some(children) = &symbol.children {
            heading_folds(doc, children, result);
        }
    }
}

fn environment_folds(doc: &Document, result: &mut Vec<FoldingRange>) {
    let mut environments = Vec::new();
    for command in &doc.syntax.commands {
        if !matches!(command.name.as_str(), "begin" | "end")
            || doc.syntax.control_operands.contains(&command.span.start)
        {
            continue;
        }
        let Some(argument) = syntax::environment_name_argument(&doc.text.source, command.span.end)
        else {
            continue;
        };
        if command.name == "begin" {
            if syntax::is_verbatim_environment(&argument.value) {
                if let Some(raw) = doc
                    .syntax
                    .ignored
                    .iter()
                    .find(|span| span.start == argument.outer_span.end)
                {
                    result.extend(fold(
                        doc,
                        command.span.start..raw.end,
                        if argument.value == "comment" {
                            FoldingRangeKind::Comment
                        } else {
                            FoldingRangeKind::Region
                        },
                    ));
                }
            } else {
                environments.push((argument.value, command.span.start));
            }
        } else if command.name == "end" {
            if let Some(index) = environments
                .iter()
                .rposition(|(name, _)| *name == argument.value)
            {
                result.extend(fold(
                    doc,
                    environments[index].1..argument.outer_span.end,
                    FoldingRangeKind::Region,
                ));
                environments.truncate(index);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Math {
    Dollar,
    DisplayDollar,
    Parenthesis,
    Bracket,
}

fn group_and_math_folds(doc: &Document, result: &mut Vec<FoldingRange>) {
    let source = &doc.text.source;
    let bytes = source.as_bytes();
    let mut braces = Vec::new();
    let mut math: Vec<(Math, usize, usize)> = Vec::new();
    // Literal conversions do not participate in the ordinary command index,
    // but must also stay opaque to the punctuation scanner.
    let mut ignored: Vec<_> = doc
        .syntax
        .ignored
        .iter()
        .chain(&doc.syntax.literal_conversions)
        .cloned()
        .collect();
    ignored.sort_by_key(|span| span.start);
    let mut ignored = ignored.iter().peekable();
    let mut i = 0;
    while i < bytes.len() {
        while ignored.peek().is_some_and(|span| span.end <= i) {
            ignored.next();
        }
        if let Some(span) = ignored
            .peek()
            .filter(|span| span.start <= i && i < span.end)
        {
            i = span.end;
            continue;
        }
        if doc.syntax.punctuation_operands.contains(&i) {
            i += 1;
            continue;
        }
        let mut marker = None;
        let mut width = 1;
        match bytes[i] {
            b'\\' => {
                let token = syntax::token(source, i).unwrap_or("\\");
                width = token.len();
                if !doc.syntax.control_operands.contains(&i) {
                    marker = match token {
                        "\\[" => Some((Math::Bracket, true)),
                        "\\]" => Some((Math::Bracket, false)),
                        "\\(" => Some((Math::Parenthesis, true)),
                        "\\)" => Some((Math::Parenthesis, false)),
                        _ => None,
                    };
                }
            }
            b'%' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'{' => braces.push(i),
            b'}' => {
                if let Some(start) = braces.pop() {
                    result.extend(fold(doc, start..i + 1, FoldingRangeKind::Region));
                }
                while math
                    .last()
                    .is_some_and(|(_, _, depth)| *depth > braces.len())
                {
                    math.pop();
                }
            }
            b'$' => {
                let active = math
                    .last()
                    .filter(|(_, _, depth)| *depth == braces.len())
                    .map(|(kind, _, _)| *kind);
                // A deeper group may return to text mode (e.g. \text{$x$}).
                // Track that formula separately from the surrounding display.
                if !matches!(active, Some(Math::Bracket | Math::Parenthesis)) {
                    let kind = if active != Some(Math::Dollar) && bytes.get(i + 1) == Some(&b'$') {
                        width = 2;
                        Math::DisplayDollar
                    } else {
                        Math::Dollar
                    };
                    marker = Some((kind, active.is_none()));
                }
            }
            _ => {}
        }
        if let Some((kind, open)) = marker {
            if open {
                if math
                    .last()
                    .is_some_and(|(_, _, depth)| *depth == braces.len())
                {
                    math.pop();
                }
                math.push((kind, i, braces.len()));
            } else if let Some((active, start, depth)) = math.pop() {
                if kind == active && depth == braces.len() {
                    result.extend(fold(doc, start..i + width, FoldingRangeKind::Region));
                }
            }
        }
        i += width;
    }
}

fn comment_folds(doc: &Document, result: &mut Vec<FoldingRange>) {
    let source = &doc.text.source;
    let mut run: Option<Range<usize>> = None;
    let mut regions = Vec::new();
    for span in &doc.syntax.comments {
        if !source[span.clone()].starts_with('%') {
            continue;
        }
        let position = doc.text.position(span.start);
        let Some(start) = doc.text.offset(Position::new(position.line, 0)) else {
            continue;
        };
        if !source[start..span.start].trim().is_empty() {
            continue;
        }
        let marker = source[span.start + 1..span.end].trim();
        let marker = marker.strip_prefix('#').unwrap_or(marker);
        match marker.split_whitespace().next() {
            Some("region") => regions.push(span.start),
            Some("endregion") => {
                if let Some(start) = regions.pop() {
                    result.extend(fold(doc, start..span.end, FoldingRangeKind::Region));
                }
            }
            _ => {}
        }
        if let Some(previous) = &mut run {
            if position.line == doc.text.position(previous.end).line + 1 {
                previous.end = span.end;
                continue;
            }
            result.extend(fold(doc, previous.clone(), FoldingRangeKind::Comment));
        }
        run = Some(span.clone());
    }
    if let Some(span) = run {
        result.extend(fold(doc, span, FoldingRangeKind::Comment));
    }
}

/// Preserve the line-only API used by other language features and callers.
pub fn folding_ranges(doc: &Document, limit: Option<u32>) -> Vec<FoldingRange> {
    folding_ranges_for_client(
        doc,
        &FoldingRangeClientCapabilities {
            range_limit: limit,
            line_folding_only: Some(true),
            ..Default::default()
        },
    )
}

pub fn folding_ranges_for_client(
    doc: &Document,
    capabilities: &FoldingRangeClientCapabilities,
) -> Vec<FoldingRange> {
    let mut result = Vec::new();
    match doc.kind {
        DocumentKind::Latex => {
            heading_folds(doc, &features::symbols(doc), &mut result);
            environment_folds(doc, &mut result);
            group_and_math_folds(doc, &mut result);
        }
        DocumentKind::Bibtex => {
            for (span, comment) in &doc.syntax.bibliography_entries {
                result.extend(fold(
                    doc,
                    span.clone(),
                    if *comment {
                        FoldingRangeKind::Comment
                    } else {
                        FoldingRangeKind::Region
                    },
                ));
            }
        }
        DocumentKind::BibtexStyle => return result,
    }
    comment_folds(doc, &mut result);
    let line_only = capabilities.line_folding_only.unwrap_or(false);
    let labels = capabilities
        .folding_range
        .as_ref()
        .and_then(|range| range.collapsed_text)
        .unwrap_or(false);
    for range in &mut result {
        if labels {
            if let Some(start) = doc.text.offset(Position::new(
                range.start_line,
                range.start_character.unwrap_or(0),
            )) {
                let line = doc.text.source[start..].lines().next().unwrap_or("").trim();
                range.collapsed_text =
                    Some(format!("{} …", line.chars().take(80).collect::<String>()));
            }
        }
        if line_only {
            range.start_character = None;
            range.end_character = None;
        }
    }
    result.sort_by_key(|range| {
        (
            range.start_line,
            range.start_character,
            std::cmp::Reverse(range.end_line),
            std::cmp::Reverse(range.end_character),
        )
    });
    result.dedup_by(|a, b| {
        a.start_line == b.start_line
            && a.start_character == b.start_character
            && a.end_line == b.end_line
            && a.end_character == b.end_character
    });
    result.truncate(capabilities.range_limit.unwrap_or(u32::MAX) as usize);
    result
}
