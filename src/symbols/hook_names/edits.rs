//! Editing requires complete possible-call coverage, beyond read-only consensus.
use super::*;
use crate::{
    language::signature,
    project::source_inputs,
    rpc::{Error, Result},
};

const IDENTITY_CHANGES: &[&str] = &[
    "def",
    "gdef",
    "edef",
    "xdef",
    "let",
    "futurelet",
    "csdef",
    "csgdef",
    "csedef",
    "csxdef",
    "cslet",
    "csletcs",
    "letcs",
    "undef",
    "gundef",
    "csundef",
    "csgundef",
    "newcommand",
    "renewcommand",
    "providecommand",
    "newrobustcmd",
    "renewrobustcmd",
    "providerobustcmd",
    "DeclareRobustCommand",
    "NewDocumentCommand",
    "RenewDocumentCommand",
    "ProvideDocumentCommand",
    "DeclareDocumentCommand",
    "NewExpandableDocumentCommand",
    "RenewExpandableDocumentCommand",
    "ProvideExpandableDocumentCommand",
    "DeclareExpandableDocumentCommand",
    "NewCommandCopy",
    "RenewCommandCopy",
    "DeclareCommandCopy",
    "newenvironment",
    "renewenvironment",
    "provideenvironment",
    "NewDocumentEnvironment",
    "RenewDocumentEnvironment",
    "ProvideDocumentEnvironment",
    "DeclareDocumentEnvironment",
    "NewEnvironmentCopy",
    "RenewEnvironmentCopy",
    "DeclareEnvironmentCopy",
];
const EXPANSIONS: &[&str] = &[
    // Deprecated public spellings remain executable in current kernels.
    // They are not part of the read-only name index's certified interfaces.
    "DisableHook",
    "ProvideHook",
    "ProvideReversedHook",
    "ProvideMirroredHookPair",
    "scantokens",
    "UseName",
    "ExpandArgs",
    "expanded",
    "@input",
    "@input@",
    "@@input",
    "read",
    "readline",
    "lowercase",
    "uppercase",
    "MakeLowercase",
    "MakeUppercase",
    "MakeTitlecase",
    "catcode",
    "lccode",
    "uccode",
];
// The parser omits their arguments even when a project changes their meaning.
// Keep their identities in the mutation graph before trusting literal spans.
const LITERAL_READERS: &[&str] = &[
    "detokenize",
    "string",
    "meaning",
    "verb",
    "url",
    "href",
    "hyperref",
    "begin",
    "end",
    "lstinline",
    "inputminted",
    "mintinline",
    "mint",
    "Verb",
    "EscVerb",
    "SaveVerb",
    "SaveGVerb",
    "UseVerb",
    "UseVerbatim",
    "LUseVerbatim",
    "BUseVerbatim",
    "UseMVerb",
    "pUseMVerb",
    "verbatim",
    "verbatim*",
    "lstlisting",
    "minted",
    "comment",
    "VerbatimWrite",
    "VerbatimBuffer",
    "VerbEnv",
    "Verbatim",
    "Verbatim*",
    "LVerbatim",
    "LVerbatim*",
    "BVerbatim",
    "BVerbatim*",
    "SaveVerbatim",
    "SaveVerbatim*",
    "VerbatimOut",
    "VerbatimOut*",
];

fn error() -> Error {
    Error::invalid_params(
        "Hook rename requires complete, unambiguous literal names and confirmed interface meanings",
    )
}

fn reserved(name: &str) -> bool {
    matches!(
        name.split('/').next().unwrap_or_default(),
        "cmd"
            | "env"
            | "file"
            | "package"
            | "class"
            | "include"
            | "para"
            | "shipout"
            | "build"
            | "begindocument"
            | "enddocument"
            | "label"
            | "selectfont"
            | "rmfamily"
            | "sffamily"
            | "ttfamily"
            | "normalfont"
            | "expand@font@defaults"
            | "bfseries"
            | "mdseries"
            | "insertmark"
    )
}

fn literal(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 512
        && !name.contains(['\\', '{', '}', '%', '#', '$', '&', '^', '~'])
        && name.trim() != "."
        && !name.trim().starts_with("./")
        && !name
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\t' | '\r' | '\n'))
}

fn internal_hook(name: &str) -> bool {
    name.starts_with("hook_")
        || name.contains("__hook")
        || name.starts_with('@') && name.contains("hook")
}

