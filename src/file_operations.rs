use crate::{
    features, graphics, navigation, syntax,
    workspace::{normalize_path, resolve_file_path, resolve_path, Document, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::*,
};

pub struct Renames(Vec<(PathBuf, PathBuf)>);

impl Renames {
    pub fn new(files: &[FileRename]) -> Result<Self> {
        let mut paths = BTreeMap::new();
        let mut destinations = BTreeSet::new();
        for file in files {
            let local = |uri: &str| {
                Url::parse(uri)
                    .ok()
                    .and_then(|uri| uri.to_file_path().ok())
                    .ok_or_else(|| Error::invalid_params("File renames require local file URIs"))
            };
            let old = local(&file.old_uri)?;
            let old = old.canonicalize().unwrap_or_else(|_| normalize_path(&old));
            let new = normalize_path(&local(&file.new_uri)?);
            if paths.insert(old, new.clone()).is_some() || !destinations.insert(new) {
                return Err(Error::invalid_params(
                    "Ambiguous file rename: duplicate source or destination",
                ));
            }
        }
        let mut paths: Vec<_> = paths.into_iter().collect();
        paths.sort_by_key(|(old, _)| std::cmp::Reverse(old.components().count()));
        Ok(Self(paths))
    }

    pub fn path(&self, path: &Path) -> PathBuf {
        for (old, new) in &self.0 {
            if let Ok(tail) = path.strip_prefix(old) {
                return if tail.as_os_str().is_empty() {
                    new.clone()
                } else {
                    new.join(tail)
                };
            }
        }
        path.into()
    }

    pub fn uri(&self, uri: &Url) -> Url {
        uri.to_file_path()
            .ok()
            .and_then(|path| Url::from_file_path(self.path(&path)).ok())
            .unwrap_or_else(|| uri.clone())
    }

    pub fn affects(&self, uri: &Url) -> bool {
        self.uri(uri) != *uri
    }
}

fn relative_path(parent: &Path, target: &Path) -> PathBuf {
    let from: Vec<_> = parent.components().collect();
    let to: Vec<_> = target.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return target.into();
    }
    let mut result = PathBuf::new();
    for _ in common..from.len() {
        result.push("..");
    }
    for component in &to[common..] {
        result.push(component.as_os_str());
    }
    result
}

fn replacement(
    renames: &Renames,
    uri: &Url,
    target: &Url,
    original: &str,
    graphics: &[PathBuf],
    root: &Url,
    command: Option<&str>,
) -> Result<String> {
    let target_path = target
        .to_file_path()
        .map_err(|_| Error::invalid_params("Invalid linked file URI"))?;
    let new_target = renames.path(&target_path);
    if original.starts_with("file:") {
        return Url::from_file_path(new_target)
            .map(|uri| uri.to_string())
            .map_err(|_| Error::invalid_params("Invalid renamed file URI"));
    }
    let extension = target_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let inferred = command
        .and_then(syntax::inferred_file_extensions)
        .unwrap_or(&["tex"]);
    let resolve = |origin| match command {
        Some(command) => resolve_file_path(origin, command, original, inferred),
        None => resolve_path(origin, original, extension),
    };
    let origin = if resolve(root).as_ref() == Some(target) {
        root.clone()
    } else if resolve(uri).as_ref() == Some(target) {
        uri.clone()
    } else {
        root.clone()
    };
    let graphic_origin = if resolve(&origin).as_ref() != Some(target) {
        graphics.iter().find(|directory| {
            graphics::resolve(directory, original, extension).as_ref() == Some(target)
        })
    } else {
        None
    };
    let origin = renames
        .uri(&origin)
        .to_file_path()
        .map_err(|_| Error::invalid_params("Invalid document URI"))?;
    let path = if Path::new(original).is_absolute() {
        new_target
    } else if let Some(directory) = graphic_origin
        .map(|directory| renames.path(directory))
        .filter(|directory| new_target.starts_with(directory))
    {
        relative_path(&directory, &new_target)
    } else {
        relative_path(
            origin
                .parent()
                .ok_or_else(|| Error::invalid_params("Invalid document path"))?,
            &new_target,
        )
    };
    let mut value = safe_path(&path)?;
    if !original.ends_with(&format!(".{extension}")) {
        if let Some(extension) = path
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|extension| inferred.contains(extension))
        {
            value.truncate(value.len() - extension.len() - 1);
        }
    }
    if original.starts_with("./") && !value.starts_with('.') {
        value.insert_str(0, "./");
    }
    Ok(value)
}

