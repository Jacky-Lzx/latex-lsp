//! Ordered source traversal for environments spanning literal input calls.
use crate::{
    signature::{definition_body_spans, CommandSignatures, EnvironmentCall},
    syntax::{self, QuickFix},
    workspace::{resolve_file_path, DocumentKind, Workspace},
};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::{DiagnosticRelatedInformation, Location, Url};

const MAX_VISITS: usize = 4096;
const MAX_EVENTS: usize = 65536;
const MAX_DEPTH: usize = 64;

#[derive(Clone)]
enum Operation {
    Environment(EnvironmentCall),
    Input(Option<Url>, bool),
    Uncertain,
    Stop,
}
#[derive(Clone, Default)]
struct Source {
    operations: Vec<(usize, Operation)>,
    uncertain_names: BTreeSet<String>,
}
#[derive(Clone)]
struct Event {
    uri: Url,
    occurrence: usize,
    call: EnvironmentCall,
}
struct LocatedFix {
    uri: Url,
    occurrence: usize,
    fix: QuickFix,
}
struct Walk<'a> {
    workspace: &'a Workspace,
    root: &'a Url,
    sources: BTreeMap<Url, Source>,
    active: BTreeSet<Url>,
    packages: BTreeSet<Url>,
    occurrences: BTreeMap<Url, usize>,
    uncertain_names: BTreeSet<String>,
    events: Vec<Event>,
    complete: bool,
    has_inputs: bool,
    visits: usize,
}

impl Walk<'_> {
    fn visit(&mut self, uri: &Url, depth: usize) {
        if depth > MAX_DEPTH || self.visits >= MAX_VISITS || !self.active.insert(uri.clone()) {
            self.complete = false;
            return;
        }
        let Some(source) = self.sources.get(uri).cloned() else {
            self.complete = false;
            self.active.remove(uri);
            return;
        };
        self.visits += 1;
        let count = self.occurrences.entry(uri.clone()).or_default();
        let occurrence = *count;
        *count += 1;
        self.uncertain_names.extend(source.uncertain_names);
        for (_, operation) in source.operations {
            if self.events.len() >= MAX_EVENTS {
                self.complete = false;
                break;
            }
            match operation {
                Operation::Environment(call) => self.events.push(Event {
                    uri: uri.clone(),
                    occurrence,
                    call,
                }),
                Operation::Input(target, package) => {
                    if !package {
                        self.has_inputs = true;
                    }
                    match target {
                        Some(target) if !package || self.packages.insert(target.clone()) => {
                            self.visit(&target, depth + 1)
                        }
                        Some(_) => {}
                        None if package => {}
                        None => self.complete = false,
                    }
                }
                Operation::Uncertain => self.complete = false,
                Operation::Stop => {
                    if uri == self.root {
                        self.complete = false;
                    }
                    break;
                }
            }
        }
        self.active.remove(uri);
    }

    fn pair(&self) -> BTreeMap<Url, Vec<QuickFix>> {
        if !self.complete {
            return BTreeMap::new();
        }
        let mut stack: Vec<&Event> = Vec::new();
        let mut issues = Vec::new();
        for event in &self.events {
            let name = &event.call.argument.value;
            if self.uncertain_names.contains(name) || syntax::is_verbatim_environment(name) {
                continue;
            }
            if event.call.begin {
                stack.push(event);
            } else if let Some(index) = stack
                .iter()
                .rposition(|open| open.call.argument.value == *name)
            {
                let missing = stack.split_off(index + 1);
                if !missing.is_empty() {
                    let doc = &self.workspace.documents[&event.uri];
                    let replacement: String = missing
                        .iter()
                        .rev()
                        .map(|open| {
                            format!(
                                "\\end{{{}}}{}",
                                open.call.argument.value,
                                doc.text.newline()
                            )
                        })
                        .collect();
                    for open in missing {
                        let missing_name = &open.call.argument.value;
                        let same_source =
                            open.uri == event.uri && open.occurrence == event.occurrence;
                        let span = if same_source {
                            open.call.argument.span.clone()
                        } else {
                            event.call.argument.span.clone()
                        };
                        let mut fix = syntax::syntax_fix(
                            &doc.text,
                            span,
                            format!("Unclosed environment `{missing_name}` before \\end{{{name}}}"),
                            format!("Close environment `{missing_name}` before `{name}`"),
                            tower_lsp::lsp_types::TextEdit {
                                range: doc.text.range(event.call.span.start, event.call.span.start),
                                new_text: replacement.clone(),
                            },
                        );
                        if !same_source {
                            self.related(&mut fix, open);
                        }
                        issues.push(LocatedFix {
                            uri: event.uri.clone(),
                            occurrence: event.occurrence,
                            fix,
                        });
                    }
                }
                stack.pop();
            } else {
                let doc = &self.workspace.documents[&event.uri];
                let fix = syntax::syntax_fix(
                    &doc.text,
                    event.call.argument.span.clone(),
                    format!("Unexpected \\end{{{name}}}"),
                    format!("Remove unexpected \\end{{{name}}}"),
                    tower_lsp::lsp_types::TextEdit {
                        range: doc.text.range(event.call.span.start, event.call.span.end),
                        new_text: String::new(),
                    },
                );
                issues.push(LocatedFix {
                    uri: event.uri.clone(),
                    occurrence: event.occurrence,
                    fix,
                });
            }
        }
        if !stack.is_empty() {
            let doc = &self.workspace.documents[self.root];
            let end = doc.text.source.len();
            let prefix = if doc.text.source.ends_with('\n') {
                ""
            } else {
                doc.text.newline()
            };
            let replacement = format!(
                "{prefix}{}",
                stack
                    .iter()
                    .rev()
                    .map(|open| format!(
                        "\\end{{{}}}{}",
                        open.call.argument.value,
                        doc.text.newline()
                    ))
                    .collect::<String>()
            );
            for open in stack {
                let name = &open.call.argument.value;
                let span = if open.uri == *self.root {
                    open.call.argument.span.clone()
                } else {
                    end..end
                };
                let mut fix = syntax::syntax_fix(
                    &doc.text,
                    span,
                    format!("Unclosed environment `{name}`"),
                    format!("Close environment `{name}`"),
                    tower_lsp::lsp_types::TextEdit {
                        range: doc.text.range(end, end),
                        new_text: replacement.clone(),
                    },
                );
                if open.uri != *self.root {
                    self.related(&mut fix, open);
                }
                issues.push(LocatedFix {
                    uri: self.root.clone(),
                    occurrence: 0,
                    fix,
                });
            }
        }
        let mut result: BTreeMap<Url, Vec<QuickFix>> = BTreeMap::new();
        for issue in &issues {
            // An edit of a repeated source must be valid in every execution.
            let agreeing: BTreeSet<_> = issues
                .iter()
                .filter(|other| other.uri == issue.uri && same_fix(&other.fix, &issue.fix))
                .map(|other| other.occurrence)
                .collect();
            if agreeing.len() != self.occurrences[&issue.uri] {
                continue;
            }
            let fixes = result.entry(issue.uri.clone()).or_default();
            if !fixes.iter().any(|fix| same_fix(fix, &issue.fix)) {
                fixes.push(issue.fix.clone());
            }
        }
        result
    }

    fn related(&self, fix: &mut QuickFix, open: &Event) {
        let text = &self.workspace.documents[&open.uri].text;
        fix.diagnostic.related_information = Some(vec![DiagnosticRelatedInformation {
            location: Location::new(
                open.uri.clone(),
                text.range(open.call.argument.span.start, open.call.argument.span.end),
            ),
            message: format!("Environment `{}` starts here", open.call.argument.value),
        }]);
    }
}

