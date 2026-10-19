use std::ops::Range;

use crate::text::Text;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, TextEdit};

#[derive(Clone, Debug)]
pub struct Argument {
    pub value: String,
    pub span: Range<usize>,
    /// Includes delimiters when present; equals `span` for bare arguments.
    pub outer_span: Range<usize>,
}

#[derive(Clone, Debug)]
pub struct Command {
    pub name: String,
    pub span: Range<usize>,
    pub argument: Option<Argument>,
}

/// A literal `\csname name\endcsname`; names involving expansion, comments,
/// or embedded whitespace remain unresolved. Inspect only the next command
/// so malformed input cannot cause repeated scans of the remaining document.
pub(crate) fn literal_csname(
    source: &str,
    commands: &[Command],
    index: usize,
) -> Option<Range<usize>> {
    let command = commands
        .get(index)
        .filter(|command| command.name == "csname")?;
    let end = commands
        .get(index + 1)
        .filter(|command| command.name == "endcsname")?;
    let span = trimmed_span(source, command.span.end..end.span.start);
    is_static_key(&source[span.clone()]).then_some(span)
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub span: Range<usize>,
    pub detail: String,
    pub is_variable: bool,
}

#[derive(Clone, Debug)]
pub struct CitationField {
    pub name: String,
    pub argument: Argument,
}

#[derive(Clone, Debug, Default)]
pub struct Syntax {
    pub commands: Vec<Command>,
    pub labels: Vec<Entry>,
    pub anchors: Vec<Entry>,
    pub anchor_references: Vec<Entry>,
    pub citations: Vec<Entry>,
    pub citation_fields: Vec<CitationField>,
    pub citation_references: Vec<Entry>,
    pub definitions: Vec<Entry>,
    pub environments: Vec<Entry>,
    pub counters: Vec<Entry>,
    pub conditionals: Vec<Entry>,
    pub toggles: Vec<Entry>,
    pub diagnostics: Vec<Diagnostic>,
    pub ignored: Vec<Range<usize>>,
    /// Actual comments, distinguished from ignored URLs and verbatim text.
    pub comments: Vec<Range<usize>>,
    pub fixes: Vec<QuickFix>,
}

#[derive(Clone, Debug)]
pub struct QuickFix {
    pub title: String,
    pub diagnostic: Diagnostic,
    pub edit: TextEdit,
    pub additional_edits: Vec<TextEdit>,
}

impl QuickFix {
    pub fn edits(&self) -> Vec<TextEdit> {
        std::iter::once(&self.edit)
            .chain(&self.additional_edits)
            .cloned()
            .collect()
    }
}

pub fn diagnostic(text: &Text, span: Range<usize>, message: String) -> Diagnostic {
    Diagnostic {
        range: text.range(span.start, span.end),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("latex-lsp".into()),
        message,
        ..Diagnostic::default()
    }
}

pub(crate) fn skip_space(source: &str, mut i: usize) -> usize {
    let bytes = source.as_bytes();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'%') {
            return i;
        }
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
    }
}

/// Read a balanced group, ignoring comments and escaped delimiters.
pub(crate) fn group(source: &str, i: usize, open: u8, close: u8) -> Option<(Argument, usize)> {
    group_mode(source, i, open, close, true)
}

fn group_mode(
    source: &str,
    i: usize,
    open: u8,
    close: u8,
    comments: bool,
) -> Option<(Argument, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(i) != Some(&open) {
        return None;
    }
    let start = i + 1;
    let mut i = start;
    let mut depth = 1;
    let mut braces = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 1;
                if i < bytes.len() {
                    i += source[i..].chars().next()?.len_utf8();
                }
                continue;
            }
            b'%' if comments => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'{' if open != b'{' => braces += 1,
            b'}' if open != b'{' => braces = braces.saturating_sub(1),
            c if c == open && braces == 0 => depth += 1,
            c if c == close && braces == 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some((
                        Argument {
                            value: source[start..i].into(),
                            span: start..i,
                            outer_span: start - 1..i + 1,
                        },
                        i + 1,
                    ));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Literal TeX filename forms: `\input chapter.tex` and `\input "my file"`.
/// Stop at token boundaries and leave macro-expanded names to the recorder.
fn input_argument(source: &str, start: usize) -> Option<(Argument, usize)> {
    let bytes = source.as_bytes();
    let quoted = bytes.get(start) == Some(&b'"');
    let content = start + usize::from(quoted);
    let mut end = content;
    while end < bytes.len() {
        let byte = bytes[end];
        if quoted && byte == b'"'
            || matches!(byte, b'\n' | b'\r')
            || !quoted && byte.is_ascii_whitespace()
        {
            break;
        }
        if matches!(
            byte,
            b'\\' | b'{' | b'}' | b'%' | b'#' | b'$' | b'&' | b'^' | b'~'
        ) || !quoted && byte == b'"'
        {
            return None;
        }
        end += 1;
    }
    let after = end + usize::from(quoted && bytes.get(end) == Some(&b'"'));
    if quoted
        && bytes
            .get(after)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }
    Some((
        Argument {
            value: source[content..end].into(),
            span: content..end,
            outer_span: start..after,
        },
        after,
    ))
}

pub fn is_reference(name: &str) -> bool {
    warns_on_undefined_reference(name)
        || matches!(
            name,
            "getrefnumber"
                | "getpagerefnumber"
                | "getrefbykeydefault"
                | "IfRefUndefinedExpandable"
                | "IfRefUndefinedBabel"
        )
}