fn safe_path(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| Error::invalid_params("File name is not valid UTF-8"))?
        .to_owned();
    #[cfg(windows)]
    let value = value.replace('\\', "/");
    if value.contains([
        '\\', '%', '#', '&', '$', '{', '}', '^', '~', '"', '\r', '\n',
    ]) {
        return Err(Error::invalid_params("Renamed path contains characters that cannot be represented safely in a LaTeX file reference"));
    }
    Ok(value)
}

fn directory_replacement(
    renames: &Renames,
    directory: &graphics::Directory,
    original: &str,
) -> Result<String> {
    if renames.path(&directory.path) == directory.path && !renames.affects(&directory.origin) {
        return Ok(original.into());
    }
    let origin = renames
        .uri(&directory.origin)
        .to_file_path()
        .map_err(|_| Error::invalid_params("Invalid graphics search directory origin"))?;
    let target = renames.path(&directory.path);
    let path = if Path::new(original).is_absolute() {
        target
    } else {
        relative_path(
            origin
                .parent()
                .ok_or_else(|| Error::invalid_params("Invalid root path"))?,
            &target,
        )
    };
    let mut new_text = safe_path(&path)?;
    if new_text.is_empty() {
        new_text.push('.');
    } else if original.starts_with("./") && !new_text.starts_with('.') {
        new_text.insert_str(0, "./");
    }
    if !new_text.ends_with('/') {
        new_text.push('/');
    }
    Ok(new_text)
}

