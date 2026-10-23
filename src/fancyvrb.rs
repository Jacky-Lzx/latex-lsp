//! FancyVerb's literal filenames and conservative formatting settings.
use crate::{syntax, text::Text};
use std::ops::Range;
use tower_lsp::lsp_types::TextEdit;

pub(crate) fn is_input(name: &str) -> bool {
    matches!(name, "VerbatimInput" | "LVerbatimInput" | "BVerbatimInput")
}

pub(crate) fn is_use(name: &str) -> bool {
    matches!(
        name,
        "UseVerb" | "UseVerbatim" | "LUseVerbatim" | "BUseVerbatim" | "UseMVerb" | "pUseMVerb"
    )
}

pub(crate) fn is_inline_save(name: &str) -> bool {
    matches!(name, "SaveVerb" | "SaveGVerb")
}

pub(crate) fn is_inline(name: &str) -> bool {
    name == "Verb" || is_inline_save(name)
}

#[derive(Clone, Copy)]
struct InlineSettings {
    fvextra: bool,
    extra: bool,
    inline_extra: Option<bool>,
}

impl Default for InlineSettings {
    fn default() -> Self {
        Self {
            fvextra: false,
            extra: true,
            inline_extra: None,
        }
    }
}

/// Local source settings only. Imported package state and arbitrary keyval
/// expansion require project execution context rather than lexical guessing.
#[derive(Default)]
pub(crate) struct InlineContext {
    settings: InlineSettings,
    scopes: Vec<InlineSettings>,
}

impl InlineContext {
    pub(crate) fn enter(&mut self) {
        self.scopes.push(self.settings);
    }

    pub(crate) fn leave(&mut self) {
        if let Some(settings) = self.scopes.pop() {
            self.settings = settings;
        }
    }

