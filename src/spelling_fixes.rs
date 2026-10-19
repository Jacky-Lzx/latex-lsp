//! Suggest bounded spelling corrections from the diagnostic's own project.
use crate::{build_log, syntax::QuickFix, workspace::Workspace};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::{Diagnostic, NumberOrString, Range, TextEdit, Url};

pub(crate) const MAX_COMPARISONS: usize = 16_384;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Label,
    Citation,
    Command,
    Environment,
    Counter,
    Anchor,
}

fn target<'a>(
    doc: &'a crate::workspace::Document,
    diagnostic: &'a Diagnostic,
) -> Option<(Kind, &'a str)> {
    if diagnostic.code == Some(NumberOrString::String("undefined-pdf-destination".into())) {
        let name = diagnostic.data.as_ref()?.get("destination")?.as_str()?;
        if build_log::undefined_destination_name(&diagnostic.message) != Some(name) {
            return None;
        }
        return doc
            .syntax
            .anchor_references
            .iter()
            .any(|entry| {
                entry.name == name
                    && doc.text.range(entry.span.start, entry.span.end) == diagnostic.range
            })
            .then_some((Kind::Anchor, name));
    }
    if let Some((citation, key)) = build_log::undefined_reference_key(&diagnostic.message) {
        return build_log::undefined_reference_ranges(
            &doc.text,
            diagnostic.range.start.line,
            &diagnostic.message,
        )
        .contains(&diagnostic.range)
        .then_some((
            if citation {
                Kind::Citation
            } else {
                Kind::Label
            },
            key,
        ));
    }
    if diagnostic.code == Some(NumberOrString::String("undefined-environment".into())) {
        let name = diagnostic.data.as_ref()?.get("environment")?.as_str()?;
        if build_log::undefined_environment_name(&diagnostic.message) != Some(name) {
            return None;
        }
        return doc
            .syntax
            .commands
            .iter()
            .any(|command| {
                command.name == "begin"
                    && command.argument.as_ref().is_some_and(|argument| {
                        let span = argument.span.clone();
                        doc.text.source[span.clone()] == *name
                            && doc.text.range(span.start, span.end) == diagnostic.range
                    })
            })
            .then_some((Kind::Environment, name));
    }
    if diagnostic.code == Some(NumberOrString::String("undefined-counter".into())) {
        let name = diagnostic.data.as_ref()?.get("counter")?.as_str()?;
        if build_log::undefined_counter_name(&diagnostic.message) != Some(name) {
            return None;
        }
        return doc
            .syntax
            .commands
            .iter()
            .any(|command| {
                crate::counters::invocations(&doc.text.source, command)
                    .into_iter()
                    .any(|(span, _)| {
                        doc.text.source[span.clone()] == *name
                            && doc.text.range(span.start, span.end) == diagnostic.range
                    })
            })
            .then_some((Kind::Counter, name));
    }
    if diagnostic.message != "Undefined control sequence."
        || diagnostic.code != Some(NumberOrString::String("undefined-control-sequence".into()))
    {
        return None;
    }
    let name = diagnostic.data.as_ref()?.get("command")?.as_str()?;
    doc.syntax
        .commands
        .iter()
        .any(|command| {
            command.name == name
                && doc.text.range(command.span.start, command.span.end) == diagnostic.range
        })
        .then_some((Kind::Command, name))
}

fn opaque_environment(
    doc: &crate::workspace::Document,
    command: &crate::syntax::Command,
    name: &str,
) -> Option<bool> {
    if command.name != "begin" || !crate::syntax::is_verbatim_environment(name) {
        return Some(false);
    }
    let argument = command.argument.as_ref()?;
    let ending = format!("\\end{{{name}}}");
    doc.syntax
        .ignored
        .iter()
        .any(|ignored| {
            ignored.start == argument.outer_span.end
                && doc.text.source[ignored.clone()].ends_with(&ending)
        })
        .then_some(true)
}

fn environment_end(doc: &crate::workspace::Document, begin: Range) -> Option<Range> {
    let mut started = false;
    let mut stack = Vec::new();
    for command in &doc.syntax.commands {
        if !matches!(command.name.as_str(), "begin" | "end") {
            continue;
        }
        if !started {
            started = command.name == "begin"
                && command.argument.as_ref().is_some_and(|argument| {
                    let span = argument.span.clone();
                    doc.text.range(span.start, span.end) == begin
                });
            if !started {
                continue;
            }
        }
        let argument = command.argument.as_ref()?;
        let span = argument.span.clone();
        let name = &doc.text.source[span.clone()];
        if !crate::syntax::is_static_key(name) {
            return None;
        }
        if command.name == "begin" {
            if opaque_environment(doc, command, name)? {
                continue;
            }
            stack.push(name);
            if stack.len() > 256 {
                return None;
            }
        } else {
            if stack.pop() != Some(name) {
                return None;
            }
            if stack.is_empty() {
                return Some(doc.text.range(span.start, span.end));
            }
        }
    }
    None
}

