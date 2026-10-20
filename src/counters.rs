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

/// Read literal names through the optional readers used by counter declarations.
/// Recognize explicit protection without expanding arbitrary TeX commands.
pub(crate) fn optional_argument(
    source: &str,
    start: usize,
    passes: usize,
) -> Option<(syntax::Argument, usize)> {
    let (outer, after) = syntax::bracket_argument(source, start)?;
    let mut argument = outer.clone();
    let mut wrapped = 0;
    for _ in 0..passes {
        let Some((mut inner, end)) = syntax::group(source, argument.span.start, b'{', b'}') else {
            break;
        };
        if end != argument.span.end {
            break;
        }
        inner.outer_span = outer.outer_span.clone();
        argument = inner;
        wrapped += 1;
    }
    let mut literal = false;
    if let Some(value) = argument.value.strip_prefix("\\detokenize") {
        let start = argument.span.end - value.len();
        if let Some((mut inner, end)) = syntax::group(source, start, b'{', b'}') {
            if end == argument.span.end && syntax::is_static_key(&inner.value) {
                inner.outer_span = outer.outer_span.clone();
                argument = inner;
                literal = true;
            }
        }
    }
    if passes == 2 && wrapped < 2 && argument.value.contains(']') && !literal {
        return Some((outer, after));
    }
    Some((argument, after))
}

fn optional_field(source: &str, command: &Command) -> Option<(syntax::Argument, usize)> {
    let first = command.argument.as_ref()?;
    if !syntax::is_static_key(&first.value) {
        return None;
    }
    let at = syntax::skip_space(source, first.outer_span.end);
    let (start, passes) = match command.name.as_str() {
        "newcounter" => (at, 1),
        "newtheorem"
            if source
                .as_bytes()
                .get(syntax::skip_space(source, command.span.end))
                != Some(&b'*') =>
        {
            if source.as_bytes().get(at) == Some(&b'[') {
                (at, 1)
            } else {
                let (_, end) = syntax::group(source, at, b'{', b'}')?;
                (syntax::skip_space(source, end), 2)
            }
        }
        _ => return None,
    };
    optional_argument(source, start, passes).map(|(argument, _)| (argument, passes))
}

fn protected_name(name: &str, passes: usize, wrapped: usize) -> String {
    // Extra braces alter the generated parent-number formatter. A literal
    // token list survives both readers while retaining the original numbering.
    if passes == 2 && wrapped <= passes && name.contains(']') {
        return format!("\\detokenize{{{name}}}");
    }
    let extra = if name.contains(']') {
        passes.saturating_sub(wrapped)
    } else {
        0
    };
    format!("{}{name}{}", "{".repeat(extra), "}".repeat(extra))
}

fn replacement_span(
    source: &str,
    argument: &syntax::Argument,
    passes: usize,
) -> (Range<usize>, usize) {
    if passes == 2 && source[argument.outer_span.start + 1..].starts_with("{{") {
        // Keeping two outer groups makes LaTeX generate \\the{parent} rather
        // than \\theparent. Replace the inner group as well as its name.
        return (
            argument.outer_span.start + 2..argument.outer_span.end - 2,
            1,
        );
    }
    (
        argument.span.clone(),
        argument.span.start - argument.outer_span.start - 1,
    )
}

