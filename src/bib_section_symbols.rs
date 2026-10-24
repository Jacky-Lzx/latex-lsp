//! Database declarations have lexical identities; relations inherit every
//! resource view in which their owner database is consumed.
use crate::{
    bib_section_keys::{self, Coexistence},
    syntax::Entry,
    workspace::{Document, DocumentKind, Workspace},
};
use std::{collections::BTreeSet, ops::Range};
use tower_lsp::lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind, Url};

struct Target<'a> {
    span: Range<usize>,
    entries: Vec<(&'a Url, &'a Document, &'a Entry)>,
}

// A declaration identity agrees across views exactly when every matching
// declaration in their union belongs to their intersection.
struct Resources {
    all: BTreeSet<Url>,
    common: BTreeSet<Url>,
}

fn resources(
    ws: &Workspace,
    uri: &Url,
    projects: Vec<bib_section_keys::Project<'_>>,
) -> Option<Resources> {
    let mut all = BTreeSet::new();
    let mut common: Option<BTreeSet<Url>> = None;
    let mut memberships = 0usize;
    let mut stored = 0usize;
    for project in projects {
        let mut scope = ws.project_documents(&project.root);
        scope.insert(project.root.clone());
        let (views, traditional) = match bib_section_keys::coexistence(ws, &project.root, &scope) {
            Coexistence::Unknown => return None,
            Coexistence::Union => (BTreeSet::from([std::sync::Arc::new(scope)]), true),
            Coexistence::Sections(views) => (views, false),
        };
        for view in views.iter().filter(|view| view.contains(uri)) {
            let mut databases = BTreeSet::new();
            for resource in view.iter() {
                memberships += 1;
                if memberships > 100_000 {
                    return None;
                }
                let doc = ws.documents.get(resource)?;
                if doc.kind != DocumentKind::Bibtex {
                    if traditional {
                        continue;
                    }
                    return None;
                }
                if all.insert(resource.clone()) {
                    stored += 2 * (resource.as_str().len() + 128);
                    if stored > 32 * 1024 * 1024 {
                        return None;
                    }
                }
                databases.insert(resource.clone());
            }
            if let Some(common) = &mut common {
                common.retain(|resource| databases.contains(resource));
            } else {
                common = Some(databases);
            }
        }
    }
    Some(Resources {
        all,
        common: common?,
    })
}

fn target<'a>(ws: &'a Workspace, uri: &'a Url, offset: usize) -> Option<Option<Target<'a>>> {
    let doc = ws
        .documents
        .get(uri)
        .filter(|doc| doc.kind == DocumentKind::Bibtex)?;
    if !ws
        .documents
        .values()
        .any(|doc| doc.kind == DocumentKind::Latex && doc.text.source.contains("refsection"))
    {
        return None;
    }
    let declarations = || {
        doc.syntax
            .citations
            .iter()
            .chain(&doc.syntax.bibliography_aliases)
    };
    let declaration =
        declarations().find(|entry| entry.span.start <= offset && offset <= entry.span.end);
    let reference = doc
        .syntax
        .citation_references
        .iter()
        .find(|entry| entry.span.start <= offset && offset <= entry.span.end);
    let entry = declaration.or(reference)?;
    // Ordinary bibliography projects retain the established symbol behavior.
    let projects = match bib_section_keys::section_projects(ws, uri) {
        Some((false, _)) => return None,
        Some((true, projects)) => projects,
        None => return Some(None),
    };
    Some((|| {
        if let Some(entry) = declaration {
            return Some(Target {
                span: entry.span.clone(),
                entries: vec![(uri, doc, entry)],
            });
        }
        // A relation in an unfinished entry or a duplicated field has no
        // certified owner/field identity, even if one operand is literal.
        if !ws.index_truncated.is_empty() || doc.text.source.len() > 16 * 1024 * 1024 {
            return None;
        }
        let owner =
            doc.syntax
                .bibliography_completion
                .types
                .iter()
                .find(|owner| {
                    owner.entry_span.as_ref().is_some_and(|span| {
                        span.start < entry.span.start && entry.span.end < span.end
                    })
                })?;
        let body = owner.entry_span.as_ref()?;
        if !doc
            .syntax
            .bibliography_entries
            .iter()
            .any(|(span, comment)| span == body && !comment)
        {
            return None;
        }
        let fields: Vec<_> = doc
            .syntax
            .bibliography_completion
            .entries
            .iter()
            .flat_map(|entry| &entry.fields)
            .filter(|field| body.start < field.span.start && field.range.end < body.end)
            .take(100_001)
            .collect();
        if fields.len() > 100_000 {
            return None;
        }
        let field = fields.iter().find(|field| {
            field.range.start < entry.span.start && entry.span.end <= field.range.end
        })?;
        if fields
            .iter()
            .filter(|candidate| candidate.name == field.name)
            .count()
            != 1
        {
            return None;
        }
        let resources = resources(ws, uri, projects)?;
        let mut resolved = Vec::new();
        let mut examined = 0usize;
        let mut bytes = 0usize;
        for resource in &resources.all {
            let (resource, doc) = ws.documents.get_key_value(resource)?;
            bytes += doc.text.source.len();
            examined += doc.syntax.citations.len() + doc.syntax.bibliography_aliases.len();
            if bytes > 16 * 1024 * 1024 || examined > 100_000 {
                return None;
            }
            for candidate in doc
                .syntax
                .citations
                .iter()
                .chain(&doc.syntax.bibliography_aliases)
                .filter(|candidate| candidate.name == entry.name)
            {
                // A matching declaration outside the common resource set means
                // that its exact URI/span identity differs between consumers.
                if !resources.common.contains(resource) {
                    return None;
                }
                resolved.push((resource, doc, candidate));
            }
        }
        Some(Target {
            span: entry.span.clone(),
            entries: resolved,
        })
    })())
}

pub(crate) fn definitions(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    Some(target(ws, uri, offset)?.map_or_else(Vec::new, |target| {
        target
            .entries
            .into_iter()
            .map(|(uri, doc, entry)| {
                Location::new(
                    uri.clone(),
                    doc.text.range(entry.span.start, entry.span.end),
                )
            })
            .collect()
    }))
}

pub(crate) fn hover(ws: &Workspace, uri: &Url, offset: usize) -> Option<Option<Hover>> {
    Some(target(ws, uri, offset)?.and_then(|target| {
        let [(entry_uri, doc, entry)] = target.entries.as_slice() else {
            return None;
        };
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::PlainText,
                value: format!(
                    "{}\n\n{}\n{}:{}",
                    entry.name,
                    entry.detail,
                    entry_uri.path(),
                    doc.text.position(entry.span.start).line + 1
                ),
            }),
            range: Some(
                ws.documents[uri]
                    .text
                    .range(target.span.start, target.span.end),
            ),
        })
    }))
}

/// A database query must not leak the old project-wide reference union while
/// the complete citation and relation occurrence graph is still unavailable.
pub(crate) fn references(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    target(ws, uri, offset).map(|_| Vec::new())
}