    pub(crate) fn observe(&mut self, command: &syntax::Command) {
        match command.name.as_str() {
            "begingroup" | "bgroup" => self.enter(),
            "endgroup" | "egroup" => self.leave(),
            name if syntax::loader_extension(name) == Some("sty") => {
                if command.argument.as_ref().is_some_and(|argument| {
                    compact_tokens(&argument.value)
                        .split(',')
                        .any(|name| matches!(name, "fvextra" | "minted"))
                }) {
                    self.settings.fvextra = true;
                }
            }
            "fvset" | "fvinlineset" if self.settings.fvextra => {
                if let Some(extra) = command
                    .argument
                    .as_ref()
                    .and_then(|argument| extra_option(&argument.value))
                {
                    if command.name == "fvset" {
                        self.settings.extra = extra;
                    } else {
                        self.settings.inline_extra = Some(extra);
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn uses_extra(&self, source: &str, command_end: usize) -> bool {
        if !self.settings.fvextra {
            return false;
        }
        let mut at = syntax::skip_space(source, command_end);
        if source.as_bytes().get(at) == Some(&b'*') {
            at = syntax::skip_space(source, at + 1);
        }
        syntax::bracket_argument(source, at)
            .and_then(|(argument, _)| extra_option(&argument.value))
            .or(self.settings.inline_extra)
            .unwrap_or(self.settings.extra)
    }
}

fn extra_option(source: &str) -> Option<bool> {
    let mut extra = None;
    for span in syntax::key_list_segments(source, 0..source.len()) {
        let segment = compact_tokens(&source[span]);
        let (key, value) = segment.split_once('=').unwrap_or((&segment, "true"));
        if key != "extra" {
            continue;
        }
        let value = value.trim();
        let value = syntax::group(value, 0, b'{', b'}')
            .filter(|(_, end)| *end == value.len())
            .map_or_else(|| value.to_owned(), |(argument, _)| argument.value);
        // Unknown expansions retain isolation rather than exposing code as
        // executable source. Only an explicit false selects original FancyVerb.
        extra = Some(value.trim() != "false");
    }
    extra
}

fn compact_tokens(source: &str) -> String {
    let mut value = String::new();
    let mut at = 0;
    loop {
        at = syntax::skip_space(source, at);
        let Some(token) = syntax::token(source, at) else {
            return value;
        };
        value.push_str(token);
        at += token.len();
    }
}

/// Saved names are ordinary TeX arguments, including a single unbraced token.
pub(crate) fn name_argument(source: &str, at: usize) -> Option<(syntax::Argument, usize)> {
    if source.as_bytes().get(at) == Some(&b'{') {
        return syntax::group(source, at, b'{', b'}');
    }
    let ch = source.get(at..)?.chars().next()?;
    if ch.is_whitespace() || ch == '}' {
        return None;
    }
    let end = if ch == '\\' {
        syntax::control_word(source, at)
            .map(|span| span.end)
            .or_else(|| {
                source[at + 1..]
                    .chars()
                    .next()
                    .map(|ch| at + 1 + ch.len_utf8())
            })?
    } else {
        at + ch.len_utf8()
    };
    Some((
        syntax::Argument {
            value: source[at..end].into(),
            span: at..end,
            outer_span: at..end,
        },
        end,
    ))
}

pub(crate) struct InlineHeader {
    pub start: usize,
    pub body: usize,
    delimiter: char,
}

impl InlineHeader {
    pub(crate) fn raw_span(&self, source: &str) -> Range<usize> {
        let line_end = source[self.body..]
            .find(['\r', '\n'])
            .map_or(source.len(), |offset| self.body + offset);
        let end = source[self.body..line_end]
            .find(self.delimiter)
            .map_or(line_end, |offset| {
                self.body + offset + self.delimiter.len_utf8()
            });
        self.start..end
    }

    pub(crate) fn closing_marker(&self, source: &str) -> Option<Range<usize>> {
        let delimiter = self.delimiter.to_string();
        let end = syntax::tokenized_delimited_end(source, self.body, &delimiter)?;
        Some(end - delimiter.len()..end)
    }
}

/// The delimiter is an ordinary token or a single-character group. Comments
/// before it still continue lines. Direct source changes body catcodes, while
/// Verb in an already collected argument executes its original control tokens.
pub(crate) fn inline_header(source: &str, at: usize) -> Option<InlineHeader> {
    let start = syntax::skip_space(source, at);
    let (delimiter, body) = if source.as_bytes().get(start) == Some(&b'{') {
        let (argument, end) = syntax::group(source, start, b'{', b'}')?;
        let mut chars = argument.value.chars();
        let delimiter = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        (delimiter, end)
    } else {
        let delimiter = source.get(start..)?.chars().next()?;
        (delimiter, start + delimiter.len_utf8())
    };
    if delimiter.is_whitespace() || matches!(delimiter, '{' | '}' | '\\' | '%') {
        return None;
    }
    Some(InlineHeader {
        start,
        body,
        delimiter,
    })
}

pub(crate) fn is_environment(name: &str) -> bool {
    matches!(
        name.strip_suffix('*').unwrap_or(name),
        "Verbatim" | "LVerbatim" | "BVerbatim" | "SaveVerbatim" | "VerbatimOut"
    )
}

/// FancyVerb checks the first literal end marker on each line. A different
/// environment there makes the complete line data, including later markers.
pub(crate) fn environment_end(source: &str, header_start: usize, name: &str) -> Option<usize> {
    let body_start = environment_body_start(source, header_start, name)?;
    environment_end_from_body(source, body_start, name)
}

/// Shared with environments whose ordinary TeX header is read before the
/// FancyVerb scanner starts (for example minted's language argument).
pub(crate) fn environment_end_from_body(
    source: &str,
    body_start: usize,
    name: &str,
) -> Option<usize> {
    let ending = format!("\\end{{{name}}}");
    let mut offset = body_start;
    for line in source[body_start..].split_inclusive('\n') {
        if let Some(start) = line.find("\\end{") {
            if line[start..].starts_with(&ending) {
                return Some(offset + start + ending.len());
            }
        }
        offset += line.len();
    }
    None
}

/// Header comments are not separately indexed inside the enclosing raw span.
/// Check only the prefix before the caret, preserving TeX's escaped percent
/// tokens and CRLF comment boundaries.
pub(crate) fn header_comment(source: &str, start: usize, cursor: usize) -> bool {
    let mut at = start;
    while at < cursor {
        match source.as_bytes()[at] {
            b'\\' => {
                at += 1;
                if at < cursor {
                    at += source[at..].chars().next().unwrap().len_utf8();
                }
            }
            b'%' => {
                while at < cursor && source.as_bytes()[at] != b'\n' {
                    at += 1;
                }
                if at == cursor {
                    return true;
                }
            }
            _ => at += 1,
        }
    }
    false
}

fn environment_body_start(source: &str, header_start: usize, name: &str) -> Option<usize> {
    let (_, at) = environment_header_end(source, header_start, name)?;
    // FV@BeginScanning discards the remainder of the header line. Markers in
    // options, saved names and extraneous header text never close the body.
    source[at..].find('\n').map(|newline| at + newline + 1)
}

/// Signature boundary and actual token end. An active newline can be a name
/// token, but must not keep a name hint alive on the following body line.
pub(crate) fn environment_header_end(
    source: &str,
    header_start: usize,
    name: &str,
) -> Option<(usize, usize)> {
    // FV@Environment makes end-of-line active before looking for an optional
    // argument. Comments may continue the header; a plain newline cannot.
    let mut at = syntax::skip_space_with_active_eol(source, header_start);
    let options = source.as_bytes().get(at) == Some(&b'[');
    if options {
        at = syntax::bracket_argument(source, at)?.1;
    }
    if matches!(
        name.strip_suffix('*').unwrap_or(name),
        "SaveVerbatim" | "VerbatimOut"
    ) {
        // Reading options restores ordinary line endings before the required
        // argument. Without options, a peeked active newline is itself a token.
        at = if options {
            syntax::skip_space(source, at)
        } else {
            syntax::skip_space_with_active_eol(source, at)
        };
        let start = at;
        at = match source.as_bytes().get(at)? {
            b'{' => syntax::group(source, at, b'{', b'}')?.1,
            b'\\' => syntax::control_word(source, at)
                .map(|span| span.end)
                .or_else(|| {
                    source[at + 1..]
                        .chars()
                        .next()
                        .map(|ch| at + 1 + ch.len_utf8())
                })?,
            b'\r' if source[at..].starts_with("\r\n") => at + 2,
            _ => at + source[at..].chars().next()?.len_utf8(),
        };
        if matches!(source.as_bytes().get(start), Some(b'\r' | b'\n')) {
            return Some((start, at));
        }
    }
    Some((at, at))
}

/// A saved block's name is read before the literal body, with active EOL
/// during optional-argument lookahead. Do not treat an ordinary newline as
/// optional-header whitespace when there are no options.
pub(crate) fn saved_environment_argument(
    source: &str,
    header_start: usize,
) -> Option<syntax::Argument> {
    let mut at = syntax::skip_space_with_active_eol(source, header_start);
    if source.as_bytes().get(at) == Some(&b'[') {
        at = syntax::skip_space(source, syntax::bracket_argument(source, at)?.1);
    }
    name_argument(source, at).map(|(argument, _)| argument)
}

pub(crate) fn environment_options(source: &str, header_start: usize) -> Option<syntax::Argument> {
    syntax::bracket_argument(
        source,
        syntax::skip_space_with_active_eol(source, header_start),
    )
    .map(|(argument, _)| argument)
}

fn safe_options(source: &str) -> bool {
    if source.contains(['\\', '%']) {
        return false;
    }
    syntax::key_list_segments(source, 0..source.len())
        .into_iter()
        .all(|span| {
            let value = source[span].trim();
            let (key, argument) = value.split_once('=').unwrap_or((value, ""));
            match key.trim() {
                "commandchars" => argument.trim() == "none",
                "" | "firstline" | "lastline" | "frame" | "framerule" | "framesep" | "label"
                | "labelposition" | "numbers" | "numbersep" | "firstnumber" | "stepnumber"
                | "numberblanklines" | "showspaces" | "showtabs" | "tabsize" | "obeytabs"
                | "fontsize" | "fontfamily" | "fontshape" | "fontseries" | "baselinestretch"
                | "xleftmargin" | "xrightmargin" | "resetmargins" | "samepage" | "gobble" => true,
                _ => false,
            }
        })
}

fn internal_command(name: &str) -> bool {
    name.starts_with("FV@") || name.starts_with("FVC@") || name.starts_with("KV@FV@")
}

pub(crate) fn safe_syntax(source: &str, parsed: &syntax::Syntax) -> bool {
    !parsed.definitions.iter().any(|definition| {
        is_input(&definition.name)
            || internal_command(&definition.name)
            || matches!(
                definition.name.as_str(),
                "fvset" | "detokenize" | "setkeys" | "define@key"
            )
    }) && parsed
        .commands
        .iter()
        .all(|command| match command.name.as_str() {
            name if internal_command(name) => false,
            "CustomVerbatimCommand" | "RecustomVerbatimCommand" => false,
            // Direct keyval calls can change execution settings or redefine
            // the meaning of an otherwise harmless formatting key.
            "setkeys" | "define@key" => command.argument.as_ref().is_some_and(|argument| {
                argument.value.split(',').all(|family| {
                    let family = family.trim();
                    syntax::is_static_key(family) && family != "FV"
                })
            }),
            "fvset" => command.argument.as_ref().is_some_and(|argument| {
                argument.outer_span.start == syntax::skip_space(source, command.span.end)
                    && safe_options(&argument.value)
            }),
            _ => true,
        })
}

pub(crate) fn safe_call(source: &str, command: &syntax::Command) -> bool {
    let mut start = syntax::skip_space(source, command.span.end);
    if source.as_bytes().get(start) == Some(&b'*') {
        start = syntax::skip_space(source, start + 1);
    }
    if source.as_bytes().get(start) != Some(&b'[') {
        return true;
    }
    syntax::bracket_argument(source, start).is_some_and(|(options, _)| safe_options(&options.value))
}

/// Keep filename wrappers outside the editable span. Only the primitive
/// detokenize wrapper is understood, and project guards reject its redefinition.
pub(crate) fn filename(source: &str, content: Range<usize>, partial: bool) -> Option<Range<usize>> {
    if source[content.clone()].contains('%') {
        return None;
    }
    let mut span = syntax::trimmed_span(source, content);
    if source[span.clone()].starts_with("\\detokenize") {
        let open = syntax::skip_space(source, span.start + "\\detokenize".len());
        if source.as_bytes().get(open) != Some(&b'{') {
            return None;
        }
        if let Some((argument, after)) = syntax::group(source, open, b'{', b'}') {
            if after > span.end || !source[after..span.end].trim().is_empty() {
                return None;
            }
            span = argument.span;
        } else if partial {
            span = open + 1..span.end;
        } else {
            return None;
        }
    }
    let quoted = source.as_bytes().get(span.start) == Some(&b'"');
    if quoted {
        span.start += 1;
        if span.start < span.end && source.as_bytes().get(span.end - 1) == Some(&b'"') {
            span.end -= 1;
        } else if !partial {
            return None;
        }
    }
    let value = &source[span.clone()];
    if !(value.is_empty() && partial || syntax::is_literal_file_path(value))
        || !partial && !quoted && value.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some(span)
}

pub(crate) fn prefix(
    source: &str,
    command: &syntax::Command,
    cursor: usize,
) -> Option<Range<usize>> {
    let open = syntax::file_name_argument_start(source, &command.name, command.span.end)?;
    if source.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let content = syntax::group(source, open, b'{', b'}')
        .map_or(open + 1..source.len(), |(argument, _)| argument.span);
    let span = filename(source, content, true)?;
    (span.start <= cursor && cursor <= span.end).then_some(span.start..cursor)
}

/// Expanding the edit to the complete argument lets a rename introduce spaces
/// and UTF-8 safely for both pdfTeX and Unicode engines.
pub(crate) fn filename_edit(
    text: &Text,
    command: &syntax::Command,
    value: String,
) -> Option<TextEdit> {
    let argument = command.argument.as_ref()?;
    let old = filename(&text.source, argument.span.clone(), false)?;
    let wrapper = &text.source[argument.span.start..old.start];
    if !value.is_ascii() && !wrapper.contains("\\detokenize") {
        return Some(TextEdit {
            range: text.range(argument.span.start, argument.span.end),
            new_text: format!("\\detokenize{{\"{value}\"}}"),
        });
    }
    let quoted =
        old.start > argument.span.start && text.source.as_bytes().get(old.start - 1) == Some(&b'"');
    Some(TextEdit {
        range: text.range(old.start, old.end),
        new_text: if !quoted && value.chars().any(char::is_whitespace) {
            format!("\"{value}\"")
        } else {
            value
        },
    })
}

pub(crate) fn completion_edit(
    text: &Text,
    command: &syntax::Command,
    mut value: String,
    replaced_end: usize,
) -> Option<TextEdit> {
    let open = syntax::file_name_argument_start(&text.source, &command.name, command.span.end)?;
    if text.source.as_bytes().get(open) != Some(&b'{') {
        return None;
    }
    let content = syntax::group(&text.source, open, b'{', b'}')
        .map_or(open + 1..text.source.len(), |(argument, _)| argument.span);
    let span = filename(&text.source, content.clone(), true)?;
    // Completing a directory in the middle of a path must keep the remaining
    // components inside the filename wrappers.
    value.push_str(text.source.get(replaced_end..span.end)?);
    let wrapper = &text.source[content.start..span.start];
    let quoted = text.source.as_bytes().get(span.start.saturating_sub(1)) == Some(&b'"');
    let value = if quoted || value.chars().any(char::is_whitespace) || !value.is_ascii() {
        format!("\"{value}\"")
    } else {
        value
    };
    let value = if wrapper.contains("\\detokenize") || !value.is_ascii() {
        format!("\\detokenize{{{value}}}")
    } else {
        value
    };
    Some(TextEdit {
        range: text.range(content.start, content.end),
        new_text: value,
    })
}
