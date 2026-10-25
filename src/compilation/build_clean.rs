//! Explicit, bounded cleanup of one configured build's known artifacts.
use crate::{config::BuildConfig, project::workspace::Workspace};
use serde_json::{json, Value};
use std::{
    collections::{hash_map::DefaultHasher, BTreeSet},
    hash::Hasher,
    io::Read,
    path::{Path, PathBuf},
};
use tower_lsp::lsp_types::Url;

const AUXILIARY: &[&str] = &[
    "aux",
    "bbl",
    "blg",
    "bcf",
    "run.xml",
    "fdb_latexmk",
    "fls",
    "log",
    "toc",
    "lof",
    "lot",
    "loa",
    "nav",
    "snm",
    "vrb",
    "out",
    "idx",
    "ind",
    "ilg",
    "glo",
    "gls",
    "glg",
    "acn",
    "acr",
    "alg",
    "nlo",
    "nls",
    "nlg",
];
const MAX_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, PartialEq)]
struct Stamp {
    length: u64,
    modified: Option<std::time::SystemTime>,
    identity: Identity,
    hash: u64,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl Identity {
    fn of(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        #[cfg(not(unix))]
        let _ = metadata;
        Self {
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        }
    }
}

fn stamp(path: &Path, budget: u64) -> Result<Stamp, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > budget {
        return Err("Not a regular file or cleanup byte budget exceeded".into());
    }
    let mut reader = std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(budget + 1);
    let mut hash = DefaultHasher::new();
    let mut length = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        length += count as u64;
        hash.write(&buffer[..count]);
    }
    if length > budget || length != metadata.len() {
        return Err("Artifact changed or cleanup byte budget exceeded".into());
    }
    Ok(Stamp {
        length,
        modified: metadata.modified().ok(),
        identity: Identity::of(&metadata),
        hash: hash.finish(),
    })
}

struct Entry {
    original: PathBuf,
    path: PathBuf,
    target: PathBuf,
    stamp: Stamp,
}

pub(crate) struct Plan {
    pub(crate) directory: PathBuf,
    original_directory: PathBuf,
    target: Option<PathBuf>,
    directory_identity: Option<Identity>,
    entries: Vec<Entry>,
    skipped: Vec<Value>,
}

