//! Biblatex argument boundaries shared by indexing, diagnostics and editor queries.
use crate::parser::syntax::{self, Argument, Command};
use std::ops::Range;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Style {
    Multi,
    Volume,
    MultiVolume,
    Data(&'static str),
    Category,
    Set,
    EntryData,
    EntryQuery,
    EntryCategoryQuery,
    ContextEntries,
}

impl Style {
    pub fn repeated(self) -> bool {
        matches!(self, Self::Multi | Self::MultiVolume)
    }
    pub fn volume(self) -> bool {
        matches!(self, Self::Volume | Self::MultiVolume)
    }
    pub fn selector(self) -> Option<&'static str> {
        match self {
            Self::Data(label) => Some(label),
            _ => None,
        }
    }
    pub fn accepts_star(self) -> bool {
        matches!(
            self,
            Self::Multi | Self::Volume | Self::MultiVolume | Self::EntryData | Self::ContextEntries
        )
    }
    pub fn context_entries(self) -> bool {
        matches!(self, Self::ContextEntries)
    }
    pub fn category_membership(self) -> bool {
        matches!(self, Self::Category)
    }
    pub fn set_definition(self) -> bool {
        matches!(self, Self::Set)
    }
    pub fn single_key(self) -> bool {
        self.context_codes() > 0
    }
    pub fn context_codes(self) -> usize {
        match self {
            Self::EntryData => 1,
            Self::EntryQuery | Self::EntryCategoryQuery => 2,
            _ => 0,
        }
    }
}

pub(crate) fn style(name: &str) -> Option<Style> {
    match name {
        "addtocategory" => Some(Style::Category),
        "assignrefcontextentries" => Some(Style::ContextEntries),
        "defbibentryset" => Some(Style::Set),
        "entrydata" => Some(Style::EntryData),
        "ifentryexists" | "ifentryinbib" => Some(Style::EntryQuery),
        "ifentrycategory" => Some(Style::EntryCategoryQuery),
        "citefield" => Some(Style::Data("field")),
        "citelist" => Some(Style::Data("literal list")),
        "citename" => Some(Style::Data("name list")),
        "cites" | "Cites" | "parencites" | "Parencites" | "footcites" | "Footcites"
        | "footcitetexts" | "Footcitetexts" | "smartcites" | "Smartcites" | "supercites"
        | "textcites" | "Textcites" | "autocites" | "Autocites" => Some(Style::Multi),
        "volcite" | "Volcite" | "pvolcite" | "Pvolcite" | "fvolcite" | "Fvolcite" | "ftvolcite"
        | "Ftvolcite" | "svolcite" | "Svolcite" | "tvolcite" | "Tvolcite" | "avolcite"
        | "Avolcite" => Some(Style::Volume),
        "volcites" | "Volcites" | "pvolcites" | "Pvolcites" | "fvolcites" | "Fvolcites"
        | "ftvolcites" | "Ftvolcites" | "svolcites" | "Svolcites" | "tvolcites" | "Tvolcites"
        | "avolcites" | "Avolcites" => Some(Style::MultiVolume),
        _ => None,
    }
}

pub(crate) fn is_structured(name: &str) -> bool {
    style(name).is_some()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Overall(usize),
    Note(usize),
    Volume,
    Pages,
    Keys,
    Format,
    Selector,
    Category,
    SetKey,
    Code(usize),
}

pub(crate) struct Field {
    pub argument: Argument,
    pub role: Role,
    pub complete: bool,
    pub group: usize,
}

pub(crate) struct Arguments {
    pub fields: Vec<Field>,
    pub end: usize,
    pub waiting: Option<Waiting>,
}

pub(crate) struct Waiting {
    pub role: Role,
    pub group: usize,
    pub span: Range<usize>,
}

impl Arguments {
    pub fn keys(&self) -> impl Iterator<Item = &Argument> {
        self.fields
            .iter()
            .filter(|field| field.role == Role::Keys && field.complete)
            .map(|field| &field.argument)
    }
}

// A blank line supplies a paragraph token, rather than ignorable TeX spaces.
pub(crate) fn space(source: &str, mut at: usize) -> usize {
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
                // A comment removes its endline without changing TeX's line state.
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

fn field(source: &str, at: usize, open: u8, group: usize, role: Role) -> Field {
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
        role,
        complete,
        group,
    }
}

// Keep the first incomplete field for completion and signature help, while
// indexing only complete key groups. Volumes and pages never enter the key index.
fn push(
    source: &str,
    result: &mut Arguments,
    at: &mut usize,
    open: u8,
    group: usize,
    role: Role,
) -> bool {
    let part = field(source, *at, open, group, role);
    let complete = part.complete;
    *at = part.argument.outer_span.end;
    result.fields.push(part);
    result.end = *at;
    if complete {
        *at = space(source, *at);
    }
    complete
}

fn wait_for(
    source: &str,
    command_end: usize,
    at: usize,
    result: &mut Arguments,
    group: usize,
    role: Role,
) {
    if at == source.len() {
        result.waiting = Some(Waiting {
            role,
            group,
            span: result
                .fields
                .last()
                .map_or(command_end, |field| field.argument.outer_span.end)..at,
        });
    }
}

