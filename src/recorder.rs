//! TeX's recorder names the files actually read, including expanded inputs.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tower_lsp::lsp_types::Url;

/// Files read as data, configuration or verbatim code by package-specific loaders.
/// Their contents are dependencies, not implicitly LaTeX source documents.
pub(crate) const DATA_EXTENSIONS: &[&str] = &[
    "csv", "tsv", "dat", "json", "xml", "yaml", "yml", "txt", "rs", "py", "pyi", "ipynb", "c",
    "cc", "cpp", "cxx", "h", "hh", "hpp", "hxx", "cu", "cuh", "cs", "fs", "fsx", "go", "java",
    "js", "mjs", "cjs", "jsx", "ts", "tsx", "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "rb",
    "php", "pl", "pm", "lua", "r", "jl", "m", "mm", "kt", "kts", "swift", "scala", "sc", "sql",
    "html", "htm", "css", "scss", "sass", "less", "vue", "svelte", "f", "for", "f90", "f95", "f03",
    "f08", "pas", "pp", "asm", "s", "toml", "ini", "cfg", "conf", "md", "rst",
];

pub(crate) fn is_data(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            DATA_EXTENSIONS
                .iter()
                .any(|data| extension.eq_ignore_ascii_case(data))
        })
}

pub fn inputs(root: &Path, contents: &str) -> BTreeSet<Url> {
    classified_inputs(root, contents).all
}

#[derive(Default)]
pub(crate) struct InputFiles {
    pub all: BTreeSet<Url>,
    pub binary: BTreeSet<Url>,
    /// Database roles reported by a bibliography tool, regardless of suffix.
    pub bibliography_databases: BTreeSet<Url>,
    /// Absolute names TeX opened, before following symlinks or resolving `..`.
    pub bindings: BTreeMap<PathBuf, PathBuf>,
    /// Logged names which disappeared before the log could be inspected.
    pub unresolved_bindings: BTreeSet<PathBuf>,
}

/// Preserve the type of the name TeX opened. Canonicalization can resolve an
/// image/data alias to an extensionless file, or even one ending in tex.
pub(crate) fn classified_inputs(root: &Path, contents: &str) -> InputFiles {
    let mut directory = root
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut result = InputFiles::default();
    for line in contents.lines().take(32768) {
        if let Some(value) = line.strip_prefix("PWD ") {
            let path = Path::new(value.trim_matches('"'));
            if path.is_absolute() {
                directory = path.into();
            }
        } else if let Some(value) = line.strip_prefix("INPUT ") {
            let path = directory.join(value.trim_matches('"'));
            let binary = crate::graphics::is_image(&path) || is_data(&path);
            if !crate::workspace::is_source(&path) && !binary {
                continue;
            }
            if let Some(target) = path.canonicalize().ok().filter(|target| target.is_file()) {
                if let Ok(uri) = Url::from_file_path(&target) {
                    if binary {
                        result.binary.insert(uri.clone());
                    }
                    result.all.insert(uri);
                    if result.bindings.len() < 4096 {
                        result.bindings.insert(path, target);
                    }
                }
            } else if result.unresolved_bindings.len() < 4096 {
                result.unresolved_bindings.insert(path);
            }
            if result.all.len() >= 4096 {
                break;
            }
        }
    }
    result
}

/// Bibliography tools read sources which TeX's recorder usually never sees.
/// BibTeX's auxiliary filename identifies whether latexmk changed directory.
pub fn bibliography_inputs(root: &Path, log_path: &Path, contents: &str) -> BTreeSet<Url> {
    let paths = BibliographyPaths::new(root, log_path, contents);
    bibliography_inputs_in(&paths, contents)
}

pub(crate) fn bibliography_inputs_in(
    paths: &BibliographyPaths<'_>,
    contents: &str,
) -> BTreeSet<Url> {
    classified_bibliography_inputs(paths, contents).all
}