fn alias_bodies(doc: &Document, signatures: &CommandSignatures<'_>) -> BTreeSet<usize> {
    let source = &doc.text.source;
    let mut bodies = signatures.builtin_alias_body_positions(source, &doc.syntax, COMMANDS);
    for entry in &doc.syntax.environments {
        let Some((begin, end)) = signature::environment_bodies(source, &doc.syntax, entry) else {
            continue;
        };
        for (name, body) in [
            (entry.name.clone(), begin),
            (format!("end{}", entry.name), end),
        ] {
            if COMMANDS
                .iter()
                .any(|builtin| signatures.invokes_builtin(&name, builtin))
            {
                bodies.insert(syntax::skip_space(source, body.span.start));
            }
        }
    }
    bodies
}

fn validate_calls(
    index: &Index,
    uri: &Url,
    doc: &Document,
    signatures: &CommandSignatures<'_>,
    potential: &signature::StoredCodeAliases,
    shifted: &BTreeSet<String>,
    names: &[&str],
) -> Result<()> {
    let source = &doc.text.source;
    let context = crate::symbols::named_references::Context::new(doc, signatures);
    if doc
        .syntax
        .definitions
        .iter()
        .any(|entry| internal_hook(&entry.name))
    {
        return Err(error());
    }
    if crate::symbols::expl3::occurrences(doc)
        .entries
        .iter()
        .any(|entry| {
            // Unmodelled expl3 expansion and copying can reach public hooks
            // without a lexical hook token at the actual invocation.
            (entry.name.contains(['_', ':']))
                && !context.literal(entry.span.start.saturating_sub(1))
        })
    {
        return Err(error());
    }
    let bodies = alias_bodies(doc, signatures);
    if doc.syntax.command_undefinitions.values().any(|target| {
        target.as_ref().is_none_or(|target| {
            potential.contains_key(&target.name) || internal_hook(&target.name)
        })
    }) {
        return Err(error());
    }
    let mutations = signatures
        .macro_mutations(source, &doc.syntax, potential)
        .ok_or_else(error)?;
    if mutations.values().flatten().any(|target| {
        target
            .as_ref()
            .is_none_or(|target| potential.contains_key(target) || internal_hook(target))
    }) {
        return Err(error());
    }
    let targets: BTreeSet<_> = doc
        .syntax
        .definitions
        .iter()
        .chain(&doc.syntax.environments)
        .flat_map(|entry| [entry.span.start, entry.span.start.saturating_sub(1)])
        .collect();
    let empty = BTreeSet::new();
    for (at, command) in doc.syntax.commands.iter().enumerate() {
        if context.literal(command.span.start)
            || targets.contains(&command.span.start)
            || doc.syntax.control_operands.contains(&command.span.start)
            || bodies.contains(&command.span.start)
        {
            continue;
        }
        let named =
            signatures.named_invocation(source, &doc.syntax, at, &empty, &context.declarations);
        if matches!(command.name.as_str(), "csuse" | "csname") && named.is_none() {
            return Err(error());
        }
        let (name, invocation) = named.unwrap_or((&command.name, command.span.clone()));
        // Lower-level hooks and direct storage access are not public name
        // references. Omitting them would falsely certify a partial edit set.
        if internal_hook(name) || name.contains(':') {
            return Err(error());
        }
        if EXPANSIONS.contains(&name) {
            return Err(error());
        }
        if potential.get(name).into_iter().flatten().any(|builtin| {
            (IDENTITY_CHANGES.contains(builtin) || EXPANSIONS.contains(builtin))
                && (name != *builtin || command.name != *builtin)
        }) {
            return Err(error());
        }
        if crate::symbols::named_commands::declares(name) {
            let probe = syntax::Command {
                name: name.into(),
                span: invocation.clone(),
                argument: None,
            };
            let span =
                crate::symbols::named_commands::declaration(source, &probe).ok_or_else(error)?;
            if internal_hook(&source[span.clone()])
                || potential.contains_key(&source[span.clone()])
                    && !doc
                        .syntax
                        .definitions
                        .iter()
                        .any(|entry| entry.span == span)
            {
                return Err(error());
            }
        }
        if syntax::declares_traditional_command(name)
            || syntax::declares_document_command(name)
            || syntax::declares_command_copy(name)
            || syntax::declares_environment(name)
            || syntax::declares_environment_copy(name)
        {
            let mut after = syntax::skip_space(source, invocation.end);
            if source.as_bytes().get(after) == Some(&b'*') {
                after = syntax::skip_space(source, after + 1);
            }
            let (span, _, complete) = field(source, after).ok_or_else(error)?;
            if !complete
                || !doc
                    .syntax
                    .definitions
                    .iter()
                    .chain(&doc.syntax.environments)
                    .any(|entry| {
                        entry.span == span
                            && entry.detail.strip_prefix("Defined with \\") == Some(name)
                    })
            {
                return Err(error());
            }
        }
        if matches!(name, "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet") {
            let start = syntax::skip_space(source, invocation.end);
            let word = syntax::control_word(source, start).ok_or_else(error)?;
            if &source[word.clone()] == r"\csname"
                || name == "futurelet" && potential.contains_key(&source[word.start + 1..word.end])
            {
                return Err(error());
            }
        }
        if (matches!(name, "input" | "include" | "subfile" | "InputIfFileExists")
            || syntax::loader_extension(name).is_some())
            && syntax::file_argument(source, command)
                .is_none_or(|argument| argument.value.contains(['\\', '#', '%']))
        {
            return Err(error());
        }
        let known = if COMMANDS.contains(&name) && signatures.command_unchanged(name) {
            Some(name)
        } else {
            signatures
                .forwarded_builtin(name)
                .filter(|builtin| COMMANDS.contains(builtin))
        };
        if shifted.contains(name) && potential.contains_key(name) {
            return Err(error());
        }
        for &builtin in potential
            .get(name)
            .into_iter()
            .flatten()
            .filter(|name| COMMANDS.contains(name))
        {
            let mut after = invocation.end;
            for _ in 0..if builtin.starts_with("NewMirroredHookPair") {
                2
            } else {
                1
            } {
                let (entry, end) = name_field(source, after, builtin).ok_or_else(error)?;
                after = end;
                if !entry.complete || !literal(&entry.name) {
                    return Err(error());
                }
                if names.contains(&entry.name.as_str())
                    && (known != Some(builtin)
                        || context.operand(doc, command.span.start)
                        || index
                            .entries
                            .get(uri)
                            .and_then(|entries| entries.get(&entry.span.start))
                            != Some(&entry))
                {
                    return Err(error());
                }
            }
            if builtin.ends_with("WithArguments")
                && (builtin.starts_with("New") || builtin.starts_with("Use"))
            {
                let (span, _, complete) = field(source, after).ok_or_else(error)?;
                if !complete
                    || source[span]
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .is_none_or(|n| n > 9)
                {
                    return Err(error());
                }
            }
        }
    }
    Ok(())
}

