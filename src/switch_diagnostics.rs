//! Verify direct etoolbox boolean and toggle operations without expanding TeX.
use crate::{signature::CommandSignatures, syntax};
use std::ops::Range;

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Boolean,
    Toggle,
}

impl Kind {
    fn command(self, name: &str) -> bool {
        match self {
            Self::Toggle => syntax::toggle_reference(name),
            Self::Boolean => matches!(
                name,
                "setbool" | "booltrue" | "boolfalse" | "ifbool" | "notbool"
            ),
        }
    }
}

pub(crate) fn boolean_test(
    doc: &crate::workspace::Document,
    range: tower_lsp::lsp_types::Range,
) -> bool {
    doc.syntax.commands.iter().any(|command| {
        matches!(command.name.as_str(), "ifbool" | "notbool")
            && command
                .argument
                .as_ref()
                .is_some_and(|name| doc.text.range(name.span.start, name.span.end) == range)
    })
}

pub(crate) fn error_invocations_with_signatures(
    source: &str,
    parsed: &syntax::Syntax,
    signatures: &CommandSignatures<'_>,
    kind: Kind,
) -> Vec<(Range<usize>, Range<usize>)> {
    let operands = signatures.argument_tokens(source, parsed);
    let overridden: std::collections::BTreeSet<_> = signatures.names().collect();
    parsed
        .commands
        .iter()
        .filter(|command| {
            kind.command(&command.name)
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
            // Conditional tests consume their branches only after it.
            if matches!(command.name.as_str(), "settoggle" | "setbool") {
                end = syntax::group(source, syntax::skip_space(source, end), b'{', b'}')?.1;
            }
            Some((name.span, start..end))
        })
        .collect()
}
