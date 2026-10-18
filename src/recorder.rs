//! TeX's recorder names the files actually read, including expanded inputs.
use std::{collections::BTreeSet, path::Path};
use tower_lsp::lsp_types::Url;

pub fn inputs(root: &Path, contents: &str) -> BTreeSet<Url> {
    let mut directory = root
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut result = BTreeSet::new();
    for line in contents.lines().take(32768) {
        if let Some(value) = line.strip_prefix("PWD ") {
            let path = Path::new(value.trim_matches('"'));
            if path.is_absolute() {
                directory = path.into();
            }
        } else if let Some(value) = line.strip_prefix("INPUT ") {
            let path = directory.join(value.trim_matches('"'));
            if !crate::workspace::is_source(&path) && !crate::graphics::is_image(&path) {
                continue;
            }
            if let Ok(path) = path.canonicalize() {
                if path.is_file() {
                    if let Ok(uri) = Url::from_file_path(path) {
                        result.insert(uri);
                    }
                }
            }
            if result.len() >= 4096 {
                break;
            }
        }
    }
    result
}

/// Bibliography tools read sources which TeX's recorder usually never sees.
/// BibTeX's auxiliary filename identifies whether latexmk changed directory.
pub fn bibliography_inputs(root: &Path, log_path: &Path, contents: &str) -> BTreeSet<Url> {
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
    let mut result = BTreeSet::new();
    for line in contents.lines().take(32768) {
        let name = if line.starts_with("Database file #") {
            line.split_once(": ").map(|(_, name)| name)
        } else if let Some(name) = line.strip_prefix("The style file: ") {
            Some(name)
        } else {
            line.split_once("Found BibTeX data source '")
                .and_then(|(_, tail)| tail.strip_suffix('\''))
        };
        let Some(name) = name else { continue };
        let name = name.trim().trim_matches('"');
        if !matches!(
            Path::new(name).extension().and_then(|ext| ext.to_str()),
            Some("bib" | "bst")
        ) {
            continue;
        }
        if let Some(uri) = [directory, source_directory, log_directory]
            .into_iter()
            .find_map(|directory| {
                let path = directory.join(name).canonicalize().ok()?;
                path.is_file()
                    .then(|| Url::from_file_path(path).ok())
                    .flatten()
            })
        {
            result.insert(uri);
        }
        if result.len() >= 4096 {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