fn scope(
    index: &Index,
    ws: &Workspace,
    uri: &Url,
    entry: &Entry,
    target: &str,
) -> Result<BTreeSet<Url>> {
    if !ws.index_truncated.is_empty() || reserved(&entry.name) {
        return Err(error());
    }
    let crate::project::workspace::SymbolProjects { mut scope, regions } = ws.symbol_projects(uri);
    let own = index.definitions(ws, uri, &entry.name);
    if own.len() != 1 {
        return Err(error());
    }
    loop {
        let before = scope.len();
        let shared: BTreeSet<_> = index
            .entries(&scope)
            .filter(|(_, other)| other.name == entry.name)
            .map(|(uri, _)| uri.clone())
            .collect();
        for project in regions
            .values()
            .filter(|project| !project.is_disjoint(&shared))
        {
            let declarations: Vec<_> = index
                .entries(project)
                .filter(|(_, other)| other.declaration && other.name == entry.name)
                .map(|(uri, entry)| Index::location(ws, uri, entry))
                .collect();
            if declarations != own {
                return Err(error());
            }
            scope.extend(project.iter().cloned());
        }
        if scope.len() == before {
            break;
        }
    }
    if scope
        .iter()
        .filter_map(|uri| ws.documents.get(uri))
        .any(crate::bibliography::bib_section_keys::database_execution)
    {
        return Err(error());
    }
    crate::project::resource_context::validate_rename(ws, &scope, &entry.name)?;
    let roots: BTreeSet<_> = scope.iter().flat_map(|uri| ws.source_roots(uri)).collect();
    for root in roots {
        let inputs = source_inputs::Index::new(ws, &root);
        if inputs.incomplete
            || inputs.calls.iter().any(|(uri, calls)| {
                calls
                    .iter()
                    .any(|call| !call.certain || source_inputs::target(uri, &root, call).is_none())
            })
        {
            return Err(error());
        }
    }
    let mut work = 0usize;
    for (project, consumers) in crate::symbols::environment_calls::project_contexts(ws) {
        if project.is_disjoint(&scope) {
            continue;
        }
        let docs: Vec<_> = project
            .iter()
            .filter_map(|uri| ws.documents.get(uri))
            .filter(|doc| doc.kind == DocumentKind::Latex)
            .collect();
        work = work.saturating_add(
            docs.iter()
                .map(|doc| {
                    doc.text.source.len()
                        + doc.syntax.commands.len() * 64
                        + (doc.syntax.definitions.len() + doc.syntax.environments.len()) * 64
                })
                .sum::<usize>(),
        );
        if work > 32 * 1024 * 1024 {
            return Err(error());
        }
        let sources = || {
            docs.iter()
                .map(|doc| (doc.text.source.as_str(), &doc.syntax))
        };
        let signatures = CommandSignatures::new(sources());
        if LITERAL_READERS
            .iter()
            .any(|reader| !signatures.command_unchanged(reader))
            || docs.iter().any(|doc| {
                doc.syntax
                    .environments
                    .iter()
                    .any(|entry| syntax::is_verbatim_environment(&entry.name))
            })
        {
            return Err(error());
        }
        let possible = signature::possible_builtin_aliases(
            sources(),
            COMMANDS
                .iter()
                .chain(crate::symbols::macro_mutations::COMMANDS)
                .chain(IDENTITY_CHANGES)
                .chain(EXPANSIONS)
                .copied()
                .chain(LITERAL_READERS.iter().copied()),
            true,
        )
        .ok_or_else(error)?;
        let shifted = signature::argument_consuming_forwarders(sources()).ok_or_else(error)?;
        for uri in consumers.intersection(&scope) {
            if let Some(doc) = ws
                .documents
                .get(uri)
                .filter(|doc| doc.kind == DocumentKind::Latex)
            {
                validate_calls(
                    index,
                    uri,
                    doc,
                    &signatures,
                    &possible,
                    &shifted,
                    &[&entry.name, target],
                )?;
            }
        }
    }
    if target != entry.name && index.entries(&scope).any(|(_, other)| other.name == target) {
        return Err(Error::invalid_params(
            "The target hook name already exists or is referenced",
        ));
    }
    Ok(scope)
}

