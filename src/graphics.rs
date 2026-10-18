//! Statically declared graphicx search directories, relative to the TeX root.
use crate::{
    syntax,
    workspace::{resolve_path, Workspace},
};
use std::{collections::BTreeSet, ops::Range, path::PathBuf};
use tower_lsp::lsp_types::Url;

pub fn is_image(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            syntax::file_extensions("includegraphics")
                .unwrap()
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

/// Keep binary dependencies out of the text index while sharing link resolution.
pub fn images(workspace: &Workspace, uri: &Url) -> BTreeSet<Url> {
    let Some(doc) = workspace.documents.get(uri) else {
        return BTreeSet::new();
    };
    let ranges: Vec<_> = doc
        .syntax
        .commands
        .iter()
        .filter(|command| command.name == "includegraphics")
        .filter_map(|command| command.argument.as_ref())
        .map(|argument| doc.text.range(argument.span.start, argument.span.end))
        .collect();
    if ranges.is_empty() {
        return BTreeSet::new();
    }
    crate::navigation::document_links(workspace, uri)
        .into_iter()
        .filter(|link| {
            ranges
                .iter()
                .any(|range| range.start <= link.range.start && range.end >= link.range.end)
        })
        .filter_map(|link| link.target)
        .collect()
}

pub struct Directory {
    pub path: PathBuf,
    pub span: Range<usize>,
    pub origin: Url,
}

pub fn declarations(workspace: &Workspace, uri: &Url) -> Vec<Directory> {
    let Some(doc) = workspace.documents.get(uri) else {
        return vec![];
    };
    let origin = workspace.root_document(uri).unwrap_or_else(|_| uri.clone());
    let Some(parent) = origin
        .to_file_path()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
    else {
        return vec![];
    };
    let mut directories = Vec::new();
    for command in doc
        .syntax
        .commands
        .iter()
        .filter(|command| command.name == "graphicspath")
    {
        let Some(argument) = &command.argument else {
            continue;
        };
        let mut i = 0;
        while i < argument.value.len() {
            i = syntax::skip_space(&argument.value, i);
            let Some((directory, end)) = syntax::group(&argument.value, i, b'{', b'}') else {
                break;
            };
            i = end;
            let span = syntax::trimmed_span(
                &doc.text.source,
                argument.span.start + directory.span.start
                    ..argument.span.start + directory.span.end,
            );
            let value = &doc.text.source[span.clone()];
            if !value.ends_with('/')
                || value.contains(['\\', '{', '}', '%', '#', '$', '&', '^', '~', '\n', '\r'])
            {
                continue;
            }
            let Ok(path) = parent.join(value).canonicalize() else {
                continue;
            };
            if path.is_dir() {
                directories.push(Directory {
                    path,
                    span,
                    origin: origin.clone(),
                });
                if directories.len() >= 256 {
                    return directories;
                }
            }
        }
    }
    directories
}

pub fn directories(workspace: &Workspace, uri: &Url) -> Vec<PathBuf> {
    let root = workspace.root_document(uri).unwrap_or_else(|_| uri.clone());
    let project = workspace.project_documents(&root);
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for document in std::iter::once(uri)
        .chain(std::iter::once(&root))
        .chain(project.iter())
    {
        if !seen.insert(document.clone()) {
            continue;
        }
        for directory in declarations(workspace, document) {
            if !result.contains(&directory.path) {
                result.push(directory.path);
            }
            if result.len() >= 256 {
                return result;
            }
        }
    }
    result
}

pub fn resolve(directory: &std::path::Path, name: &str, extension: &str) -> Option<Url> {
    let origin = Url::from_file_path(directory.join("__latex_lsp_graphics__.tex")).ok()?;
    resolve_path(&origin, name, extension)
}
