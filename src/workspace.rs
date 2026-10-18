use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use tower_lsp::lsp_types::Url;

use crate::{syntax::Syntax, text::Text};

const MAX_FILES: usize = 1024;
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
pub const SOURCE_EXTENSIONS: &[&str] = &["tex", "ltx", "bib", "sty", "cls", "def", "clo"];

pub fn is_source(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|extension| SOURCE_EXTENSIONS.contains(&extension))
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
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

/// The first real magic comment in the first 20 lines, excluding raw content.
pub fn root_directive(doc: &Document) -> Option<crate::syntax::Argument> {
    magic_directive(doc, &["root"])
}

pub(crate) fn is_root(doc: &Document) -> bool {
    doc.syntax.commands.iter().any(|command| {
        command.name == "documentclass"
            || command.name == "begin"
                && command
                    .argument
                    .as_ref()
                    .is_some_and(|argument| argument.value == "document")
    })
}

fn magic_directive(doc: &Document, fields: &[&str]) -> Option<crate::syntax::Argument> {
    let mut offset = 0;
    for line in doc.text.source.split_inclusive('\n').take(20) {
        let trimmed = line.trim_start();
        let comment_start = offset + line.len() - trimmed.len();
        if (doc.bibtex
            || doc
                .syntax
                .ignored
                .iter()
                .any(|span| span.start == comment_start))
            && trimmed.starts_with('%')
        {
            if let Some((name, value)) = trimmed[1..].trim().split_once('=') {
                if fields
                    .iter()
                    .any(|field| name.trim().eq_ignore_ascii_case(&format!("!tex {field}")))
                {
                    let value = value.trim().trim_matches('"');
                    let start = offset + line.find('=').unwrap() + 1;
                    let start = start + line[start - offset..].find(value).unwrap();
                    return Some(crate::syntax::Argument {
                        value: value.into(),
                        span: start..start + value.len(),
                    });
                }
            }
        }
        offset += line.len();
    }
    None
}

#[derive(Default)]
pub struct Workspace {
    pub documents: BTreeMap<Url, Document>,
    pub roots: BTreeSet<PathBuf>,
    pub snippets: bool,
    pub watch_files: bool,
    pub hierarchical_symbols: bool,
    pub versioned_edits: bool,
    pub folding_limit: Option<u32>,
    pub build_progress: bool,
    pub code_action_literals: bool,
    pub apply_edits: bool,
    pub config: crate::config::Config,
    pub build_diagnostics: BTreeMap<Url, BTreeMap<Url, Vec<tower_lsp::lsp_types::Diagnostic>>>,
    pub recorded_inputs: BTreeMap<Url, BTreeSet<Url>>,
}

pub(crate) struct SymbolProjects {
    pub scope: BTreeSet<Url>,
    pub regions: BTreeMap<Url, BTreeSet<Url>>,
}

impl Workspace {
    pub fn build_config(&self, root: &Url) -> crate::config::BuildConfig {
        let mut config = self.config.build.clone();
        let latexmk = Path::new(&config.executable)
            .file_stem()
            .and_then(|name| name.to_str())
            == Some("latexmk");
        if latexmk && config.engine.is_none() && config.arguments.is_none() {
            config.engine = self
                .documents
                .get(root)
                .and_then(|doc| magic_directive(doc, &["program", "ts-program"]))
                .and_then(|argument| crate::config::Engine::from_program(&argument.value));
        }
        config
    }

    pub fn diagnostics(&self, uri: &Url) -> Vec<tower_lsp::lsp_types::Diagnostic> {
        self.diagnostic_snapshots([uri.clone()])
            .pop()
            .map(|(_, diagnostics, _)| diagnostics)
            .unwrap_or_default()
    }

    pub fn diagnostic_snapshots(
        &self,
        uris: impl IntoIterator<Item = Url>,
    ) -> Vec<(Url, Vec<tower_lsp::lsp_types::Diagnostic>, Option<i32>)> {
        let uris: BTreeSet<_> = uris.into_iter().collect();
        let mut duplicates = crate::project_diagnostics::duplicates(self, &uris);
        uris.into_iter()
            .map(|uri| {
                let version = self.documents.get(&uri).and_then(|doc| doc.version);
                let mut diagnostics = self.local_and_build_diagnostics(&uri);
                diagnostics.extend(duplicates.remove(&uri).unwrap_or_default());
                diagnostics
                    .sort_by_key(|diagnostic| (diagnostic.range.start, diagnostic.message.clone()));
                (uri, diagnostics, version)
            })
            .collect()
    }

    fn local_and_build_diagnostics(&self, uri: &Url) -> Vec<tower_lsp::lsp_types::Diagnostic> {
        let mut diagnostics = self
            .documents
            .get(uri)
            .map(crate::features::diagnostics)
            .unwrap_or_default();
        for project in self.build_diagnostics.values() {
            if let Some(build) = project.get(uri) {
                diagnostics.extend(build.iter().cloned());
            }
        }
        diagnostics.sort_by_key(|diagnostic| (diagnostic.range.start, diagnostic.message.clone()));
        diagnostics
            .dedup_by(|a, b| a.range == b.range && a.message == b.message && a.source == b.source);
        diagnostics
    }

