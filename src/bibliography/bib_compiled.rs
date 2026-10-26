//! Compiled numeric labels keep their actual refsection and refcontext.
use crate::{
    bibliography::bib_section_keys::{self, SourceEvent},
    parser::syntax,
    project::workspace::{Document, DocumentKind, Workspace},
    symbols::commands::CommandSignatures,
    symbols::named_references::Context,
};
use lsp_types::{Position, Range, Url};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Classic,
    Unknown,
    Numeric(bool),
}

fn numeric_style(value: &str) -> bool {
    matches!(value, "numeric" | "numeric-comp" | "numeric-verb")
}

fn options(source: &str, end: usize) -> Option<(bool, bool)> {
    let at = syntax::skip_space(source, end);
    if source.as_bytes().get(at) != Some(&b'[') {
        return Some((false, false));
    }
    let end = syntax::optional_note_end(source, at)?;
    let source = &source[at + 1..end - 1];
    if source.contains(['%', '\\', '#']) {
        return None;
    }
    let mut numeric = false;
    let mut deferred = false;
    for span in syntax::key_list_segments(source, 0..source.len()) {
        let option = source[span].trim();
        if option.is_empty() {
            continue;
        }
        let (key, value) = option.split_once('=').unwrap_or((option, "true"));
        let key = key.trim();
        let value = value.trim();
        let value = if value.starts_with('{') {
            let (argument, end) = syntax::group(value, 0, b'{', b'}')?;
            if end != value.len() {
                return None;
            }
            argument.value
        } else {
            value.into()
        };
        if matches!(key, "style" | "citestyle" | "bibstyle") {
            if !numeric_style(&value) {
                return None;
            }
            numeric |= key != "bibstyle";
        } else if key == "defernumbers" {
            deferred = match value.as_str() {
                "true" => true,
                "false" => false,
                _ => return None,
            };
        } else if !matches!(
            key,
            "sorting"
                | "backend"
                | "maxnames"
                | "minnames"
                | "maxcitenames"
                | "mincitenames"
                | "maxbibnames"
                | "minbibnames"
                | "url"
                | "doi"
                | "isbn"
                | "eprint"
                | "hyperref"
                | "autocite"
        ) {
            return None;
        }
    }
    Some((numeric, deferred))
}

