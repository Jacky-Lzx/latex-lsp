//! Resolve dynamic rc settings only at a build boundary, never during polling.
use super::{BuildConfig, BuildResolution, Engine};
use crate::compilation::file_checks::InputSnapshot;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::{atomic::AtomicBool, Mutex, OnceLock},
};

#[derive(Clone)]
struct Cached {
    config: BuildConfig,
    inputs: Vec<(PathBuf, InputSnapshot)>,
}
static CACHE: OnceLock<Mutex<BTreeMap<String, Cached>>> = OnceLock::new();

fn key(root: &Path, config: &BuildConfig) -> String {
    let mut environment: Vec<_> = std::env::vars_os().collect();
    environment.sort();
    format!(
        "{}\n{}\n{environment:?}",
        root.display(),
        serde_json::to_string(&config.unresolved()).unwrap()
    )
}

pub(super) fn cached(root: &Path, config: &BuildConfig) -> Option<BuildConfig> {
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if cache.is_empty() {
        return None;
    }
    let key = key(root, config);
    let entry = cache.get(&key)?;
    for (path, _) in &entry.inputs {
        crate::diagnostics::diagnostic_inputs::observe(path);
    }
    if entry
        .inputs
        .iter()
        .all(|(path, before)| InputSnapshot::file(path) == *before)
    {
        return Some(entry.config.clone());
    }
    cache.remove(&key);
    None
}

/// Include missing higher-priority candidates so later file creation invalidates
/// the effective configuration, as well as edits to selected files.
pub(super) fn candidates(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let system = match std::env::var_os("LATEXMKRCSYS") {
        Some(path) => vec![PathBuf::from(path)],
        None => ["LatexMk", "latexmkrc"]
            .into_iter()
            .flat_map(|name| {
                [
                    "/cygdrive/c/latexmk",
                    "/etc",
                    "/opt/local/share/latexmk",
                    "/usr/local/share/latexmk",
                    "/usr/local/lib/latexmk",
                ]
                .into_iter()
                .map(move |parent| Path::new(parent).join(name))
            })
            .collect(),
    };
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    let user: Vec<_> = xdg
        .map(|path| path.join("latexmk/latexmkrc"))
        .into_iter()
        .chain(home.map(|path| path.join(".latexmkrc")))
        .collect();
    let local: Vec<_> = root
        .parent()
        .into_iter()
        .flat_map(|parent| [parent.join(".latexmkrc"), parent.join("latexmkrc")])
        .collect();
    for group in [system, user, local] {
        for path in group {
            let selected = crate::diagnostics::diagnostic_inputs::is_file(&path);
            paths.push(path);
            if selected {
                break;
            }
        }
    }
    paths
}

#[derive(Deserialize)]
struct Effective {
    output: String,
    auxiliary: String,
    final_output: String,
    mode: u32,
    sources: Vec<PathBuf>,
}

// -e executes after standard rc loading, in the same Perl environment as a
// normal invocation. JSON::PP ships with Perl; no shell or TeX engine is run.
const REPORT: &str = r#"require JSON::PP; print "\nLATEX_LSP_CONFIGURATION ", JSON::PP->new->utf8->encode({output => $out_dir, auxiliary => $aux_dir, final_output => $out2_dir, mode => 0+$pdf_mode, sources => [@rc_files_read]}), "\n";"#;

