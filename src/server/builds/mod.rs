//! Build queueing and connection-owned coordination.
use crate::project::store::WorkspaceStore;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use crate::rpc::{Error, Result};
use crate::server::client::Client;
use crate::{
    compilation::{build_log, file_checks::InputSnapshot},
    config::BuildConfig,
};
use lsp_types::*;
use serde_json::{json, Value};
use tokio::sync::Notify;

use crate::compilation::build::{reject_outdated_pdf, run_with_mode, Outcome};
mod completion;

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
    checks: Arc<crate::compilation::file_checks::Snapshot>,
    artifact: Option<PathBuf>,
    // Preserve the path passed to the compiler as well as initial physical
    // artifact names: a directory alias may be retargeted while it runs.
    output_directory: Option<PathBuf>,
    auxiliary_directory: Option<PathBuf>,
    auxiliary_artifact: Option<PathBuf>,
    index_artifacts: BTreeSet<PathBuf>,
    bibliography_output_directory: Option<PathBuf>,
}

fn artifact_binding(path: PathBuf) -> PathBuf {
    path.parent()
        .and_then(|parent| parent.canonicalize().ok())
        .zip(path.file_name())
        .map_or_else(
            || crate::project::workspace::normalize_path(&path),
            |(parent, name)| parent.join(name),
        )
}

impl Record {
    fn current_artifact(&self, path: &Path) -> Option<PathBuf> {
        Some(
            self.output_directory
                .as_ref()?
                .canonicalize()
                .ok()?
                .join(path.file_name()?),
        )
    }

    fn shares_pdf(&self, artifact: &Path) -> bool {
        self.artifact.as_ref().is_some_and(|previous| {
            previous == artifact || self.current_artifact(previous).as_deref() == Some(artifact)
        })
    }

    fn shared_index(&self, candidates: &BTreeSet<PathBuf>) -> Option<PathBuf> {
        self.index_artifacts.iter().find_map(|path| {
            [
                Some(path.clone()),
                self.auxiliary_directory.as_ref().and_then(|directory| {
                    Some(directory.canonicalize().ok()?.join(path.file_name()?))
                }),
            ]
            .into_iter()
            .flatten()
            .find(|path| candidates.contains(path))
        })
    }
}

struct QueuedBuild {
    uri: Url,
    rebuild: bool,
}