pub(crate) fn warns_on_undefined_reference(name: &str) -> bool {
    matches!(
        name,
        "ref"
            | "Ref"
            | "eqref"
            | "pageref"
            | "autopageref"
            | "autoref"
            | "cref"
            | "Cref"
            | "vref"
            | "nameref"
            | "refused"
    )
}

pub(crate) fn is_single_reference(name: &str) -> bool {
    is_reference(name) && !matches!(name, "cref" | "Cref")
}

pub(crate) fn reference_accepts_star(name: &str) -> bool {
    matches!(
        name,
        "ref"
            | "Ref"
            | "pageref"
            | "autopageref"
            | "autoref"
            | "nameref"
            | "vref"
            | "cref"
            | "Cref"
    )
}

pub(crate) fn is_counter_reference(name: &str) -> bool {
    matches!(
        name,
        "setcounterref" | "addtocounterref" | "setcounterpageref" | "addtocounterpageref"
    )
}

pub(crate) fn counter_reference_label_start(source: &str, command: &Command) -> Option<usize> {
    if !is_counter_reference(&command.name) {
        return None;
    }
    let (_, end) = group(source, skip_space(source, command.span.end), b'{', b'}')?;
    Some(skip_space(source, end))
}

pub(crate) fn counter_reference_label(source: &str, command: &Command) -> Option<Argument> {
    group(
        source,
        counter_reference_label_start(source, command)?,
        b'{',
        b'}',
    )
    .map(|(argument, _)| argument)
}

/// These commands keep their label outside the ordinary first required argument.
pub(crate) fn has_separate_label_argument(name: &str) -> bool {
    is_counter_reference(name) || name == "hyperref"
}

pub(crate) fn separate_label_argument_start(
    source: &str,
    command: &Command,
) -> Option<(usize, u8)> {
    if matches!(command.name.as_str(), "hyperref" | "hyperdef") {
        let start = skip_space(source, command.span.end);
        if source.as_bytes().get(start) != Some(&b'[') {
            return None;
        }
        if source.as_bytes().get(start + 1) == Some(&b'{') {
            if group(source, start + 1, b'{', b'}').is_some_and(|(_, end)| {
                end < source.len() && source.as_bytes().get(end) != Some(&b']')
            }) {
                return None;
            }
            Some((start + 1, b'{'))
        } else {
            Some((start, b'['))
        }
    } else {
        counter_reference_label_start(source, command).map(|start| (start, b'{'))
    }
}

pub(crate) fn separate_label_argument(source: &str, command: &Command) -> Option<Argument> {
    if !matches!(command.name.as_str(), "hyperref" | "hyperdef") {
        return counter_reference_label(source, command);
    }
    let outer_start = skip_space(source, command.span.end);
    let (outer, _) = group(source, outer_start, b'[', b']')?;
    if let Some((mut inner, end)) = group(source, outer.span.start, b'{', b'}') {
        if end == outer.span.end {
            inner.outer_span = outer.outer_span;
            return Some(inner);
        }
    }
    Some(outer)
}

pub(crate) fn label_name_text(source: &str, command: &Command, name: &str) -> String {
    if matches!(command.name.as_str(), "hyperref" | "hyperdef")
        && name.contains(['[', ']'])
        && separate_label_argument_start(source, command).is_some_and(|(_, open)| open == b'[')
    {
        format!("{{{name}}}")
    } else {
        name.into()
    }
}

pub(crate) fn label_replacement_text(
    text: &Text,
    commands: &[Command],
    range: tower_lsp::lsp_types::Range,
    name: &str,
) -> String {
    if !name.contains(['[', ']']) {
        return name.into();
    }
    commands
        .iter()
        .find(|command| {
            matches!(command.name.as_str(), "hyperref" | "hyperdef")
                && separate_label_argument(&text.source, command).is_some_and(|argument| {
                    text.range(argument.span.start, argument.span.end) == range
                })
        })
        .map_or_else(
            || name.into(),
            |command| label_name_text(&text.source, command, name),
        )
}

pub(crate) fn is_static_key(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().any(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '\\' | '{'
                        | '}'
                        | ','
                        | '%'
                        | '#'
                        | '$'
                        | '&'
                        | '^'
                        | '~'
                        | '"'
                        | '('
                        | ')'
                        | '='
                )
        })
}

pub fn file_extensions(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "input" => &["tex", "ltx", "sty", "cls", "def", "clo"],
        "include" | "includeonly" | "subfile" => &["tex"],
        "bibliography" | "addbibresource" => &["bib"],
        "bibliographystyle" => &["bst"],
        "usepackage" | "RequirePackage" => &["sty"],
        "documentclass" | "LoadClass" => &["cls"],
        "includegraphics" => &["pdf", "png", "jpg", "jpeg", "eps", "svg"],
        _ => return None,
    })
}

/// A literal filename that can be replaced without introducing TeX tokens.
pub(crate) fn is_literal_file_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8192
        && !value.starts_with(' ')
        && !value.ends_with(' ')
        && !value.contains("  ")
        && !value.chars().any(|ch| ch.is_whitespace() && ch != ' ')
        && !value.contains([
            '\\', '{', '}', '%', '#', '$', '&', '^', '~', '"', '\r', '\n', '\0',
        ])
}

pub fn file_list(name: &str) -> bool {
    matches!(
        name,
        "bibliography" | "usepackage" | "RequirePackage" | "includeonly"
    )
}

