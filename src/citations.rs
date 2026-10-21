//! Biblatex multicite argument boundaries, shared by indexing and editor queries.
use crate::syntax::{self, Argument, Command};
use std::ops::Range;

pub(crate) fn is_multi(name: &str) -> bool {
    matches!(
        name,
        "cites"
            | "Cites"
            | "parencites"
            | "Parencites"
            | "footcites"
            | "Footcites"
            | "footcitetexts"
            | "Footcitetexts"
            | "smartcites"
            | "Smartcites"
            | "supercites"
            | "textcites"
            | "Textcites"
            | "autocites"
            | "Autocites"
    )
}

pub(crate) struct Field {
    pub argument: Argument,
    pub open: u8,
    pub complete: bool,
    pub group: usize,
}

pub(crate) struct Arguments {
    pub fields: Vec<Field>,
    pub end: usize,
}

impl Arguments {
    pub fn keys(&self) -> impl Iterator<Item = &Argument> {
        self.fields
            .iter()
            .filter(|field| field.open == b'{' && field.complete)
            .map(|field| &field.argument)
    }
}

// A blank line supplies a paragraph token, rather than ignorable TeX spaces.
fn space(source: &str, mut at: usize) -> usize {
    let bytes = source.as_bytes();
    let mut newline = false;
    loop {
        match bytes.get(at) {
            Some(b'%') => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                if at < bytes.len() {
                    at += 1;
                }
                // A comment removes its endline without changing TeX's
                // current line state.
            }
            Some(b' ' | b'\t') => at += 1,
            Some(b'\r' | b'\n') if !newline => {
                if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
                    at += 1;
                }
                at += 1;
                newline = true;
            }
            _ => return at,
        }
    }
}

fn field(source: &str, at: usize, open: u8, group: usize) -> Field {
    let parsed = match open {
        b'(' => syntax::parenthesis_note(source, at),
        b'[' => syntax::bracket_argument(source, at),
        _ => syntax::group(source, at, b'{', b'}'),
    };
    let complete = parsed.is_some();
    let argument = parsed
        .map(|(argument, _)| argument)
        .unwrap_or_else(|| Argument {
            value: source[at + 1..].into(),
            span: at + 1..source.len(),
            outer_span: at..source.len(),
        });
    Field {
        argument,
        open,
        complete,
        group,
    }
}

pub(crate) fn arguments(source: &str, end: usize) -> Arguments {
    let mut at = space(source, end);
    if source.as_bytes().get(at) == Some(&b'*') {
        at = space(source, at + 1);
    }
    let mut result = Arguments {
        fields: Vec::new(),
        end: at,
    };
    for _ in 0..2 {
        if source.as_bytes().get(at) != Some(&b'(') {
            break;
        }
        let part = field(source, at, b'(', 0);
        let complete = part.complete;
        at = part.argument.outer_span.end;
        result.fields.push(part);
        result.end = at;
        if !complete {
            return result;
        }
        at = space(source, at);
    }
    let mut group = 0;
    loop {
        group += 1;
        for _ in 0..2 {
            if source.as_bytes().get(at) != Some(&b'[') {
                break;
            }
            let part = field(source, at, b'[', group);
            let complete = part.complete;
            at = part.argument.outer_span.end;
            result.fields.push(part);
            result.end = at;
            if !complete {
                return result;
            }
            at = space(source, at);
        }
        if source.as_bytes().get(at) != Some(&b'{') {
            break;
        }
        let part = field(source, at, b'{', group);
        let complete = part.complete;
        at = part.argument.outer_span.end;
        result.fields.push(part);
        result.end = at;
        if !complete {
            break;
        }
        at = space(source, at);
    }
    // Biblatex looks ahead to the first non-space token before executing
    // the accumulated citations. Its warnings can therefore name that line.
    result.end = at + source[at..].chars().next().map_or(0, char::len_utf8);
    result
}

pub(crate) fn prefix(source: &str, command: &Command, cursor: usize) -> Option<Range<usize>> {
    let parsed = arguments(source, command.span.end);
    parsed.fields.iter().find_map(|field| {
        (field.open == b'{')
            .then(|| {
                syntax::active_key_list_prefix(
                    source,
                    field.argument.outer_span.start,
                    cursor,
                    syntax::is_static_reference_key,
                )
            })
            .flatten()
    })
}
