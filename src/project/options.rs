//! Static key/value metadata for common LaTeX option contexts.
use crate::{parser::syntax, project::workspace::Workspace};
use std::{collections::BTreeSet, ops::Range as Span};
use tower_lsp::lsp_types::*;

type OptionSpec = (&'static str, &'static str);
const GRAPHICS: &[OptionSpec] = &[
    ("width", "\\linewidth|\\textwidth|0.5\\textwidth"),
    ("height", "\\textheight|0.5\\textheight"),
    ("scale", "0.5|1|2"),
    ("angle", "0|90|180|270"),
    ("keepaspectratio", "true|false"),
    ("clip", "true|false"),
    ("trim", "="),
    ("viewport", "="),
    ("page", "1|2"),
    ("draft", "true|false"),
];
const GEOMETRY: &[OptionSpec] = &[
    ("margin", "1in|2cm|3cm"),
    ("left", "2cm|3cm"),
    ("right", "2cm|3cm"),
    ("top", "2cm|3cm"),
    ("bottom", "2cm|3cm"),
    ("paper", "a4paper|letterpaper|a5paper"),
    ("a4paper", ""),
    ("letterpaper", ""),
    ("landscape", "true|false"),
    ("portrait", ""),
    ("twoside", "true|false"),
    ("includeheadfoot", "true|false"),
    ("showframe", "true|false"),
];
const HYPERREF: &[OptionSpec] = &[
    ("colorlinks", "true|false"),
    ("linkcolor", "blue|black|red"),
    ("citecolor", "blue|black|green"),
    ("urlcolor", "blue|black|cyan"),
    ("hidelinks", ""),
    ("bookmarks", "true|false"),
    ("bookmarksopen", "true|false"),
    ("pdftitle", "="),
    ("pdfauthor", "="),
    ("pdfsubject", "="),
];
const CLASS: &[OptionSpec] = &[
    ("10pt", ""),
    ("11pt", ""),
    ("12pt", ""),
    ("a4paper", ""),
    ("letterpaper", ""),
    ("oneside", ""),
    ("twoside", ""),
    ("onecolumn", ""),
    ("twocolumn", ""),
    ("draft", ""),
    ("final", ""),
    ("landscape", ""),
    ("openright", ""),
    ("openany", ""),
];
const ENUMITEM: &[OptionSpec] = &[
    ("label", "="),
    ("ref", "="),
    ("leftmargin", "*|!|0pt|2em"),
    ("itemsep", "0pt|1ex"),
    ("topsep", "0pt|1ex"),
    ("parsep", "0pt|1ex"),
    ("nosep", ""),
    ("noitemsep", ""),
    ("resume", ""),
    ("start", "1|2"),
    ("align", "left|right|parleft"),
];
fn profile(name: &str) -> Option<&'static [OptionSpec]> {
    Some(match name {
        "graphicx" | "Gin" => GRAPHICS,
        "geometry" => GEOMETRY,
        "hyperref" => HYPERREF,
        "class" => CLASS,
        "enumitem" => ENUMITEM,
        _ => return None,
    })
}

fn segments(source: &str, span: Span<usize>) -> Vec<Span<usize>> {
    let mut result = Vec::new();
    let mut start = span.start;
    let mut at = start;
    while at < span.end {
        match source.as_bytes()[at] {
            b'{' => {
                if let Some((_, end)) = syntax::group(source, at, b'{', b'}') {
                    at = end;
                    continue;
                }
            }
            b'%' => {
                at = source[at..span.end]
                    .find('\n')
                    .map_or(span.end, |len| at + len + 1);
                continue;
            }
            b'\\' => {
                if let Some(value) = syntax::token(source, at) {
                    at += value.len();
                    continue;
                }
            }
            b',' => {
                result.push(start..at);
                start = at + 1;
            }
            _ => {}
        }
        at += source[at..].chars().next().unwrap().len_utf8();
    }
    result.push(start..span.end);
    result
}