/// Completion can offer explicit source extensions that TeX does not infer.
pub(crate) fn inferred_file_extensions(name: &str) -> Option<&'static [&'static str]> {
    let extensions = file_extensions(name)?;
    Some(if name == "input" {
        &extensions[..1]
    } else {
        extensions
    })
}

pub(crate) fn file_lookup_names<'a>(
    command: &str,
    value: &'a str,
) -> impl Iterator<Item = std::borrow::Cow<'a, str>> {
    let recognized = std::path::Path::new(value)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["tex", "sty", "cls", "def", "clo", "fd", "aux", "bbl", "ldf"].contains(&extension)
        });
    let append = !value.is_empty()
        && match command {
            "include" | "includeonly" => !value.ends_with(".tex"),
            "input" => !recognized,
            _ => false,
        };
    let preferred = if append {
        std::borrow::Cow::Owned(format!("{value}.tex"))
    } else {
        std::borrow::Cow::Borrowed(value)
    };
    std::iter::once(preferred)
        .chain((command == "input" && append).then_some(std::borrow::Cow::Borrowed(value)))
}

fn declares_command(name: &str) -> bool {
    declares_register(name)
        || declares_font_switch(name)
        || declares_paired_delimiter(name)
        || declares_document_command(name)
        || declares_command_copy(name)
        || matches!(
            name,
            "newcommand"
                | "renewcommand"
                | "providecommand"
                | "newrobustcmd"
                | "renewrobustcmd"
                | "providerobustcmd"
                | "DeclareMathOperator"
                | "DeclareTextFontCommand"
                | "DeclareMathAlphabet"
                | "DeclareSymbolFontAlphabet"
                | "DeclareRobustCommand"
                | "letcs"
        )
}

pub(crate) fn declares_document_command(name: &str) -> bool {
    matches!(
        name,
        "NewDocumentCommand"
            | "RenewDocumentCommand"
            | "ProvideDocumentCommand"
            | "DeclareDocumentCommand"
            | "NewExpandableDocumentCommand"
            | "RenewExpandableDocumentCommand"
            | "ProvideExpandableDocumentCommand"
            | "DeclareExpandableDocumentCommand"
    )
}

pub(crate) fn declares_command_copy(name: &str) -> bool {
    matches!(
        name,
        "NewCommandCopy" | "RenewCommandCopy" | "DeclareCommandCopy"
    )
}

pub(crate) fn declares_font_switch(name: &str) -> bool {
    matches!(
        name,
        "newfontfamily"
            | "renewfontfamily"
            | "providefontfamily"
            | "setfontfamily"
            | "newfontface"
            | "renewfontface"
            | "providefontface"
            | "setfontface"
    )
}

pub(crate) fn declares_paired_delimiter(name: &str) -> bool {
    matches!(
        name,
        "DeclarePairedDelimiter" | "DeclarePairedDelimiterX" | "DeclarePairedDelimiterXPP"
    )
}

pub(crate) fn declares_register(name: &str) -> bool {
    matches!(
        name,
        "newlength"
            | "newsavebox"
            | "newfont"
            | "newcount"
            | "newdimen"
            | "newskip"
            | "newmuskip"
            | "newtoks"
            | "newbox"
            | "newread"
            | "newwrite"
            | "newinsert"
            | "newfam"
            | "chardef"
            | "mathchardef"
            | "countdef"
            | "dimendef"
            | "skipdef"
            | "muskipdef"
            | "toksdef"
            | "font"
            | "DeclareFixedFont"
            | "DeclareMathSymbol"
    )
}

pub fn declares_environment(name: &str) -> bool {
    declares_environment_copy(name)
        || matches!(
            name,
            "newenvironment"
                | "renewenvironment"
                | "provideenvironment"
                | "NewDocumentEnvironment"
                | "RenewDocumentEnvironment"
                | "ProvideDocumentEnvironment"
                | "DeclareDocumentEnvironment"
                | "NewEnviron"
                | "RenewEnviron"
                | "newtheorem"
        )
}

pub(crate) fn declares_environment_copy(name: &str) -> bool {
    matches!(
        name,
        "NewEnvironmentCopy" | "RenewEnvironmentCopy" | "DeclareEnvironmentCopy"
    )
}

pub(crate) fn environment_copy_source_start(source: &str, command: &Command) -> Option<usize> {
    if !declares_environment_copy(&command.name) {
        return None;
    }
    let argument = command.argument.as_ref()?;
    Some(skip_space(source, argument.outer_span.end))
}

pub(crate) fn environment_copy_source(source: &str, command: &Command) -> Option<Argument> {
    let start = environment_copy_source_start(source, command)?;
    group(source, start, b'{', b'}').map(|(argument, _)| argument)
}

pub(crate) fn is_verbatim_environment(name: &str) -> bool {
    matches!(
        name,
        "verbatim" | "verbatim*" | "lstlisting" | "minted" | "comment" | "Verbatim"
    )
}