fn source(
    workspace: &Workspace,
    root: &Url,
    uri: &Url,
    signatures: &CommandSignatures<'_>,
) -> Source {
    let doc = &workspace.documents[uri];
    let mut result = Source::default();
    let calls = signatures.environment_invocations(&doc.text.source, &doc.syntax);
    result.uncertain_names.extend(
        calls
            .iter()
            .filter(|call| call.operand || call.deferred)
            .map(|call| call.argument.value.clone()),
    );
    result.operations.extend(
        calls
            .into_iter()
            .filter(|call| !call.operand && !call.deferred)
            .map(|call| (call.span.start, Operation::Environment(call))),
    );
    let operands = signatures.argument_tokens(&doc.text.source, &doc.syntax);
    let bodies = definition_body_spans(&doc.text.source, &doc.syntax);
    for command in &doc.syntax.commands {
        if operands.contains(&command.span.start)
            || bodies.iter().any(|span| span.contains(&command.span.start))
        {
            continue;
        }
        let package = syntax::loader_extension(&command.name);
        let input = matches!(command.name.as_str(), "input" | "include");
        if !input && package.is_none() {
            if command.name == "endinput" {
                // TeX finishes the current input line before ending the file.
                // Do not infer a partial stack if that line still executes
                // tokens, or if endinput no longer has its primitive meaning.
                let remainder = doc.text.source[command.span.end..]
                    .split(['\r', '\n'])
                    .next()
                    .unwrap_or_default()
                    .trim_start();
                let operation = if signatures.command_unchanged("endinput")
                    && (remainder.is_empty() || remainder.starts_with('%'))
                {
                    Operation::Stop
                } else {
                    Operation::Uncertain
                };
                result.operations.push((command.span.start, operation));
                continue;
            }
            if matches!(command.name.as_str(), "InputIfFileExists" | "subfile") {
                result
                    .operations
                    .push((command.span.start, Operation::Input(None, false)));
                continue;
            }
            if command.name.starts_with("if")
                || matches!(
                    command.name.as_str(),
                    "else"
                        | "fi"
                        | "IfFileExists"
                        | "InputIfFileExists"
                        | "includeonly"
                        | "endinput"
                        | "subfile"
                )
            {
                result
                    .operations
                    .push((command.span.start, Operation::Uncertain));
            }
            continue;
        }
        if !signatures.command_unchanged(&command.name) {
            result.operations.push((
                command.span.start,
                if input {
                    Operation::Input(None, false)
                } else {
                    Operation::Uncertain
                },
            ));
            continue;
        }
        let Some(argument) = syntax::file_argument(&doc.text.source, command) else {
            result.operations.push((
                command.span.start,
                if input {
                    Operation::Input(None, false)
                } else {
                    Operation::Uncertain
                },
            ));
            continue;
        };
        let values: Vec<_> = if package == Some("sty") {
            argument.value.split(',').map(str::trim).collect()
        } else {
            vec![argument.value.as_str()]
        };
        for value in values {
            let target = syntax::is_literal_file_path(value)
                .then(|| {
                    resolve_file_path(root, &command.name, value, &[package.unwrap_or("tex")])
                        .or_else(|| {
                            resolve_file_path(
                                uri,
                                &command.name,
                                value,
                                &[package.unwrap_or("tex")],
                            )
                        })
                        .filter(|target| {
                            workspace
                                .documents
                                .get(target)
                                .is_some_and(|doc| doc.kind == DocumentKind::Latex)
                        })
                })
                .flatten();
            if package.is_some() && !syntax::is_literal_file_path(value) {
                result
                    .operations
                    .push((command.span.start, Operation::Uncertain));
            } else {
                result.operations.push((
                    command.span.start,
                    Operation::Input(target, package.is_some()),
                ));
            }
        }
    }
    result.operations.sort_by_key(|(start, _)| *start);
    result
}

