use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use tower_lsp::lsp_types::Url;

use crate::{syntax::Syntax, text::Text};

const MAX_FILES: usize = 1024;
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Document {
    pub text: Text,
    pub syntax: Syntax,
    pub version: Option<i32>,
    pub bibtex: bool,
}

impl Document {
    pub fn new(uri: &Url, source: String, version: Option<i32>) -> Self {
        let text = Text::new(source);
        let bibtex = uri.path().ends_with(".bib");
        let syntax = if bibtex {
            Syntax::bibtex(&text)
        } else {
            Syntax::latex(&text)
        };
        Self {
            text,
            syntax,
            version,
            bibtex,
        }
    }
}

#[derive(Default)]
pub struct Workspace {
    pub documents: BTreeMap<Url, Document>,
    pub roots: BTreeSet<PathBuf>,
    pub snippets: bool,
    pub watch_files: bool,
    pub hierarchical_symbols: bool,
    pub versioned_edits: bool,
}

impl Workspace {
    pub fn add_root(&mut self, root: PathBuf) {
        if self.roots.insert(root.clone()) {
            self.scan(&root, 0);
        }
    }

    fn scan(&mut self, directory: &Path, depth: usize) {
        if depth > 16 || self.documents.len() >= MAX_FILES {
            return;
        }
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if self.documents.len() >= MAX_FILES {
                break;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !name.starts_with('.')
                    && !matches!(
                        name.as_ref(),
                        "target" | "build" | "node_modules" | "vendor"
                    )
                {
                    self.scan(&path, depth + 1);
                }
            } else if kind.is_file()
                && matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("tex" | "bib" | "sty" | "cls")
                )
            {
                if let Ok(uri) = Url::from_file_path(&path) {
                    self.reload(&uri);
                }
            }
        }
    }

    pub fn reload(&mut self, uri: &Url) {
        // Unsaved editor content always wins over filesystem notifications.
        if self
            .documents
            .get(uri)
            .is_some_and(|doc| doc.version.is_some())
        {
            return;
        }
        if self.documents.len() >= MAX_FILES && !self.documents.contains_key(uri) {
            return;
        }
        let source = uri.to_file_path().ok().and_then(|path| {
            let metadata = std::fs::metadata(&path).ok()?;
            if !metadata.is_file() || metadata.len() > MAX_FILE_SIZE {
                return None;
            }
            std::fs::read_to_string(path).ok()
        });
        if let Some(source) = source {
            self.documents
                .insert(uri.clone(), Document::new(uri, source, None));
        } else {
            self.documents.remove(uri);
        }
    }

    pub fn open(&mut self, uri: Url, source: String, version: i32) {
        if self.roots.is_empty() {
            if let Ok(path) = uri.to_file_path() {
                if let Some(parent) = path.parent() {
                    self.add_root(parent.to_path_buf());
                }
            }
        }
        self.documents
            .insert(uri.clone(), Document::new(&uri, source, Some(version)));
        self.load_dependencies(&uri, 0);
    }

    pub fn load_dependencies(&mut self, uri: &Url, depth: usize) {
        self.load_dependencies_inner(uri, depth, &mut BTreeSet::new());
    }

    fn load_dependencies_inner(&mut self, uri: &Url, depth: usize, visited: &mut BTreeSet<Url>) {
        if depth >= 16 || !visited.insert(uri.clone()) {
            return;
        }
        let Some(doc) = self.documents.get(uri) else {
            return;
        };
        let dependencies: Vec<_> = doc
            .syntax
            .commands
            .iter()
            .filter_map(|command| {
                let extension = match command.name.as_str() {
                    "input" | "include" | "subfile" => "tex",
                    "bibliography" | "addbibresource" => "bib",
                    _ => return None,
                };
                let argument = command.argument.as_ref()?;
                Some(
                    argument
                        .value
                        .split(',')
                        .filter_map(|value| resolve_path(uri, value.trim(), extension))
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect();
        for dependency in dependencies {
            if !self.documents.contains_key(&dependency) {
                self.reload(&dependency);
            }
            self.load_dependencies_inner(&dependency, depth + 1, visited);
        }
    }
}

pub fn resolve_path(uri: &Url, value: &str, extension: &str) -> Option<Url> {
    if value.is_empty() || value.contains(['\\', '{', '}', '#']) {
        return None;
    }
    let parent = uri.to_file_path().ok()?.parent()?.to_path_buf();
    let path = parent.join(value);
    let path = if path.is_file() {
        path
    } else if path.extension().is_none() {
        path.with_extension(extension)
    } else {
        path
    };
    let path = path.canonicalize().ok()?;
    path.is_file()
        .then(|| Url::from_file_path(path).ok())
        .flatten()
}
