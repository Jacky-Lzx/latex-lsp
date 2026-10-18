use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
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
    queued: Option<Url>,
    cancel_generation: u64,
    inputs: BTreeSet<Url>,
}

#[derive(Clone)]
pub struct BuildManager {
    client: Client,
    workspace: Arc<RwLock<Workspace>>,
    state: Arc<Mutex<BTreeMap<Url, Record>>>,
    sequence: Arc<AtomicU64>,
    idle: Arc<Notify>,
    closing: Arc<AtomicBool>,
    tasks: Arc<AtomicUsize>,
}

struct BuildInput {
    root: Url,
    root_path: PathBuf,
    snapshots: BTreeMap<Url, (Option<i32>, String)>,
    config: BuildConfig,
    progress_supported: bool,
    project: BTreeSet<Url>,
}

struct TaskGuard {
    tasks: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.tasks.fetch_sub(1, Ordering::SeqCst);
        self.idle.notify_one();
    }
}

impl BuildManager {
    pub fn new(client: Client, workspace: Arc<RwLock<Workspace>>) -> Self {
        Self {
            client,
            workspace,
            state: Arc::default(),
            sequence: Arc::new(AtomicU64::new(1)),
            idle: Arc::default(),
            closing: Arc::default(),
            tasks: Arc::default(),
        }
    }

    pub async fn start(&self, uri: Url) -> Result<Value> {
        self.start_request(uri, None).await
    }