pub(crate) fn active_static_argument(
    source: &str,
    start: usize,
    open: u8,
    cursor: usize,
) -> Option<Range<usize>> {
    if source.as_bytes().get(start) != Some(&open) || cursor <= start {
        return None;
    }
    let close = if open == b'{' { b'}' } else { b']' };
    let span = if let Some((argument, _)) = group(source, start, open, close) {
        argument.span
    } else {
        start + 1..source.len()
    };
    if cursor > span.end {
        return None;
    }
    let prefix = source[span.start..cursor].trim_start();
    if !prefix.is_empty() && !is_static_key(prefix) {
        return None;
    }
    let value = &source[span.clone()];
    let value = if source.as_bytes().get(span.end) == Some(&close) {
        value
    } else {
        value.split(['\r', '\n']).next().unwrap_or_default()
    }
    .trim();
    if !value.is_empty() && !is_static_key(value) {
        return None;
    }
    let start = cursor - prefix.len();
    let end = cursor
        + source[cursor..span.end]
            .chars()
            .take_while(|ch| {
                !ch.is_whitespace()
                    && !matches!(
                        ch,
                        '{' | '}' | '[' | ']' | '\\' | '%' | ',' | '#' | '$' | '&' | '^' | '~'
                    )
            })
            .map(char::len_utf8)
            .sum::<usize>();
    Some(start..end)
}

/// Selecting a name completion replaces the entire argument content. Outer
/// whitespace in that content belongs to the old name, not the selected one.
pub(crate) fn active_name_argument(
    source: &str,
    start: usize,
    open: u8,
    cursor: usize,
) -> Option<Range<usize>> {
    active_static_argument(source, start, open, cursor)?;
    let close = if open == b'{' { b'}' } else { b']' };
    let span = group(source, start, open, close).map_or_else(
        || {
            let end = source[cursor..]
                .find(['\r', '\n'])
                .map_or(source.len(), |length| cursor + length);
            start + 1..end
        },
        |(argument, _)| argument.span,
    );
    let value = source[span.clone()].trim();
    (value.is_empty() || is_static_key(value)).then_some(span)
}

/// LSP requires a completion's main edit to stay on the request line.
pub(crate) fn name_completion_ranges(
    text: &Text,
    span: Range<usize>,
    cursor: usize,
) -> Option<(tower_lsp::lsp_types::Range, Vec<TextEdit>)> {
    let line_start = text.source[..cursor].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text.source[cursor..]
        .find(['\r', '\n'])
        .map_or(text.source.len(), |length| cursor + length);
    let main = span.start.max(line_start)..span.end.min(line_end);
    // An empty request line between two deletion ranges cannot anchor a
    // nonoverlapping main insertion. Keep that ambiguous completion unresolved.
    if main.is_empty() && (span.start < main.start || main.end < span.end) {
        return None;
    }
    let additional = [span.start..main.start, main.end..span.end]
        .into_iter()
        .filter(|range| !range.is_empty())
        .map(|range| TextEdit {
            range: text.range(range.start, range.end),
            new_text: String::new(),
        })
        .collect();
    Some((text.range(main.start, main.end), additional))
}

pub(crate) fn control_word(source: &str, start: usize) -> Option<Range<usize>> {
    if source.as_bytes().get(start) != Some(&b'\\') {
        return None;
    }
    let length = source[start + 1..]
        .bytes()
        .take_while(|byte| byte.is_ascii_alphabetic() || *byte == b'@')
        .count();
    (length > 0).then_some(start..start + 1 + length)
}

pub(crate) fn conditional_declaration(
    source: &str,
    commands: &[Command],
    command: &Command,
) -> Option<Range<usize>> {
    if declares_boolean(&command.name) {
        let span = trimmed_span(source, command.argument.as_ref()?.span.clone());
        return is_static_key(&source[span.clone()]).then_some(span);
    }
    if command.name != "newif" {
        return None;
    }
    let span = control_word(source, skip_space(source, command.span.end))?;
    if let Some(name) = source[span.start + 1..span.end].strip_prefix("if") {
        return (!name.is_empty()).then_some(span.start + 3..span.end);
    }
    let index = commands
        .binary_search_by_key(&command.span.start, |command| command.span.start)
        .ok()?;
    let previous = commands.get(index.checked_sub(1)?)?;
    let target = commands.get(index + 1)?;
    if previous.name != "expandafter"
        || skip_space(source, previous.span.end) != command.span.start
        || skip_space(source, command.span.end) != target.span.start
    {
        return None;
    }
    let span = literal_csname(source, commands, index + 1)?;
    let name = source[span.clone()].strip_prefix("if")?;
    (!name.is_empty()).then_some(span.start + 2..span.end)
}

pub(crate) fn declares_boolean(name: &str) -> bool {
    matches!(
        name,
        "newboolean" | "provideboolean" | "newbool" | "providebool"
    )
}

pub(crate) fn boolean_reference(name: &str) -> bool {
    matches!(
        name,
        "boolean" | "setboolean" | "setbool" | "booltrue" | "boolfalse" | "ifbool" | "notbool"
    )
}

pub(crate) fn declares_toggle(name: &str) -> bool {
    matches!(name, "newtoggle" | "providetoggle")
}

pub(crate) fn toggle_reference(name: &str) -> bool {
    matches!(
        name,
        "settoggle" | "toggletrue" | "togglefalse" | "iftoggle" | "nottoggle"
    )
}

pub fn is_citation(name: &str) -> bool {
    matches!(
        name,
        "cite"
            | "citep"
            | "citet"
            | "citeauthor"
            | "citeyear"
            | "citealt"
            | "citealp"
            | "citenum"
            | "citeyearpar"
            | "citefullauthor"
            | "Citet"
            | "Citep"
            | "Citealt"
            | "Citealp"
            | "Citeauthor"
            | "citetalias"
            | "citepalias"
            | "defcitealias"
            | "autocite"
            | "parencite"
            | "textcite"
            | "nocite"
            | "footcite"
    )
}

