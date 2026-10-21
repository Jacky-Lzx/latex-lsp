//! Statically declared graphicx search directories, relative to the TeX root.
use crate::{
    syntax,
    workspace::{normalize_path, resolve_path, Workspace},
};
use std::{
    collections::BTreeSet,
    ops::Range,
    path::{Path, PathBuf},
};
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
    collect_images(
        workspace,
        uri,
        crate::navigation::document_links(workspace, uri),
    )
}

pub(crate) fn images_in(workspace: &Workspace, uri: &Url, root: &Url) -> BTreeSet<Url> {
    collect_images(
        workspace,
        uri,
        crate::navigation::document_links_in(workspace, uri, root),
    )
}

fn collect_images(
    workspace: &Workspace,
    uri: &Url,
    links: Vec<tower_lsp::lsp_types::DocumentLink>,
) -> BTreeSet<Url> {
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
    links
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
    if !workspace.documents.get(uri).is_some_and(|doc| {
        doc.syntax
            .commands
            .iter()
            .any(|command| command.name == "graphicspath")
    }) {
        return vec![];
    }
    let origins = workspace.source_roots(uri);
    origins
        .iter()
        .flat_map(|origin| declarations_in(workspace, uri, origin))
        .collect()
}

pub(crate) fn declarations_in(workspace: &Workspace, uri: &Url, origin: &Url) -> Vec<Directory> {
    declarations_with(workspace, uri, origin, false).0
}

// Navigation uses canonical destinations; build conditions must retain the
// lexical directory alias and include directories which do not exist yet.
fn declarations_with(
    workspace: &Workspace,
    uri: &Url,
    origin: &Url,
    for_conditions: bool,
) -> (Vec<Directory>, bool) {
    let Some(doc) = workspace.documents.get(uri) else {
        return (vec![], true);
    };
    let Some(parent) = origin
        .to_file_path()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
    else {
        return (vec![], false);
    };
    let mut directories = Vec::new();
    let mut complete = true;
    for command in doc
        .syntax
        .commands
        .iter()
        .filter(|command| command.name == "graphicspath")
    {
        let Some(argument) = &command.argument else {
            complete = false;
            continue;
        };
        if for_conditions
            && argument.outer_span.start != syntax::skip_space(&doc.text.source, command.span.end)
        {
            complete = false;
            continue;
        }
        let mut i = 0;
        while i < argument.value.len() {
            i = syntax::skip_space(&argument.value, i);
            if i == argument.value.len() {
                break;
            }
            let Some((directory, end)) = syntax::group(&argument.value, i, b'{', b'}') else {
                complete = false;
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
                complete = false;
                continue;
            }
            let path = parent.join(value);
            let path = if !for_conditions {
                path.canonicalize()
                    .unwrap_or_else(|_| normalize_path(&path))
            } else {
                path
            };
            directories.push(Directory {
                path,
                span,
                origin: origin.clone(),
            });
            if directories.len() >= 256 {
                return (directories, false);
            }
        }
    }
    (directories, complete)
}

/// Potential static search directories, including missing paths and lexical
/// aliases. Dynamic, incomplete or repeated declarations cannot define a safe
/// search without inferring macro execution order and scope.
pub(crate) fn condition_directories_in(
    workspace: &Workspace,
    uri: &Url,
    root: &Url,
) -> Option<Vec<PathBuf>> {
    let project = workspace.project_documents(root);
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    let mut declarations = 0;
    for document in std::iter::once(uri)
        .chain(std::iter::once(root))
        .chain(project.iter())
    {
        if !seen.insert(document.clone()) {
            continue;
        }
        declarations += workspace.documents.get(document).map_or(0, |doc| {
            doc.syntax
                .commands
                .iter()
                .filter(|command| command.name == "graphicspath")
                .count()
        });
        if declarations > 1 {
            return None;
        }
        let (directories, complete) = declarations_with(workspace, document, root, true);
        if !complete {
            return None;
        }
        for directory in directories {
            if !result.contains(&directory.path) {
                result.push(directory.path);
            }
            if result.len() >= 256 {
                return None;
            }
        }
    }
    Some(result)
}

pub fn directories(workspace: &Workspace, uri: &Url) -> Vec<PathBuf> {
    if let Ok(root) = workspace.root_document(uri) {
        return directories_in(workspace, uri, &root);
    }
    let mut result = Vec::new();
    for root in workspace.source_roots(uri) {
        for directory in directories_in(workspace, uri, &root) {
            if !result.contains(&directory) {
                result.push(directory);
            }
            if result.len() >= 256 {
                return result;
            }
        }
    }
    result
}

pub(crate) fn directories_in(workspace: &Workspace, uri: &Url, root: &Url) -> Vec<PathBuf> {
    let project = workspace.project_documents(root);
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for document in std::iter::once(uri)
        .chain(std::iter::once(root))
        .chain(project.iter())
    {
        if !seen.insert(document.clone()) {
            continue;
        }
        for directory in declarations_in(workspace, document, root) {
            if directory.path.is_dir() && !result.contains(&directory.path) {
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

/// graphicx searches one extension across the root and its declared directories
/// before trying the next extension. Included sources do not change TeX's cwd.
pub(crate) fn resolve_in(
    root: &Url,
    name: &str,
    extensions: &[&str],
    directories: &[PathBuf],
) -> Option<Url> {
    resolve_with(root, name, extensions, directories, |path| {
        let path = path.canonicalize().ok()?;
        path.is_file().then_some(path)
    })
}

/// The same search can inspect the filesystem after a proposed rename batch
/// without moving files or constructing temporary copies of binary assets.
pub(crate) fn resolve_with(
    root: &Url,
    name: &str,
    extensions: &[&str],
    directories: &[PathBuf],
    file: impl Fn(&Path) -> Option<PathBuf>,
) -> Option<Url> {
    let mut literal = name.to_owned();
    for character in ['_', '%', '#', '&', '$', ' '] {
        literal = literal.replace(&format!("\\{character}"), &character.to_string());
    }
    let name = literal.as_str();
    if name.is_empty() || name.contains(['\\', '{', '}']) {
        return None;
    }
    let parent = root.to_file_path().ok()?.parent()?.to_owned();
    let exact = |name: &str| {
        file(&parent.join(name)).or_else(|| {
            directories
                .iter()
                .find_map(|directory| file(&directory.join(name)))
        })
    };
    let explicit = std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str());
    let direct = explicit.and_then(|_| exact(name));
    let selected = if direct.is_some()
        && explicit.is_some_and(|extension| {
            syntax::file_extensions("includegraphics")
                .unwrap()
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        }) {
        direct
    } else {
        // graphicx also tries appending its extensions to a dotted filename
        // when its explicit name is missing or has no known graphics rule.
        extensions
            .iter()
            .find_map(|extension| exact(&format!("{name}.{extension}")))
            .or(direct)
    };
    selected.and_then(|path| Url::from_file_path(path).ok())
}
