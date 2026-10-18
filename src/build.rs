use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use crate::{build_log, config::BuildConfig, workspace::Workspace};
use serde_json::{json, Value};
use tokio::sync::{Notify, RwLock};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::*,
    Client,
};

const MAX_OUTPUT: usize = 4 * 1024 * 1024;

pub enum BuildStarted {}
impl notification::Notification for BuildStarted {
    type Params = Value;
    const METHOD: &'static str = "latex/buildStarted";
}
pub enum BuildFinished {}
impl notification::Notification for BuildFinished {
    type Params = Value;
    const METHOD: &'static str = "latex/buildFinished";
}

#[derive(Default)]
struct Record {
    active: Option<(u64, Arc<AtomicBool>)>,
    result: Option<Value>,
}

pub struct BuildManager {
    client: Client,
    workspace: Arc<RwLock<Workspace>>,
    state: Arc<Mutex<BTreeMap<Url, Record>>>,
    sequence: AtomicU64,
    idle: Arc<Notify>,
}

impl BuildManager {
    pub fn new(client: Client, workspace: Arc<RwLock<Workspace>>) -> Self {
        Self {
            client,
            workspace,
            state: Arc::default(),
            sequence: AtomicU64::new(1),
            idle: Arc::default(),
        }
    }

    pub async fn start(&self, uri: Url) -> Result<Value> {
        let mut workspace = self.workspace.write().await;
        let root = workspace
            .root_document(&uri)
            .map_err(Error::invalid_params)?;
        workspace.reload(&root);
        workspace.load_dependencies(&root, 0);
        let project = workspace.project_documents(&root);
        for uri in &project {
            workspace.reload(uri);
        }
        let mut snapshots = BTreeMap::new();
        for uri in project {
            if let Some(doc) = workspace.documents.get(&uri) {
                let saved = uri
                    .to_file_path()
                    .ok()
                    .and_then(|path| std::fs::read_to_string(path).ok());
                if saved.as_deref() != Some(&doc.text.source) {
                    return Err(Error::invalid_params(format!(
                        "Save {} before compiling",
                        uri.path()
                    )));
                }
                snapshots.insert(uri, (doc.version, doc.text.source.clone()));
            }
        }
        let config = workspace.config.build.clone();
        let root_path = root
            .to_file_path()
            .map_err(|_| Error::invalid_params("Compilation requires a local file"))?;
        drop(workspace);
        let build_id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut state = self.state.lock().unwrap();
            let record = state.entry(root.clone()).or_default();
            if let Some((id, _)) = &record.active {
                return Ok(json!({"status":"running", "buildId":id, "rootUri":root}));
            }
            record.active = Some((build_id, cancel.clone()));
        }
        let client = self.client.clone();
        let workspace = self.workspace.clone();
        let state = self.state.clone();
        let idle = self.idle.clone();
        let root_uri = root.clone();
        tokio::spawn(async move {
            client
                .send_notification::<BuildStarted>(json!({"buildId":build_id, "rootUri":root_uri}))
                .await;
            let started = Instant::now();
            let worker_root = root_path.clone();
            let outcome =
                tokio::task::spawn_blocking(move || run(&worker_root, &config, &cancel)).await;
            let outcome = outcome
                .unwrap_or_else(|error| Outcome::error(format!("Compiler task failed: {error}")));
            let mut current = workspace.write().await;
            let stale = snapshots.iter().any(|(uri, (version, text))| {
                current
                    .documents
                    .get(uri)
                    .is_none_or(|doc| doc.version != *version || doc.text.source != *text)
                    || uri
                        .to_file_path()
                        .ok()
                        .and_then(|path| std::fs::read_to_string(path).ok())
                        .as_ref()
                        != Some(text)
            });
            let count = outcome.diagnostics.values().map(Vec::len).sum::<usize>();
            let result = json!({
                "buildId":build_id, "rootUri":root_uri, "status":outcome.status,
                "exitCode":outcome.exit_code, "message":outcome.message,
                "pdfUri":outcome.pdf.as_ref().and_then(|path| Url::from_file_path(path).ok()),
                "logUri":outcome.log.as_ref().and_then(|path| Url::from_file_path(path).ok()),
                "diagnosticsCount":count, "stale":stale, "durationMs":started.elapsed().as_millis() as u64,
                "output":outcome.output,
            });
            let mut affected = BTreeSet::new();
            if !stale && matches!(outcome.status, "success" | "error") {
                if let Some(old) = current
                    .build_diagnostics
                    .insert(root_uri.clone(), outcome.diagnostics)
                {
                    affected.extend(old.into_keys());
                }
                if let Some(new) = current.build_diagnostics.get(&root_uri) {
                    affected.extend(new.keys().cloned());
                }
                affected.insert(root_uri.clone());
            }
            let notifications = affected
                .into_iter()
                .map(|uri| {
                    let version = current.documents.get(&uri).and_then(|doc| doc.version);
                    let diagnostics = current.diagnostics(&uri);
                    (uri, diagnostics, version)
                })
                .collect::<Vec<_>>();
            drop(current);
            {
                let mut state = state.lock().unwrap();
                let record = state.entry(root_uri.clone()).or_default();
                record.active = None;
                record.result = Some(result.clone());
            }
            idle.notify_one();
            for (uri, diagnostics, version) in notifications {
                client.publish_diagnostics(uri, diagnostics, version).await;
            }
            client.send_notification::<BuildFinished>(result).await;
        });
        Ok(json!({"status":"started", "buildId":build_id, "rootUri":root}))
    }

    pub fn cancel(&self, root: Option<&Url>) -> usize {
        let state = self.state.lock().unwrap();
        let mut count = 0;
        for (uri, record) in state.iter() {
            if root.is_none_or(|root| root == uri) {
                if let Some((_, cancel)) = &record.active {
                    cancel.store(true, Ordering::Relaxed);
                    count += 1;
                }
            }
        }
        count
    }

    pub fn status(&self, root: &Url) -> Value {
        let state = self.state.lock().unwrap();
        match state.get(root) {
            Some(Record {
                active: Some((id, _)),
                ..
            }) => json!({"status":"running", "buildId":id, "rootUri":root}),
            Some(Record {
                result: Some(result),
                ..
            }) => result.clone(),
            _ => json!({"status":"idle", "rootUri":root}),
        }
    }

    pub async fn shutdown(&self) {
        self.cancel(None);
        loop {
            let finished = self.idle.notified();
            if self
                .state
                .lock()
                .unwrap()
                .values()
                .all(|record| record.active.is_none())
            {
                break;
            }
            finished.await;
        }
    }
}