/// Edits target the old URIs: the client applies them before moving the files.
pub fn will_rename(workspace: &Workspace, files: &[FileRename]) -> Result<Option<WorkspaceEdit>> {
    if files.is_empty() {
        return Ok(None);
    }
    let renames = Renames::new(files)?;
    let mut changes = BTreeMap::new();
    let regions = workspace.saved_projects();
    for (uri, doc) in &workspace.documents {
        let roots = workspace.source_roots_in(uri, &regions);
        let mut directories = BTreeMap::new();
        for root in &roots {
            for directory in graphics::declarations_in(workspace, uri, root) {
                let original = &doc.text.source[directory.span.clone()];
                let new_text = directory_replacement(&renames, &directory, original)?;
                let span = (directory.span.start, directory.span.end);
                if directories
                    .get(&span)
                    .is_some_and(|value| *value != new_text)
                {
                    return Err(Error::invalid_params("Shared graphics search path would require different replacements in different projects"));
                }
                directories.insert(span, new_text);
            }
        }
        for ((start, end), new_text) in directories {
            if new_text != doc.text.source[start..end] {
                changes
                    .entry(uri.clone())
                    .or_insert_with(Vec::new)
                    .push(TextEdit {
                        range: doc.text.range(start, end),
                        new_text,
                    });
            }
        }
        let mut links = BTreeMap::<(usize, usize), (String, usize)>::new();
        for root in &roots {
            let graphics = graphics::directories_in(workspace, uri, root);
            for link in navigation::document_links_in(workspace, uri, root) {
                let Some(target) = link.target.filter(|target| target.scheme() == "file") else {
                    continue;
                };
                let (Some(start), Some(end)) = (
                    doc.text.offset(link.range.start),
                    doc.text.offset(link.range.end),
                ) else {
                    continue;
                };
                let original = &doc.text.source[start..end];
                let target_changed = target
                    .to_file_path()
                    .ok()
                    .is_some_and(|path| renames.path(&path) != path);
                let document_changed = renames.affects(uri);
                let root_changed = renames.affects(root);
                // Retain spelling in contexts that do not require a change too:
                // every consumer of a shared parameter must agree on its edit.
                let new_text = if target_changed || document_changed || root_changed {
                    let command = doc.syntax.commands.iter().find(|command| {
                        syntax::file_extensions(&command.name).is_some()
                            && command.argument.as_ref().is_some_and(|argument| {
                                argument.span.start <= start && argument.span.end >= end
                            })
                    });
                    let required_extension = command.and_then(|command| {
                        let extension = match command.name.as_str() {
                            "usepackage" | "RequirePackage" => "sty",
                            "documentclass" | "LoadClass" => "cls",
                            "include" | "includeonly" => "tex",
                            "bibliographystyle" => "bst",
                            _ => return None,
                        };
                        Some((command, extension))
                    });
                    if let Some((command, extension)) =
                        required_extension.filter(|_| target_changed)
                    {
                        let renamed = renames
                            .uri(&target)
                            .to_file_path()
                            .map_err(|_| Error::invalid_params("Invalid renamed file URI"))?;
                        if target
                            .to_file_path()
                            .ok()
                            .as_ref()
                            .and_then(|path| path.extension())
                            != renamed.extension()
                            && renamed.extension().and_then(|extension| extension.to_str())
                                != Some(extension)
                        {
                            return Err(Error::invalid_params(format!(
                                "File loaded by \\{} must retain the .{extension} extension",
                                command.name
                            )));
                        }
                    }
                    replacement(
                        &renames,
                        uri,
                        &target,
                        original,
                        if command.is_some_and(|command| command.name == "includegraphics") {
                            &graphics
                        } else {
                            &[]
                        },
                        if link.tooltip.as_deref() == Some("TeX root document") {
                            uri
                        } else {
                            root
                        },
                        command.map(|command| command.name.as_str()),
                    )?
                } else {
                    original.into()
                };
                if let Some((previous, count)) = links.get_mut(&(start, end)) {
                    if *previous != new_text {
                        return Err(Error::invalid_params("Shared file reference would require different replacements in different projects"));
                    }
                    *count += 1;
                } else {
                    links.insert((start, end), (new_text, 1));
                }
            }
        }
        for ((start, end), (mut new_text, count)) in links {
            if new_text != doc.text.source[start..end] {
                if count != roots.len() {
                    return Err(Error::invalid_params(
                        "Shared file reference cannot be updated consistently across build roots",
                    ));
                }
                if new_text.chars().any(char::is_whitespace)
                    && doc.syntax.commands.iter().any(|command| {
                        command.name == "input"
                            && command.argument.as_ref().is_some_and(|argument| {
                                argument.span == (start..end)
                                    && argument.outer_span == argument.span
                            })
                    })
                {
                    new_text = format!("{{{new_text}}}");
                }
                changes
                    .entry(uri.clone())
                    .or_insert_with(Vec::new)
                    .push(TextEdit {
                        range: doc.text.range(start, end),
                        new_text,
                    });
            }
        }
    }
    Ok((!changes.is_empty()).then(|| features::workspace_edit(workspace, changes)))
}

pub fn did_rename(workspace: &mut Workspace, files: &[FileRename]) -> Result<BTreeSet<Url>> {
    if files.is_empty() {
        return Ok(BTreeSet::new());
    }
    let renames = Renames::new(files)?;
    let mut affected = BTreeSet::new();
    for project in workspace.build_diagnostics.values() {
        affected.extend(project.keys().cloned());
    }
    workspace.build_diagnostics.clear();
    workspace.build_diagnostic_sources.clear();
    for inputs_by_root in [
        &mut workspace.recorded_inputs,
        &mut workspace.bibliography_inputs,
    ] {
        *inputs_by_root = std::mem::take(inputs_by_root)
            .into_iter()
            .map(|(root, inputs)| {
                (
                    renames.uri(&root),
                    inputs.iter().map(|uri| renames.uri(uri)).collect(),
                )
            })
            .collect();
    }
    workspace.roots = workspace
        .roots
        .iter()
        .map(|root| renames.path(root))
        .collect();
    let mut moved = Vec::new();
    for (uri, doc) in std::mem::take(&mut workspace.documents) {
        let new_uri = renames.uri(&uri);
        if new_uri != uri {
            affected.insert(uri);
            affected.insert(new_uri.clone());
            moved.push((
                new_uri.clone(),
                Document::new(&new_uri, doc.text.source, doc.version),
            ));
        } else {
            workspace.documents.insert(uri, doc);
        }
    }
    workspace.documents.extend(moved);
    // Closed documents may also contain edits returned by willRenameFiles.
    let documents: Vec<_> = workspace.documents.keys().cloned().collect();
    for uri in documents {
        workspace.reload(&uri);
    }
    Ok(affected)
}
