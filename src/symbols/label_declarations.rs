//! Exclude declarations proved to be data, without certifying unknown code.
use crate::{
    language::signature::{source_alias, CommandSignatures},
    parser::syntax::{self, Entry},
    project::workspace::{Document, DocumentKind, Workspace},
};
use lsp_types::Url;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default)]
pub(crate) struct Index {
    inactive: BTreeMap<Url, BTreeSet<usize>>,
}

impl Index {
    pub(crate) fn new(workspace: &Workspace, uri: &Url) -> Self {
        if workspace
            .documents
            .values()
            .all(|doc| doc.syntax.labels.is_empty())
        {
            return Self::default();
        }
        let scope = workspace.symbol_scope(uri);
        let projects: Vec<_> = workspace
            .symbol_regions()
            .into_values()
            .filter(|project| project.contains(uri) && project.is_subset(&scope))
            .collect();
        if projects.is_empty() {
            return Self::for_scope(workspace, &scope).as_ref().clone();
        }
        let mut result = Self::default();
        for project in projects {
            for (uri, inactive) in &Self::for_scope(workspace, &project).inactive {
                result
                    .inactive
                    .entry(uri.clone())
                    .and_modify(|known| known.retain(|at| inactive.contains(at)))
                    .or_insert_with(|| inactive.clone());
            }
        }
        result
    }

    pub(crate) fn for_scope(workspace: &Workspace, scope: &BTreeSet<Url>) -> std::sync::Arc<Self> {
        super::query_cache::labels(workspace, scope, || Self::build(workspace, scope))
    }

    pub(super) fn cache_bytes(&self) -> usize {
        self.inactive
            .iter()
            .map(|(uri, positions)| {
                uri.as_str().len() + 64 + positions.len() * (std::mem::size_of::<usize>() + 64)
            })
            .sum()
    }

    fn build(workspace: &Workspace, scope: &BTreeSet<Url>) -> Self {
        let docs: Vec<_> = workspace
            .documents
            .iter()
            .filter(|(uri, doc)| scope.contains(*uri) && doc.kind == DocumentKind::Latex)
            .collect();
        if docs.iter().all(|(_, doc)| doc.syntax.labels.is_empty())
            || docs
                .iter()
                .map(|(_, doc)| doc.text.source.len())
                .sum::<usize>()
                > 16 * 1024 * 1024
            || docs
                .iter()
                .map(|(_, doc)| doc.syntax.commands.len())
                .sum::<usize>()
                > 100_000
        {
            return Self {
                inactive: docs
                    .into_iter()
                    .map(|(uri, _)| (uri.clone(), BTreeSet::new()))
                    .collect(),
            };
        }
        let sources: Vec<_> = docs.iter().map(|(_, doc)| *doc).collect();
        let signatures = super::query_cache::project_signatures(workspace, scope);
        let reader_context = reader_context(&sources, &signatures);
        Self {
            inactive: docs
                .into_iter()
                .map(|(uri, doc)| {
                    (
                        uri.clone(),
                        if doc.syntax.labels.is_empty() {
                            BTreeSet::new()
                        } else {
                            inactive(
                                doc,
                                &signatures,
                                &readers(&sources, doc, &signatures, &reader_context),
                            )
                        },
                    )
                })
                .collect(),
        }
    }

    pub(crate) fn contains(&self, uri: &Url, entry: &Entry) -> bool {
        !self.is_data(uri, entry.span.start)
    }

    pub(crate) fn is_data(&self, uri: &Url, at: usize) -> bool {
        self.inactive
            .get(uri)
            .is_some_and(|positions| positions.contains(&at))
    }
}

pub(crate) fn local(doc: &Document) -> BTreeSet<usize> {
    if doc.syntax.labels.is_empty()
        || doc.text.source.len() > 16 * 1024 * 1024
        || doc.syntax.commands.len() > 100_000
    {
        return BTreeSet::new();
    }
    let signatures = CommandSignatures::new([(doc.text.source.as_str(), &doc.syntax)]);
    let sources = [doc];
    let reader_context = reader_context(&sources, &signatures);
    inactive(
        doc,
        &signatures,
        &readers(&sources, doc, &signatures, &reader_context),
    )
}