/// Alias definitions name one key and do not themselves request a citation.
pub(crate) fn is_single_citation(name: &str) -> bool {
    name == "defcitealias"
}

pub(crate) fn warns_on_undefined_citation(name: &str) -> bool {
    is_citation(name) && !is_single_citation(name)
}

pub(crate) fn citation_accepts_star(name: &str) -> bool {
    is_citation(name)
        && !matches!(
            name,
            "nocite"
                | "citenum"
                | "citeyearpar"
                | "citefullauthor"
                | "citetalias"
                | "citepalias"
                | "defcitealias"
        )
}

pub(crate) fn citation_accepts_notes(name: &str) -> bool {
    is_citation(name) && !matches!(name, "nocite" | "citenum" | "defcitealias")
}

pub fn section_level(name: &str) -> Option<u32> {
    match name {
        "part" => Some(0),
        "chapter" => Some(1),
        "section" => Some(2),
        "subsection" => Some(3),
        "subsubsection" => Some(4),
        "paragraph" => Some(5),
        "subparagraph" => Some(6),
        _ => None,
    }
}

impl Syntax {
    fn fix(
        &mut self,
        text: &Text,
        span: Range<usize>,
        message: String,
        title: String,
        edit: TextEdit,
    ) {
        let mut diagnostic = diagnostic(text, span, message);
        diagnostic.code = Some(NumberOrString::String("syntax".into()));
        self.diagnostics.push(diagnostic.clone());
        self.fixes.push(QuickFix {
            title,
            diagnostic,
            edit,
            additional_edits: vec![],
        });
    }