pub(crate) fn prepare(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
) -> Option<Result<Option<PrepareRenameResponse>>> {
    let index = Index::new(ws, uri, offset, true);
    let entry = index.target(uri, offset, true)?;
    if index.definitions(ws, uri, &entry.name).is_empty() {
        return Some(Ok(None));
    }
    Some(scope(&index, ws, uri, entry, &entry.name).map(|_| {
        Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: Index::location(ws, uri, entry).range,
            placeholder: entry.name.clone(),
        })
    }))
}

pub(crate) fn rename(
    ws: &Workspace,
    uri: &Url,
    offset: usize,
    name: &str,
) -> Option<Result<Option<WorkspaceEdit>>> {
    let index = Index::new(ws, uri, offset, true);
    let entry = index.target(uri, offset, true)?;
    if index.definitions(ws, uri, &entry.name).is_empty() {
        return Some(Ok(None));
    }
    Some((|| {
        if !valid(name) || !literal(name) || reserved(name) {
            return Err(Error::invalid_params(
                "Invalid hook name or reserved kernel/generic hook namespace",
            ));
        }
        let scope = scope(&index, ws, uri, entry, name)?;
        if name == entry.name {
            return Ok(Some(WorkspaceEdit::default()));
        }
        let mut changes = BTreeMap::<Url, Vec<TextEdit>>::new();
        for (uri, other) in index
            .entries(&scope)
            .filter(|(_, other)| other.name == entry.name)
        {
            changes.entry(uri.clone()).or_default().push(TextEdit {
                range: Index::location(ws, uri, other).range,
                new_text: if other.grouped {
                    name.into()
                } else {
                    format!("{{{name}}}")
                },
            });
        }
        Ok(Some(crate::language::features::workspace_edit(ws, changes)))
    })())
}
