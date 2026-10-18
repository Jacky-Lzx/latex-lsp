use std::ops::Range;

use crate::text::Text;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, TextEdit};

#[derive(Clone, Debug)]
pub struct Argument {
    pub value: String,
    pub span: Range<usize>,
}

#[derive(Clone, Debug)]
pub struct Command {
    pub name: String,
    pub span: Range<usize>,
    pub argument: Option<Argument>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub span: Range<usize>,
    pub detail: String,
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
    pub citations: Vec<Entry>,
    pub citation_fields: Vec<CitationField>,
    pub citation_references: Vec<Entry>,
    pub definitions: Vec<Entry>,
    pub environments: Vec<Entry>,
    pub diagnostics: Vec<Diagnostic>,
    pub ignored: Vec<Range<usize>>,
    pub fixes: Vec<QuickFix>,
}

#[derive(Clone, Debug)]
pub struct QuickFix {
    pub title: String,
    pub diagnostic: Diagnostic,
    pub edit: TextEdit,
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

fn skip_space(source: &str, mut i: usize) -> usize {
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
fn group(source: &str, i: usize, open: u8, close: u8) -> Option<(Argument, usize)> {
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
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some((
                        Argument {
                            value: source[start..i].into(),
                            span: start..i,
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

pub fn is_reference(name: &str) -> bool {
    matches!(
        name,
        "ref" | "eqref" | "pageref" | "autoref" | "cref" | "Cref" | "vref" | "nameref"
    )
}

pub fn file_extensions(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "input" => &["tex", "ltx", "sty", "cls", "def", "clo"],
        "include" | "subfile" => &["tex"],
        "bibliography" | "addbibresource" => &["bib"],
        "usepackage" | "RequirePackage" => &["sty"],
        "documentclass" | "LoadClass" => &["cls"],
        "includegraphics" => &["pdf", "png", "jpg", "jpeg", "eps", "svg"],
        _ => return None,
    })
}

pub fn file_list(name: &str) -> bool {
    matches!(name, "bibliography" | "usepackage" | "RequirePackage")
}

fn declares_command(name: &str) -> bool {
    matches!(
        name,
        "newcommand"
            | "renewcommand"
            | "providecommand"
            | "DeclareMathOperator"
            | "DeclareRobustCommand"
            | "NewDocumentCommand"
            | "RenewDocumentCommand"
            | "ProvideDocumentCommand"
            | "DeclareDocumentCommand"
    )
}

pub fn declares_environment(name: &str) -> bool {
    matches!(
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

fn control_word(source: &str, start: usize) -> Option<Range<usize>> {
    if source.as_bytes().get(start) != Some(&b'\\') {
        return None;
    }
    let length = source[start + 1..]
        .bytes()
        .take_while(|byte| byte.is_ascii_alphabetic() || *byte == b'@')
        .count();
    (length > 0).then_some(start..start + 1 + length)
}

pub fn is_citation(name: &str) -> bool {
    matches!(
        name,
        "cite"
            | "citep"
            | "citet"
            | "citeauthor"
            | "citeyear"
            | "autocite"
            | "parencite"
            | "textcite"
            | "nocite"
            | "footcite"
    )
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
                    if bytes.get(i) == Some(&b'*') {
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
                    while let Some((_, end)) = group(source, arg_start, b'[', b']') {
                        arg_start = skip_space(source, end);
                    }
                    let url_argument = matches!(name, "url" | "href");
                    let parsed = group_mode(source, arg_start, b'{', b'}', !url_argument);
                    let argument = parsed.as_ref().map(|(a, _)| a.clone());
                    let command = Command {
                        name: name.into(),
                        span: start..command_end,
                        argument,
                    };
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
                    if let Some(span) = declaration {
                        result.definitions.push(Entry {
                            name: source[span.start + 1..span.end].into(),
                            span,
                            detail: format!("Defined with \\{name}"),
                        });
                    }
                    if let Some(arg) = &command.argument {
                        let entry = || Entry {
                            name: arg.value.trim().into(),
                            span: trimmed_span(source, arg.span.clone()),
                            detail: format!("\\{name}"),
                        };
                        if declares_environment(name) && !arg.value.trim().is_empty() {
                            result.environments.push(Entry {
                                detail: format!("Defined with \\{name}"),
                                ..entry()
                            });
                        }
                        match name {
                            "label" => result.labels.push(entry()),
                            "bibitem" => result.citations.push(entry()),
                            "begin" => {
                                let env = arg.value.trim();
                                if matches!(
                                    env,
                                    "verbatim"
                                        | "verbatim*"
                                        | "lstlisting"
                                        | "minted"
                                        | "comment"
                                        | "Verbatim"
                                ) {
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
                                } else {
                                    environments.push((env.into(), arg.span.clone()));
                                }
                            }
                            "end" => {
                                let env = arg.value.trim();
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
        result
    }

    pub fn bibtex(text: &Text) -> Self {
        let mut result = Self::default();
        let source = &text.source;
        let bytes = source.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
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
            "label", "begin", "end", "verb", "article",
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
                            .filter_map(|cmd| cmd.argument.as_ref().map(|arg| arg.span.clone())),
                    )
                    .chain(
                        syntax
                            .labels
                            .iter()
                            .chain(&syntax.citations)
                            .chain(&syntax.citation_references)
                            .chain(&syntax.definitions)
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
