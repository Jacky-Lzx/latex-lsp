//! Existence checks influence output without necessarily reading source text.
use crate::{
    config::BuildConfig,
    syntax,
    workspace::{normalize_path, resolve_file_path_with, Workspace},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use tower_lsp::lsp_types::Url;

const MAX_PATHS: usize = 4096;
const MAX_ROOTS: usize = 128;

#[derive(Debug, Default)]
pub struct Snapshot {
    paths: BTreeMap<PathBuf, Option<PathBuf>>,
    watched: BTreeSet<Url>,
    artifact: Option<PathBuf>,
}

fn file(path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    path.is_file().then_some(path)
}

impl Snapshot {
    pub(crate) fn capture(workspace: &Workspace, root: &Url, config: &BuildConfig) -> Self {
        let Ok(root_path) = root.to_file_path() else {
            return Self::default();
        };
        let sources = workspace.project_documents(root);
        let mut outputs: BTreeSet<_> = [
            "pdf",
            "log",
            "fls",
            "aux",
            "bbl",
            "blg",
            "toc",
            "lof",
            "lot",
            "out",
            "nav",
            "snm",
            "vrb",
            "dvi",
            "xdv",
            "synctex",
            "synctex.gz",
            "fdb_latexmk",
            "run.xml",
            "bcf",
        ]
        .into_iter()
        .filter_map(|extension| config.artifact_path(&root_path, extension))
        .map(|path| normalize_path(&path))
        .collect();
        // \include creates auxiliary files under the configured output folder.
        if let Some(parent) = root_path.parent() {
            for source in &sources {
                if let Some(doc) = workspace.documents.get(source) {
                    for command in doc
                        .syntax
                        .commands
                        .iter()
                        .filter(|command| command.name == "include")
                    {
                        if let Some(argument) = &command.argument {
                            let name = argument.value.trim();
                            if syntax::is_literal_file_path(name) {
                                let base = name.strip_suffix(".tex").unwrap_or(name);
                                outputs.insert(normalize_path(
                                    &parent
                                        .join(&config.output_directory)
                                        .join(format!("{base}.aux")),
                                ));
                            }
                        }
                    }
                }
            }
        }
        let paths = RefCell::new(BTreeMap::new());
        'sources: for source in sources {
            let Some(doc) = workspace.documents.get(&source) else {
                continue;
            };
            for command in &doc.syntax.commands {
                if paths.borrow().len() >= MAX_PATHS {
                    break 'sources;
                }
                if !matches!(command.name.as_str(), "IfFileExists" | "InputIfFileExists") {
                    continue;
                }
                let Some(argument) = &command.argument else {
                    continue;
                };
                let name = argument.value.trim();
                if !syntax::is_literal_file_path(name) {
                    continue;
                }
                for origin in [root, &source] {
                    let selected =
                        resolve_file_path_with(origin, &command.name, name, &["tex"], |path| {
                            let before = file(path);
                            let mut paths = paths.borrow_mut();
                            if paths.len() < MAX_PATHS && !outputs.contains(&normalize_path(path)) {
                                paths.insert(path.to_path_buf(), before.clone());
                            }
                            before
                        });
                    if selected.is_some() || source == *root {
                        break;
                    }
                }
            }
        }
        let paths = paths.into_inner();
        let watched = paths
            .iter()
            .flat_map(|(path, before)| std::iter::once(path).chain(before.iter()))
            .filter_map(|path| Url::from_file_path(path).ok())
            .collect();
        Self {
            paths,
            watched,
            artifact: config
                .artifact_path(&root_path, "pdf")
                .map(|path| normalize_path(&path)),
        }
    }

    pub(crate) fn watched(&self) -> &BTreeSet<Url> {
        &self.watched
    }

    pub(crate) fn changed_path(&self) -> Option<&Path> {
        self.paths
            .iter()
            .find_map(|(path, before)| (file(path) != *before).then_some(path.as_path()))
    }

    pub(crate) fn matches_pdf(&self, pdf: &Path) -> bool {
        self.artifact.as_ref() == Some(&normalize_path(pdf))
    }

    pub(crate) fn remember(map: &mut BTreeMap<Url, Arc<Self>>, root: Url, snapshot: Arc<Self>) {
        if snapshot.paths.is_empty() {
            map.remove(&root);
            return;
        }
        map.insert(root.clone(), snapshot);
        while map.len() > MAX_ROOTS {
            let victim = map.keys().find(|key| **key != root).unwrap().clone();
            map.remove(&victim);
        }
    }
}
