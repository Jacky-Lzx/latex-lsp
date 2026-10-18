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

/// Name selection and the complete argument tail of a direct counter operation.
/// Keep value expressions intact and leave unbraced or computed names unresolved.
pub(crate) fn invocation(source: &str, command: &Command) -> Option<(Range<usize>, Range<usize>)> {
    let value = match command.name.as_str() {
        "setcounter" | "addtocounter" => true,
        "stepcounter" | "refstepcounter" | "usecounter" => false,
        _ => return None,
    };
    let (argument, mut end) = syntax::group(
        source,
        syntax::skip_space(source, command.span.end),
        b'{',
        b'}',
    )?;
    let span = argument.span;
    if !syntax::is_static_key(&source[span.clone()]) {
        return None;
    }
    if value {
        end = syntax::group(source, syntax::skip_space(source, end), b'{', b'}')?.1;
    }
    Some((span, argument.outer_span.start..end))
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
