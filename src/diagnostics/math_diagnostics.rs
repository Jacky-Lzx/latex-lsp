//! Conservative, opt-in math checks using static project command meanings.
use crate::{
    language::signature::CommandSignatures,
    parser::syntax,
    project::workspace::{Document, DocumentKind, Workspace},
};
use lsp_types::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range as Span,
};

const MAX_DIAGNOSTICS: usize = 128;
const SOURCE_LOADERS: &[&str] = &[
    "input",
    "include",
    "subfile",
    "InputIfFileExists",
    "loadglsentries",
    "import",
    "subimport",
    "inputfrom",
    "subinputfrom",
    "includefrom",
    "subincludefrom",
];

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Inline,
    Display,
}
impl Mode {
    fn description(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Display => "display",
        }
    }
}
struct Open {
    mode: Mode,
    span: Span<usize>,
}

#[derive(Default, PartialEq)]
struct Semantics {
    operands: BTreeSet<usize>,
    dollars: BTreeSet<usize>,
    markers: BTreeMap<usize, (usize, Mode, bool)>,
}
impl Semantics {
    fn new(doc: &Document, signatures: &CommandSignatures<'_>) -> Option<Self> {
        let source = &doc.text.source;
        let mut operands = signatures.known_argument_tokens(source, &doc.syntax);
        let literals: Vec<_> = source
            .char_indices()
            .filter_map(|(at, ch)| {
                (ch == '$'
                    || ch == '\\'
                        && source
                            .as_bytes()
                            .get(at + 1)
                            .is_some_and(|next| matches!(next, b'(' | b')' | b'[' | b']')))
                .then_some(at)
            })
            .collect();
        // Literal argument queries revisit the command stream. Bound their
        // combined work before asking about punctuation in a large source.
        if literals
            .len()
            .saturating_mul(doc.syntax.commands.len().max(1))
            > 4_000_000
        {
            return None;
        }
        let mut dollars = BTreeSet::new();
        for at in literals {
            if signatures.known_literal_is_argument(source, &doc.syntax, at) {
                if source.as_bytes()[at] == b'$' {
                    dollars.insert(at);
                } else {
                    operands.insert(at);
                }
            }
        }
        let declarations = doc
            .syntax
            .definitions
            .iter()
            .map(|entry| (entry.span.start, entry.span.end))
            .collect();
        let mut markers = BTreeMap::new();
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            if operands.contains(&command.span.start)
                || doc.syntax.control_operands.contains(&command.span.start)
            {
                continue;
            }
            let (name, span) = signatures
                .named_invocation(source, &doc.syntax, index, &operands, &declarations)
                .unwrap_or((&command.name, command.span.clone()));
            let Some(delimiter) = ["(", ")", "[", "]"]
                .into_iter()
                .find(|delimiter| signatures.invokes_builtin(name, delimiter))
            else {
                continue;
            };
            markers.insert(
                span.start,
                (
                    span.end,
                    if matches!(delimiter, "(" | ")") {
                        Mode::Inline
                    } else {
                        Mode::Display
                    },
                    matches!(delimiter, "(" | "["),
                ),
            );
        }
        Some(Self {
            operands,
            dollars,
            markers,
        })
    }
}