pub(crate) fn classified_bibliography_inputs(
    paths: &BibliographyPaths<'_>,
    contents: &str,
) -> InputFiles {
    let mut result = InputFiles::default();
    for name in contents.lines().take(32768).filter_map(bibliography_source) {
        if let Some((path, target)) = paths.resolve_binding(name) {
            if let Ok(uri) = Url::from_file_path(&target) {
                if paths.is_database(name) {
                    result.bibliography_databases.insert(uri.clone());
                }
                result.all.insert(uri);
                if result.bindings.len() < 4096 {
                    result.bindings.insert(path, target);
                }
            }
        } else {
            result.unresolved_bindings.extend(
                paths
                    .candidates(name)
                    .take(4096 - result.unresolved_bindings.len()),
            );
        }
        if result.all.len() >= 4096 {
            break;
        }
    }
    result
}

pub(crate) fn bibliography_source(line: &str) -> Option<&str> {
    bibliography_database_source(line).or_else(|| {
        line.strip_prefix("The style file: ")
            .map(str::trim)
            .filter(|name| name.trim_matches('"').ends_with(".bst"))
    })
}

pub(crate) fn bibliography_database_source(line: &str) -> Option<&str> {
    if line.starts_with("Database file #") {
        line.split_once(": ")
            .map(|(_, name)| name.trim())
            .filter(|name| name.trim_matches('"').ends_with(".bib"))
    } else {
        line.split_once("INFO - ")
            .and_then(|(_, message)| message.strip_prefix("Found BibTeX data source '"))
            .and_then(|tail| tail.strip_suffix('\''))
            .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
    }
}

/// Keep dependency discovery and diagnostic locations in the same search order.
pub(crate) struct BibliographyPaths<'a> {
    directories: [&'a Path; 3],
    bib_directories: Vec<PathBuf>,
    bst_directories: Vec<PathBuf>,
    /// Complete logged database names, before resolution.
    bibliography_names: BTreeSet<String>,
    /// BibTeX prints an extra suffix for already suffixed style arguments.
    style_names: BTreeMap<String, String>,
    bibliography_sources: OnceLock<BibliographySources>,
}

#[derive(Default)]
struct BibliographySources {
    raw: BTreeMap<String, Option<PathBuf>>,
    targets: BTreeSet<PathBuf>,
    opened: BTreeSet<PathBuf>,
    missing: BTreeSet<PathBuf>,
    candidates: BTreeSet<PathBuf>,
    extensions: BTreeSet<String>,
    extensionless_names: BTreeSet<String>,
}

impl BibliographySources {
    fn record_suffix(&mut self, path: &Path) {
        if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
            self.extensions.insert(extension.to_owned());
        } else if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
            self.extensionless_names.insert(name.to_owned());
        }
    }
}

pub(crate) struct RecordedBibliographySource {
    /// A missing, changed or unselected input has no confirmed location.
    pub path: Option<PathBuf>,
    /// False when only the complete name in another search slot is known.
    pub selected: bool,
}

/// Remove redundant separators and `.` without folding a symlink's `..`.
/// An existing parent also gives missing leaves a physical directory identity.
fn bibliography_binding_names(path: &Path) -> impl Iterator<Item = PathBuf> {
    let physical = path.file_name().and_then(|name| {
        path.parent()?
            .canonicalize()
            .ok()
            .map(|parent| parent.join(name))
    });
    std::iter::once(path.components().collect()).chain(physical)
}

impl<'a> BibliographyPaths<'a> {
    pub(crate) fn new(root: &'a Path, log_path: &'a Path, contents: &str) -> Self {
        let source_directory = root.parent().unwrap_or_else(|| Path::new("."));
        let log_directory = log_path.parent().unwrap_or(source_directory);
        let directory = contents
            .lines()
            .take(32768)
            .find_map(|line| line.strip_prefix("The top-level auxiliary file: "))
            .map(|name| {
                let auxiliary = crate::workspace::normalize_path(
                    &source_directory.join(name.trim().trim_matches('"')),
                );
                if auxiliary == crate::workspace::normalize_path(&log_path.with_extension("aux")) {
                    source_directory
                } else {
                    log_directory
                }
            })
            .unwrap_or(source_directory);
        let mut bibliography_names = BTreeSet::new();
        for name in contents
            .lines()
            .take(32768)
            .filter_map(bibliography_database_source)
            .map(|name| name.trim().trim_matches('"'))
        {
            bibliography_names.insert(name);
            if bibliography_names.len() >= 4096 {
                break;
            }
        }
        Self {
            directories: [directory, source_directory, log_directory],
            bib_directories: Vec::new(),
            bst_directories: Vec::new(),
            bibliography_names: bibliography_names.into_iter().map(str::to_owned).collect(),
            style_names: contents
                .lines()
                .take(32768)
                .filter_map(|line| line.strip_prefix("The style file: "))
                .map(|name| name.trim().trim_matches('"'))
                .filter(|name| name.ends_with(".bst.bst"))
                .take(4096)
                .map(|name| (name.into(), name[..name.len() - 4].into()))
                .collect(),
            bibliography_sources: OnceLock::new(),
        }
    }

