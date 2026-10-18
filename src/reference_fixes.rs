//! Suggest bounded spelling corrections from the diagnostic's own project.
use crate::{build_log, syntax::QuickFix, workspace::Workspace};
use std::collections::{BTreeMap, BTreeSet};
use tower_lsp::lsp_types::{Range, TextEdit, Url};

fn distance(left: &str, right: &str, limit: usize) -> Option<usize> {
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
                || diagnostic.range.start > range.end
                || range.start > diagnostic.range.end
            {
                continue;
            }
            examined += 1;
            if examined > 64 {
                return result;
            }
            let Some((citation, key)) = build_log::undefined_reference_key(&diagnostic.message)
            else {
                continue;
            };
            let length = key.chars().count();
            if !(3..=128).contains(&length)
                || !crate::syntax::is_static_key(key)
                || !build_log::undefined_reference_ranges(
                    &doc.text,
                    diagnostic.range.start.line,
                    &diagnostic.message,
                )
                .contains(&diagnostic.range)
            {
                continue;
            }
            let mut candidates = BTreeMap::<String, usize>::new();
            let mut total_entries = 0;
            for document in project
                .iter()
                .filter_map(|uri| workspace.documents.get(uri))
            {
                let entries = if citation {
                    &document.syntax.citations
                } else {
                    &document.syntax.labels
                };
                for entry in entries {
                    *candidates.entry(entry.name.clone()).or_default() += 1;
                    total_entries += 1;
                    if total_entries >= 4096 {
                        break;
                    }
                }
                if total_entries >= 4096 {
                    break;
                }
            }
            // A known key can be unresolved because of build order or macros.
            if total_entries >= 4096 || candidates.contains_key(key) {
                continue;
            }
            let mut candidates: Vec<_> = candidates
                .into_iter()
                .filter(|(name, count)| *count == 1 && crate::syntax::is_static_key(name))
                .filter_map(|(name, _)| {
                    comparisons += 1;
                    if comparisons > 16384 {
                        return None;
                    }
                    distance(key, &name, if length < 8 { 1 } else { 2 }).map(|score| (score, name))
                })
                .collect();
            candidates.sort();
            for (_, name) in candidates.into_iter().take(5) {
                if !seen.insert((diagnostic.range.start, diagnostic.range.end, name.clone())) {
                    continue;
                }
                result.push(QuickFix {
                    title: format!(
                        "Replace with {} `{name}`",
                        if citation { "citation" } else { "label" }
                    ),
                    diagnostic: diagnostic.clone(),
                    edit: TextEdit {
                        range: diagnostic.range,
                        new_text: name,
                    },
                });
                if result.len() >= 256 {
                    return result;
                }
            }
        }
    }
    result
}
