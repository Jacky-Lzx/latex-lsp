//! Static file inputs displayed as code, with independent package guards.
use crate::{
    parser::{listings, syntax},
    project::workspace::{resolve_file_path, Workspace},
    text::Text,
};
use lsp_types::{TextEdit, Url};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

const COMMANDS: [&str; 6] = [
    "lstinputlisting",
    "verbatiminput",
    "VerbatimInput",
    "LVerbatimInput",
    "BVerbatimInput",
    "inputminted",
];

/// Code readers with filename wrappers beyond a plain literal group.
pub(crate) fn wrapped_filename(name: &str) -> bool {
    crate::parser::fancyvrb::is_input(name) || name == "inputminted"
}

pub(crate) fn filename(
    source: &str,
    name: &str,
    span: Range<usize>,
    partial: bool,
) -> Option<Range<usize>> {
    if name == "inputminted" {
        crate::parser::minted::filename(source, span, partial)
    } else {
        crate::parser::fancyvrb::filename(source, span, partial)
    }
}

pub(crate) fn prefix(
    source: &str,
    command: &syntax::Command,
    cursor: usize,
) -> Option<Range<usize>> {
    if command.name == "inputminted" {
        crate::parser::minted::prefix(source, command, cursor)
    } else {
        crate::parser::fancyvrb::prefix(source, command, cursor)
    }
}

pub(crate) fn filename_edit(
    text: &Text,
    command: &syntax::Command,
    value: String,
) -> Option<TextEdit> {
    if command.name == "inputminted" {
        crate::parser::minted::filename_edit(text, command, value)
    } else {
        crate::parser::fancyvrb::filename_edit(text, command, value)
    }
}

pub(crate) fn completion_edit(
    text: &Text,
    command: &syntax::Command,
    value: String,
    end: usize,
) -> Option<TextEdit> {
    if command.name == "inputminted" {
        crate::parser::minted::completion_edit(text, command, value, end)
    } else {
        crate::parser::fancyvrb::completion_edit(text, command, value, end)
    }
}

pub(crate) fn safe_syntax(source: &str, parsed: &syntax::Syntax, name: &str) -> bool {
    match name {
        name if crate::parser::fancyvrb::is_input(name) => {
            crate::parser::fancyvrb::safe_syntax(source, parsed)
        }
        "lstinputlisting" => listings::safe_syntax(source, parsed),
        "inputminted" => crate::parser::minted::safe_syntax(source, parsed),
        "verbatiminput" => !parsed.definitions.iter().any(|definition| {
            matches!(
                definition.name.as_str(),
                "verbatiminput" | "verbatim@input" | "verbatim@readfile"
            )
        }),
        _ => false,
    }
}

fn safe_project(workspace: &Workspace, sources: &BTreeSet<Url>, name: &str) -> bool {
    if name == "inputminted"
        && sources
            .iter()
            .filter_map(|uri| workspace.documents.get(uri))
            .any(|doc| crate::parser::minted::raw_file_reader(&doc.text.source, &doc.syntax))
        && sources
            .iter()
            .filter_map(|uri| workspace.documents.get(uri))
            .any(|doc| {
                doc.syntax
                    .commands
                    .iter()
                    .filter(|command| command.name == "inputminted")
                    .filter_map(|command| syntax::file_argument(&doc.text.source, command))
                    .any(|argument| argument.value.chars().any(char::is_whitespace))
            })
    {
        return false;
    }
    sources
        .iter()
        .filter_map(|uri| workspace.documents.get(uri))
        .all(|doc| safe_syntax(&doc.text.source, &doc.syntax, name))
}

pub(crate) fn safe_call(source: &str, command: &syntax::Command) -> bool {
    match command.name.as_str() {
        "lstinputlisting" => listings::safe_call(source, command),
        "inputminted" => crate::parser::minted::safe_call(source, command),
        name if crate::parser::fancyvrb::is_input(name) => {
            crate::parser::fancyvrb::safe_call(source, command)
        }
        _ => true,
    }
}

