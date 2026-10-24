//! Ordinary citations use their compiled default context unless explicitly assigned.
use crate::{syntax, workspace::Workspace};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::Url;

pub(crate) fn flat_prefix(value: &str) -> bool {
    value.chars().count() <= 256
        && !value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace() || "\\{}%#$&~^_".contains(ch))
}
pub(crate) fn static_context(source: &str, name: &str, end: usize) -> bool {
    (|| {
        let mut at = end;
        match name {
            "endrefcontext" => return Some(()),
            "begin" | "end" => {
                let (argument, next) =
                    syntax::group(source, syntax::skip_space(source, at), b'{', b'}')?;
                if argument.value != "refcontext" {
                    return None;
                }
                if name == "end" {
                    return Some(());
                }
                at = next;
            }
            "refcontext" | "newrefcontext" => {}
            _ => return None,
        }
        at = syntax::skip_space(source, at);
        if source.as_bytes().get(at) == Some(&b'[') {
            let end = syntax::optional_note_end(source, at)?;
            let options = &source[at + 1..end - 1];
            let mut seen = BTreeSet::new();
            for span in syntax::key_list_segments(options, 0..options.len()) {
                let option = options[span].trim();
                if option.is_empty() {
                    continue;
                }
                let (key, raw) = option.split_once('=')?;
                let key = key.trim();
                if !seen.insert(key) {
                    return None;
                }
                let raw = raw.trim();
                let value = if raw.starts_with('{') {
                    let (argument, end) = syntax::group(raw, 0, b'{', b'}')?;
                    if end != raw.len() {
                        return None;
                    }
                    argument.value
                } else {
                    raw.to_string()
                };
                if !(match key {
                    "labelprefix" => flat_prefix(&value),
                    "sorting" => syntax::is_static_key(&value),
                    _ => false,
                }) {
                    return None;
                }
            }
            at = end;
        }
        // A following group names a declared context; that path remains opaque.
        (source.as_bytes().get(syntax::skip_space(source, at)) != Some(&b'{')).then_some(())
    })()
    .is_some()
}
type Labels = BTreeMap<(u32, String), Option<String>>;
pub(crate) fn numeric_labels(ws: &Workspace, root: &Url) -> Option<(Labels, usize)> {
    let (contexts, mut work) = crate::auxiliary::numeric_citation_contexts(ws, root)?;
    if contexts
        .values()
        .flatten()
        .all(|(context, _)| context.split('/').nth(2) == Some(""))
    {
        return crate::auxiliary::numeric_citation_labels(ws, root);
    }
    let prefixes = crate::bib_bbl::prefixes(ws, root).map(|(prefixes, bytes)| {
        work += bytes;
        prefixes
    });
    let labels = contexts
        .into_iter()
        .map(|((section, key), value)| {
            let label = value.and_then(|(context, number)| {
                if context.split('/').nth(2) == Some("") {
                    return Some(number);
                }
                let prefix = prefixes.as_ref()?.get(&(section, context, key.clone()))?;
                let label = format!("{prefix}{number}");
                (label.chars().count() <= 256).then_some(label)
            });
            ((section, key), label)
        })
        .collect();
    Some((labels, work))
}
