//! Verify direct etoolbox toggle operations without expanding TeX.
use crate::{signature::CommandSignatures, syntax};
use std::ops::Range;

pub(crate) fn error_invocations_with_signatures(
    source: &str,
    parsed: &syntax::Syntax,
    signatures: &CommandSignatures<'_>,
) -> Vec<(Range<usize>, Range<usize>)> {
    let operands = signatures.argument_tokens(source, parsed);
    let overridden: std::collections::BTreeSet<_> = signatures.names().collect();
    parsed
        .commands
        .iter()
        .filter(|command| {
            syntax::toggle_reference(&command.name)
                && !operands.contains(&command.span.start)
                && !overridden.contains(command.name.as_str())
        })
        .filter_map(|command| {
            let start = syntax::skip_space(source, command.span.end);
            let (name, mut end) = syntax::group(source, start, b'{', b'}')?;
            if !syntax::is_static_key(&source[name.span.clone()]) {
                return None;
            }
            // The setters consume their value before reporting an error;
            // iftoggle/nottoggle consume their branches only after it.
            if command.name == "settoggle" {
                end = syntax::group(source, syntax::skip_space(source, end), b'{', b'}')?.1;
            }
            Some((name.span, start..end))
        })
        .collect()
}