fn closing_environment_fix(
    doc: &crate::workspace::Document,
    diagnostic: &Diagnostic,
) -> Option<QuickFix> {
    let (name, expected) = build_log::mismatched_environment_names(&diagnostic.message)?;
    let data = diagnostic.data.as_ref()?;
    if name == expected
        || data.get("environment")?.as_str()? != name
        || data.get("expectedEnvironment")?.as_str()? != expected
    {
        return None;
    }
    let mut stack = Vec::new();
    for command in &doc.syntax.commands {
        if !matches!(command.name.as_str(), "begin" | "end") {
            continue;
        }
        let argument = command.argument.as_ref()?;
        let span = argument.span.clone();
        let argument_name = &doc.text.source[span.clone()];
        if !crate::syntax::is_static_key(argument_name) {
            return None;
        }
        if command.name == "end" && doc.text.range(span.start, span.end) == diagnostic.range {
            if argument_name != name || stack.last().copied() != Some(expected) {
                return None;
            }
            return Some(QuickFix {
                title: format!("Close environment `{expected}`"),
                diagnostic: diagnostic.clone(),
                edit: TextEdit {
                    range: diagnostic.range,
                    new_text: expected.into(),
                },
                additional_edits: vec![],
            });
        }
        if opaque_environment(doc, command, argument_name)? {
            continue;
        }
        if command.name == "begin" {
            stack.push(argument_name);
            if stack.len() > 256 {
                return None;
            }
        } else if stack.pop() != Some(argument_name) {
            return None;
        }
    }
    None
}

fn valid_name(kind: Kind, name: &str) -> bool {
    if kind == Kind::Command {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic() || byte == b'@')
    } else {
        crate::syntax::is_static_key(name)
    }
}

pub(crate) fn distance(left: &str, right: &str, limit: usize) -> Option<usize> {
    let left: Vec<_> = left.chars().take(129).collect();
    let right: Vec<_> = right.chars().take(129).collect();
    if left.len() > 128 || right.len() > 128 || left.len().abs_diff(right.len()) > limit {
        return None;
    }
    let mut previous: Vec<_> = (0..=right.len()).collect();
    let mut before_previous = previous.clone();
    for (i, a) in left.iter().enumerate() {
        let mut current = vec![i + 1; right.len() + 1];
        for (j, b) in right.iter().enumerate() {
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + usize::from(a != b));
            if i > 0 && j > 0 && *a == right[j - 1] && left[i - 1] == *b {
                current[j + 1] = current[j + 1].min(before_previous[j - 1] + 1);
            }
        }
        before_previous = previous;
        previous = current;
    }
    let result = previous[right.len()];
    (result <= limit).then_some(result)
}

