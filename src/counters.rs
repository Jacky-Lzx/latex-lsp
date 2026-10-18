//! Static counter names and argument completion without evaluating TeX.
use crate::{
    syntax::{self, Command},
    workspace::{Document, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use tower_lsp::lsp_types::*;

pub(crate) const BUILTINS: &[&str] = &[
    "page",
    "part",
    "chapter",
    "section",
    "subsection",
    "subsubsection",
    "paragraph",
    "subparagraph",
    "equation",
    "figure",
    "table",
    "footnote",
    "mpfootnote",
    "enumi",
    "enumii",
    "enumiii",
    "enumiv",
    "secnumdepth",
    "tocdepth",
];

/// Static name selections and the complete argument tail of direct counter operations.
/// Names must be literal; values, format expressions and theorem titles stay intact.
pub(crate) fn invocations(source: &str, command: &Command) -> Vec<(Range<usize>, Range<usize>)> {
    let Some((arguments, tail)) = invocation_arguments(source, command) else {
        return vec![];
    };
    let names: Option<Vec<_>> = arguments
        .into_iter()
        .map(|argument| {
            let span = argument.span;
            syntax::is_static_key(&source[span.clone()]).then_some(span)
        })
        .collect();
    names
        .unwrap_or_default()
        .into_iter()
        .map(|span| (span, tail.clone()))
        .collect()
}

fn invocation_arguments(
    source: &str,
    command: &Command,
) -> Option<(Vec<syntax::Argument>, Range<usize>)> {
    let start = syntax::skip_space(source, command.span.end);
    let mut at = start;
    let resetting = matches!(
        command.name.as_str(),
        "counterwithin" | "counterwithout" | "numberwithin"
    );
    if resetting {
        if command.name != "numberwithin" && source.as_bytes().get(at) == Some(&b'*') {
            at = syntax::skip_space(source, at + 1);
        }
        if source.as_bytes().get(at) == Some(&b'[') {
            at = syntax::skip_space(source, syntax::group(source, at, b'[', b']')?.1);
        }
    }
    if !resetting
        && !matches!(
            command.name.as_str(),
            "setcounter"
                | "addtocounter"
                | "stepcounter"
                | "refstepcounter"
                | "usecounter"
                | "newcounter"
                | "newaliascnt"
                | "newtheorem"
        )
    {
        return None;
    }
    let (first, mut end) = syntax::group(source, at, b'{', b'}')?;
    let mut names = Vec::new();
    match command.name.as_str() {
        "setcounter" | "addtocounter" => {
            end = syntax::group(source, syntax::skip_space(source, end), b'{', b'}')?.1;
            names.push(first);
        }
        "stepcounter" | "refstepcounter" | "usecounter" => names.push(first),
        "counterwithin" | "counterwithout" | "numberwithin" => {
            let (second, after) =
                syntax::group(source, syntax::skip_space(source, end), b'{', b'}')?;
            end = after;
            names.extend([first, second]);
        }
        "newcounter" | "newaliascnt" | "newtheorem" => {
            if !syntax::is_static_key(&source[first.span]) {
                return None;
            }
            at = syntax::skip_space(source, end);
            let (name, after) = match command.name.as_str() {
                "newcounter" => syntax::group(source, at, b'[', b']')?,
                "newaliascnt" => syntax::group(source, at, b'{', b'}')?,
                _ if source.as_bytes().get(at) == Some(&b'[') => {
                    let (shared, after) = syntax::group(source, at, b'[', b']')?;
                    let (_, end) =
                        syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?;
                    (shared, end)
                }
                _ => {
                    let (_, after) = syntax::group(source, at, b'{', b'}')?;
                    syntax::group(source, syntax::skip_space(source, after), b'[', b']')?
                }
            };
            names.push(name);
            end = after;
        }
        _ => return None,
    }
    Some((names, start..end))
}

/// A spelling repair must not create a self alias or reset relationship.
pub(crate) fn replacement_conflicts(
    doc: &Document,
    range: tower_lsp::lsp_types::Range,
) -> BTreeSet<&str> {
    let source = &doc.text.source;
    let mut result = BTreeSet::new();
    for command in &doc.syntax.commands {
        let names = invocations(source, command);
        if !names
            .iter()
            .any(|(span, _)| doc.text.range(span.start, span.end) == range)
        {
            continue;
        }
        for (span, _) in names {
            if doc.text.range(span.start, span.end) != range {
                result.insert(&source[span]);
            }
        }
        if matches!(
            command.name.as_str(),
            "newcounter" | "newaliascnt" | "newtheorem"
        ) {
            if let Some(argument) = &command.argument {
                result.insert(&source[argument.span.clone()]);
            }
        }
    }
    result
}

fn names(workspace: &Workspace, scope: &BTreeSet<Url>) -> BTreeMap<String, String> {
    let mut names: BTreeMap<_, _> = BUILTINS
        .iter()
        .map(|name| ((*name).into(), "LaTeX counter".into()))
        .collect();
    for (uri, doc) in &workspace.documents {
        if !scope.contains(uri) {
            continue;
        }
        for command in &doc.syntax.commands {
            if matches!(command.name.as_str(), "newcounter" | "newaliascnt") {
                if let Some(name) = command
                    .argument
                    .as_ref()
                    .map(|argument| argument.value.as_str())
                    .filter(|name| syntax::is_static_key(name))
                {
                    names.insert(
                        name.into(),
                        format!("Defined with \\{} — {}", command.name, uri.path()),
                    );
                }
            }
        }
        for name in crate::generated::counters(doc) {
            if syntax::is_static_key(name) {
                names.insert(name.into(), format!("Theorem counter — {}", uri.path()));
            }
        }
    }
    names
}

fn closed(source: &str, start: usize, open: u8) -> Option<usize> {
    syntax::group(source, start, open, if open == b'{' { b'}' } else { b']' })
        .map(|(_, end)| syntax::skip_space(source, end))
}

fn context(doc: &Document, command: &Command, cursor: usize) -> Option<Range<usize>> {
    let source = &doc.text.source;
    let mut at = command.span.end;
    let starred = source.as_bytes().get(at) == Some(&b'*');
    if starred {
        at += 1;
    }
    at = syntax::skip_space(source, at);
    match command.name.as_str() {
        "setcounter" | "addtocounter" | "stepcounter" | "refstepcounter" | "usecounter"
        | "value" | "arabic" | "roman" | "Roman" | "alph" | "Alph" | "fnsymbol"
        | "aliascntresetthe" => syntax::active_static_argument(source, at, b'{', cursor),
        "counterwithin" | "counterwithout" | "numberwithin" | "@addtoreset"
        | "@removefromreset" => {
            if source.as_bytes().get(at) == Some(&b'[') {
                at = closed(source, at, b'[')?;
            }
            syntax::active_static_argument(source, at, b'{', cursor).or_else(|| {
                syntax::active_static_argument(source, closed(source, at, b'{')?, b'{', cursor)
            })
        }
        "newcounter" => {
            syntax::active_static_argument(source, closed(source, at, b'{')?, b'[', cursor)
        }
        "newaliascnt" => {
            syntax::active_static_argument(source, closed(source, at, b'{')?, b'{', cursor)
        }
        "newtheorem" if !starred => {
            at = closed(source, at, b'{')?;
            if source.as_bytes().get(at) == Some(&b'[') {
                syntax::active_static_argument(source, at, b'[', cursor)
            } else {
                syntax::active_static_argument(source, closed(source, at, b'{')?, b'[', cursor)
            }
        }
        _ => None,
    }
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &Url,
    cursor: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = workspace.documents.get(uri)?;
    let span = doc
        .syntax
        .commands
        .iter()
        .rev()
        .filter(|command| command.span.end <= cursor)
        .take(128)
        .find_map(|command| context(doc, command, cursor))?;
    let prefix = &doc.text.source[span.start..cursor];
    let range = doc.text.range(span.start, span.end);
    Some(
        names(workspace, scope)
            .into_iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, detail)| CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::VARIABLE),
                detail: Some(detail),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range,
                    new_text: name,
                })),
                ..Default::default()
            })
            .collect(),
    )
}
