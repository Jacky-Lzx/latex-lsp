use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use crate::{
    build_log,
    config::BuildConfig,
    file_checks::{fingerprint, ArtifactFingerprint, InputSnapshot, Snapshot},
    workspace::Workspace,
};
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
    queued: Option<QueuedBuild>,
    rebuild: bool,
    cancel_generation: u64,
    inputs: BTreeMap<Url, InputSnapshot>,
    checks: Arc<crate::file_checks::Snapshot>,
    artifact: Option<PathBuf>,
}

struct QueuedBuild {
    uri: Url,
    rebuild: bool,
}

#[derive(Clone)]
pub struct BuildManager {
    client: Client,
    workspace: Arc<RwLock<Workspace>>,
    state: Arc<Mutex<BTreeMap<Url, Record>>>,
    sequence: Arc<AtomicU64>,
    idle: Arc<Notify>,
    closing: Arc<AtomicBool>,
    disconnected: Arc<AtomicBool>,
    tasks: Arc<AtomicUsize>,
}

struct BuildInput {
    root: Url,
    root_path: PathBuf,
    snapshots: BTreeMap<Url, (Option<i32>, String)>,
    file_snapshots: BTreeMap<Url, InputSnapshot>,
    config: BuildConfig,
    progress_supported: bool,
    project: BTreeSet<Url>,
    static_code_inputs: BTreeSet<Url>,
    checks: Arc<crate::file_checks::Snapshot>,
    rebuild: bool,
}

