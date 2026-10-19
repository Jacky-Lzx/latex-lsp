//! Literal PDF destinations are independent of LaTeX reference labels.
use crate::{
    build_log::{self, DestinationWarningKind},
    syntax::{self, Argument, Command, Entry},
    workspace::{Document, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, Location, NumberOrString, Range as LspRange,
    TextEdit, Url,
};

/// Destination warnings are emitted at shipout without a source line. Only map
/// names confirmed by the compiler, using inputs from this particular build.
pub(crate) fn resolve_diagnostics(
    workspace: &Workspace,
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
) {
    let mut names = BTreeMap::<DestinationWarningKind, BTreeSet<&str>>::new();
    for (kind, name) in diagnostics
        .values()
        .flatten()
        .filter(|diagnostic| {
            diagnostic.source.as_deref() == Some("latex-build")
                && !build_log::unresolved_location(diagnostic)
        })
        .filter_map(|diagnostic| build_log::destination_warning(&diagnostic.message))
    {
        names.entry(kind).or_default().insert(name);
    }
    if names.is_empty() {
        return;
    }
    let mut locations = BTreeMap::<(DestinationWarningKind, &str), Vec<(Url, LspRange)>>::new();
    for uri in sources {
        let Some(doc) = workspace.documents.get(uri) else {
            continue;
        };
        for (kind, entries) in [
            (
                DestinationWarningKind::Missing,
                &doc.syntax.anchor_references,
            ),
            (DestinationWarningKind::Duplicate, &doc.syntax.anchors),
        ] {
            let Some(names) = names.get(&kind) else {
                continue;
            };
            for entry in entries {
                if names.contains(entry.name.as_str()) {
                    locations.entry((kind, &entry.name)).or_default().push((
                        uri.clone(),
                        doc.text.range(entry.span.start, entry.span.end),
                    ));
                }
            }
        }
    }
    drop(names);
    let mut seen = BTreeSet::new();
    let mut result = BTreeMap::<Url, Vec<Diagnostic>>::new();
    for (uri, items) in std::mem::take(diagnostics) {
        for diagnostic in items {
            let mapped = (diagnostic.source.as_deref() == Some("latex-build")
                && !build_log::unresolved_location(&diagnostic))
            .then(|| build_log::destination_warning(&diagnostic.message))
            .flatten()
            .and_then(|(kind, name)| {
                locations
                    .get(&(kind, name))
                    .map(|locations| (kind, name, locations))
            });
            if let Some((kind, name, locations)) = mapped {
                for (uri, range) in locations {
                    if !seen.insert((
                        uri.clone(),
                        range.start,
                        range.end,
                        diagnostic.message.clone(),
                    )) {
                        continue;
                    }
                    let mut precise = diagnostic.clone();
                    precise.range = *range;
                    precise.code = Some(NumberOrString::String(kind.code().into()));
                    precise.data = Some(serde_json::json!({"destination":name}));
                    if kind == DestinationWarningKind::Duplicate {
                        let related: Vec<_> = locations
                            .iter()
                            .filter(|(other, at)| other != uri || at != range)
                            .take(16)
                            .map(|(uri, range)| DiagnosticRelatedInformation {
                                location: Location::new(uri.clone(), *range),
                                message: format!(
                                    "Other explicit PDF destination `{name}` in this build"
                                ),
                            })
                            .collect();
                        precise.related_information = (!related.is_empty()).then_some(related);
                    }
                    result.entry(uri.clone()).or_default().push(precise);
                }
            } else {
                result.entry(uri.clone()).or_default().push(diagnostic);
            }
        }
    }
    *diagnostics = result;
}

struct Destination {
    name: String,
    span: Range<usize>,
    category: Argument,
    key: Argument,
}

struct Bookmark {
    name: String,
    level: Option<Argument>,
    key: Argument,
}

fn literal_level(level: &str) -> bool {
    let digits = level.strip_prefix(['+', '-']).unwrap_or(level);
    !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        // hyperref initializes bookmark parents down to level -4; a bookmark
        // itself must be at least -3 to have a defined parent.
        && level.parse::<i32>().is_ok_and(|value| value >= -3)
}

fn bookmark(source: &str, command: &Command) -> Option<Bookmark> {
    if command.name != "pdfbookmark" {
        return None;
    }
    let mut at = syntax::skip_space(source, command.span.end);
    let level = if source.as_bytes().get(at) == Some(&b'[') {
        let (level, after) = syntax::group(source, at, b'[', b']')?;
        if !literal_level(&level.value) {
            return None;
        }
        at = syntax::skip_space(source, after);
        Some(level)
    } else {
        None
    };
    let (_, after) = syntax::group(source, at, b'{', b'}')?;
    let (key, _) = syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?;
    if !syntax::is_static_key(&key.value) {
        return None;
    }
    let name = format!(
        "{}.{}",
        key.value,
        level.as_ref().map_or("0", |level| level.value.as_str())
    );
    Some(Bookmark { name, level, key })
}

fn compound(source: &str, command: &Command) -> Option<Destination> {
    let mut at = syntax::skip_space(source, command.span.end);
    match command.name.as_str() {
        "hyperdef" => {
            if source.as_bytes().get(at) == Some(&b'[') {
                at = syntax::skip_space(source, syntax::group(source, at, b'[', b']')?.1);
            }
        }
        "hyperref" => {
            let url = command.argument.as_ref()?;
            if source.as_bytes().get(at) != Some(&b'{')
                || url.outer_span.start != at
                || !url.value.is_empty()
            {
                return None;
            }
            at = syntax::skip_space(source, url.outer_span.end);
        }
        _ => return None,
    }
    let (category, after) = syntax::group(source, at, b'{', b'}')?;
    let (key, _) = syntax::group(source, syntax::skip_space(source, after), b'{', b'}')?;
    if [&category, &key]
        .iter()
        .any(|argument| !argument.value.is_empty() && !syntax::is_static_key(&argument.value))
    {
        return None;
    }
    let name = if category.value.is_empty() {
        key.value.clone()
    } else {
        format!("{}.{}", category.value, key.value)
    };
    if name.is_empty() {
        return None;
    }
    Some(Destination {
        name,
        span: category.span.start..key.span.end,
        category,
        key,
    })
}

pub(crate) fn entry(source: &str, command: &Command) -> Option<(bool, Entry)> {
    let declaration = matches!(
        command.name.as_str(),
        "hypertarget" | "hyperdef" | "pdfbookmark"
    );
    let (name, span) = if command.name == "pdfbookmark" {
        let bookmark = bookmark(source, command)?;
        (bookmark.name, bookmark.key.span)
    } else if matches!(command.name.as_str(), "hypertarget" | "hyperlink") {
        let (argument, _) = syntax::group(
            source,
            syntax::skip_space(source, command.span.end),
            b'{',
            b'}',
        )?;
        if !syntax::is_static_key(&argument.value) {
            return None;
        }
        (argument.value, argument.span)
    } else {
        let destination = compound(source, command)?;
        (destination.name, destination.span)
    };
    Some((
        declaration,
        Entry {
            name,
            span,
            detail: format!("PDF destination — \\{}", command.name),
            is_variable: false,
        },
    ))
}

/// Compound destinations have distinct name fields separated by TeX syntax.
pub(crate) fn name_spans(source: &str, command: &Command) -> Vec<(bool, Range<usize>)> {
    if let Some(destination) = compound(source, command) {
        let declaration = command.name == "hyperdef";
        return [destination.category.span, destination.key.span]
            .into_iter()
            .filter(|span| !span.is_empty())
            .map(|span| (declaration, span))
            .collect();
    }
    entry(source, command)
        .map(|(declaration, entry)| (declaration, entry.span))
        .into_iter()
        .collect()
}

/// Preserve the source separators and comments when a destination uses two fields.
pub(crate) fn rename_edits(
    doc: &Document,
    range: LspRange,
    new_name: &str,
) -> tower_lsp::jsonrpc::Result<Vec<TextEdit>> {
    for command in &doc.syntax.commands {
        if let Some(bookmark) = bookmark(&doc.text.source, command) {
            if doc
                .text
                .range(bookmark.key.span.start, bookmark.key.span.end)
                != range
            {
                continue;
            }
            let (base, level) = new_name.rsplit_once('.').filter(|(base, level)| syntax::is_static_key(base) && literal_level(level)).ok_or_else(|| tower_lsp::jsonrpc::Error::invalid_params("A PDF bookmark destination must end with a supported literal numeric level (at least -3), for example `name.0`"))?;
            let mut edits = Vec::new();
            if bookmark.key.value != base {
                edits.push(TextEdit {
                    range,
                    new_text: base.into(),
                });
            }
            if let Some(old) = bookmark.level {
                if old.value != level {
                    edits.push(TextEdit {
                        range: doc.text.range(old.span.start, old.span.end),
                        new_text: level.into(),
                    });
                }
            } else if level != "0" {
                edits.push(TextEdit {
                    range: doc.text.range(command.span.end, command.span.end),
                    new_text: format!("[{level}]"),
                });
            }
            return Ok(edits);
        }
        let Some(destination) = compound(&doc.text.source, command) else {
            continue;
        };
        if doc.text.range(destination.span.start, destination.span.end) != range {
            continue;
        }
        let prefix = format!("{}.", destination.category.value);
        let (category, key) =
            if !destination.category.value.is_empty() && new_name.starts_with(&prefix) {
                (
                    destination.category.value.as_str(),
                    &new_name[prefix.len()..],
                )
            } else {
                ("", new_name)
            };
        return Ok([(&destination.category, category), (&destination.key, key)]
            .into_iter()
            .filter(|(argument, value)| argument.value != *value)
            .map(|(argument, value)| TextEdit {
                range: doc.text.range(argument.span.start, argument.span.end),
                new_text: value.into(),
            })
            .collect());
    }
    Ok(vec![TextEdit {
        range,
        new_text: new_name.into(),
    }])
}

/// A single edit also lets legacy command clients distinguish compound fixes.
pub(crate) fn replacement_text(doc: &Document, range: LspRange, new_name: &str) -> Option<String> {
    let start = doc.text.offset(range.start)?;
    let end = doc.text.offset(range.end)?;
    let mut result = doc.text.source.get(start..end)?.to_owned();
    let mut edits = rename_edits(doc, range, new_name).ok()?;
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    for edit in edits {
        let from = doc.text.offset(edit.range.start)?;
        let to = doc.text.offset(edit.range.end)?;
        if from < start || to > end || from > to {
            return None;
        }
        result.replace_range(from - start..to - start, &edit.new_text);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_sources_cannot_supply_or_reuse_destination_locations() {
        let root = Url::parse("file:///anchors/main.tex").unwrap();
        let child = Url::parse("file:///anchors/child.tex").unwrap();
        let mut workspace = Workspace::default();
        workspace.documents.insert(
            child.clone(),
            Document::new(
                &child,
                "\\hyperlink{lost}{Text}\\hypertarget{copy}{Target}".into(),
                None,
            ),
        );
        for message in [
            "warning  (pdf backend): unreferenced destination with name 'lost'",
            "warning  (pdf backend): ignoring duplicate destination with the name 'copy'",
        ] {
            for flag in ["unresolvedSource", "unresolvedLine"] {
                let unresolved = Diagnostic {
                    source: Some("latex-build".into()),
                    message: message.into(),
                    data: Some(serde_json::json!({flag:true})),
                    ..Default::default()
                };
                for trusted in [false, true] {
                    let mut items = vec![unresolved.clone()];
                    if trusted {
                        items.push(Diagnostic {
                            data: None,
                            ..unresolved.clone()
                        });
                    }
                    let mut diagnostics = [(root.clone(), items)].into();
                    resolve_diagnostics(&workspace, &[child.clone()].into(), &mut diagnostics);
                    assert_eq!(diagnostics[&root], vec![unresolved.clone()]);
                    assert_eq!(diagnostics.contains_key(&child), trusted);
                    if trusted {
                        assert_eq!(diagnostics[&child].len(), 1);
                        assert!(diagnostics[&child][0].code.is_some());
                    }
                }
            }
        }
    }

    #[test]
    fn duplicate_compiler_destinations_link_only_the_recorded_explicit_declarations() {
        let root = Url::parse("file:///anchors/main.tex").unwrap();
        let child = Url::parse("file:///anchors/child.tex").unwrap();
        let excluded = Url::parse("file:///anchors/excluded.tex").unwrap();
        let mut workspace = Workspace::default();
        for (uri, source) in [(&root, "😀 \\hypertarget{section.1}{Target}\\hyperlink{section.1}{Link}\\label{section.1}\n"), (&child, "\\hyperdef[label]{section}% keep\r\n{1}{Target}\n"), (&excluded, "\\hypertarget{section.1}{Excluded}")] {
            workspace.documents.insert(uri.clone(), Document::new(uri, source.into(), None));
        }
        let warning = Diagnostic {
            source: Some("latex-build".into()),
            message:
                "warning  (pdf backend): ignoring duplicate destination with the name 'section.1'"
                    .into(),
            ..Default::default()
        };
        let unknown = Diagnostic {
            message:
                "warning  (pdf backend): ignoring duplicate destination with the name 'generated.1'"
                    .into(),
            ..warning.clone()
        };
        let mut diagnostics = [(
            root.clone(),
            vec![warning.clone(), warning, unknown.clone()],
        )]
        .into();
        resolve_diagnostics(
            &workspace,
            &[root.clone(), child.clone()].into(),
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 2);
        assert!(!diagnostics.contains_key(&excluded));
        assert_eq!(diagnostics[&root].len(), 2);
        assert!(diagnostics[&root].contains(&unknown));
        assert_eq!(diagnostics[&child].len(), 1);
        for (uri, other) in [(&root, &child), (&child, &root)] {
            let doc = &workspace.documents[uri];
            let entry = &doc.syntax.anchors[0];
            let item = &diagnostics[uri][0];
            assert_eq!(item.range, doc.text.range(entry.span.start, entry.span.end));
            assert_eq!(
                item.code,
                Some(NumberOrString::String("duplicate-pdf-destination".into()))
            );
            let related = item.related_information.as_ref().unwrap();
            assert_eq!(related.len(), 1);
            assert_eq!(&related[0].location.uri, other);
        }
    }

    #[test]
    fn compiler_destinations_map_only_to_recorded_sources_and_merge_repetitions() {
        let root = Url::parse("file:///anchors/main.tex").unwrap();
        let child = Url::parse("file:///anchors/child.tex").unwrap();
        let excluded = Url::parse("file:///anchors/excluded.tex").unwrap();
        let mut workspace = Workspace::default();
        for uri in [&root, &child, &excluded] {
            workspace.documents.insert(uri.clone(), Document::new(uri, "😀 \\hyperlink{lost}{Text}\r\n\\hyperref{}{}% preserve\r\n{lost}{Text}\\label{lost}\n".into(), None));
        }
        let warning = Diagnostic {
            source: Some("latex-build".into()),
            message: "warning  (pdf backend): unreferenced destination with name 'lost'".into(),
            ..Default::default()
        };
        let unknown = Diagnostic {
            message: "warning  (pdf backend): unreferenced destination with name 'unknown'".into(),
            ..warning.clone()
        };
        let external = Diagnostic {
            source: Some("other".into()),
            ..warning.clone()
        };
        let mut diagnostics = [(
            root.clone(),
            vec![warning.clone(), warning, unknown.clone(), external.clone()],
        )]
        .into();
        resolve_diagnostics(
            &workspace,
            &[root.clone(), child.clone()].into(),
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 2);
        assert!(!diagnostics.contains_key(&excluded));
        assert_eq!(diagnostics[&root].len(), 4);
        assert_eq!(diagnostics[&child].len(), 2);
        assert!(diagnostics[&root].contains(&unknown));
        assert!(diagnostics[&root].contains(&external));
        for uri in [&root, &child] {
            let doc = &workspace.documents[uri];
            for (diagnostic, entry) in diagnostics[uri]
                .iter()
                .take(2)
                .zip(&doc.syntax.anchor_references)
            {
                assert_eq!(
                    diagnostic.range,
                    doc.text.range(entry.span.start, entry.span.end)
                );
                assert_eq!(
                    diagnostic.data,
                    Some(serde_json::json!({"destination":"lost"}))
                );
            }
        }
    }
}
