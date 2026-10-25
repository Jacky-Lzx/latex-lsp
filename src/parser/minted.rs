//! Minted headers use ordinary TeX arguments; code is read verbatim.
use crate::{
    parser::syntax::{self, Argument},
    text::Text,
};
use std::ops::Range;
use tower_lsp::lsp_types::TextEdit;

pub(crate) fn is_inline(name: &str) -> bool {
    matches!(name, "mintinline" | "mint")
}

pub(crate) fn language(source: &str, end: usize) -> Option<Argument> {
    let at = options_start(source, end)?;
    syntax::group(source, at, b'{', b'}').map(|(argument, _)| argument)
}

/// The optional settings and language are ordinary TeX arguments. Only after
/// they finish does FancyVerb discard the rest of that header line and scan
/// literal body lines; a marker nested in the header cannot close the block.
pub(crate) fn environment_end(source: &str, header_start: usize) -> Option<usize> {
    let language_end = environment_header_end(source, header_start)?;
    let body_start = language_end + source[language_end..].find('\n')? + 1;
    crate::parser::fancyvrb::environment_end_from_body(source, body_start, "minted")
}

pub(crate) fn environment_header_end(source: &str, header_start: usize) -> Option<usize> {
    let start = options_start(source, header_start)?;
    crate::parser::fancyvrb::name_argument(source, start).map(|(_, end)| end)
}

