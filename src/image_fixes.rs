//! Compiler-confirmed spelling corrections for static graphicx filenames.
use crate::{build_log, graphics, spelling_fixes, syntax, workspace::Workspace};
use std::{collections::BTreeSet, path::Path};
use tower_lsp::lsp_types::{Diagnostic, TextEdit, Url};

pub(crate) fn fixes(
    workspace: &Workspace,
    root: &Url,
    uri: &Url,
    diagnostic: &Diagnostic,
    comparisons: &mut usize,
) -> Vec<syntax::QuickFix> {
    if *comparisons >= spelling_fixes::MAX_COMPARISONS {
        return vec![];
    }
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let Some(reported) = build_log::missing_graphic_name(&diagnostic.message) else {
        return vec![];
    };
    if diagnostic
        .data
        .as_ref()
        .and_then(|data| data.get("file"))
        .and_then(|file| file.as_str())
        != Some(reported)
    {
        return vec![];
    }
    let argument = doc
        .syntax
        .commands
        .iter()
        .filter(|command| command.name == "includegraphics")
        .filter_map(|command| command.argument.as_ref())
        .find(|argument| {
            let span = syntax::trimmed_span(&doc.text.source, argument.span.clone());
            doc.text.range(span.start, span.end) == diagnostic.range
                && argument.value.trim() == reported
        });
    let Some(argument) = argument else {
        return vec![];
    };
    let value = argument.value.trim();
    if !syntax::is_literal_file_path(value) {
        return vec![];
    }
    let Some(parent) = root
        .to_file_path()
        .ok()
        .and_then(|path| path.parent().map(Path::to_owned))
    else {
        return vec![];
    };
    let mut directories = vec![parent];
    directories.extend(graphics::directories_in(workspace, uri, root));
    let extensions = syntax::file_extensions("includegraphics").unwrap();
    let resolve = |name: &str| {
        directories.iter().find_map(|directory| {
            extensions
                .iter()
                .find_map(|extension| graphics::resolve(directory, name, extension))
        })
    };
    // A compiler error that contradicts the current search paths may be stale.
    if resolve(value).is_some() {
        return vec![];
    }
    let separator = value.rfind('/').map_or(0, |index| index + 1);
    let prefix = &value[..separator];
    let original = &value[separator..];
    let explicit = Path::new(original).extension().is_some();
    let length = original.chars().count();
    if !(3..=128).contains(&length) {
        return vec![];
    }
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    let mut examined = 0;
    'directories: for directory in &directories {
        let directory = directory.join(prefix);
        let directory = directory.canonicalize().unwrap_or(directory);
        if !seen.insert(directory.clone()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in files.flatten() {
            examined += 1;
            if examined > 4096 || *comparisons >= spelling_fixes::MAX_COMPARISONS {
                break 'directories;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !syntax::is_literal_file_path(&name)
                || name.starts_with('.') && !original.starts_with('.')
                || !graphics::is_image(Path::new(&name))
            {
                continue;
            }
            let candidate = if explicit {
                name.as_str()
            } else {
                let Some(stem) = Path::new(&name).file_stem().and_then(|stem| stem.to_str()) else {
                    continue;
                };
                stem
            };
            *comparisons += 1;
            let Some(score) =
                spelling_fixes::distance(original, candidate, if length < 8 { 1 } else { 2 })
            else {
                continue;
            };
            if score == 0 || !entry.path().is_file() {
                continue;
            }
            // Keep the actual extension, matching file completion and avoiding
            // dependence on the driver's extension inference and search order.
            let replacement = format!("{prefix}{name}");
            if resolve(&replacement).is_some() {
                candidates.push((score, replacement));
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
        .into_iter()
        .take(5)
        .map(|(_, replacement)| syntax::QuickFix {
            title: format!("Replace with image file `{replacement}`"),
            diagnostic: diagnostic.clone(),
            edit: TextEdit {
                range: diagnostic.range,
                new_text: replacement,
            },
            additional_edits: vec![],
        })
        .collect()
}