fn mode(docs: &[(Url, &Document)], signatures: &CommandSignatures<'_>) -> Mode {
    let mut backend = false;
    let mut deferred = false;
    let mut numeric = false;
    let mut uncertain = false;
    let mut packages = 0usize;
    let mut contexts = false;
    for (_, doc) in docs {
        let source = &doc.text.source;
        let context = Context::new(doc, signatures);
        let positions = signatures.source_command_positions(source, &doc.syntax);
        let definitions = crate::symbols::commands::definition_data_spans(source, &doc.syntax);
        let assignment_bodies = signatures.builtin_alias_body_positions(
            source,
            &doc.syntax,
            crate::bibliography::bib_context_assignments::COMMANDS,
        );
        for (index, command) in doc.syntax.commands.iter().enumerate() {
            let (name, span) = signatures
                .named_invocation(
                    source,
                    &doc.syntax,
                    index,
                    &BTreeSet::new(),
                    &context.declarations,
                )
                .unwrap_or((&command.name, command.span.clone()));
            let canonical = signatures.forwarded_builtin(name).unwrap_or(name);
            let refcontext = canonical.contains("refcontext")
                || matches!(canonical, "begin" | "end")
                    && syntax::environment_name_argument(source, span.end)
                        .is_some_and(|argument| argument.value == "refcontext");
            let definition = definitions
                .iter()
                .any(|span| span.contains(&command.span.start));
            // Pure forwarding bodies and their declaration targets are data.
            // The hint trace separately certifies each declaration's execution.
            if refcontext
                && (!context.literal(command.span.start) || definition)
                && !assignment_bodies.contains(&command.span.start)
                && !(context
                    .declarations
                    .contains(&(command.span.start, command.span.end))
                    && signatures
                        .forwarded_builtin(&command.name)
                        .and_then(crate::bibliography::bib_context_assignments::target)
                        .is_some())
            {
                let safe = !definition
                    && (crate::bibliography::bib_context_assignments::target(canonical).is_some()
                        || command.name == canonical && name == canonical)
                    && positions.contains(&command.span.start)
                    && !context.operand(doc, command.span.start)
                    && signatures.command_unchanged(canonical)
                    && signatures.command_unchanged("refcontext")
                    && signatures.command_unchanged("endrefcontext")
                    && signatures.environment_primitives_unchanged()
                    && !docs.iter().any(|(_, doc)| {
                        doc.syntax
                            .environments
                            .iter()
                            .any(|entry| entry.name == "refcontext")
                    })
                    && (crate::bibliography::bib_context::static_context(
                        source, canonical, span.end,
                    ) || crate::bibliography::bib_context_assignments::target(canonical)
                        .is_some()
                        && crate::bibliography::bib_context_assignments::parse(
                            source, span.end, canonical,
                        )
                        .is_some());
                uncertain |= !safe;
                contexts |= safe;
            }
            if !uncertain
                && (matches!(
                    canonical,
                    "ExecuteBibliographyOptions"
                        | "DeclareFieldFormat"
                        | "DeclareFieldAlias"
                        | "DeclareCiteCommand"
                        | "DeclareMultiCiteCommand"
                        | "DeclareAutoCiteCommand"
                        | "renewbibmacro"
                        | "DeclareSourcemap"
                        | "DeclareStyleSourcemap"
                        | "PassOptionsToPackage"
                        | "PassOptionsToClass"
                        | "AtEveryCitekey"
                        | "AtEveryBibitem"
                        | "AtDataInput"
                        | "AtEveryEntrykey"
                        | "AtBeginBibliography"
                        | "clearfield"
                        | "restorefield"
                        | "DeclareFieldInputHandler"
                ))
                && (!context.literal(command.span.start)
                    || definitions
                        .iter()
                        .any(|span| span.contains(&command.span.start)))
            {
                uncertain = true;
            }
            if context.literal(command.span.start)
                || context.operand(doc, command.span.start)
                || !positions.contains(&command.span.start)
                || !signatures.command_unchanged(canonical)
            {
                continue;
            }
            backend |= matches!(
                canonical,
                "addbibresource" | "addglobalbib" | "addsectionbib"
            );
            if syntax::loader_extension(canonical) != Some("sty") {
                continue;
            }
            let resolved = syntax::Command {
                name: canonical.into(),
                span: span.clone(),
                argument: command.argument.clone(),
            };
            let Some(argument) = syntax::file_argument(source, &resolved) else {
                continue;
            };
            let names: Vec<_> = argument.value.split(',').map(syntax::loader_name).collect();
            if !names.iter().any(|name| name == "biblatex") {
                continue;
            }
            backend = true;
            packages += 1;
            uncertain |=
                canonical != name || names.len() != 1 || canonical == "RequirePackageWithOptions";
            match options(source, span.end) {
                Some((value, defer)) => {
                    numeric |= value;
                    deferred = defer;
                }
                None => uncertain = true,
            }
        }
    }
    if !backend {
        Mode::Classic
    } else if uncertain || packages != 1 || !numeric || contexts && !deferred {
        Mode::Unknown
    } else {
        Mode::Numeric(deferred)
    }
}