    /// latexmk prepends source paths before invoking BibTeX in another cwd.
    /// Its reported search paths take precedence over cwd-based fallbacks.
    pub(crate) fn with_search_output(mut self, output: &str) -> Self {
        // The selected names depend on the final tool-specific search paths.
        self.bibliography_sources = OnceLock::new();
        for line in output.lines().take(32768) {
            for (prefix, directories) in [
                ("BIBINPUTS='", &mut self.bib_directories),
                ("BSTINPUTS='", &mut self.bst_directories),
            ] {
                if let Some(tail) = line.trim().strip_prefix(prefix) {
                    if let Some(value) = tail.strip_suffix("'.").or_else(|| tail.strip_suffix('\''))
                    {
                        *directories = std::env::split_paths(value)
                            .take(256)
                            .map(|path| self.directories[0].join(path))
                            .collect();
                    }
                }
            }
        }
        self
    }

    pub(crate) fn resolve(&self, name: &str) -> Option<PathBuf> {
        self.resolve_binding(name).map(|(_, target)| target)
    }

    fn is_database(&self, name: &str) -> bool {
        self.bibliography_names
            .contains(name.trim().trim_matches('"'))
    }

    pub(crate) fn recorded_bibliography_source(
        &self,
        name: &str,
    ) -> Option<RecordedBibliographySource> {
        if self.bibliography_names.is_empty() {
            return None;
        }
        let sources = self.sources();
        if let Some(target) = sources.raw.get(name) {
            // Once a source was selected, disappearance must not silently
            // switch its raw name to an unrelated fallback database.
            return Some(RecordedBibliographySource {
                path: target
                    .as_ref()
                    .filter(|target| self.resolve(name).as_ref() == Some(*target))
                    .cloned(),
                selected: true,
            });
        }
        // Prefer a confirmed tool input over an unreported same-named file in
        // a fallback directory. Candidate order still selects between inputs.
        let mut named = false;
        let mut selected = false;
        for path in self.database_candidates(name) {
            if let Some(target) = path.canonicalize().ok().filter(|path| path.is_file()) {
                if sources.targets.contains(&target) {
                    return Some(RecordedBibliographySource {
                        path: Some(target),
                        selected: true,
                    });
                }
            }
            for binding in bibliography_binding_names(&path) {
                let input = sources.opened.contains(&binding) || sources.missing.contains(&binding);
                selected |= input;
                named |= input || sources.candidates.contains(&binding);
            }
        }
        named.then_some(RecordedBibliographySource {
            path: None,
            selected,
        })
    }

