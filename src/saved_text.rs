//! FancyVerb's saved text keys have their own namespaces and exact spelling.
use crate::{
    fancyvrb,
    syntax::{self, Argument, Command, Entry, Syntax},
    workspace::{Document, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range as Span,
};
use tower_lsp::{jsonrpc, lsp_types::*};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Saved,
    Moving,
}

fn role(command: &Command) -> Option<(Kind, bool)> {
    match command.name.as_str() {
        "SaveVerb" => Some((Kind::Saved, true)),
        "SaveGVerb" => Some((Kind::Moving, true)),
        "UseVerb" | "UseVerbatim" | "LUseVerbatim" | "BUseVerbatim" => Some((Kind::Saved, false)),
        "UseMVerb" | "pUseMVerb" => Some((Kind::Moving, false)),
        "begin"
            if command.argument.as_ref().is_some_and(|argument| {
                matches!(argument.value.as_str(), "SaveVerbatim" | "SaveVerbatim*")
            }) =>
        {
            Some((Kind::Saved, true))
        }
        _ => None,
    }
}

fn argument_start(source: &str, command: &Command) -> Option<usize> {
    syntax::file_name_argument_start(source, &command.name, command.span.end)
}

fn argument(source: &str, command: &Command) -> Option<Argument> {
    if command.name == "begin" {
        fancyvrb::saved_environment_argument(source, command.argument.as_ref()?.outer_span.end)
    } else {
        fancyvrb::name_argument(source, argument_start(source, command)?)
            .map(|(argument, _)| argument)
    }
}

/// A conservative literal token list: spaces and commas are meaningful, while
/// expansion, comments, grouping and TeX's active input syntax stay unresolved.
pub(crate) fn literal_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|ch| {
            !ch.is_control()
                && (!ch.is_whitespace() || ch == ' ')
                && !matches!(ch, '\\' | '{' | '}' | '%' | '#' | '~' | '^' | '$' | '&')
        })
}

fn command_reference(name: &str, span: Span<usize>) -> Entry {
    Entry {
        name: name.into(),
        span,
        detail: "Command referenced in a saved-text name".into(),
        is_variable: false,
    }
}