    async fn start_request(
        &self,
        uri: Url,
        queued_generation: Option<(Url, u64)>,
    ) -> Result<Value> {
        let mut workspace = self.workspace.write().await;
        let root = workspace
            .root_document(&uri)
            .map_err(Error::invalid_params)?;
        workspace.refresh_project(&root);
        let project = workspace.project_documents(&root);
        let mut snapshots = BTreeMap::new();
        for uri in &project {
            if let Some(doc) = workspace.documents.get(uri) {
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
                snapshots.insert(uri.clone(), (doc.version, doc.text.source.clone()));
            }
        }
        // Dynamic dependencies may only be known after TeX writes its recorder.
        // Capture other open buffers without requiring unrelated files be saved.
        for (uri, doc) in &workspace.documents {
            if doc.version.is_some() {
                snapshots
                    .entry(uri.clone())
                    .or_insert_with(|| (doc.version, doc.text.source.clone()));
            }
        }
        let config = workspace.build_config(&root);
        let progress_supported = workspace.build_progress;
        let root_path = root
            .to_file_path()
            .map_err(|_| Error::invalid_params("Compilation requires a local file"))?;
        drop(workspace);
        let build_id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut state = self.state.lock().unwrap();
            if self.closing.load(Ordering::SeqCst) {
                return Err(Error::invalid_request());
            }
            if let Some((previous_root, generation)) = queued_generation {
                if state
                    .get(&previous_root)
                    .is_none_or(|record| record.cancel_generation != generation)
                {
                    return Ok(json!({"status":"cancelled", "rootUri":root}));
                }
            }
            let record = state.entry(root.clone()).or_default();
            if let Some((id, _)) = &record.active {
                return Ok(
                    json!({"status":"running", "buildId":id, "rootUri":root, "queued":record.queued.is_some()}),
                );
            }
            record.active = Some((build_id, cancel.clone()));
            record.inputs = project.clone();
            self.tasks.fetch_add(1, Ordering::SeqCst);
        }
        self.launch(
            BuildInput {
                root: root.clone(),
                root_path,
                snapshots,
                config,
                progress_supported,
                project,
            },
            build_id,
            cancel,
        );
        Ok(json!({"status":"started", "buildId":build_id, "rootUri":root}))
    }

    /// Coalesce repeated saves into one follow-up build using the newest files.
    pub async fn on_save(&self, uri: Url) -> Result<()> {
        let root = self
            .workspace
            .read()
            .await
            .root_document(&uri)
            .map_err(Error::invalid_params)?;
        {
            let mut state = self.state.lock().unwrap();
            if let Some(record) = state.get_mut(&root) {
                if let Some((_, cancel)) = &record.active {
                    if !cancel.load(Ordering::Relaxed) {
                        record.queued = Some(uri);
                    }
                    return Ok(());
                }
            }
        }
        self.start(uri).await.map(|_| ())
    }

    fn launch(&self, input: BuildInput, build_id: u64, cancel: Arc<AtomicBool>) {
        let BuildInput {
            root: root_uri,
            root_path,
            snapshots,
            config,
            progress_supported,
            mut project,
        } = input;
        let manager = self.clone();
        let guard = TaskGuard {
            tasks: self.tasks.clone(),
            idle: self.idle.clone(),
        };
        let client = self.client.clone();
        let workspace = self.workspace.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let executable = config.executable.clone();
            let engine = config.effective_engine();
            client
                .send_notification::<BuildStarted>(json!({"buildId":build_id, "rootUri":root_uri, "executable":executable, "engine":engine}))
                .await;
            let started = Instant::now();
            let token = NumberOrString::String(format!("latex-build-{build_id}"));
            // Some clients advertise progress but never answer create requests.
            // A bounded wait keeps compilation and shutdown responsive.
            let progress = progress_supported
                && matches!(
                    tokio::time::timeout(
                        Duration::from_secs(1),
                        client.send_request::<request::WorkDoneProgressCreate>(
                            WorkDoneProgressCreateParams {
                                token: token.clone()
                            }
                        )
                    )
                    .await,
                    Ok(Ok(()))
                );
            if progress {
                client
                    .send_notification::<notification::Progress>(ProgressParams {
                        token: token.clone(),
                        value: ProgressParamsValue::WorkDone(WorkDoneProgress::Begin(
                            WorkDoneProgressBegin {
                                title: format!(
                                    "LaTeX: {}",
                                    root_path.file_name().unwrap_or_default().to_string_lossy()
                                ),
                                cancellable: Some(true),
                                message: Some(format!("Running {}", config.executable)),
                                percentage: None,
                            },
                        )),
                    })
                    .await;
            }
            let worker_root = root_path.clone();
            let wall_started = std::time::SystemTime::now();
            let outcome =
                tokio::task::spawn_blocking(move || run(&worker_root, &config, &cancel)).await;
            let mut outcome = outcome
                .unwrap_or_else(|error| Outcome::error(format!("Compiler task failed: {error}")));
            let mut current = workspace.write().await;
            if let Some(inputs) = &outcome.inputs {
                let recorded = current.project_inputs(&root_uri, inputs);
                project.extend(recorded.iter().cloned());
                if matches!(outcome.status, "success" | "error") {
                    current
                        .recorded_inputs
                        .insert(root_uri.clone(), recorded.clone());
                    for uri in &recorded {
                        current.reload(uri);
                    }
                }
            }
            // A no-op driver may deliberately ignore an \includeonly-excluded
            // file. Prefer recorded inputs when available, retaining BibTeX
            // sources which TeX's own recorder does not normally list.
            let mut artifact_inputs = current
                .recorded_inputs
                .get(&root_uri)
                .cloned()
                .unwrap_or_else(|| project.clone());
            artifact_inputs.insert(root_uri.clone());
            artifact_inputs.extend(
                project
                    .iter()
                    .filter(|uri| uri.path().ends_with(".bib"))
                    .cloned(),
            );
            reject_outdated_pdf(
                &root_path,
                &mut outcome,
                artifact_inputs
                    .iter()
                    .filter_map(|uri| uri.to_file_path().ok()),
            );
            let stale = project.iter().any(|uri| {
                if let Some((version, text)) = snapshots.get(uri) {
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
                } else {
                    let Ok(path) = uri.to_file_path() else {
                        return true;
                    };
                    current.documents.get(uri).is_some_and(|doc| {
                        doc.version.is_some()
                            && std::fs::read_to_string(&path).ok().as_ref()
                                != Some(&doc.text.source)
                    }) || std::fs::metadata(path)
                        .ok()
                        .and_then(|metadata| metadata.modified().ok())
                        .is_none_or(|modified| modified > wall_started)
                }
            });
            let count = outcome.diagnostics.values().map(Vec::len).sum::<usize>();
            let result = json!({
                "buildId":build_id, "rootUri":root_uri, "status":outcome.status,
                "exitCode":outcome.exit_code, "message":if stale { format!("{} (sources changed; diagnostics discarded)",outcome.message) } else { outcome.message },
                "pdfUri":outcome.pdf.as_ref().and_then(|path| Url::from_file_path(path).ok()),
                "logUri":outcome.log.as_ref().and_then(|path| Url::from_file_path(path).ok()),
                "diagnosticsCount":count, "stale":stale, "durationMs":started.elapsed().as_millis() as u64,
                "dependenciesCount":project.len(),
                "executable":executable, "engine":engine,
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
            for (uri, diagnostics, version) in notifications {
                client.publish_diagnostics(uri, diagnostics, version).await;
            }
            if progress {
                client
                    .send_notification::<notification::Progress>(ProgressParams {
                        token,
                        value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(
                            WorkDoneProgressEnd {
                                message: result["message"].as_str().map(str::to_owned),
                            },
                        )),
                    })
                    .await;
            }
            client.send_notification::<BuildFinished>(result).await;
            let queued = state.lock().unwrap().get_mut(&root_uri).and_then(|record| {
                // A new manual build may already own the record. Its saves
                // belong to that build and must remain in its queue.
                if record.active.is_some() {
                    return None;
                }
                record
                    .queued
                    .take()
                    .map(|uri| (uri, record.cancel_generation))
            });
            if let Some((uri, generation)) = queued {
                if !manager.closing.load(Ordering::SeqCst)
                    && workspace.read().await.config.build.on_save
                {
                    if let Err(error) = manager
                        .start_request(uri, Some((root_uri, generation)))
                        .await
                    {
                        client
                            .log_message(
                                MessageType::WARNING,
                                format!("Queued compilation skipped: {}", error.message),
                            )
                            .await;
                    }
                }
            }
        });
    }

    pub fn cancel(&self, root: Option<&Url>) -> usize {
        let mut state = self.state.lock().unwrap();
        let mut count = 0;
        for (uri, record) in state.iter_mut() {
            if root.is_none_or(|root| root == uri) {
                record.queued = None;
                record.cancel_generation = record.cancel_generation.wrapping_add(1);
                if let Some((_, cancel)) = &record.active {
                    cancel.store(true, Ordering::Relaxed);
                    count += 1;
                }
            }
        }
        count
    }

    pub fn cancel_progress(&self, token: &ProgressToken) {
        let NumberOrString::String(token) = token else {
            return;
        };
        let Some(id) = token
            .strip_prefix("latex-build-")
            .and_then(|id| id.parse::<u64>().ok())
        else {
            return;
        };
        for record in self.state.lock().unwrap().values_mut() {
            if let Some((active, cancel)) = &record.active {
                if *active == id {
                    record.queued = None;
                    record.cancel_generation = record.cancel_generation.wrapping_add(1);
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn status(&self, root: &Url) -> Value {
        let state = self.state.lock().unwrap();
        match state.get(root) {
            Some(Record {
                active: Some((id, _)),
                queued,
                ..
            }) => {
                json!({"status":"running", "buildId":id, "rootUri":root, "queued":queued.is_some()})
            }
            Some(Record {
                result: Some(result),
                ..
            }) => result.clone(),
            _ => json!({"status":"idle", "rootUri":root}),
        }
    }

    pub async fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.cancel(None);
        loop {
            let finished = self.idle.notified();
            if self.tasks.load(Ordering::SeqCst) == 0 {
                break;
            }
            finished.await;
        }
    }

    pub async fn files_renamed(&self, renames: &crate::file_operations::Renames) {
        let mut builds = BTreeSet::new();
        {
            let mut state = self.state.lock().unwrap();
            for (root, record) in state.iter_mut() {
                if renames.affects(root) || record.inputs.iter().any(|uri| renames.affects(uri)) {
                    record.queued = None;
                    record.cancel_generation = record.cancel_generation.wrapping_add(1);
                    if let Some((id, cancel)) = &record.active {
                        cancel.store(true, Ordering::Relaxed);
                        builds.insert(*id);
                    }
                }
            }
        }
        loop {
            let finished = self.idle.notified();
            if !self.state.lock().unwrap().values().any(|record| {
                record
                    .active
                    .as_ref()
                    .is_some_and(|(id, _)| builds.contains(id))
            }) {
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
    pub inputs: Option<BTreeSet<Url>>,
    pub pdf_updated: bool,
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
            inputs: None,
            pdf_updated: false,
        }
    }
}

pub(crate) fn capture(mut reader: impl Read) -> Vec<u8> {
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

pub(crate) fn kill_group(child: &mut Child) {
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
    reject_outdated_pdf(root, &mut outcome, std::iter::once(root.to_path_buf()));
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

fn reject_outdated_pdf(root: &Path, outcome: &mut Outcome, sources: impl Iterator<Item = PathBuf>) {
    if outcome.status != "success" || outcome.pdf_updated {
        return;
    }
    let Some(pdf) = outcome
        .pdf
        .as_ref()
        .and_then(|pdf| fingerprint(pdf))
        .and_then(|(_, modified)| modified)
    else {
        return;
    };
    let outdated = sources.into_iter().find(|source| {
        std::fs::metadata(source)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .is_some_and(|modified| modified > pdf)
    });
    if let Some(source) = outdated {
        outcome.status = "error";
        outcome.pdf = None;
        outcome.message = format!(
            "Compiler left an outdated PDF unchanged; rebuild after changes to {}",
            source.display()
        );
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
    let pdf_before = fingerprint(&pdf_path);
    let recorder_path = output_dir.join(format!("{}.fls", stem.to_string_lossy()));
    let recorder_before = fingerprint(&recorder_path);
    let bibliography_before = bibliography_logs(&output_dir);
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
    let log = read_log(&log_path).unwrap_or_default();
    outcome.pdf_updated = outcome
        .pdf
        .as_ref()
        .is_some_and(|pdf| fingerprint(pdf) != pdf_before);
    let recorder_after = fingerprint(&recorder_path);
    if recorder_after.is_some() && recorder_after != recorder_before {
        outcome.inputs = read_log(&recorder_path).map(|log| crate::recorder::inputs(root, &log));
    }
    // latexmk can run TeX several times. The final .log is authoritative;
    // earlier stdout passes may contain references that are already resolved.
    outcome.diagnostics = build_log::parse(root, if log.is_empty() { &output } else { &log });
    for (path, fingerprint) in bibliography_logs(&output_dir) {
        // An old .blg left by a previous failed build is not this build's result.
        if bibliography_before.get(&path) == Some(&fingerprint) {
            continue;
        }
        if let Some(log) = read_log(&path) {
            for (uri, diagnostics) in build_log::parse_bibliography(root, &log) {
                outcome
                    .diagnostics
                    .entry(uri)
                    .or_default()
                    .extend(diagnostics);
            }
        }
    }
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

fn read_log(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut text = Vec::new();
    file.take((MAX_OUTPUT * 2) as u64)
        .read_to_end(&mut text)
        .ok()?;
    Some(String::from_utf8_lossy(&text).into_owned())
}

fn fingerprint(path: &Path) -> Option<(u64, Option<std::time::SystemTime>)> {
    let metadata = std::fs::metadata(path).ok()?;
    metadata
        .is_file()
        .then_some((metadata.len(), metadata.modified().ok()))
}

fn bibliography_logs(directory: &Path) -> BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)> {
    std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()? != "blg" {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then_some((path, (metadata.len(), metadata.modified().ok())))
        })
        .take(64)
        .collect()
}