pub(crate) fn replacement_edit(
    doc: &Document,
    range: tower_lsp::lsp_types::Range,
    name: &str,
) -> TextEdit {
    if doc.syntax.name_counter_references.iter().any(|entry| {
        doc.text.range(entry.span.start, entry.span.end) == range
            && doc
                .text
                .source
                .as_bytes()
                .get(entry.span.start.saturating_sub(1))
                != Some(&b'{')
    }) {
        return TextEdit {
            range,
            new_text: format!("{{{name}}}"),
        };
    }
    for command in &doc.syntax.commands {
        if let Some((argument, passes)) = optional_field(&doc.text.source, command) {
            if doc.text.range(argument.span.start, argument.span.end) == range {
                let (span, wrapped) = replacement_span(&doc.text.source, &argument, passes);
                return TextEdit {
                    range: doc.text.range(span.start, span.end),
                    new_text: protected_name(name, passes, wrapped),
                };
            }
        }
    }
    TextEdit {
        range,
        new_text: name.into(),
    }
}

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
                | "setcounterref"
                | "addtocounterref"
                | "setcounterpageref"
                | "addtocounterpageref"
        )
    {
        return None;
    }
    let (first, mut end) = syntax::group(source, at, b'{', b'}')?;
    let mut names = Vec::new();
    match command.name.as_str() {
        "setcounter"
        | "addtocounter"
        | "setcounterref"
        | "addtocounterref"
        | "setcounterpageref"
        | "addtocounterpageref" => {
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
                "newcounter" => optional_argument(source, at, 1)?,
                "newaliascnt" => syntax::group(source, at, b'{', b'}')?,
                _ if source.as_bytes().get(at) == Some(&b'[') => {
                    let (shared, after) = optional_argument(source, at, 1)?;
                    let (_, end) =
                        syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?;
                    (shared, end)
                }
                _ => {
                    let (_, after) = syntax::group(source, at, b'{', b'}')?;
                    optional_argument(source, syntax::skip_space(source, after), 2)?
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

struct CompletionContext {
    span: Range<usize>,
    edit_span: Range<usize>,
    passes: usize,
    wrapped: usize,
}

fn optional_context(
    source: &str,
    start: usize,
    cursor: usize,
    passes: usize,
) -> Option<CompletionContext> {
    if let Some((argument, _)) = optional_argument(source, start, passes) {
        if cursor < argument.span.start || cursor > argument.span.end {
            return None;
        }
        let value = source[argument.span.clone()].trim();
        let prefix = source[argument.span.start..cursor].trim_start();
        if (!value.is_empty() && !syntax::is_static_key(value))
            || (!prefix.is_empty() && !syntax::is_static_key(prefix))
        {
            return None;
        }
        let (edit_span, wrapped) = replacement_span(source, &argument, passes);
        return Some(CompletionContext {
            wrapped,
            edit_span,
            span: argument.span,
            passes,
        });
    }
    if source.as_bytes().get(start) != Some(&b'[') {
        return None;
    }
    let mut at = start;
    let mut wrapped = 0;
    while wrapped < passes && source.as_bytes().get(at + 1) == Some(&b'{') {
        at += 1;
        wrapped += 1;
    }
    let span =
        syntax::active_label_argument(source, at, if wrapped == 0 { b'[' } else { b'{' }, cursor)?;
    let edit_span = if passes == 2 && wrapped == 2 {
        wrapped = 1;
        span.start - 1..span.end + usize::from(source.as_bytes().get(span.end) == Some(&b'}'))
    } else {
        span.clone()
    };
    Some(CompletionContext {
        span,
        edit_span,
        passes,
        wrapped,
    })
}

fn context(doc: &Document, command: &Command, cursor: usize) -> Option<CompletionContext> {
    let source = &doc.text.source;
    let mut at = command.span.end;
    let starred = source.as_bytes().get(at) == Some(&b'*');
    if starred && syntax::is_counter_reference(&command.name) {
        return None;
    }
    if starred {
        at += 1;
    }
    at = syntax::skip_space(source, at);
    let span = match command.name.as_str() {
        "setcounter"
        | "addtocounter"
        | "setcounterref"
        | "addtocounterref"
        | "setcounterpageref"
        | "addtocounterpageref"
        | "stepcounter"
        | "refstepcounter"
        | "usecounter"
        | "value"
        | "arabic"
        | "roman"
        | "Roman"
        | "alph"
        | "Alph"
        | "fnsymbol"
        | "aliascntresetthe" => syntax::active_name_argument(source, at, b'{', cursor),
        "counterwithin" | "counterwithout" | "numberwithin" | "@addtoreset"
        | "@removefromreset" => {
            if source.as_bytes().get(at) == Some(&b'[') {
                at = closed(source, at, b'[')?;
            }
            syntax::active_name_argument(source, at, b'{', cursor).or_else(|| {
                syntax::active_name_argument(source, closed(source, at, b'{')?, b'{', cursor)
            })
        }
        "newcounter" => {
            return optional_context(source, closed(source, at, b'{')?, cursor, 1);
        }
        "newaliascnt" => {
            syntax::active_name_argument(source, closed(source, at, b'{')?, b'{', cursor)
        }
        "newtheorem" if !starred => {
            at = closed(source, at, b'{')?;
            if source.as_bytes().get(at) == Some(&b'[') {
                return optional_context(source, at, cursor, 1);
            } else {
                return optional_context(source, closed(source, at, b'{')?, cursor, 2);
            }
        }
        _ => None,
    }?;
    Some(CompletionContext {
        edit_span: span.clone(),
        span,
        passes: 0,
        wrapped: 0,
    })
}

pub(crate) fn is_formatter(name: &str) -> bool {
    matches!(
        name,
        "value" | "arabic" | "roman" | "Roman" | "alph" | "Alph" | "fnsymbol"
    )
}

pub(crate) fn saved_name_completion(
    workspace: &Workspace,
    uri: &Url,
    cursor: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = workspace.documents.get(uri)?;
    let (context, bare) = doc
        .syntax
        .name_command_references
        .iter()
        .rev()
        .filter(|entry| {
            entry.span.end <= cursor
                && is_formatter(&entry.name)
                && doc.text.source[entry.span.clone()].starts_with('\\')
        })
        .take(128)
        .find_map(|entry| {
            if let Some(context) = context(
                doc,
                &Command {
                    name: entry.name.clone(),
                    span: entry.span.clone(),
                    argument: None,
                },
                cursor,
            ) {
                return Some((context, false));
            }
            let start = syntax::skip_space(&doc.text.source, entry.span.end);
            let operand = doc.syntax.name_counter_references.iter().find(|operand| {
                operand.span.start == start
                    && operand.span.end == start + 1
                    && operand.name.is_ascii()
                    && operand.span.start <= cursor
                    && cursor <= operand.span.end
            })?;
            Some((
                CompletionContext {
                    span: operand.span.clone(),
                    edit_span: operand.span.clone(),
                    passes: 0,
                    wrapped: 0,
                },
                true,
            ))
        })?;
    completion_items(workspace, doc, cursor, scope, context, bare)
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &Url,
    cursor: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = workspace.documents.get(uri)?;
    let context = doc
        .syntax
        .commands
        .iter()
        .rev()
        .filter(|command| command.span.end <= cursor)
        .take(128)
        .find_map(|command| context(doc, command, cursor))?;
    completion_items(workspace, doc, cursor, scope, context, false)
}

fn completion_items(
    workspace: &Workspace,
    doc: &Document,
    cursor: usize,
    scope: &BTreeSet<Url>,
    context: CompletionContext,
    bare: bool,
) -> Option<Vec<CompletionItem>> {
    let prefix = doc.text.source[context.span.start..cursor].trim_start();
    let (range, additional_edits) =
        syntax::name_completion_ranges(&doc.text, context.edit_span, cursor)?;
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
                    new_text: if bare {
                        format!("{{{name}}}")
                    } else {
                        protected_name(&name, context.passes, context.wrapped)
                    },
                })),
                additional_text_edits: (!additional_edits.is_empty())
                    .then(|| additional_edits.clone()),
                ..Default::default()
            })
            .collect(),
    )
}