struct TaskGuard {
    tasks: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.tasks.fetch_sub(1, Ordering::SeqCst);
        self.idle.notify_waiters();
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
            disconnected: Arc::default(),
            tasks: Arc::default(),
        }
    }

    pub async fn start(&self, uri: Url) -> Result<Value> {
        self.start_request(uri, None, false, None).await
    }

    pub async fn rebuild(&self, uri: Url) -> Result<Value> {
        self.start_request(uri, None, true, None).await
    }

    pub(crate) async fn rebuild_guarded(
        &self,
        uri: Url,
        guard: crate::editing::RebuildGuard,
    ) -> Result<Value> {
        self.start_request(uri, None, true, Some(guard)).await
    }

    async fn while_open<F: Future>(&self, action: F) -> Option<F::Output> {
        if self.disconnected.load(Ordering::SeqCst) {
            return None;
        }
        let closed = async {
            loop {
                let changed = self.idle.notified();
                if self.disconnected.load(Ordering::SeqCst) {
                    return;
                }
                changed.await;
            }
        };
        // EOF can stop the transport from consuming outgoing notifications.
        // Cancellation must still finish when that channel is backpressured.
        tokio::select! {
            output = action => Some(output),
            _ = closed => None,
        }
    }

    async fn start_request(
        &self,
        uri: Url,
        queued_generation: Option<(Url, u64)>,
        rebuild: bool,
        guard: Option<crate::editing::RebuildGuard>,
    ) -> Result<Value> {
        let mut workspace = self.workspace.write().await;
        let root = workspace
            .root_document(&uri)
            .map_err(Error::invalid_params)?;
        if let Some((owner, _)) = &queued_generation {
            if owner != &root {
                return Ok(json!({"status":"cancelled", "rootUri":owner}));
            }
        }
        if guard.as_ref().is_some_and(|guard| {
            root != uri || !crate::editing::valid_rebuild_guard(&workspace, &root, guard)
        }) {
            return Err(Error::invalid_params(
                "Rebuild action is outdated; request code actions again",
            ));
        }
        workspace.refresh_project(&root);
        let static_code_inputs = crate::code_inputs::inputs(&workspace, &root);
        let mut project = workspace.project_files(&root);
        // Known opened paths remain build dependencies even when their current
        // targets are outside the source index. Capture the actual target now,
        // rather than the one observed by the previous compiler invocation.
        project.extend(
            workspace
                .project_bindings(&root)
                .filter_map(|(path, _)| path.canonicalize().ok())
                .filter(|path| path.is_file())
                .filter_map(|path| Url::from_file_path(path).ok()),
        );
        let mut snapshots = BTreeMap::new();
        let mut file_snapshots = BTreeMap::new();
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
            } else if let Ok(path) = uri.to_file_path() {
                file_snapshots.insert(uri.clone(), InputSnapshot::file(&path));
            }
        }
        // Dynamic dependencies may only be known after TeX writes its recorder.
        // Capture other open buffers without requiring unrelated files be saved.
        for (uri, doc) in &workspace.documents {
            if doc.version.is_some() {
                snapshots
                    .entry(uri.clone())
                    .or_insert_with(|| (doc.version, doc.text.source.clone()));
                // A dynamic input may have unsaved editor text. The compiler
                // reads disk; retain both versions without requiring unrelated
                // open buffers to be saved.
                if !project.contains(uri) {
                    if let Ok(path) = uri.to_file_path() {
                        file_snapshots.insert(uri.clone(), InputSnapshot::file(&path));
                    }
                }
            }
        }
        let config = workspace.build_config(&root);
        if rebuild {
            config.rebuild_arguments().map_err(Error::invalid_params)?;
        }
        let checks = Arc::new(crate::file_checks::Snapshot::capture(
            &workspace, &root, &config,
        ));
        let progress_supported = workspace.build_progress;
        let root_path = root
            .to_file_path()
            .map_err(|_| Error::invalid_params("Compilation requires a local file"))?;
        drop(workspace);
        let build_id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let artifact = config.artifact_path(&root_path, "pdf").map(|path| {
            let path = crate::workspace::normalize_path(&path);
            path.parent()
                .and_then(|parent| parent.canonicalize().ok())
                .zip(path.file_name())
                .map_or_else(|| path.clone(), |(parent, name)| parent.join(name))
        });
        {
            let mut state = self.state.lock().unwrap();
            if self.closing.load(Ordering::SeqCst) {
                return Err(Error::invalid_request());
            }
            if let Some((previous_root, generation)) = &queued_generation {
                if state
                    .get(previous_root)
                    .is_none_or(|record| record.cancel_generation != *generation)
                {
                    return Ok(json!({"status":"cancelled", "rootUri":root}));
                }
            }
            if let Some((owner, _)) = state.iter().find(|(owner, record)| {
                *owner != &root
                    && record.active.is_some()
                    && artifact.is_some()
                    && record.artifact == artifact
            }) {
                return Err(Error::invalid_params(format!(
                    "Another active build writes the same output file: {}; wait for {} to finish or choose a different jobName or outputDirectory",
                    artifact.as_ref().unwrap().display(), owner
                )));
            }
            let record = state.entry(root.clone()).or_default();
            if let Some((id, _)) = &record.active {
                // A fresh manual request asks for a follow-up even when the
                // current build is forced. An older queued request is already
                // satisfied by a newer forced build which captured its inputs.
                if rebuild && (queued_generation.is_none() || !record.rebuild) {
                    record.queued = Some(QueuedBuild { uri, rebuild: true });
                }
                return Ok(
                    json!({"status":"running", "buildId":id, "rootUri":root, "queued":record.queued.is_some(), "rebuild":record.rebuild, "queuedRebuild":record.queued.as_ref().is_some_and(|queued| queued.rebuild)}),
                );
            }
            if rebuild {
                record.queued = None;
                if queued_generation.is_none() {
                    // Consume older pending force requests before releasing
                    // the wire-order barrier; their callbacks must not revive.
                    record.cancel_generation = record.cancel_generation.wrapping_add(1);
                }
            }
            record.active = Some((build_id, cancel.clone()));
            record.rebuild = rebuild;
            record.inputs = project
                .iter()
                .map(|uri| {
                    let snapshot = snapshots.get(uri).map_or_else(
                        || {
                            file_snapshots
                                .get(uri)
                                .cloned()
                                .unwrap_or(InputSnapshot::File(None))
                        },
                        |(_, text)| InputSnapshot::text(text),
                    );
                    (uri.clone(), snapshot)
                })
                .collect();
            record.checks = checks.clone();
            record.artifact = artifact;
            self.tasks.fetch_add(1, Ordering::SeqCst);
        }
        self.launch(
            BuildInput {
                root: root.clone(),
                root_path,
                snapshots,
                file_snapshots,
                config,
                progress_supported,
                project,
                static_code_inputs,
                checks,
                rebuild,
            },
            build_id,
            cancel,
        );
        Ok(json!({"status":"started", "buildId":build_id, "rootUri":root, "rebuild":rebuild}))
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
                        let rebuild = record.queued.as_ref().is_some_and(|queued| queued.rebuild);
                        record.queued = Some(QueuedBuild { uri, rebuild });
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
            file_snapshots,
            config,
            progress_supported,
            mut project,
            static_code_inputs,
            checks,
            rebuild,
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
            let engine = if rebuild && config.rebuild_arguments.is_some() {
                None
            } else {
                config.effective_engine()
            };
            let job_name = config
                .job_name(&root_path)
                .map(|name| name.to_string_lossy().into_owned());
            let _ = manager
                .while_open(client.send_notification::<BuildStarted>(json!({
                    "buildId": build_id,
                    "rootUri": root_uri,
                    "executable": executable,
                    "engine": engine,
                    "jobName": job_name,
                    "rebuild": rebuild,
                })))
                .await;
            let started = Instant::now();
            let token = NumberOrString::String(format!("latex-build-{build_id}"));
            // Some clients advertise progress but never answer create requests.
            // A bounded wait keeps compilation and shutdown responsive.
            let progress = progress_supported
                && matches!(
                    manager
                        .while_open(tokio::time::timeout(
                            Duration::from_secs(1),
                            client.send_request::<request::WorkDoneProgressCreate>(
                                WorkDoneProgressCreateParams {
                                    token: token.clone()
                                }
                            )
                        ))
                        .await,
                    Some(Ok(Ok(())))
                );
            if progress {
                let _ = manager
                    .while_open(client.send_notification::<notification::Progress>(
                        ProgressParams {
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
                        },
                    ))
                    .await;
            }
            let worker_root = root_path.clone();
            let previous_pdf = workspace.read().await.pdf_checks.get(&root_uri).cloned();
            let worker_inputs: Vec<_> = project
                .iter()
                .filter_map(|uri| uri.to_file_path().ok())
                .collect();
            let wall_started = std::time::SystemTime::now();
            let outcome = tokio::task::spawn_blocking(move || {
                run_with_mode(
                    &worker_root,
                    &config,
                    &cancel,
                    rebuild,
                    previous_pdf.as_deref(),
                    worker_inputs.into_iter(),
                )
            })
            .await;
            let mut outcome = outcome
                .unwrap_or_else(|error| Outcome::error(format!("Compiler task failed: {error}")));
            let mut current = workspace.write().await;
            let mut checks = checks;
            if let Some(inputs) = &outcome.inputs {
                let recorded =
                    current.project_inputs_with_binary(&root_uri, inputs, &outcome.binary_inputs);
                project.extend(recorded.iter().cloned());
                if matches!(outcome.status, "success" | "error") {
                    current
                        .recorded_inputs
                        .insert(root_uri.clone(), recorded.clone());
                    current.recorded_binary_inputs.insert(
                        root_uri.clone(),
                        outcome
                            .binary_inputs
                            .intersection(&recorded)
                            .cloned()
                            .collect(),
                    );
                    // Keep observations of known opened paths even if a new
                    // external target is outside the source indexing scope.
                    // Dropping that path would erase an in-flight retarget.
                    let bindings: BTreeMap<_, _> = outcome
                        .bindings
                        .iter()
                        .filter(|(path, target)| {
                            Url::from_file_path(target).is_ok_and(|uri| recorded.contains(&uri))
                                || current
                                    .recorded_bindings
                                    .get(&root_uri)
                                    .is_some_and(|previous| previous.contains_key(*path))
                        })
                        .map(|(path, target)| (path.clone(), target.clone()))
                        .chain(outcome.unresolved_bindings.iter().filter_map(|path| {
                            current
                                .recorded_bindings
                                .get(&root_uri)?
                                .get(path)
                                .map(|target| (path.clone(), target.clone()))
                        }))
                        .collect();
                    current.recorded_bindings.insert(root_uri.clone(), bindings);
                    for uri in &recorded {
                        current.reload_recorded(uri, outcome.binary_inputs.contains(uri));
                    }
                }
            }
            if let Some(inputs) = &outcome.bibliography_inputs {
                // Database/style roles come from the bibliography tool, not a
                // canonical suffix which could look like an image or dataset.
                let recorded =
                    current.project_inputs_with_binary(&root_uri, inputs, &BTreeSet::new());
                project.extend(recorded.iter().cloned());
                if matches!(outcome.status, "success" | "error") {
                    current
                        .bibliography_inputs
                        .insert(root_uri.clone(), recorded.clone());
                    let bindings =
                        outcome
                            .bibliography_bindings
                            .iter()
                            .filter(|(path, target)| {
                                Url::from_file_path(target).is_ok_and(|uri| recorded.contains(&uri))
                                    || current
                                        .bibliography_bindings
                                        .get(&root_uri)
                                        .is_some_and(|previous| previous.contains_key(*path))
                            })
                            .map(|(path, target)| (path.clone(), target.clone()))
                            .chain(outcome.bibliography_unresolved_bindings.iter().filter_map(
                                |path| {
                                    current
                                        .bibliography_bindings
                                        .get(&root_uri)?
                                        .get(path)
                                        .map(|target| (path.clone(), target.clone()))
                                },
                            ))
                            .collect();
                    current
                        .bibliography_bindings
                        .insert(root_uri.clone(), bindings);
                    for uri in &recorded {
                        current.reload_bibliography(uri);
                    }
                }
            }
            if matches!(outcome.status, "success" | "error") {
                let empty = BTreeMap::new();
                checks = Arc::new(
                    checks.with_bindings([
                        (
                            current.recorded_bindings.get(&root_uri).unwrap_or(&empty),
                            if outcome.inputs.is_some() {
                                crate::file_checks::BindingStatus::Updated
                            } else if outcome.pdf_updated {
                                crate::file_checks::BindingStatus::Reused
                            } else {
                                crate::file_checks::BindingStatus::Unchanged
                            },
                        ),
                        (
                            current
                                .bibliography_bindings
                                .get(&root_uri)
                                .unwrap_or(&empty),
                            if outcome.bibliography_inputs.is_some() {
                                crate::file_checks::BindingStatus::Updated
                            } else {
                                crate::file_checks::BindingStatus::Reused
                            },
                        ),
                    ]),
                );
            }
            if outcome.bibliography_inputs.is_none() {
                if let Some(previous) =
                    current
                        .bibliography_checks
                        .get(&root_uri)
                        .filter(|previous| {
                            outcome
                                .pdf
                                .as_ref()
                                .is_some_and(|pdf| previous.matches_pdf(pdf))
                        })
                {
                    checks = Arc::new(checks.with_reused_inputs(previous));
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
            if let Some(inputs) = current.bibliography_inputs.get(&root_uri) {
                artifact_inputs.extend(inputs.iter().cloned());
            }
            artifact_inputs.extend(current.bibliography_files(&root_uri));
            // Recorder classification cannot enumerate arbitrary code suffixes.
            // Retain static data inputs in the PDF's content history too.
            // An in-flight source edit must not erase the original data scope.
            artifact_inputs.extend(static_code_inputs);
            artifact_inputs.extend(crate::code_inputs::inputs(&current, &root_uri));
            artifact_inputs.extend(
                current
                    .project_bindings(&root_uri)
                    .filter_map(|(_, target)| Url::from_file_path(target).ok()),
            );
            reject_outdated_pdf(
                &root_path,
                &mut outcome,
                artifact_inputs
                    .iter()
                    .filter_map(|uri| uri.to_file_path().ok()),
                current.pdf_checks.get(&root_uri).map(Arc::as_ref),
            );
            if outcome.status == "success" && !outcome.pdf_updated {
                if let Some(reason) = outcome.pdf.as_ref().and_then(|pdf| {
                    current
                        .pdf_checks
                        .get(&root_uri)
                        .filter(|previous| previous.matches_pdf(pdf))
                        .and_then(|previous| {
                            previous
                                .changed_path()
                                .map(|path| format!("file condition changed ({})", path.display()))
                                .or_else(|| {
                                    previous
                                        .changed_input()
                                        .map(|uri| format!("input content changed ({uri})"))
                                })
                                .or_else(|| {
                                    previous.pdf_changed().then(|| {
                                        format!("build artifacts changed ({})", pdf.display())
                                    })
                                })
                        })
                }) {
                    outcome.status = "error";
                    outcome.message = format!("Compiler returned an outdated PDF: {reason}");
                    outcome.pdf = None;
                    outcome.pages = None;
                    outcome
                        .diagnostics
                        .entry(root_uri.clone())
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
            let stale =
                checks.changed_path().is_some()
                    || project.iter().any(|uri| {
                        if let Some((version, text)) = snapshots.get(uri) {
                            current.documents.get(uri).is_none_or(|doc| {
                                doc.version != *version || doc.text.source != *text
                            }) || uri
                                .to_file_path()
                                .ok()
                                .and_then(|path| std::fs::read_to_string(path).ok())
                                .as_ref()
                                != Some(text)
                                || file_snapshots
                                    .get(uri)
                                    .is_some_and(|before| before.changed(uri))
                        } else if let Some(before) = file_snapshots.get(uri) {
                            before.changed(uri)
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
            let diagnostic_sources = artifact_inputs
                .intersection(&outcome.diagnostic_sources)
                .cloned()
                .collect();
            if !stale {
                // System package implementations are not user declarations.
                // Use the confirmed project inputs, including local .sty/.cls
                // and indexed external sources, for database source lookup.
                if !outcome.diagnostics_truncated {
                    build_log::locate_missing_bibliography_resources(
                        &diagnostic_sources,
                        &mut outcome.diagnostics,
                    );
                }
                crate::anchors::resolve_diagnostics(
                    &current,
                    &diagnostic_sources,
                    &mut outcome.diagnostics,
                );
                crate::label_diagnostics::resolve_diagnostics(
                    &current,
                    &diagnostic_sources,
                    &mut outcome.diagnostics,
                );
            }
            let count = outcome.diagnostics.values().map(Vec::len).sum::<usize>();
            let errors = outcome
                .diagnostics
                .values()
                .flatten()
                .filter(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
                .count();
            let warnings = outcome
                .diagnostics
                .values()
                .flatten()
                .filter(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::WARNING))
                .count();
            let pdf_size = outcome
                .pdf
                .as_ref()
                .and_then(|pdf| std::fs::metadata(pdf).ok())
                .map(|metadata| metadata.len());
            let result = json!({
                "buildId":build_id, "rootUri":root_uri, "status":outcome.status,
                "exitCode":outcome.exit_code, "message":if stale { format!("{} (sources changed; diagnostics discarded)",outcome.message) } else { outcome.message },
                "pdfUri":outcome.pdf.as_ref().and_then(|path| Url::from_file_path(path).ok()),
                "logUri":outcome.log.as_ref().and_then(|path| Url::from_file_path(path).ok()),
                "diagnosticsCount":count, "stale":stale, "durationMs":started.elapsed().as_millis() as u64,
                "errorsCount":errors, "warningsCount":warnings,
                "diagnosticsTruncated":outcome.diagnostics_truncated,
                "analysis":if stale { None } else { outcome.analysis.as_ref() },
                "pages":outcome.pages, "pdfSize":pdf_size,
                "dependenciesCount":project.len(),
                "executable":executable, "engine":engine,
                "jobName":job_name,
                "rebuild":rebuild,
                "output":outcome.output,
            });
            // Preserve the inputs which produced the PDF even when this run
            // became stale; a later no-op must not adopt changed source contents.
            let inputs: BTreeMap<Url, InputSnapshot> =
                if matches!(outcome.status, "success" | "error") {
                    project
                        .union(&artifact_inputs)
                        .map(|uri| {
                            let snapshot = if let Some(before) = file_snapshots.get(uri) {
                                before.clone()
                            } else if let Some((_, text)) = snapshots.get(uri) {
                                InputSnapshot::text(text)
                            } else if let Some(doc) =
                                current.documents.get(uri).filter(|_| project.contains(uri))
                            {
                                InputSnapshot::text(&doc.text.source)
                            } else {
                                uri.to_file_path()
                                    .map_or(InputSnapshot::File(None), |path| {
                                        InputSnapshot::file(&path)
                                    })
                            };
                            (uri.clone(), snapshot)
                        })
                        .collect()
                } else {
                    BTreeMap::new()
                };
            if outcome.bibliography_inputs.is_some()
                && matches!(outcome.status, "success" | "error")
            {
                let mut bibliography = current
                    .bibliography_inputs
                    .get(&root_uri)
                    .cloned()
                    .unwrap_or_default();
                bibliography.extend(
                    current
                        .bibliography_bindings
                        .get(&root_uri)
                        .into_iter()
                        .flat_map(|bindings| bindings.values())
                        .filter_map(|path| Url::from_file_path(path).ok()),
                );
                let contents = bibliography
                    .into_iter()
                    .map(|uri| {
                        let before = inputs.get(&uri).cloned().unwrap_or_else(|| {
                            uri.to_file_path()
                                .map_or(InputSnapshot::File(None), |path| {
                                    InputSnapshot::file(&path)
                                })
                        });
                        (uri, before)
                    })
                    .collect();
                crate::file_checks::Snapshot::remember(
                    &mut current.bibliography_checks,
                    root_uri.clone(),
                    Arc::new(checks.with_inputs(contents)),
                );
            }
            let mut affected = BTreeSet::new();
            // Even discarded diagnostics can accompany a newly written PDF.
            // Retain the inputs and contents of the PDF actually produced.
            // A successful no-op must not adopt a different recorder scope.
            if outcome.status == "success"
                && (!stale || outcome.pdf_updated)
                && outcome.pdf.as_ref().is_some_and(|pdf| {
                    outcome.pdf_updated
                        || current
                            .pdf_checks
                            .get(&root_uri)
                            .is_none_or(|previous| !previous.matches_pdf(pdf))
                })
            {
                crate::file_checks::Snapshot::remember(
                    &mut current.pdf_checks,
                    root_uri.clone(),
                    Arc::new(
                        checks
                            .with_inputs(
                                inputs
                                    .iter()
                                    .filter(|(uri, _)| artifact_inputs.contains(*uri))
                                    .map(|(uri, snapshot)| (uri.clone(), snapshot.clone()))
                                    .collect(),
                            )
                            .with_artifact_contents(),
                    ),
                );
            }
            if stale {
                if let Some(old) = current.build_diagnostics.remove(&root_uri) {
                    affected.extend(old.into_keys());
                }
                current.build_diagnostic_sources.remove(&root_uri);
                current.build_checks.remove(&root_uri);
                affected.insert(root_uri.clone());
            } else if matches!(outcome.status, "success" | "error") {
                crate::file_checks::Snapshot::remember(
                    &mut current.build_checks,
                    root_uri.clone(),
                    Arc::new(checks.with_inputs(inputs)),
                );
                current
                    .build_diagnostic_sources
                    .insert(root_uri.clone(), diagnostic_sources);
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
            affected.extend(
                current
                    .documents
                    .iter()
                    .filter_map(|(uri, doc)| doc.version.is_some().then_some(uri.clone())),
            );
            let delivery = current.diagnostic_delivery.clone();
            let notifications = if delivery.is_pull() {
                vec![]
            } else {
                current.diagnostic_snapshots(affected)
            };
            drop(current);
            {
                let mut state = state.lock().unwrap();
                let record = state.entry(root_uri.clone()).or_default();
                record.active = None;
                record.result = Some(result.clone());
            }
            let _ = manager
                .while_open(delivery.publish(&client, notifications))
                .await;
            if progress {
                let _ = manager
                    .while_open(client.send_notification::<notification::Progress>(
                        ProgressParams {
                            token,
                            value: ProgressParamsValue::WorkDone(WorkDoneProgress::End(
                                WorkDoneProgressEnd {
                                    message: result["message"].as_str().map(str::to_owned),
                                },
                            )),
                        },
                    ))
                    .await;
            }
            let _ = manager
                .while_open(client.send_notification::<BuildFinished>(result))
                .await;
            let queued = state.lock().unwrap().get_mut(&root_uri).and_then(|record| {
                // A new manual build may already own the record. Its saves
                // belong to that build and must remain in its queue.
                if record.active.is_some() {
                    return None;
                }
                record
                    .queued
                    .take()
                    .map(|queued| (queued, record.cancel_generation))
            });
            if let Some((queued, generation)) = queued {
                if !manager.closing.load(Ordering::SeqCst)
                    && (queued.rebuild || workspace.read().await.config.build.on_save)
                {
                    if let Err(error) = manager
                        .start_request(
                            queued.uri,
                            Some((root_uri, generation)),
                            queued.rebuild,
                            None,
                        )
                        .await
                    {
                        let _ = manager
                            .while_open(client.log_message(
                                MessageType::WARNING,
                                format!("Queued compilation skipped: {}", error.message),
                            ))
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
                rebuild,
                ..
            }) => {
                json!({"status":"running", "buildId":id, "rootUri":root, "queued":queued.is_some(), "rebuild":rebuild, "queuedRebuild":queued.as_ref().is_some_and(|queued| queued.rebuild)})
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
        self.idle.notify_waiters();
        self.cancel(None);
        loop {
            let finished = self.idle.notified();
            if self.tasks.load(Ordering::SeqCst) == 0 {
                break;
            }
            finished.await;
        }
    }

    pub fn disconnect(&self) {
        self.disconnected.store(true, Ordering::SeqCst);
        self.idle.notify_waiters();
    }

    pub async fn files_renamed(&self, renames: &crate::file_operations::Renames) {
        self.cancel_changed_inputs(|uri| renames.affects(uri), false)
            .await;
    }

    pub async fn files_changed(&self, changes: &crate::file_operations::FileChanges) {
        self.cancel_changed_inputs(|uri| changes.affects(uri), false)
            .await;
    }

    pub async fn watched_files_changed(&self, changes: &crate::file_operations::FileChanges) {
        self.cancel_changed_inputs(|uri| changes.affects(uri), true)
            .await;
    }

    async fn cancel_changed_inputs(&self, affects: impl Fn(&Url) -> bool, verify_inputs: bool) {
        let mut builds = BTreeSet::new();
        {
            let mut state = self.state.lock().unwrap();
            for (root, record) in state.iter_mut() {
                if (!verify_inputs && affects(root))
                    || record.inputs.iter().any(|(uri, snapshot)| {
                        affects(uri) && (!verify_inputs || snapshot.changed(uri))
                    })
                    || (record.checks.watched().iter().any(&affects)
                        && record.checks.changed_path().is_some())
                {
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
    pub pages: Option<u32>,
    pub log: Option<PathBuf>,
    pub diagnostics: BTreeMap<Url, Vec<Diagnostic>>,
    pub diagnostics_truncated: bool,
    /// Summary available only from this run's observed final TeX log.
    pub analysis: Option<Value>,
    /// Sources whose file-entry events were present in the final TeX log.
    pub diagnostic_sources: BTreeSet<Url>,
    pub output: String,
    pub inputs: Option<BTreeSet<Url>>,
    /// Recorder-confirmed image/data types, before resolving symlink names.
    pub binary_inputs: BTreeSet<Url>,
    pub bindings: BTreeMap<PathBuf, PathBuf>,
    pub unresolved_bindings: BTreeSet<PathBuf>,
    pub bibliography_inputs: Option<BTreeSet<Url>>,
    pub bibliography_bindings: BTreeMap<PathBuf, PathBuf>,
    pub bibliography_unresolved_bindings: BTreeSet<PathBuf>,
    pub pdf_updated: bool,
}

impl Outcome {
    fn error(message: String) -> Self {
        Self {
            status: "error",
            exit_code: None,
            message,
            pdf: None,
            pages: None,
            log: None,
            diagnostics: BTreeMap::new(),
            diagnostics_truncated: false,
            analysis: None,
            diagnostic_sources: BTreeSet::new(),
            output: String::new(),
            inputs: None,
            binary_inputs: BTreeSet::new(),
            bindings: BTreeMap::new(),
            unresolved_bindings: BTreeSet::new(),
            bibliography_inputs: None,
            bibliography_bindings: BTreeMap::new(),
            bibliography_unresolved_bindings: BTreeSet::new(),
            pdf_updated: false,
        }
    }
}

pub(crate) fn capture(reader: impl Read) -> Vec<u8> {
    crate::output::capture(reader, MAX_OUTPUT)
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
    run_with_mode(
        root,
        config,
        cancel,
        false,
        None,
        std::iter::once(root.to_path_buf()),
    )
}

pub fn rebuild(root: &Path, config: &BuildConfig, cancel: &AtomicBool) -> Outcome {
    run_with_mode(
        root,
        config,
        cancel,
        true,
        None,
        std::iter::once(root.to_path_buf()),
    )
}

fn run_with_mode(
    root: &Path,
    config: &BuildConfig,
    cancel: &AtomicBool,
    rebuild: bool,
    previous_pdf: Option<&Snapshot>,
    inputs: impl Iterator<Item = PathBuf>,
) -> Outcome {
    // A task can be cancelled while waiting for client progress registration,
    // before its worker starts. Preserve existing artifacts in that case.
    if cancel.load(Ordering::Relaxed) {
        return Outcome {
            status: "cancelled",
            ..Outcome::error("Compilation cancelled".into())
        };
    }
    // On first opening a project there is no content history for its existing
    // PDF. latexmk may skip a source saved with identical contents even though
    // its timestamp is newer. Force one pass to establish a verified PDF and
    // recorder instead of rejecting that legitimate incremental build.
    let refresh = !rebuild
        && config.arguments.is_none()
        && config.rebuild_arguments.is_none()
        && Path::new(&config.executable)
            .file_stem()
            .is_some_and(|name| name == "latexmk")
        && config.artifact_path(root, "pdf").is_some_and(|pdf| {
            previous_pdf.is_none_or(|previous| !previous.matches_pdf(&pdf))
                && outdated_pdf_input(&pdf, inputs, None).is_some()
        });
    let mut outcome = run_inner(root, config, cancel, rebuild || refresh);
    reject_outdated_pdf(
        root,
        &mut outcome,
        std::iter::once(root.to_path_buf()),
        previous_pdf,
    );
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

fn reject_outdated_pdf(
    root: &Path,
    outcome: &mut Outcome,
    sources: impl Iterator<Item = PathBuf>,
    previous_pdf: Option<&Snapshot>,
) {
    if outcome.status != "success" || outcome.pdf_updated {
        return;
    }
    let Some(pdf_path) = outcome.pdf.as_ref() else {
        return;
    };
    if let Some(source) = outdated_pdf_input(pdf_path, sources, previous_pdf) {
        outcome.status = "error";
        outcome.pdf = None;
        outcome.pages = None;
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

fn outdated_pdf_input(
    pdf_path: &Path,
    mut sources: impl Iterator<Item = PathBuf>,
    previous_pdf: Option<&Snapshot>,
) -> Option<PathBuf> {
    let pdf = fingerprint(pdf_path).and_then(|(_, modified)| modified)?;
    // Editors can rewrite identical inputs after the PDF was produced, while
    // latexmk correctly skips TeX. Only override the timestamp check using the
    // input history of this exact, unmodified PDF; unknown inputs stay unsafe.
    let previous_pdf =
        previous_pdf.filter(|previous| previous.matches_pdf(pdf_path) && !previous.pdf_changed());
    sources.find(|source| {
        std::fs::metadata(source)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .is_none_or(|modified| {
                modified > pdf
                    && previous_pdf.is_none_or(|previous| {
                        Url::from_file_path(source).map_or(true, |uri| previous.input_changed(&uri))
                    })
            })
    })
}

fn run_inner(root: &Path, config: &BuildConfig, cancel: &AtomicBool, rebuild: bool) -> Outcome {
    let Some(parent) = root.parent() else {
        return Outcome::error("Invalid root path".into());
    };
    // Validate custom rebuild arguments before touching logs or artifacts.
    let arguments = if rebuild {
        match config.expanded_rebuild_arguments(root) {
            Ok(arguments) => arguments,
            Err(message) => return Outcome::error(message),
        }
    } else {
        let Some(arguments) = config.expanded_arguments(root) else {
            return Outcome::error("Invalid root filename".into());
        };
        arguments
    };
    let output_dir = parent.join(&config.output_directory);
    if let Err(error) = std::fs::create_dir_all(&output_dir) {
        return Outcome::error(format!("Cannot create build directory: {error}"));
    }
    if matches!(
        Path::new(&config.executable)
            .file_stem()
            .and_then(|name| name.to_str()),
        Some("latexmk" | "pdflatex" | "xelatex" | "lualatex")
    ) {
        if let Err(error) = prepare_include_directories(root, &output_dir) {
            return Outcome::error(format!(
                "Cannot prepare chapter auxiliary directories: {error}"
            ));
        }
    }
    let Some(log_path) = config.artifact_path(root, "log") else {
        return Outcome::error("Invalid root filename".into());
    };
    let pdf_path = config.artifact_path(root, "pdf").unwrap();
    let pdf_before = ArtifactFingerprint::capture(&pdf_path);
    let recorder_path = config.artifact_path(root, "fls").unwrap();
    let recorder_before = ArtifactFingerprint::capture(&recorder_path);
    let bibliography_before = bibliography_logs(&output_dir);
    // A previous log must never be interpreted as the result of a failed spawn.
    if log_path.exists() {
        if let Err(error) = std::fs::remove_file(&log_path) {
            return Outcome::error(format!("Cannot replace compiler log: {error}"));
        }
    }
    let diagnostic_project = build_log::ProjectSources::capture(root);
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
        build_log::decode_output(&stdout),
        build_log::decode_output(&stderr)
    );
    let log = read_log(&log_path).unwrap_or_default();
    outcome.pdf_updated = outcome
        .pdf
        .as_ref()
        .is_some_and(|pdf| ArtifactFingerprint::capture(pdf) != pdf_before);
    let recorder_after = ArtifactFingerprint::capture(&recorder_path);
    if recorder_after.is_some() && recorder_after != recorder_before {
        // Partial dependency lists must not replace the last complete recorder.
        if let Some(inputs) = read_log(&recorder_path)
            .filter(|log| !crate::output::truncated(log))
            .map(|log| crate::recorder::classified_inputs(root, &log))
        {
            outcome.inputs = Some(inputs.all);
            outcome.binary_inputs = inputs.binary;
            outcome.bindings = inputs.bindings;
            outcome.unresolved_bindings = inputs.unresolved_bindings;
        }
    }
    // latexmk can run TeX several times. The final .log is authoritative;
    // earlier stdout passes may contain references that are already resolved.
    let diagnostic_log = if log.is_empty() { &output } else { &log };
    outcome.diagnostics_truncated = crate::output::truncated(diagnostic_log);
    (outcome.diagnostics, outcome.diagnostic_sources) =
        build_log::parse_with_project_sources(root, diagnostic_log, &diagnostic_project);
    // A failed PDF driver may report its cause only on stderr. Retain the final
    // TeX log's authority: fill an unexplained failure, without reviving an old
    // backend error after a successful pass or a different final TeX error.
    if !log.is_empty()
        && outcome.status == "error"
        && !outcome
            .diagnostics
            .values()
            .flatten()
            .any(|diagnostic| diagnostic.severity == Some(DiagnosticSeverity::ERROR))
    {
        let driver_diagnostics = build_log::parse_driver_output(root, &output);
        if !driver_diagnostics.is_empty() {
            outcome.diagnostics_truncated |= crate::output::truncated(&output);
        }
        for (uri, diagnostics) in driver_diagnostics {
            let entries = outcome.diagnostics.entry(uri).or_default();
            for diagnostic in diagnostics {
                if !entries.contains(&diagnostic) {
                    entries.push(diagnostic);
                }
            }
        }
    }
    let mut bibliography_complete = true;
    for (path, fingerprint) in bibliography_logs(&output_dir) {
        // An old .blg left by a previous failed build is not this build's result.
        if bibliography_before.get(&path) == Some(&fingerprint) {
            continue;
        }
        if let Some(log) = read_log(&path) {
            let truncated = crate::output::truncated(&log);
            bibliography_complete &= !truncated;
            outcome.diagnostics_truncated |= truncated;
            let paths = crate::recorder::BibliographyPaths::new(root, &path, &log)
                .with_search_output(&output);
            if !truncated {
                let inputs = crate::recorder::classified_bibliography_inputs(&paths, &log);
                outcome
                    .bibliography_inputs
                    .get_or_insert_with(BTreeSet::new)
                    .extend(inputs.all);
                outcome.bibliography_bindings.extend(inputs.bindings);
                outcome
                    .bibliography_unresolved_bindings
                    .extend(inputs.unresolved_bindings);
            }
            let mut diagnostics = build_log::parse_bibliography_in(root, &paths, &log);
            if !outcome.diagnostics_truncated {
                build_log::locate_missing_bibliography_styles(
                    &outcome.diagnostic_sources,
                    &mut diagnostics,
                );
            }
            for (uri, diagnostics) in diagnostics {
                outcome
                    .diagnostics
                    .entry(uri)
                    .or_default()
                    .extend(diagnostics);
            }
        }
    }
    if !bibliography_complete {
        outcome.bibliography_inputs = None;
        outcome.bibliography_bindings.clear();
        outcome.bibliography_unresolved_bindings.clear();
    }
    if Path::new(&config.executable)
        .file_stem()
        .and_then(|name| name.to_str())
        == Some("latexmk")
    {
        for (uri, diagnostics) in
            build_log::parse_latexmk_missing_databases(root, &log_path, &output)
        {
            for diagnostic in diagnostics {
                // An actual BibTeX error is more useful than the driver's
                // warning about the same missing database.
                let duplicate = outcome.diagnostics.values().flatten().any(|item| {
                    item.code == diagnostic.code
                        && item.data.as_ref().and_then(|data| data.get("file"))
                            == diagnostic.data.as_ref().and_then(|data| data.get("file"))
                });
                if !duplicate {
                    outcome
                        .diagnostics
                        .entry(uri.clone())
                        .or_default()
                        .push(diagnostic);
                }
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
    if outcome.pdf.is_some() {
        outcome.pages = build_log::page_count(if log.is_empty() { &output } else { &log });
    }
    outcome.output = output
        .chars()
        .rev()
        .take(8192)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if !log.is_empty() && matches!(outcome.status, "success" | "error") {
        outcome.analysis = Some(build_log::compilation_analysis(
            &outcome.diagnostics,
            outcome.diagnostics_truncated,
        ));
    }
    outcome
}

/// TeX writes `chapters/name.aux` below the output directory, but does not
/// create `chapters/`. latexmk's recovery is not reliable with file-line errors.
/// Prepare only static paths contained in that output directory.
fn prepare_include_directories(root: &Path, output: &Path) -> std::io::Result<()> {
    let Ok(uri) = Url::from_file_path(root) else {
        return Ok(());
    };
    let mut workspace = Workspace::default();
    workspace.refresh_project(&uri);
    let inputs = crate::source_inputs::Index::new(&workspace, &uri);
    let mut directories = BTreeSet::new();
    for (uri, doc) in &workspace.documents {
        let indirect = inputs
            .calls
            .get(uri)
            .into_iter()
            .flatten()
            .filter(|call| call.certain)
            .map(|call| &call.command);
        for command in doc
            .syntax
            .commands
            .iter()
            .chain(indirect)
            .filter(|command| command.name == "include")
            .take(4096)
        {
            let Some(argument) = crate::syntax::file_argument(&doc.text.source, command) else {
                continue;
            };
            let name = argument.value.trim();
            if name.is_empty() || name.contains(['\\', '%', '#', '$', '{', '}', '\n', '\r', '\0']) {
                continue;
            }
            let path = Path::new(name);
            if path.components().any(|component| {
                !matches!(
                    component,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            }) {
                continue;
            }
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                directories.insert(output.join(parent));
                if directories.len() >= 1024 {
                    break;
                }
            }
        }
        if directories.len() >= 1024 {
            break;
        }
    }
    for directory in directories {
        std::fs::create_dir_all(directory)?;
    }
    Ok(())
}

fn read_log(path: &Path) -> Option<String> {
    let text = crate::output::read(path, MAX_OUTPUT * 2).ok()?;
    Some(build_log::decode_output(&text).into_owned())
}

fn bibliography_logs(directory: &Path) -> BTreeMap<PathBuf, ArtifactFingerprint> {
    std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()? != "blg" {
                return None;
            }
            if !entry.metadata().ok()?.is_file() {
                return None;
            }
            let snapshot = ArtifactFingerprint::capture(&path)?;
            Some((path, snapshot))
        })
        .take(64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_rebuilds_do_not_follow_a_changed_root_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.tex");
        let second = directory.path().join("second.tex");
        let source = "\\documentclass{article}\n";
        std::fs::write(&first, source).unwrap();
        std::fs::write(&second, source).unwrap();
        let first = Url::from_file_path(first).unwrap();
        let second = Url::from_file_path(second).unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(directory.path().into());
        workspace.config.build.root_file = Some("second.tex".into());
        workspace.config.build.executable = "unavailable-custom-compiler".into();
        workspace.config.build.rebuild_arguments = Some(vec![]);
        let workspace = Arc::new(RwLock::new(workspace));
        let mut manager = None;
        let (_service, _socket) = tower_lsp::LspService::new(|client| {
            manager = Some(BuildManager::new(client.clone(), workspace));
            crate::Backend::new(client)
        });
        let manager = manager.unwrap();
        manager
            .state
            .lock()
            .unwrap()
            .insert(first.clone(), Record::default());
        for rebuild in [false, true] {
            let delayed = manager
                .start_request(first.clone(), Some((first.clone(), 0)), rebuild, None)
                .await
                .unwrap();
            assert_eq!(delayed["status"], "cancelled", "{delayed}");
            assert_eq!(delayed["rootUri"], first.as_str());
            assert_eq!(manager.status(&second)["status"], "idle");
            assert_eq!(manager.tasks.load(Ordering::SeqCst), 0);
            assert!(!directory.path().join("build").exists());
        }
    }

    #[tokio::test]
    async fn delayed_force_callbacks_preserve_newer_builds_saves_and_cancellations() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "\\documentclass{article}\n";
        std::fs::write(&main, source).unwrap();
        let uri = Url::from_file_path(main).unwrap();
        let mut workspace = Workspace::default();
        workspace.open(uri.clone(), source.into(), 1);
        let workspace = Arc::new(RwLock::new(workspace));
        let mut manager = None;
        let (_service, _socket) = tower_lsp::LspService::new(|client| {
            manager = Some(BuildManager::new(client.clone(), workspace));
            crate::Backend::new(client)
        });
        let manager = manager.unwrap();
        manager.state.lock().unwrap().insert(
            uri.clone(),
            Record {
                active: Some((99, Arc::default())),
                rebuild: true,
                queued: Some(QueuedBuild {
                    uri: uri.clone(),
                    rebuild: false,
                }),
                ..Default::default()
            },
        );
        // A force callback from an earlier completion is satisfied by a newer
        // forced build, and must leave that build's ordinary save queue intact.
        let delayed = manager
            .start_request(uri.clone(), Some((uri.clone(), 0)), true, None)
            .await
            .unwrap();
        assert_eq!(delayed["queuedRebuild"], false);
        assert_eq!(delayed["queued"], true);
        // A new user request still asks for another force after the active one.
        let manual = manager.rebuild(uri.clone()).await.unwrap();
        assert_eq!(manual["queuedRebuild"], true);
        // If an ordinary build overtook the old callback, retain the force.
        {
            let mut state = manager.state.lock().unwrap();
            let record = state.get_mut(&uri).unwrap();
            record.rebuild = false;
            record.queued = None;
        }
        let delayed = manager
            .start_request(uri.clone(), Some((uri.clone(), 0)), true, None)
            .await
            .unwrap();
        assert_eq!(delayed["queuedRebuild"], true);
        assert_eq!(delayed["rebuild"], false);
        manager.cancel(Some(&uri));
        let delayed = manager
            .start_request(uri.clone(), Some((uri.clone(), 0)), true, None)
            .await
            .unwrap();
        assert_eq!(delayed["status"], "cancelled");
        assert!(manager.state.lock().unwrap()[&uri].queued.is_none());
        assert_eq!(manager.tasks.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_new_manual_rebuild_invalidates_a_detached_force_callback() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("main.tex");
        let source = "\\documentclass{article}\n";
        std::fs::write(&main, source).unwrap();
        let uri = Url::from_file_path(main).unwrap();
        let mut workspace = Workspace::default();
        workspace.open(uri.clone(), source.into(), 1);
        workspace.config.build.executable = "unavailable-custom-compiler".into();
        workspace.config.build.rebuild_arguments = Some(vec![]);
        let workspace = Arc::new(RwLock::new(workspace));
        let mut manager = None;
        let (_service, _socket) = tower_lsp::LspService::new(|client| {
            manager = Some(BuildManager::new(client.clone(), workspace));
            crate::Backend::new(client)
        });
        let manager = manager.unwrap();
        manager
            .state
            .lock()
            .unwrap()
            .insert(uri.clone(), Record::default());
        let manual = manager.rebuild(uri.clone()).await.unwrap();
        assert_eq!(manual["status"], "started");
        let delayed = manager
            .start_request(uri.clone(), Some((uri.clone(), 0)), true, None)
            .await
            .unwrap();
        assert_eq!(delayed["status"], "cancelled");
        assert!(manager.state.lock().unwrap()[&uri].queued.is_none());
        manager.disconnect();
        manager.shutdown().await;
        assert_eq!(manager.tasks.load(Ordering::SeqCst), 0);
    }
}
