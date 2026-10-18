//! Linearly match grouped call arguments; do not execute macro definitions.
use crate::{
    catalog::COMMANDS,
    syntax::{self, Entry},
    workspace::{Document, Workspace},
};
use tower_lsp::lsp_types::*;

#[derive(Clone)]
enum Kind {
    Required,
    Optional,
    Star,
}

struct Parameter {
    kind: Kind,
    label: String,
}

struct Signature {
    name: String,
    parameters: Vec<Parameter>,
    documentation: String,
    starred: bool,
}

impl Signature {
    fn information(self, active: usize) -> SignatureHelp {
        let label = format!(
            "\\{}{}",
            self.name,
            self.parameters
                .iter()
                .map(|parameter| parameter.label.as_str())
                .collect::<String>()
        );
        SignatureHelp {
            signatures: vec![SignatureInformation {
                label,
                documentation: Some(Documentation::String(self.documentation)),
                parameters: Some(
                    self.parameters
                        .into_iter()
                        .map(|parameter| ParameterInformation {
                            label: ParameterLabel::Simple(parameter.label),
                            documentation: None,
                        })
                        .collect(),
                ),
                active_parameter: Some(active as u32),
            }],
            active_signature: Some(0),
            active_parameter: Some(active as u32),
        }
    }
}

fn builtin(name: &str) -> Option<Signature> {
    let (arguments, starred): (&[(&str, bool)], bool) = match name {
        "frac" | "dfrac" | "tfrac" | "binom" => {
            (&[("numerator", false), ("denominator", false)], false)
        }
        "sqrt" => (&[("degree", true), ("expression", false)], false),
        "href" => (&[("url", false), ("text", false)], false),
        "url" => (&[("url", false)], false),
        "includegraphics" => (&[("options", true), ("file", false)], true),
        "documentclass" | "usepackage" | "RequirePackage" => {
            (&[("options", true), ("name", false)], false)
        }
        "input" | "include" | "subfile" | "bibliography" | "bibliographystyle" => {
            (&[("file", false)], false)
        }
        "addbibresource" => (&[("options", true), ("file", false)], false),
        "part" | "chapter" | "section" | "subsection" | "subsubsection" | "paragraph"
        | "subparagraph" | "caption" => (&[("short title", true), ("title", false)], true),
        "footnote" => (&[("number", true), ("text", false)], false),
        "textbf" | "textit" | "texttt" | "emph" | "title" | "author" | "date" => {
            (&[("text", false)], false)
        }
        "begin" | "end" => (&[("environment", false)], false),
        "label" => (&[("key", false)], false),
        name if syntax::is_reference(name) => (&[("keys", false)], false),
        name if syntax::is_citation(name) && name != "nocite" => (
            &[
                ("note / prenote", true),
                ("postnote", true),
                ("keys", false),
            ],
            true,
        ),
        "nocite" => (&[("keys", false)], false),
        _ => return None,
    };
    let parameters = arguments
        .iter()
        .map(|(name, optional)| Parameter {
            kind: if *optional {
                Kind::Optional
            } else {
                Kind::Required
            },
            label: if *optional {
                format!("[{name}]")
            } else {
                format!("{{{name}}}")
            },
        })
        .collect();
    let documentation = COMMANDS
        .iter()
        .find(|(command, _, _)| *command == name)
        .map(|(_, _, documentation)| *documentation)
        .unwrap_or("LaTeX command arguments.")
        .into();
    Some(Signature {
        name: name.into(),
        parameters,
        documentation,
        starred,
    })
}

fn definition(uri: &Url, doc: &Document, entry: &Entry) -> Option<Signature> {
    let source = &doc.text.source;
    let declaration = entry.detail.strip_prefix("Defined with \\")?;
    let mut i = entry.span.end;
    if source.as_bytes().get(i) == Some(&b'}') {
        i += 1;
    }
    i = syntax::skip_space(source, i);
    let mut parameters = Vec::new();
    match declaration {
        "newcommand" | "renewcommand" | "providecommand" | "DeclareRobustCommand" => {
            let (count, end) = syntax::group(source, i, b'[', b']')?;
            let count: usize = count.value.trim().parse().ok()?;
            if !(1..=9).contains(&count) {
                return None;
            }
            i = syntax::skip_space(source, end);
            let default = syntax::group(source, i, b'[', b']').map(|(argument, _)| argument.value);
            for number in 1..=count {
                let optional = number == 1 && default.is_some();
                let label = if optional {
                    format!("[#1={}]", short(default.as_deref().unwrap()))
                } else {
                    format!("{{#{number}}}")
                };
                parameters.push(Parameter {
                    kind: if optional {
                        Kind::Optional
                    } else {
                        Kind::Required
                    },
                    label,
                });
            }
        }
        "NewDocumentCommand"
        | "RenewDocumentCommand"
        | "ProvideDocumentCommand"
        | "DeclareDocumentCommand" => {
            let (arguments, _) = syntax::group(source, i, b'{', b'}')?;
            let source = &arguments.value;
            let mut i = 0;
            while i < source.len() {
                i = syntax::skip_space(source, i);
                let Some(&kind) = source.as_bytes().get(i) else {
                    break;
                };
                i += 1;
                if matches!(kind, b'+' | b'!') {
                    continue;
                }
                let number = parameters.len() + 1;
                let (kind, label) = match kind {
                    b'm' => (Kind::Required, format!("{{#{number}}}")),
                    b'o' => (Kind::Optional, format!("[#{number}]")),
                    b'O' => {
                        i = syntax::skip_space(source, i);
                        let (default, end) = syntax::group(source, i, b'{', b'}')?;
                        i = end;
                        (
                            Kind::Optional,
                            format!("[#{number}={}]", short(&default.value)),
                        )
                    }
                    b's' => (Kind::Star, "*".into()),
                    _ => return None,
                };
                parameters.push(Parameter { kind, label });
                if parameters.len() > 9 {
                    return None;
                }
            }
        }
        "def" | "gdef" | "edef" | "xdef" => {
            while source.as_bytes().get(i) == Some(&b'#') {
                let number = parameters.len() + 1;
                if number > 9 || source.as_bytes().get(i + 1) != Some(&(b'0' + number as u8)) {
                    return None;
                }
                parameters.push(Parameter {
                    kind: Kind::Required,
                    label: format!("{{#{number}}}"),
                });
                i += 2;
            }
            if source.as_bytes().get(i) != Some(&b'{') {
                return None;
            }
        }
        _ => return None,
    }
    if parameters.is_empty() {
        return None;
    }
    Some(Signature {
        name: entry.name.clone(),
        parameters,
        documentation: format!(
            "{}\n{}:{}",
            entry.detail,
            uri.path(),
            doc.text.position(entry.span.start).line + 1
        ),
        starred: false,
    })
}