    pub fn latex(text: &Text) -> Self {
        let source = &text.source;
        let bytes = source.as_bytes();
        let mut result = Self::default();
        let mut braces = Vec::new();
        let mut environments: Vec<(String, Range<usize>)> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'%' => {
                    let start = i;
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                    result.ignored.push(start..i);
                    result.comments.push(start..i);
                }
                b'{' => {
                    braces.push(i);
                    i += 1;
                }
                b'}' => {
                    if braces.pop().is_none() {
                        result.fix(
                            text,
                            i..i + 1,
                            "Unexpected closing brace".into(),
                            "Remove unexpected `}`".into(),
                            TextEdit {
                                range: text.range(i, i + 1),
                                new_text: String::new(),
                            },
                        );
                    }
                    i += 1;
                }
                b'\\' => {
                    let start = i;
                    i += 1;
                    let name_start = i;
                    while i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'@') {
                        i += 1;
                    }
                    if name_start == i {
                        if i < bytes.len() {
                            i += source[i..].chars().next().unwrap().len_utf8();
                        }
                        continue;
                    }
                    let name = &source[name_start..i];
                    let command_end = i;
                    let starred = name != "input" && bytes.get(i) == Some(&b'*');
                    if starred {
                        i += 1;
                    }
                    if matches!(name, "verb" | "lstinline") {
                        let raw_start = i;
                        if name == "lstinline" {
                            if let Some((_, end)) = group(source, i, b'[', b']') {
                                i = end;
                            }
                        }
                        if let Some(delimiter) =
                            source[i..].chars().next().filter(|ch| !ch.is_whitespace())
                        {
                            i += delimiter.len_utf8();
                            while i < bytes.len() && bytes[i] != b'\n' {
                                let ch = source[i..].chars().next().unwrap();
                                i += ch.len_utf8();
                                if ch == delimiter {
                                    break;
                                }
                            }
                        }
                        result.ignored.push(raw_start..i);
                        continue;
                    }
                    let mut arg_start = skip_space(source, i);
                    if !starred
                        && (reference_accepts_star(name) || citation_accepts_star(name))
                        && bytes.get(arg_start) == Some(&b'*')
                    {
                        arg_start = skip_space(source, arg_start + 1);
                    }
                    let invalid_citation = is_citation(name)
                        && (starred && !citation_accepts_star(name)
                            || bytes.get(arg_start) == Some(&b'[')
                                && !citation_accepts_notes(name));
                    if name != "input" {
                        while let Some((_, end)) = group(source, arg_start, b'[', b']') {
                            arg_start = skip_space(source, end);
                        }
                    }
                    let url_argument = matches!(name, "url" | "href")
                        || name == "hyperref"
                            && bytes.get(skip_space(source, command_end)) == Some(&b'{');
                    let parsed = group_mode(source, arg_start, b'{', b'}', !url_argument)
                        .or_else(|| {
                            (name == "input" && bytes.get(arg_start) != Some(&b'{'))
                                .then(|| input_argument(source, arg_start))
                                .flatten()
                        })
                        .filter(|_| !invalid_citation);
                    let argument = parsed.as_ref().map(|(a, _)| a.clone());
                    let command = Command {
                        name: name.into(),
                        span: start..command_end,
                        argument,
                    };
                    if let Some((declaration, entry)) = crate::anchors::entry(source, &command) {
                        if declaration {
                            result.anchors.push(entry);
                        } else {
                            result.anchor_references.push(entry);
                        }
                    }
                    if name == "hyperdef" {
                        if let Some(argument) = separate_label_argument(source, &command)
                            .filter(|argument| is_static_key(&argument.value))
                        {
                            result.labels.push(Entry {
                                name: argument.value,
                                span: argument.span,
                                detail: "Defined with \\hyperdef".into(),
                                is_variable: false,
                            });
                        }
                    }
                    let declaration = if declares_command(name) {
                        if let Some(argument) = &command.argument {
                            let span = trimmed_span(source, argument.span.clone());
                            control_word(source, span.start).filter(|word| *word == span)
                        } else {
                            control_word(source, arg_start)
                        }
                    } else if matches!(name, "def" | "gdef" | "edef" | "xdef" | "let") {
                        control_word(source, arg_start)
                    } else {
                        None
                    };
                    let declaration = declaration.filter(|span| {
                        // expandafter expands a csname target before the
                        // declaration sees it; it does not define `csname`.
                        !(source[span.clone()] == *"\\csname"
                            && skip_space(source, command.span.end) == span.start
                            && result.commands.last().is_some_and(|previous| {
                                previous.name == "expandafter"
                                    && skip_space(source, previous.span.end) == command.span.start
                            }))
                    });
                    if let Some(span) = declaration {
                        result.definitions.push(Entry {
                            name: source[span.start + 1..span.end].into(),
                            span,
                            detail: format!("Defined with \\{name}"),
                            is_variable: declares_register(name),
                        });
                    }
                    if let Some(arg) = &command.argument {
                        let entry = || Entry {
                            name: arg.value.trim().into(),
                            span: trimmed_span(source, arg.span.clone()),
                            detail: format!("\\{name}"),
                            is_variable: false,
                        };
                        if declares_environment(name) && is_static_key(&arg.value) {
                            result.environments.push(Entry {
                                detail: format!("Defined with \\{name}"),
                                ..entry()
                            });
                        }
                        match name {
                            "newcounter" | "newaliascnt" if is_static_key(&arg.value) => {
                                result.counters.push(Entry {
                                    is_variable: true,
                                    ..entry()
                                });
                            }
                            "label" if is_static_key(&arg.value) => result.labels.push(entry()),
                            "bibitem" => result.citations.push(entry()),
                            "begin" => {
                                let env = arg.value.as_str();
                                if is_verbatim_environment(env) {
                                    let body_start = parsed.as_ref().unwrap().1;
                                    let ending = format!("\\end{{{env}}}");
                                    if let Some(relative) = source[body_start..].find(&ending) {
                                        i = body_start + relative + ending.len();
                                    } else {
                                        result.fix(
                                            text,
                                            arg.span.clone(),
                                            format!("Unclosed environment `{env}`"),
                                            format!("Close environment `{env}`"),
                                            environment_end(text, env),
                                        );
                                        i = source.len();
                                    }
                                    result.ignored.push(body_start..i);
                                    if env == "comment" {
                                        result.comments.push(body_start..i);
                                    }
                                } else {
                                    environments.push((env.into(), arg.span.clone()));
                                }
                            }
                            "end" => {
                                let env = arg.value.as_str();
                                if let Some(index) =
                                    environments.iter().rposition(|(name, _)| name == env)
                                {
                                    for (name, span) in environments.drain(index + 1..) {
                                        result.fix(text, span, format!("Unclosed environment `{name}` before \\end{{{env}}}"), format!("Close environment `{name}` before `{env}`"), TextEdit { range:text.range(start, start), new_text:format!("\\end{{{name}}}{}", text.newline()) });
                                    }
                                    environments.pop();
                                } else {
                                    result.fix(
                                        text,
                                        arg.span.clone(),
                                        format!("Unexpected \\end{{{env}}}"),
                                        format!("Remove unexpected \\end{{{env}}}"),
                                        TextEdit {
                                            range: text.range(start, arg.span.end + 1),
                                            new_text: String::new(),
                                        },
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                    if url_argument {
                        if let Some((argument, end)) = parsed {
                            result.ignored.push(argument.span);
                            i = end;
                        }
                    }
                    result.commands.push(command);
                }
                _ => i += 1,
            }
        }
        let comment_at_end = result
            .ignored
            .last()
            .is_some_and(|span| span.end == source.len() && source[span.clone()].starts_with('%'));
        for start in braces {
            result.fix(
                text,
                start..start + 1,
                "Unclosed brace".into(),
                "Insert missing `}`".into(),
                TextEdit {
                    range: text.range(source.len(), source.len()),
                    new_text: if comment_at_end {
                        format!("{}}}", text.newline())
                    } else {
                        "}".into()
                    },
                },
            );
        }
        for (name, span) in environments {
            result.fix(
                text,
                span,
                format!("Unclosed environment `{name}`"),
                format!("Close environment `{name}`"),
                environment_end(text, &name),
            );
        }
        for (index, commands) in result.commands.windows(3).enumerate() {
            let [expand, declaration, target] = commands else {
                unreachable!()
            };
            if expand.name == "expandafter"
                && (declares_command(&declaration.name)
                    || matches!(
                        declaration.name.as_str(),
                        "def" | "gdef" | "edef" | "xdef" | "let"
                    ))
                && skip_space(source, expand.span.end) == declaration.span.start
                && skip_space(source, declaration.span.end) == target.span.start
            {
                if let Some(span) = literal_csname(source, &result.commands, index + 2) {
                    result.definitions.push(Entry {
                        name: source[span.clone()].into(),
                        span,
                        detail: format!("Defined with \\{}", declaration.name),
                        is_variable: declares_register(&declaration.name),
                    });
                }
            }
        }
        for command in &result.commands {
            if let Some(span) = crate::named_commands::declaration(source, command) {
                result.definitions.push(Entry {
                    name: source[span.clone()].into(),
                    span,
                    detail: format!("Defined with \\{}", command.name),
                    is_variable: false,
                });
            }
            if declares_toggle(&command.name) {
                if let Some(argument) = &command.argument {
                    let span = trimmed_span(source, argument.span.clone());
                    if is_static_key(&source[span.clone()]) {
                        result.toggles.push(Entry {
                            name: source[span.clone()].into(),
                            span,
                            detail: format!("Defined with \\{}", command.name),
                            is_variable: false,
                        });
                    }
                }
            }
            if let Some(span) = conditional_declaration(source, &result.commands, command) {
                result.conditionals.push(Entry {
                    name: source[span.clone()].into(),
                    span,
                    detail: format!("Defined with \\{}", command.name),
                    is_variable: false,
                });
            }
        }
        result.definitions.sort_by_key(|entry| entry.span.start);
        result
    }

    pub fn bibtex(text: &Text) -> Self {
        let mut result = Self::default();
        let source = &text.source;
        let bytes = source.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                let start = i;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                result.ignored.push(start..i);
                result.comments.push(start..i);
                continue;
            }
            if bytes[i] != b'@' {
                i += 1;
                continue;
            }
            i += 1;
            let kind_start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            let kind = &source[kind_start..i];
            i = skip_space(source, i);
            let Some(&open @ (b'{' | b'(')) = bytes.get(i) else {
                continue;
            };
            let close = if open == b'{' { b'}' } else { b')' };
            let content = i + 1;
            // Skip the whole entry so @ signs in field values never become entries.
            let (end, closed) = bib_entry_end(source, i, close);
            if kind.eq_ignore_ascii_case("comment") {
                result.ignored.push(kind_start - 1..end);
                result.comments.push(kind_start - 1..end);
            }
            if !matches!(
                kind.to_ascii_lowercase().as_str(),
                "comment" | "string" | "preamble"
            ) {
                let key_start = skip_space(source, content);
                let mut key_end = key_start;
                while key_end < end && !matches!(bytes[key_end], b',' | b'}' | b')' | b'\n') {
                    key_end += 1;
                }
                let span = trimmed_span(source, key_start..key_end);
                if !span.is_empty() && bytes.get(key_end) == Some(&b',') {
                    result.citations.push(Entry {
                        name: source[span.clone()].into(),
                        span,
                        detail: format!("BibTeX {kind}"),
                        is_variable: false,
                    });
                    let body_end = if closed { end - 1 } else { end };
                    result.bibliography_fields(source, key_end + 1, body_end);
                }
            }
            i = end;
        }
        result
    }

