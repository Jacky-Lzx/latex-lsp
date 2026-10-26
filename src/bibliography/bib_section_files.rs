//! Static file provenance of explicit biblatex reference-section resources.
use crate::{
    parser::syntax::{self, Argument, Command},
    project::workspace::{resolve_file_path, Document, DocumentKind, Workspace},
    symbols::commands::{CommandSignatures, SourceCall},
    symbols::named_references::Context,
};
use lsp_types::Url;
use std::collections::{BTreeMap, BTreeSet};

// These internal readers let the common path machinery retain the native
// distinction between registered exact names and inferred .bib filenames.
pub(crate) const INFERRED: &str = "__texquill_section_bib";
pub(crate) const EXACT: &str = "__texquill_section_resource";
pub(crate) const DEPENDENCY: &str = "__texquill_selected_registration";
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
pub(crate) fn literal_item(value: &str) -> bool {
    crate::bibliography::bib_resources::literal_path(value) && !value.contains([',', ']'])
}
pub(crate) fn editable_item(value: &str) -> bool {
    literal_item(value) && !value.find(".bib").is_some_and(|at| at + 4 != value.len())
}

#[derive(Clone)]
struct Registration {
    uri: Url,
    command: Command,
    target: Option<Url>,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct LabelBinding {
    pub uri: Url,
    pub span: std::ops::Range<usize>,
    pub filename: String,
    target: Option<Url>,
}
#[derive(Default)]
pub(crate) struct Registry {
    paths: BTreeMap<String, Option<Registration>>,
    labels: BTreeMap<String, Option<Registration>>,
    declarations: BTreeMap<String, Option<LabelBinding>>,
    uncertain: bool,
    defaults: Option<BTreeSet<Url>>,
    globals: Option<BTreeSet<Url>>,
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
pub(crate) fn resource_label(source: &str, end: usize) -> Result<Option<Argument>, ()> {
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
        let segment = syntax::trimmed_span(source, segment);
        let option = &source[segment.clone()];
        let Some(equal) = option.find('=') else {
            continue;
        };
        if option[..equal].trim() != "label" {
            continue;
        }
        let outer_span = syntax::trimmed_span(source, segment.start + equal + 1..segment.end);
        let mut span = outer_span.clone();
        if let Some((argument, end)) = syntax::group(source, span.start, b'{', b'}') {
            if end == span.end {
                span = argument.span;
            }
        }
        let value = &source[span.clone()];
        if !syntax::is_static_key(value) {
            return Err(());
        }
        label = Some(Argument {
            value: value.into(),
            span,
            outer_span,
        });
    }
    Ok(label)
}
fn lookup(root: &Url, uri: &Url, name: &str, value: &str) -> Option<Url> {
    resolve_file_path(root, name, value, &["bib"])
        .or_else(|| resolve_file_path(uri, name, value, &["bib"]))
}
// Biber resolves these names from the compilation root. The input-relative
// fallback remains useful for file links, but cannot certify a citation target.
fn root_target(root: &Url, registration: &Registration) -> Option<Url> {
    let name = &registration.command.argument.as_ref()?.value;
    let target = lookup(root, root, EXACT, name)?;
    (registration.target.as_ref() == Some(&target)).then_some(target)
}
impl Registry {
    pub(crate) fn new(
        root: &Url,
        docs: &[(Url, &Document)],
        signatures: &CommandSignatures<'_>,
    ) -> Self {
        let mut result = Self {
            defaults: Some(BTreeSet::new()),
            globals: Some(BTreeSet::new()),
            ..Default::default()
        };
        // A copied registration can preserve an earlier resource meaning.
        // It cannot certify an unregistered filename or a unique label here.
        for (_, doc) in docs {
            if doc.syntax.definitions.iter().any(|entry| {
                crate::symbols::commands::literal_copy_target(
                    &doc.text.source,
                    &doc.syntax,
                    entry,
                    false,
                )
                .is_some_and(|(name, _)| {
                    crate::bibliography::bib_resources::references_file(&name)
                        || signatures
                            .forwarded_builtin(&name)
                            .is_some_and(crate::bibliography::bib_resources::references_file)
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
                    .filter(|(name, _)| crate::bibliography::bib_resources::references_file(name))
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
                let Some(raw) = crate::bibliography::bib_resources::read_argument(
                    source,
                    command.span.end,
                    signatures.resource_is_tokenized(source, &doc.syntax, command.span.start),
                ) else {
                    result.uncertain = true;
                    continue;
                };
                command.argument = Some(raw.clone());
                let local = crate::bibliography::bib_resources::argument(source, &command);
                let registration = local.map(|argument| Registration {
                    uri: uri.clone(),
                    target: lookup(root, uri, EXACT, &argument.value),
                    command: Command {
                        argument: Some(argument),
                        ..command
                    },
                });
                if !crate::bibliography::bib_resources::literal_path(&raw.value) {
                    result.uncertain = true;
                }
                let resources = match name {
                    "addbibresource" => Some(&mut result.defaults),
                    "addglobalbib" => Some(&mut result.globals),
                    _ => None,
                };
                if let Some(resources) = resources {
                    if let Some(target) = registration
                        .as_ref()
                        .and_then(|entry| root_target(root, entry))
                    {
                        if let Some(resources) = resources.as_mut() {
                            resources.insert(target);
                        }
                    } else {
                        *resources = None;
                    }
                }
                bind(&mut result.paths, raw.value, registration.clone());
                if let Some(label) = label {
                    let binding = registration.as_ref().map(|entry| LabelBinding {
                        uri: uri.clone(),
                        span: label.span,
                        filename: entry.command.argument.as_ref().unwrap().value.clone(),
                        target: entry.target.clone(),
                    });
                    result
                        .declarations
                        .entry(label.value.clone())
                        .and_modify(|previous| {
                            if *previous != binding {
                                *previous = None;
                            }
                        })
                        .or_insert(binding);
                    bind(&mut result.labels, label.value, registration);
                }
            }
        }
        result
    }

    pub(crate) fn is_label(&self, name: &str) -> bool {
        self.labels.contains_key(name)
    }

    pub(crate) fn label_names(&self) -> BTreeSet<String> {
        self.labels.keys().cloned().collect()
    }

    pub(crate) fn label_bindings(&self) -> Option<BTreeMap<String, LabelBinding>> {
        (!self.uncertain).then(|| {
            self.declarations
                .iter()
                .filter_map(|(name, binding)| {
                    binding.clone().map(|binding| (name.clone(), binding))
                })
                .collect()
        })
    }

    /// Roles are collected independently of the path map: registering a path
    /// for a section first must not erase its later default or global role.
    pub(crate) fn resources(
        &self,
        root: &Url,
        selected: Option<&[String]>,
    ) -> Option<BTreeSet<Url>> {
        if self.uncertain {
            return None;
        }
        let mut result = self.globals.clone()?;
        if let Some(selected) = selected.filter(|items| !items.is_empty()) {
            for item in selected {
                let target = if let Some(binding) = self.labels.get(item) {
                    root_target(root, binding.as_ref()?)?
                } else if let Some(binding) = self.paths.get(item) {
                    root_target(root, binding.as_ref()?)?
                } else {
                    if !editable_item(item) {
                        return None;
                    }
                    lookup(root, root, INFERRED, item)?
                };
                result.insert(target);
            }
        } else {
            result.extend(self.defaults.clone()?);
        }
        Some(result)
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
    let docs = crate::project::source_inputs::bounded_latex_sources(workspace, scope)?;
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
            if crate::project::resource_context::ambiguous_header(&doc.syntax, at) {
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
                if !trimmed.is_empty() && !editable_item(trimmed) && !registry.is_label(trimmed) {
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