    /// Filter ordinary message words before attempting path equivalence on
    /// additional database suffixes. Bare .bib paths retain the existing rules.
    pub(crate) fn could_name_database(&self, name: &str) -> bool {
        let path = Path::new(name);
        if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
            extension == "bib" || self.sources().extensions.contains(extension)
        } else {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| self.sources().extensionless_names.contains(name))
        }
    }

    fn sources(&self) -> &BibliographySources {
        self.bibliography_sources.get_or_init(|| {
            let mut sources = BibliographySources::default();
            for name in &self.bibliography_names {
                sources.record_suffix(Path::new(name));
                let missing = if let Some((opened, target)) = self.resolve_binding(name) {
                    sources.record_suffix(&target);
                    sources.opened.extend(bibliography_binding_names(&opened));
                    sources.targets.insert(target.clone());
                    sources.raw.insert(name.clone(), Some(target));
                    false
                } else {
                    sources.raw.insert(name.clone(), None);
                    true
                };
                // Other search candidates still delimit a complete source
                // name, but cannot supply a different database's location.
                for path in self.candidates(name) {
                    if sources.candidates.len() >= 8192
                        && (!missing || sources.missing.len() >= 8192)
                    {
                        break;
                    }
                    for binding in bibliography_binding_names(&path) {
                        if sources.candidates.len() < 8192 {
                            sources.candidates.insert(binding.clone());
                        }
                        if missing && sources.missing.len() < 8192 {
                            sources.missing.insert(binding);
                        }
                    }
                }
            }
            sources
        })
    }

    fn resolve_binding(&self, name: &str) -> Option<(PathBuf, PathBuf)> {
        self.candidates(name).find_map(|path| {
            let target = path.canonicalize().ok()?;
            target.is_file().then_some((path, target))
        })
    }

    fn database_candidates<'b>(&'b self, name: &'b str) -> impl Iterator<Item = PathBuf> + 'b {
        self.bib_directories
            .iter()
            .map(PathBuf::as_path)
            .chain(self.directories)
            .map(move |directory| directory.join(name.trim().trim_matches('"')))
    }

    fn candidates<'b>(&'b self, name: &'b str) -> impl Iterator<Item = PathBuf> + 'b {
        let name = name.trim().trim_matches('"');
        // This is a printed name, not a fallback search. Even when both files
        // exist, BibTeX opened foo.bst for a logged foo.bst.bst. Other tools and
        // database names retain their literal spelling.
        let database = self.is_database(name);
        let name = if database {
            name
        } else {
            self.style_names.get(name).map_or(name, String::as_str)
        };
        let search = if !database && name.ends_with(".bst") {
            &self.bst_directories
        } else {
            &self.bib_directories
        };
        search
            .iter()
            .map(PathBuf::as_path)
            .chain(self.directories)
            .map(move |directory| directory.join(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biber_database_roles_select_bibinput_paths_even_for_style_suffixes() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        for folder in ["data", "styles"] {
            std::fs::create_dir(root.join(folder)).unwrap();
            for name in ["refs.bst", "refs.bst.bst", "references", "article.bst"] {
                std::fs::write(root.join(folder).join(name), "@book{alpha}\n").unwrap();
            }
        }
        let main = root.join("main.tex");
        let log_path = root.join("main.blg");
        for name in ["refs.bst", "refs.bst.bst", "references"] {
            let log =
                format!("INFO - Found BibTeX data source '{name}'\nThe style file: article.bst\n");
            let paths = BibliographyPaths::new(&main, &log_path, &log)
                .with_search_output("BIBINPUTS='data'\nBSTINPUTS='styles'\n");
            let inputs = classified_bibliography_inputs(&paths, &log);
            let database = Url::from_file_path(root.join("data").join(name)).unwrap();
            let style = Url::from_file_path(root.join("styles/article.bst")).unwrap();
            assert_eq!(inputs.all, [database.clone(), style].into());
            assert_eq!(inputs.bibliography_databases, [database].into());
            assert_eq!(
                paths
                    .recorded_bibliography_source(&format!("./{name}"))
                    .unwrap()
                    .path,
                Some(root.join("data").join(name)),
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn bibliography_source_equivalence_preserves_symlink_parents_and_reconfigured_search_paths() {
        let directory = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("build")).unwrap();
        std::fs::create_dir(shared.path().join("child")).unwrap();
        std::os::unix::fs::symlink(shared.path().join("child"), root.join("selected")).unwrap();
        let name = "refs.bib': copy.bib";
        let local = root.join(name);
        let actual = shared.path().join(name);
        std::fs::write(&local, "@book{wrong}\n").unwrap();
        std::fs::write(&actual, "@book{entry}\n").unwrap();
        let main = root.join("main.tex");
        let blg = root.join("build/main.blg");
        let log = format!("INFO - Found BibTeX data source '{name}'\n");
        let paths = BibliographyPaths::new(&main, &blg, &log);
        assert_eq!(
            paths
                .recorded_bibliography_source(local.to_str().unwrap())
                .unwrap()
                .path,
            Some(local.clone())
        );
        let paths = paths.with_search_output(&format!(
            "BIBINPUTS='{}'\n",
            root.join("selected/..").display()
        ));
        assert_eq!(
            paths
                .recorded_bibliography_source(actual.to_str().unwrap())
                .unwrap()
                .path,
            Some(actual.clone())
        );
        assert!(paths
            .recorded_bibliography_source(local.to_str().unwrap())
            .unwrap()
            .path
            .is_none());
        std::fs::create_dir(root.join("unrelated")).unwrap();
        let unrelated = root.join("unrelated").join(name);
        std::fs::write(&unrelated, "@book{unrelated}\n").unwrap();
        assert!(paths
            .recorded_bibliography_source(unrelated.to_str().unwrap())
            .is_none());
        let inputs = classified_bibliography_inputs(&paths, &log);
        assert_eq!(inputs.all, [Url::from_file_path(&actual).unwrap()].into());
        assert_eq!(
            inputs.bindings,
            [(root.join("selected/..").join(name), actual.clone())].into()
        );
        std::fs::remove_file(&actual).unwrap();
        for source in [
            actual.to_str().unwrap().to_owned(),
            format!("./{name}"),
            name.to_owned(),
        ] {
            assert!(
                paths
                    .recorded_bibliography_source(&source)
                    .unwrap()
                    .path
                    .is_none(),
                "{source}"
            );
        }
        // A fresh tool run can select the fallback file, but a diagnostic
        // explicitly naming the missing preferred path must remain unresolved.
        let fresh = BibliographyPaths::new(&main, &blg, &log).with_search_output(&format!(
            "BIBINPUTS='{}'\n",
            root.join("selected/..").display()
        ));
        assert!(fresh
            .recorded_bibliography_source(actual.to_str().unwrap())
            .unwrap()
            .path
            .is_none());
        assert_eq!(
            fresh
                .recorded_bibliography_source(&format!("./{name}"))
                .unwrap()
                .path,
            Some(local)
        );
    }

    #[test]
    #[cfg(unix)]
    fn bibliography_bindings_keep_selected_search_paths_before_symlink_resolution() {
        let directory = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("build")).unwrap();
        std::fs::create_dir(shared.path().join("child")).unwrap();
        std::fs::write(directory.path().join("refs.bib"), "decoy\n").unwrap();
        let bib = shared.path().join("refs.bib");
        let style = shared.path().join("physical.bst");
        std::fs::write(&bib, "@book{shared}\n").unwrap();
        std::fs::write(&style, "style\n").unwrap();
        std::os::unix::fs::symlink(
            shared.path().join("child"),
            directory.path().join("selected"),
        )
        .unwrap();
        let style_alias = directory.path().join("named style.bst");
        std::os::unix::fs::symlink(&style, &style_alias).unwrap();
        let main = directory.path().join("main.tex");
        let blg = directory.path().join("build/main.blg");
        let log = "The top-level auxiliary file: build/main.aux\nDatabase file #1: refs.bib\nINFO - Found BibTeX data source 'refs.bib'\nThe style file: \"named style.bst\"\nDatabase file #2: missing.bib\n";
        let paths = BibliographyPaths::new(&main, &blg, log).with_search_output(&format!(
            "BIBINPUTS='{}'\nBSTINPUTS='{}'\n",
            directory.path().join("selected/..").display(),
            directory.path().display()
        ));
        let inputs = classified_bibliography_inputs(&paths, log);
        assert_eq!(
            inputs.all,
            [&bib, &style]
                .map(|path| Url::from_file_path(path).unwrap())
                .into()
        );
        assert_eq!(
            inputs.bindings,
            [
                (directory.path().join("selected/../refs.bib"), bib),
                (style_alias, style),
            ]
            .into()
        );
        assert_eq!(bibliography_inputs_in(&paths, log), inputs.all);
    }

    #[test]
    #[cfg(unix)]
    fn recorder_keeps_opened_alias_names_and_symlink_parent_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let child = shared.path().join("child");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(shared.path().join("table.csv"), "REAL\n").unwrap();
        std::fs::write(directory.path().join("table.csv"), "DECOY\n").unwrap();
        std::os::unix::fs::symlink(&child, directory.path().join("link")).unwrap();
        let opened = directory.path().join("link/../table.csv");
        let files = classified_inputs(
            &directory.path().join("main.tex"),
            &format!(
                "PWD {}\nINPUT link/../table.csv\nINPUT table.csv\n",
                directory.path().display()
            ),
        );
        assert_eq!(files.bindings[&opened], shared.path().join("table.csv"));
        assert_eq!(
            files.bindings[&directory.path().join("table.csv")],
            directory.path().join("table.csv")
        );
        assert_eq!(files.all.len(), 2);
    }

    #[test]
    #[cfg(unix)]
    fn recorder_preserves_binary_alias_roles_before_canonicalizing_suffixes() {
        let directory = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let image = shared.path().join("image.tex");
        let data = shared.path().join("data.payload");
        let code = shared.path().join("code.tex");
        let code_source = shared.path().join("source.rs");
        let source = shared.path().join("source.csv");
        let local_source = directory.path().join("local.csv");
        let mut log = format!("PWD {}\n", directory.path().display());
        for (alias, target) in [
            ("image.PNG", &image),
            ("data.CSV", &data),
            ("code.PY", &code),
            ("code-source.sty", &code_source),
            ("package.sty", &source),
            ("local.sty", &local_source),
        ] {
            std::fs::write(target, "contents").unwrap();
            std::os::unix::fs::symlink(target, directory.path().join(alias)).unwrap();
            log.push_str(&format!("INPUT {alias}\n"));
        }
        std::fs::write(&local_source, "\\label{source-role}\n").unwrap();
        let root = directory.path().join("main.tex");
        let files = classified_inputs(&root, &log);
        assert_eq!(
            files.all,
            [&image, &data, &code, &code_source, &source, &local_source]
                .map(|path| Url::from_file_path(path).unwrap())
                .into()
        );
        assert_eq!(
            files.binary,
            [&image, &data, &code]
                .map(|path| Url::from_file_path(path).unwrap())
                .into()
        );
        assert_eq!(inputs(&root, &log), files.all);
        let mut workspace = crate::workspace::Workspace::default();
        let root_uri = Url::from_file_path(root).unwrap();
        let selected = workspace.project_inputs_with_binary(&root_uri, &files.all, &files.binary);
        // A distribution source alias resolving to a data-looking suffix must
        // not gain the privileges of an image/data input.
        let local_source_uri = Url::from_file_path(local_source).unwrap();
        let mut expected = files.binary.clone();
        expected.insert(local_source_uri.clone());
        assert_eq!(selected, expected);
        workspace
            .recorded_binary_inputs
            .insert(root_uri, files.binary.clone());
        for uri in &selected {
            workspace.reload_recorded(uri, files.binary.contains(uri));
            workspace.reload(uri);
        }
        assert_eq!(workspace.documents.len(), 1);
        assert_eq!(
            workspace.documents[&local_source_uri].syntax.labels[0].name,
            "source-role"
        );
    }

    #[test]
    fn recorder_tracks_confirmed_data_reads_without_outputs_missing_files_or_directories() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("main.tex");
        let mut contents = format!("PWD {}\n", directory.path().display());
        let mut expected = BTreeSet::new();
        for name in [
            "中文 table.csv",
            "table.TSV",
            "raw.dat",
            "data.json",
            "data.xml",
            "data.yaml",
            "data.yml",
            "data.txt",
            "code.RS",
            "code.py",
            "types.pyi",
            "notebook.ipynb",
            "main.CPP",
            "header.hpp",
            "kernel.cu",
            "program.java",
            "server.go",
            "component.tsx",
            "module.mjs",
            "build.sh",
            "query.sql",
            "script.lua",
            "analysis.R",
            "program.f90",
            "app.kt",
            "page.html",
            "style.scss",
            "settings.toml",
            "build.ini",
            "config.conf",
        ] {
            let path = directory.path().join(name);
            std::fs::write(&path, "data").unwrap();
            contents.push_str(&format!("INPUT {name}\nINPUT {name}\n"));
            expected.insert(Url::from_file_path(path).unwrap());
        }
        std::fs::write(directory.path().join("output.csv"), "generated").unwrap();
        std::fs::write(directory.path().join("main.aux"), "auxiliary").unwrap();
        std::fs::create_dir(directory.path().join("directory.dat")).unwrap();
        contents.push_str(
            "OUTPUT output.csv\nINPUT main.aux\nINPUT missing.csv\nINPUT directory.dat\n",
        );
        assert_eq!(inputs(&root, &contents), expected);
    }

    #[test]
    fn bibliography_search_paths_reported_by_latexmk_override_cwd_and_remain_tool_specific() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let output = root.join("build");
        std::fs::create_dir(&output).unwrap();
        for base in [root, &output] {
            for name in ["refs.bib", "local.bst"] {
                std::fs::write(base.join(name), "").unwrap();
            }
        }
        let log = "The top-level auxiliary file: main.aux\nDatabase file #1: refs.bib\nThe style file: local.bst\n";
        let main = root.join("main.tex");
        let blg = output.join("main.blg");
        let paths = BibliographyPaths::new(&main, &blg, log).with_search_output(&format!(
            "BIBINPUTS='ignored'\n  BIBINPUTS='{}'\n  BSTINPUTS='{}'.\n",
            std::env::join_paths([root, &output])
                .unwrap()
                .to_str()
                .unwrap(),
            std::env::join_paths([Path::new("")])
                .unwrap()
                .to_str()
                .unwrap(),
        ));
        assert_eq!(
            bibliography_inputs_in(&paths, log),
            [
                Url::from_file_path(root.join("refs.bib")).unwrap(),
                Url::from_file_path(output.join("local.bst")).unwrap()
            ]
            .into()
        );
    }

    #[test]
    #[cfg(unix)]
    fn bibliography_bindings_preserve_literal_quotes_and_attached_punctuation() {
        for quote in ['\'', '"'] {
            for suffix in [
                ":copy.bib",
                ";copy.bib",
                ")copy.bib",
                "]copy.bib",
                "}copy.bib",
                ",copy.bib",
                " line 700 copy.bib",
                ": copy.bib",
                "; copy.bib",
                ") copy.bib",
                ", line 700 copy.bib",
                ", in file 'wrong.bib' copy.bib",
            ] {
                let directory = tempfile::tempdir().unwrap();
                let root = directory.path();
                let name = format!("refs.bib{quote}{suffix}");
                let path = root.join(&name);
                std::fs::write(&path, "@book{entry}\n").unwrap();
                std::fs::write(root.join("refs.bib"), "decoy\n").unwrap();
                let main = root.join("main.tex");
                let blg = root.join("main.blg");
                let log = format!("INFO - Found BibTeX data source '{name}'\n");
                let paths = BibliographyPaths::new(&main, &blg, &log);
                let inputs = classified_bibliography_inputs(&paths, &log);
                assert_eq!(inputs.all, [Url::from_file_path(&path).unwrap()].into());
                assert_eq!(inputs.bindings, [(path.clone(), path.clone())].into());
                assert!(inputs.unresolved_bindings.is_empty());
                std::fs::remove_file(&path).unwrap();
                let inputs = classified_bibliography_inputs(&paths, &log);
                assert!(inputs.all.is_empty());
                assert!(inputs.bindings.is_empty());
                assert_eq!(inputs.unresolved_bindings, [path].into());
            }
        }
    }

    #[test]
    fn bibliography_inputs_resolve_bibtex_and_biber_names_without_diagnostic_false_positives() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir_all(root.join("build")).unwrap();
        for name in [
            "中文 refs.bib",
            "author's refs.bib",
            "local.bst",
            "ignored.tex",
        ] {
            std::fs::write(root.join(name), "").unwrap();
        }
        let log = "The top-level auxiliary file: build/main.aux\nDatabase file #1: 中文 refs.bib\nDatabase file #2: \"中文 refs.bib\"\nThe style file: local.bst\nINFO - Found BibTeX data source 'author's refs.bib'\nWARN - Missing 'unread.bib'\nDatabase file #3: ignored.tex\nDatabase file #4: missing.bib\n";
        let inputs = bibliography_inputs(&root.join("main.tex"), &root.join("build/main.blg"), log);
        let expected = ["中文 refs.bib", "author's refs.bib", "local.bst"]
            .map(|name| Url::from_file_path(root.join(name)).unwrap())
            .into();
        assert_eq!(inputs, expected);
    }

    #[test]
    fn bibliography_inputs_follow_bibtex_working_directory_before_fallbacks() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("build")).unwrap();
        for name in ["refs.bib", "build/refs.bib", "build/fallback.bst"] {
            std::fs::write(root.join(name), "").unwrap();
        }
        for (auxiliary, expected) in [
            ("main.aux", "build/refs.bib"),
            ("build/main.aux", "refs.bib"),
        ] {
            let log = format!("The top-level auxiliary file: {auxiliary}\nDatabase file #1: refs.bib\nThe style file: fallback.bst\n");
            let inputs =
                bibliography_inputs(&root.join("main.tex"), &root.join("build/main.blg"), &log);
            assert_eq!(
                inputs,
                [
                    Url::from_file_path(root.join(expected)).unwrap(),
                    Url::from_file_path(root.join("build/fallback.bst")).unwrap()
                ]
                .into()
            );
        }
    }

    #[test]
    fn recorder_handles_spaces_unicode_pwd_changes_and_duplicate_inputs() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("chapters")).unwrap();
        for name in [
            "main.tex",
            "chapters/中文 file.tex",
            "local.sty",
            "main.aux",
            "figure.PNG",
        ] {
            std::fs::write(directory.path().join(name), "").unwrap();
        }
        let root = directory.path().join("main.tex");
        let contents = format!("PWD {}\nINPUT main.tex\nINPUT ./main.tex\nOUTPUT local.sty\nINPUT local.sty\nINPUT main.aux\nINPUT figure.PNG\nINPUT missing.tex\nPWD {}\nINPUT 中文 file.tex\n",directory.path().display(),directory.path().join("chapters").display());
        let inputs = inputs(&root, &contents);
        assert_eq!(inputs.len(), 4);
        assert!(inputs.contains(
            &Url::from_file_path(directory.path().join("chapters/中文 file.tex")).unwrap()
        ));
    }
}