pub struct Outcome {
    pub status: &'static str,
    pub exit_code: Option<i32>,
    pub message: String,
    pub pdf: Option<PathBuf>,
    pub log: Option<PathBuf>,
    pub diagnostics: BTreeMap<Url, Vec<Diagnostic>>,
    pub output: String,
}

impl Outcome {
    fn error(message: String) -> Self {
        Self {
            status: "error",
            exit_code: None,
            message,
            pdf: None,
            log: None,
            diagnostics: BTreeMap::new(),
            output: String::new(),
        }
    }
}

fn capture(mut reader: impl Read) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let keep = count.min(MAX_OUTPUT.saturating_sub(output.len()));
                output.extend_from_slice(&buffer[..keep]);
            }
        }
    }
    output
}

fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    // The compiler owns a new process group. Kill its descendants too, so TeX
    // engines spawned by latexmk cannot outlive a cancelled build.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}

pub fn run(root: &Path, config: &BuildConfig, cancel: &AtomicBool) -> Outcome {
    let mut outcome = run_inner(root, config, cancel);
    if outcome.status == "error"
        && !outcome
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
    {
        if let Ok(uri) = Url::from_file_path(root) {
            outcome
                .diagnostics
                .entry(uri)
                .or_default()
                .push(Diagnostic {
                    range: Range::default(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("latex-build".into()),
                    message: outcome.message.clone(),
                    ..Default::default()
                });
        }
    }
    outcome
}

fn run_inner(root: &Path, config: &BuildConfig, cancel: &AtomicBool) -> Outcome {
    let Some(parent) = root.parent() else {
        return Outcome::error("Invalid root path".into());
    };
    let output_dir = parent.join(&config.output_directory);
    if let Err(error) = std::fs::create_dir_all(&output_dir) {
        return Outcome::error(format!("Cannot create build directory: {error}"));
    }
    let Some(stem) = root.file_stem() else {
        return Outcome::error("Invalid root filename".into());
    };
    let log_path = output_dir.join(format!("{}.log", stem.to_string_lossy()));
    let pdf_path = output_dir.join(format!("{}.pdf", stem.to_string_lossy()));
    // A previous log must never be interpreted as the result of a failed spawn.
    if log_path.exists() {
        if let Err(error) = std::fs::remove_file(&log_path) {
            return Outcome::error(format!("Cannot replace compiler log: {error}"));
        }
    }
    let arguments = config
        .arguments()
        .into_iter()
        .map(|argument| {
            argument
                .replace("%f", &root.to_string_lossy())
                .replace("%o", &output_dir.to_string_lossy())
        })
        .collect::<Vec<_>>();
    let mut command = Command::new(&config.executable);
    command
        .args(arguments)
        .current_dir(parent)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut retries = 0;
    let mut child = loop {
        match command.spawn() {
            Ok(child) => break child,
            // A newly installed/replaced executable can briefly be ETXTBSY.
            // Retry this transient OS error only, with a bounded backoff.
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && retries < 3 => {
                std::thread::sleep(Duration::from_millis(10 << retries));
                retries += 1;
            }
            Err(error) => {
                return Outcome::error(format!("Cannot start {}: {error}", config.executable))
            }
        }
    };
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout = std::thread::spawn(move || capture(stdout));
    let stderr = std::thread::spawn(move || capture(stderr));
    let started = Instant::now();
    let mut outcome = Outcome::error("Compilation failed".into());
    loop {
        if cancel.load(Ordering::Relaxed)
            || started.elapsed() > Duration::from_secs(config.timeout_seconds)
        {
            outcome.status = if cancel.load(Ordering::Relaxed) {
                "cancelled"
            } else {
                "timeout"
            };
            outcome.message = if outcome.status == "cancelled" {
                "Compilation cancelled"
            } else {
                "Compilation timed out"
            }
            .into();
            kill_group(&mut child);
            let _ = child.wait();
            break;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                outcome.exit_code = status.code();
                if status.success() {
                    if std::fs::metadata(&pdf_path).is_ok_and(|metadata| metadata.len() > 0) {
                        outcome.status = "success";
                        outcome.message = "Compilation succeeded".into();
                        outcome.pdf = Some(pdf_path);
                    } else {
                        outcome.message = "Compiler exited successfully but produced no PDF".into();
                    }
                } else {
                    outcome.message = format!("{} exited with {}", config.executable, status);
                }
                kill_group(&mut child);
                break;
            }
            Err(error) => {
                outcome.message = format!("Cannot wait for compiler: {error}");
                kill_group(&mut child);
                let _ = child.wait();
                break;
            }
            _ => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    let output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    let log = std::fs::File::open(&log_path)
        .ok()
        .and_then(|file| {
            let mut text = String::new();
            file.take((MAX_OUTPUT * 2) as u64)
                .read_to_string(&mut text)
                .ok()?;
            Some(text)
        })
        .unwrap_or_default();
    // latexmk can run TeX several times. The final .log is authoritative;
    // earlier stdout passes may contain references that are already resolved.
    outcome.diagnostics = build_log::parse(root, if log.is_empty() { &output } else { &log });
    if outcome.status == "success"
        && outcome
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
    {
        outcome.status = "error";
        outcome.message = "Compiler reported errors; generated PDF may be incomplete".into();
        outcome.pdf = None;
    }
    if log_path.is_file() {
        outcome.log = Some(log_path);
    }
    if outcome.status == "error" {
        if let Some(error) = outcome
            .diagnostics
            .values()
            .flatten()
            .find(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
        {
            outcome.message = error.message.clone();
        }
    }
    // Notifications carry a bounded tail; pipe readers above still drain all output.
    outcome.output = output
        .chars()
        .rev()
        .take(8192)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    outcome
}