type Interpretations = Vec<(bool, Vec<QuickFix>)>;

pub(crate) fn fixes(
    workspace: &Workspace,
    requested: &BTreeSet<Url>,
) -> BTreeMap<Url, Vec<QuickFix>> {
    let mut interpretations: BTreeMap<Url, Interpretations> = BTreeMap::new();
    for (root, scope) in workspace.saved_projects() {
        if scope.is_disjoint(requested) {
            continue;
        }
        let signatures = CommandSignatures::new(
            workspace
                .documents
                .iter()
                .filter(|(uri, doc)| scope.contains(*uri) && doc.kind == DocumentKind::Latex)
                .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
        );
        let sources = scope
            .iter()
            .filter(|uri| {
                workspace
                    .documents
                    .get(*uri)
                    .is_some_and(|doc| doc.kind == DocumentKind::Latex)
            })
            .map(|uri| (uri.clone(), source(workspace, &root, uri, &signatures)))
            .collect();
        let mut walk = Walk {
            workspace,
            root: &root,
            sources,
            active: BTreeSet::new(),
            packages: BTreeSet::new(),
            occurrences: BTreeMap::new(),
            uncertain_names: BTreeSet::new(),
            events: vec![],
            complete: signatures.environment_primitives_unchanged(),
            has_inputs: false,
            visits: 0,
        };
        walk.visit(&root, 0);
        let mut found = walk.pair();
        for uri in scope.intersection(requested) {
            interpretations
                .entry(uri.clone())
                .or_default()
                .push((walk.has_inputs, found.remove(uri).unwrap_or_default()));
        }
    }
    let mut result = BTreeMap::new();
    for (uri, mut contexts) in interpretations {
        if !contexts.iter().any(|(inputs, _)| *inputs) {
            continue;
        }
        let (_, mut agreed) = contexts.pop().unwrap();
        agreed.retain(|fix| {
            contexts
                .iter()
                .all(|(_, fixes)| fixes.iter().any(|other| same_fix(fix, other)))
        });
        result.insert(uri, agreed);
    }
    result
}

fn same_fix(a: &QuickFix, b: &QuickFix) -> bool {
    a.diagnostic == b.diagnostic && a.edit == b.edit && a.additional_edits == b.additional_edits
}
