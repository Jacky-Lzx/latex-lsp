//! Bibliography authoring contexts come from the ordinary entry/field scanner.
use crate::{
    parser::syntax,
    project::workspace::{Document, Workspace},
};
use lsp_types::*;
use std::{collections::BTreeSet, ops::Range};

#[derive(Clone, Debug)]
pub(crate) struct Type {
    pub span: Range<usize>,
    pub has_body: bool,
    /// Parsed construct extent, including an unfinished body while editing.
    pub entry_span: Option<Range<usize>>,
    /// An ordinary entry's literal key, associated while parsing its header.
    pub key: Option<Range<usize>>,
}

#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub name: String,
    pub span: Range<usize>,
    /// Field name, assignment and unexpanded value, excluding the comma.
    pub range: Range<usize>,
    pub assigned: bool,
    /// The value scanner confirmed a complete expression, including literals.
    pub value_valid: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Entry {
    pub kind: String,
    pub fields: Vec<Field>,
    pub gaps: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Index {
    pub types: Vec<Type>,
    pub entries: Vec<Entry>,
}

// Templates use widely supported BibTeX fields; biblatex also accepts these
// traditional names. These are useful starting fields, not schema validation.
const TYPES: &[(&str, &str, &[&str])] = &[
    (
        "article",
        "An article in a journal or periodical.",
        &["author", "title", "journal", "year"],
    ),
    (
        "book",
        "A book, with an author or editor and a publisher.",
        &["author", "title", "publisher", "year"],
    ),
    (
        "booklet",
        "A printed work without a named publisher.",
        &["title", "author", "year"],
    ),
    (
        "conference",
        "A conference paper; a traditional alias for inproceedings.",
        &["author", "title", "booktitle", "year"],
    ),
    (
        "inbook",
        "A chapter or other part of a book.",
        &["author", "title", "chapter", "publisher", "year"],
    ),
    (
        "incollection",
        "A contribution with its own title in a collection.",
        &["author", "title", "booktitle", "publisher", "year"],
    ),
    (
        "inproceedings",
        "A contribution in conference proceedings.",
        &["author", "title", "booktitle", "year"],
    ),
    (
        "manual",
        "Technical documentation or a manual.",
        &["title", "author", "year"],
    ),
    (
        "mastersthesis",
        "A master's thesis (traditional BibTeX type).",
        &["author", "title", "school", "year"],
    ),
    (
        "misc",
        "A work that does not fit another bibliography type.",
        &["author", "title", "year"],
    ),
    (
        "phdthesis",
        "A doctoral thesis (traditional BibTeX type).",
        &["author", "title", "school", "year"],
    ),
    (
        "proceedings",
        "The proceedings of a conference.",
        &["title", "editor", "year"],
    ),
    (
        "techreport",
        "A technical report (traditional BibTeX type).",
        &["author", "title", "institution", "year"],
    ),
    (
        "unpublished",
        "An unpublished work with an explanatory note.",
        &["author", "title", "note"],
    ),
    (
        "bookinbook",
        "A separately titled work within another book (biblatex).",
        &["author", "title", "booktitle", "year"],
    ),
    (
        "collection",
        "An edited collection of contributions (biblatex).",
        &["editor", "title", "publisher", "year"],
    ),
    (
        "dataset",
        "A research dataset; requires a bibliography style that supports it.",
        &["author", "title", "date", "doi"],
    ),
    (
        "inreference",
        "An entry in a reference work (biblatex).",
        &["author", "title", "booktitle", "year"],
    ),
    (
        "mvbook",
        "A multi-volume book (biblatex).",
        &["author", "title", "publisher", "year"],
    ),
    (
        "mvcollection",
        "A multi-volume edited collection (biblatex).",
        &["editor", "title", "publisher", "year"],
    ),
    (
        "mvproceedings",
        "Multi-volume conference proceedings (biblatex).",
        &["title", "editor", "year"],
    ),
    (
        "mvreference",
        "A multi-volume reference work (biblatex).",
        &["editor", "title", "publisher", "year"],
    ),
    (
        "online",
        "An online resource (biblatex).",
        &["author", "title", "url", "urldate"],
    ),
    (
        "patent",
        "A patent or patent application (biblatex).",
        &["author", "title", "number", "date"],
    ),
    (
        "periodical",
        "An entire issue of a journal or periodical (biblatex).",
        &["editor", "title", "date"],
    ),
    (
        "reference",
        "A reference work such as a dictionary (biblatex).",
        &["editor", "title", "publisher", "year"],
    ),
    (
        "report",
        "A report with an institution and type (biblatex).",
        &["author", "title", "institution", "year"],
    ),
    (
        "set",
        "A set of bibliography entries (biblatex).",
        &["entryset"],
    ),
    (
        "software",
        "Software; requires a bibliography style that supports it.",
        &["author", "title", "version", "date", "url"],
    ),
    (
        "suppbook",
        "Supplementary material in a book (biblatex).",
        &["author", "title", "booktitle", "year"],
    ),
    (
        "suppcollection",
        "Supplementary material in a collection (biblatex).",
        &["author", "title", "booktitle", "year"],
    ),
    (
        "suppperiodical",
        "Supplementary material in a periodical (biblatex).",
        &["author", "title", "journaltitle", "year"],
    ),
    (
        "thesis",
        "A thesis with a type and institution (biblatex).",
        &["author", "title", "type", "institution", "year"],
    ),
    (
        "xdata",
        "Shared data inherited through the xdata field (biblatex).",
        &[],
    ),
    (
        "comment",
        "A comment block that does not define a bibliography entry.",
        &[],
    ),
    ("string", "A bibliography string abbreviation.", &[]),
    (
        "preamble",
        "Text passed to the generated bibliography preamble.",
        &[],
    ),
];

const FIELDS: &[(&str, &str)] = &[
    ("abstract", "An abstract or summary of the work."),
    (
        "addendum",
        "Additional bibliographic information (biblatex).",
    ),
    (
        "address",
        "The publisher's address; biblatex also uses location.",
    ),
    ("afterword", "Authors of an afterword (biblatex)."),
    ("annotation", "An annotation for an annotated bibliography."),
    ("annotator", "Authors of annotations (biblatex)."),
    (
        "archiveprefix",
        "The e-print archive, such as arXiv; biblatex uses eprinttype.",
    ),
    (
        "author",
        "Authors, separated by the word 'and'; names may use 'Family, Given'. Biber also supports extended names such as 'family=Doe, given=Jane'.",
    ),
    (
        "bookauthor",
        "The author of the containing book (biblatex).",
    ),
    (
        "booksubtitle",
        "The subtitle of the containing book (biblatex).",
    ),
    (
        "booktitle",
        "The title of the containing book or proceedings.",
    ),
    ("chapter", "A chapter number or identifier."),
    ("commentator", "Authors of commentary (biblatex)."),
    (
        "crossref",
        "The key of a parent entry whose fields may be inherited.",
    ),
    (
        "date",
        "A publication date in ISO form, such as 2026-10-23 (biblatex).",
    ),
    (
        "doi",
        "A digital object identifier, such as 10.1234/example.",
    ),
    ("edition", "The edition of a book or manual."),
    ("editor", "Editors, separated by the word 'and'."),
    ("editora", "An additional editor list (biblatex)."),
    ("editorb", "An additional editor list (biblatex)."),
    ("editorc", "An additional editor list (biblatex)."),
    ("editortype", "The role of the editors (biblatex)."),
    ("eid", "An electronic article identifier (biblatex)."),
    (
        "entryset",
        "A comma-separated list of keys belonging to an entry set (biblatex).",
    ),
    ("eprint", "An identifier in an electronic archive."),
    (
        "eprintclass",
        "The class or subject category of the e-print (biblatex).",
    ),
    (
        "eprinttype",
        "The electronic archive or service, such as arxiv (biblatex).",
    ),
    ("eventdate", "The date of the event (biblatex)."),
    (
        "eventtitle",
        "The title of the conference or event (biblatex).",
    ),
    (
        "file",
        "An attachment reference in the format used by a reference manager.",
    ),
    ("foreword", "Authors of a foreword (biblatex)."),
    ("holder", "The holders of a patent (biblatex)."),
    (
        "howpublished",
        "How a work without a conventional publisher was published.",
    ),
    (
        "ids",
        "Alternate citation keys for this entry, usually separated by commas (biblatex).",
    ),
    (
        "institution",
        "The institution responsible for a report or thesis.",
    ),
    ("introduction", "Authors of an introduction (biblatex)."),
    ("isbn", "The international standard book number."),
    ("ismn", "The international standard music number."),
    ("issn", "The international standard serial number."),
    (
        "issue",
        "An issue designation that is not a journal number (biblatex).",
    ),
    (
        "issuesubtitle",
        "The subtitle of a journal issue (biblatex).",
    ),
    ("issuetitle", "The title of a journal issue (biblatex)."),
    (
        "journal",
        "The name of the journal; biblatex also uses journaltitle.",
    ),
    ("journalsubtitle", "The subtitle of the journal (biblatex)."),
    ("journaltitle", "The title of the journal (biblatex)."),
    (
        "keywords",
        "Keywords for searching or bibliography filtering.",
    ),
    (
        "label",
        "An explicit citation label for bibliography styles that support it.",
    ),
    (
        "langid",
        "The language used to format the entry (biblatex).",
    ),
    ("language", "The language of the work."),
    ("lista", "Custom bibliography literal list slot a (biblatex)."),
    ("listb", "Custom bibliography literal list slot b (biblatex)."),
    ("listc", "Custom bibliography literal list slot c (biblatex)."),
    ("listd", "Custom bibliography literal list slot d (biblatex)."),
    ("liste", "Custom bibliography literal list slot e (biblatex)."),
    ("listf", "Custom bibliography literal list slot f (biblatex)."),
    ("location", "The place of publication (biblatex)."),
    ("maintitle", "The title of a multi-volume work (biblatex)."),
    (
        "month",
        "The publication month, often a BibTeX abbreviation such as jan.",
    ),
    ("namea", "Custom bibliography name list slot a (biblatex)."),
    ("nameb", "Custom bibliography name list slot b (biblatex)."),
    ("namec", "Custom bibliography name list slot c (biblatex)."),
    (
        "note",
        "Additional information to display in the bibliography.",
    ),
    (
        "number",
        "The number of a journal issue, report, patent or series volume.",
    ),
    (
        "options",
        "Per-entry biblatex options, as a comma-separated key=value list.",
    ),
    ("organization", "The organization responsible for the work."),
    ("origdate", "The date of the original edition (biblatex)."),
    ("origlanguage", "The language of the original work (biblatex)."),
    ("origlocation", "Places of publication of the original edition (biblatex)."),
    ("origpublisher", "Publishers of the original edition (biblatex)."),
    ("pages", "A page or page range, such as 12--34."),
    ("pagetotal", "The total number of pages (biblatex)."),
    (
        "presort",
        "A sorting prefix used before the other sorting fields (biblatex).",
    ),
    (
        "primaryclass",
        "The primary archive subject class; biblatex uses eprintclass.",
    ),
    ("publisher", "The publisher of the work."),
    (
        "related",
        "A comma-separated list of related entry keys (biblatex).",
    ),
    (
        "relatedtype",
        "The relationship to the entries in related (biblatex).",
    ),
    (
        "school",
        "The institution awarding a thesis; biblatex also uses institution.",
    ),
    ("series", "The name of a publication series."),
    ("shortauthor", "An abbreviated author list (biblatex)."),
    ("shorteditor", "An abbreviated editor list (biblatex)."),
    ("shorthand", "An explicit short citation label (biblatex)."),
    ("shortjournal", "An abbreviated journal title (biblatex)."),
    ("shorttitle", "An abbreviated title (biblatex)."),
    ("sortkey", "An explicit sorting key (biblatex)."),
    (
        "sortname",
        "A name list used for sorting instead of the author or editor (biblatex).",
    ),
    ("sorttitle", "A title used for sorting (biblatex)."),
    ("subtitle", "The subtitle of the work (biblatex)."),
    (
        "title",
        "The title of the work; protect significant capitalization with braces.",
    ),
    (
        "translator",
        "Translators, separated by the word 'and' (biblatex).",
    ),
    ("type", "The type of a report, thesis or patent."),
    ("url", "An absolute URL for the work."),
    (
        "urldate",
        "The access date of an online resource (biblatex).",
    ),
    ("venue", "The location of a conference or event (biblatex)."),
    ("version", "The version of a work or software (biblatex)."),
    ("volume", "The volume of a journal, book or series."),
    (
        "volumes",
        "The number of volumes in a multi-volume work (biblatex).",
    ),
    (
        "xdata",
        "A comma-separated list of shared data entry keys (biblatex).",
    ),
    ("xref", "A related parent entry key (biblatex)."),
    (
        "year",
        "The year of publication; biblatex also accepts a complete date.",
    ),
];

pub(crate) fn type_description(name: &str) -> Option<&'static str> {
    TYPES
        .iter()
        .find(|(entry_type, _, _)| *entry_type == name)
        .map(|(_, description, _)| *description)
}

