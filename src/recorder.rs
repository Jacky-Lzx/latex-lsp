//! TeX's recorder names the files actually read, including expanded inputs.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use tower_lsp::lsp_types::Url;

/// Files commonly read as data with openin or package-specific table loaders.
/// Their contents are dependencies, not implicitly LaTeX source documents.
pub(crate) const DATA_EXTENSIONS: &[&str] =
    &["csv", "tsv", "dat", "json", "xml", "yaml", "yml", "txt"];

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
            if let Ok(path) = path.canonicalize() {
                if path.is_file() {
                    if let Ok(uri) = Url::from_file_path(path) {
                        if binary {
                            result.binary.insert(uri.clone());
                        }
                        result.all.insert(uri);
                    }
                }
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
    let mut result = BTreeSet::new();
    for name in contents.lines().take(32768).filter_map(bibliography_source) {
        if !matches!(
            Path::new(name.trim_matches('"'))
                .extension()
                .and_then(|ext| ext.to_str()),
            Some("bib" | "bst")
        ) {
            continue;
        }
        if let Some(uri) = paths
            .resolve(name)
            .and_then(|path| Url::from_file_path(path).ok())
        {
            result.insert(uri);
        }
        if result.len() >= 4096 {
            break;
        }
    }
    result
}

pub(crate) fn bibliography_source(line: &str) -> Option<&str> {
    if line.starts_with("Database file #") {
        line.split_once(": ").map(|(_, name)| name.trim())
    } else if let Some(name) = line.strip_prefix("The style file: ") {
        Some(name.trim())
    } else {
        line.split_once("Found BibTeX data source '")
            .and_then(|(_, tail)| tail.strip_suffix('\''))
    }
}

/// Keep dependency discovery and diagnostic locations in the same search order.
pub(crate) struct BibliographyPaths<'a> {
    directories: [&'a Path; 3],
    bib_directories: Vec<PathBuf>,
    bst_directories: Vec<PathBuf>,
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
        Self {
            directories: [directory, source_directory, log_directory],
            bib_directories: Vec::new(),
            bst_directories: Vec::new(),
        }
    }

    /// latexmk prepends source paths before invoking BibTeX in another cwd.
    /// Its reported search paths take precedence over cwd-based fallbacks.
    pub(crate) fn with_search_output(mut self, output: &str) -> Self {
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
        let name = name.trim().trim_matches('"');
        let search = if name.ends_with(".bst") {
            &self.bst_directories
        } else {
            &self.bib_directories
        };
        search
            .iter()
            .map(PathBuf::as_path)
            .chain(self.directories)
            .find_map(|directory| {
                let path = directory.join(name).canonicalize().ok()?;
                path.is_file().then_some(path)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn recorder_preserves_binary_alias_roles_before_canonicalizing_suffixes() {
        let directory = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        let image = shared.path().join("image.tex");
        let data = shared.path().join("data.payload");
        let source = shared.path().join("source.csv");
        let local_source = directory.path().join("local.csv");
        let mut log = format!("PWD {}\n", directory.path().display());
        for (alias, target) in [
            ("image.PNG", &image),
            ("data.CSV", &data),
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
            [&image, &data, &source, &local_source]
                .map(|path| Url::from_file_path(path).unwrap())
                .into()
        );
        assert_eq!(
            files.binary,
            [&image, &data]
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
