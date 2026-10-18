use crate::{syntax, workspace::Workspace};
use std::{collections::BTreeMap, ops::Range, path::Path};
use tower_lsp::lsp_types::*;

pub fn complete(
    workspace: &Workspace,
    uri: &Url,
    command: &str,
    span: Range<usize>,
) -> Vec<CompletionItem> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let Some(extensions) = syntax::file_extensions(command) else {
        return vec![];
    };
    let prefix = &doc.text.source[span.clone()];
    if prefix.contains(['\\', '%', '#', '$', '&', '{', '}', '^', '~']) {
        return vec![];
    }
    let separator = prefix.rfind('/').map_or(0, |index| index + 1);
    let directory = &prefix[..separator];
    let name_prefix = &prefix[separator..];
    let suffix = &doc.text.source[span.end..];
    let suffix_length = suffix
        .find(['/', '{', '}', ',', '\\', '%', '\n', '\r'])
        .unwrap_or(suffix.len());
    let suffix_length = suffix[..suffix_length].trim_end().len();
    let existing_slash = suffix.as_bytes().get(suffix_length) == Some(&b'/');
    let existing_name = &doc.text.source[span.start + separator..span.end + suffix_length];
    let range = doc.text.range(span.start, span.end + suffix_length);
    let roots = workspace
        .root_document(uri)
        .map(|root| vec![root])
        .unwrap_or_else(|_| workspace.source_roots(uri));
    let mut candidates = BTreeMap::new();
    let mut directories = std::iter::once(uri)
        .chain(roots.iter().filter(|root| *root != uri))
        .filter_map(|origin| {
            origin
                .to_file_path()
                .ok()?
                .parent()
                .map(|path| path.to_path_buf())
        })
        .collect::<Vec<_>>();
    if command == "includegraphics" {
        directories.extend(crate::graphics::directories(workspace, uri));
    }
    for parent in directories {
        let Ok(entries) = std::fs::read_dir(parent.join(directory)) else {
            continue;
        };
        for entry in entries.take(4096).flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !name.starts_with(name_prefix)
                || name.starts_with('.') && !name_prefix.starts_with('.')
                || name.contains(['\\', '%', '#', '$', '&', '{', '}', '^', '~', '\n', '\r'])
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let folder = metadata.is_dir();
            if !folder && (!metadata.is_file() || existing_slash) {
                continue;
            }
            let extension = Path::new(&name)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("");
            if !folder
                && !extensions
                    .iter()
                    .any(|ext| extension.eq_ignore_ascii_case(ext))
            {
                continue;
            }
            let label = format!("{directory}{name}{}", if folder { "/" } else { "" });
            if candidates.contains_key(&label) {
                continue;
            }
            let mut new_text = format!("{directory}{name}");
            if folder {
                if !existing_slash {
                    new_text.push('/');
                }
            } else if !matches!(command, "includegraphics" | "addbibresource") {
                let explicit_extension = existing_name
                    .rsplit_once('.')
                    .is_some_and(|(_, typed)| extension.starts_with(typed));
                // Uppercase extensions cannot be inferred by TeX on a case-sensitive filesystem.
                if !explicit_extension && extensions.first() == Some(&extension) {
                    new_text.truncate(new_text.len() - extension.len() - 1);
                }
            }
            candidates.insert(
                label.clone(),
                CompletionItem {
                    label: label.clone(),
                    filter_text: Some(label.clone()),
                    sort_text: Some(format!("{}{label}", if folder { "0" } else { "1" })),
                    detail: Some(entry.path().display().to_string()),
                    kind: Some(if folder {
                        CompletionItemKind::FOLDER
                    } else {
                        CompletionItemKind::FILE
                    }),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit { range, new_text })),
                    ..Default::default()
                },
            );
        }
    }
    let mut result: Vec<_> = candidates.into_values().collect();
    result.sort_by(|a, b| a.sort_text.cmp(&b.sort_text));
    result.truncate(256);
    result
}