/// Direct braced code balances literal braces. Backslashes and percent signs in
/// the body are data, so neither escapes a brace nor starts a TeX comment.
/// Both forms accept line breaks, as fvextra's ReadVArg does. An unfinished
/// argument remains literal through the end of input. Already collected macro
/// arguments retain ordinary TeX tokens, including comments before the code.
pub(crate) fn inline_span(
    source: &str,
    after_language: usize,
    tokenized: bool,
) -> Option<Range<usize>> {
    // Direct ReadVArg changes catcodes before looking ahead: '%' and '\\'
    // can be delimiters. Collected arguments keep their original catcodes.
    let start = if tokenized {
        syntax::skip_space(source, after_language)
    } else {
        after_language
            + source[after_language..]
                .bytes()
                .take_while(u8::is_ascii_whitespace)
                .count()
    };
    let delimiter = source.get(start..)?.chars().next()?;
    if delimiter.is_whitespace() || delimiter == '}' {
        return None;
    }
    if tokenized {
        let end = syntax::tokenized_code_end(source, start).unwrap_or(source.len());
        return Some(start..end);
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

fn options_start(source: &str, end: usize) -> Option<usize> {
    let mut at = syntax::skip_space(source, end);
    if source.as_bytes().get(at) == Some(&b'[') {
        at = syntax::skip_space(source, syntax::bracket_argument(source, at)?.1);
    }
    Some(at)
}

/// The highlighted filename is sent to Python as a string. Quotes are not
/// filename delimiters there; detokenize protects TeX characters without
/// introducing quotes into that string.
pub(crate) fn filename(source: &str, content: Range<usize>, partial: bool) -> Option<Range<usize>> {
    let mut span = syntax::trimmed_span(source, content);
    if source[span.clone()].starts_with("\\detokenize") {
        let open = syntax::skip_space(source, span.start + "\\detokenize".len());
        if source.as_bytes().get(open) != Some(&b'{') {
            return None;
        }
        if let Some((argument, end)) = syntax::group(source, open, b'{', b'}') {
            if end > span.end || !source[end..span.end].trim().is_empty() {
                return None;
            }
            span = argument.span;
        } else if partial {
            span = open + 1..span.end;
        } else {
            return None;
        }
    }
    let value = &source[span.clone()];
    (value.is_empty() && partial || syntax::is_literal_file_path(value)).then_some(span)
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

pub(crate) fn filename_edit(
    text: &Text,
    command: &syntax::Command,
    value: String,
) -> Option<TextEdit> {
    let argument = command.argument.as_ref()?;
    let span = filename(&text.source, argument.span.clone(), false)?;
    if !value.is_ascii() && !text.source[argument.span.start..span.start].contains("\\detokenize") {
        Some(TextEdit {
            range: text.range(argument.span.start, argument.span.end),
            new_text: format!("\\detokenize{{{value}}}"),
        })
    } else {
        Some(TextEdit {
            range: text.range(span.start, span.end),
            new_text: value,
        })
    }
}

pub(crate) fn completion_edit(
    text: &Text,
    command: &syntax::Command,
    mut value: String,
    replaced_end: usize,
) -> Option<TextEdit> {
    let open = syntax::file_name_argument_start(&text.source, &command.name, command.span.end)?;
    let content = syntax::group(&text.source, open, b'{', b'}')
        .map_or(open + 1..text.source.len(), |(argument, _)| argument.span);
    let span = filename(&text.source, content.clone(), true)?;
    value.push_str(text.source.get(replaced_end..span.end)?);
    let wrapped =
        !value.is_ascii() || text.source[content.start..span.start].contains("\\detokenize");
    Some(TextEdit {
        range: text.range(content.start, content.end),
        new_text: if wrapped {
            format!("\\detokenize{{{value}}}")
        } else {
            value
        },
    })
}

fn safe_options(source: &str) -> bool {
    if source.contains(['\\', '%']) {
        return false;
    }
    syntax::key_list_segments(source, 0..source.len())
        .into_iter()
        .all(|span| {
            let raw = &source[span];
            let (key, value) = raw.split_once('=').unwrap_or((raw, ""));
            match key.trim() {
                "texcomments" | "mathescape" => value.trim() == "false",
                "commandchars" => value.trim() == "none",
                ""
                | "style"
                | "firstline"
                | "lastline"
                | "linenos"
                | "firstnumber"
                | "stepnumber"
                | "numbersep"
                | "numberblanklines"
                | "showspaces"
                | "showtabs"
                | "tabsize"
                | "obeytabs"
                | "gobble"
                | "autogobble"
                | "breaklines"
                | "breakanywhere"
                | "breakautoindent"
                | "breakindent"
                | "breaksymbolleft"
                | "breaksymbolright"
                | "frame"
                | "framerule"
                | "framesep"
                | "rulecolor"
                | "bgcolor"
                | "fillcolor"
                | "label"
                | "labelposition"
                | "fontsize"
                | "fontfamily"
                | "fontshape"
                | "fontseries"
                | "baselinestretch"
                | "xleftmargin"
                | "xrightmargin"
                | "resetmargins"
                | "samepage"
                | "highlightlines"
                | "highlightcolor"
                | "stripall"
                | "stripnl"
                | "encoding"
                | "outencoding"
                | "python3"
                | "startinline"
                | "funcnamehighlighting" => true,
                _ => false,
            }
        })
}

pub(crate) fn safe_call(source: &str, command: &syntax::Command) -> bool {
    let Some(language) = language(source, command.span.end) else {
        return false;
    };
    if !syntax::is_static_key(language.value.trim()) {
        return false;
    }
    let start = syntax::skip_space(source, command.span.end);
    source.as_bytes().get(start) != Some(&b'[')
        || syntax::bracket_argument(source, start)
            .is_some_and(|(options, _)| safe_options(&options.value))
}

/// Verbatim mode (and legacy draft mode) bypasses Python and uses openin.
/// Keep its whitespace-sensitive path rules separate from normal highlighting.
pub(crate) fn raw_file_reader(source: &str, parsed: &syntax::Syntax) -> bool {
    parsed.commands.iter().any(|command| {
        if !matches!(
            command.name.as_str(),
            "usepackage" | "RequirePackage" | "PassOptionsToPackage"
        ) {
            return false;
        }
        let Some(argument) = syntax::file_argument(source, command) else {
            return false;
        };
        if !argument
            .value
            .split(',')
            .any(|name| name.trim() == "minted")
        {
            return false;
        }
        let options = if command.name == "PassOptionsToPackage" {
            command.argument.clone()
        } else {
            syntax::bracket_argument(source, syntax::skip_space(source, command.span.end))
                .map(|(argument, _)| argument)
        };
        options.is_some_and(|options| {
            syntax::key_list_segments(&options.value, 0..options.value.len())
                .into_iter()
                .any(|span| {
                    let option = options.value[span].trim();
                    let (key, value) = option.split_once('=').unwrap_or((option, "true"));
                    matches!(key.trim(), "verbatim" | "draft") && value.trim() != "false"
                })
        })
    })
}

pub(crate) fn safe_syntax(source: &str, parsed: &syntax::Syntax) -> bool {
    crate::parser::fancyvrb::safe_syntax(source, parsed)
        && !parsed.definitions.iter().any(|definition| {
            definition.name.starts_with("minted@")
                || definition.name.starts_with("RobustInputMinted")
                || matches!(
                    definition.name.as_str(),
                    "inputminted" | "setminted" | "import@path"
                )
        })
        && parsed
            .commands
            .iter()
            .all(|command| match command.name.as_str() {
                // import changes the filename prefix independently of the root.
                "import" | "subimport" | "inputfrom" | "subinputfrom" | "includefrom"
                | "subincludefrom" => false,
                name if name.starts_with("minted@") => false,
                "setminted" => options_start(source, command.span.end).is_some_and(|start| {
                    command.argument.as_ref().is_some_and(|argument| {
                        argument.outer_span.start == start && safe_options(&argument.value)
                    })
                }),
                "pgfkeys" | "pgfkeysalso" => command.argument.as_ref().is_some_and(|argument| {
                    !argument.value.contains(['\\', '%']) && !argument.value.contains("/minted")
                }),
                _ => true,
            })
}
