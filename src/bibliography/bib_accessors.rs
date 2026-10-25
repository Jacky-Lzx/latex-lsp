//! Complete biblatex data selectors without treating them as citation keys.
use crate::{
    bibliography::{bib_completion, citations},
    language::signature,
    parser::syntax,
    project::workspace::{DocumentKind, Workspace},
};
use lsp_types::*;
use std::{collections::BTreeSet, ops::Range as Span};

#[derive(Clone, Copy)]
enum Kind {
    Field,
    List,
    Name,
    EntryType,
}

impl Kind {
    fn names(self) -> &'static [&'static str] {
        // Common output types, fields and lists in biblatex's default model.
        // Input-only constructs and legacy aliases are deliberately absent.
        match self {
            Self::EntryType => &[
                "article",
                "book",
                "bookinbook",
                "booklet",
                "collection",
                "dataset",
                "inbook",
                "incollection",
                "inproceedings",
                "inreference",
                "manual",
                "misc",
                "mvbook",
                "mvcollection",
                "mvproceedings",
                "mvreference",
                "online",
                "patent",
                "periodical",
                "proceedings",
                "reference",
                "report",
                "set",
                "software",
                "suppbook",
                "suppcollection",
                "suppperiodical",
                "thesis",
                "unpublished",
            ],
            Self::Field => &[
                "abstract",
                "addendum",
                "annotation",
                "booksubtitle",
                "booktitle",
                "chapter",
                "doi",
                "edition",
                "eid",
                "eprint",
                "eprintclass",
                "eprinttype",
                "eventtitle",
                "howpublished",
                "isbn",
                "ismn",
                "issn",
                "issue",
                "issuesubtitle",
                "issuetitle",
                "journalsubtitle",
                "journaltitle",
                "label",
                "langid",
                "maintitle",
                "month",
                "note",
                "number",
                "pages",
                "pagetotal",
                "relatedtype",
                "series",
                "shorthand",
                "shortjournal",
                "shorttitle",
                "subtitle",
                "title",
                "type",
                "url",
                "venue",
                "version",
                "volume",
                "volumes",
                "year",
            ],
            Self::List => &[
                "institution",
                "language",
                "lista",
                "listb",
                "listc",
                "listd",
                "liste",
                "listf",
                "location",
                "organization",
                "origlanguage",
                "origlocation",
                "origpublisher",
                "publisher",
            ],
            Self::Name => &[
                "afterword",
                "annotator",
                "author",
                "bookauthor",
                "commentator",
                "editor",
                "editora",
                "editorb",
                "editorc",
                "foreword",
                "holder",
                "introduction",
                "namea",
                "nameb",
                "namec",
                "shortauthor",
                "shorteditor",
                "translator",
            ],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Field => "Biblatex field",
            Self::List => "Biblatex literal list",
            Self::Name => "Biblatex name list",
            Self::EntryType => "Biblatex entry type",
        }
    }

    fn completion_kind(self) -> CompletionItemKind {
        match self {
            Self::EntryType => CompletionItemKind::CLASS,
            _ => CompletionItemKind::FIELD,
        }
    }

    fn description(self, name: &str) -> &'static str {
        match self {
            Self::EntryType => bib_completion::type_description(name),
            _ => bib_completion::field_description(name),
        }
        .unwrap_or(self.label())
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | ':' | '-'))
}

fn input_hook_context(
    workspace: &Workspace,
    uri: &Url,
    offset: usize,
) -> Option<(Kind, Span<usize>, Vec<Span<usize>>)> {
    if !workspace.index_truncated.is_empty() {
        return None;
    }
    let doc = workspace.documents.get(uri)?;
    let source = &doc.text.source;
    let start = source[..offset]
        .rmatch_indices('[')
        .take(128)
        .find(|(at, _)| !doc.syntax.is_ignored(*at))?
        .0;
    let name =
        syntax::active_commented_name_argument(source, start, b'[', offset, false, valid_name)
            .or_else(|| {
                syntax::active_label_argument(source, start, b'[', offset)
                    .filter(|span| {
                        let value = source[span.clone()].trim();
                        value.is_empty() || valid_name(value)
                    })
                    .map(|span| (span, vec![]))
            })?;
    let mut found = false;
    let mut bytes = 0usize;
    let mut commands = 0usize;
    let mut definitions = 0usize;
    for (scope, consumers) in crate::symbols::environment_calls::project_contexts(workspace) {
        if !consumers.contains(uri) {
            continue;
        }
        let docs: Vec<_> = scope
            .iter()
            .filter_map(|uri| workspace.documents.get(uri))
            .filter(|doc| doc.kind == DocumentKind::Latex)
            .collect();
        bytes = bytes.saturating_add(docs.iter().map(|doc| doc.text.source.len()).sum::<usize>());
        commands = commands.saturating_add(
            docs.iter()
                .map(|doc| doc.syntax.commands.len())
                .sum::<usize>(),
        );
        definitions = definitions.saturating_add(
            docs.iter()
                .map(|doc| doc.syntax.definitions.len() + doc.syntax.environments.len())
                .sum::<usize>(),
        );
        if bytes > 32 * 1024 * 1024 || commands > 200_000 || definitions > 8192 {
            return None;
        }
        let signatures = signature::CommandSignatures::new(
            docs.iter()
                .map(|doc| (doc.text.source.as_str(), &doc.syntax)),
        );
        let context = crate::symbols::named_references::Context::new(doc, &signatures);
        let empty = BTreeSet::new();
        let known = doc
            .syntax
            .commands
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, command)| command.span.end <= start)
            .take(128)
            .any(|(index, command)| {
                if context.operand(doc, command.span.start) {
                    return false;
                }
                let (name, span) = signatures
                    .named_invocation(source, &doc.syntax, index, &empty, &context.declarations)
                    .unwrap_or((&command.name, command.span.clone()));
                syntax::skip_space(source, span.end) == start
                    && signatures.invokes_builtin(name, "AtDataInput")
            });
        if !known {
            return None;
        }
        found = true;
    }
    found.then_some((Kind::EntryType, name.0, name.1))
}

