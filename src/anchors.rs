//! Literal PDF destinations are independent of LaTeX reference labels.
use crate::{
    syntax::{self, Argument, Command, Entry},
    workspace::{Document, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use tower_lsp::lsp_types::{Diagnostic, NumberOrString, Range as LspRange, TextEdit, Url};

/// Destination warnings are emitted at shipout without a source line. Only map
/// names confirmed by the compiler, using inputs from this particular build.
pub(crate) fn resolve_diagnostics(
    workspace: &Workspace,
    sources: &BTreeSet<Url>,
    diagnostics: &mut BTreeMap<Url, Vec<Diagnostic>>,
) {
    let names: BTreeSet<_> = diagnostics
        .values()
        .flatten()
        .filter(|diagnostic| diagnostic.source.as_deref() == Some("latex-build"))
        .filter_map(|diagnostic| crate::build_log::undefined_destination_name(&diagnostic.message))
        .map(str::to_owned)
        .collect();
    if names.is_empty() {
        return;
    }
    let mut locations = BTreeMap::<&str, Vec<(Url, LspRange)>>::new();
    for uri in sources {
        let Some(doc) = workspace.documents.get(uri) else {
            continue;
        };
        for entry in &doc.syntax.anchor_references {
            if names.contains(&entry.name) {
                locations.entry(&entry.name).or_default().push((
                    uri.clone(),
                    doc.text.range(entry.span.start, entry.span.end),
                ));
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut result = BTreeMap::<Url, Vec<Diagnostic>>::new();
    for (uri, items) in std::mem::take(diagnostics) {
        for diagnostic in items {
            let mapped = (diagnostic.source.as_deref() == Some("latex-build"))
                .then(|| crate::build_log::undefined_destination_name(&diagnostic.message))
                .flatten()
                .and_then(|name| locations.get(name).map(|locations| (name, locations)));
            if let Some((name, locations)) = mapped {
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
                    precise.code = Some(NumberOrString::String("undefined-pdf-destination".into()));
                    precise.data = Some(serde_json::json!({"destination":name}));
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
    let declaration = matches!(command.name.as_str(), "hypertarget" | "hyperdef");
    let (name, span) = if matches!(command.name.as_str(), "hypertarget" | "hyperlink") {
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

/// Preserve the source separators and comments when a destination uses two fields.
pub(crate) fn rename_edits(doc: &Document, range: LspRange, new_name: &str) -> Vec<TextEdit> {
    for command in &doc.syntax.commands {
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
        return [(&destination.category, category), (&destination.key, key)]
            .into_iter()
            .filter(|(argument, value)| argument.value != *value)
            .map(|(argument, value)| TextEdit {
                range: doc.text.range(argument.span.start, argument.span.end),
                new_text: value.into(),
            })
            .collect();
    }
    vec![TextEdit {
        range,
        new_text: new_name.into(),
    }]
}

/// A single edit also lets legacy command clients distinguish compound fixes.
pub(crate) fn replacement_text(doc: &Document, range: LspRange, new_name: &str) -> Option<String> {
    let start = doc.text.offset(range.start)?;
    let end = doc.text.offset(range.end)?;
    let mut result = doc.text.source.get(start..end)?.to_owned();
    let mut edits = rename_edits(doc, range, new_name);
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
