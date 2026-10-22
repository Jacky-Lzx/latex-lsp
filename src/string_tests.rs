//! Literal data read by etoolbox's string predicates, before their branches.
use crate::syntax;
use std::ops::Range;

pub(crate) fn data_count(name: &str) -> Option<usize> {
    match name {
        "ifstrequal" => Some(2),
        "ifstrempty" | "ifblank" | "notblank" => Some(1),
        _ => None,
    }
}

pub(crate) struct LiteralArgument {
    pub start: usize,
    pub span: Range<usize>,
    pub end: usize,
}

pub(crate) fn ends_in_comment(source: &str, span: &Range<usize>) -> bool {
    let source = &source[..span.end];
    let mut at = span.start;
    while at < source.len() {
        if source.as_bytes()[at] == b'%' {
            let Some(end) = source[at..].find('\n') else {
                return true;
            };
            at += end + 1;
        } else {
            at = syntax::token(source, at).map_or(source.len(), |token| at + token.len());
        }
    }
    false
}

pub(crate) fn arguments(source: &str, mut at: usize, count: usize) -> Vec<LiteralArgument> {
    let mut result = Vec::new();
    for _ in 0..count {
        at = syntax::skip_space(source, at);
        if source.as_bytes().get(at) == Some(&b'{') {
            if let Some((argument, end)) = syntax::group(source, at, b'{', b'}') {
                result.push(LiteralArgument {
                    start: at,
                    span: argument.span,
                    end,
                });
                at = end;
            } else {
                // Visit the opening brace normally to retain its missing-brace
                // diagnostic, while keeping unfinished data out of the index.
                result.push(LiteralArgument {
                    start: at + 1,
                    span: at + 1..source.len(),
                    end: source.len(),
                });
                break;
            }
        } else if let Some(token) = syntax::required_token(source, at).filter(|token| *token != "}")
        {
            let end = at + token.len();
            result.push(LiteralArgument {
                start: at,
                span: at..end,
                end,
            });
            at = end;
        } else {
            break;
        }
    }
    result
}
