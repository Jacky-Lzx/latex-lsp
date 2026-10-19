//! Captured file conditions and content fingerprints for build validity.
use crate::{
    config::BuildConfig,
    syntax,
    workspace::{normalize_path, resolve_file_path_with, Workspace},
};
use std::{
    cell::RefCell,
    collections::{hash_map::DefaultHasher, BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use tower_lsp::lsp_types::Url;

const MAX_PATHS: usize = 4096;
const MAX_ROOTS: usize = 128;

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    paths: BTreeMap<PathBuf, Option<PathBuf>>,
    watched: BTreeSet<Url>,
    artifact: Option<PathBuf>,
    inputs: BTreeMap<Url, InputSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InputSnapshot {
    Text { length: usize, hash: u64 },
    File(Option<(u64, u64)>),
}

impl InputSnapshot {
    pub(crate) fn text(text: &str) -> Self {
        let mut hash = DefaultHasher::new();
        text.hash(&mut hash);
        Self::Text {
            length: text.len(),
            hash: hash.finish(),
        }
    }

    pub(crate) fn file(path: &Path) -> Self {
        let read = || {
            if !std::fs::metadata(path).ok()?.is_file() {
                return None;
            }
            let mut file = std::fs::File::open(path).ok()?;
            let mut hash = DefaultHasher::new();
            let mut length = 0;
            let mut buffer = [0; 64 * 1024];
            loop {
                let count = file.read(&mut buffer).ok()?;
                if count == 0 {
                    break;
                }
                length += count as u64;
                hash.write(&buffer[..count]);
            }
            Some((length, hash.finish()))
        };
        Self::File(read())
    }

    pub(crate) fn changed(&self, uri: &Url) -> bool {
        let Ok(path) = uri.to_file_path() else {
            return true;
        };
        match self {
            Self::Text { length, .. } => {
                std::fs::metadata(&path).map_or(true, |metadata| metadata.len() != *length as u64)
                    || std::fs::read_to_string(path)
                        .ok()
                        .is_none_or(|text| Self::text(&text) != *self)
            }
            Self::File(before) => {
                let length = std::fs::metadata(&path)
                    .ok()
                    .filter(|metadata| metadata.is_file())
                    .map(|metadata| metadata.len());
                length != before.as_ref().map(|(length, _)| *length) || Self::file(&path) != *self
            }
        }
    }
}

pub(crate) fn fingerprint(path: &Path) -> Option<(u64, Option<std::time::SystemTime>)> {
    let metadata = std::fs::metadata(path).ok()?;
    metadata
        .is_file()
        .then_some((metadata.len(), metadata.modified().ok()))
}

fn file(path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    path.is_file().then_some(path)
}

// A missing file cannot be canonicalized, but an existing parent may be a
// symlink. Editors can report its eventual creation through the physical path.
fn canonical_candidate(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|parent| {
        parent
            .canonicalize()
            .ok()
            .map(|canonical| canonical.join(path.strip_prefix(parent).unwrap()))
    })
}

impl Snapshot {
    pub(crate) fn capture(workspace: &Workspace, root: &Url, config: &BuildConfig) -> Self {
        let Ok(root_path) = root.to_file_path() else {
            return Self::default();
        };
        let sources = workspace.project_documents(root);
        // A missing chapter is meaningful only when it can be selected.
        // Unknown or conflicting filters must not guess that TeX reads it.
        let include_filters: Option<Vec<BTreeSet<_>>> = sources
            .iter()
            .filter_map(|source| workspace.documents.get(source))
            .flat_map(|doc| &doc.syntax.commands)
            .filter(|command| command.name == "includeonly")
            .map(|command| {
                command
                    .argument
                    .as_ref()?
                    .value
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(|name| {
                        syntax::is_literal_file_path(name)
                            .then(|| name.strip_suffix(".tex").unwrap_or(name).to_owned())
                    })
                    .collect()
            })
            .collect();
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
                if !matches!(
                    command.name.as_str(),
                    "IfFileExists" | "InputIfFileExists" | "include" | "input" | "subfile"
                ) {
                    continue;
                }
                let Some(argument) = &command.argument else {
                    continue;
                };
                let name = argument.value.trim();
                if !syntax::is_literal_file_path(name) {
                    continue;
                }
                if command.name == "include" {
                    let base = name.strip_suffix(".tex").unwrap_or(name);
                    let selected = include_filters
                        .as_ref()
                        .is_some_and(|filters| filters.iter().all(|names| names.contains(base)));
                    if !selected
                        || [root, &source].into_iter().any(|origin| {
                            resolve_file_path_with(origin, "include", name, &["tex"], file)
                                .is_some()
                        })
                    {
                        continue;
                    }
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
            .flat_map(|(path, before)| {
                [
                    Some(path.clone()),
                    before.clone().or_else(|| canonical_candidate(path)),
                ]
                .into_iter()
                .flatten()
            })
            .filter_map(|path| Url::from_file_path(path).ok())
            .collect();
        Self {
            paths,
            watched,
            inputs: BTreeMap::new(),
            artifact: config
                .artifact_path(&root_path, "pdf")
                .map(|path| normalize_path(&path)),
        }
    }

    pub(crate) fn with_inputs(&self, inputs: BTreeMap<Url, InputSnapshot>) -> Self {
        Self {
            inputs,
            ..self.clone()
        }
    }

    pub(crate) fn input_changed(&self, uri: &Url) -> bool {
        // Unknown inputs invalidate conservatively, including newly resolved
        // dependencies and histories evicted by the root limit.
        self.inputs
            .get(uri)
            .is_none_or(|before| before.changed(uri))
    }

    pub(crate) fn changed_input(&self) -> Option<&Url> {
        self.inputs
            .iter()
            .find_map(|(uri, before)| before.changed(uri).then_some(uri))
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
        if snapshot.paths.is_empty() && snapshot.inputs.is_empty() {
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
