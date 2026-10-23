//! listings environment boundaries and conservative default filename search.
use crate::syntax;
use std::ops::Range;

/// lstinline reads an ordinary optional argument before its delimiter. Braced
/// code ends at the first literal right brace; body catcodes neither balance
/// braces nor recognize escapes or comments. An unfinished body ends at EOL.
pub(crate) fn inline_span(source: &str, command_end: usize) -> Option<Range<usize>> {
    let mut start = syntax::skip_space(source, command_end);
    if source.as_bytes().get(start) == Some(&b'[') {
        start = syntax::skip_space(source, syntax::bracket_argument(source, start)?.1);
    }
    let delimiter = source.get(start..)?.chars().next()?;
    if delimiter.is_whitespace() || matches!(delimiter, '}' | '\\' | '#' | '%') {
        return None;
    }
    let close = if delimiter == '{' { '}' } else { delimiter };
    let body = start + delimiter.len_utf8();
    let line_end = source[body..]
        .find(['\r', '\n'])
        .map_or(source.len(), |offset| body + offset);
    let end = source[body..line_end]
        .find(close)
        .map_or(line_end, |offset| body + offset + close.len_utf8());
    Some(start..end)
}

/// listings reads its optional settings and one name token before changing
/// the body catcodes. A newline name token starts the body immediately; any
/// other name makes lstenv@Process discard the remainder of its header line.
pub(crate) fn environment_end(source: &str, header_start: usize) -> Option<usize> {
    let at = environment_options_end(source, header_start)?;
    let at = syntax::skip_space_with_active_eol(source, at);
    let body_start = match source.as_bytes().get(at)? {
        b'\r' if source[at..].starts_with("\r\n") => at + 2,
        b'\r' | b'\n' => at + 1,
        _ => {
            let end = if source.as_bytes()[at] == b'{' {
                syntax::group(source, at, b'{', b'}')?.1
            } else {
                at + syntax::token(source, at)?.len()
            };
            end + source[end..].find('\n')? + 1
        }
    };
    let ending = "\\end{lstlisting}";
    source[body_start..]
        .find(ending)
        .map(|offset| body_start + offset + ending.len())
}

/// The public header has one optional settings list. Share its boundary with
/// signature help, stopping lookahead at an ordinary active newline.
pub(crate) fn environment_options_end(source: &str, header_start: usize) -> Option<usize> {
    let mut at = syntax::skip_space_with_active_eol(source, header_start);
    if source.as_bytes().get(at) == Some(&b'[') {
        at = syntax::bracket_argument(source, at)?.1;
    }
    Some(at)
}

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
