//! Complete bibliography source in completion documentation and hover windows.
use crate::{
    parser::syntax,
    project::workspace::{Document, DocumentKind, Workspace},
};
use lsp_types::*;

fn raw<'a>(doc: &'a Document, entry: &syntax::Entry) -> Option<&'a str> {
    if doc.kind != DocumentKind::Bibtex {
        return None;
    }
    let types = &doc.syntax.bibliography_completion.types;
    let owner = types.get(
        types
            .partition_point(|owner| owner.span.start < entry.span.start)
            .checked_sub(1)?,
    )?;
    let span = owner.entry_span.as_ref()?;
    span.contains(&entry.span.start)
        .then(|| &doc.text.source[span.clone()])
}

pub(crate) fn contents(doc: &Document, entry: &syntax::Entry) -> Option<MarkupContent> {
    let source = raw(doc, entry)?;
    // A literal code fence inside a field must not terminate the preview.
    let longest = source
        .split(|ch| ch != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(3.max(longest + 1));
    Some(MarkupContent {
        kind: MarkupKind::Markdown,
        value: format!("{fence}bibtex\n{source}\n{fence}"),
    })
}

pub(crate) fn hover(doc: &Document, entry: &syntax::Entry) -> MarkupContent {
    contents(doc, entry).unwrap_or_else(|| MarkupContent {
        kind: MarkupKind::PlainText,
        value: format!("{}\n\n{}", entry.name, entry.detail),
    })
}

pub(crate) fn documentation(
    ws: &Workspace,
    uri: &Url,
    name: &str,
    detail: &str,
) -> Option<Documentation> {
    // A materialized source set can shadow a database book with the same key.
    // Keep its execution-derived description instead of previewing that book.
    if !detail.starts_with("BibTeX ") && !detail.starts_with("Biblatex alias for ") {
        return None;
    }
    let doc = ws.documents.get(uri)?;
    let entry = doc
        .syntax
        .citations
        .iter()
        .chain(&doc.syntax.bibliography_aliases)
        .find(|entry| entry.name == name)?;
    contents(doc, entry).map(Documentation::MarkupContent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_preserve_complete_records_and_exclude_neighbors() {
        for record in [
            "@article{key,\r\n  title = {Nested {title} with @book{fake}}, % keep\r\n  pages={1--9},\r\n}",
            "@Book(key, title=\"A {nested} title\", year=2026)",
            "@book{key}",
        ] {
            let uri = Url::parse("file:///中文目录/refs.bib").unwrap();
            let doc = Document::new(&uri, format!("@comment{{ignored}}\n{record}\n@book{{after,title={{Other}}}}"), Some(1));
            let entry = doc.syntax.citations.iter().find(|entry| entry.name == "key").unwrap();
            assert_eq!(raw(&doc, entry), Some(record));
            assert_eq!(contents(&doc, entry).unwrap().value, format!("```bibtex\n{record}\n```"));
        }
    }

    #[test]
    fn aliases_show_the_owning_record_and_literal_fences_stay_inside_it() {
        let uri = Url::parse("file:///refs.bib").unwrap();
        let record = "@book{primary,ids={alias},title={A ``` literal fence}}";
        let doc = Document::new(&uri, record.into(), Some(1));
        let alias = doc
            .syntax
            .bibliography_aliases
            .iter()
            .find(|entry| entry.name == "alias")
            .unwrap();
        assert_eq!(raw(&doc, alias), Some(record));
        assert_eq!(
            contents(&doc, alias).unwrap().value,
            format!("````bibtex\n{record}\n````")
        );
        let mut ws = Workspace::default();
        ws.documents.insert(uri.clone(), doc);
        assert!(documentation(&ws, &uri, "alias", "Biblatex alias for primary").is_some());
        assert!(documentation(&ws, &uri, "primary", "Dynamic bibliography set: child").is_none());
        assert!(documentation(&ws, &uri, "alias", "Bibliography alias of primary").is_none());
    }
}
