//! Static file provenance of explicit biblatex reference-section resources.
use crate::{
    named_references::Context,
    signature::{CommandSignatures, SourceCall},
    syntax::{self, Argument, Command},
    workspace::{resolve_file_path, Document, DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::Url;

// These internal readers let the common path machinery retain the native
// distinction between registered exact names and inferred .bib filenames.
pub(crate) const INFERRED: &str = "__latex_lsp_section_bib";
pub(crate) const EXACT: &str = "__latex_lsp_section_resource";
pub(crate) const DEPENDENCY: &str = "__latex_lsp_selected_registration";
pub(crate) fn file_command(name: &str) -> bool {
    matches!(name, INFERRED | EXACT)
}
pub(crate) fn resource_command(name: &str) -> bool {
    file_command(name) || name == DEPENDENCY
}

#[derive(Default)]
pub(crate) struct Selection {
    pub calls: BTreeMap<Url, Vec<SourceCall>>,
    pub labels: BTreeSet<String>,
}
pub(crate) fn editable_item(value: &str) -> bool {
    crate::bib_resources::literal_path(value)
        && !value.contains([',', ']'])
        && !value.find(".bib").is_some_and(|at| at + 4 != value.len())
}

#[derive(Clone)]
struct Registration {
    uri: Url,
    command: Command,
    target: Option<Url>,
}
#[derive(Default)]
struct Registry {
    paths: BTreeMap<String, Option<Registration>>,
    labels: BTreeMap<String, Option<Registration>>,
    uncertain: bool,
}
fn bind(
    map: &mut BTreeMap<String, Option<Registration>>,
    key: String,
    registration: Option<Registration>,
) {
    map.entry(key)
        .and_modify(|previous| {
            if !previous
                .as_ref()
                .zip(registration.as_ref())
                .is_some_and(|(a, b)| a.target.is_some() && a.target == b.target)
            {
                *previous = None;
            }
        })
        .or_insert(registration);
}
fn resolved<'a>(
    doc: &'a Document,
    index: usize,
    signatures: &'a CommandSignatures<'a>,
    context: &Context,
) -> Option<(&'a str, std::ops::Range<usize>)> {
    let command = &doc.syntax.commands[index];
    if context.operand(doc, command.span.start) {
        return None;
    }
    let (name, span) = signatures
        .named_invocation(
            &doc.text.source,
            &doc.syntax,
            index,
            &BTreeSet::new(),
            &context.declarations,
        )
        .unwrap_or((&command.name, command.span.clone()));
    let name = signatures.forwarded_builtin(name).unwrap_or(name);
    signatures.command_unchanged(name).then_some((name, span))
}
fn resource_label(source: &str, end: usize) -> Result<Option<String>, ()> {
    let at = syntax::skip_space(source, end);
    if source.as_bytes().get(at) != Some(&b'[') {
        return Ok(None);
    }
    let (argument, _) = syntax::bracket_argument(source, at).ok_or(())?;
    if argument.value.contains('%') {
        return Err(());
    }
    let mut label = None;
    for segment in syntax::key_list_segments(source, argument.span) {
        let option = source[segment].trim();
        let Some((key, value)) = option.split_once('=') else {
            continue;
        };
        if key.trim() != "label" {
            continue;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            .unwrap_or(value);
        if !syntax::is_static_key(value) {
            return Err(());
        }
        label = Some(value.into());
    }
    Ok(label)
}
fn lookup(root: &Url, uri: &Url, name: &str, value: &str) -> Option<Url> {
    resolve_file_path(root, name, value, &["bib"])
        .or_else(|| resolve_file_path(uri, name, value, &["bib"]))
}
impl Registry {
    fn new(root: &Url, docs: &[(Url, &Document)], signatures: &CommandSignatures<'_>) -> Self {
        let mut result = Self::default();
        // A copied registration can preserve an earlier resource meaning.
        // It cannot certify an unregistered filename or a unique label here.
        for (_, doc) in docs {
            if doc.syntax.definitions.iter().any(|entry| {
                crate::signature::literal_copy_target(&doc.text.source, &doc.syntax, entry, false)
                    .is_some_and(|(name, _)| {
                        crate::bib_resources::references_file(&name)
                            || signatures
                                .forwarded_builtin(&name)
                                .is_some_and(crate::bib_resources::references_file)
                    })
            }) {
                result.uncertain = true;
            }
        }
        for (uri, doc) in docs {
            let source = &doc.text.source;
            let positions = signatures.source_command_positions(source, &doc.syntax);
            let context = Context::new(doc, signatures);
            for (index, command) in doc.syntax.commands.iter().enumerate() {
                if !positions.contains(&command.span.start) {
                    continue;
                }
                let Some((name, span)) = resolved(doc, index, signatures, &context)
                    .filter(|(name, _)| crate::bib_resources::references_file(name))
                else {
                    continue;
                };
                let mut command = Command {
                    name: name.into(),
                    span,
                    argument: None,
                };
                let label = match resource_label(source, command.span.end) {
                    Ok(label) => label,
                    Err(()) => {
                        result.uncertain = true;
                        continue;
                    }
                };
                let Some(raw) = crate::bib_resources::read_argument(
                    source,
                    command.span.end,
                    signatures.resource_is_tokenized(source, &doc.syntax, command.span.start),
                ) else {
                    result.uncertain = true;
                    continue;
                };
                command.argument = Some(raw.clone());
                let local = crate::bib_resources::argument(source, &command);
                let registration = local.map(|argument| Registration {
                    uri: uri.clone(),
                    target: lookup(root, uri, EXACT, &argument.value),
                    command: Command {
                        argument: Some(argument),
                        ..command
                    },
                });
                if !crate::bib_resources::literal_path(&raw.value) {
                    result.uncertain = true;
                }
                bind(&mut result.paths, raw.value, registration.clone());
                if let Some(label) = label {
                    bind(&mut result.labels, label, registration);
                }
            }
        }
        result
    }
}

/// SourceCall arguments carry one item of the native comma-separated header.
/// Labels only load their registered file; their text remains a symbol and is
/// never replaced by a filename during a file move.
pub(crate) fn calls(
    workspace: &Workspace,
    root: &Url,
    scope: &BTreeSet<Url>,
    signatures: &CommandSignatures<'_>,
) -> Option<Selection> {
    let candidate = |doc: &Document| {
        doc.kind == DocumentKind::Latex
            && (doc.text.source.contains("refsection")
                || doc.syntax.commands.iter().any(|command| {
                    command.argument.as_ref().is_some_and(|argument| {
                        matches!(argument.value.as_str(), "refsection" | "newrefsection")
                    })
                }))
    };
    if !scope
        .iter()
        .filter_map(|uri| workspace.documents.get(uri))
        .any(candidate)
    {
        return Some(Selection::default());
    }
    let docs = crate::source_inputs::bounded_latex_sources(workspace, scope)?;
    let registry = Registry::new(root, &docs, signatures);
    if registry.uncertain {
        return Some(Selection::default());
    }
    let mut result = BTreeMap::<Url, Vec<SourceCall>>::new();
    let mut count = 0usize;
    for (uri, doc) in &docs {
        let source = &doc.text.source;
        let context = Context::new(doc, signatures);
        let positions = signatures.source_command_positions(source, &doc.syntax);
        for index in 0..doc.syntax.commands.len() {
            if !positions.contains(&doc.syntax.commands[index].span.start) {
                continue;
            }
            let Some((name, span)) = resolved(doc, index, signatures, &context) else {
                continue;
            };
            let end = if matches!(name, "newrefsection" | "refsection") {
                span.end
            } else if name == "begin" && signatures.command_unchanged("refsection") {
                let Some(environment) = syntax::environment_name_argument(source, span.end)
                    .filter(|argument| argument.value == "refsection")
                else {
                    continue;
                };
                environment.outer_span.end
            } else {
                continue;
            };
            let at = syntax::skip_space(source, end);
            if crate::resource_context::ambiguous_header(&doc.syntax, at) {
                continue;
            }
            if source.as_bytes().get(at) != Some(&b'[') {
                continue;
            }
            let raw = doc.syntax.raw_resource_brackets.contains(&at);
            let argument = if raw {
                syntax::verbatim_file_bracket(source, at)
            } else {
                syntax::bracket_argument(source, at)
            };
            let content = argument.map_or(at + 1..source.len(), |(argument, _)| argument.span);
            let segments = if raw {
                syntax::verbatim_list_segments(source, content)
            } else {
                syntax::key_list_segments(source, content)
            };
            for segment in segments {
                count += 1;
                if count > 8192 {
                    return None;
                }
                let value = &source[segment.clone()];
                let trimmed = value.trim();
                let start = segment.start + value.len() - value.trim_start().len();
                let item = if raw || trimmed.is_empty() {
                    start..start + trimmed.len()
                } else {
                    let Some(item) = syntax::literal_file_span(source, segment, false) else {
                        continue;
                    };
                    item
                };
                let trimmed = &source[item.clone()];
                if !trimmed.is_empty() && !editable_item(trimmed) {
                    continue;
                }
                if let Some(binding) = registry.labels.get(trimmed) {
                    if let Some(registration) = binding {
                        let mut command = registration.command.clone();
                        command.name = DEPENDENCY.into();
                        result
                            .entry(registration.uri.clone())
                            .or_default()
                            .push(SourceCall {
                                command,
                                certain: true,
                            });
                    }
                    continue;
                }
                let command_name = if let Some(binding) = registry.paths.get(trimmed) {
                    let Some(registration) = binding else {
                        continue;
                    };
                    if lookup(root, uri, EXACT, trimmed) != registration.target {
                        continue;
                    }
                    EXACT
                } else {
                    // biblatex's legacy stripbib stops at the first .bib.
                    // Only ordinary optional-suffix spellings are editable.
                    if trimmed
                        .find(".bib")
                        .is_some_and(|at| at + 4 != trimmed.len())
                    {
                        continue;
                    }
                    INFERRED
                };
                result.entry(uri.clone()).or_default().push(SourceCall {
                    command: Command {
                        name: command_name.into(),
                        span: span.clone(),
                        argument: Some(Argument {
                            value: trimmed.into(),
                            span: item.clone(),
                            outer_span: item,
                        }),
                    },
                    certain: true,
                });
            }
        }
    }
    for calls in result.values_mut() {
        calls.sort_by_key(|call| {
            (
                call.command.span.start,
                call.command
                    .argument
                    .as_ref()
                    .map(|argument| argument.span.start),
            )
        });
        calls.dedup_by(|a, b| {
            a.command.name == b.command.name
                && a.command.span == b.command.span
                && a.command.argument.as_ref().map(|argument| &argument.span)
                    == b.command.argument.as_ref().map(|argument| &argument.span)
        });
    }
    Some(Selection {
        calls: result,
        labels: registry.labels.into_keys().collect(),
    })
}
