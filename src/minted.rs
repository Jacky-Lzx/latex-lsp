//! Minted headers use ordinary TeX arguments; code is read verbatim.
use crate::syntax::{self, Argument};
use std::ops::Range;

pub(crate) fn is_inline(name: &str) -> bool {
    matches!(name, "mintinline" | "mint")
}

pub(crate) fn language(source: &str, end: usize) -> Option<Argument> {
    let mut at = syntax::skip_space(source, end);
    if source.as_bytes().get(at) == Some(&b'[') {
        at = syntax::skip_space(source, syntax::bracket_argument(source, at)?.1);
    }
    syntax::group(source, at, b'{', b'}').map(|(argument, _)| argument)
}

/// Braced code balances literal braces. Backslashes and percent signs in
/// the body are data, so neither escapes a brace nor starts a TeX comment.
/// Both forms accept line breaks, as fvextra's ReadVArg does. An unfinished
/// argument remains literal through the end of input.
pub(crate) fn inline_span(source: &str, after_language: usize) -> Option<Range<usize>> {
    // ReadVArg changes catcodes before looking ahead: '%' and '\\' can be
    // delimiters here, and a percent sign no longer starts a header comment.
    let start = after_language
        + source[after_language..]
            .bytes()
            .take_while(u8::is_ascii_whitespace)
            .count();
    let delimiter = source.get(start..)?.chars().next()?;
    if delimiter.is_whitespace() || delimiter == '}' {
        return None;
    }
    let mut depth = 1usize;
    let body = start + delimiter.len_utf8();
    for (offset, ch) in source[body..].char_indices() {
        let at = body + offset;
        if delimiter == '{' {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return Some(start..at + 1);
            }
        } else if ch == delimiter {
            return Some(start..at + ch.len_utf8());
        }
    }
    Some(start..source.len())
}