pub(super) fn resolve(
    root: &Path,
    config: &BuildConfig,
    cancel: &AtomicBool,
) -> Result<BuildConfig, String> {
    let base = config.unresolved();
    let parent = root.parent().ok_or("Invalid root path")?;
    let mut command = Command::new(&base.executable);
    command
        .current_dir(parent)
        .args(["-dir-report-only", "-e", REPORT])
        .arg(root);
    let output = crate::compilation::build_tools::execute(
        command,
        &base.executable,
        base.timeout_seconds.min(5),
        cancel,
    )
    .map_err(|failure| {
        format!(
            "Cannot resolve latexmk configuration: {}\n{}",
            failure.message, failure.output
        )
    })?;
    let line = output
        .lines()
        .find_map(|line| line.strip_prefix("LATEX_LSP_CONFIGURATION "))
        .ok_or("latexmk did not report its effective configuration")?;
    let effective: Effective = serde_json::from_str(line)
        .map_err(|error| format!("Invalid latexmk configuration report: {error}"))?;
    let mut resolved = base.clone();
    resolved.resolution = BuildResolution::default();
    resolved.resolution.native = true;
    for path in effective.sources {
        let path = parent.join(path);
        if !resolved.resolution.sources.contains(&path) {
            resolved.resolution.sources.push(path);
        }
    }
    if resolved.resolution.sources.len() > 256 {
        return Err("Too many latexmk configuration files".into());
    }
    // Track literal do/require includes as well: Perl's do does not add an
    // entry to latexmk's process_rc_file list. Dynamic include values are resolved by
    // Perl, but should use process_rc_file when change tracking is needed.
    let mut position = 0;
    while position < resolved.resolution.sources.len() {
        let path = &resolved.resolution.sources[position];
        for include in literal_includes(path) {
            let include = parent.join(include);
            if !resolved.resolution.sources.contains(&include) {
                resolved.resolution.sources.push(include);
            }
        }
        if resolved.resolution.sources.len() > 256 {
            return Err("Too many latexmk configuration files".into());
        }
        position += 1;
    }
    let engine = match effective.mode {
        0 => None,
        1 => Some(Engine::PdfLatex),
        4 => Some(Engine::LuaLatex),
        5 => Some(Engine::XeLatex),
        _ if resolved.engine.is_some() => None,
        mode => {
            return Err(format!(
                "Unsupported latexmk $pdf_mode = {mode}; configure build.engine explicitly"
            ))
        }
    };
    if resolved.engine.is_none() {
        resolved.engine = engine;
        resolved.resolution.inferred.push("engine".into());
    }
    if resolved.output_directory.is_none() {
        resolved.output_directory = Some(if effective.output.is_empty() {
            config
                .output_directory
                .clone()
                .unwrap_or_else(|| ".".into())
        } else {
            valid_directory(effective.output)?
        });
        resolved.resolution.inferred.push("outputDirectory".into());
    }
    for (target, value, name) in [
        (
            &mut resolved.auxiliary_directory,
            effective.auxiliary,
            "auxiliaryDirectory",
        ),
        (
            &mut resolved.final_output_directory,
            effective.final_output,
            "finalOutputDirectory",
        ),
    ] {
        if target.is_none() {
            *target = if value.is_empty() {
                resolved.output_directory.clone()
            } else {
                Some(valid_directory(value)?)
            };
            resolved.resolution.inferred.push(name.into());
        }
    }
    resolved.resolution.dependencies = candidates(root);
    resolved
        .resolution
        .dependencies
        .extend(resolved.resolution.sources.clone());
    let inputs = resolved
        .resolution
        .dependencies
        .iter()
        .map(|path| (path.clone(), InputSnapshot::file(path)))
        .collect();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if cache.len() >= 128 {
        cache.clear();
    }
    cache.insert(
        key(root, &base),
        Cached {
            config: resolved.clone(),
            inputs,
        },
    );
    Ok(resolved)
}

fn valid_directory(value: String) -> Result<String, String> {
    if value.contains(['\0', '\n', '\r']) {
        Err("Invalid characters in latexmk directory".into())
    } else {
        Ok(value)
    }
}

fn literal_includes(path: &Path) -> Vec<PathBuf> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return vec![];
    };
    if metadata.len() > 1024 * 1024 {
        return vec![];
    }
    let Ok(source) = std::fs::read_to_string(path) else {
        return vec![];
    };
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let value = line
                .strip_prefix("do ")
                .or_else(|| line.strip_prefix("require "))?
                .trim_start();
            let quote = value.chars().next()?;
            if !matches!(quote, '\'' | '"') {
                return None;
            }
            let tail = &value[1..];
            let value = tail.split(quote).next()?;
            (!value.contains(['$', '@', '\\'])).then(|| PathBuf::from(value))
        })
        .collect()
}