/// Some owns the backend namespace even when its compiled labels are unknown.
/// Traditional bibcite records cannot stand in for biblatex data labels.
pub(crate) fn numbers(
    ws: &Workspace,
    uri: &Url,
    range: Range,
) -> Option<Vec<(Position, String, String)>> {
    let doc = ws.documents.get(uri)?;
    if doc.kind != DocumentKind::Latex {
        return None;
    }
    if !ws.documents.values().any(|doc| {
        doc.kind == DocumentKind::Latex
            && (doc.text.source.contains("biblatex")
                || doc.text.source.contains("addbib")
                || doc.text.source.contains("addsectionbib"))
    }) {
        return None;
    }
    let Some((_, projects)) = bib_section_keys::section_projects(ws, uri) else {
        return Some(Vec::new());
    };
    let modes: Vec<_> = projects
        .iter()
        .map(|project| mode(&project.docs, &project.signatures))
        .collect();
    if modes.iter().all(|mode| *mode == Mode::Classic) {
        return None;
    }
    if modes.iter().any(|mode| !matches!(mode, Mode::Numeric(_))) {
        return Some(Vec::new());
    }
    let printing: BTreeSet<_> =
        crate::diagnostics::reference_diagnostics::section_hint_occurrences(ws, uri)
            .into_iter()
            .filter(|(citation, _)| *citation)
            .map(|(_, span)| (span.start, span.end))
            .collect();
    if printing.is_empty() {
        return Some(Vec::new());
    }
    let mut common = None::<BTreeMap<(usize, usize), Option<String>>>;
    let mut work = 0usize;
    for (project, mode) in projects.into_iter().zip(modes) {
        let compiled = if mode == Mode::Numeric(true) {
            crate::bibliography::bib_context::numeric_labels(ws, &project.root)
        } else {
            crate::bibliography::bib_bbl::numeric_labels(ws, &project.root)
        };
        let Some((labels, bytes)) = compiled else {
            return Some(Vec::new());
        };
        work += bytes;
        if work > 32 * 1024 * 1024 {
            return Some(Vec::new());
        }
        let Some(trace) =
            bib_section_keys::hint_trace(ws, &project.root, &project.docs, &project.signatures)
        else {
            return Some(Vec::new());
        };
        work += trace.work;
        if work > 32 * 1024 * 1024 {
            return Some(Vec::new());
        }
        if trace
            .resources
            .values()
            .flat_map(|resources| resources.iter())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .any(|uri| {
                let doc = &ws.documents[uri];
                doc.syntax
                    .bibliography_completion
                    .types
                    .iter()
                    .any(|entry| {
                        doc.text.source[entry.span.clone()].eq_ignore_ascii_case("preamble")
                    })
            })
        {
            return Some(Vec::new());
        }
        let has_assignments = trace
            .assignments
            .as_ref()
            .is_some_and(|assignments| !assignments.is_empty());
        if !has_assignments && labels.values().all(Option::is_none) {
            return Some(Vec::new());
        }
        let has_categories = trace.assignments.as_ref().is_some_and(|assignments| {
            assignments.iter().any(|assignment| {
                assignment.target
                    == crate::bibliography::bib_context_assignments::Target::Categories
            })
        });
        let category_members = if has_categories {
            match crate::bibliography::bib_context_categories::certify(
                ws,
                &project.root,
                &project.docs,
                &project.signatures,
                &trace.category_members,
                &mut work,
            ) {
                Some(members) => members,
                None => return Some(Vec::new()),
            }
        } else {
            BTreeMap::new()
        };
        let has_keywords = trace.assignments.as_ref().is_some_and(|assignments| {
            assignments.iter().any(|assignment| {
                assignment.target == crate::bibliography::bib_context_assignments::Target::Keywords
            })
        });
        let keyword_members = if has_keywords {
            match crate::bibliography::bib_bbl::keyword_members(ws, &project.root) {
                Some((members, bytes)) => {
                    work += bytes;
                    members
                }
                None => return Some(Vec::new()),
            }
        } else {
            BTreeMap::new()
        };
        let global_sorting = has_assignments
            .then(|| {
                crate::bibliography::bib_context_assignments::global_sorting(
                    &project.docs,
                    &project.signatures,
                )
            })
            .flatten();
        let assigned_labels = if has_assignments {
            (|| {
                let (records, bytes) =
                    crate::compilation::auxiliary::numeric_citation_records(ws, &project.root)?;
                work += bytes;
                let (prefixes, bytes) = crate::bibliography::bib_bbl::prefixes(ws, &project.root)?;
                work += bytes;
                let compiled = crate::bibliography::bib_context_assignments::Compiled::new(
                    &records.numbers,
                    prefixes,
                )?;
                work += compiled.work;
                Some(compiled)
            })()
        } else {
            None
        };
        if work > 32 * 1024 * 1024 {
            return Some(Vec::new());
        }
        let mut assignments = trace.assignments.unwrap_or_default().into_iter().peekable();
        let mut active_assignments = BTreeMap::<
            (
                u32,
                crate::bibliography::bib_context_assignments::Target,
                bool,
                String,
            ),
            std::sync::Arc<crate::bibliography::bib_context_assignments::Selection>,
        >::new();
        let mut group_order = BTreeMap::new();
        let mut next_group = 0usize;
        let mut occurrences = BTreeMap::new();
        for (index, event) in trace.events.into_iter().enumerate() {
            while assignments
                .peek()
                .is_some_and(|assignment| assignment.before <= index)
            {
                let assignment = assignments.next().unwrap();
                let selection = std::sync::Arc::new(assignment.selection);
                for key in assignment.keys {
                    work += key.len() + 64;
                    if work > 32 * 1024 * 1024 || active_assignments.len() >= 100_000 {
                        return Some(Vec::new());
                    }
                    if assignment.target
                        != crate::bibliography::bib_context_assignments::Target::Entries
                    {
                        next_group += 1;
                        group_order.insert((assignment.target, key.clone()), next_group);
                    }
                    active_assignments.insert(
                        (assignment.section, assignment.target, assignment.weak, key),
                        selection.clone(),
                    );
                }
            }
            let SourceEvent::Request(section, key, false) = event else {
                continue;
            };
            let span = (key.span.start, key.span.end);
            if key.uri != *uri || !printing.contains(&span) {
                continue;
            }
            use crate::bibliography::bib_context_assignments::Target;
            let weak = !trace.explicit_requests.contains(&index);
            let mut automatic = None;
            // The category pass overrides the keyword pass, regardless of
            // strength or when the two commands were executed. Each pass has
            // its own global append order and section-local assignments.
            for (target, members) in [
                (
                    Target::Keywords,
                    keyword_members.get(&(section, key.name.clone())),
                ),
                (Target::Categories, category_members.get(&key.name)),
            ] {
                work += members.map_or(0, |members| {
                    members.iter().map(|name| name.len() + 96).sum::<usize>()
                });
                if work > 32 * 1024 * 1024 {
                    return Some(Vec::new());
                }
                let selected = members
                    .into_iter()
                    .flatten()
                    .filter_map(|name| {
                        let selection = active_assignments
                            .get(&(section, target, false, name.clone()))
                            .or_else(|| {
                                weak.then(|| {
                                    active_assignments.get(&(section, target, true, name.clone()))
                                })
                                .flatten()
                            })?;
                        Some((group_order.get(&(target, name.clone()))?, selection))
                    })
                    .max_by_key(|(order, _)| *order)
                    .map(|(_, selection)| selection);
                if selected.is_some() {
                    automatic = selected;
                }
            }
            let selected = active_assignments
                .get(&(section, Target::Entries, false, "*".to_string()))
                .or_else(|| {
                    active_assignments.get(&(section, Target::Entries, false, key.name.clone()))
                })
                .or_else(|| {
                    weak.then(|| {
                        active_assignments
                            .get(&(section, Target::Entries, true, "*".to_string()))
                            .or_else(|| {
                                active_assignments.get(&(
                                    section,
                                    Target::Entries,
                                    true,
                                    key.name.clone(),
                                ))
                            })
                    })
                    .flatten()
                })
                .or(automatic);
            let number = if let Some(selection) = selected {
                assigned_labels.as_ref().and_then(|compiled| {
                    compiled.label(section, &key.name, selection, global_sorting.as_deref())
                })
            } else {
                labels.get(&(section, key.name)).cloned().flatten()
            };
            work += 64 + number.as_ref().map_or(0, String::len);
            if work > 32 * 1024 * 1024 {
                return Some(Vec::new());
            }
            match occurrences.entry(span) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(number);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if entry.get() != &number {
                        entry.insert(None);
                    }
                }
            }
        }
        if let Some(common) = &mut common {
            for (span, number) in common {
                if occurrences.get(span) != Some(number) {
                    *number = None;
                }
            }
        } else {
            common = Some(occurrences);
        }
    }
    Some(
        common
            .unwrap_or_default()
            .into_iter()
            .filter_map(|((start, end), number)| {
                let position = doc.text.position(end);
                if position < range.start || position > range.end {
                    return None;
                }
                Some((position, doc.text.source[start..end].to_string(), number?))
            })
            .take(1024)
            .collect(),
    )
}

#[cfg(test)]
#[path = "bib_compiled_tests.rs"]
mod tests;