fn literal_inputs(
    workspace: &Workspace,
    sources: &BTreeSet<Url>,
    root: &Url,
    name: &str,
    active: &BTreeMap<Url, BTreeSet<usize>>,
) -> BTreeSet<Url> {
    sources
        .iter()
        .filter_map(|uri| Some((workspace.documents.get(uri)?, active.get(uri)?)))
        .flat_map(|(doc, positions)| {
            doc.syntax.commands.iter().filter_map(move |command| {
                if command.name != name
                    || !positions.contains(&command.span.start)
                    || !safe_call(&doc.text.source, command)
                {
                    return None;
                }
                let argument = syntax::file_argument(&doc.text.source, command)?;
                resolve_file_path(root, &command.name, &argument.value, &[""])
            })
        })
        .take(4096)
        .collect()
}

/// A display call proves a data dependency, but not an exclusively data role:
/// another call can execute the very same file using commandchars or escapes.
fn displayed_inputs(workspace: &Workspace, sources: &BTreeSet<Url>, root: &Url) -> BTreeSet<Url> {
    let active = crate::project::source_inputs::possible_direct_file_positions(workspace, sources);
    let safe: BTreeSet<_> = COMMANDS
        .iter()
        .copied()
        .filter(|name| safe_project(workspace, sources, name))
        .collect();
    let mut displayed: BTreeSet<_> = safe
        .iter()
        .flat_map(|name| literal_inputs(workspace, sources, root, name, &active))
        .collect();
    for (doc, positions) in sources
        .iter()
        .filter_map(|uri| Some((workspace.documents.get(uri)?, active.get(uri)?)))
    {
        for command in &doc.syntax.commands {
            if !positions.contains(&command.span.start)
                || !syntax::code_input_command(&command.name)
                || safe.contains(command.name.as_str()) && safe_call(&doc.text.source, command)
            {
                continue;
            }
            // Redefinitions and dynamic filenames cannot identify which
            // recorded source was executed. Retain the recorder's source role.
            if !safe.contains(command.name.as_str()) {
                return BTreeSet::new();
            }
            let Some(argument) = syntax::file_argument(&doc.text.source, command) else {
                return BTreeSet::new();
            };
            if let Some(input) = resolve_file_path(root, &command.name, &argument.value, &[""]) {
                displayed.remove(&input);
            }
        }
    }
    displayed
}

fn context_sources(workspace: &Workspace, uri: &Url, root: &Url) -> BTreeSet<Url> {
    let mut explicit = workspace.source_documents_from(root, root);
    if uri != root {
        explicit.extend(workspace.source_documents_from(uri, root));
    }
    // Only understood calls can identify data. A redefined command may execute
    // the file as real source, including settings for the other package.
    let displayed = displayed_inputs(workspace, &explicit, root);
    let mut sources = explicit;
    if let Some(recorded) = workspace.recorded_inputs.get(root) {
        sources.extend(recorded.difference(&displayed).cloned());
    }
    sources
}

pub(crate) fn allows_spaces(workspace: &Workspace, uri: &Url, root: &Url) -> bool {
    context_sources(workspace, uri, root)
        .iter()
        .filter_map(|uri| workspace.documents.get(uri))
        .all(|doc| !crate::parser::minted::raw_file_reader(&doc.text.source, &doc.syntax))
}

pub(crate) fn displayed(workspace: &Workspace, root: &Url) -> BTreeSet<Url> {
    displayed_inputs(workspace, &context_sources(workspace, root, root), root)
}

pub(crate) fn supported(
    workspace: &Workspace,
    uri: &Url,
    root: &Url,
    command: &syntax::Command,
) -> bool {
    syntax::code_input_command(&command.name)
        && workspace
            .documents
            .get(uri)
            .is_some_and(|doc| safe_call(&doc.text.source, command))
        && safe_project(
            workspace,
            &context_sources(workspace, uri, root),
            &command.name,
        )
}

/// Uses the independent source walk so it is safe during project traversal.
pub(crate) fn inputs(workspace: &Workspace, root: &Url) -> BTreeSet<Url> {
    // Only these literal readers contribute inputs below. Most projects do
    // not use them, so avoid building a second source graph just to find none.
    if !workspace.documents.values().any(|doc| {
        doc.syntax
            .commands
            .iter()
            .any(|command| COMMANDS.contains(&command.name.as_str()))
    }) {
        return BTreeSet::new();
    }
    let sources = context_sources(workspace, root, root);
    let active = crate::project::source_inputs::possible_direct_file_positions(workspace, &sources);
    COMMANDS
        .iter()
        .filter(|name| safe_project(workspace, &sources, name))
        .flat_map(|name| literal_inputs(workspace, &sources, root, name, &active))
        .collect()
}