fn inactive(
    doc: &Document,
    signatures: &CommandSignatures<'_>,
    readers: &BTreeMap<String, usize>,
) -> BTreeSet<usize> {
    // Failure to finish a proof preserves the structural index.
    if doc.text.source.len() > 16 * 1024 * 1024 || doc.syntax.commands.len() > 100_000 {
        return BTreeSet::new();
    }
    let tokens = signatures.nonexecuting_argument_tokens(&doc.text.source, &doc.syntax, readers);
    doc.syntax
        .labels
        .iter()
        .filter(|entry| {
            let before = doc
                .syntax
                .commands
                .partition_point(|command| command.span.end <= entry.span.start);
            before
                .checked_sub(1)
                .and_then(|index| doc.syntax.commands.get(index))
                .is_some_and(|command| tokens.contains(&command.span.start))
        })
        .map(|entry| entry.span.start)
        .collect()
}

#[derive(Default)]
struct ReaderContext {
    native: [bool; 2],
    custom: bool,
}

/// Facts shared by all consumers of one immutable project interpretation.
fn reader_context(docs: &[&Document], signatures: &CommandSignatures<'_>) -> ReaderContext {
    // These interfaces can change replacement text without adding a competing
    // declaration to the structural index. No execution proof survives them.
    let modifies = |name: &str| {
        name.contains("appto")
            || name.contains("preto")
            || name.contains("patchcmd")
            || matches!(name, "scantokens" | "g@addto@macro")
    };
    if docs.iter().any(|doc| {
        !doc.syntax.command_undefinitions.is_empty()
            || doc
                .syntax
                .commands
                .iter()
                .enumerate()
                .any(|(index, command)| {
                    modifies(&command.name)
                        || command.name == "csname"
                            && crate::parser::syntax::literal_csname(
                                &doc.text.source,
                                &doc.syntax.commands,
                                index,
                            )
                            .is_none()
                        || (command.name == "csuse"
                            || crate::symbols::named_commands::declares(&command.name))
                            && command.argument.as_ref().is_none_or(|argument| {
                                !crate::parser::syntax::is_static_key(&argument.value)
                            })
                        || (crate::project::source_inputs::source_file_command(&command.name)
                            || crate::parser::syntax::loader_extension(&command.name).is_some()
                            || crate::parser::syntax::code_input_command(&command.name))
                            && if crate::parser::syntax::file_list(&command.name) {
                                crate::parser::syntax::complete_file_list_spans(
                                    &doc.text.source,
                                    command,
                                )
                                .is_none()
                            } else {
                                crate::parser::syntax::file_argument(&doc.text.source, command)
                                    .is_none_or(|argument| {
                                        !crate::parser::syntax::is_literal_file_path(
                                            &argument.value,
                                        )
                                    })
                            }
                        || command.name == "csuse"
                            && command
                                .argument
                                .as_ref()
                                .is_some_and(|argument| modifies(&argument.value))
                        || command.name == "csname"
                            && crate::parser::syntax::literal_csname(
                                &doc.text.source,
                                &doc.syntax.commands,
                                index,
                            )
                            .is_some_and(|span| modifies(&doc.text.source[span]))
                })
    }) {
        return ReaderContext::default();
    }
    let native = ["typeout", "message"].map(|name| signatures.command_unchanged(name));
    // Conditional/local installation cannot prove a user's current meaning.
    // Native readers above remain useful even when user proofs are unavailable.
    let custom = !docs.iter().any(|doc| {
        doc.syntax.commands.iter().any(|command| {
            command.name.starts_with("if")
                || matches!(
                    command.name.as_str(),
                    "unless" | "begingroup" | "endgroup" | "bgroup" | "egroup"
                )
        })
    });
    ReaderContext { native, custom }
}

