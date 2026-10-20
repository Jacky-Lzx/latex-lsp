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
                || command == "VerbatimInput" && ch == '"'
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
    let Some((range, mut additional_edits)) =
        syntax::name_completion_ranges(&doc.text, span.start..span.end + suffix_length, span.end)
    else {
        return vec![];
    };
    let filter_prefix = raw_prefix.rsplit('\n').next().unwrap_or(raw_prefix);
    let roots = workspace
        .root_document(uri)
        .map(|root| vec![root])
        .unwrap_or_else(|_| workspace.source_roots(uri));
    if syntax::code_input_command(command)
        && !doc.syntax.commands.iter().any(|candidate| {
            candidate.name == command
                && (if command == "VerbatimInput" {
                    crate::fancyvrb::prefix(&doc.text.source, candidate, span.end)
                        == Some(span.clone())
                } else {
                    syntax::file_name_argument_start(&doc.text.source, command, candidate.span.end)
                        == Some(span.start.saturating_sub(1))
                })
                && roots
                    .iter()
                    .all(|root| crate::code_inputs::supported(workspace, uri, root, candidate))
        })
    {
        return vec![];
    }
    let mut candidates = BTreeMap::new();
    let mut insertions = BTreeSet::new();
    let mut directories = roots
        .iter()
        .chain(std::iter::once(uri).filter(|uri| {
            !matches!(
                command,
                "includegraphics" | "lstinputlisting" | "verbatiminput" | "VerbatimInput"
            ) && !roots.contains(uri)
        }))
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
                || (compact || syntax::passes_options(command) || command == "bibliographystyle")
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
                && !matches!(
                    command,
                    "IfFileExists" | "lstinputlisting" | "verbatiminput" | "VerbatimInput"
                )
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
            } else if !matches!(
                command,
                "includegraphics"
                    | "addbibresource"
                    | "lstinputlisting"
                    | "verbatiminput"
                    | "VerbatimInput"
            ) {
                let explicit_extension = existing_name
                    .rsplit_once('.')
                    .is_some_and(|(_, typed)| extension.starts_with(typed));
                // Uppercase extensions cannot be inferred by TeX on a case-sensitive filesystem.
                if (syntax::package_class_extension(command).is_some() || !explicit_extension)
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
            let mut filter_start = "";
            if command == "input" {
                if let Some(argument) = input_argument {
                    let completed = format!(
                        "{new_text}{}",
                        &doc.text.source[span.end + suffix_length..argument_end]
                    );
                    if syntax::input_filename_needs_braces(&doc.text.source, argument, &completed) {
                        let edit = syntax::input_filename_edit(&doc.text, argument, completed);
                        edit_range = edit.range;
                        new_text = edit.new_text;
                        filter_start = &doc.text.source[argument.outer_span.start..span.start];
                    }
                }
            }
            if command == "VerbatimInput" {
                let candidate = doc.syntax.commands.iter().find(|candidate| {
                    candidate.name == command
                        && crate::fancyvrb::prefix(&doc.text.source, candidate, span.end)
                            == Some(span.clone())
                });
                let Some(edit) = candidate.and_then(|candidate| {
                    crate::fancyvrb::completion_edit(
                        &doc.text,
                        candidate,
                        new_text.clone(),
                        span.end + suffix_length,
                    )
                }) else {
                    continue;
                };
                let Some((range, extra)) = syntax::name_completion_ranges(
                    &doc.text,
                    doc.text.offset(edit.range.start).unwrap()
                        ..doc.text.offset(edit.range.end).unwrap(),
                    span.end,
                ) else {
                    continue;
                };
                edit_range = range;
                additional_edits = extra;
                new_text = edit.new_text;
                let from = doc.text.offset(edit_range.start).unwrap();
                filter_start = &doc.text.source[from..span.start];
            }
            candidates.insert(
                label.clone(),
                CompletionItem {
                    label: label.clone(),
                    // Clients filter against the text before the cursor. Keep
                    // its spelling while displaying the real target path.
                    filter_text: Some(format!(
                        "{filter_start}{filter_prefix}{}",
                        &label[prefix.len()..]
                    )),
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
