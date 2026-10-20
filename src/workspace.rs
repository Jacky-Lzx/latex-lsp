use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use tower_lsp::lsp_types::Url;

use crate::{syntax::Syntax, text::Text};

const MAX_FILES: usize = 1024;
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
pub const SOURCE_EXTENSIONS: &[&str] = &["tex", "ltx", "bib", "bst", "sty", "cls", "def", "clo"];

pub fn is_source(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|extension| SOURCE_EXTENSIONS.contains(&extension))
}

/// Editors and filesystem URLs can spell the same percent-encoded path
/// differently. Keep one document identity without resolving symlinks or
/// changing the case of filesystem names.
pub(crate) fn document_uri(uri: Url) -> Url {
    let Ok(path) = uri.to_file_path() else {
        return uri;
    };
    let Ok(mut normalized) = Url::from_file_path(path) else {
        return uri;
    };
    normalized.set_query(uri.query());
    normalized.set_fragment(uri.fragment());
    normalized
}

#[cfg(test)]
mod uri_tests {
    use super::*;

    #[test]
    fn file_uri_identity_preserves_path_case_queries_fragments_and_non_file_schemes() {
        let uri =
            Url::parse("file:///tmp/%e7%ab%a0%e8%8a%82%20Draft.tex?mode=%2f#part%2f").unwrap();
        let normalized = document_uri(uri);
        assert_eq!(
            normalized.as_str(),
            "file:///tmp/%E7%AB%A0%E8%8A%82%20Draft.tex?mode=%2f#part%2f"
        );
        assert_eq!(document_uri(normalized.clone()), normalized);
        for value in [
            "untitled:Draft.tex",
            "https://example.test/%e7%ab%a0.tex",
            "file://remote-host/Chapter.tex",
        ] {
            let uri = Url::parse(value).unwrap();
            if uri.to_file_path().is_err() {
                assert_eq!(document_uri(uri.clone()), uri);
            }
        }
        assert_ne!(
            document_uri(Url::parse("file:///tmp/Draft.tex").unwrap()),
            document_uri(Url::parse("file:///tmp/draft.tex").unwrap())
        );
    }
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
    pub kind: DocumentKind,
    /// Input role inferred before resolving aliases; otherwise use the URI.
    pub kind_hint: Option<DocumentKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentKind {
    Latex,
    Bibtex,
    BibtexStyle,
}

impl Document {
    pub fn new(uri: &Url, source: String, version: Option<i32>) -> Self {
        Self::new_with_kind(uri, source, version, None)
    }

    pub(crate) fn new_with_kind(
        uri: &Url,
        source: String,
        version: Option<i32>,
        kind_hint: Option<DocumentKind>,
    ) -> Self {
        let text = Text::new(source);
        let kind = kind_hint.unwrap_or_else(|| {
            if uri.path().ends_with(".bib") {
                DocumentKind::Bibtex
            } else if uri.path().ends_with(".bst") {
                DocumentKind::BibtexStyle
            } else {
                DocumentKind::Latex
            }
        });
        let syntax = Self::parse(&text, kind);
        Self {
            text,
            syntax,
            version,
            kind,
            kind_hint,
        }
    }

    fn parse(text: &Text, kind: DocumentKind) -> Syntax {
        match kind {
            DocumentKind::Bibtex => Syntax::bibtex(text),
            DocumentKind::Latex => Syntax::latex(text),
            // BibTeX styles use a stack language, not LaTeX commands/braces.
            DocumentKind::BibtexStyle => Syntax::default(),
        }
    }

