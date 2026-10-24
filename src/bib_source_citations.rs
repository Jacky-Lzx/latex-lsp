//! Citation candidates at every numeric execution of a pending edit.
use crate::{
    bib_section_keys,
    bib_source_catalog::Catalog,
    workspace::{DocumentKind, Workspace},
};
use tower_lsp::lsp_types::*;
pub(crate) fn completion(ws: &Workspace, uri: &Url, offset: usize) -> Option<Vec<CompletionItem>> {
    let doc = ws
        .documents
        .get(uri)
        .filter(|doc| doc.kind == DocumentKind::Latex)?;
    if !ws
        .documents
        .values()
        .any(|doc| doc.kind == DocumentKind::Latex && doc.text.source.contains("refsection"))
    {
        return None;
    }
    if !ws
        .documents
        .values()
        .any(|doc| doc.kind == DocumentKind::Latex && doc.text.source.contains("defbibentryset"))
    {
        return None;
    }
    let (selected, projects) = bib_section_keys::section_projects(ws, uri)?;
    if !selected
        || !projects
            .iter()
            .any(|project| bib_section_keys::has_source_sets(&project.docs, &project.signatures))
        || !projects.iter().any(|project| {
            bib_section_keys::source_completion_candidate(doc, &project.signatures, offset)
        })
    {
        return None;
    }
    Some(
        (|| {
            if !ws.index_truncated.is_empty() {
                return None;
            }
            let mut catalog = Catalog::default();
            let mut context = None;
            for project in &projects {
                let completion = bib_section_keys::source_completion_trace(
                    ws,
                    &project.root,
                    &project.docs,
                    &project.signatures,
                    uri,
                    offset,
                )?;
                let identity = (
                    completion.span.clone(),
                    completion.removals.clone(),
                    completion.member,
                );
                if context
                    .as_ref()
                    .is_some_and(|previous| *previous != identity)
                {
                    return None;
                }
                context = Some(identity);
                catalog.trace_completion(&completion).ok()?;
            }
            let (span, removals, member) = context?;
            let candidates = catalog
                .candidates(ws, uri, if member { "member" } else { "cite" })
                .ok()?;
            let mut items = crate::citation_search::candidate_items(
                doc,
                offset,
                span,
                candidates.iter().map(|(name, candidate)| {
                    (&candidate.uri, name.as_str(), candidate.detail.as_str())
                }),
            )?;
            items.truncate(ws.config.indexing.completion_limit.saturating_add(1));
            for item in &mut items {
                if !removals.is_empty() {
                    item.additional_text_edits
                        .get_or_insert_with(Vec::new)
                        .extend(removals.iter().map(|span| TextEdit {
                            range: doc.text.range(span.start, span.end),
                            new_text: String::new(),
                        }));
                }
            }
            Some(items)
        })()
        .unwrap_or_default(),
    )
}
