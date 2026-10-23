//! Independent namespaces for glossary entries and index terms/streams.
use crate::{
    syntax,
    workspace::{Document, DocumentKind, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range as Span,
};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::*,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Glossary,
    IndexTerm,
    IndexStream,
}
#[derive(Clone, Debug)]
pub(crate) struct Occurrence {
    pub name: String,
    pub span: Span<usize>,
    pub kind: Kind,
    pub declaration: bool,
    pub detail: String,
}
impl Kind {
    fn description(self) -> &'static str {
        match self {
            Self::Glossary => "glossary entry",
            Self::IndexTerm => "index term",
            Self::IndexStream => "index stream",
        }
    }
    pub(crate) fn valid(self, name: &str) -> bool {
        if self == Self::IndexTerm {
            !name.trim().is_empty()
                && name.trim() == name
                && name.len() <= 512
                && !name.contains([
                    '\\', '{', '}', '%', '@', '|', '#', '$', '&', '^', '~', '"', '\n', '\r',
                ])
        } else {
            syntax::is_static_key(name)
        }
    }
}
fn glossary(name: &str) -> Option<bool> {
    Some(match name {
        "newglossaryentry"
        | "longnewglossaryentry"
        | "newacronym"
        | "newabbreviation"
        | "newacro"
        | "acrodef"
        | "DeclareAcronym" => true,
        "gls" | "Gls" | "GLS" | "glspl" | "Glspl" | "GLSpl" | "glsfirst" | "glsfirstplural"
        | "glstext" | "glsentryname" | "glsentrytext" | "glsentryplural" | "glsentrydesc"
        | "glsentryshort" | "glsentrylong" | "glslink" | "glsdisp" | "glsadd" | "glsreset"
        | "glsunset" | "acrshort" | "Acrshort" | "acrshortpl" | "acrlong" | "Acrlong"
        | "acrlongpl" | "acrfull" | "Acrfull" | "ac" | "Ac" | "acp" | "acf" | "acfp" | "acs"
        | "acsp" | "acl" | "aclp" | "iac" | "Iac" => false,
        _ => return None,
    })
}
fn group(source: &str, at: usize, open: u8) -> Option<syntax::Argument> {
    if source.as_bytes().get(at) != Some(&open) {
        return None;
    }
    let parsed = if open == b'[' {
        syntax::bracket_argument(source, at)
    } else {
        syntax::group(source, at, b'{', b'}')
    };
    Some(
        parsed
            .map(|(arg, _)| arg)
            .unwrap_or_else(|| syntax::Argument {
                value: source[at + 1..].into(),
                span: at + 1..source.len(),
                outer_span: at..source.len(),
            }),
    )
}
fn first(source: &str, cmd: &syntax::Command) -> Option<syntax::Argument> {
    let mut at = syntax::skip_space(source, cmd.span.end);
    if source.as_bytes().get(at) == Some(&b'*') {
        at = syntax::skip_space(source, at + 1);
    }
    while source.as_bytes().get(at) == Some(&b'[') {
        let (_, end) = syntax::bracket_argument(source, at)?;
        at = syntax::skip_space(source, end);
    }
    group(source, at, b'{')
}
fn occurrences(doc: &Document) -> Vec<Occurrence> {
    if doc.kind != DocumentKind::Latex {
        return vec![];
    }
    let source = &doc.text.source;
    let positions = crate::signature::CommandSignatures::source_context(std::iter::once((
        source.as_str(),
        &doc.syntax,
    )))
    .source_command_positions(source, &doc.syntax);
    let mut result = Vec::new();
    for cmd in doc
        .syntax
        .commands
        .iter()
        .filter(|cmd| positions.contains(&cmd.span.start))
    {
        if let Some(declaration) = glossary(&cmd.name) {
            let Some(arg) = first(source, cmd) else {
                continue;
            };
            let mut detail = if declaration {
                let at = syntax::skip_space(source, arg.outer_span.end);
                group(source, at, b'{')
                    .map(|arg| arg.value)
                    .unwrap_or_default()
            } else {
                String::new()
            };
            if declaration && matches!(cmd.name.as_str(), "newacronym" | "newabbreviation") {
                if let Some(short) =
                    group(source, syntax::skip_space(source, arg.outer_span.end), b'{')
                {
                    if let Some(long) = group(
                        source,
                        syntax::skip_space(source, short.outer_span.end),
                        b'{',
                    ) {
                        detail = format!("{} — {}", short.value, long.value);
                    }
                }
            }
            result.push(Occurrence {
                name: arg.value,
                span: arg.span,
                kind: Kind::Glossary,
                declaration,
                detail: detail.chars().take(256).collect(),
            });
        } else if cmd.name == "newindex" {
            if let Some(arg) = first(source, cmd) {
                result.push(Occurrence {
                    name: arg.value,
                    span: arg.span,
                    kind: Kind::IndexStream,
                    declaration: true,
                    detail: "Declared index stream".into(),
                });
            }
        } else if matches!(cmd.name.as_str(), "index" | "sindex" | "printindex") {
            let at = syntax::skip_space(source, cmd.span.end);
            if let Some(arg) = group(source, at, b'[') {
                result.push(Occurrence {
                    name: arg.value,
                    span: arg.span,
                    kind: Kind::IndexStream,
                    declaration: false,
                    detail: String::new(),
                });
            }
            if cmd.name != "printindex" {
                if let Some(arg) = first(source, cmd) {
                    let end = arg
                        .value
                        .find(['@', '|'])
                        .map_or(arg.span.end, |len| arg.span.start + len);
                    let span = syntax::trimmed_span(source, arg.span.start..end);
                    result.push(Occurrence {
                        name: source[span.clone()].into(),
                        span,
                        kind: Kind::IndexTerm,
                        declaration: true,
                        detail: arg.value.chars().take(256).collect(),
                    });
                }
            }
        } else if cmd.name == "makeindex" {
            let at = syntax::skip_space(source, cmd.span.end);
            if let Some(arg) = group(source, at, b'[') {
                for segment in syntax::key_list_segments(source, arg.span) {
                    if let Some((key, value)) = source[segment.clone()].split_once('=') {
                        if key.trim() == "name" {
                            let start = segment.end - value.len();
                            let span = syntax::trimmed_span(source, start..segment.end);
                            result.push(Occurrence {
                                name: source[span.clone()].into(),
                                span,
                                kind: Kind::IndexStream,
                                declaration: true,
                                detail: "Declared index stream".into(),
                            });
                        }
                    }
                }
            }
        }
    }
    result
}
pub(crate) fn entries(doc: &Document) -> &[Occurrence] {
    doc.extra_symbols
        .get_or_init(|| occurrences(doc))
        .as_slice()
}
pub(crate) fn at(doc: &Document, offset: usize) -> Option<&Occurrence> {
    entries(doc).iter().find(|entry| {
        entry.kind.valid(&entry.name) && entry.span.start <= offset && offset <= entry.span.end
    })
}
fn indexed<'a>(ws: &'a Workspace, scope: &BTreeSet<Url>) -> Vec<(&'a Url, &'a Occurrence)> {
    ws.documents
        .iter()
        .filter(|(uri, _)| scope.contains(*uri))
        .flat_map(|(uri, doc)| {
            entries(doc)
                .iter()
                .filter(|entry| entry.kind.valid(&entry.name))
                .map(move |entry| (uri, entry))
        })
        .collect()
}
fn location(ws: &Workspace, uri: &Url, entry: &Occurrence) -> Location {
    Location {
        uri: uri.clone(),
        range: ws.documents[uri]
            .text
            .range(entry.span.start, entry.span.end),
    }
}
pub(crate) fn definitions(ws: &Workspace, uri: &Url, entry: &Occurrence) -> Vec<Location> {
    indexed(ws, &ws.symbol_scope(uri))
        .into_iter()
        .filter(|(_, candidate)| {
            candidate.kind == entry.kind && candidate.name == entry.name && candidate.declaration
        })
        .map(|(uri, entry)| location(ws, uri, entry))
        .collect()
}
pub(crate) fn references(
    ws: &Workspace,
    uri: &Url,
    entry: &Occurrence,
    include: bool,
) -> Vec<Location> {
    let scope = rename_scope(ws, uri, entry).unwrap_or_else(|_| ws.symbol_scope(uri));
    indexed(ws, &scope)
        .into_iter()
        .filter(|(_, candidate)| {
            candidate.kind == entry.kind
                && candidate.name == entry.name
                && (include || !candidate.declaration || entry.kind == Kind::IndexTerm)
        })
        .map(|(uri, entry)| location(ws, uri, entry))
        .collect()
}
fn rename_scope(ws: &Workspace, uri: &Url, entry: &Occurrence) -> Result<BTreeSet<Url>> {
    let crate::workspace::SymbolProjects { mut scope, regions } = ws.symbol_projects(uri);
    let own = definitions(ws, uri, entry);
    if entry.kind != Kind::IndexTerm && own.len() != 1 {
        return Err(Error::invalid_params(
            "Symbol rename requires one unambiguous project declaration",
        ));
    }
    if entry.kind == Kind::IndexTerm {
        loop {
            let before = scope.len();
            let shared: Vec<_> = indexed(ws, &scope)
                .into_iter()
                .filter(|(_, other)| other.kind == entry.kind && other.name == entry.name)
                .map(|(uri, _)| uri)
                .collect();
            for project in regions
                .values()
                .filter(|project| shared.iter().any(|uri| project.contains(*uri)))
            {
                scope.extend(project.iter().cloned());
            }
            if scope.len() == before {
                break;
            }
        }
    } else {
        for project in regions
            .values()
            .filter(|project| project.contains(&own[0].uri))
        {
            scope.extend(project.iter().cloned());
        }
        for (source, _) in indexed(ws, &scope)
            .into_iter()
            .filter(|(_, other)| other.kind == entry.kind && other.name == entry.name)
        {
            for project in regions.values().filter(|project| project.contains(source)) {
                let declarations: Vec<_> = indexed(ws, project)
                    .into_iter()
                    .filter(|(_, other)| {
                        other.kind == entry.kind && other.name == entry.name && other.declaration
                    })
                    .map(|(uri, other)| location(ws, uri, other))
                    .collect();
                if declarations != own {
                    return Err(Error::invalid_params(
                        "Shared symbol resolves to different declarations in another project",
                    ));
                }
            }
        }
    }
    Ok(scope)
}
pub(crate) fn prepare(
    ws: &Workspace,
    uri: &Url,
    entry: &Occurrence,
) -> Result<Option<PrepareRenameResponse>> {
    rename_scope(ws, uri, entry)?;
    Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
        range: location(ws, uri, entry).range,
        placeholder: entry.name.clone(),
    }))
}
pub(crate) fn rename(
    ws: &Workspace,
    uri: &Url,
    entry: &Occurrence,
    name: &str,
) -> Result<Option<WorkspaceEdit>> {
    if !entry.kind.valid(name) {
        return Err(Error::invalid_params("Invalid symbol name"));
    }
    let scope = rename_scope(ws, uri, entry)?;
    let index = indexed(ws, &scope);
    if name != entry.name
        && index
            .iter()
            .any(|(_, other)| other.kind == entry.kind && other.name == name)
    {
        return Err(Error::invalid_params(
            "The target symbol name already exists",
        ));
    }
    let mut changes = BTreeMap::<Url, Vec<TextEdit>>::new();
    for (uri, other) in index
        .into_iter()
        .filter(|(_, other)| other.kind == entry.kind && other.name == entry.name)
    {
        changes.entry(uri.clone()).or_default().push(TextEdit {
            range: location(ws, uri, other).range,
            new_text: name.into(),
        });
    }
    Ok(Some(crate::features::workspace_edit(ws, changes)))
}
pub(crate) fn hover(ws: &Workspace, uri: &Url, entry: &Occurrence) -> Hover {
    let detail = indexed(ws, &ws.symbol_scope(uri))
        .into_iter()
        .find(|(_, other)| {
            other.kind == entry.kind && other.name == entry.name && other.declaration
        })
        .map(|(uri, entry)| format!("{}\n{}", entry.detail, uri.path()))
        .unwrap_or_default();
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::PlainText,
            value: format!("{} ({})\n{}", entry.name, entry.kind.description(), detail),
        }),
        range: Some(location(ws, uri, entry).range),
    }
}
pub(crate) fn completion(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<CompletionItem>> {
    let doc = ws.documents.get(uri)?;
    let context = entries(doc).iter().find(|entry| {
        entry.span.start <= offset
            && offset <= entry.span.end
            && (entry.kind == Kind::IndexTerm || !entry.declaration)
    })?;
    let prefix = &doc.text.source[context.span.start..offset];
    if prefix.contains(['\\', '{', '}', '%']) {
        return None;
    }
    let (range, extra) = syntax::name_completion_ranges(&doc.text, context.span.clone(), offset)?;
    let mut seen = BTreeSet::new();
    Some(
        indexed(ws, &ws.symbol_scope(uri))
            .into_iter()
            .filter(|(_, entry)| {
                entry.kind == context.kind
                    && entry.declaration
                    && entry.name.starts_with(prefix)
                    && seen.insert(entry.name.clone())
            })
            .map(|(_, entry)| CompletionItem {
                label: entry.name.clone(),
                kind: Some(CompletionItemKind::REFERENCE),
                detail: Some(entry.detail.clone()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: entry.name.clone(),
                })),
                additional_text_edits: (!extra.is_empty()).then(|| extra.clone()),
                ..Default::default()
            })
            .collect(),
    )
}
#[allow(deprecated)]
pub(crate) fn symbols(doc: &Document) -> Vec<DocumentSymbol> {
    let mut seen = BTreeSet::new();
    entries(doc)
        .iter()
        .filter(|entry| {
            entry.declaration
                && entry.kind.valid(&entry.name)
                && seen.insert((entry.kind, entry.name.clone()))
        })
        .map(|entry| DocumentSymbol {
            name: entry.name.clone(),
            detail: Some(format!("{} — {}", entry.kind.description(), entry.detail)),
            kind: if entry.kind == Kind::IndexStream {
                SymbolKind::NAMESPACE
            } else {
                SymbolKind::KEY
            },
            tags: None,
            deprecated: None,
            range: doc.text.range(entry.span.start, entry.span.end),
            selection_range: doc.text.range(entry.span.start, entry.span.end),
            children: None,
        })
        .collect()
}
pub(crate) fn diagnostics(ws: &Workspace, uri: &Url) -> Vec<Diagnostic> {
    let Some(doc) = ws.documents.get(uri) else {
        return vec![];
    };
    let config = ws.settings(uri).diagnostics;
    let index = indexed(ws, &ws.symbol_scope(uri));
    let mut result = Vec::new();
    for entry in entries(doc)
        .iter()
        .filter(|entry| entry.kind.valid(&entry.name) && entry.kind != Kind::IndexTerm)
    {
        let declarations: Vec<_> = index
            .iter()
            .filter(|(_, other)| {
                other.kind == entry.kind && other.name == entry.name && other.declaration
            })
            .collect();
        let enabled = if entry.kind == Kind::Glossary {
            config.undefined_glossary
        } else {
            config.undefined_indexes
        };
        if !entry.declaration
            && declarations.is_empty()
            && enabled
            && !(entry.kind == Kind::IndexStream && entry.name == "default")
        {
            result.push(Diagnostic {
                range: location(ws, uri, entry).range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("latex-lsp".into()),
                code: Some(NumberOrString::String(
                    if entry.kind == Kind::Glossary {
                        "undefined-glossary"
                    } else {
                        "undefined-index"
                    }
                    .into(),
                )),
                message: format!("Undefined {} `{}`", entry.kind.description(), entry.name),
                ..Default::default()
            });
        }
        if entry.declaration && declarations.len() > 1 {
            result.push(Diagnostic {
                range: location(ws, uri, entry).range,
                severity: Some(DiagnosticSeverity::WARNING),
                source: Some("latex-lsp".into()),
                code: Some(NumberOrString::String("duplicate-symbol".into())),
                message: format!("Duplicate {} `{}`", entry.kind.description(), entry.name),
                related_information: Some(
                    declarations
                        .into_iter()
                        .filter(|(other_uri, other)| *other_uri != uri || other.span != entry.span)
                        .take(8)
                        .map(|(uri, other)| DiagnosticRelatedInformation {
                            location: location(ws, uri, other),
                            message: "Other declaration".into(),
                        })
                        .collect(),
                ),
                ..Default::default()
            });
        }
    }
    result
}