pub(crate) fn arguments(source: &str, end: usize, style: Style) -> Arguments {
    let mut at = space(source, end);
    let mut result = Arguments {
        fields: Vec::new(),
        end: at,
        waiting: None,
    };
    if source.as_bytes().get(at) == Some(&b'*') {
        if !style.accepts_star() {
            return result;
        }
        at = space(source, at + 1);
        result.end = at;
    }
    if style.context_entries() {
        // Native assignments require a bracket argument, even when empty.
        // A second bracket is not another citation note.
        for (open, role) in [(b'[', Role::Note(0)), (b'{', Role::Keys)] {
            if source.as_bytes().get(at) != Some(&open) {
                wait_for(source, end, at, &mut result, 1, role);
                break;
            }
            if !push(source, &mut result, &mut at, open, 1, role) {
                return result;
            }
        }
        return result;
    }
    if style.single_key() {
        for role in std::iter::once(Role::Keys)
            .chain(matches!(style, Style::EntryCategoryQuery).then_some(Role::Category))
            .chain((0..style.context_codes()).map(Role::Code))
        {
            if source.as_bytes().get(at) != Some(&b'{') {
                wait_for(source, end, at, &mut result, 1, role);
                break;
            }
            if !push(source, &mut result, &mut at, b'{', 1, role) {
                return result;
            }
        }
        return result;
    }
    if style.category_membership() || style.set_definition() {
        let name = if style.set_definition() {
            Role::SetKey
        } else {
            Role::Category
        };
        for role in [name, Role::Keys] {
            if source.as_bytes().get(at) != Some(&b'{') {
                wait_for(source, end, at, &mut result, 1, role);
                break;
            }
            if !push(source, &mut result, &mut at, b'{', 1, role) {
                return result;
            }
        }
        return result;
    }
    if style.repeated() {
        for note in 0..2 {
            if source.as_bytes().get(at) != Some(&b'(') {
                break;
            }
            if !push(source, &mut result, &mut at, b'(', 0, Role::Overall(note)) {
                return result;
            }
        }
    }
    let mut group = 0;
    loop {
        group += 1;
        // Biblatex continues a multicite only when the next token is a note
        // or a brace group. Ordinary text or a control sequence ends it,
        // even though a volume inside a started group may be a bare token.
        if group > 1 && !matches!(source.as_bytes().get(at), Some(b'[' | b'{')) {
            break;
        }
        for note in 0..if style.volume() { 1 } else { 2 } {
            if source.as_bytes().get(at) != Some(&b'[') {
                break;
            }
            if !push(source, &mut result, &mut at, b'[', group, Role::Note(note)) {
                return result;
            }
        }
        if style.volume() {
            if source.as_bytes().get(at) == Some(&b'{') {
                if !push(source, &mut result, &mut at, b'{', group, Role::Volume) {
                    return result;
                }
            } else if let Some(value) = syntax::required_token(source, at) {
                let span = at..at + value.len();
                result.fields.push(Field {
                    argument: Argument {
                        value: value.into(),
                        span: span.clone(),
                        outer_span: span.clone(),
                    },
                    role: Role::Volume,
                    complete: true,
                    group,
                });
                at = space(source, span.end);
            } else {
                wait_for(source, end, at, &mut result, group, Role::Volume);
                break;
            }
            if source.as_bytes().get(at) == Some(&b'[')
                && !push(source, &mut result, &mut at, b'[', group, Role::Pages)
            {
                return result;
            }
        }
        if source.as_bytes().get(at) != Some(&b'{') {
            wait_for(source, end, at, &mut result, group, Role::Keys);
            break;
        }
        if !push(source, &mut result, &mut at, b'{', group, Role::Keys) {
            return result;
        }
        if style.selector().is_some() {
            if source.as_bytes().get(at) == Some(&b'[')
                && !push(source, &mut result, &mut at, b'[', group, Role::Format)
            {
                return result;
            }
            if source.as_bytes().get(at) != Some(&b'{') {
                wait_for(source, end, at, &mut result, group, Role::Selector);
                break;
            }
            if !push(source, &mut result, &mut at, b'{', group, Role::Selector) {
                return result;
            }
        }
        if !style.repeated() {
            break;
        }
    }
    // Biblatex can read the next non-space token before executing citations,
    // so its warning line may belong to the following command.
    result.end = at + source[at..].chars().next().map_or(0, char::len_utf8);
    result
}

pub(crate) fn prefix(source: &str, command: &Command, cursor: usize) -> Option<Range<usize>> {
    let style = style(&command.name)?;
    if style.single_key() {
        let (span, _) = single_key_context(source, command, cursor)?;
        let prefix = source[span.start..cursor].trim_start();
        return Some(cursor - prefix.len()..cursor);
    }
    let parsed = arguments(source, command.span.end, style);
    parsed.fields.iter().find_map(|field| {
        (field.role == Role::Keys)
            .then(|| {
                syntax::active_key_list_prefix(
                    source,
                    field.argument.outer_span.start,
                    cursor,
                    syntax::is_static_citation_key,
                )
            })
            .flatten()
    })
}

pub(crate) fn key_spans(source: &str, argument: &Argument, style: Style) -> Vec<Range<usize>> {
    if style.single_key() {
        let argument = syntax::single_citation_argument(source, argument.clone());
        if syntax::is_static_citation_key(&argument.value) {
            vec![argument.span]
        } else {
            Vec::new()
        }
    } else {
        syntax::citation_list_spans(source, argument.span.clone())
    }
}

pub(crate) fn single_key_context(
    source: &str,
    command: &Command,
    cursor: usize,
) -> Option<(Range<usize>, Vec<Range<usize>>)> {
    let style = style(&command.name).filter(|style| style.single_key())?;
    let parsed = arguments(source, command.span.end, style);
    let start = parsed.fields.first()?.argument.outer_span.start;
    syntax::active_commented_name_argument(
        source,
        start,
        b'{',
        cursor,
        true,
        syntax::is_static_citation_key,
    )
    .or_else(|| {
        syntax::active_citation_argument(source, start, b'{', cursor).map(|span| (span, Vec::new()))
    })
}
