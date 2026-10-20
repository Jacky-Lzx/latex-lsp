//! Conservative support for listings' default filename search.
use crate::{
    syntax,
    workspace::{resolve_file_path, Workspace},
};
use std::collections::BTreeSet;
use tower_lsp::lsp_types::Url;

// Styles, custom keys, and macro-valued options may alter inputpath. Only
// infer the default search when all relevant settings are understood.
fn safe_options(source: &str) -> bool {
    if source.contains(['\\', '%']) {
        return false;
    }
    syntax::key_list_segments(source, 0..source.len())
        .into_iter()
        .all(|span| {
            let value = source[span].trim();
            let key = value.split('=').next().unwrap_or("").trim();
            matches!(
                key,
                "" | "language"
                    | "firstline"
                    | "lastline"
                    | "linerange"
                    | "stepnumber"
                    | "firstnumber"
                    | "numbers"
                    | "numbersep"
                    | "numberblanklines"
                    | "showspaces"
                    | "showstringspaces"
                    | "showtabs"
                    | "tabsize"
                    | "columns"
                    | "keepspaces"
                    | "breaklines"
                    | "breakatwhitespace"
                    | "caption"
                    | "title"
                    | "label"
                    | "captionpos"
                    | "frame"
                    | "framerule"
                    | "framesep"
                    | "xleftmargin"
                    | "xrightmargin"
                    | "aboveskip"
                    | "belowskip"
                    | "linewidth"
            )
        })
}

fn safe_call(source: &str, command: &syntax::Command) -> bool {
    let start = syntax::skip_space(source, command.span.end);
    if source.as_bytes().get(start) != Some(&b'[') {
        return true;
    }
    syntax::bracket_argument(source, start).is_some_and(|(options, _)| safe_options(&options.value))
}

fn literal_inputs(workspace: &Workspace, sources: &BTreeSet<Url>, root: &Url) -> BTreeSet<Url> {
    sources
        .iter()
        .filter_map(|uri| workspace.documents.get(uri))
        .flat_map(|doc| {
            doc.syntax.commands.iter().filter_map(move |command| {
                if command.name != "lstinputlisting" || !safe_call(&doc.text.source, command) {
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
    let listed = literal_inputs(workspace, &explicit, root);
    let mut sources = explicit.clone();
    // Include recorded dynamic sources in the settings guard, while a shown
    // code sample's apparent lstset commands never configure the real project.
    if let Some(recorded) = workspace.recorded_inputs.get(root) {
        sources.extend(recorded.difference(&listed).cloned());
    }
    sources
}

fn safe_project(workspace: &Workspace, sources: &BTreeSet<Url>) -> bool {
    sources
        .iter()
        .filter_map(|uri| workspace.documents.get(uri))
        .all(|doc| {
            !doc.syntax.definitions.iter().any(|definition| {
                matches!(
                    definition.name.as_str(),
                    "lstinputlisting" | "lstset" | "lst@inputpath"
                )
            }) && doc.syntax.commands.iter().all(|command| {
                command.name != "lstset"
                    || command.argument.as_ref().is_some_and(|argument| {
                        argument.outer_span.start
                            == syntax::skip_space(&doc.text.source, command.span.end)
                            && safe_options(&argument.value)
                    })
            })
        })
}

pub(crate) fn supported(
    workspace: &Workspace,
    uri: &Url,
    root: &Url,
    command: &syntax::Command,
) -> bool {
    workspace
        .documents
        .get(uri)
        .is_some_and(|doc| safe_call(&doc.text.source, command))
        && safe_project(workspace, &context_sources(workspace, uri, root))
}

/// Build dependencies have data roles even if a sample ends in .tex. The
/// independent source walk keeps this query safe inside project traversal.
pub(crate) fn inputs(workspace: &Workspace, root: &Url) -> BTreeSet<Url> {
    let sources = context_sources(workspace, root, root);
    if !safe_project(workspace, &sources) {
        return BTreeSet::new();
    }
    literal_inputs(workspace, &sources, root)
}