fn readers(
    docs: &[&Document],
    consumer: &Document,
    signatures: &CommandSignatures<'_>,
    context: &ReaderContext,
) -> BTreeMap<String, usize> {
    let mut result: BTreeMap<_, _> = ["typeout", "message"]
        .into_iter()
        .zip(context.native)
        .filter(|(_, unchanged)| *unchanged)
        .map(|(name, _)| (name.to_owned(), 0))
        .collect();
    if !context.custom {
        return result;
    }
    let mut work = 0usize;
    let mut definitions = BTreeMap::<&str, Vec<(bool, Option<String>, usize)>>::new();
    for doc in docs {
        // Most chapters only consume macros. Building argument/execution
        // positions for them cannot contribute a reader declaration, and
        // repeats a full source scan for every consumer in the project.
        if doc.syntax.definitions.is_empty() {
            continue;
        }
        if doc.syntax.definitions.len() > 4096 {
            return result;
        }
        // Only the current consumer can certify a local reader installation.
        // Other sources still contribute conflicts and the same work budget,
        // but their execution positions cannot affect this proof.
        let positions = std::ptr::eq(*doc, consumer)
            .then(|| signatures.source_command_positions(&doc.text.source, &doc.syntax));
        for entry in &doc.syntax.definitions {
            work += doc.text.source.len();
            if work > 16 * 1024 * 1024 {
                return result;
            }
            let declaration = entry.detail.strip_prefix("Defined with \\").unwrap_or("");
            let before = doc
                .syntax
                .commands
                .partition_point(|command| command.span.start < entry.span.start);
            let plain = signatures.command_unchanged(declaration)
                && std::ptr::eq(*doc, consumer)
                && !matches!(declaration, "providecommand" | "providerobustcmd")
                && (crate::parser::syntax::declares_traditional_command(declaration)
                    || matches!(declaration, "def" | "gdef" | "csdef" | "csgdef"))
                && doc.syntax.commands[..before]
                    .iter()
                    .rev()
                    .find(|command| command.name == declaration)
                    .is_some_and(|command| {
                        positions
                            .as_ref()
                            .is_some_and(|positions| positions.contains(&command.span.start))
                            && !crate::project::resource_context::grouped(
                                &doc.text.source,
                                &doc.syntax,
                                command.span.start,
                            )
                    });
            let alias = plain
                .then(|| source_alias(&doc.text.source, &doc.syntax, entry))
                .flatten();
            let dropped = plain
                && alias.is_none()
                && crate::language::signature::command_definition_body(
                    &doc.text.source,
                    &doc.syntax,
                    entry,
                )
                .is_some_and(|body| {
                    !body.value.contains('#')
                        || signatures.single_required_argument(&entry.name)
                            && data_body(&doc.text.source, &body, signatures)
                });
            definitions
                .entry(&entry.name)
                .or_default()
                .push((dropped, alias, entry.span.end));
        }
    }
    result.extend(
        definitions
            .iter()
            .filter(|(_, entries)| entries.len() == 1 && entries[0].0)
            .map(|(name, entries)| ((*name).to_owned(), entries[0].2)),
    );
    for _ in 0..32 {
        let before = result.len();
        for (name, entries) in &definitions {
            if entries.len() == 1
                && entries[0]
                    .1
                    .as_ref()
                    .is_some_and(|target| result.contains_key(target))
            {
                let after = entries[0].2.max(result[entries[0].1.as_ref().unwrap()]);
                result.insert((*name).to_owned(), after);
            }
        }
        if result.len() == before {
            break;
        }
    }
    result
}

/// Certify a complete direct data reader, never a substring under an unknown
/// wrapper which could remove the reader and execute its remaining argument.
fn data_body(source: &str, body: &syntax::Argument, signatures: &CommandSignatures<'_>) -> bool {
    let start = syntax::skip_space(source, body.span.start);
    let Some(command) = syntax::control_word(source, start) else {
        return false;
    };
    let name = &source[command.start + 1..command.end];
    if !matches!(name, "detokenize" | "typeout" | "message") || !signatures.command_unchanged(name)
    {
        return false;
    }
    let Some((argument, end)) =
        syntax::group(source, syntax::skip_space(source, command.end), b'{', b'}')
    else {
        return false;
    };
    let parameter = syntax::skip_space(source, argument.span.start);
    end <= body.span.end
        && syntax::skip_space(source, end) == body.span.end
        && source[parameter..].starts_with("#1")
        && syntax::skip_space(source, parameter + 2) == argument.span.end
}
