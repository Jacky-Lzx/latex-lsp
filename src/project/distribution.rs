//! Read TeX filename databases without importing distribution code into projects.
use crate::{config::DistributionConfig, parser::syntax};
use lsp_types::Url;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default)]
pub struct Distribution {
    pub(crate) files: BTreeMap<String, Url>,
}

impl Distribution {
    pub fn discover(config: &DistributionConfig) -> Self {
        Self::discover_cancelled(config, || false)
    }

    pub(crate) fn discover_cancelled(
        config: &DistributionConfig,
        cancelled: impl Fn() -> bool,
    ) -> Self {
        if !config.enabled {
            return Self::default();
        }
        let mut roots = config.roots.clone();
        if let Some(paths) = texmf_paths(&config.executable, &cancelled) {
            roots.extend(std::env::split_paths(paths.trim()));
        }
        if cancelled() {
            return Self::default();
        }
        Self::from_roots(&roots)
    }

    pub fn from_roots(roots: &[PathBuf]) -> Self {
        let mut result = Self::default();
        for root in roots.iter().take(64) {
            let root = Path::new(root.to_str().unwrap_or("").trim_start_matches('!'));
            let Ok(file) = std::fs::File::open(root.join("ls-R")) else {
                continue;
            };
            let mut data = String::new();
            if file
                .take(32 * 1024 * 1024 + 1)
                .read_to_string(&mut data)
                .is_err()
                || data.len() > 32 * 1024 * 1024
            {
                continue;
            }
            result.add_database(root, &data);
        }
        result
    }

    fn add_database(&mut self, root: &Path, data: &str) {
        let mut directory = PathBuf::new();
        for line in data.lines() {
            if self.files.len() >= 100_000 {
                break;
            }
            if let Some(value) = line
                .strip_prefix("./")
                .and_then(|value| value.strip_suffix(':'))
            {
                directory = PathBuf::from(value);
                continue;
            }
            if directory.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            }) {
                continue;
            }
            let extension = Path::new(line).extension().and_then(|ext| ext.to_str());
            if !directory.starts_with("tex") && !directory.starts_with("bibtex")
                || !matches!(
                    extension,
                    Some("sty" | "cls" | "bst" | "bbx" | "cbx" | "lbx" | "dbx")
                )
                || line
                    .chars()
                    .any(|ch| ch.is_whitespace() || "/\\{}%#$&^~\"".contains(ch))
            {
                continue;
            }
            let path = root.join(&directory).join(line);
            if let Ok(uri) = Url::from_file_path(path) {
                self.files.entry(line.into()).or_insert(uri);
            }
        }
    }

    pub(crate) fn resolve(&self, command: &str, value: &str) -> Option<Url> {
        let extension = syntax::package_class_extension(command)
            .or_else(|| (command == "bibliographystyle").then_some("bst"))?;
        let name = syntax::loader_name(value);
        if name.contains(['/', '\\', '{', '}', '%']) || name.is_empty() {
            return None;
        }
        let file = if extension == "bst" && name.ends_with(".bst") {
            name
        } else {
            format!("{name}.{extension}")
        };
        self.files
            .get(&file)
            .filter(|uri| uri.to_file_path().is_ok_and(|path| path.is_file()))
            .cloned()
    }
}

fn texmf_paths(executable: &str, cancelled: &impl Fn() -> bool) -> Option<String> {
    let mut command = Command::new(executable);
    command
        .arg("--expand-path=$TEXMFDBS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().ok()?;
    let output = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut data = Vec::new();
        output.take(64 * 1024).read_to_end(&mut data).ok()?;
        String::from_utf8(data).ok()
    });
    let start = Instant::now();
    let success = loop {
        if cancelled() {
            break false;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Err(_) => break false,
            _ if start.elapsed() > Duration::from_secs(2) => break false,
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    crate::compilation::build::kill_group(&mut child);
    let _ = child.wait();
    let output = reader.join().ok().flatten();
    success.then_some(output).flatten()
}