#[derive(Clone)]
pub struct BuildManager {
    client: Client,
    workspace: WorkspaceStore,
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
    checks: Arc<crate::compilation::file_checks::Snapshot>,
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
    pub fn new(client: Client, workspace: WorkspaceStore) -> Self {
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

    pub(crate) async fn clean(&self, uri: Url, artifacts: bool, dry_run: bool) -> Result<Value> {
        let mut workspace = self.workspace.write().await;
        let root = workspace
            .root_document(&uri)
            .map_err(Error::invalid_params)?;
        workspace.refresh_project(&root);
        let config = workspace.build_config(&root);
        config
            .validate_resolution()
            .map_err(Error::invalid_params)?;
        let root_path = root
            .to_file_path()
            .map_err(|_| Error::invalid_params("Cleanup requires a local file"))?;
        let physical = |path: PathBuf| {
            path.canonicalize()
                .ok()
                .or_else(|| {
                    path.parent()?
                        .canonicalize()
                        .ok()
                        .map(|parent| parent.join(path.file_name().unwrap()))
                })
                .unwrap_or_else(|| crate::project::workspace::normalize_path(&path))
        };
        let artifact = config
            .artifact_path(&root_path, "pdf")
            .map(artifact_binding);
        let index_artifacts: BTreeSet<_> =
            crate::compilation::index_processing::outputs(&root_path, &config)
                .into_iter()
                .map(artifact_binding)
                .collect();
        let directories: Vec<_> = [
            config.output_path(&root_path),
            config.auxiliary_path(&root_path),
            config.final_output_path(&root_path),
        ]
        .into_iter()
        .flatten()
        .map(&physical)
        .collect();
        let directory = config.output_path(&root_path).map(physical);
        let (result, affected) = {
            // Serialize with build reservation and queued generation checks.
            // No process is cancelled implicitly by a cleanup request.
            let mut state = self.state.lock().unwrap();
            if self.closing.load(Ordering::SeqCst) {
                return Err(Error::invalid_request());
            }
            for (owner, record) in state.iter() {
                let shared = artifact
                    .as_ref()
                    .is_some_and(|path| record.shares_pdf(path))
                    || record.shared_index(&index_artifacts).is_some();
                let same_directory = record
                    .artifact
                    .as_ref()
                    .and_then(|path| path.parent())
                    .zip(directory.as_deref())
                    .is_some_and(|(a, b)| a == b)
                    || record.output_directory.as_ref().is_some_and(|path| {
                        directory
                            .as_ref()
                            .is_some_and(|directory| physical(path.clone()) == *directory)
                    })
                    || record
                        .index_artifacts
                        .iter()
                        .any(|path| directory.as_ref().is_some_and(|dir| path.starts_with(dir)));
                if record.active.is_some()
                    && (owner == &root
                        || shared
                        || same_directory
                        || [&record.output_directory, &record.auxiliary_directory]
                            .into_iter()
                            .flatten()
                            .any(|path| directories.contains(&physical(path.clone()))))
                {
                    return Err(Error::invalid_params(format!(
                        "Wait for the active build of {owner} before cleaning its output directory"
                    )));
                }
                if owner != &root && shared {
                    return Err(Error::invalid_params(format!("Build artifacts are shared with {owner}; choose a separate jobName or outputDirectory before cleaning")));
                }
            }
            let plan = crate::compilation::build_clean::Plan::prepare(
                &workspace, &root, &config, artifacts,
            )
            .map_err(Error::invalid_params)?;
            let mut result = plan.execute(dry_run).map_err(Error::invalid_params)?;
            result["rootUri"] = json!(root);
            result["outputDirectory"] = json!(Url::from_directory_path(&plan.directory).ok());
            result["artifacts"] = json!(artifacts);
            let mut affected = BTreeSet::new();
            if !dry_run {
                let record = state.entry(root.clone()).or_default();
                record.queued = None;
                record.cancel_generation = record.cancel_generation.wrapping_add(1);
                record.result = None;
                record.inputs.clear();
                record.checks = Arc::default();
                if let Some(old) = workspace.project.build_diagnostics.remove(&root) {
                    affected.extend(old.into_keys());
                }
                workspace.project.build_diagnostic_sources.remove(&root);
                workspace.project.build_checks.remove(&root);
                if artifacts {
                    workspace.project.pdf_checks.remove(&root);
                }
                affected.insert(root.clone());
                affected.extend(
                    workspace
                        .documents
                        .iter()
                        .filter_map(|(uri, doc)| doc.version.is_some().then_some(uri.clone())),
                );
            }
            (result, affected)
        };
        let delivery = workspace.diagnostic_delivery.clone();
        let notifications = if delivery.is_pull() {
            vec![]
        } else {
            workspace.diagnostic_snapshots(affected)
        };
        drop(workspace);
        if !dry_run {
            delivery.publish(&self.client, notifications).await;
        }
        Ok(result)
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
        let path = root
            .to_file_path()
            .map_err(|_| Error::invalid_params("Compilation requires a local file"))?;
        let mut config = workspace.build_config(&root).for_build_mode(&path, rebuild);
        if config.resolution.native || !config.resolution.issues.is_empty() {
            let path = root
                .to_file_path()
                .map_err(|_| Error::invalid_params("Compilation requires a local file"))?;
            drop(workspace);
            config = tokio::task::spawn_blocking(move || {
                config
                    .unresolved()
                    .resolve_for_build(&path, &AtomicBool::new(false))
            })
            .await
            .map_err(|error| Error::invalid_params(error.to_string()))?
            .map_err(Error::invalid_params)?;
            workspace = self.workspace.write().await;
            if workspace.root_document(&uri).ok().as_ref() != Some(&root) {
                return Err(Error::invalid_params(
                    "Root changed during latexmk configuration resolution; build again",
                ));
            }
            // Cache lookup now supplies the native paths. Explicit editor settings
            // must still match the configuration which requested the probe.
            let current = workspace.build_config(&root);
            if serde_json::to_value(current.unresolved()).unwrap()
                != serde_json::to_value(config.unresolved()).unwrap()
            {
                return Err(Error::invalid_params(
                    "Build settings changed during latexmk configuration resolution; build again",
                ));
            }
        }
        config
            .validate_resolution()
            .map_err(Error::invalid_params)?;
        let static_code_inputs = crate::project::code_inputs::inputs(&workspace, &root);
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
        if let Ok(path) = root.to_file_path() {
            project.extend(
                crate::compilation::index_processing::dependencies(&path, &config)
                    .into_iter()
                    .chain(crate::compilation::bibliography_processing::dependencies(
                        &path, &config,
                    ))
                    .flat_map(|path| {
                        let physical = path.canonicalize().ok();
                        [Some(path), physical].into_iter().flatten()
                    })
                    .filter_map(|path| Url::from_file_path(path).ok()),
            );
        }
        let mut snapshots = BTreeMap::new();
        let mut file_snapshots = BTreeMap::new();
        for uri in &project {
            if let Some(doc) = workspace.documents.get(uri) {
                let saved = uri
                    .to_file_path()
                    .ok()
                    .and_then(|path| std::fs::read_to_string(path).ok());
                if !saved
                    .as_deref()
                    .is_some_and(|saved| doc.text.matches_saved(saved))
                {
                    return Err(Error::invalid_params(format!(
                        "Save {} before compiling",
                        uri.path()
                    )));
                }
                snapshots.insert(uri.clone(), (doc.version, doc.text.source.clone()));
                if let Ok(path) = uri.to_file_path() {
                    // Retain the exact bytes TeX reads separately from the
                    // editor's newline representation and document version.
                    file_snapshots.insert(uri.clone(), InputSnapshot::file(&path));
                }
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
        if rebuild {
            config.rebuild_arguments().map_err(Error::invalid_params)?;
        }
        let checks = Arc::new(crate::compilation::file_checks::Snapshot::capture(
            &workspace, &root, &config,
        ));
        let progress_supported = workspace.client.build_progress;
        let root_path = root
            .to_file_path()
            .map_err(|_| Error::invalid_params("Compilation requires a local file"))?;
        drop(workspace);
        let build_id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let artifact = config
            .artifact_path(&root_path, "pdf")
            .map(artifact_binding);
        let index_artifacts: BTreeSet<_> =
            crate::compilation::index_processing::outputs(&root_path, &config)
                .into_iter()
                .map(artifact_binding)
                .collect();
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
                    && (artifact
                        .as_ref()
                        .is_some_and(|path| record.shares_pdf(path))
                        || config.artifact_path(&root_path, "log").is_some_and(|path| {
                            record.auxiliary_artifact.as_ref().is_some_and(|previous| {
                                artifact_binding(previous.clone()) == artifact_binding(path)
                            })
                        }))
            }) {
                return Err(Error::invalid_params(format!(
                    "Another active build writes the same output file: {}; wait for {} to finish or choose a different jobName or outputDirectory",
                    artifact.as_ref().unwrap().display(), owner
                )));
            }
            if let Some((owner, path)) = state.iter().find_map(|(owner, record)| {
                if owner == &root || record.active.is_none() {
                    return None;
                }
                record
                    .shared_index(&index_artifacts)
                    .map(|path| (owner, path))
            }) {
                return Err(Error::invalid_params(format!("Another active build writes the same index output file: {}; wait for {} to finish or choose a different index jobName or outputDirectory",
                    path.display(), owner)));
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
                    let snapshot = file_snapshots
                        .get(uri)
                        .cloned()
                        .unwrap_or(InputSnapshot::File(None));
                    (uri.clone(), snapshot)
                })
                .collect();
            record.checks = Arc::new(checks.with_inputs(file_snapshots.clone()));
            record.artifact = artifact;
            record.output_directory = config.final_output_path(&root_path);
            record.auxiliary_directory = config.auxiliary_path(&root_path);
            record.auxiliary_artifact = config.artifact_path(&root_path, "log");
            record.index_artifacts = index_artifacts;
            record.bibliography_output_directory =
                config.bibliography_tool.as_ref().and_then(|_| {
                    config
                        .auxiliary_path(&root_path)
                        .map(|path| crate::project::workspace::normalize_path(&path))
                });
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

    pub async fn files_renamed(&self, renames: &crate::editing::file_operations::Renames) {
        self.cancel_changed_inputs(|uri| renames.affects(uri), false)
            .await;
    }

    pub async fn files_changed(&self, changes: &crate::editing::file_operations::FileChanges) {
        self.cancel_changed_inputs(|uri| changes.affects(uri), false)
            .await;
    }

    pub async fn watched_files_changed(
        &self,
        changes: &crate::editing::file_operations::FileChanges,
    ) -> BTreeSet<Url> {
        // Capture affected roots before cancellation or diagnostic refresh drops
        // their old input snapshots. A delayed duplicate must not enqueue work.
        let roots = self
            .state
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, record)| record.checks.image_changed(|uri| changes.affects(uri)))
            .map(|(root, _)| root.clone())
            .collect();
        self.cancel_changed_inputs(|uri| changes.affects(uri), true)
            .await;
        roots
    }

    async fn cancel_changed_inputs(&self, affects: impl Fn(&Url) -> bool, verify_inputs: bool) {
        let mut builds = BTreeSet::new();
        {
            let mut state = self.state.lock().unwrap();
            for (root, record) in state.iter_mut() {
                // TeX may rewrite a database itself (e.g. filecontents).
                // Watch notifications cannot identify the writer. For explicit
                // pipelines, validate databases inside the output directory at
                // completion using recorder/backend snapshots instead.
                let defer_generated_database = |uri: &Url| {
                    verify_inputs
                        && record.active.is_some()
                        && uri.to_file_path().is_ok_and(|path| {
                            path.extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("bib"))
                                && record.bibliography_output_directory.as_ref().is_some_and(
                                    |directory| {
                                        crate::project::workspace::normalize_path(&path)
                                            .starts_with(directory)
                                    },
                                )
                        })
                };
                if (!verify_inputs && affects(root))
                    || record.checks.image_changed(&affects)
                    || record.inputs.iter().any(|(uri, snapshot)| {
                        !defer_generated_database(uri)
                            && affects(uri)
                            && (!verify_inputs || snapshot.changed(uri))
                    })
                    || (record
                        .checks
                        .watched()
                        .iter()
                        .any(|uri| !defer_generated_database(uri) && affects(uri))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::workspace::Workspace;

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
        let workspace = WorkspaceStore::new(workspace);
        let (client, _outgoing) = Client::new();
        let manager = BuildManager::new(client, workspace);
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
        let workspace = WorkspaceStore::new(workspace);
        let (client, _outgoing) = Client::new();
        let manager = BuildManager::new(client, workspace);
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
        let workspace = WorkspaceStore::new(workspace);
        let (client, _outgoing) = Client::new();
        let manager = BuildManager::new(client, workspace);
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