pub(crate) fn field_description(name: &str) -> Option<&'static str> {
    FIELDS
        .iter()
        .find(|(field, _)| *field == name)
        .map(|(_, description)| *description)
}

fn contains(span: &Range<usize>, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

fn snippet_name(name: &str) -> String {
    name.replace('\\', "\\\\").replace('$', "\\$")
}

fn template(name: &str, fields: &[&str], newline: &str) -> String {
    match name {
        "comment" => return "comment{${1:text}}$0".into(),
        "string" => return "string{${1:name} = {${2:value}}}$0".into(),
        "preamble" => return "preamble{\"${1:text}\"}$0".into(),
        _ => {}
    }
    let escaped_name = snippet_name(name);
    let mut result = format!("{escaped_name}{{${{1:key}},");
    for (index, field) in fields.iter().enumerate() {
        result.push_str(newline);
        result.push_str(&format!("  {field} = {{${{{}:}}}},", index + 2));
    }
    result.push_str(newline);
    if fields.is_empty() {
        result.push_str("  $0");
        result.push_str(newline);
    }
    result.push('}');
    if !fields.is_empty() {
        result.push_str("$0");
    }
    result
}

fn field_context(doc: &Document, offset: usize) -> Option<(&Entry, Range<usize>, bool)> {
    for entry in &doc.syntax.bibliography_completion.entries {
        if let Some(field) = entry
            .fields
            .iter()
            .find(|field| contains(&field.span, offset))
        {
            return Some((entry, field.span.clone(), field.assigned));
        }
        if let Some(gap) = entry.gaps.iter().find(|gap| contains(gap, offset)) {
            let line_start = doc.text.source[..offset].rfind('\n').map_or(0, |at| at + 1);
            // These gaps contain only whitespace and real BibTeX comments.
            // A percent sign in a field value never belongs to a gap.
            if doc.text.source[gap.start.max(line_start)..offset].contains('%') {
                return None;
            }
            let after = syntax::skip_space(&doc.text.source, offset);
            return Some((
                entry,
                offset..offset,
                doc.text.source.as_bytes().get(after) == Some(&b'='),
            ));
        }
    }
    None
}

pub(crate) fn completion(
    workspace: &Workspace,
    doc: &Document,
    offset: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let source = &doc.text.source;
    if let Some(kind) = doc
        .syntax
        .bibliography_completion
        .types
        .iter()
        .find(|kind| contains(&kind.span, offset))
    {
        let prefix = source[kind.span.start..offset].to_ascii_lowercase();
        let mut candidates: Vec<_> = TYPES
            .iter()
            .map(|&(name, docs, fields)| (name.to_owned(), docs.to_owned(), fields))
            .collect();
        let mut seen: BTreeSet<_> = TYPES.iter().map(|(name, _, _)| name.to_string()).collect();
        for (uri, other) in &workspace.documents {
            if !scope.contains(uri) {
                continue;
            }
            for kind in &other.syntax.bibliography_completion.types {
                let name = &other.text.source[kind.span.clone()];
                if kind.has_body
                    && !name.is_empty()
                    && !name.as_bytes()[0].is_ascii_digit()
                    && seen.insert(name.to_ascii_lowercase())
                {
                    candidates.push((name.into(), "Entry type used in this project.".into(), &[]));
                }
            }
        }
        let after = syntax::skip_space(source, kind.span.end);
        let snippets = workspace.snippets
            && !kind.has_body
            && (after == source.len() || source.as_bytes().get(after) == Some(&b'@'));
        return Some(
            candidates
                .into_iter()
                .filter(|(name, _, _)| name.to_ascii_lowercase().starts_with(&prefix))
                .take(workspace.completion_budget())
                .map(|(name, docs, fields)| CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some("BibTeX entry type".into()),
                    documentation: Some(Documentation::String(docs)),
                    insert_text_format: Some(if snippets {
                        InsertTextFormat::SNIPPET
                    } else {
                        InsertTextFormat::PLAIN_TEXT
                    }),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: doc.text.range(kind.span.start, kind.span.end),
                        new_text: if snippets {
                            template(&name, fields, doc.text.newline())
                        } else {
                            name.clone()
                        },
                    })),
                    sort_text: Some(name.to_ascii_lowercase()),
                    ..Default::default()
                })
                .collect(),
        );
    }
    let (entry, span, assigned) = field_context(doc, offset)?;
    let prefix = source[span.start..offset].to_ascii_lowercase();
    let after = syntax::skip_space(source, span.end);
    let comma = source.as_bytes().get(after) == Some(&b',');
    let common = TYPES
        .iter()
        .find(|(name, _, _)| *name == entry.kind)
        .map(|(_, _, fields)| *fields)
        .unwrap_or(&[]);
    let used: BTreeSet<_> = entry
        .fields
        .iter()
        .filter(|field| field.assigned && field.span != span)
        .map(|field| field.name.as_str())
        .collect();
    let mut candidates: Vec<_> = FIELDS
        .iter()
        .map(|&(name, docs)| (name, docs, false))
        .collect();
    let mut seen: BTreeSet<_> = FIELDS.iter().map(|(name, _)| name.to_string()).collect();
    // Prefer this buffer's spelling when the same custom field occurs in
    // multiple resources. Unassigned names may still be unfinished prefixes.
    for other in std::iter::once(doc).chain(
        workspace
            .documents
            .iter()
            .filter(|(uri, _)| scope.contains(*uri))
            .map(|(_, doc)| doc),
    ) {
        for field in other
            .syntax
            .bibliography_completion
            .entries
            .iter()
            .flat_map(|entry| &entry.fields)
            .filter(|field| field.assigned)
        {
            if seen.insert(field.name.clone()) {
                candidates.push((
                    &other.text.source[field.span.clone()],
                    "Custom field used in this project.",
                    true,
                ));
            }
        }
    }
    Some(
        candidates
            .into_iter()
            .filter(|(name, _, _)| {
                let normalized = name.to_ascii_lowercase();
                normalized.starts_with(&prefix) && !used.contains(normalized.as_str())
            })
            .take(workspace.completion_budget())
            .map(|(name, docs, custom)| {
                let snippet = workspace.snippets && !assigned;
                let normalized = name.to_ascii_lowercase();
                let common = common.contains(&normalized.as_str());
                CompletionItem {
                    label: name.into(),
                    kind: Some(CompletionItemKind::FIELD),
                    detail: Some(
                        if common {
                            "Common field for this entry type"
                        } else if custom {
                            "Custom field used in this project"
                        } else {
                            "BibTeX / biblatex field"
                        }
                        .into(),
                    ),
                    documentation: Some(Documentation::String(docs.into())),
                    insert_text_format: Some(if snippet {
                        InsertTextFormat::SNIPPET
                    } else {
                        InsertTextFormat::PLAIN_TEXT
                    }),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: doc.text.range(span.start, span.end),
                        new_text: if snippet {
                            format!(
                                "{} = {{${{1:}}}}{}",
                                snippet_name(name),
                                if comma { "" } else { "," }
                            )
                        } else {
                            name.into()
                        },
                    })),
                    sort_text: Some(format!(
                        "{}{normalized}",
                        if common {
                            '0'
                        } else if custom {
                            '2'
                        } else {
                            '1'
                        }
                    )),
                    ..Default::default()
                }
            })
            .collect(),
    )
}