pub(crate) fn completion(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<CompletionItem>> {
    let doc = ws.documents.get(uri)?;
    let source = &doc.text.source;
    let scope = ws.symbol_scope(uri);
    for command in doc.syntax.commands.iter().rev().filter(|command| {
        command.span.end <= offset && !doc.syntax.control_operands.contains(&command.span.start)
    }) {
        if ws
            .documents
            .iter()
            .filter(|(uri, _)| scope.contains(*uri))
            .any(|(_, doc)| {
                doc.syntax
                    .definitions
                    .iter()
                    .any(|entry| entry.name == command.name)
            })
        {
            continue;
        }
        let mut at = syntax::skip_space(source, command.span.end);
        if source.as_bytes().get(at) == Some(&b'*') {
            at = syntax::skip_space(source, at + 1);
        }
        let (span, profiles) = match command.name.as_str() {
            "includegraphics" | "usepackage" | "RequirePackage" | "documentclass" | "LoadClass" => {
                if source.as_bytes().get(at) != Some(&b'[') {
                    continue;
                }
                let (span, end) = syntax::bracket_argument(source, at)
                    .map(|(arg, end)| (arg.span, end))
                    .unwrap_or((at + 1..source.len(), source.len()));
                let profiles = match command.name.as_str() {
                    "includegraphics" => vec!["graphicx".to_string()],
                    "documentclass" | "LoadClass" => vec!["class".to_string()],
                    _ => syntax::group(source, syntax::skip_space(source, end), b'{', b'}')
                        .map(|(arg, _)| {
                            arg.value
                                .split(',')
                                .map(str::trim)
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                (span, profiles)
            }
            "geometry" | "newgeometry" | "hypersetup" => {
                let span = syntax::group(source, at, b'{', b'}')
                    .map(|(arg, _)| arg.span)
                    .or_else(|| {
                        (source.as_bytes().get(at) == Some(&b'{')).then_some(at + 1..source.len())
                    })?;
                (
                    span,
                    vec![if command.name == "hypersetup" {
                        "hyperref"
                    } else {
                        "geometry"
                    }
                    .into()],
                )
            }
            "setkeys" => {
                let Some((family, end)) = syntax::group(source, at, b'{', b'}') else {
                    continue;
                };
                let Some((arg, _)) =
                    syntax::group(source, syntax::skip_space(source, end), b'{', b'}')
                else {
                    continue;
                };
                (arg.span, vec![family.value])
            }
            "begin" => {
                let Some((env, end)) = syntax::group(source, at, b'{', b'}') else {
                    continue;
                };
                if !matches!(env.value.as_str(), "itemize" | "enumerate" | "description") {
                    continue;
                }
                let Some((arg, _)) =
                    syntax::bracket_argument(source, syntax::skip_space(source, end))
                else {
                    continue;
                };
                (arg.span, vec!["enumitem".into()])
            }
            _ => continue,
        };
        if offset < span.start || offset > span.end {
            continue;
        }
        let profiles: Vec<_> = profiles.iter().filter_map(|name| profile(name)).collect();
        if profiles.is_empty() {
            return None;
        }
        let segments = segments(source, span);
        let active = segments
            .iter()
            .find(|span| span.start <= offset && offset <= span.end)?;
        if source[active.clone()].contains('%') {
            return None;
        }
        let mut trimmed = syntax::trimmed_span(source, active.clone());
        trimmed.start = trimmed.start.min(offset);
        trimmed.end = trimmed.end.max(offset);
        let equals = source[trimmed.clone()]
            .find('=')
            .map(|len| trimmed.start + len);
        let values = equals.filter(|equals| *equals < offset);
        let (span, prefix) = if let Some(equals) = values {
            let start = equals + 1 + source[equals + 1..offset].len()
                - source[equals + 1..offset].trim_start().len();
            (start..trimmed.end, &source[start..offset])
        } else {
            (
                trimmed.start..equals.unwrap_or(trimmed.end),
                &source[trimmed.start..offset],
            )
        };
        if prefix.contains(['{', '}', '\n', '\r']) {
            return None;
        }
        let (range, extra) = syntax::name_completion_ranges(&doc.text, span, offset)?;
        let used: BTreeSet<_> = segments
            .iter()
            .filter(|span| *span != active)
            .map(|span| source[span.clone()].split('=').next().unwrap().trim())
            .collect();
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        for &(key, options) in profiles.into_iter().flatten() {
            if values.is_some() {
                if source[trimmed.start..equals.unwrap()].trim() != key {
                    continue;
                }
                for value in options
                    .split('|')
                    .filter(|value| !value.is_empty() && *value != "=" && value.starts_with(prefix))
                {
                    if seen.insert(value) {
                        result.push((
                            value.to_string(),
                            value.to_string(),
                            CompletionItemKind::VALUE,
                        ));
                    }
                }
            } else if key.starts_with(prefix.trim()) && !used.contains(key) && seen.insert(key) {
                let append = equals.is_none() && !options.is_empty();
                result.push((
                    key.to_string(),
                    format!("{key}{}", if append { "=" } else { "" }),
                    CompletionItemKind::PROPERTY,
                ));
            }
        }
        return Some(
            result
                .into_iter()
                .map(|(label, new_text, kind)| CompletionItem {
                    label,
                    kind: Some(kind),
                    detail: Some("LaTeX option".into()),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text })),
                    additional_text_edits: (!extra.is_empty()).then(|| extra.clone()),
                    ..Default::default()
                })
                .collect(),
        );
    }
    None
}