    pub fn clear_build_diagnostics(&mut self, uri: &Url) {
        for project in self.build_diagnostics.values_mut() {
            project.remove(uri);
        }
    }

    pub fn dependencies(&self, uri: &Url) -> Vec<Url> {
        self.dependencies_from(uri, uri)
    }

    fn dependencies_from(&self, uri: &Url, root: &Url) -> Vec<Url> {
        let Some(doc) = self.documents.get(uri) else {
            return vec![];
        };
        let mut dependencies: Vec<Url> = doc
            .syntax
            .commands
            .iter()
            .flat_map(|command| {
                let extension = match command.name.as_str() {
                    "input" | "include" | "subfile" => "tex",
                    "bibliography" | "addbibresource" => "bib",
                    "usepackage" | "RequirePackage" => "sty",
                    "documentclass" | "LoadClass" => "cls",
                    _ => return vec![],
                };
                command
                    .argument
                    .as_ref()
                    .map(|arg| {
                        let multiple = crate::syntax::file_list(&command.name);
                        let values = if multiple {
                            arg.value.split(',').collect::<Vec<_>>()
                        } else {
                            vec![arg.value.as_str()]
                        };
                        values
                            .into_iter()
                            .filter_map(|value| {
                                resolve_path(root, value.trim(), extension)
                                    .or_else(|| resolve_path(uri, value.trim(), extension))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();
        if uri == root {
            if let Some(recorded) = self.recorded_inputs.get(root) {
                dependencies.extend(recorded.iter().cloned());
            }
        }
        dependencies
    }

    /// Do not index TeX's distribution packages outside the project. Explicitly
    /// opened or already indexed external documents remain part of the project.
    pub fn project_inputs(&self, root: &Url, inputs: &BTreeSet<Url>) -> BTreeSet<Url> {
        let parent = root
            .to_file_path()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        inputs
            .iter()
            .filter(|uri| {
                self.documents.contains_key(*uri)
                    || uri.to_file_path().ok().is_some_and(|path| {
                        parent
                            .as_ref()
                            .is_some_and(|parent| path.starts_with(parent))
                            || self.roots.iter().any(|root| path.starts_with(root))
                    })
            })
            .take(MAX_FILES)
            .cloned()
            .collect()
    }

    pub fn project_documents(&self, root: &Url) -> BTreeSet<Url> {
        let mut visited = BTreeSet::new();
        let mut pending = vec![root.clone()];
        while let Some(uri) = pending.pop() {
            if visited.insert(uri.clone()) {
                pending.extend(self.dependencies_from(&uri, root));
            }
        }
        visited
    }

    pub(crate) fn saved_projects(&self) -> BTreeMap<Url, BTreeSet<Url>> {
        self.documents
            .iter()
            .filter(|(uri, doc)| {
                is_root(doc)
                    && uri.to_file_path().is_ok_and(|path| {
                        path.is_file()
                            && matches!(
                                path.extension().and_then(|extension| extension.to_str()),
                                Some("tex" | "ltx")
                            )
                    })
            })
            .map(|(root, _)| (root.clone(), self.project_documents(root)))
            .collect()
    }

    pub(crate) fn symbol_projects(&self, uri: &Url) -> SymbolProjects {
        let regions = self.saved_projects();
        let selected = self.root_document(uri).ok().filter(|root| {
            regions.contains_key(root) || *root != *uri || self.config.build.root_file.is_some()
        });
        let mut scope: BTreeSet<_> = if let Some(root) = selected {
            self.project_documents(&root)
        } else {
            regions
                .values()
                .filter(|project| project.contains(uri))
                .flat_map(|project| project.iter().cloned())
                .collect()
        };
        if scope.is_empty() {
            // Unassociated buffers retain workspace-wide editing before saving.
            scope.extend(self.documents.keys().cloned());
        }
        scope.insert(uri.clone());
        SymbolProjects { scope, regions }
    }

    pub(crate) fn symbol_scope(&self, uri: &Url) -> BTreeSet<Url> {
        if let Ok(root) = self.root_document(uri) {
            if root != *uri
                || self.config.build.root_file.is_some()
                || self.documents.get(&root).is_some_and(is_root)
            {
                let mut scope = self.project_documents(&root);
                scope.insert(uri.clone());
                return scope;
            }
        }
        self.symbol_projects(uri).scope
    }

    pub(crate) fn source_roots(&self, uri: &Url) -> Vec<Url> {
        self.source_roots_in(uri, &self.saved_projects())
    }

    pub(crate) fn source_roots_in(
        &self,
        uri: &Url,
        regions: &BTreeMap<Url, BTreeSet<Url>>,
    ) -> Vec<Url> {
        let roots: Vec<_> = regions
            .iter()
            .filter(|(_, project)| project.contains(uri))
            .map(|(root, _)| root.clone())
            .collect();
        if roots.is_empty() {
            vec![self.root_document(uri).unwrap_or_else(|_| uri.clone())]
        } else {
            roots
        }
    }

    /// Saved sources and image dependencies used by builds and artifact checks.
    pub fn project_files(&self, root: &Url) -> BTreeSet<Url> {
        let mut files = self.project_documents(root);
        let images: BTreeSet<_> = files
            .iter()
            .flat_map(|uri| crate::graphics::images_in(self, uri, root))
            .collect();
        files.extend(images);
        files
    }

    /// Explicit root, then TeX magic comments, then the unique containing document.
    pub fn root_document(&self, uri: &Url) -> Result<Url, String> {
        let path = uri
            .to_file_path()
            .map_err(|_| "Compilation requires a local file URI")?;
        if let Some(root_file) = &self.config.build.root_file {
            let folder = self
                .roots
                .iter()
                .filter(|root| path.starts_with(root))
                .max_by_key(|root| root.components().count())
                .map(PathBuf::as_path)
                .or_else(|| path.parent())
                .ok_or("Cannot determine workspace directory")?;
            return file_uri(&folder.join(root_file));
        }
        let mut root = uri.clone();
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(root.clone()) {
                return Err("Cyclic TeX root directives".into());
            }
            let disk = if self.documents.contains_key(&root) {
                None
            } else {
                root.to_file_path()
                    .ok()
                    .and_then(|path| std::fs::read_to_string(path).ok())
                    .map(|text| Document::new(&root, text, None))
            };
            let doc = self
                .documents
                .get(&root)
                .or(disk.as_ref())
                .ok_or("Save the document before compiling")?;
            let magic = root_directive(doc);
            if let Some(magic) = magic {
                let parent = root
                    .to_file_path()
                    .map_err(|_| "Invalid root URI")?
                    .parent()
                    .ok_or("Invalid root path")?
                    .to_path_buf();
                root = file_uri(&parent.join(magic.value))?;
            } else {
                if root != *uri {
                    return Ok(root);
                }
                break;
            }
        }
        if self.documents.get(uri).is_some_and(is_root) {
            return file_uri(&path);
        }
        let candidates = self
            .documents
            .iter()
            .filter(|(candidate, doc)| {
                *candidate != uri && is_root(doc) && self.project_documents(candidate).contains(uri)
            })
            .map(|(uri, _)| uri.clone())
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [root] => Ok(root.clone()),
            [] => file_uri(&path),
            _ => Err("Multiple root documents include this file; set build.rootFile or a % !TeX root directive".into()),
        }
    }

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
            } else if kind.is_file() && is_source(&path) {
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
            if crate::graphics::is_image(&path) {
                return None;
            }
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
        self.load_dependencies_inner(uri, uri, depth, &mut BTreeSet::new(), false);
    }

    /// Reload each saved dependency before walking its includes: closed child
    /// files may have gained new dependencies since the previous build.
    pub fn refresh_project(&mut self, root: &Url) {
        self.load_dependencies_inner(root, root, 0, &mut BTreeSet::new(), true);
    }

    fn load_dependencies_inner(
        &mut self,
        uri: &Url,
        root: &Url,
        depth: usize,
        visited: &mut BTreeSet<Url>,
        refresh: bool,
    ) {
        if depth >= 16 || !visited.insert(uri.clone()) {
            return;
        }
        if refresh {
            self.reload(uri);
        }
        let dependencies = self.dependencies_from(uri, root);
        for dependency in dependencies {
            if !self.documents.contains_key(&dependency) {
                self.reload(&dependency);
            }
            self.load_dependencies_inner(&dependency, root, depth + 1, visited, refresh);
        }
    }
}

fn file_uri(path: &Path) -> Result<Url, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("Cannot read root document {}: {error}", path.display()))?;
    if !path.is_file()
        || !matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("tex" | "ltx")
        )
    {
        return Err("Root document must be a saved .tex or .ltx file".into());
    }
    Url::from_file_path(path).map_err(|_| "Cannot convert root document path to URI".into())
}

pub fn resolve_path(uri: &Url, value: &str, extension: &str) -> Option<Url> {
    let mut value = value.to_owned();
    for character in ['_', '%', '#', '&', '$', ' '] {
        value = value.replace(&format!("\\{character}"), &character.to_string());
    }
    if value.is_empty() || value.contains(['\\', '{', '}']) {
        return None;
    }
    let parent = uri.to_file_path().ok()?.parent()?.to_path_buf();
    let path = parent.join(&value);
    let path = if path.is_file() {
        path
    } else if path.extension().and_then(|value| value.to_str()) != Some(extension) {
        let mut name = path.as_os_str().to_os_string();
        name.push(".");
        name.push(extension);
        PathBuf::from(name)
    } else {
        path
    };
    let path = path.canonicalize().ok()?;
    path.is_file()
        .then(|| Url::from_file_path(path).ok())
        .flatten()
}