    fn bibliography_fields(&mut self, source: &str, mut i: usize, end: usize) {
        let bytes = source.as_bytes();
        while i < end {
            i = skip_space(source, i);
            if bytes.get(i) == Some(&b',') {
                i += 1;
                continue;
            }
            let start = i;
            while i < end && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'-')) {
                i += 1;
            }
            if start == i {
                break;
            }
            let name = source[start..i].to_ascii_lowercase();
            i = skip_space(source, i);
            if bytes.get(i) != Some(&b'=') {
                break;
            }
            let start = skip_space(source, i + 1);
            let value_end = bib_value_end(source, start, end);
            i = value_end + 1;
            if !matches!(
                name.as_str(),
                "crossref" | "xref" | "xdata" | "related" | "entryset"
            ) {
                continue;
            }
            let Some(argument) = bib_literal(source, start, value_end) else {
                continue;
            };
            // Macro concatenations, escaped values and nested TeX groups cannot
            // be renamed safely without evaluating BibTeX's string language.
            if argument.value.contains(['\\', '{', '}', '%', '#', '"']) {
                continue;
            }
            let multiple = matches!(name.as_str(), "xdata" | "related" | "entryset");
            let mut offset = argument.span.start;
            for key in if multiple {
                argument.value.split(',').collect::<Vec<_>>()
            } else {
                vec![argument.value.as_str()]
            } {
                let span = trimmed_span(source, offset..offset + key.len());
                if !span.is_empty() {
                    self.citation_references.push(Entry {
                        name: source[span.clone()].into(),
                        span,
                        detail: format!("BibTeX {name} reference"),
                        is_variable: false,
                    });
                }
                offset += key.len() + 1;
            }
            self.citation_fields.push(CitationField { name, argument });
        }
    }

    pub fn is_ignored(&self, offset: usize) -> bool {
        self.ignored
            .iter()
            .any(|span| span.start <= offset && offset < span.end)
    }
}

fn environment_end(text: &Text, name: &str) -> TextEdit {
    let source = &text.source;
    let newline = text.newline();
    let prefix = if source.ends_with('\n') { "" } else { newline };
    TextEdit {
        range: text.range(source.len(), source.len()),
        new_text: format!("{prefix}\\end{{{name}}}{newline}"),
    }
}

fn bib_entry_end(source: &str, start: usize, close: u8) -> (usize, bool) {
    let bytes = source.as_bytes();
    let mut depth = 0u32;
    let mut quoted = false;
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i = (i + 2).min(bytes.len());
                continue;
            }
            b'%' if depth == 0 && !quoted => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'"' if depth == 0 => quoted = !quoted,
            b'{' => depth += 1,
            b'}' if depth > 0 => depth -= 1,
            b if b == close && depth == 0 && !quoted => return (i + 1, true),
            _ => {}
        }
        i += 1;
    }
    (bytes.len(), false)
}

