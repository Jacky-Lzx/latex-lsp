//! Layer bounded JSON project settings over the editor's defaults.
use crate::{config::Config, workspace::Workspace};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};
use tower_lsp::lsp_types::Url;

#[derive(Clone)]
struct Cached {
    stamp: (Option<SystemTime>, u64),
    value: Result<Value, String>,
}
static CACHE: OnceLock<Mutex<BTreeMap<PathBuf, Cached>>> = OnceLock::new();

pub(crate) fn invalidate() {
    CACHE.get_or_init(Default::default).lock().unwrap().clear();
}

fn read(path: &Path) -> Option<Result<Value, String>> {
    let metadata = std::fs::metadata(path).ok()?;
    let stamp = (metadata.modified().ok(), metadata.len());
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some(entry) = cache.get(path).filter(|entry| entry.stamp == stamp) {
        return Some(entry.value.clone());
    }
    let value = if metadata.len() > 1024 * 1024 {
        Err("Project configuration exceeds 1 MiB".into())
    } else {
        std::fs::read_to_string(path)
            .map_err(|err| err.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|err| err.to_string()))
    };
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(
        path.into(),
        Cached {
            stamp,
            value: value.clone(),
        },
    );
    Some(value)
}

fn merge(base: &mut Value, patch: Value) {
    if let (Some(base), Value::Object(patch)) = (base.as_object_mut(), &patch) {
        for (name, value) in patch {
            merge(base.entry(name).or_insert(Value::Null), value.clone());
        }
    } else {
        *base = patch;
    }
}

pub(crate) fn load(ws: &Workspace, uri: &Url, project: Option<&Url>) -> (Config, Vec<String>) {
    let Ok(path) = uri.to_file_path() else {
        return (ws.config.clone(), vec![]);
    };
    let Some(parent) = path.parent() else {
        return (ws.config.clone(), vec![]);
    };
    let boundary = ws
        .roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .map(PathBuf::as_path)
        .unwrap_or(parent);
    let mut directories: Vec<_> = parent
        .ancestors()
        .take(32)
        .take_while(|dir| dir.starts_with(boundary))
        .collect();
    directories.reverse();
    let mut value = serde_json::to_value(&ws.config).unwrap();
    let mut errors = Vec::new();
    for directory in directories {
        let file = directory.join(".latex-lsp.json");
        let Some(raw) = read(&file) else {
            continue;
        };
        let result = raw.and_then(|raw| {
            let mut patch = raw.get("latex").cloned().unwrap_or(raw);
            let object = patch
                .as_object_mut()
                .ok_or("Expected a JSON settings object")?;
            if object.contains_key("distribution") { return Err("distribution is a server setting; configure it through initializationOptions or didChangeConfiguration".into()); }
            let projects = object.remove("projects");
            let mut candidate = value.clone();
            merge(&mut candidate, patch);
            if let Some(projects) = projects {
                let projects = projects
                    .as_object()
                    .ok_or("projects must map root filenames to settings objects")?;
                for (root, patch) in projects {
                    let mut validated = candidate.clone();
                    merge(&mut validated, patch.clone());
                    Config::parse(validated)?;
                    if project
                        .and_then(|uri| uri.to_file_path().ok())
                        .is_some_and(|path| {
                            crate::workspace::normalize_path(&directory.join(root))
                                == crate::workspace::normalize_path(&path)
                        })
                    {
                        merge(&mut candidate, patch.clone());
                    }
                }
            }
            let parsed = Config::parse(candidate.clone())?;
            // A rootFile from this file is relative to its own directory.
            if let Some(root) = parsed
                .build
                .root_file
                .as_ref()
                .filter(|root| Path::new(root).is_relative())
            {
                if candidate["build"]["rootFile"] != value["build"]["rootFile"] {
                    candidate["build"]["rootFile"] =
                        Value::String(directory.join(root).to_string_lossy().into_owned());
                }
            }
            Ok(candidate)
        });
        match result {
            Ok(candidate) => value = candidate,
            Err(error) => errors.push(format!("{}: {error}", file.display())),
        }
    }
    (
        Config::parse(value).unwrap_or_else(|_| ws.config.clone()),
        errors,
    )
}