fn short(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect()
}

fn signature(workspace: &Workspace, name: &str) -> Option<Signature> {
    let mut definitions = workspace.documents.iter().flat_map(|(uri, doc)| {
        doc.syntax
            .definitions
            .iter()
            .filter(move |entry| entry.name == name)
            .map(move |entry| (uri, doc, entry))
    });
    if let Some((uri, doc, entry)) = definitions.next() {
        if definitions.next().is_some() {
            return None;
        }
        definition(uri, doc, entry)
    } else {
        builtin(name)
    }
}

fn group_end(doc: &Document, start: usize, cursor: usize, open: u8, close: u8) -> Option<usize> {
    let source = &doc.text.source;
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut braces = 0usize;
    let mut i = start + 1;
    // Parser-emitted ignored spans are ordered and do not overlap.
    let mut ignored = doc.syntax.ignored.partition_point(|span| span.end <= i);
    while i < cursor {
        while doc
            .syntax
            .ignored
            .get(ignored)
            .is_some_and(|span| span.end <= i)
        {
            ignored += 1;
        }
        if let Some(span) = doc.syntax.ignored.get(ignored) {
            if span.start <= i {
                i = span.end;
                continue;
            }
        }
        match bytes[i] {
            b'\\' => {
                i += 1;
                if i < source.len() {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
                continue;
            }
            b'{' if open != b'{' => braces += 1,
            b'}' if open != b'{' => braces = braces.saturating_sub(1),
            ch if ch == open && braces == 0 => depth += 1,
            ch if ch == close && braces == 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn active_parameter(
    doc: &Document,
    end: usize,
    cursor: usize,
    signature: &Signature,
) -> Option<usize> {
    let source = &doc.text.source;
    let mut i = end;
    if signature.starred && source.as_bytes().get(i) == Some(&b'*') {
        i += 1;
    }
    for (index, parameter) in signature.parameters.iter().enumerate() {
        i = syntax::skip_space(source, i);
        if i == cursor {
            match parameter.kind {
                Kind::Optional if source.as_bytes().get(i).is_some_and(|byte| *byte != b'[') => {
                    continue
                }
                Kind::Star if source.as_bytes().get(i).is_some_and(|byte| *byte != b'*') => {
                    continue
                }
                _ => {}
            }
        }
        if i >= cursor {
            return Some(index);
        }
        let (open, close) = match parameter.kind {
            Kind::Star => {
                if source.as_bytes().get(i) == Some(&b'*') {
                    i += 1;
                }
                continue;
            }
            Kind::Optional if source.as_bytes().get(i) != Some(&b'[') => continue,
            Kind::Optional => (b'[', b']'),
            Kind::Required if source.as_bytes().get(i) != Some(&b'{') => return None,
            Kind::Required => (b'{', b'}'),
        };
        match group_end(doc, i, cursor, open, close) {
            Some(end) => i = end,
            None => return Some(index),
        }
    }
    None
}

pub fn help(workspace: &Workspace, uri: &Url, position: Position) -> Option<SignatureHelp> {
    let doc = workspace.documents.get(uri)?;
    if doc.bibtex {
        return None;
    }
    let cursor = doc.text.offset(position)?;
    if doc.syntax.ignored.iter().any(|span| {
        span.start <= cursor.saturating_sub(1)
            && cursor.saturating_sub(1) < span.end
            && !doc.syntax.commands.iter().any(|command| {
                matches!(command.name.as_str(), "url" | "href")
                    && command
                        .argument
                        .as_ref()
                        .is_some_and(|argument| argument.span == *span)
            })
    }) {
        return None;
    }
    for command in doc
        .syntax
        .commands
        .iter()
        .rev()
        .filter(|command| command.span.end <= cursor)
        .take(128)
    {
        if let Some(signature) = signature(workspace, &command.name) {
            if let Some(active) = active_parameter(doc, command.span.end, cursor, &signature) {
                return Some(signature.information(active));
            }
        }
    }
    None
}
