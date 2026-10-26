//! Static length assignments used to verify etoolbox build diagnostics.
use crate::{parser::syntax, symbols::commands::CommandSignatures};
use std::ops::Range;

pub(crate) const BUILTINS: &[&str] = &[
    "textwidth",
    "textheight",
    "linewidth",
    "columnwidth",
    "columnsep",
    "parindent",
    "parskip",
    "baselineskip",
    "lineskip",
    "lineskiplimit",
    "topskip",
    "hsize",
    "vsize",
    "hoffset",
    "voffset",
    "abovedisplayskip",
    "abovedisplayshortskip",
    "belowdisplayskip",
    "belowdisplayshortskip",
    "tabcolsep",
    "arraycolsep",
    "fboxsep",
    "fboxrule",
    "itemsep",
    "parsep",
    "topsep",
    "partopsep",
    "leftmargin",
    "rightmargin",
    "emergencystretch",
];

pub(crate) fn declaration(entry: &syntax::Entry) -> bool {
    entry.is_variable
        && matches!(
            entry.detail.strip_prefix("Defined with \\"),
            Some("newlength" | "newskip" | "newdimen" | "skipdef" | "dimendef")
        )
}

/// Retain the whole control word, including its backslash, and the complete
/// assignment tail. Dynamic operands and incomplete expressions stay unlocated.
pub(crate) fn error_invocations_with_signatures(
    source: &str,
    parsed: &syntax::Syntax,
    signatures: &CommandSignatures<'_>,
) -> Vec<(Range<usize>, Range<usize>)> {
    if signatures.names().any(|name| name == "deflength") {
        return vec![];
    }
    let operands = signatures.argument_tokens(source, parsed);
    parsed
        .commands
        .iter()
        .filter(|command| command.name == "deflength" && !operands.contains(&command.span.start))
        .filter_map(|command| {
            let start = syntax::skip_space(source, command.span.end);
            let (span, end) = if source.as_bytes().get(start) == Some(&b'{') {
                let (argument, end) = syntax::group(source, start, b'{', b'}')?;
                let span = syntax::control_word(source, argument.span.start)?;
                if span != argument.span {
                    return None;
                }
                (span, end)
            } else {
                let span = syntax::control_word(source, start)?;
                let end = span.end;
                (span, end)
            };
            let (_, end) = syntax::group(source, syntax::skip_space(source, end), b'{', b'}')?;
            Some((span, start..end))
        })
        .collect()
}