#[cfg(test)]
mod suffixed_style_names {
    use super::*;

    #[test]
    fn logged_extra_style_suffixes_select_the_real_file_even_with_an_existing_namesake() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main = root.join("main.tex");
        let single = root.join("local,style.bst");
        let double = root.join("local,style.bst.bst");
        std::fs::write(&main, "Main.").unwrap();
        std::fs::write(&single, "Real style").unwrap();
        std::fs::write(&double, "Unrelated style").unwrap();
        let blg = root.join("main.blg");
        let log = "The style file: local,style.bst.bst\n";
        let paths = BibliographyPaths::new(&main, &blg, log);
        assert_eq!(paths.resolve("local,style.bst.bst"), Some(single.clone()));
        let inputs = classified_bibliography_inputs(&paths, log);
        assert_eq!(
            inputs.all,
            BTreeSet::from([Url::from_file_path(&single).unwrap()])
        );
        assert_eq!(
            inputs.bindings,
            BTreeMap::from([(single.clone(), single.clone())])
        );
        let database = "Database file #1: local,style.bst.bst\n";
        let database_paths = BibliographyPaths::new(&main, &blg, database);
        assert_eq!(database_paths.resolve("local,style.bst.bst"), Some(double));
        std::fs::remove_file(&single).unwrap();
        let inputs = classified_bibliography_inputs(&paths, log);
        assert!(inputs.all.is_empty());
        assert!(inputs.unresolved_bindings.contains(&single));
    }

    #[test]
    fn printed_style_names_remove_only_the_single_extra_suffix_and_keep_search_precedence() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("build")).unwrap();
        std::fs::create_dir(root.join("styles")).unwrap();
        let main = root.join("main.tex");
        let blg = root.join("build/main.blg");
        let input = root.join("styles/local.bst.bst");
        std::fs::write(&input, "Selected style").unwrap();
        std::fs::write(root.join("build/local.bst.bst"), "Fallback namesake").unwrap();
        let log = "The top-level auxiliary file: main.aux\nThe style file: local.bst.bst.bst\n";
        let paths = BibliographyPaths::new(&main, &blg, log)
            .with_search_output("BSTINPUTS='../styles:'.\n");
        assert_eq!(paths.resolve("local.bst.bst.bst"), Some(input));
    }
}