#[derive(Default)]
struct Context {
    continuation: bool,
    unsafe_catcodes: bool,
    boundaries: BTreeMap<usize, usize>,
    potential_inputs: BTreeSet<String>,
    semantics: Option<Semantics>,
}
#[derive(Default)]
pub(crate) struct Index {
    diagnostics: BTreeMap<Url, Vec<Diagnostic>>,
}
impl Index {
    pub(crate) fn new(workspace: &Workspace, requested: &BTreeSet<Url>) -> Self {
        let enabled: BTreeSet<_> = requested
            .iter()
            .filter(|uri| {
                workspace
                    .documents
                    .get(*uri)
                    .is_some_and(|doc| doc.kind == DocumentKind::Latex)
                    && workspace.settings(uri).diagnostics.math_delimiters
            })
            .cloned()
            .collect();
        if enabled.is_empty() {
            return Self::default();
        }
        let projects = workspace.saved_projects();
        let mut roots: BTreeMap<Url, BTreeSet<Url>> = BTreeMap::new();
        for uri in &enabled {
            for root in workspace.source_roots_in(uri, &projects) {
                roots.entry(root).or_default().insert(uri.clone());
            }
        }
        let mut contexts: BTreeMap<Url, Context> = BTreeMap::new();
        // Build each source/definition context once per consuming root, rather
        // than rebuilding the whole project separately for every requested file.
        for (root, consumers) in roots {
            let scope = workspace.symbol_scope(&root);
            let docs: Vec<_> = scope
                .iter()
                .filter_map(|uri| workspace.documents.get(uri))
                .filter(|doc| doc.kind == DocumentKind::Latex)
                .collect();
            if !workspace.index_truncated.is_empty()
                || docs.iter().map(|doc| doc.text.source.len()).sum::<usize>() > 16 * 1024 * 1024
                || docs
                    .iter()
                    .map(|doc| doc.syntax.commands.len())
                    .sum::<usize>()
                    > 100_000
                || docs
                    .iter()
                    .map(|doc| doc.syntax.definitions.len() + doc.syntax.environments.len())
                    .sum::<usize>()
                    > 4096
            {
                for uri in consumers {
                    contexts.entry(uri).or_default().unsafe_catcodes = true;
                }
                continue;
            }
            let signatures = CommandSignatures::new(scope.iter().filter_map(|uri| {
                workspace
                    .documents
                    .get(uri)
                    .filter(|doc| doc.kind == DocumentKind::Latex)
                    .map(|doc| (doc.text.source.as_str(), &doc.syntax))
            }));
            let unsafe_catcodes = ["(", ")", "[", "]"]
                .iter()
                .any(|name| !signatures.command_unchanged(name))
                || scope
                    .iter()
                    .filter_map(|uri| workspace.documents.get(uri))
                    .any(changes_math_syntax);
            let mut users: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for doc in scope.iter().filter_map(|uri| workspace.documents.get(uri)) {
                for entry in &doc.syntax.definitions {
                    if let Some(target) = crate::language::signature::source_alias(
                        &doc.text.source,
                        &doc.syntax,
                        entry,
                    )
                    .or_else(|| {
                        crate::language::signature::literal_copy_target(
                            &doc.text.source,
                            &doc.syntax,
                            entry,
                            false,
                        )
                        .map(|(target, _)| target)
                    }) {
                        users.entry(target).or_default().push(entry.name.clone());
                    }
                }
            }
            let mut pending: Vec<_> = SOURCE_LOADERS
                .iter()
                .map(|name| (*name).to_owned())
                .collect();
            let mut potential_inputs = BTreeSet::new();
            while let Some(name) = pending.pop() {
                if potential_inputs.insert(name.clone()) {
                    pending.extend(users.get(&name).into_iter().flatten().cloned());
                }
            }
            let inputs = crate::project::source_inputs::Index::new(workspace, &root);
            for uri in consumers {
                let context = contexts.entry(uri.clone()).or_default();
                context.continuation |= uri != root || inputs.incomplete;
                context.unsafe_catcodes |= unsafe_catcodes;
                if let Some(semantics) = Semantics::new(&workspace.documents[&uri], &signatures) {
                    // A shared source must have the same math and parameter
                    // interpretation in every consuming project.
                    context.unsafe_catcodes |= context
                        .semantics
                        .as_ref()
                        .is_some_and(|previous| previous != &semantics);
                    context.semantics.get_or_insert(semantics);
                } else {
                    context.unsafe_catcodes = true;
                }
                // Captured meanings remain possible mode boundaries even
                // when dependency discovery cannot certify their execution.
                context
                    .potential_inputs
                    .extend(potential_inputs.iter().cloned());
                for call in inputs.calls.get(&uri).into_iter().flatten() {
                    context.boundaries.insert(
                        call.command.span.start,
                        call.command
                            .argument
                            .as_ref()
                            .map_or(call.command.span.end, |arg| arg.outer_span.end),
                    );
                }
            }
        }
        Self {
            diagnostics: contexts
                .into_iter()
                .map(|(uri, context)| {
                    let diagnostics = check(&workspace.documents[&uri], &uri, context);
                    (uri, diagnostics)
                })
                .collect(),
        }
    }
    pub(crate) fn take(&mut self, uri: &Url) -> Vec<Diagnostic> {
        self.diagnostics.remove(uri).unwrap_or_default()
    }
}