/// Name arguments expand control words but do not execute LaTeX declarations.
/// Literal conversion primitives consume data, which must remain untouched.
fn name_references(source: &str, span: Span<usize>) -> (Vec<Entry>, Vec<Entry>) {
    let mut result = Vec::new();
    let mut counters = Vec::new();
    let mut at = span.start;
    while at < span.end {
        match source.as_bytes()[at] {
            b'%' => {
                at = source[at..span.end]
                    .find('\n')
                    .map_or(span.end, |newline| at + newline + 1);
            }
            b'\\' => {
                let Some(word) =
                    syntax::control_word(source, at).filter(|word| word.end <= span.end)
                else {
                    at += 1;
                    if at < span.end {
                        at += source[at..].chars().next().unwrap().len_utf8()
                    }
                    continue;
                };
                let name = &source[word.start + 1..word.end];
                result.push(command_reference(name, word.clone()));
                at = word.end;
                match name {
                    "value" | "arabic" | "roman" | "Roman" | "alph" | "Alph" | "fnsymbol" => {
                        let start = syntax::skip_space(source, at);
                        if let Some((argument, _)) =
                            fancyvrb::name_argument(source, start).filter(|(argument, end)| {
                                *end <= span.end
                                    && syntax::is_static_key(&argument.value)
                                    && (argument.span != argument.outer_span
                                        || argument.value.is_ascii())
                            })
                        {
                            counters.push(Entry {
                                name: argument.value,
                                span: argument.span,
                                detail: format!(
                                    "Counter expanded with \\{name} in a saved-text name"
                                ),
                                is_variable: false,
                            });
                        }
                    }
                    "detokenize" => {
                        let open = syntax::skip_space(source, at);
                        if let Some((_, end)) = syntax::group(source, open, b'{', b'}')
                            .filter(|(_, end)| *end <= span.end)
                        {
                            at = end;
                        }
                    }
                    "string" => {
                        // Unlike detokenize, string reads exactly one token,
                        // including a brace rather than a complete group.
                        at = syntax::skip_space(source, at).min(span.end);
                        if at < span.end {
                            at = syntax::control_word(source, at)
                                .map_or_else(
                                    || {
                                        let first = source[at..].chars().next().unwrap();
                                        let after = at + first.len_utf8();
                                        if first == '\\' && after < span.end {
                                            after
                                                + source[after..].chars().next().unwrap().len_utf8()
                                        } else {
                                            after
                                        }
                                    },
                                    |word| word.end,
                                )
                                .min(span.end);
                        }
                    }
                    "csname" => {
                        if let Some(offset) = source[at..span.end].find('\\') {
                            let close = at + offset;
                            if syntax::control_word(source, close).is_some_and(|word| {
                                word.end <= span.end && &source[word] == "\\endcsname"
                            }) {
                                // TeX skips the control word's delimiter, but
                                // spaces before endcsname belong to the name.
                                let name_span = syntax::skip_space(source, at).min(close)..close;
                                if syntax::is_static_key(&source[name_span.clone()]) {
                                    result.push(command_reference(
                                        &source[name_span.clone()],
                                        name_span,
                                    ));
                                }
                            }
                        }
                    }
                    "csuse" => {
                        let open = syntax::skip_space(source, at);
                        if let Some((argument, _)) = syntax::group(source, open, b'{', b'}')
                            .filter(|(_, end)| *end <= span.end)
                        {
                            if syntax::is_static_key(&argument.value) {
                                result.push(command_reference(&argument.value, argument.span));
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => at += source[at..].chars().next().unwrap().len_utf8(),
        }
    }
    (result, counters)
}

pub(crate) fn index(source: &str, parsed: &mut Syntax) {
    for command in &parsed.commands {
        let Some((kind, declaration)) = role(command) else {
            continue;
        };
        let Some(argument) = argument(source, command) else {
            continue;
        };
        let (commands, counters) = name_references(source, argument.span.clone());
        parsed.name_command_references.extend(commands);
        parsed.name_counter_references.extend(counters);
        if !literal_name(&argument.value) {
            continue;
        }
        let entry = Entry {
            name: argument.value,
            span: argument.span,
            detail: if declaration {
                format!(
                    "Saved text defined with \\{}",
                    if command.name == "begin" {
                        "SaveVerbatim"
                    } else {
                        &command.name
                    }
                )
            } else {
                format!("Saved text used with \\{}", command.name)
            },
            is_variable: false,
        };
        match (kind, declaration) {
            (Kind::Saved, true) => parsed.saved_text.push(entry),
            (Kind::Saved, false) => parsed.saved_text_references.push(entry),
            (Kind::Moving, true) => parsed.moving_text.push(entry),
            (Kind::Moving, false) => parsed.moving_text_references.push(entry),
        }
    }
}

pub(crate) fn declarations(doc: &Document, kind: Kind) -> &[Entry] {
    match kind {
        Kind::Saved => &doc.syntax.saved_text,
        Kind::Moving => &doc.syntax.moving_text,
    }
}

pub(crate) fn references(doc: &Document, kind: Kind) -> &[Entry] {
    match kind {
        Kind::Saved => &doc.syntax.saved_text_references,
        Kind::Moving => &doc.syntax.moving_text_references,
    }
}

fn executable_catcodes(options: &str) -> bool {
    syntax::key_list_segments(options, 0..options.len())
        .into_iter()
        .any(|span| {
            let segment = &options[span];
            let (key, value) = segment.split_once('=').unwrap_or((segment, ""));
            match key.trim() {
                "commandchars" => !matches!(value.trim(), "none" | "{none}"),
                "codes" | "codes*" | "defineactive" => !matches!(value.trim(), "" | "{}"),
                key => key.contains('\\'),
            }
        })
}

fn executable_call(source: &str, command: &Command) -> bool {
    if command.name == "fvset" {
        return command
            .argument
            .as_ref()
            .is_none_or(|argument| executable_catcodes(&argument.value));
    }
    if command.name == "begin"
        && command
            .argument
            .as_ref()
            .is_some_and(|argument| fancyvrb::is_environment(&argument.value))
    {
        return fancyvrb::environment_options(
            source,
            command.argument.as_ref().unwrap().outer_span.end,
        )
        .is_some_and(|options| executable_catcodes(&options.value));
    }
    if !(fancyvrb::is_inline(&command.name)
        || fancyvrb::is_use(&command.name)
        || fancyvrb::is_input(&command.name))
    {
        return false;
    }
    let mut at = syntax::skip_space(source, command.span.end);
    if source.as_bytes().get(at) == Some(&b'*') {
        at = syntax::skip_space(source, at + 1)
    }
    source.as_bytes().get(at) == Some(&b'[')
        && syntax::bracket_argument(source, at)
            .is_none_or(|(options, _)| executable_catcodes(&options.value))
}

fn known_command(name: &str) -> bool {
    fancyvrb::is_inline(name)
        || fancyvrb::is_use(name)
        || fancyvrb::is_input(name)
        || matches!(name, "SaveVerbatim" | "SaveVerbatim*")
}

pub(crate) fn validate(
    workspace: &Workspace,
    scope: &BTreeSet<Url>,
    kind: Kind,
) -> jsonrpc::Result<()> {
    for (uri, doc) in &workspace.documents {
        if !scope.contains(uri) {
            continue;
        }
        let customized = doc.syntax.definitions.iter().any(|entry| {
            matches!(
                entry.name.as_str(),
                "SaveVerb"
                    | "SaveGVerb"
                    | "SaveVerbatim"
                    | "endSaveVerbatim"
                    | "fvset"
                    | "UseVerb"
                    | "UseVerbatim"
                    | "LUseVerbatim"
                    | "BUseVerbatim"
                    | "UseMVerb"
                    | "pUseMVerb"
            ) || entry.name.starts_with("FV@")
                || entry.name.starts_with("FVC@")
                || entry.name.starts_with("KV@FV@")
        }) || doc
            .syntax
            .environments
            .iter()
            .any(|entry| matches!(entry.name.as_str(), "SaveVerbatim" | "SaveVerbatim*"));
        if customized {
            return Err(jsonrpc::Error::invalid_params(
                "Saved-text rename cannot resolve redefined FancyVerb commands",
            ));
        }
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            let indirect = command.name == "csuse"
                && command
                    .argument
                    .as_ref()
                    .is_some_and(|argument| known_command(&argument.value))
                || command.name == "csname"
                    && syntax::literal_csname(&doc.text.source, &doc.syntax.commands, index)
                        .is_some_and(|span| known_command(&doc.text.source[span]));
            if matches!(
                command.name.as_str(),
                "SaveMVerb"
                    | "CustomVerbatimCommand"
                    | "RecustomVerbatimCommand"
                    | "CustomVerbatimEnvironment"
                    | "RecustomVerbatimEnvironment"
            ) || command.name.starts_with("FV@")
                || command.name.starts_with("FVC@")
                || command.name.starts_with("KV@FV@")
                || matches!(command.name.as_str(), "setkeys" | "define@key")
                    && command.argument.as_ref().is_none_or(|argument| {
                        argument.value.split(',').any(|family| {
                            family.trim() == "FV" || !syntax::is_static_key(family.trim())
                        })
                    })
                || command.name == "csname"
                    && syntax::literal_csname(&doc.text.source, &doc.syntax.commands, index)
                        .is_some_and(|span| doc.text.source[span].starts_with("FV@"))
                || indirect
            {
                return Err(jsonrpc::Error::invalid_params("Saved-text rename cannot resolve moving definitions or customized FancyVerb internals"));
            }
            if executable_call(&doc.text.source, command) {
                return Err(jsonrpc::Error::invalid_params(
                    "Saved-text rename cannot resolve executable FancyVerb catcode settings",
                ));
            }
            if role(command).is_some_and(|(entry_kind, _)| entry_kind == kind)
                && !argument(&doc.text.source, command)
                    .is_some_and(|argument| literal_name(&argument.value))
            {
                return Err(jsonrpc::Error::invalid_params(
                    "Saved-text rename requires literal names in every call in this project",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn replacement_edit(doc: &Document, range: Range, new_name: &str) -> TextEdit {
    for command in &doc.syntax.commands {
        if role(command).is_none() {
            continue;
        }
        let Some(argument) = argument(&doc.text.source, command) else {
            continue;
        };
        if doc.text.range(argument.span.start, argument.span.end) == range {
            return TextEdit {
                range,
                new_text: if argument.span == argument.outer_span {
                    format!("{{{new_name}}}")
                } else {
                    new_name.into()
                },
            };
        }
    }
    TextEdit {
        range,
        new_text: new_name.into(),
    }
}

pub(crate) fn completion(
    workspace: &Workspace,
    uri: &Url,
    cursor: usize,
    scope: &BTreeSet<Url>,
) -> Option<Vec<CompletionItem>> {
    let doc = workspace.documents.get(uri)?;
    let source = &doc.text.source;
    let (kind, span) = doc
        .syntax
        .commands
        .iter()
        .rev()
        .filter(|command| command.span.end <= cursor)
        .find_map(|command| {
            let (kind, declaration) = role(command)?;
            if declaration {
                return None;
            }
            let at = argument_start(source, command)?;
            let span = if source.as_bytes().get(at) == Some(&b'{') {
                syntax::group(source, at, b'{', b'}')
                    .map_or(at + 1..source.len(), |(argument, _)| argument.span)
            } else {
                argument(source, command)?.span
            };
            if cursor < span.start || cursor > span.end {
                return None;
            }
            let value = &source[span.clone()];
            (value.is_empty() || literal_name(value)).then_some((kind, span))
        })?;
    let prefix = &source[span.start..cursor];
    let mut names = BTreeMap::new();
    for (uri, doc) in &workspace.documents {
        if !scope.contains(uri) {
            continue;
        }
        for entry in declarations(doc, kind) {
            if entry.name.starts_with(prefix) {
                names
                    .entry(entry.name.clone())
                    .or_insert_with(|| entry.detail.clone());
            }
        }
    }
    Some(
        names
            .into_iter()
            .map(|(name, detail)| CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::CONSTANT),
                detail: Some(detail),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: doc.text.range(span.start, span.end),
                    new_text: if source.as_bytes().get(span.start.saturating_sub(1)) == Some(&b'{')
                    {
                        name
                    } else {
                        format!("{{{name}}}")
                    },
                })),
                ..Default::default()
            })
            .collect(),
    )
}
