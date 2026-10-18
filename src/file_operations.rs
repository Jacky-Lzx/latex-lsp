use crate::{
    features, navigation,
    workspace::{resolve_path, Document, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
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
            let old = old.canonicalize().unwrap_or_else(|_| normalize(&old));
            let new = normalize(&local(&file.new_uri)?);
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

    fn uri(&self, uri: &Url) -> Url {
        uri.to_file_path()
            .ok()
            .and_then(|path| Url::from_file_path(self.path(&path)).ok())
            .unwrap_or_else(|| uri.clone())
    }

    pub fn affects(&self, uri: &Url) -> bool {
        self.uri(uri) != *uri
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            _ => result.push(component.as_os_str()),
        }
    }
    result
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
    workspace: &Workspace,
    renames: &Renames,
    uri: &Url,
    target: &Url,
    original: &str,
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
    let origin = if resolve_path(uri, original, extension).as_ref() == Some(target) {
        uri.clone()
    } else {
        workspace.root_document(uri).unwrap_or_else(|_| uri.clone())
    };
    let origin = renames
        .uri(&origin)
        .to_file_path()
        .map_err(|_| Error::invalid_params("Invalid document URI"))?;
    let path = if Path::new(original).is_absolute() {
        new_target
    } else {
        relative_path(
            origin
                .parent()
                .ok_or_else(|| Error::invalid_params("Invalid document path"))?,
            &new_target,
        )
    };
    let mut value = path
        .to_str()
        .ok_or_else(|| Error::invalid_params("File name is not valid UTF-8"))?
        .to_owned();
    #[cfg(windows)]
    {
        value = value.replace('\\', "/");
    }
    if value.contains(['\\', '%', '#', '&', '$', '{', '}', '^', '~', '\r', '\n']) {
        return Err(Error::invalid_params("Renamed path contains characters that cannot be represented safely in a LaTeX file reference"));
    }
    if !original.ends_with(&format!(".{extension}")) {
        if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
            value.truncate(value.len() - extension.len() - 1);
        }
    }
    if original.starts_with("./") && !value.starts_with('.') {
        value.insert_str(0, "./");
    }
    Ok(value)
}

/// Edits target the old URIs: the client applies them before moving the files.
pub fn will_rename(workspace: &Workspace, files: &[FileRename]) -> Result<Option<WorkspaceEdit>> {
    if files.is_empty() {
        return Ok(None);
    }
    let renames = Renames::new(files)?;
    let mut changes = BTreeMap::new();
    for (uri, doc) in &workspace.documents {
        for link in navigation::document_links(workspace, uri) {
            let Some(target) = link.target.filter(|target| target.scheme() == "file") else {
                continue;
            };
            let Some(start) = doc.text.offset(link.range.start) else {
                continue;
            };
            let Some(end) = doc.text.offset(link.range.end) else {
                continue;
            };
            let original = &doc.text.source[start..end];
            // Unchanged links should retain their exact spelling and escaping.
            let target_changed = target
                .to_file_path()
                .ok()
                .is_some_and(|path| renames.path(&path) != path);
            let document_changed = renames.uri(uri) != *uri;
            let root_changed = workspace
                .root_document(uri)
                .ok()
                .is_some_and(|root| renames.uri(&root) != root);
            if !target_changed && !document_changed && !root_changed {
                continue;
            }
            let new_text = replacement(workspace, &renames, uri, &target, original)?;
            if new_text != original {
                changes
                    .entry(uri.clone())
                    .or_insert_with(Vec::new)
                    .push(TextEdit {
                        range: link.range,
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
