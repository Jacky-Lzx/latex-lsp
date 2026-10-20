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

/// Whether a tool supplied new path observations or reused an older artifact.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum BindingStatus {
    Updated,
    Reused,
    Unchanged,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    paths: BTreeMap<PathBuf, Option<PathBuf>>,
    recorded: BTreeMap<PathBuf, Option<PathBuf>>,
    ambiguous: BTreeSet<PathBuf>,
    watched: BTreeSet<Url>,
    artifact: Option<PathBuf>,
    inputs: BTreeMap<Url, InputSnapshot>,
    pdf_contents: Option<InputSnapshot>,
    synctex_contents: BTreeMap<PathBuf, InputSnapshot>,
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
        self.changed_file(&path)
    }

    fn changed_file(&self, path: &Path) -> bool {
        match self {
            Self::Text { length, .. } => {
                std::fs::metadata(path).map_or(true, |metadata| metadata.len() != *length as u64)
                    || std::fs::read_to_string(path)
                        .ok()
                        .is_none_or(|text| Self::text(&text) != *self)
            }
            Self::File(before) => {
                let length = std::fs::metadata(path)
                    .ok()
                    .filter(|metadata| metadata.is_file())
                    .map(|metadata| metadata.len());
                length != before.as_ref().map(|(length, _)| *length) || Self::file(path) != *self
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

/// Metadata detects rewrites of identical outputs; contents detect replacements
/// by tools which preserve the output's length and modification time.
#[derive(Debug, PartialEq)]
pub(crate) struct ArtifactFingerprint {
    metadata: (u64, Option<std::time::SystemTime>),
    contents: InputSnapshot,
}

impl ArtifactFingerprint {
    pub(crate) fn capture(path: &Path) -> Option<Self> {
        Some(Self {
            metadata: fingerprint(path)?,
            contents: InputSnapshot::file(path),
        })
    }
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
        let redefined_bibliography_commands: BTreeSet<_> = sources
            .iter()
            .filter_map(|uri| workspace.documents.get(uri))
            .flat_map(|doc| &doc.syntax.definitions)
            .filter(|entry| {
                matches!(
                    entry.name.as_str(),
                    "bibliography" | "nobibliography" | "bibliographystyle"
                )
            })
            .map(|entry| entry.name.as_str())
            .collect();
        // A missing chapter is meaningful only when it can be selected.
        // Unknown or conflicting filters must not guess that TeX reads it.
        let include_filters: Option<Vec<BTreeSet<_>>> = sources
            .iter()
            .filter_map(|source| workspace.documents.get(source))
            .flat_map(|doc| {
                doc.syntax
                    .commands
                    .iter()
                    .filter(|command| command.name == "includeonly")
                    .map(move |command| {
                        syntax::includeonly_names(&doc.text.source, command)
                            .map(|names| names.into_iter().collect())
                    })
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
        let observe = |origin: &Url, command: &str, name: &str, extension: &str| {
            resolve_file_path_with(origin, command, name, &[extension], |path| {
                let before = file(path);
                let mut paths = paths.borrow_mut();
                if paths.len() < MAX_PATHS && !outputs.contains(&normalize_path(path)) {
                    paths.insert(path.to_path_buf(), before.clone());
                }
                before
            })
        };
        'sources: for source in sources {
            let Some(doc) = workspace.documents.get(&source) else {
                continue;
            };
            for command in &doc.syntax.commands {
                if paths.borrow().len() >= MAX_PATHS {
                    break 'sources;
                }
                if matches!(
                    command.name.as_str(),
                    "bibliography" | "nobibliography" | "bibliographystyle"
                ) {
                    let (names, extension) = if command.name == "bibliographystyle" {
                        if redefined_bibliography_commands.contains("bibliographystyle") {
                            continue;
                        }
                        let Some(argument) = syntax::file_argument(&doc.text.source, command)
                        else {
                            continue;
                        };
                        (vec![argument.value.clone()], "bst")
                    } else {
                        if redefined_bibliography_commands.contains("bibliography")
                            || redefined_bibliography_commands.contains("nobibliography")
                        {
                            continue;
                        }
                        let Some(spans) =
                            syntax::complete_file_list_spans(&doc.text.source, command)
                        else {
                            continue;
                        };
                        (
                            spans
                                .into_iter()
                                .map(|span| doc.text.source[span].to_owned())
                                .collect(),
                            "bib",
                        )
                    };
                    for name in names {
                        if paths.borrow().len() >= MAX_PATHS {
                            break 'sources;
                        }
                        // These names use the real input suffix, including an
                        // explicit .bst; BibTeX's doubled display suffix is
                        // only a log convention, not a second file candidate.
                        for origin in [root, &source] {
                            if observe(origin, &command.name, &name, extension).is_some()
                                || source == *root
                            {
                                break;
                            }
                        }
                    }
                    continue;
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
                    let selected = observe(origin, &command.name, name, "tex");
                    if selected.is_some() || source == *root {
                        break;
                    }
                }
            }
        }
        let paths = paths.into_inner();
        let recorded = [
            &workspace.recorded_bindings,
            &workspace.bibliography_bindings,
        ]
        .into_iter()
        .filter_map(|by_root| by_root.get(root))
        .flat_map(|bindings| bindings.keys())
        .map(|path| (path.clone(), file(path)))
        .collect();
        let mut snapshot = Self {
            paths,
            recorded,
            artifact: config
                .artifact_path(&root_path, "pdf")
                .map(|path| normalize_path(&path)),
            ..Self::default()
        };
        snapshot.refresh_watched();
        snapshot
    }

    fn refresh_watched(&mut self) {
        self.watched = self
            .paths
            .iter()
            .chain(&self.recorded)
            .flat_map(|(path, before)| {
                let mut watched = vec![path.clone()];
                watched.extend(before.clone().or_else(|| canonical_candidate(path)));
                // File URIs collapse `..`, while the kernel traverses it after
                // following symlinks. Retain intermediate paths for events on
                // a directory alias which disappears from the normalized URI.
                if path
                    .components()
                    .any(|part| part == std::path::Component::ParentDir)
                {
                    watched.extend(path.ancestors().skip(1).map(Path::to_path_buf));
                }
                watched
            })
            .filter_map(|path| Url::from_file_path(normalize_path(&path)).ok())
            .collect();
    }

    /// Merge independent tool scopes. Reused tool outputs retain identities
    /// from their last run. Without a new TeX log or PDF, retain the filesystem
    /// conditions captured before this build; the old PDF has its own history.
    pub(crate) fn with_bindings<'a>(
        &self,
        scopes: impl IntoIterator<Item = (&'a BTreeMap<PathBuf, PathBuf>, BindingStatus)>,
    ) -> Self {
        let mut snapshot = self.clone();
        snapshot.recorded.clear();
        snapshot.ambiguous.clear();
        for (bindings, status) in scopes {
            for (path, target) in bindings {
                let before = self
                    .recorded
                    .get(path)
                    .cloned()
                    .unwrap_or_else(|| Some(target.clone()));
                // A retarget during compilation makes the PDF's actual input
                // uncertain, even if the alias is restored before completion.
                if status != BindingStatus::Unchanged && before.as_ref() != Some(target) {
                    snapshot.ambiguous.insert(path.clone());
                }
                // An unchanged tool log describes the last run of that tool,
                // not the current filesystem. A new TeX PDF can still reuse an
                // old .bbl; it must not adopt a retargeted bibliography alias.
                snapshot.recorded.insert(
                    path.clone(),
                    if status == BindingStatus::Reused {
                        Some(target.clone())
                    } else {
                        before
                    },
                );
            }
        }
        snapshot.refresh_watched();
        snapshot
    }

    /// A new TeX PDF can reuse a .bbl generated from older bibliography data.
    /// Once those inputs differ, keep that PDF uncertain even after restoration.
    pub(crate) fn with_reused_inputs(&self, previous: &Self) -> Self {
        let mut snapshot = self.clone();
        snapshot.ambiguous.extend(
            previous
                .inputs
                .iter()
                .filter(|(uri, before)| before.changed(uri))
                .filter_map(|(uri, _)| uri.to_file_path().ok()),
        );
        snapshot
    }

    /// Bibliography tool history follows moved inputs and output directories;
    /// historical PDF checks deliberately retain their original paths.
    pub(crate) fn renamed_inputs(&self, rename: impl Fn(&Path) -> PathBuf) -> Self {
        Self {
            artifact: self.artifact.as_deref().map(&rename),
            inputs: self
                .inputs
                .iter()
                .filter_map(|(uri, before)| {
                    Some((
                        Url::from_file_path(rename(&uri.to_file_path().ok()?)).ok()?,
                        before.clone(),
                    ))
                })
                .collect(),
            ..Self::default()
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

    pub(crate) fn with_artifact_contents(mut self) -> Self {
        self.pdf_contents = self.artifact.as_deref().map(InputSnapshot::file);
        if let Some(pdf) = &self.artifact {
            // Record both names, including missing alternatives: introducing a
            // second format can change which mapping the external tool reads.
            self.synctex_contents = ["synctex.gz", "synctex"]
                .into_iter()
                .map(|extension| {
                    let path = pdf.with_extension(extension);
                    let contents = InputSnapshot::file(&path);
                    (path, contents)
                })
                .collect();
        }
        self
    }

    pub(crate) fn pdf_changed(&self) -> bool {
        self.artifact
            .as_deref()
            .zip(self.pdf_contents.as_ref())
            .is_some_and(|(path, before)| before.changed_file(path))
    }

    pub(crate) fn synctex_changed(&self) -> bool {
        self.synctex_contents
            .iter()
            .any(|(path, before)| before.changed_file(path))
    }

    pub(crate) fn watched(&self) -> &BTreeSet<Url> {
        &self.watched
    }

    pub(crate) fn changed_path(&self) -> Option<&Path> {
        self.paths
            .iter()
            .chain(&self.recorded)
            .find_map(|(path, before)| (file(path) != *before).then_some(path.as_path()))
            .or_else(|| self.ambiguous.first().map(PathBuf::as_path))
    }

    pub(crate) fn matches_pdf(&self, pdf: &Path) -> bool {
        self.artifact.as_ref() == Some(&normalize_path(pdf))
    }

    pub(crate) fn remember(map: &mut BTreeMap<Url, Arc<Self>>, root: Url, snapshot: Arc<Self>) {
        if snapshot.paths.is_empty() && snapshot.recorded.is_empty() && snapshot.inputs.is_empty() {
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn moved_bibliography_history_keeps_content_fingerprints_and_tracks_the_new_output_path() {
        let directory = tempfile::tempdir().unwrap();
        let old = directory.path().join("old");
        let new = directory.path().join("new");
        std::fs::create_dir(&old).unwrap();
        let bibliography = old.join("refs.bib");
        std::fs::write(&bibliography, "OLD\n").unwrap();
        let history = Snapshot {
            artifact: Some(old.join("main.pdf")),
            inputs: [(
                Url::from_file_path(&bibliography).unwrap(),
                InputSnapshot::file(&bibliography),
            )]
            .into(),
            ..Default::default()
        };
        std::fs::rename(&old, &new).unwrap();
        let moved = history.renamed_inputs(|path| new.join(path.strip_prefix(&old).unwrap()));
        assert!(moved.matches_pdf(&new.join("main.pdf")));
        assert!(moved.changed_input().is_none());
        std::fs::write(new.join("refs.bib"), "NEW\n").unwrap();
        assert_eq!(
            moved.changed_input(),
            Some(&Url::from_file_path(new.join("refs.bib")).unwrap())
        );
        let uncertain = Snapshot::default().with_reused_inputs(&moved);
        std::fs::write(new.join("refs.bib"), "OLD\n").unwrap();
        assert!(moved.changed_input().is_none());
        assert_eq!(
            uncertain.changed_path(),
            Some(new.join("refs.bib").as_path())
        );
    }

    #[test]
    fn tex_and_bibliography_scopes_refresh_independently_and_keep_unread_bibliography_history() {
        let directory = tempfile::tempdir().unwrap();
        let root = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let chapter = directory.path().join("chapter.tex");
        let first = directory.path().join("first.bib");
        let second = directory.path().join("second.bib");
        let alias = directory.path().join("refs.bib");
        for path in [&chapter, &first, &second] {
            std::fs::write(path, "contents\n").unwrap();
        }
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let mut workspace = Workspace::default();
        let tex: BTreeMap<_, _> = [(chapter.clone(), chapter.clone())].into();
        let bib: BTreeMap<_, _> = [(alias.clone(), first.clone())].into();
        workspace.recorded_bindings.insert(root.clone(), tex);
        workspace
            .bibliography_bindings
            .insert(root.clone(), bib.clone());
        let before = Snapshot::capture(&workspace, &root, &BuildConfig::default());
        let completed = before.with_bindings([
            (&BTreeMap::new(), BindingStatus::Updated),
            (&bib, BindingStatus::Reused),
        ]);
        std::fs::remove_file(chapter).unwrap();
        assert!(completed.changed_path().is_none());
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&second, &alias).unwrap();
        assert_eq!(completed.changed_path(), Some(alias.as_path()));
        // A new TeX run which did not rerun BibTeX still used the old .bbl.
        let next = Snapshot::capture(&workspace, &root, &BuildConfig::default());
        let unread = next.with_bindings([
            (&BTreeMap::new(), BindingStatus::Updated),
            (&bib, BindingStatus::Reused),
        ]);
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        assert_eq!(unread.changed_path(), Some(alias.as_path()));
        let rebuilt =
            Snapshot::capture(&workspace, &root, &BuildConfig::default()).with_bindings([
                (&BTreeMap::new(), BindingStatus::Updated),
                (&bib, BindingStatus::Updated),
            ]);
        assert!(rebuilt.changed_path().is_none());
    }

    #[test]
    fn recorder_parent_traversal_watches_the_directory_alias_before_dotdot() {
        let directory = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        std::fs::create_dir(shared.path().join("child")).unwrap();
        let data = shared.path().join("table.csv");
        std::fs::write(&data, "data\n").unwrap();
        let alias = directory.path().join("link");
        std::os::unix::fs::symlink(shared.path().join("child"), &alias).unwrap();
        let opened = alias.join("../table.csv");
        let root = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let mut workspace = Workspace::default();
        workspace
            .recorded_bindings
            .insert(root.clone(), [(opened.clone(), data.clone())].into());
        let before = Snapshot::capture(&workspace, &root, &BuildConfig::default());
        assert!(before
            .watched()
            .contains(&Url::from_file_path(&alias).unwrap()));
        assert!(before
            .watched()
            .contains(&Url::from_file_path(&data).unwrap()));
        let lexical = directory.path().join("table.csv");
        assert!(before
            .watched()
            .contains(&Url::from_file_path(lexical).unwrap()));
        std::fs::remove_file(alias).unwrap();
        assert_eq!(before.changed_path(), Some(opened.as_path()));
    }

    #[test]
    fn recorded_bindings_capture_current_identity_and_keep_ambiguous_pdfs_invalid() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.csv");
        let second = directory.path().join("second.csv");
        let alias = directory.path().join("table.csv");
        for path in [&first, &second] {
            std::fs::write(path, "same bytes\n").unwrap();
        }
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let root = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let mut workspace = Workspace::default();
        workspace
            .recorded_bindings
            .insert(root.clone(), [(alias.clone(), first.clone())].into());
        let before = Snapshot::capture(&workspace, &root, &BuildConfig::default());
        assert!(before.changed_path().is_none());
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&second, &alias).unwrap();
        assert_eq!(before.changed_path(), Some(alias.as_path()));
        let complete = before.with_bindings([(
            &[(alias.clone(), second.clone())].into(),
            BindingStatus::Updated,
        )]);
        assert_eq!(complete.changed_path(), Some(alias.as_path()));
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        assert!(before.changed_path().is_none());
        // The newly written PDF has uncertain provenance; restoring the old
        // alias must not retroactively validate it.
        assert_eq!(complete.changed_path(), Some(alias.as_path()));
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&second, &alias).unwrap();
        let next = Snapshot::capture(&workspace, &root, &BuildConfig::default());
        assert!(next
            .watched()
            .contains(&Url::from_file_path(&second).unwrap()));
        let rebuilt = next.with_bindings([(&[(alias, second)].into(), BindingStatus::Updated)]);
        assert!(rebuilt.changed_path().is_none());
    }

    #[test]
    fn new_recorder_scope_drops_unused_bindings_but_keeps_static_conditions() {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("table.csv");
        let condition = directory.path().join("optional.tex");
        std::fs::write(&data, "OLD\n").unwrap();
        let before = Snapshot {
            paths: [(condition.clone(), None)].into(),
            recorded: [(data.clone(), Some(data.clone()))].into(),
            ..Default::default()
        };
        let complete = before.with_bindings([(&BTreeMap::new(), BindingStatus::Updated)]);
        assert!(!complete
            .watched()
            .contains(&Url::from_file_path(&data).unwrap()));
        assert!(complete
            .watched()
            .contains(&Url::from_file_path(&condition).unwrap()));
        std::fs::remove_file(&data).unwrap();
        assert!(complete.changed_path().is_none());
        std::fs::write(&condition, "new\n").unwrap();
        assert_eq!(complete.changed_path(), Some(condition.as_path()));
    }
}

#[cfg(test)]
mod bibliography_conditions {
    use super::*;

    fn capture(root: &Path, source: &str) -> Snapshot {
        std::fs::write(root.join("main.tex"), source).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(root.into());
        Snapshot::capture(
            &workspace,
            &Url::from_file_path(root.join("main.tex")).unwrap(),
            &BuildConfig::default(),
        )
    }

    #[test]
    fn static_bibliography_conditions_watch_real_suffixes_comments_lists_and_compacted_names() {
        for (command, extension, stem) in [
            ("bibliography", "bib", "mi ssing"),
            ("nobibliography", "bib", "mi ssing"),
            ("bibliographystyle", "bst", "missing,style"),
        ] {
            for explicit in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                std::fs::create_dir(root.join("db")).unwrap();
                let suffix = if explicit {
                    format!(".{extension}")
                } else {
                    String::new()
                };
                let name = format!("db/{stem}{suffix}");
                let item = if extension == "bib" {
                    format!("first,% 😀 before\r\n {name}% after,\r\n,second")
                } else {
                    format!("% 😀 before\r\n {name}% after\r\n")
                };
                let source = format!("\\{command}{{{item}}}\r\n");
                let snapshot = capture(root, &source);
                assert!(snapshot.changed_path().is_none(), "{source}");
                let actual = root.join(format!("db/{}.{extension}", stem.replace(' ', "")));
                assert!(
                    snapshot
                        .watched()
                        .contains(&Url::from_file_path(&actual).unwrap()),
                    "{snapshot:?}"
                );
                let doubled = actual.with_file_name(format!(
                    "{}.{extension}",
                    actual.file_name().unwrap().to_str().unwrap()
                ));
                std::fs::write(doubled, "Decoy.").unwrap();
                assert!(snapshot.changed_path().is_none(), "{source}");
                std::fs::write(&actual, "New input.\n").unwrap();
                assert!(snapshot.changed_path().is_some(), "{source}");
            }
        }
    }

    #[test]
    fn dynamic_incomplete_ignored_and_redefined_declarations_do_not_invent_conditions() {
        for source in [
            r"\bibliography{missing,\chosen}",
            r"\nobibliography{missing,\chosen}",
            r"\bibliography*{missing}",
            r"\nobibliography*{missing}",
            r"\bibliography[note]{missing}",
            "\\bibliography{mi% split\nssing}",
            r"\bibliographystyle{\chosen}",
            r"\bibliographystyle*{missing}",
            r"\bibliographystyle{ missing}",
            r"\newcommand{\bibliography}[1]{}\bibliography{missing}",
            r"\newcommand{\nobibliography}[1]{}\bibliography{missing}",
            r"\newcommand{\bibliographystyle}[1]{}\bibliographystyle{missing}",
            r"\verb|\bibliography{missing}|",
            r"% \bibliographystyle{missing}",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let snapshot = capture(root, source);
            assert!(snapshot.paths.is_empty(), "{source}\n{snapshot:?}");
            for extension in ["bib", "bst"] {
                std::fs::write(root.join(format!("missing.{extension}")), "New input.").unwrap();
            }
            assert!(snapshot.changed_path().is_none(), "{source}");
        }
    }

    #[test]
    fn database_and_style_redefinitions_only_disable_their_own_conditions() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for (source, extension) in [
            (
                r"\newcommand{\bibliographystyle}[1]{}\bibliography{missing}",
                "bib",
            ),
            (
                r"\newcommand{\bibliography}[1]{}\bibliographystyle{missing}",
                "bst",
            ),
        ] {
            let snapshot = capture(root, source);
            let actual = root.join(format!("missing.{extension}"));
            std::fs::write(&actual, "New input.").unwrap();
            assert!(snapshot.changed_path().is_some(), "{source}");
            std::fs::remove_file(actual).unwrap();
        }
    }

    #[test]
    fn bibliography_lookups_watch_root_candidates_before_source_relative_fallbacks() {
        for (command, extension) in [("bibliography", "bib"), ("bibliographystyle", "bst")] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            std::fs::create_dir(root.join("sub")).unwrap();
            std::fs::write(
                root.join("sub/settings.tex"),
                format!("\\{command}{{missing}}\n"),
            )
            .unwrap();
            std::fs::write(
                root.join(format!("sub/missing.{extension}")),
                "Existing input.\n",
            )
            .unwrap();
            let snapshot = capture(root, r"\input{sub/settings}");
            assert!(snapshot.changed_path().is_none());
            std::fs::write(
                root.join(format!("missing.{extension}")),
                "Higher priority input.\n",
            )
            .unwrap();
            assert!(snapshot.changed_path().is_some());
        }
    }

    #[test]
    #[cfg(unix)]
    fn first_static_bibliography_aliases_detect_retargets_without_tool_history() {
        use std::os::unix::fs::symlink;
        for (command, extension) in [("bibliography", "bib"), ("bibliographystyle", "bst")] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let old = root.join(format!("old.{extension}"));
            let new = root.join(format!("new.{extension}"));
            std::fs::write(&old, "Old input.\n").unwrap();
            std::fs::write(&new, "New input.\n").unwrap();
            let alias = root.join(format!("alias.{extension}"));
            symlink(&old, &alias).unwrap();
            let snapshot = capture(root, &format!("\\{command}{{alias}}"));
            assert!(snapshot.changed_path().is_none());
            std::fs::remove_file(&alias).unwrap();
            symlink(&new, &alias).unwrap();
            assert!(snapshot.changed_path().is_some());
        }
    }

    #[test]
    #[cfg(unix)]
    fn missing_bibliography_files_under_directory_aliases_watch_physical_paths() {
        use std::os::unix::fs::symlink;
        for (command, extension) in [("bibliography", "bib"), ("bibliographystyle", "bst")] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let project = root.join("project");
            let shared = root.join("shared");
            std::fs::create_dir(&project).unwrap();
            std::fs::create_dir(&shared).unwrap();
            symlink(&shared, project.join("alias")).unwrap();
            let snapshot = capture(&project, &format!("\\{command}{{alias/missing}}"));
            let physical = shared.join(format!("missing.{extension}"));
            assert!(
                snapshot
                    .watched()
                    .contains(&Url::from_file_path(&physical).unwrap()),
                "{snapshot:?}"
            );
            std::fs::write(physical, "New input.\n").unwrap();
            assert!(snapshot.changed_path().is_some());
        }
    }

    #[test]
    fn bibliography_conditions_share_the_existing_candidate_budget() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let names = (0..MAX_PATHS + 10)
            .map(|n| format!("missing{n}"))
            .collect::<Vec<_>>()
            .join(",");
        let snapshot = capture(root, &format!("\\bibliography{{{names}}}"));
        assert_eq!(snapshot.paths.len(), MAX_PATHS);
        assert!(snapshot.changed_path().is_none());
    }
}