fn bib_value_end(source: &str, start: usize, end: usize) -> usize {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut i = start;
    while i < end {
        match bytes[i] {
            b'\\' => {
                i = (i + 2).min(end);
                continue;
            }
            b'%' if depth == 0 && !quoted => {
                while i < end && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'"' if depth == 0 => quoted = !quoted,
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 && !quoted => return i,
            _ => {}
        }
        i += 1;
    }
    end
}

fn bib_literal(source: &str, start: usize, end: usize) -> Option<Argument> {
    let bytes = source.as_bytes();
    let (argument, after) = match bytes.get(start)? {
        b'{' => group_mode(source, start, b'{', b'}', false).or_else(|| {
            (end == source.len()).then(|| {
                (
                    Argument {
                        value: source[start + 1..end].into(),
                        span: start + 1..end,
                        outer_span: start..end,
                    },
                    end,
                )
            })
        })?,
        b'"' => {
            let mut i = start + 1;
            let mut depth = 0usize;
            while i < end {
                match bytes[i] {
                    b'\\' => {
                        i = (i + 2).min(end);
                        continue;
                    }
                    b'{' => depth += 1,
                    b'}' => depth = depth.saturating_sub(1),
                    b'"' if depth == 0 => break,
                    _ => {}
                }
                i += 1;
            }
            if bytes.get(i) != Some(&b'"') && end != source.len() {
                return None;
            }
            (
                Argument {
                    value: source[start + 1..i].into(),
                    span: start + 1..i,
                    outer_span: start..(i + 1).min(end),
                },
                (i + 1).min(end),
            )
        }
        _ => return None,
    };
    (after <= end && skip_space(source, after) >= end).then_some(argument)
}

pub fn trimmed_span(source: &str, span: Range<usize>) -> Range<usize> {
    let value = &source[span.clone()];
    let start = span.start + value.len() - value.trim_start().len();
    start..start + value.trim().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_escapes_and_verbatim_are_opaque() {
        let text = Text::new(
            r"\% {ok} % } \label{fake}
\verb|{\label{fake}}| \begin{verbatim}
} \label{fake}
\end{verbatim}
\label{real} \section[short]{A {nested} title}"
                .into(),
        );
        let syntax = Syntax::latex(&text);
        assert!(syntax.diagnostics.is_empty(), "{:?}", syntax.diagnostics);
        assert_eq!(syntax.labels.len(), 1);
        assert_eq!(syntax.labels[0].name, "real");
        assert_eq!(
            syntax
                .commands
                .last()
                .unwrap()
                .argument
                .as_ref()
                .unwrap()
                .value,
            "A {nested} title"
        );
    }

    #[test]
    fn malformed_environments_and_braces_recover() {
        let syntax = Syntax::latex(&Text::new(r"\begin{a}\begin{b}\end{a}} {".into()));
        assert_eq!(syntax.diagnostics.len(), 3);
        assert!(syntax
            .diagnostics
            .iter()
            .any(|d| d.message.contains("environment `b`")));
    }

    #[test]
    fn bibtex_skips_fields_and_special_entries() {
        let text = Text::new("@comment{not,a key}\n@article{中文2026, title={An @book{fake, title} example}}\n@book(other, title=\"Text\")".into());
        let syntax = Syntax::bibtex(&text);
        assert_eq!(
            syntax
                .citations
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["中文2026", "other"]
        );
    }

    #[test]
    fn inline_options_and_unicode_delimiters_preserve_following_commands() {
        let text = Text::new(
            r"\lstinline[language=TeX]|{\label{fake}}| \verb§}\label{fake}§ \label{real}".into(),
        );
        let syntax = Syntax::latex(&text);
        assert!(syntax.diagnostics.is_empty());
        assert_eq!(syntax.labels.len(), 1);
        assert_eq!(syntax.labels[0].name, "real");
        for span in syntax.ignored {
            assert!(text.source.is_char_boundary(span.start));
            assert!(text.source.is_char_boundary(span.end));
        }
    }

    #[test]
    fn incomplete_unicode_inputs_keep_valid_spans() {
        let fragments = [
            "中", "😀", "é", "\\", "{", "}", "%", "\n", "[", "]", "@", "(", ")", ",", "\"", " ",
            "label", "begin", "end", "verb", "article", "input", "\\input ",
        ];
        let mut seed = 12345u32;
        for _ in 0..512 {
            let mut source = String::new();
            for _ in 0..64 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                source.push_str(fragments[seed as usize % fragments.len()]);
            }
            let text = Text::new(source);
            for syntax in [Syntax::latex(&text), Syntax::bibtex(&text)] {
                let spans = syntax
                    .commands
                    .iter()
                    .map(|cmd| cmd.span.clone())
                    .chain(
                        syntax
                            .commands
                            .iter()
                            .filter_map(|cmd| cmd.argument.as_ref())
                            .flat_map(|arg| [arg.span.clone(), arg.outer_span.clone()]),
                    )
                    .chain(
                        syntax
                            .labels
                            .iter()
                            .chain(&syntax.citations)
                            .chain(&syntax.citation_references)
                            .chain(&syntax.definitions)
                            .chain(&syntax.environments)
                            .chain(&syntax.counters)
                            .chain(&syntax.conditionals)
                            .chain(&syntax.toggles)
                            .map(|entry| entry.span.clone()),
                    )
                    .chain(syntax.ignored.iter().cloned());
                for span in spans {
                    assert!(text.source.get(span).is_some());
                }
                for diagnostic in syntax.diagnostics {
                    assert!(diagnostic.range.start <= diagnostic.range.end);
                    assert!(text.offset(diagnostic.range.start).is_some());
                    assert!(text.offset(diagnostic.range.end).is_some());
                }
            }
        }
    }
}
