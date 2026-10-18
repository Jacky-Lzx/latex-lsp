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
                    .map(|argument| argument.value.trim())
                    .filter(|name| syntax::is_static_key(name))
                {
                    names.insert(
                        name.into(),
                        format!("Defined with \\{} — {}", command.name, uri.path()),
                    );
                }
            }
        }
        for name in crate::environment::counters(doc) {
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

fn active(source: &str, start: usize, open: u8, cursor: usize) -> Option<Range<usize>> {
    if source.as_bytes().get(start) != Some(&open) || cursor <= start {
        return None;
    }
    let close = if open == b'{' { b'}' } else { b']' };
    let span = if let Some((argument, _)) = syntax::group(source, start, open, close) {
        argument.span
    } else {
        start + 1..source.len()
    };
    if cursor > span.end {
        return None;
    }
    let prefix = source[span.start..cursor].trim_start();
    if !prefix.is_empty() && !syntax::is_static_key(prefix) {
        return None;
    }
    let value = &source[span.clone()];
    let value = if source.as_bytes().get(span.end) == Some(&close) {
        value
    } else {
        value.split(['\r', '\n']).next().unwrap_or_default()
    }
    .trim();
    if !value.is_empty() && !syntax::is_static_key(value) {
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
        | "aliascntresetthe" => active(source, at, b'{', cursor),
        "counterwithin" | "counterwithout" | "numberwithin" | "@addtoreset"
        | "@removefromreset" => {
            if source.as_bytes().get(at) == Some(&b'[') {
                at = closed(source, at, b'[')?;
            }
            active(source, at, b'{', cursor)
                .or_else(|| active(source, closed(source, at, b'{')?, b'{', cursor))
        }
        "newcounter" => active(source, closed(source, at, b'{')?, b'[', cursor),
        "newaliascnt" => active(source, closed(source, at, b'{')?, b'{', cursor),
        "newtheorem" if !starred => {
            at = closed(source, at, b'{')?;
            if source.as_bytes().get(at) == Some(&b'[') {
                active(source, at, b'[', cursor)
            } else {
                active(source, closed(source, at, b'{')?, b'[', cursor)
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
