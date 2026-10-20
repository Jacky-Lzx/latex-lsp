//! Conservative support for listings' default filename search.
use crate::syntax;

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

pub(crate) fn safe_call(source: &str, command: &syntax::Command) -> bool {
    let start = syntax::skip_space(source, command.span.end);
    if source.as_bytes().get(start) != Some(&b'[') {
        return true;
    }
    syntax::bracket_argument(source, start).is_some_and(|(options, _)| safe_options(&options.value))
}

pub(crate) fn safe_syntax(source: &str, parsed: &syntax::Syntax) -> bool {
    !parsed.definitions.iter().any(|definition| {
        matches!(
            definition.name.as_str(),
            "lstinputlisting" | "lstset" | "lst@inputpath"
        )
    }) && parsed.commands.iter().all(|command| {
        command.name != "lstset"
            || command.argument.as_ref().is_some_and(|argument| {
                argument.outer_span.start == syntax::skip_space(source, command.span.end)
                    && safe_options(&argument.value)
            })
    })
}

/// The package prints the last suffix separately, defaulting to tex for a
/// suffixless basename. Dots in the directory never become file extensions.
pub(crate) fn missing_report(value: &str) -> String {
    let separator = value.rfind('/').map_or(0, |at| at + 1);
    match value[separator..].rsplit_once('.') {
        Some((base, extension)) => format!("{}{base}(.{extension})", &value[..separator]),
        None => format!("{value}(.tex)"),
    }
}