fn context(
    workspace: &Workspace,
    uri: &Url,
    offset: usize,
    scope: &BTreeSet<Url>,
) -> Option<(Kind, Span<usize>, Vec<Span<usize>>)> {
    let doc = workspace.documents.get(uri)?;
    if doc.kind != DocumentKind::Latex || doc.syntax.is_ignored(offset.saturating_sub(1)) {
        return None;
    }
    if let Some(context) = input_hook_context(workspace, uri, offset) {
        return Some(context);
    }
    let source = &doc.text.source;
    for command in doc
        .syntax
        .commands
        .iter()
        .rev()
        .filter(|command| {
            command.span.end <= offset && !doc.syntax.control_operands.contains(&command.span.start)
        })
        .take(128)
    {
        let (kind, notes) = match command.name.as_str() {
            "printfield" => (Kind::Field, Some(1)),
            "printlist" => (Kind::List, Some(2)),
            "printnames" => (Kind::Name, Some(2)),
            "thefield" | "strfield" | "iffieldundef" | "iffieldequalstr" | "iffieldequals"
            | "iffieldsequal" => (Kind::Field, Some(0)),
            "iflistundef" | "iflistequals" => (Kind::List, Some(0)),
            "ifnameundef" | "ifnameequals" => (Kind::Name, Some(0)),
            "ifentrytype" => (Kind::EntryType, Some(0)),
            "citefield" => (Kind::Field, None),
            "citelist" => (Kind::List, None),
            "citename" => (Kind::Name, None),
            _ => continue,
        };
        let mut start = if let Some(notes) = notes {
            let mut at = citations::space(source, command.span.end);
            for _ in 0..notes {
                if source.as_bytes().get(at) != Some(&b'[') {
                    break;
                }
                let (_, end) = syntax::bracket_argument(source, at)?;
                at = citations::space(source, end);
            }
            at
        } else {
            let parsed =
                citations::arguments(source, command.span.end, citations::style(&command.name)?);
            let Some(field) = parsed
                .fields
                .iter()
                .find(|field| field.role == citations::Role::Selector)
            else {
                continue;
            };
            field.argument.outer_span.start
        };
        let selectors = if command.name == "iffieldsequal" {
            2
        } else {
            1
        };
        for selector in 0..selectors {
            let name = syntax::active_commented_name_argument(
                source, start, b'{', offset, true, valid_name,
            )
            .or_else(|| {
                syntax::active_name_argument(source, start, b'{', offset)
                    .filter(|span| {
                        let value = source[span.clone()].trim();
                        value.is_empty() || valid_name(value)
                    })
                    .map(|span| (span, Vec::new()))
            });
            if let Some((span, removals)) = name {
                if signature::uses_builtin(workspace, &command.name, scope) {
                    return Some((kind, span, removals));
                }
            }
            if selector + 1 == selectors {
                break;
            }
            let Some((_, end)) = syntax::group(source, start, b'{', b'}') else {
                break;
            };
            if end > offset {
                break;
            }
            start = citations::space(source, end);
        }
    }
    None
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &Url,
    offset: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let (kind, span, removals) = context(workspace, uri, offset, scope)?;
    let doc = &workspace.documents[uri];
    let prefix = doc.text.source[span.start..offset]
        .trim_start()
        .to_ascii_lowercase();
    let (range, mut additional) = syntax::name_completion_ranges(&doc.text, span, offset)?;
    additional.extend(removals.into_iter().map(|span| TextEdit {
        range: doc.text.range(span.start, span.end),
        new_text: String::new(),
    }));
    Some(
        kind.names()
            .iter()
            .filter(|name| name.starts_with(&prefix))
            .map(|name| CompletionItem {
                label: (*name).into(),
                kind: Some(kind.completion_kind()),
                detail: Some(kind.label().into()),
                documentation: Some(Documentation::String(kind.description(name).into())),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: (*name).into(),
                })),
                additional_text_edits: (!additional.is_empty()).then(|| additional.clone()),
                ..Default::default()
            })
            .collect(),
    )
}

pub(crate) fn hover(workspace: &Workspace, uri: &Url, offset: usize) -> Option<Hover> {
    if workspace.documents.get(uri)?.syntax.is_ignored(offset) {
        return None;
    }
    let scope = workspace.symbol_scope(uri);
    let (kind, span, _) = context(workspace, uri, offset, &scope)?;
    let doc = &workspace.documents[uri];
    let span = syntax::trimmed_span(&doc.text.source, span);
    let name = &doc.text.source[span.clone()];
    kind.names().contains(&name).then(|| Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::PlainText,
            value: format!("{name}\n\n{}", kind.description(name)),
        }),
        range: Some(doc.text.range(span.start, span.end)),
    })
}
