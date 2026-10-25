//! Resource reader catcodes certified at their input positions.
use crate::{
    language::signature::{source_alias, CommandSignatures},
    parser::syntax::{self, Command, Syntax},
    project::workspace::{DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reader {
    pub start: usize,
    pub end: usize,
    pub bracket: bool,
    pub opaque: bool,
}
pub(crate) type Readers = BTreeMap<usize, Option<Reader>>;
fn opaque_reader(source: &str, mut reader: Reader) -> Reader {
    reader.opaque = true;
    let normal = if reader.bracket {
        syntax::bracket_argument(source, reader.start)
    } else {
        syntax::group(source, reader.start, b'{', b'}')
    };
    reader.end = reader.end.max(normal.map_or(source.len(), |(_, end)| end));
    reader
}
pub(crate) fn ambiguous_header(parsed: &Syntax, at: usize) -> bool {
    parsed
        .contextual_resource_readers
        .values()
        .flatten()
        .any(|reader| reader.opaque && reader.start == at)
}
pub(crate) fn validate_rename(
    ws: &Workspace,
    scope: &BTreeSet<Url>,
    name: &str,
) -> tower_lsp::jsonrpc::Result<()> {
    if scope
        .iter()
        .filter_map(|uri| ws.documents.get(uri))
        .any(|doc| {
            doc.syntax
                .contextual_resource_readers
                .values()
                .flatten()
                .any(|reader| {
                    reader.opaque && doc.text.source[reader.start..reader.end].contains(name)
                })
        })
    {
        return Err(tower_lsp::jsonrpc::Error::invalid_params("A resource argument has an unconfirmed reader meaning; a complete symbol rename cannot be confirmed"));
    }
    Ok(())
}

pub(crate) fn validate_workspace_edit(
    ws: &Workspace,
    edit: &tower_lsp::lsp_types::WorkspaceEdit,
) -> tower_lsp::jsonrpc::Result<()> {
    use tower_lsp::lsp_types::{DocumentChangeOperation, DocumentChanges, OneOf, TextEdit};
    if !ws.documents.values().any(|doc| {
        doc.syntax
            .contextual_resource_readers
            .values()
            .flatten()
            .any(|reader| reader.opaque)
    }) {
        return Ok(());
    }
    let mut scopes = BTreeMap::new();
    let mut checked = BTreeSet::new();
    let mut check = |uri: &Url, edit: &TextEdit| {
        if let Some(doc) = ws.documents.get(uri) {
            if let Some((start, end)) = doc
                .text
                .offset(edit.range.start)
                .zip(doc.text.offset(edit.range.end))
            {
                let name = doc.text.source[start..end].trim_start_matches('\\');
                if !name.is_empty() && checked.insert((uri.clone(), name.to_owned())) {
                    let scope = scopes
                        .entry(uri.clone())
                        .or_insert_with(|| ws.symbol_scope(uri));
                    validate_rename(ws, scope, name)?;
                }
            }
        }
        Ok(())
    };
    if let Some(changes) = &edit.changes {
        for (uri, edits) in changes {
            for edit in edits {
                check(uri, edit)?;
            }
        }
    }
    let documents: Vec<_> = match &edit.document_changes {
        Some(DocumentChanges::Edits(edits)) => edits.iter().collect(),
        Some(DocumentChanges::Operations(operations)) => operations
            .iter()
            .filter_map(|operation| match operation {
                DocumentChangeOperation::Edit(edit) => Some(edit),
                _ => None,
            })
            .collect(),
        None => vec![],
    };
    for document in documents {
        for edit in &document.edits {
            check(
                &document.text_document.uri,
                match edit {
                    OneOf::Left(edit) => edit,
                    OneOf::Right(edit) => &edit.text_edit,
                },
            )?;
        }
    }
    Ok(())
}

const BUILTINS: &[&str] = &[
    "newrefsection",
    "refsection",
    "begin",
    "addbibresource",
    "addglobalbib",
    "addsectionbib",
];

fn prefix(syntax: &Syntax, end: usize) -> Syntax {
    let mut result = syntax.clone();
    result.commands.retain(|command| command.span.start <= end);
    result.definitions.retain(|entry| entry.span.start < end);
    result.environments.retain(|entry| entry.span.start < end);
    result.command_undefinitions.retain(|at, _| *at < end);
    result
}
pub(crate) fn grouped(source: &str, parsed: &Syntax, end: usize) -> bool {
    let mut depth = 0usize;
    let mut at = 0;
    let mut ignored: Vec<_> = parsed.ignored.iter().collect();
    ignored.sort_by_key(|span| span.start);
    let mut next = 0;
    while at < end {
        while ignored.get(next).is_some_and(|span| span.end <= at) {
            next += 1;
        }
        if let Some(span) = ignored.get(next).filter(|span| span.start <= at) {
            at = span.end;
            continue;
        }
        match source.as_bytes()[at] {
            b'\\' => {
                at += 1;
                if at < end {
                    at += source[at..].chars().next().unwrap().len_utf8();
                }
            }
            b'{' => {
                depth += 1;
                at += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                at += 1;
            }
            _ => at += source[at..].chars().next().unwrap().len_utf8(),
        }
    }
    depth > 0
}
#[derive(Default)]
struct Analysis {
    readers: BTreeMap<Url, Readers>,
    prefixes: BTreeMap<Url, usize>,
    visited: BTreeSet<Url>,
    stack: BTreeSet<Url>,
    uncertain: bool,
    visits: usize,
    queries: usize,
    incomplete: bool,
    resource_names: BTreeSet<String>,
}
impl Analysis {
    fn walk(&mut self, ws: &Workspace, root: &Url, uri: &Url, names: &BTreeSet<String>) {
        if self.visits >= 1024 || self.stack.len() >= 16 || !self.stack.insert(uri.clone()) {
            self.uncertain = true;
            self.incomplete = true;
            return;
        }
        let Some(doc) = ws
            .documents
            .get(uri)
            .filter(|doc| doc.kind == DocumentKind::Latex)
        else {
            self.stack.remove(uri);
            return;
        };
        self.visits += 1;
        self.visited.insert(uri.clone());
        let source = &doc.text.source;
        let scoped: BTreeSet<_> = doc
            .syntax
            .definitions
            .iter()
            .filter_map(|entry| {
                let name = entry.detail.strip_prefix("Defined with \\")?;
                let before = doc
                    .syntax
                    .commands
                    .partition_point(|command| command.span.start < entry.span.start);
                let declaration = doc.syntax.commands[..before]
                    .iter()
                    .rev()
                    .find(|command| command.name == name)?;
                grouped(source, &doc.syntax, declaration.span.start)
                    .then_some(declaration.span.start)
            })
            .collect();
        // Snapshots retain declarations only up to the current load/call.
        for (index, lexical) in doc.syntax.commands.iter().enumerate() {
            // The opening csname schedules its decision at the closing token.
            if lexical.name == "endcsname" {
                continue;
            }
            let scoped_assignment = scoped.contains(&lexical.span.start);
            let candidate = names.contains(&lexical.name)
                || doc
                    .syntax
                    .contextual_resource_readers
                    .contains_key(&lexical.span.start)
                || matches!(lexical.name.as_str(), "csuse" | "csname" | "endcsname");
            let loader = crate::project::source_inputs::source_file_command(&lexical.name)
                || syntax::loader_extension(&lexical.name).is_some()
                || matches!(
                    lexical.name.as_str(),
                    "import" | "subimport" | "IfFileExists" | "includeonly"
                );
            let conditional = matches!(
                lexical.name.as_str(),
                "if" | "ifx"
                    | "ifcat"
                    | "ifdefined"
                    | "ifcsname"
                    | "ifnum"
                    | "ifdim"
                    | "ifodd"
                    | "ifcase"
                    | "unless"
                    | "begingroup"
                    | "endgroup"
                    | "bgroup"
                    | "egroup"
            );
            if !candidate && !loader && !conditional && !scoped_assignment {
                continue;
            }
            self.queries += 1;
            if self.queries > 4096 {
                self.uncertain = true;
                self.incomplete = true;
                break;
            }
            self.prefixes.insert(uri.clone(), lexical.span.end);
            let snapshots: Vec<_> = self
                .prefixes
                .iter()
                .filter_map(|(uri, end)| {
                    ws.documents
                        .get(uri)
                        .map(|doc| (&doc.text.source, prefix(&doc.syntax, *end)))
                })
                .collect();
            let signatures = CommandSignatures::source_context(
                snapshots
                    .iter()
                    .map(|(text, syntax)| (text.as_str(), syntax)),
            );
            let positions = signatures.source_command_positions(source, &doc.syntax);
            let declarations = doc
                .syntax
                .definitions
                .iter()
                .map(|entry| (entry.span.start, entry.span.end))
                .collect();
            let (name, span) = signatures
                .named_invocation(source, &doc.syntax, index, &BTreeSet::new(), &declarations)
                .unwrap_or((&lexical.name, lexical.span.clone()));
            let builtin = signatures.forwarded_builtin(name).unwrap_or(name);
            let native = !self.uncertain
                && positions.contains(&lexical.span.start)
                && signatures.command_unchanged(builtin)
                && !signatures.resource_is_tokenized(source, &doc.syntax, lexical.span.start)
                && !doc.syntax.control_operands.contains(&lexical.span.start);
            let trigger = if lexical.name == "csname" {
                doc.syntax
                    .commands
                    .iter()
                    .find(|command| command.name == "endcsname" && command.span.end == span.end)
                    .map_or(lexical.span.start, |command| command.span.start)
            } else {
                lexical.span.start
            };
            let prior = doc
                .syntax
                .contextual_resource_readers
                .contains_key(&trigger);
            let resource_candidate = (self.resource_names.contains(name)
                || BUILTINS.contains(&builtin))
                && (builtin != "begin"
                    || syntax::environment_name_argument(source, span.end)
                        .is_some_and(|environment| environment.value == "refsection"));
            if candidate && (resource_candidate || prior) {
                let reader = (|| {
                    if !BUILTINS.contains(&builtin) {
                        return None;
                    }
                    let opaque = self.uncertain || !signatures.command_unchanged(builtin);
                    if !native && !opaque {
                        return None;
                    }
                    let (at, bracket) =
                        if crate::bibliography::bib_resources::references_file(builtin) {
                            (
                                crate::bibliography::bib_resources::start(source, span.end)?,
                                false,
                            )
                        } else {
                            let end = if builtin == "begin" {
                                let env = syntax::environment_name_argument(source, span.end)?;
                                if env.value != "refsection"
                                    || !signatures.command_unchanged("refsection")
                                {
                                    return None;
                                }
                                env.outer_span.end
                            } else {
                                span.end
                            };
                            let at = syntax::skip_space(source, end);
                            if source.as_bytes().get(at) != Some(&b'[') {
                                return None;
                            }
                            (at, true)
                        };
                    let end = if bracket {
                        syntax::verbatim_file_bracket(source, at)
                    } else {
                        syntax::verbatim_file_group(source, at)
                    }
                    .map_or(source.len(), |(_, end)| end);
                    Some(Reader {
                        start: at,
                        end,
                        bracket,
                        opaque,
                    })
                    .map(|reader| {
                        if reader.opaque {
                            opaque_reader(source, reader)
                        } else {
                            reader
                        }
                    })
                })();
                self.readers
                    .entry(uri.clone())
                    .or_default()
                    .entry(trigger)
                    .and_modify(|previous| {
                        if *previous != reader {
                            let possible = previous.as_ref().or(reader.as_ref()).cloned();
                            if let Some(mut common) = possible {
                                if let Some(other) = reader.as_ref() {
                                    common.start = common.start.min(other.start);
                                    common.end = common.end.max(other.end);
                                }
                                common.bracket = source.as_bytes().get(common.start) == Some(&b'[');
                                *previous = Some(opaque_reader(source, common));
                            }
                        }
                    })
                    .or_insert(reader);
            }
            if !positions.contains(&lexical.span.start) {
                continue;
            }
            if conditional
                || scoped_assignment
                || matches!(
                    builtin,
                    "InputIfFileExists" | "IfFileExists" | "includeonly"
                )
            {
                self.uncertain = true;
                continue;
            }
            if !signatures.command_unchanged(builtin) {
                if loader {
                    self.uncertain = true;
                } else {
                    continue;
                }
            }
            let command = if builtin == lexical.name && span == lexical.span {
                lexical.clone()
            } else {
                let argument = if syntax::loader_extension(builtin).is_some() {
                    syntax::file_name_argument_start(source, builtin, span.end)
                        .and_then(|at| syntax::group(source, at, b'{', b'}'))
                        .map(|(argument, _)| argument)
                } else {
                    syntax::source_input_argument(source, builtin, span.end)
                };
                Command {
                    name: builtin.into(),
                    span,
                    argument,
                }
            };
            if crate::project::source_inputs::source_file_command(&command.name)
                || syntax::loader_extension(&command.name).is_some()
            {
                let static_name = if syntax::file_list(&command.name) {
                    syntax::complete_file_list_spans(source, &command).is_some()
                } else {
                    syntax::file_argument(source, &command)
                        .is_some_and(|argument| syntax::is_literal_file_path(&argument.value))
                };
                if !static_name {
                    self.uncertain = true;
                }
            }
            let dependencies = ws.literal_command_dependencies(uri, root, &command);
            for (dependency, kind) in dependencies {
                if kind == DocumentKind::Latex {
                    self.walk(ws, root, &dependency, names);
                }
            }
        }
        self.prefixes.insert(uri.clone(), source.len());
        self.stack.remove(uri);
    }
}

fn names(ws: &Workspace, loaders: bool) -> BTreeSet<String> {
    let mut names: BTreeSet<_> = BUILTINS.iter().map(|name| (*name).to_owned()).collect();
    if loaders {
        names.extend(
            [
                "input",
                "include",
                "subfile",
                "usepackage",
                "RequirePackage",
                "RequirePackageWithOptions",
                "documentclass",
                "LoadClass",
                "LoadClassWithOptions",
            ]
            .into_iter()
            .map(str::to_owned),
        );
    }
    for _ in 0..32 {
        let before = names.len();
        for doc in ws
            .documents
            .values()
            .filter(|doc| doc.kind == DocumentKind::Latex)
        {
            for entry in &doc.syntax.definitions {
                if source_alias(&doc.text.source, &doc.syntax, entry)
                    .is_some_and(|name| names.contains(&name))
                {
                    names.insert(entry.name.clone());
                }
            }
        }
        if names.len() == before {
            break;
        }
    }
    names
}
fn analyze_root(analysis: &mut Analysis, ws: &Workspace, root: &Url, names: &BTreeSet<String>) {
    let scope = ws.project_documents(root);
    let bounded =
        crate::project::source_inputs::bounded_latex_sources(ws, &scope).is_some_and(|docs| {
            docs.iter()
                .map(|(_, doc)| doc.syntax.commands.len())
                .sum::<usize>()
                <= 100_000
                && docs
                    .iter()
                    .map(|(_, doc)| doc.syntax.definitions.len() + doc.syntax.environments.len())
                    .sum::<usize>()
                    <= 8192
        });
    analysis.prefixes.clear();
    analysis.uncertain = false;
    analysis.incomplete = false;
    analysis.visits = 0;
    analysis.queries = 0;
    if bounded {
        analysis.walk(ws, root, root, names);
    }
    if !bounded || analysis.incomplete {
        for uri in &scope {
            analysis.visited.insert(uri.clone());
            if let Some(doc) = ws.documents.get(uri).filter(|doc| {
                doc.kind == DocumentKind::Latex
                    && crate::bibliography::bib_resources::changes_when_tokenized(&doc.text.source)
            }) {
                // Ordinary tokenization is the common safe prefix whenever a
                // resource reader cannot be certified within this root's budget.
                let readers = doc
                    .syntax
                    .commands
                    .iter()
                    .map(|command| {
                        let possible = doc
                            .syntax
                            .contextual_resource_readers
                            .get(&command.span.start)
                            .cloned()
                            .flatten()
                            .or_else(|| {
                                let at = if crate::bibliography::bib_resources::references_file(
                                    &command.name,
                                ) {
                                    crate::bibliography::bib_resources::start(
                                        &doc.text.source,
                                        command.span.end,
                                    )?
                                } else {
                                    syntax::skip_space(&doc.text.source, command.span.end)
                                };
                                let bracket = doc.text.source.as_bytes().get(at) == Some(&b'[');
                                if !bracket && doc.text.source.as_bytes().get(at) != Some(&b'{') {
                                    return None;
                                }
                                Some(Reader {
                                    start: at,
                                    end: at,
                                    bracket,
                                    opaque: true,
                                })
                            });
                        (
                            command.span.start,
                            possible.map(|reader| opaque_reader(&doc.text.source, reader)),
                        )
                    })
                    .collect();
                analysis.readers.insert(uri.clone(), readers);
            }
        }
    }
}

/// Return changed documents so the workspace can discover newly exposed inputs.
pub(crate) fn refresh(ws: &mut Workspace) -> BTreeSet<Url> {
    if !ws.documents.values().any(|doc| {
        doc.kind == DocumentKind::Latex
            && (!doc.syntax.contextual_resource_readers.is_empty()
                || doc.text.source.contains("refsection")
                || doc.text.source.contains("addbibresource")
                || doc.text.source.contains("addglobalbib")
                || doc.text.source.contains("addsectionbib"))
    }) {
        return BTreeSet::new();
    }
    let resource_names = names(ws, false);
    let needed = ws.documents.values().any(|doc| {
        doc.kind == DocumentKind::Latex
            && crate::bibliography::bib_resources::changes_when_tokenized(&doc.text.source)
            && (doc.syntax.commands.iter().any(|command| {
                (resource_names.contains(&command.name)
                    && (command.name != "begin"
                        || syntax::environment_name_argument(&doc.text.source, command.span.end)
                            .is_some_and(|environment| environment.value == "refsection")))
                    || matches!(command.name.as_str(), "csuse" | "csname")
            }) || !doc.syntax.contextual_resource_readers.is_empty())
    });
    if !needed {
        return BTreeSet::new();
    }
    let names = names(ws, true);
    let mut analysis = Analysis {
        resource_names,
        ..Default::default()
    };
    let roots: Vec<_> = ws
        .documents
        .keys()
        .filter(|uri| ws.is_saved_root(uri))
        .cloned()
        .collect();
    for root in roots {
        analyze_root(&mut analysis, ws, &root, &names);
    }
    let uncovered: Vec<_> = ws
        .documents
        .iter()
        .filter(|(uri, doc)| doc.kind == DocumentKind::Latex && !analysis.visited.contains(*uri))
        .map(|(uri, _)| uri.clone())
        .collect();
    for uri in uncovered {
        if !analysis.visited.contains(&uri) {
            analyze_root(&mut analysis, ws, &uri, &names);
        }
    }
    let mut changed = BTreeSet::new();
    for (uri, mut readers) in analysis.readers {
        let Some(doc) = ws.documents.get_mut(&uri) else {
            continue;
        };
        if !crate::bibliography::bib_resources::changes_when_tokenized(&doc.text.source) {
            readers.clear();
        }
        if doc.syntax.contextual_resource_readers == readers {
            continue;
        }
        let old = &doc.syntax;
        let mut syntax = Syntax::latex_with_resource_context(
            &doc.text,
            |command, known| {
                old.string_test_arities
                    .get(&command.span.start)
                    .copied()
                    .unwrap_or_else(|| {
                        crate::symbols::string_tests::data_count(&command.name).filter(|_| known)
                    })
            },
            &readers,
        );
        syntax.contextual_command_shapes = old.contextual_command_shapes.clone();
        syntax.contextual_string_tests = old.contextual_string_tests;
        syntax.contextual_string_operands = old.contextual_string_operands;
        syntax.ambiguous_string_tests = old.ambiguous_string_tests.clone();
        if old.contextual_string_operands {
            syntax
                .control_operands
                .extend(old.control_operands.iter().copied());
        }
        doc.syntax = syntax;
        doc.semantic_full = Default::default();
        doc.extra_symbols = Default::default();
        changed.insert(uri);
    }
    changed
}
