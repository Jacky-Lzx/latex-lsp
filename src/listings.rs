//! Conservative support for listings' default filename search.
use crate::{syntax, workspace::Workspace};
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

pub(crate) fn supported(
    workspace: &Workspace,
    uri: &Url,
    root: &Url,
    command: &syntax::Command,
) -> bool {
    let Some(doc) = workspace.documents.get(uri) else {
        return false;
    };
    let start = syntax::skip_space(&doc.text.source, command.span.end);
    if doc.text.source.as_bytes().get(start) == Some(&b'[') {
        let Some((options, _)) = syntax::bracket_argument(&doc.text.source, start) else {
            return false;
        };
        if !safe_options(&options.value) {
            return false;
        }
    }
    workspace
        .project_documents_from(uri, root)
        .into_iter()
        .chain(workspace.project_documents(root))
        .filter_map(|uri| workspace.documents.get(&uri))
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