pub fn fixes(workspace: &Workspace, uri: &Url, range: Range) -> Vec<QuickFix> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let mut examined = 0;
    let mut comparisons = 0;
    for (root, diagnostics) in &workspace.build_diagnostics {
        let Some(diagnostics) = diagnostics.get(uri) else {
            continue;
        };
        let project = workspace.project_documents(root);
        for diagnostic in diagnostics {
            if diagnostic.source.as_deref() != Some("latex-build")
                || build_log::unresolved_location(diagnostic)
                || diagnostic.range.start > range.end
                || range.start > diagnostic.range.end
            {
                continue;
            }
            examined += 1;
            if examined > 64 {
                return result;
            }
            if matches!(diagnostic.code.as_ref(), Some(NumberOrString::String(code)) if matches!(code.as_str(), "missing-include" | "missing-input"))
            {
                for fix in
                    crate::file_fixes::fixes(workspace, root, uri, diagnostic, &mut comparisons)
                {
                    if seen.insert((
                        diagnostic.range.start,
                        diagnostic.range.end,
                        fix.edit.new_text.clone(),
                    )) {
                        result.push(fix);
                        if result.len() >= 256 {
                            return result;
                        }
                    }
                }
                continue;
            }
            if diagnostic.code == Some(NumberOrString::String("mismatched-environment-end".into()))
            {
                if let Some(fix) = closing_environment_fix(doc, diagnostic) {
                    if seen.insert((
                        diagnostic.range.start,
                        diagnostic.range.end,
                        fix.edit.new_text.clone(),
                    )) {
                        result.push(fix);
                        if result.len() >= 256 {
                            return result;
                        }
                    }
                }
                continue;
            }
            let Some((kind, key)) = target(doc, diagnostic) else {
                continue;
            };
            let counter_conflicts = if kind == Kind::Counter {
                crate::counters::replacement_conflicts(doc, diagnostic.range)
            } else {
                BTreeSet::new()
            };
            let paired_end = if kind == Kind::Environment {
                let Some(end) = environment_end(doc, diagnostic.range) else {
                    continue;
                };
                Some(end)
            } else {
                None
            };
            let length = key.chars().count();
            if !(3..=128).contains(&length) || !valid_name(kind, key) {
                continue;
            }
            let mut candidates = BTreeMap::<String, usize>::new();
            let mut total_entries = 0;
            for document in project
                .iter()
                .filter(|uri| {
                    kind != Kind::Anchor
                        || workspace
                            .build_diagnostic_sources
                            .get(root)
                            .is_none_or(|sources| sources.contains(*uri))
                })
                .filter_map(|uri| workspace.documents.get(uri))
            {
                let entries = match kind {
                    Kind::Citation => &document.syntax.citations,
                    Kind::Label => &document.syntax.labels,
                    Kind::Command => &document.syntax.definitions,
                    Kind::Environment => &document.syntax.environments,
                    Kind::Counter => &document.syntax.counters,
                    Kind::Anchor => &document.syntax.anchors,
                };
                for entry in entries {
                    *candidates.entry(entry.name.clone()).or_default() += 1;
                    total_entries += 1;
                    if total_entries >= 4096 {
                        break;
                    }
                }
                if kind == Kind::Command {
                    for name in crate::generated::commands(document).take(4096) {
                        *candidates.entry(name).or_default() += 1;
                        total_entries += 1;
                        if total_entries >= 4096 {
                            break;
                        }
                    }
                }
                if kind == Kind::Counter {
                    for name in crate::generated::counters(document).take(4096) {
                        *candidates.entry(name.into()).or_default() += 1;
                        total_entries += 1;
                        if total_entries >= 4096 {
                            break;
                        }
                    }
                }
                if total_entries >= 4096 {
                    break;
                }
            }
            if kind == Kind::Command {
                for name in crate::catalog::COMMANDS
                    .iter()
                    .map(|(name, _, _)| *name)
                    .chain(crate::catalog::PRIMITIVES.iter().copied())
                {
                    candidates.entry(name.into()).or_insert(1);
                }
            }
            if kind == Kind::Environment {
                for name in crate::catalog::ENVIRONMENTS {
                    candidates.entry((*name).into()).or_insert(1);
                }
            }
            if kind == Kind::Counter {
                for name in crate::counters::BUILTINS {
                    candidates.entry((*name).into()).or_insert(1);
                }
            }
            // A known key can be unresolved because of build order or macros.
            if total_entries >= 4096 || candidates.contains_key(key) {
                continue;
            }
            let mut candidates: Vec<_> = candidates
                .into_iter()
                .filter(|(name, count)| {
                    *count == 1
                        && valid_name(kind, name)
                        && !counter_conflicts.contains(name.as_str())
                })
                .take(MAX_COMPARISONS.saturating_sub(comparisons))
                .filter_map(|(name, _)| {
                    comparisons += 1;
                    distance(key, &name, if length < 8 { 1 } else { 2 }).map(|score| (score, name))
                })
                .collect();
            candidates.sort();
            for (_, name) in candidates.into_iter().take(5) {
                let (category, replacement) = match kind {
                    Kind::Label => (
                        "label",
                        crate::syntax::label_replacement_text(
                            &doc.text,
                            &doc.syntax.commands,
                            diagnostic.range,
                            &name,
                        ),
                    ),
                    Kind::Citation => ("citation", name.clone()),
                    Kind::Command => ("command", format!("\\{name}")),
                    Kind::Environment => ("environment", name.clone()),
                    Kind::Counter => ("counter", name.clone()),
                    Kind::Anchor => {
                        let Some(replacement) =
                            crate::anchors::replacement_text(doc, diagnostic.range, &name)
                        else {
                            continue;
                        };
                        ("PDF destination", replacement)
                    }
                };
                if !seen.insert((diagnostic.range.start, diagnostic.range.end, name.clone())) {
                    continue;
                }
                result.push(QuickFix {
                    title: format!(
                        "Replace with {category} `{}`",
                        if matches!(kind, Kind::Label | Kind::Anchor) {
                            &name
                        } else {
                            &replacement
                        }
                    ),
                    diagnostic: diagnostic.clone(),
                    edit: TextEdit {
                        range: diagnostic.range,
                        new_text: replacement.clone(),
                    },
                    additional_edits: paired_end
                        .into_iter()
                        .map(|range| TextEdit {
                            range,
                            new_text: replacement.clone(),
                        })
                        .collect(),
                });
                if result.len() >= 256 {
                    return result;
                }
            }
        }
    }
    result
}
