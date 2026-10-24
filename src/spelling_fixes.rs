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
    Length,
    Toggle,
    Boolean,
    Anchor,
    Color,
    ColorModel,
}

fn target<'a>(
    doc: &'a crate::workspace::Document,
    diagnostic: &'a Diagnostic,
    signatures: Option<&crate::signature::CommandSignatures<'_>>,
) -> Option<(Kind, &'a str)> {
    if diagnostic.code == Some(NumberOrString::String("undefined-pdf-destination".into())) {
        let name = diagnostic.data.as_ref()?.get("destination")?.as_str()?;
        if build_log::undefined_destination_name(&diagnostic.message).as_deref() != Some(name) {
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
        if !build_log::undefined_reference_ranges(
            &doc.text,
            diagnostic.range.start.line,
            &diagnostic.message,
        )
        .contains(&diagnostic.range)
        {
            return None;
        }
        let key = if citation {
            key
        } else {
            // Auxiliary warning names may differ from the verified source key.
            let start = doc.text.offset(diagnostic.range.start)?;
            let end = doc.text.offset(diagnostic.range.end)?;
            doc.text.source.get(start..end)?
        };
        return Some((
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
        return build_log::environment_error_invocations_with_signatures(
            &doc.text.source,
            &doc.syntax,
            signatures?,
            "begin",
        )
        .iter()
        .any(|(span, _)| {
            doc.text.source[span.clone()] == *name
                && doc.text.range(span.start, span.end) == diagnostic.range
        })
        .then_some((Kind::Environment, name));
    }
    if diagnostic.code == Some(NumberOrString::String("undefined-counter".into())) {
        let name = diagnostic.data.as_ref()?.get("counter")?.as_str()?;
        if build_log::undefined_counter_name(&diagnostic.message) != Some(name) {
            return None;
        }
        return crate::counters::error_invocations_with_signatures(
            &doc.text.source,
            &doc.syntax,
            signatures?,
        )
        .iter()
        .any(|(span, _)| {
            doc.text.source[span.clone()] == *name
                && doc.text.range(span.start, span.end) == diagnostic.range
        })
        .then_some((Kind::Counter, name));
    }

    if diagnostic.code == Some(NumberOrString::String("undefined-length".into())) {
        let name = diagnostic.data.as_ref()?.get("length")?.as_str()?;
        if build_log::undefined_length_name(&diagnostic.message) != Some(name) {
            return None;
        }
        return crate::lengths::error_invocations_with_signatures(
            &doc.text.source,
            &doc.syntax,
            signatures?,
        )
        .iter()
        .any(|(span, _)| {
            doc.text.source[span.clone()].strip_prefix('\\') == Some(name)
                && doc.text.range(span.start, span.end) == diagnostic.range
        })
        .then_some((Kind::Length, name));
    }

    if diagnostic.code == Some(NumberOrString::String("undefined-toggle".into())) {
        let name = diagnostic.data.as_ref()?.get("toggle")?.as_str()?;
        if build_log::undefined_toggle_name(&diagnostic.message) != Some(name) {
            return None;
        }
        return crate::switch_diagnostics::error_invocations_with_signatures(
            &doc.text.source,
            &doc.syntax,
            signatures?,
            crate::switch_diagnostics::Kind::Toggle,
            diagnostic
                .message
                .starts_with(build_log::BOOLEAN_EXPRESSION_ERROR),
        )
        .iter()
        .any(|(span, _)| {
            doc.text.source[span.clone()] == *name
                && doc.text.range(span.start, span.end) == diagnostic.range
        })
        .then_some((Kind::Toggle, name));
    }

    if diagnostic.code == Some(NumberOrString::String("undefined-boolean".into())) {
        let name = diagnostic.data.as_ref()?.get("boolean")?.as_str()?;
        if build_log::undefined_boolean_name(&diagnostic.message) != Some(name) {
            return None;
        }
        return crate::switch_diagnostics::error_invocations_with_signatures(
            &doc.text.source,
            &doc.syntax,
            signatures?,
            crate::switch_diagnostics::Kind::Boolean,
            diagnostic
                .message
                .starts_with(build_log::BOOLEAN_EXPRESSION_ERROR),
        )
        .iter()
        .any(|(span, _)| {
            doc.text.source[span.clone()] == *name
                && doc.text.range(span.start, span.end) == diagnostic.range
        })
        .then_some((Kind::Boolean, name));
    }

    if diagnostic.code == Some(NumberOrString::String("undefined-color".into())) {
        let name = diagnostic.data.as_ref()?.get("color")?.as_str()?;
        if build_log::undefined_color_name(&diagnostic.message) != Some(name) {
            return None;
        }
        return crate::named_colors::error_invocations_with_signatures(
            &doc.text.source,
            &doc.syntax,
            signatures?,
        )
        .iter()
        .any(|(span, _)| {
            doc.text.source[span.clone()] == *name
                && doc.text.range(span.start, span.end) == diagnostic.range
        })
        .then_some((Kind::Color, name));
    }

    if diagnostic.code == Some(NumberOrString::String("undefined-color-model".into())) {
        let model = diagnostic.data.as_ref()?.get("model")?.as_str()?;
        if build_log::undefined_color_model_name(&diagnostic.message) != Some(model) {
            return None;
        }
        return crate::named_colors::model_error_invocations_with_signatures(
            &doc.text.source,
            &doc.syntax,
            signatures?,
        )
        .iter()
        .any(|(span, _)| {
            doc.text.source[span.clone()] == *model
                && doc.text.range(span.start, span.end) == diagnostic.range
        })
        .then_some((Kind::ColorModel, model));
    }
    if diagnostic.message != "Undefined control sequence."
        || diagnostic.code != Some(NumberOrString::String("undefined-control-sequence".into()))
    {
        return None;
    }
    let name = diagnostic.data.as_ref()?.get("command")?.as_str()?;
    let direct = doc.syntax.commands.iter().any(|command| {
        command.name == name
            && doc.text.range(command.span.start, command.span.end) == diagnostic.range
    });
    let saved = doc.syntax.name_command_references.iter().any(|entry| {
        entry.name == name
            && doc.text.source[entry.span.clone()].starts_with('\\')
            && doc.text.range(entry.span.start, entry.span.end) == diagnostic.range
    });
    (direct || saved).then_some((Kind::Command, name))
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

fn environment_end(
    doc: &crate::workspace::Document,
    begin: Range,
    signatures: &crate::signature::CommandSignatures<'_>,
) -> Option<Range> {
    let operands = signatures.argument_tokens(&doc.text.source, &doc.syntax);
    let mut started = false;
    let mut stack = Vec::new();
    for command in &doc.syntax.commands {
        if !matches!(command.name.as_str(), "begin" | "end")
            || operands.contains(&command.span.start)
        {
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
    signatures: &crate::signature::CommandSignatures<'_>,
) -> Option<QuickFix> {
    let (name, expected) = build_log::mismatched_environment_names(&diagnostic.message)?;
    let data = diagnostic.data.as_ref()?;
    if name == expected
        || data.get("environment")?.as_str()? != name
        || data.get("expectedEnvironment")?.as_str()? != expected
    {
        return None;
    }
    if !build_log::environment_error_invocations_with_signatures(
        &doc.text.source,
        &doc.syntax,
        signatures,
        "end",
    )
    .iter()
    .any(|(span, _)| {
        doc.text.source[span.clone()] == *name
            && doc.text.range(span.start, span.end) == diagnostic.range
    }) {
        return None;
    }
    let operands = signatures.argument_tokens(&doc.text.source, &doc.syntax);
    let mut stack = Vec::new();
    for command in &doc.syntax.commands {
        if !matches!(command.name.as_str(), "begin" | "end")
            || operands.contains(&command.span.start)
        {
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
    if kind == Kind::Citation {
        crate::syntax::is_static_citation_key(name)
    } else if matches!(kind, Kind::Label | Kind::Anchor) {
        crate::syntax::is_static_reference_key(name)
    } else if matches!(kind, Kind::Color | Kind::ColorModel) {
        crate::named_colors::literal_name(name)
    } else if matches!(kind, Kind::Command | Kind::Length) {
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

pub fn fixes(
    workspace: &Workspace,
    uri: &Url,
    range: Range,
    requested: &[Diagnostic],
) -> Vec<QuickFix> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let mut result = crate::reference_diagnostics::fixes(workspace, uri, range, requested);
    let mut seen = BTreeSet::new();
    let mut examined = 0;
    let mut comparisons = 0;
    let citation_resources = std::cell::OnceCell::new();
    let source_statuses = std::cell::OnceCell::new();
    for (root, diagnostics) in &workspace.build_diagnostics {
        let Some(diagnostics) = diagnostics.get(uri) else {
            continue;
        };
        let project = workspace.project_documents(root);
        let mut signatures = BTreeMap::new();
        for diagnostic in diagnostics {
            // Match the selected warning before deduplicating edits. Hyperref
            // can report both Hyper reference and Reference for the same key.
            if diagnostic.source.as_deref() != Some("latex-build")
                || build_log::unresolved_location(diagnostic)
                || diagnostic.range.start > range.end
                || range.start > diagnostic.range.end
                || !requested.is_empty()
                    && !requested.iter().any(|reported| {
                        reported.range == diagnostic.range
                            && reported.message == diagnostic.message
                            && reported.source == diagnostic.source
                    })
            {
                continue;
            }
            examined += 1;
            if examined > 64 {
                return result;
            }
            if diagnostic.code == Some(NumberOrString::String(crate::bib_diagnostics::CODE.into()))
            {
                for fix in crate::bib_diagnostics::fixes(
                    workspace,
                    root,
                    uri,
                    diagnostic,
                    &mut comparisons,
                ) {
                    if seen.insert((
                        diagnostic.range.start,
                        diagnostic.range.end,
                        fix.edit.new_text.clone(),
                    )) {
                        result.push(fix);
                    }
                }
                continue;
            }
            let literal = matches!(diagnostic.code.as_ref(), Some(NumberOrString::String(code)) if matches!(code.as_str(), "unescaped-special-character" | "extra-alignment-tab" | "extra-closing-brace" | "double-subscript" | "double-superscript" | "unmatched-right-delimiter" | "missing-right-delimiter" | "missing-delimiter-operand"));
            let invocation_scope = if literal
                || matches!(diagnostic.code.as_ref(), Some(NumberOrString::String(code)) if matches!(code.as_str(), "undefined-color" | "undefined-color-model" | "undefined-counter" | "undefined-length" | "undefined-toggle" | "undefined-boolean" | "undefined-environment" | "mismatched-environment-end" | "missing-input" | "missing-include" | "missing-package" | "missing-class" | "missing-bibliography-style" | "missing-bibliography-database"))
            {
                let confirmed = workspace.build_diagnostic_sources.get(root);
                let scope = if let Some(value) = diagnostic
                    .data
                    .as_ref()
                    .and_then(|data| data.get("macroSources"))
                {
                    let Ok(scope) = serde_json::from_value::<BTreeSet<Url>>(value.clone()) else {
                        continue;
                    };
                    if !scope.contains(uri)
                        || !scope.is_subset(&project)
                        || confirmed.is_some_and(|confirmed| !scope.is_subset(confirmed))
                    {
                        continue;
                    }
                    scope
                } else {
                    project
                        .iter()
                        .filter(|uri| confirmed.is_none_or(|sources| sources.contains(*uri)))
                        .cloned()
                        .collect()
                };
                let paused = if let Some(value) = diagnostic
                    .data
                    .as_ref()
                    .and_then(|data| data.get("macroPausedSources"))
                {
                    let Ok(paused) = serde_json::from_value::<BTreeSet<Url>>(value.clone()) else {
                        continue;
                    };
                    if !paused.is_subset(&project)
                        || !paused.is_disjoint(&scope)
                        || confirmed.is_some_and(|confirmed| !paused.is_subset(confirmed))
                    {
                        continue;
                    }
                    paused
                } else {
                    BTreeSet::new()
                };
                Some((scope, paused))
            } else {
                None
            };
            let invocation_signatures = invocation_scope
                .as_ref()
                .map(|(scope, paused)| {
                    signatures
                        .entry((scope.clone(), paused.clone()))
                        .or_insert_with(|| {
                            crate::signature::CommandSignatures::with_ambiguous_environments(
                                workspace
                                    .documents
                                    .iter()
                                    .filter(|(uri, doc)| {
                                        scope.contains(*uri)
                                            && doc.kind == crate::workspace::DocumentKind::Latex
                                    })
                                    .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
                                workspace
                                    .documents
                                    .iter()
                                    .filter(|(uri, doc)| {
                                        paused.contains(*uri)
                                            && doc.kind == crate::workspace::DocumentKind::Latex
                                    })
                                    .flat_map(|(_, doc)| {
                                        doc.syntax
                                            .definitions
                                            .iter()
                                            .map(|entry| entry.name.as_str())
                                    }),
                                workspace
                                    .documents
                                    .iter()
                                    .filter(|(uri, doc)| {
                                        paused.contains(*uri)
                                            && doc.kind == crate::workspace::DocumentKind::Latex
                                    })
                                    .flat_map(|(_, doc)| {
                                        doc.syntax
                                            .environments
                                            .iter()
                                            .map(|entry| entry.name.as_str())
                                    }),
                            )
                            .with_literal_sources(
                                workspace
                                    .documents
                                    .iter()
                                    .filter(|(uri, doc)| {
                                        paused.contains(*uri)
                                            && doc.kind == crate::workspace::DocumentKind::Latex
                                    })
                                    .map(|(_, doc)| (doc.text.source.as_str(), &doc.syntax)),
                            )
                        })
                })
                .map(|signatures| &*signatures);
            if literal {
                if let Some(fix) = crate::special_characters::fix(
                    doc,
                    diagnostic,
                    invocation_signatures,
                )
                .or_else(|| {
                    crate::right_delimiters::fix(doc, diagnostic, invocation_signatures).or_else(
                        || crate::delimiter_operands::fix(doc, diagnostic, invocation_signatures),
                    )
                }) {
                    if seen.insert((
                        diagnostic.range.start,
                        diagnostic.range.end,
                        fix.edit.new_text.clone(),
                    )) {
                        result.push(fix);
                    }
                }
                continue;
            }
            if matches!(diagnostic.code.as_ref(), Some(NumberOrString::String(code)) if matches!(code.as_str(), "missing-include" | "missing-input" | "missing-listing" | "missing-verbatim-input" | "missing-fancyverb-input" | "missing-minted-input" | "missing-image" | "missing-package" | "missing-class" | "missing-bibliography-style" | "missing-bibliography-database"))
            {
                let fixes = if diagnostic.code
                    == Some(NumberOrString::String("missing-image".into()))
                {
                    crate::image_fixes::fixes(workspace, root, uri, diagnostic, &mut comparisons)
                } else {
                    crate::file_fixes::fixes(
                        workspace,
                        root,
                        uri,
                        diagnostic,
                        invocation_signatures,
                        &mut comparisons,
                    )
                };
                for fix in fixes {
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
                if let Some(fix) = closing_environment_fix(
                    doc,
                    diagnostic,
                    invocation_signatures.expect("environment signature scope"),
                ) {
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
            let Some((kind, key)) = target(doc, diagnostic, invocation_signatures) else {
                continue;
            };
            if kind == Kind::Citation {
                if let Some(statuses) = source_statuses
                    .get_or_init(|| crate::bib_section_references::source_statuses(workspace, uri))
                {
                    let span = doc
                        .text
                        .offset(diagnostic.range.start)
                        .zip(doc.text.offset(diagnostic.range.end));
                    if span.is_some_and(|span| statuses.get(&span) == Some(&false)) {
                        for fix in crate::bib_source_citations::repairs(workspace, uri, diagnostic)
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
                    }
                    continue;
                }
            }
            let citation_scope = if kind == Kind::Citation {
                if let Some(resources) = citation_resources
                    .get_or_init(|| crate::bib_section_keys::occurrence_resources(workspace, uri))
                {
                    let (Some(start), Some(end)) = (
                        doc.text.offset(diagnostic.range.start),
                        doc.text.offset(diagnostic.range.end),
                    ) else {
                        continue;
                    };
                    let Some(resources) = resources.get(&(start, end)) else {
                        continue;
                    };
                    Some(resources)
                } else {
                    None
                }
            } else {
                None
            };
            let counter_conflicts = if kind == Kind::Counter {
                crate::counters::replacement_conflicts(doc, diagnostic.range)
            } else {
                BTreeSet::new()
            };
            let paired_end = if kind == Kind::Environment {
                let Some(end) = environment_end(
                    doc,
                    diagnostic.range,
                    invocation_signatures.expect("environment signature scope"),
                ) else {
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
            let mut length_conflicts = BTreeSet::new();
            let mut switch_owners = BTreeMap::<String, usize>::new();
            let boolean_test = kind == Kind::Boolean
                && crate::switch_diagnostics::boolean_test(doc, diagnostic.range);
            let mut total_entries = 0;
            for document in project
                .iter()
                .filter(|uri| {
                    citation_scope.is_none_or(|resources| resources.contains(*uri))
                        && (kind != Kind::Anchor
                            || workspace
                                .build_diagnostic_sources
                                .get(root)
                                .is_none_or(|sources| sources.contains(*uri)))
                })
                .filter_map(|uri| workspace.documents.get(uri))
            {
                let entries: &[crate::syntax::Entry] = match kind {
                    Kind::Citation => &document.syntax.citations,
                    Kind::Label => &document.syntax.labels,
                    Kind::Command | Kind::Length => &document.syntax.definitions,
                    Kind::Environment => &document.syntax.environments,
                    Kind::Counter => &document.syntax.counters,
                    Kind::Toggle => &document.syntax.toggles,
                    Kind::Boolean => &document.syntax.conditionals,
                    Kind::Anchor => &document.syntax.anchors,
                    Kind::Color => &document.syntax.colors,
                    Kind::ColorModel => &[],
                };
                for entry in entries {
                    if kind == Kind::Length && !crate::lengths::declaration(entry) {
                        length_conflicts.insert(entry.name.clone());
                    }
                    *candidates.entry(entry.name.clone()).or_default() += 1;
                    total_entries += 1;
                    if total_entries >= 4096 {
                        break;
                    }
                }
                if kind == Kind::Length {
                    // Environment, counter and switch declarations can own
                    // the same control word as an allocated length.
                    for name in crate::generated::commands(document).take(4096) {
                        length_conflicts.insert(name);
                        total_entries += 1;
                        if total_entries >= 4096 {
                            break;
                        }
                    }
                }
                if matches!(kind, Kind::Toggle | Kind::Boolean) {
                    // Toggle declarations own one internal command, booleans
                    // own a condition and two setters.
                    // Additional macros or generated commands can replace it.
                    for name in document
                        .syntax
                        .definitions
                        .iter()
                        .map(|entry| entry.name.clone())
                        .chain(crate::generated::commands(document))
                        .take(4096)
                    {
                        let owned = if kind == Kind::Toggle {
                            name.strip_prefix("etb@tgl@")
                        } else {
                            Some(name.as_str())
                        };
                        if let Some(name) = owned {
                            *switch_owners.entry(name.into()).or_default() += 1;
                        }
                        total_entries += 1;
                        if total_entries >= 4096 {
                            break;
                        }
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
            if kind == Kind::Length {
                for name in crate::lengths::BUILTINS {
                    candidates.entry((*name).into()).or_insert(1);
                }
            }
            if boolean_test {
                for name in ["true", "false"] {
                    candidates.entry(name.into()).or_insert(1);
                }
            }
            if kind == Kind::Color {
                for name in crate::named_colors::BUILTINS {
                    candidates.entry((*name).into()).or_insert(1);
                }
            }
            if kind == Kind::ColorModel {
                for model in crate::named_colors::MODELS {
                    candidates.entry((*model).into()).or_insert(1);
                }
            }
            // A known key can be unresolved because of build order or macros.
            if total_entries >= 4096
                || candidates.contains_key(key)
                || length_conflicts.contains(key)
                || kind == Kind::Toggle && switch_owners.contains_key(key)
                || kind == Kind::Boolean && switch_owners.contains_key(&format!("if{key}"))
            {
                continue;
            }
            let mut candidates: Vec<_> = candidates
                .into_iter()
                .filter(|(name, count)| {
                    *count == 1
                        && valid_name(kind, name)
                        && !counter_conflicts.contains(name.as_str())
                        && !length_conflicts.contains(name)
                        && (kind != Kind::Toggle || switch_owners.get(name) == Some(&1))
                        && (kind != Kind::Boolean
                            || if matches!(name.as_str(), "true" | "false") {
                                boolean_test && !switch_owners.contains_key(&format!("if{name}"))
                            } else {
                                crate::generated::conditional_names(name)
                                    .iter()
                                    .all(|control| switch_owners.get(control) == Some(&1))
                            })
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
                    Kind::Length => ("length", format!("\\{name}")),
                    Kind::Toggle => ("toggle", name.clone()),
                    Kind::Boolean => ("boolean", name.clone()),
                    Kind::Environment => ("environment", name.clone()),
                    Kind::Counter => ("counter", name.clone()),
                    Kind::Color => ("color", name.clone()),
                    Kind::ColorModel => ("color model", name.clone()),
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
                        if matches!(kind, Kind::Label | Kind::Anchor | Kind::Counter) {
                            &name
                        } else {
                            &replacement
                        }
                    ),
                    diagnostic: diagnostic.clone(),
                    edit: if kind == Kind::Counter {
                        crate::counters::replacement_edit(doc, diagnostic.range, &name)
                    } else {
                        TextEdit {
                            range: diagnostic.range,
                            new_text: replacement.clone(),
                        }
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
