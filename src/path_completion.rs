use crate::{syntax, workspace::Workspace};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::Path,
};
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
    let raw_prefix = &doc.text.source[span.clone()];
    let compact = syntax::file_name_ignores_spaces(command);
    let normalized_prefix = compact.then(|| syntax::loader_name(raw_prefix));
    let prefix = normalized_prefix.as_deref().unwrap_or(raw_prefix);
    if prefix.contains(['\\', '%', '#', '$', '&', '{', '}', '^', '~']) {
        return vec![];
    }
    let separator = prefix.rfind('/').map_or(0, |index| index + 1);
    let directory = &prefix[..separator];
    let name_prefix = &prefix[separator..];
    let input_argument = matches!(command, "input" | "InputIfFileExists" | "IfFileExists")
        .then(|| {
            doc.syntax
                .commands
                .iter()
                .filter(|candidate| candidate.name == command)
                .filter_map(|command| command.argument.as_ref())
                .find(|argument| argument.span.start == span.start && span.end <= argument.span.end)
        })
        .flatten();
    let argument_end = input_argument.map_or(doc.text.source.len(), |argument| argument.span.end);
    let suffix = &doc.text.source[span.end..argument_end];
    let suffix_length = suffix
        .find(|ch| {
            matches!(ch, '/' | '{' | '}' | '\\' | '%')
                || ch == ',' && syntax::file_list(command)
                || !compact && matches!(ch, '\n' | '\r')
        })
        .unwrap_or(suffix.len());
    let suffix_length = suffix[..suffix_length].trim_end().len();
    let existing_slash = suffix.as_bytes().get(suffix_length) == Some(&b'/');
    // Normalization changes byte lengths; edits and suffix inspection still
    // use offsets in the original document, including UTF-8 and CRLF.
    let raw_separator = raw_prefix.rfind('/').map_or(0, |index| index + 1);
    let raw_name = &doc.text.source[span.start + raw_separator..span.end + suffix_length];
    let normalized_name = compact.then(|| syntax::loader_name(raw_name));
    let existing_name = normalized_name.as_deref().unwrap_or(raw_name);
    let Some((range, additional_edits)) =
        syntax::name_completion_ranges(&doc.text, span.start..span.end + suffix_length, span.end)
    else {
        return vec![];
    };
    let filter_prefix = raw_prefix.rsplit('\n').next().unwrap_or(raw_prefix);
    let roots = workspace
        .root_document(uri)
        .map(|root| vec![root])
        .unwrap_or_else(|_| workspace.source_roots(uri));
    let mut candidates = BTreeMap::new();
    let mut insertions = BTreeSet::new();
    let mut directories = roots
        .iter()
        .chain(
            std::iter::once(uri).filter(|uri| command != "includegraphics" && !roots.contains(uri)),
        )
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
        let Ok(origin) = Url::from_file_path(parent.join("__latex_lsp_completion__.tex")) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(parent.join(directory)) else {
            continue;
        };
        for entry in entries.take(4096).flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !name.starts_with(name_prefix)
                || name.starts_with('.') && !name_prefix.starts_with('.')
                || (compact || command == "bibliographystyle")
                    && name.contains([' ', '\t', '\r', '\n'])
                || name.contains([
                    '\\', '%', '#', '$', '&', '{', '}', '^', '~', '"', '\n', '\r',
                ])
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
                && command != "IfFileExists"
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
                if (syntax::loader_extension(command).is_some() || !explicit_extension)
                    && extensions.first() == Some(&extension)
                {
                    new_text.truncate(new_text.len() - extension.len() - 1);
                }
            }
            if !folder {
                let resolved = crate::workspace::resolve_file_path(
                    &origin,
                    command,
                    &new_text,
                    syntax::inferred_file_extensions(command).unwrap(),
                );
                let candidate = entry
                    .path()
                    .canonicalize()
                    .ok()
                    .and_then(|path| Url::from_file_path(path).ok());
                if resolved != candidate {
                    continue;
                }
            }
            if !insertions.insert(new_text.clone()) {
                continue;
            }
            let mut edit_range = range;
            if input_argument.is_some_and(|argument| argument.outer_span == argument.span)
                && new_text.chars().any(char::is_whitespace)
            {
                new_text.push_str(&doc.text.source[span.end + suffix_length..argument_end]);
                new_text = format!("{{{new_text}}}");
                edit_range = doc.text.range(span.start, argument_end);
            }
            candidates.insert(
                label.clone(),
                CompletionItem {
                    label: label.clone(),
                    // Clients filter against the text before the cursor. Keep
                    // its spelling while displaying the real target path.
                    filter_text: Some(format!("{filter_prefix}{}", &label[prefix.len()..])),
                    sort_text: Some(format!("{}{label}", if folder { "0" } else { "1" })),
                    detail: Some(entry.path().display().to_string()),
                    kind: Some(if folder {
                        CompletionItemKind::FOLDER
                    } else {
                        CompletionItemKind::FILE
                    }),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: edit_range,
                        new_text,
                    })),
                    additional_text_edits: (!additional_edits.is_empty())
                        .then(|| additional_edits.clone()),
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
