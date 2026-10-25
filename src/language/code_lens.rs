//! Resolved build actions using the same root selection as explicit commands.
use crate::project::workspace::{self, DocumentKind, Workspace};
use lsp_types::{CodeLens, Command, Position, Range, Url};
use std::collections::BTreeMap;

pub fn lenses(workspace: &Workspace, uri: &Url) -> Vec<CodeLens> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    if doc.kind != DocumentKind::Latex || uri.to_file_path().is_err() {
        return vec![];
    }
    let explicit =
        workspace.config.build.root_file.is_some() || workspace::root_directive(doc).is_some();
    let roots = match workspace.root_document(uri) {
        Ok(root) if explicit || root != *uri || workspace.is_root(uri) => {
            BTreeMap::from([(root, uri.clone())])
        }
        Ok(_) => return vec![],
        Err(_) if explicit => return vec![],
        Err(_) => {
            let mut roots = BTreeMap::new();
            for candidate in workspace.source_roots(uri) {
                if candidate == *uri || !workspace.is_root(&candidate) {
                    continue;
                }
                if let Ok(root) = workspace.root_document(&candidate) {
                    roots.entry(root).or_insert(candidate);
                }
            }
            roots
        }
    };
    let range = doc
        .syntax
        .commands
        .iter()
        .find(|command| {
            command.name == "documentclass"
                || command.name == "begin"
                    && command
                        .argument
                        .as_ref()
                        .is_some_and(|argument| argument.value == "document")
        })
        .map(|command| doc.text.range(command.span.start, command.span.end))
        .unwrap_or_else(|| Range::new(Position::new(0, 0), Position::new(0, 0)));
    let separate_folders = roots.len() > 1 && workspace.roots.len() > 1;
    roots
        .into_iter()
        .filter(|(root, _)| {
            root.to_file_path().is_ok_and(|path| {
                path.is_file()
                    && matches!(
                        path.extension().and_then(|extension| extension.to_str()),
                        Some("tex" | "ltx")
                    )
            })
        })
        .take(32)
        .map(|(root, source)| {
            let label = root
                .to_file_path()
                .ok()
                .map(|path| {
                    if separate_folders {
                        return path.to_string_lossy().into_owned();
                    }
                    workspace
                        .roots
                        .iter()
                        .filter_map(|folder| {
                            path.strip_prefix(folder)
                                .ok()
                                .map(|relative| (folder.components().count(), relative))
                        })
                        .max_by_key(|(depth, _)| *depth)
                        .map(|(_, relative)| relative.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| root.to_string());
            CodeLens {
                range,
                command: Some(Command {
                    title: format!("Build {label}"),
                    command: "latex.build".into(),
                    // Keep the original context for relative rootFile settings.
                    // Ambiguous sources instead pass the selected parent.
                    arguments: Some(vec![serde_json::json!(source)]),
                }),
                data: None,
            }
        })
        .collect()
}