pub(crate) fn hover(doc: &Document, offset: usize) -> Option<Hover> {
    let source = &doc.text.source;
    let (name, span, description) = if let Some(kind) = doc
        .syntax
        .bibliography_completion
        .types
        .iter()
        .find(|kind| contains(&kind.span, offset))
    {
        let name = source[kind.span.clone()].to_ascii_lowercase();
        let description = type_description(&name)?;
        (name, kind.span.clone(), description)
    } else {
        let field = doc
            .syntax
            .bibliography_completion
            .entries
            .iter()
            .flat_map(|entry| &entry.fields)
            .find(|field| contains(&field.span, offset))?;
        let description = FIELDS.iter().find(|(name, _)| *name == field.name)?.1;
        (field.name.clone(), field.span.clone(), description)
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::PlainText,
            value: format!("{name}\n\n{description}"),
        }),
        range: Some(doc.text.range(span.start, span.end)),
    })
}

/// Resolve the entry owning a primary key or IDS alias without rescanning the
/// database for each completion. Entry headers are ordered by source position.
pub(crate) fn xdata_owner<'a>(doc: &'a Document, key: &std::ops::Range<usize>) -> Option<&'a str> {
    let owners = &doc.syntax.bibliography_completion.types;
    let index = owners
        .partition_point(|owner| owner.span.start < key.start)
        .checked_sub(1)?;
    let owner = owners.get(index)?;
    let body = owner.entry_span.as_ref()?;
    if body.start >= key.start
        || key.end > body.end
        || !doc.text.source[owner.span.clone()].eq_ignore_ascii_case("xdata")
    {
        return None;
    }
    Some(&doc.text.source[owner.key.as_ref()?.clone()])
}
pub(crate) fn is_xdata_entry(doc: &Document, key: &std::ops::Range<usize>) -> bool {
    xdata_owner(doc, key).is_some()
}
/// Type filtering must retain competing declarations and an alias's primary
/// owner. Borrow names and bound the complete classification pass.
pub(crate) fn xdata_candidates<'a>(
    workspace: &'a Workspace,
    scope: &BTreeSet<Url>,
) -> BTreeSet<&'a str> {
    let mut names = std::collections::BTreeMap::<&str, Option<&str>>::new();
    let mut bytes = 0usize;
    let mut count = 0usize;
    if !workspace.index_truncated.is_empty() {
        return BTreeSet::new();
    }
    for uri in scope {
        let Some(doc) = workspace.documents.get(uri) else {
            return BTreeSet::new();
        };
        bytes += doc.text.source.len();
        count += doc.syntax.citations.len() + doc.syntax.bibliography_aliases.len();
        if bytes > 16 * 1024 * 1024 || count > 100_000 {
            return BTreeSet::new();
        }
        for entry in doc
            .syntax
            .citations
            .iter()
            .chain(&doc.syntax.bibliography_aliases)
        {
            names
                .entry(entry.name.as_str())
                .and_modify(|owner| *owner = None)
                .or_insert_with(|| xdata_owner(doc, &entry.span));
        }
    }
    names
        .iter()
        .filter_map(|(name, owner)| {
            names
                .get((*owner)?)
                .is_some_and(Option::is_some)
                .then_some(*name)
        })
        .collect()
}