    fn set_kind_hint(&mut self, kind: DocumentKind) {
        if self.kind != kind {
            self.kind = kind;
            self.syntax = Self::parse(&self.text, kind);
        }
        self.kind_hint = Some(kind);
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

fn is_saved_root(uri: &Url, doc: &Document) -> bool {
    is_root(doc)
        && uri.to_file_path().is_ok_and(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("tex" | "ltx")
            ) && file_uri(&path).is_ok()
        })
}

fn magic_directive(doc: &Document, fields: &[&str]) -> Option<crate::syntax::Argument> {
    let mut offset = 0;
    for line in doc.text.source.split_inclusive('\n').take(20) {
        let trimmed = line.trim_start();
        let comment_start = offset + line.len() - trimmed.len();
        if (doc.kind != DocumentKind::Latex
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
                        outer_span: start..start + value.len(),
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
    pub diagnostic_delivery: crate::diagnostic::Delivery,
    pub config: crate::config::Config,
    pub build_diagnostics: BTreeMap<Url, BTreeMap<Url, Vec<tower_lsp::lsp_types::Diagnostic>>>,
    /// Sources confirmed read for each root's latest published build diagnostics.
    pub build_diagnostic_sources: BTreeMap<Url, BTreeSet<Url>>,
    pub recorded_inputs: BTreeMap<Url, BTreeSet<Url>>,
    /// Image/data roles retained independently of canonical filename suffixes.
    pub recorded_binary_inputs: BTreeMap<Url, BTreeSet<Url>>,
    /// Opened recorder paths and the physical files selected by the last build.
    pub recorded_bindings: BTreeMap<Url, BTreeMap<PathBuf, PathBuf>>,
    pub bibliography_inputs: BTreeMap<Url, BTreeSet<Url>>,
    /// Paths selected by the last complete BibTeX/Biber logs, before symlinks.
    pub bibliography_bindings: BTreeMap<Url, BTreeMap<PathBuf, PathBuf>>,
    /// Input contents from each root's last complete bibliography tool logs.
    pub bibliography_checks: BTreeMap<Url, std::sync::Arc<crate::file_checks::Snapshot>>,
    /// File conditions and input snapshots for the latest published diagnostics.
    pub build_checks: BTreeMap<Url, std::sync::Arc<crate::file_checks::Snapshot>>,
    pub pdf_checks: BTreeMap<Url, std::sync::Arc<crate::file_checks::Snapshot>>,
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
        diagnostics.dedup_by(|a, b| {
            a.range == b.range
                && a.message == b.message
                && a.source == b.source
                && a.code == b.code
                && a.data == b.data
        });
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
        if !self.documents.contains_key(uri) {
            return vec![];
        }
        let mut dependencies = self.source_dependencies_from(uri, root);
        if uri == root {
            if let Some(recorded) = self.recorded_inputs.get(root) {
                // Recorder suffixes cannot distinguish displayed TeX code
                // from executed source. Explicit includes above retain their
                // source role independently, including in another project.
                let displayed = crate::code_inputs::displayed(self, root);
                dependencies.extend(recorded.difference(&displayed).cloned());
            }
            if let Some(recorded) = self.bibliography_inputs.get(root) {
                dependencies.extend(recorded.iter().cloned());
            }
        }
        dependencies
    }

    fn source_dependencies_from(&self, uri: &Url, root: &Url) -> Vec<Url> {
        self.source_dependencies_with_kinds(uri, root)
            .into_iter()
            .map(|(uri, _)| uri)
            .collect()
    }

    fn source_dependencies_with_kinds(&self, uri: &Url, root: &Url) -> Vec<(Url, DocumentKind)> {
        let Some(doc) = self.documents.get(uri) else {
            return vec![];
        };
        doc.syntax
            .commands
            .iter()
            .flat_map(|command| {
                let extension = match command.name.as_str() {
                    "input" | "InputIfFileExists" | "include" | "subfile" => "tex",
                    "bibliography" | "nobibliography" | "addbibresource" => "bib",
                    "bibliographystyle" => "bst",
                    name => match crate::syntax::loader_extension(name) {
                        Some(extension) => extension,
                        None => return vec![],
                    },
                };
                crate::syntax::file_argument(&doc.text.source, command)
                    .map(|arg| {
                        let multiple = crate::syntax::file_list(&command.name);
                        let values = if let Some(spans) =
                            crate::syntax::file_list_spans(&doc.text.source, command)
                        {
                            spans
                                .into_iter()
                                .map(|span| &doc.text.source[span])
                                .collect::<Vec<_>>()
                        } else if multiple {
                            arg.value.split(',').collect::<Vec<_>>()
                        } else {
                            vec![arg.value.as_str()]
                        };
                        values
                            .into_iter()
                            .filter_map(|value| {
                                let value = if command.name == "bibliographystyle" {
                                    value
                                } else {
                                    value.trim()
                                };
                                let kind = match extension {
                                    "bib" => DocumentKind::Bibtex,
                                    "bst" => DocumentKind::BibtexStyle,
                                    _ => DocumentKind::Latex,
                                };
                                resolve_file_path(root, &command.name, value, &[extension])
                                    .or_else(|| {
                                        resolve_file_path(uri, &command.name, value, &[extension])
                                    })
                                    .map(|uri| (uri, kind))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Do not index TeX's distribution packages outside the project. Explicitly
    /// opened or already indexed external documents remain part of the project.
    /// Recorded images and data can be shared outside the roots; their presence
    /// alone does not load their contents into the source document index.
    pub fn project_inputs(&self, root: &Url, inputs: &BTreeSet<Url>) -> BTreeSet<Url> {
        let binary = inputs
            .iter()
            .filter(|uri| {
                uri.to_file_path().ok().is_some_and(|path| {
                    crate::graphics::is_image(&path) || crate::recorder::is_data(&path)
                })
            })
            .cloned()
            .collect();
        self.project_inputs_with_binary(root, inputs, &binary)
    }

    pub(crate) fn project_inputs_with_binary(
        &self,
        root: &Url,
        inputs: &BTreeSet<Url>,
        binary: &BTreeSet<Url>,
    ) -> BTreeSet<Url> {
        let parent = root
            .to_file_path()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        inputs
            .iter()
            .filter(|uri| {
                self.documents.contains_key(*uri)
                    || binary.contains(*uri)
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
        self.project_documents_from(root, root)
    }

    /// Explicit source traversal, independent of recorder and data inputs.
    pub(crate) fn source_documents_from(&self, source: &Url, root: &Url) -> BTreeSet<Url> {
        let mut visited = BTreeSet::new();
        let mut pending = vec![source.clone()];
        while let Some(uri) = pending.pop() {
            if visited.insert(uri.clone()) {
                pending.extend(self.source_dependencies_from(&uri, root));
            }
        }
        visited
    }

    pub(crate) fn project_documents_from(&self, source: &Url, root: &Url) -> BTreeSet<Url> {
        let mut visited = BTreeSet::new();
        let mut pending = vec![source.clone()];
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
            .filter(|(uri, doc)| is_saved_root(uri, doc))
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

    /// Opened path names and selected physical targets, independently of the
    /// source index. A known external alias can select an unindexed target.
    pub(crate) fn project_bindings<'a>(
        &'a self,
        root: &'a Url,
    ) -> impl Iterator<Item = (&'a PathBuf, &'a PathBuf)> {
        [&self.recorded_bindings, &self.bibliography_bindings]
            .into_iter()
            .filter_map(move |by_root| by_root.get(root))
            .flat_map(|bindings| bindings.iter())
    }

    /// Static bibliography references retain their role even when their
    /// physical targets have unrelated suffixes or cannot be parsed as UTF-8.
    pub(crate) fn bibliography_files(&self, root: &Url) -> BTreeSet<Url> {
        self.project_documents(root)
            .into_iter()
            .flat_map(|uri| self.source_dependencies_with_kinds(&uri, root))
            .filter_map(|(uri, kind)| (kind != DocumentKind::Latex).then_some(uri))
            .collect()
    }

    /// Saved sources and image dependencies used by builds and artifact checks.
    pub fn project_files(&self, root: &Url) -> BTreeSet<Url> {
        let mut files = self.project_documents(root);
        files.extend(crate::code_inputs::inputs(self, root));
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
            // Build notifications expose resolved roots. Recognize those URIs
            // before applying a relative setting again in a different folder.
            let requested = path
                .canonicalize()
                .unwrap_or_else(|_| normalize_path(&path));
            for folder in &self.roots {
                let configured = folder.join(root_file);
                let identity = configured
                    .canonicalize()
                    .unwrap_or_else(|_| normalize_path(&configured));
                if identity == requested {
                    return file_uri(&configured);
                }
            }
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
            let selected = file_uri(&path);
            if selected.is_ok()
                || matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("tex" | "ltx")
                )
            {
                return selected;
            }
            // Package/support files may define a command containing
            // begin{document}; continue looking for their containing project.
        }
        let candidates = self
            .documents
            .iter()
            .filter(|(candidate, doc)| {
                *candidate != uri
                    && is_saved_root(candidate, doc)
                    && self.project_files(candidate).contains(uri)
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
        self.reload_document(uri, self.documents.contains_key(uri));
    }

    fn recorded_kind(&self, uri: &Url) -> Option<DocumentKind> {
        let path = uri.to_file_path().ok()?;
        let mut style = false;
        for (name, target) in self
            .bibliography_bindings
            .values()
            .flat_map(|bindings| bindings.iter())
        {
            if *target == path {
                match name.extension().and_then(|ext| ext.to_str()) {
                    Some("bib") => return Some(DocumentKind::Bibtex),
                    Some("bst") => style = true,
                    _ => {}
                }
            }
        }
        if style {
            return Some(DocumentKind::BibtexStyle);
        }
        self.recorded_bindings
            .values()
            .flat_map(|bindings| bindings.iter())
            .any(|(name, target)| {
                *target == path && is_source(name) && !crate::recorder::is_data(name)
            })
            .then_some(DocumentKind::Latex)
    }

    pub(crate) fn reload_recorded(&mut self, uri: &Url, binary: bool) {
        if binary && !self.documents.contains_key(uri) {
            return;
        }
        // A source alias may resolve to a data-looking suffix. Its recorder
        // role is stronger evidence than the canonical name's extension.
        if binary {
            self.reload_document(uri, true);
        } else {
            self.reload_document_as(uri, true, Some(DocumentKind::Latex));
        }
    }

    pub(crate) fn reload_bibliography(&mut self, uri: &Url) {
        // A fresh tool log is a new observation, unlike a passive disk reload.
        let kind_hint = self.recorded_kind(uri);
        self.reload_document_as(uri, true, kind_hint);
    }

    fn reload_document(&mut self, uri: &Url, parse_source: bool) {
        let kind_hint = self
            .documents
            .get(uri)
            .and_then(|doc| doc.kind_hint)
            .or_else(|| self.recorded_kind(uri));
        self.reload_document_as(uri, parse_source || kind_hint.is_some(), kind_hint);
    }

    fn reload_document_as(
        &mut self,
        uri: &Url,
        parse_source: bool,
        kind_hint: Option<DocumentKind>,
    ) {
        // Updating the parser role must preserve unsaved editor contents.
        if let Some(doc) = self
            .documents
            .get_mut(uri)
            .filter(|doc| doc.version.is_some())
        {
            if let Some(kind) = kind_hint {
                doc.set_kind_hint(kind);
            }
            return;
        }
        if self.documents.len() >= MAX_FILES && !self.documents.contains_key(uri) {
            return;
        }
        let source = uri.to_file_path().ok().and_then(|path| {
            if crate::graphics::is_image(&path) && kind_hint.is_none()
                || !parse_source
                    && (crate::recorder::is_data(&path)
                        || self
                            .recorded_binary_inputs
                            .values()
                            .any(|inputs| inputs.contains(uri)))
            {
                return None;
            }
            let metadata = std::fs::metadata(&path).ok()?;
            if !metadata.is_file() || metadata.len() > MAX_FILE_SIZE {
                return None;
            }
            std::fs::read_to_string(path).ok()
        });
        if let Some(source) = source {
            self.documents.insert(
                uri.clone(),
                Document::new_with_kind(uri, source, None, kind_hint),
            );
        } else {
            self.documents.remove(uri);
        }
    }

    pub(crate) fn refresh_paths(&mut self, paths: &[PathBuf]) -> BTreeSet<Url> {
        let matches = |uri: &Url| {
            uri.to_file_path()
                .is_ok_and(|path| paths.iter().any(|changed| path.starts_with(changed)))
        };
        let mut affected: BTreeSet<_> = self
            .documents
            .keys()
            .filter(|uri| matches(uri))
            .cloned()
            .collect();
        for uri in &affected {
            self.reload(uri);
        }
        for path in paths {
            if path.is_dir() {
                self.scan(path, 0);
            } else if is_source(path) {
                if let Ok(uri) = Url::from_file_path(path) {
                    self.reload(&uri);
                    affected.insert(uri);
                }
            }
        }
        affected.extend(self.documents.keys().filter(|uri| matches(uri)).cloned());
        let mut refresh = affected.clone();
        refresh.extend(
            self.saved_projects()
                .into_iter()
                .filter(|(_, project)| project.iter().any(&matches))
                .map(|(root, _)| root),
        );
        for uri in &refresh {
            self.load_dependencies(uri, 0);
        }
        affected.extend(self.documents.keys().filter(|uri| matches(uri)).cloned());
        affected
    }

    pub(crate) fn close(&mut self, uri: &Url) {
        let previous_hint = self.documents.remove(uri).and_then(|doc| doc.kind_hint);
        let kind_hint = previous_hint.or_else(|| self.recorded_kind(uri));
        // Discard unsaved text while retaining the independently known role.
        self.reload_document_as(uri, kind_hint.is_some(), kind_hint);
    }

    pub fn open(&mut self, uri: Url, source: String, version: i32) {
        let uri = document_uri(uri);
        if self.roots.is_empty() {
            if let Ok(path) = uri.to_file_path() {
                if let Some(parent) = path.parent() {
                    self.add_root(parent.to_path_buf());
                }
            }
        }
        let kind_hint = self
            .documents
            .get(&uri)
            .and_then(|doc| doc.kind_hint)
            .or_else(|| self.recorded_kind(&uri));
        self.documents.insert(
            uri.clone(),
            Document::new_with_kind(&uri, source, Some(version), kind_hint),
        );
        self.load_dependencies(&uri, 0);
    }

    pub fn load_dependencies(&mut self, uri: &Url, depth: usize) {
        self.load_dependencies_inner(uri, uri, depth, &mut BTreeSet::new(), false);
    }

    /// Reload each saved dependency before walking its includes: closed child
    /// files may have gained new dependencies since the previous build.
    pub fn refresh_project(&mut self, root: &Url) {
        self.load_dependencies_inner(root, root, 0, &mut BTreeSet::new(), true);
        // Displayed .tex files may already have been scanned as documents.
        // Refresh closed text without following its apparent LaTeX commands.
        for uri in crate::code_inputs::inputs(self, root) {
            if self.documents.contains_key(&uri) {
                self.reload(&uri);
            }
        }
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
        let sources: BTreeMap<_, _> = self
            .source_dependencies_with_kinds(uri, root)
            .into_iter()
            .collect();
        for dependency in dependencies {
            if let Some(kind) = sources.get(&dependency) {
                if let Some(doc) = self.documents.get_mut(&dependency) {
                    doc.set_kind_hint(*kind);
                } else {
                    self.reload_document_as(&dependency, true, Some(*kind));
                }
            } else if !self.documents.contains_key(&dependency) {
                // Explicit source inclusion keeps working for unusual names
                // such as input{notes.txt}; recorder-only data stays unparsed.
                self.reload_document(&dependency, false);
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

pub(crate) fn resolve_file_path(
    uri: &Url,
    command: &str,
    value: &str,
    extensions: &[&str],
) -> Option<Url> {
    resolve_file_path_with(uri, command, value, extensions, existing_file)
}

/// Share TeX's filename lookup with the proposed filesystem used for renames.
pub(crate) fn resolve_file_path_with(
    uri: &Url,
    command: &str,
    value: &str,
    extensions: &[&str],
    file: impl Fn(&Path) -> Option<PathBuf>,
) -> Option<Url> {
    crate::syntax::file_lookup_names(command, value).find_map(|name| {
        extensions
            .iter()
            .find_map(|extension| resolve_path_with(uri, &name, extension, &file))
    })
}

fn existing_file(path: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    canonical.is_file().then_some(canonical)
}

pub fn resolve_path(uri: &Url, value: &str, extension: &str) -> Option<Url> {
    resolve_path_with(uri, value, extension, &existing_file)
}

fn resolve_path_with(
    uri: &Url,
    value: &str,
    extension: &str,
    file: &impl Fn(&Path) -> Option<PathBuf>,
) -> Option<Url> {
    let mut value = value.to_owned();
    for character in ['_', '%', '#', '&', '$', ' '] {
        value = value.replace(&format!("\\{character}"), &character.to_string());
    }
    if value.is_empty() || value.contains(['\\', '{', '}']) {
        return None;
    }
    let parent = uri.to_file_path().ok()?.parent()?.to_path_buf();
    let path = parent.join(&value);
    let selected = file(&path).or_else(|| {
        if extension.is_empty()
            || path.extension().and_then(|value| value.to_str()) == Some(extension)
        {
            return None;
        }
        let mut name = path.as_os_str().to_os_string();
        name.push(".");
        name.push(extension);
        file(&PathBuf::from(name))
    })?;
    Url::from_file_path(selected).ok()
}
