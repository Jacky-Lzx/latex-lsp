//! Static file inputs displayed as code, with independent package guards.
use crate::{
    listings, syntax,
    workspace::{resolve_file_path, Workspace},
};
use std::collections::BTreeSet;
use tower_lsp::lsp_types::Url;

const COMMANDS: [&str; 2] = ["lstinputlisting", "verbatiminput"];

fn safe_document(source: &str, parsed: &syntax::Syntax, name: &str) -> bool {
    match name {
        "lstinputlisting" => listings::safe_syntax(source, parsed),
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
    sources
        .iter()
        .filter_map(|uri| workspace.documents.get(uri))
        .all(|doc| safe_document(&doc.text.source, &doc.syntax, name))
}

fn safe_call(source: &str, command: &syntax::Command) -> bool {
    command.name != "lstinputlisting" || listings::safe_call(source, command)
}

fn literal_inputs(
    workspace: &Workspace,
    sources: &BTreeSet<Url>,
    root: &Url,
    name: &str,
) -> BTreeSet<Url> {
    sources
        .iter()
        .filter_map(|uri| workspace.documents.get(uri))
        .flat_map(|doc| {
            doc.syntax.commands.iter().filter_map(move |command| {
                if command.name != name || !safe_call(&doc.text.source, command) {
                    return None;
                }
                let argument = syntax::file_argument(&doc.text.source, command)?;
                resolve_file_path(root, &command.name, &argument.value, &[""])
            })
        })
        .take(4096)
        .collect()
}

fn context_sources(workspace: &Workspace, uri: &Url, root: &Url) -> BTreeSet<Url> {
    let mut explicit = workspace.source_documents_from(root, root);
    explicit.extend(workspace.source_documents_from(uri, root));
    // Only understood calls can identify data. A redefined command may execute
    // the file as real source, including settings for the other package.
    let displayed: BTreeSet<_> = COMMANDS
        .iter()
        .filter(|name| safe_project(workspace, &explicit, name))
        .flat_map(|name| literal_inputs(workspace, &explicit, root, name))
        .collect();
    let mut sources = explicit;
    if let Some(recorded) = workspace.recorded_inputs.get(root) {
        sources.extend(recorded.difference(&displayed).cloned());
    }
    sources
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
    let sources = context_sources(workspace, root, root);
    COMMANDS
        .iter()
        .filter(|name| safe_project(workspace, &sources, name))
        .flat_map(|name| literal_inputs(workspace, &sources, root, name))
        .collect()
}