impl Plan {
    pub(crate) fn prepare(
        ws: &Workspace,
        root: &Url,
        config: &BuildConfig,
        artifacts: bool,
    ) -> Result<Self, String> {
        let root_path = root
            .to_file_path()
            .map_err(|_| "Cleanup requires a local file")?;
        let original_directory = config
            .output_path(&root_path)
            .ok_or("Invalid output directory")?;
        let directory = crate::project::workspace::normalize_path(&original_directory);
        if original_directory.canonicalize().ok() != directory.canonicalize().ok() {
            return Err(
                "Cleanup requires an unambiguous output directory; choose its direct path".into(),
            );
        }
        let target = match directory.canonicalize() {
            Ok(path) if path.is_dir() => Some(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            _ => return Err("Cannot resolve build output directory".into()),
        };
        let directory_identity = target
            .as_ref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|metadata| Identity::of(&metadata));
        let mut paths: BTreeSet<_> = AUXILIARY
            .iter()
            .filter_map(|extension| config.artifact_path(&root_path, extension))
            .collect();
        if artifacts {
            paths.extend(
                ["pdf", "synctex", "synctex.gz"]
                    .iter()
                    .filter_map(|extension| config.artifact_path(&root_path, extension)),
            );
        }
        // Explicit index jobs can have a different job name. Restrict suffixes
        // even for custom tools: configuration must not authorize deleting source.
        for tool in &config.index_tools {
            let mut selected = config.clone();
            if tool.job_name.is_some() {
                selected.job_name.clone_from(&tool.job_name);
            }
            paths.extend(
                [&tool.input_extension, &tool.output_extension]
                    .into_iter()
                    .filter(|extension| AUXILIARY.contains(&extension.as_str()))
                    .filter_map(|extension| selected.artifact_path(&root_path, extension)),
            );
        }
        paths.extend(
            crate::compilation::index_processing::outputs(&root_path, config)
                .into_iter()
                .filter(|path| {
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| AUXILIARY.contains(&extension))
                }),
        );
        // A complete recorder can confirm included chapters' auxiliary files.
        // Never interpret arbitrary OUTPUT entries as deletion instructions.
        if let Some(fls) = config.artifact_path(&root_path, "fls") {
            if std::fs::symlink_metadata(&fls)
                .is_ok_and(|meta| meta.file_type().is_file() && meta.len() <= 4 * 1024 * 1024)
            {
                let mut contents = String::new();
                std::fs::File::open(fls)
                    .map_err(|e| e.to_string())?
                    .take(4 * 1024 * 1024 + 1)
                    .read_to_string(&mut contents)
                    .map_err(|e| e.to_string())?;
                if contents.len() <= 4 * 1024 * 1024
                    && !crate::server::output::truncated(&contents)
                    && contents.lines().count() <= 32768
                {
                    let mut pwd = root_path.parent().unwrap().to_owned();
                    for line in contents.lines() {
                        if let Some(value) = line.strip_prefix("PWD ") {
                            let value = Path::new(value.trim_matches('"'));
                            if value.is_absolute() {
                                pwd = value.to_owned();
                            }
                        } else if let Some(value) = line.strip_prefix("OUTPUT ") {
                            let path = pwd.join(value.trim_matches('"'));
                            if path.extension().is_some_and(|extension| extension == "aux") {
                                paths.insert(path);
                            }
                        }
                    }
                }
            }
        }
        if paths.len() > 4096 {
            return Err("Cleanup file budget exceeded".into());
        }
        let protected: BTreeSet<_> = ws
            .documents
            .keys()
            .cloned()
            .chain(ws.project_files(root))
            .filter_map(|uri| uri.to_file_path().ok())
            .chain(crate::compilation::index_processing::dependencies(
                &root_path, config,
            ))
            .chain(std::iter::once(root_path))
            .filter_map(|path| path.canonicalize().ok())
            .collect();
        #[cfg(unix)]
        let protected_identities: BTreeSet<_> = protected
            .iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|meta| Identity::of(&meta))
            .collect();
        let mut plan = Self {
            directory,
            original_directory,
            target,
            directory_identity,
            entries: vec![],
            skipped: vec![],
        };
        let mut budget = MAX_BYTES;
        let mut seen = BTreeSet::new();
        for original in paths {
            let path = crate::project::workspace::normalize_path(&original);
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("Cannot inspect {}: {error}", path.display())),
            };
            let resolved = path.canonicalize().ok();
            let reason = if !metadata.file_type().is_file() {
                Some("not-regular-file")
            } else if original.canonicalize().ok() != resolved {
                Some("ambiguous-path")
            } else if resolved.as_ref().is_none_or(|path| {
                plan.target
                    .as_ref()
                    .is_none_or(|directory| !path.starts_with(directory))
            }) {
                Some("outside-output-directory")
            } else if resolved
                .as_ref()
                .is_some_and(|path| protected.contains(path))
            {
                Some("source-or-open-document")
            } else {
                None
            };
            #[cfg(unix)]
            let reason = reason.or_else(|| {
                protected_identities
                    .contains(&Identity::of(&metadata))
                    .then_some("source-hard-link")
            });
            if let Some(reason) = reason {
                plan.skipped
                    .push(json!({"uri":Url::from_file_path(&path).ok(),"reason":reason}));
                continue;
            }
            let resolved = resolved.unwrap();
            if !seen.insert(resolved.clone()) {
                continue;
            }
            let stamp = stamp(&path, budget)
                .map_err(|error| format!("Cannot plan {}: {error}", path.display()))?;
            budget -= stamp.length;
            plan.entries.push(Entry {
                original,
                path,
                target: resolved,
                stamp,
            });
        }
        Ok(plan)
    }

    fn valid_directory(&self) -> bool {
        self.directory.canonicalize().ok() == self.target
            && self.original_directory.canonicalize().ok() == self.target
            && self
                .target
                .as_ref()
                .and_then(|path| std::fs::metadata(path).ok())
                .map(|metadata| Identity::of(&metadata))
                == self.directory_identity
    }

    fn valid_entry(&self, entry: &Entry) -> bool {
        self.valid_directory()
            && entry.original.canonicalize().ok().as_ref() == Some(&entry.target)
            && entry.path.canonicalize().ok().as_ref() == Some(&entry.target)
            && stamp(&entry.path, MAX_BYTES).ok().as_ref() == Some(&entry.stamp)
    }

    pub(crate) fn execute(&self, dry_run: bool) -> Result<Value, String> {
        if !self.valid_directory() || self.entries.iter().any(|entry| !self.valid_entry(entry)) {
            return Err(
                "Build artifacts changed during cleanup planning; request cleanup again".into(),
            );
        }
        let files: Vec<_> = self
            .entries
            .iter()
            .map(|entry| Url::from_file_path(&entry.path).unwrap())
            .collect();
        let mut removed = vec![];
        let mut errors = vec![];
        if !dry_run {
            for (entry, uri) in self.entries.iter().zip(&files) {
                if !self.valid_entry(entry) {
                    errors
                        .push(json!({"uri":uri,"message":"Artifact or output directory changed"}));
                    break;
                }
                match std::fs::remove_file(&entry.path) {
                    Ok(()) => removed.push(uri),
                    Err(error) => errors.push(json!({"uri":uri,"message":error.to_string()})),
                }
            }
        }
        Ok(
            json!({"dryRun":dry_run,"files":files,"removed":removed,"skipped":self.skipped,"errors":errors}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Workspace, Url, BuildConfig) {
        let dir = tempfile::tempdir().unwrap();
        let root = Url::from_file_path(dir.path().join("main.tex")).unwrap();
        std::fs::write(root.to_file_path().unwrap(), "\\documentclass{article}").unwrap();
        let mut ws = Workspace::default();
        ws.documents.insert(
            root.clone(),
            crate::project::workspace::Document::new(
                &root,
                "\\documentclass{article}".into(),
                Some(1),
            ),
        );
        let config = BuildConfig {
            output_directory: Some("out".into()),
            job_name: Some("任务".into()),
            ..Default::default()
        };
        (dir, ws, root, config)
    }

    #[test]
    #[cfg(unix)]
    fn parent_traversal_across_directory_aliases_cannot_clean_the_wrong_output_tree() {
        use std::os::unix::fs::symlink;
        let (dir, ws, root, mut config) = fixture();
        std::fs::create_dir_all(dir.path().join("nested/deeper")).unwrap();
        for parent in ["out", "nested/out"] {
            std::fs::create_dir(dir.path().join(parent)).unwrap();
            std::fs::write(dir.path().join(parent).join("任务.aux"), "preserve").unwrap();
        }
        symlink(dir.path().join("nested/deeper"), dir.path().join("alias")).unwrap();
        config.output_directory = Some("alias/../out".into());
        assert!(Plan::prepare(&ws, &root, &config, false).is_err());
        for parent in ["out", "nested/out"] {
            assert!(dir.path().join(parent).join("任务.aux").exists());
        }
    }

    #[test]
    #[cfg(unix)]
    fn ambiguous_recorder_paths_do_not_authorize_the_lexically_matching_auxiliary() {
        use std::os::unix::fs::symlink;
        let (dir, ws, root, config) = fixture();
        std::fs::create_dir_all(dir.path().join("nested/deeper")).unwrap();
        for parent in ["out", "nested/out"] {
            std::fs::create_dir(dir.path().join(parent)).unwrap();
            std::fs::write(dir.path().join(parent).join("chapter.aux"), "preserve").unwrap();
        }
        symlink(dir.path().join("nested/deeper"), dir.path().join("alias")).unwrap();
        std::fs::write(
            dir.path().join("out/任务.fls"),
            format!(
                "PWD {}\nOUTPUT alias/../out/chapter.aux\n",
                dir.path().display()
            ),
        )
        .unwrap();
        let result = Plan::prepare(&ws, &root, &config, false)
            .unwrap()
            .execute(false)
            .unwrap();
        assert_eq!(result["skipped"][0]["reason"], "ambiguous-path");
        assert_eq!(result["removed"].as_array().unwrap().len(), 1);
        for parent in ["out", "nested/out"] {
            assert!(dir.path().join(parent).join("chapter.aux").exists());
        }
    }

    #[test]
    #[cfg(unix)]
    fn original_parent_path_bindings_are_revalidated_even_when_the_normalized_path_is_unchanged() {
        use std::os::unix::fs::symlink;
        let (dir, ws, root, mut config) = fixture();
        std::fs::create_dir(dir.path().join("inner")).unwrap();
        std::fs::create_dir_all(dir.path().join("nested/deeper")).unwrap();
        for parent in ["out", "nested/out"] {
            std::fs::create_dir(dir.path().join(parent)).unwrap();
            std::fs::write(dir.path().join(parent).join("任务.aux"), "preserve").unwrap();
        }
        symlink(dir.path().join("inner"), dir.path().join("alias")).unwrap();
        config.output_directory = Some("alias/../out".into());
        let plan = Plan::prepare(&ws, &root, &config, false).unwrap();
        assert_eq!(
            plan.execute(true).unwrap()["files"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_file(dir.path().join("alias")).unwrap();
        symlink(dir.path().join("nested/deeper"), dir.path().join("alias")).unwrap();
        assert!(plan.execute(false).is_err());
        for parent in ["out", "nested/out"] {
            assert!(dir.path().join(parent).join("任务.aux").exists());
        }
    }

    #[test]
    fn preview_is_read_only_and_auxiliary_cleanup_preserves_pdf_sources_and_other_jobs() {
        let (dir, ws, root, config) = fixture();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        for name in [
            "任务.aux",
            "任务.run.xml",
            "任务.pdf",
            "任务.synctex.gz",
            "任务.bib",
            "任务.ist",
            "other.aux",
        ] {
            std::fs::write(out.join(name), name).unwrap();
        }
        let plan = Plan::prepare(&ws, &root, &config, false).unwrap();
        assert_eq!(
            plan.execute(true).unwrap()["files"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(out.join("任务.aux").exists());
        assert_eq!(
            plan.execute(false).unwrap()["removed"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        for name in [
            "任务.pdf",
            "任务.synctex.gz",
            "任务.bib",
            "任务.ist",
            "other.aux",
        ] {
            assert!(out.join(name).exists());
        }
        let full = Plan::prepare(&ws, &root, &config, true)
            .unwrap()
            .execute(false)
            .unwrap();
        assert_eq!(full["removed"].as_array().unwrap().len(), 2);
        assert!(root.to_file_path().unwrap().exists());
    }

    #[test]
    fn recorder_only_adds_aux_files_inside_the_output_directory() {
        let (dir, ws, root, config) = fixture();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("chapters")).unwrap();
        for path in [
            out.join("chapters/one.aux"),
            out.join("generated.bib"),
            dir.path().join("outside.aux"),
        ] {
            std::fs::write(path, "keep or clean").unwrap();
        }
        std::fs::write(out.join("任务.fls"), format!("PWD {}\nOUTPUT out/chapters/one.aux\nOUTPUT outside.aux\nOUTPUT out/generated.bib\n", dir.path().display())).unwrap();
        let result = Plan::prepare(&ws, &root, &config, false)
            .unwrap()
            .execute(false)
            .unwrap();
        assert_eq!(result["removed"].as_array().unwrap().len(), 2);
        assert_eq!(result["skipped"][0]["reason"], "outside-output-directory");
        assert!(out.join("generated.bib").exists());
        assert!(dir.path().join("outside.aux").exists());
    }

    #[test]
    fn opened_artifacts_and_directories_are_protected() {
        let (dir, mut ws, root, config) = fixture();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("任务.log")).unwrap();
        let path = out.join("任务.aux");
        std::fs::write(&path, "unsaved source").unwrap();
        let uri = Url::from_file_path(&path).unwrap();
        ws.documents.insert(
            uri.clone(),
            crate::project::workspace::Document::new(&uri, "unsaved".into(), Some(2)),
        );
        let result = Plan::prepare(&ws, &root, &config, false)
            .unwrap()
            .execute(false)
            .unwrap();
        assert_eq!(result["removed"], json!([]));
        assert_eq!(result["skipped"].as_array().unwrap().len(), 2);
        assert!(path.exists());
    }

    #[test]
    fn changed_artifact_aborts_before_any_file_is_deleted_and_missing_directory_stays_missing() {
        let (dir, ws, root, config) = fixture();
        let out = dir.path().join("out");
        assert_eq!(
            Plan::prepare(&ws, &root, &config, false)
                .unwrap()
                .execute(false)
                .unwrap()["files"],
            json!([])
        );
        assert!(!out.exists());
        std::fs::create_dir(&out).unwrap();
        std::fs::write(out.join("任务.aux"), "original").unwrap();
        std::fs::write(out.join("任务.log"), "original").unwrap();
        let plan = Plan::prepare(&ws, &root, &config, false).unwrap();
        std::fs::write(out.join("任务.log"), "modified").unwrap();
        assert!(plan.execute(false).is_err());
        assert!(out.join("任务.aux").exists());
    }

    #[test]
    #[cfg(unix)]
    fn symlinks_hard_linked_sources_and_retargeted_output_directories_are_protected() {
        use std::os::unix::fs::symlink;
        let (dir, ws, root, config) = fixture();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        symlink(root.to_file_path().unwrap(), out.join("任务.aux")).unwrap();
        std::fs::hard_link(root.to_file_path().unwrap(), out.join("任务.log")).unwrap();
        std::fs::write(out.join("任务.bbl"), "original").unwrap();
        let plan = Plan::prepare(&ws, &root, &config, false).unwrap();
        assert_eq!(
            plan.execute(true).unwrap()["skipped"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        std::fs::rename(&out, dir.path().join("old")).unwrap();
        std::fs::create_dir(&out).unwrap();
        std::fs::write(out.join("任务.bbl"), "replacement").unwrap();
        assert!(plan.execute(false).is_err());
        assert!(out.join("任务.bbl").exists());
        assert!(dir.path().join("old/任务.bbl").exists());
    }

    #[test]
    fn named_index_jobs_are_cleaned_but_custom_source_suffixes_are_not() {
        let (dir, ws, root, mut config) = fixture();
        config.index_tools = serde_json::from_value(json!([
            {"executable":"makeindex","jobName":"names","inputExtension":"idx","outputExtension":"ind"},
            {"executable":"custom","jobName":"source","inputExtension":"tex","outputExtension":"bib"}
        ])).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        for name in [
            "names.idx",
            "names.ind",
            "names.ilg",
            "source.tex",
            "source.bib",
        ] {
            std::fs::write(out.join(name), name).unwrap();
        }
        let result = Plan::prepare(&ws, &root, &config, false)
            .unwrap()
            .execute(false)
            .unwrap();
        assert_eq!(result["removed"].as_array().unwrap().len(), 3);
        assert!(out.join("source.tex").exists());
        assert!(out.join("source.bib").exists());
    }
}