fn changes_math_syntax(doc: &Document) -> bool {
    doc.syntax.commands.iter().any(|command| {
        if command.name == "catcode" {
            return true;
        }
        if !matches!(
            command.name.as_str(),
            "def" | "gdef" | "edef" | "xdef" | "let" | "futurelet"
        ) && !syntax::declares_traditional_command(&command.name)
            && !syntax::declares_document_command(&command.name)
        {
            return false;
        }
        let target = syntax::token(
            &doc.text.source,
            syntax::skip_space(&doc.text.source, command.span.end),
        );
        matches!(target, Some("\\(" | "\\)" | "\\[" | "\\]"))
            || command
                .argument
                .as_ref()
                .is_some_and(|arg| matches!(arg.value.trim(), "\\(" | "\\)" | "\\[" | "\\]"))
    })
}

fn math_environment(name: &str) -> bool {
    matches!(
        name.trim_end_matches('*'),
        "math"
            | "displaymath"
            | "equation"
            | "align"
            | "alignat"
            | "flalign"
            | "gather"
            | "multline"
            | "eqnarray"
    )
}
fn box_body(source: &str, command: &syntax::Command) -> Option<Span<usize>> {
    if let Some(argument) = &command.argument {
        return Some(argument.outer_span.clone());
    }
    if !matches!(command.name.as_str(), "hbox" | "vbox" | "vtop") {
        return None;
    }
    let mut at = syntax::skip_space(source, command.span.end);
    if !source[at..].starts_with("to") && !source[at..].starts_with("spread") {
        return None;
    }
    let limit = source.len().min(at.saturating_add(512));
    while at < limit {
        match source.as_bytes()[at] {
            b'{' => return syntax::group(source, at, b'{', b'}').map(|(body, _)| body.outer_span),
            b'}' | b'$' => break,
            b'%' => at = syntax::skip_space(source, at),
            b'\\' => at += syntax::token(source, at)?.len(),
            _ => at += 1,
        }
    }
    None
}

fn merge(mut spans: Vec<Span<usize>>) -> Vec<Span<usize>> {
    spans.sort_by_key(|span| span.start);
    let mut merged: Vec<Span<usize>> = vec![];
    for span in spans {
        if let Some(last) = merged.last_mut().filter(|last| span.start <= last.end) {
            last.end = last.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    merged
}
fn check(doc: &Document, uri: &Url, context: Context) -> Vec<Diagnostic> {
    if context.unsafe_catcodes {
        return vec![];
    }
    let semantics = context.semantics.unwrap_or_default();
    let source = &doc.text.source;
    let bytes = source.as_bytes();
    let mut opaque: Vec<_> = doc
        .syntax
        .ignored
        .iter()
        .chain(&doc.syntax.literal_conversions)
        .chain(&doc.syntax.literal_tests)
        .chain(&doc.syntax.ambiguous_string_tests)
        .cloned()
        .collect();
    opaque.extend(crate::symbols::expl3::math_data_spans(doc));
    let deferred = merge(opaque.clone());
    let mut deferred_index = 0;
    let mut environments: Vec<(String, usize)> = vec![];
    let mut boundaries = context.boundaries;
    let mut names = Vec::new();
    let mut islands = BTreeMap::new();
    let mut terminals = BTreeSet::new();
    for command in &doc.syntax.commands {
        while deferred
            .get(deferred_index)
            .is_some_and(|span| span.end <= command.span.start)
        {
            deferred_index += 1;
        }
        if deferred
            .get(deferred_index)
            .is_some_and(|span| span.contains(&command.span.start))
            || doc.syntax.control_operands.contains(&command.span.start)
            || semantics.operands.contains(&command.span.start)
        {
            continue;
        }
        if SOURCE_LOADERS.contains(&command.name.as_str())
            || context.potential_inputs.contains(&command.name)
        {
            boundaries.insert(
                command.span.start,
                command
                    .argument
                    .as_ref()
                    .map_or(command.span.end, |arg| arg.outer_span.end),
            );
        }
        if command.name == "csname" {
            names.push(command.span.start);
        }
        if command.name == "endcsname" {
            if let Some(start) = names.pop() {
                if !semantics.markers.contains_key(&start) {
                    opaque.push(start..command.span.end);
                }
            }
        }
        if command.name == "end"
            && command
                .argument
                .as_ref()
                .is_some_and(|arg| arg.value == "document")
        {
            terminals.insert(command.span.start);
        }
        if matches!(
            command.name.as_str(),
            "mbox"
                | "hbox"
                | "vbox"
                | "vtop"
                | "text"
                | "textnormal"
                | "textrm"
                | "textsf"
                | "texttt"
                | "textup"
                | "textit"
                | "textsl"
                | "textsc"
                | "textbf"
                | "textmd"
                | "emph"
        ) {
            if let Some(body) = box_body(source, command) {
                islands.insert(body.start, body.end);
            }
        }
        if syntax::file_extensions(&command.name).is_some() {
            if let Some(argument) = syntax::file_argument(source, command) {
                opaque.push(argument.outer_span.clone());
            }
        }
        if syntax::is_reference(&command.name)
            || syntax::is_citation(&command.name)
            || matches!(command.name.as_str(), "label" | "ensuremath")
        {
            if let Some(argument) = &command.argument {
                opaque.push(argument.outer_span.clone());
            }
        }
        if let Some(argument) = syntax::separate_label_argument(source, command)
            .or_else(|| syntax::range_reference_end(source, command))
        {
            opaque.push(argument.outer_span);
        }
        if matches!(command.name.as_str(), "begin" | "end") {
            let Some(argument) = &command.argument else {
                continue;
            };
            if !math_environment(&argument.value) {
                continue;
            }
            if command.name == "begin" {
                environments.push((argument.value.clone(), command.span.start));
            } else if let Some(i) = environments
                .iter()
                .rposition(|(name, _)| name == &argument.value)
            {
                opaque.push(environments[i].1..argument.outer_span.end);
                environments.truncate(i);
            }
        }
    }
    opaque.extend(names.into_iter().map(|start| start..source.len()));
    opaque.extend(
        environments
            .into_iter()
            .map(|(_, start)| start..source.len()),
    );
    for entries in [
        &doc.syntax.labels,
        &doc.syntax.anchors,
        &doc.syntax.anchor_references,
        &doc.syntax.citations,
        &doc.syntax.citation_references,
        &doc.syntax.counters,
        &doc.syntax.environments,
        &doc.syntax.conditionals,
        &doc.syntax.toggles,
        &doc.syntax.saved_text,
        &doc.syntax.saved_text_references,
    ] {
        opaque.extend(entries.iter().map(|entry| entry.span.clone()));
    }
    let opaque = merge(opaque);
    let mut opaque_index = 0;
    let mut result = vec![];
    let mut emit = |span: Span<usize>, code: &str, message: String, open: Option<&Open>| {
        if result.len() == MAX_DIAGNOSTICS {
            return;
        }
        result.push(Diagnostic {
            range: doc.text.range(span.start, span.end),
            severity: Some(DiagnosticSeverity::WARNING),
            source: Some("latex-lsp".into()),
            code: Some(NumberOrString::String(code.into())),
            message,
            related_information: open.map(|open| {
                vec![DiagnosticRelatedInformation {
                    location: Location::new(
                        uri.clone(),
                        doc.text.range(open.span.start, open.span.end),
                    ),
                    message: format!("{} math starts here", open.mode.description()),
                }]
            }),
            ..Default::default()
        });
    };
    let mut open: Option<Open> = None;
    let mut unknown = context.continuation;
    let mut at = 0;
    let mut terminal = false;
    while at < bytes.len() {
        // A named input's csname is data, but the invocation still changes
        // the surrounding mode. Handle that boundary before its name span.
        if let Some(end) = boundaries.get(&at) {
            open = None;
            unknown = true;
            at = *end;
            continue;
        }
        while opaque.get(opaque_index).is_some_and(|span| span.end <= at) {
            opaque_index += 1;
        }
        if let Some(span) = opaque.get(opaque_index).filter(|span| span.contains(&at)) {
            at = span.end;
            continue;
        }
        if open.is_some() {
            if let Some(end) = islands.get(&at) {
                at = *end;
                continue;
            }
        }
        if doc.syntax.punctuation_operands.contains(&at) || semantics.dollars.contains(&at) {
            at += 1;
            continue;
        }
        let mut width = 1;
        let mut marker = None;
        match bytes[at] {
            b'\\' => {
                let token = syntax::token(source, at).unwrap_or("\\");
                width = token.len();
                if !doc.syntax.control_operands.contains(&at) && !semantics.operands.contains(&at) {
                    if let Some((end, mode, opening)) = semantics.markers.get(&at) {
                        width = end - at;
                        marker = Some((*mode, *opening));
                    } else {
                        // The syntax command list indexes control words;
                        // literal control-symbol delimiters still come from
                        // the token stream, with the same operand filtering.
                        marker = match token {
                            "\\(" => Some((Mode::Inline, true)),
                            "\\[" => Some((Mode::Display, true)),
                            "\\)" => Some((Mode::Inline, false)),
                            "\\]" => Some((Mode::Display, false)),
                            _ => None,
                        };
                    }
                    if token == "\\endinput" {
                        open = None;
                        break;
                    }
                    if terminals.contains(&at) {
                        terminal = true;
                        break;
                    }
                    // Conditional branches can contain deliberately unbalanced
                    // shifts. Forget the current mode at execution boundaries.
                    if token.starts_with("\\if") || matches!(token, "\\else" | "\\fi" | "\\or") {
                        open = None;
                        unknown = true;
                    }
                }
            }
            b'$' => {
                if open.as_ref().map(|open| open.mode) != Some(Mode::Inline)
                    && bytes.get(at + 1) == Some(&b'$')
                {
                    width = 2;
                }
                if !unknown || open.is_some() {
                    marker = Some((
                        if width == 2 {
                            Mode::Display
                        } else {
                            Mode::Inline
                        },
                        open.is_none(),
                    ));
                }
            }
            _ => {}
        }
        if let Some((mode, opening)) = marker {
            if opening {
                if let Some(active) = &open {
                    emit(
                        at..at + width,
                        "nested-math",
                        format!(
                            "Math delimiter starts inside {} math",
                            active.mode.description()
                        ),
                        Some(active),
                    );
                } else {
                    open = Some(Open {
                        mode,
                        span: at..at + width,
                    });
                }
            } else if let Some(active) = open.take() {
                if active.mode != mode {
                    emit(
                        at..at + width,
                        "mismatched-math",
                        format!(
                            "Cannot close {} math with a delimiter for {} math",
                            active.mode.description(),
                            mode.description()
                        ),
                        Some(&active),
                    );
                }
            } else if !unknown {
                emit(
                    at..at + width,
                    "unexpected-math-end",
                    "Math closing delimiter has no matching opener".into(),
                    None,
                );
            }
        }
        at += width;
    }
    if terminal || !context.continuation {
        if let Some(open) = open {
            emit(
                open.span,
                "unclosed-math",
                format!("Unclosed {} math", open.mode.description()),
                None,
            );
        }
    }
    result
}
