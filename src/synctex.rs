//! Query SyncTeX without launching viewers or editors.
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime},
};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{Notify, Semaphore};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::*,
};

use crate::{file_checks::InputSnapshot, workspace::Workspace};

#[derive(Clone)]
pub struct Queries {
    slots: Arc<Semaphore>,
    closing: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

impl Default for Queries {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(4)),
            closing: Arc::default(),
            active: Arc::default(),
            idle: Arc::default(),
        }
    }
}

struct Cancellation {
    request: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
}

impl Cancellation {
    fn cancelled(&self) -> bool {
        self.request.load(Ordering::SeqCst) || self.closing.load(Ordering::SeqCst)
    }
}

pub struct QueryTask {
    handle: tokio::task::JoinHandle<Result<Value>>,
    cancel: Arc<AtomicBool>,
}

impl QueryTask {
    pub async fn wait(&mut self) -> Result<Value> {
        (&mut self.handle)
            .await
            .map_err(|error| Error::invalid_params(format!("SyncTeX task failed: {error}")))?
    }
}

impl Drop for QueryTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

struct ActiveQuery {
    active: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

impl Drop for ActiveQuery {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.idle.notify_waiters();
    }
}

impl Queries {
    pub fn start(&self, project: Project, value: Value, inverse_search: bool) -> Result<QueryTask> {
        enum Request {
            Forward(ForwardRequest),
            Inverse(InverseRequest),
        }
        let request = if inverse_search {
            Request::Inverse(serde_json::from_value(value).map_err(|error| {
                Error::invalid_params(format!("Invalid inverse search request: {error}"))
            })?)
        } else {
            Request::Forward(serde_json::from_value(value).map_err(|error| {
                Error::invalid_params(format!("Invalid forward search request: {error}"))
            })?)
        };
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            Error::invalid_params("Too many concurrent SyncTeX queries; retry shortly")
        })?;
        if self.closing.load(Ordering::SeqCst) {
            return Err(Error::invalid_request());
        }
        self.active.fetch_add(1, Ordering::SeqCst);
        let guard = ActiveQuery {
            active: self.active.clone(),
            idle: self.idle.clone(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let cancellation = Cancellation {
            request: cancel.clone(),
            closing: self.closing.clone(),
        };
        let handle = tokio::task::spawn_blocking(move || {
            let (_permit, _guard) = (permit, guard);
            if cancellation.cancelled() {
                return Err(Error::request_cancelled());
            }
            match request {
                Request::Forward(request) => forward(&project, &request, &cancellation),
                Request::Inverse(request) => inverse(&project, &request, &cancellation),
            }
        });
        Ok(QueryTask { handle, cancel })
    }

    pub async fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        loop {
            let idle = self.idle.notified();
            if self.active.load(Ordering::SeqCst) == 0 {
                break;
            }
            idle.await;
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForwardRequest {
    pub uri: Url,
    pub position: Position,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InverseRequest {
    pub uri: Url,
    pub page: u32,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub root: Url,
    pub pdf: PathBuf,
    directory: PathBuf,
    artifacts: Vec<(PathBuf, u64, SystemTime, InputSnapshot)>,
}

/// Refuse to map current editor text against old or incomplete build artifacts.
pub fn prepare(workspace: &Workspace, uri: &Url) -> Result<Project> {
    let root = workspace
        .root_document(uri)
        .map_err(Error::invalid_params)?;
    let path = root
        .to_file_path()
        .map_err(|_| Error::invalid_params("SyncTeX requires local files"))?;
    let directory = path.parent().unwrap().to_path_buf();
    let config = workspace.build_config(&root);
    let pdf = config
        .artifact_path(&path, "pdf")
        .ok_or_else(|| Error::invalid_params("Invalid root filename"))?;
    if workspace
        .pdf_checks
        .get(&root)
        .is_some_and(|checks| checks.matches_pdf(&pdf) && checks.changed_path().is_some())
    {
        return Err(Error::invalid_params(
            "File conditions changed; compile again before querying SyncTeX",
        ));
    }
    if workspace
        .pdf_checks
        .get(&root)
        .is_some_and(|checks| checks.matches_pdf(&pdf) && checks.changed_input().is_some())
    {
        return Err(Error::invalid_params(
            "Input contents changed; compile again before querying SyncTeX",
        ));
    }
    if workspace.pdf_checks.get(&root).is_some_and(|checks| {
        checks.matches_pdf(&pdf) && (checks.pdf_changed() || checks.synctex_changed())
    }) {
        return Err(Error::invalid_params(
            "Build artifacts changed; compile again before querying SyncTeX",
        ));
    }
    let mut artifacts = Vec::new();
    for path in [
        pdf.clone(),
        config.artifact_path(&path, "synctex.gz").unwrap(),
        config.artifact_path(&path, "synctex").unwrap(),
    ] {
        if let Ok(metadata) = std::fs::metadata(&path) {
            if metadata.is_file() && metadata.len() > 0 {
                if let Ok(modified) = metadata.modified() {
                    let contents = InputSnapshot::file(&path);
                    artifacts.push((path, metadata.len(), modified, contents));
                }
            }
        }
    }
    if !artifacts.iter().any(|(path, ..)| path == &pdf) || artifacts.len() < 2 {
        return Err(Error::invalid_params(
            "Compile with SyncTeX enabled first; PDF or SyncTeX data is missing",
        ));
    }
    let recorded = artifacts
        .iter()
        .filter(|(path, ..)| path != &pdf)
        .map(|(_, _, modified, _)| *modified)
        .max()
        .unwrap();
    let mut inputs = workspace.project_files(&root);
    inputs.insert(uri.clone());
    for input in inputs {
        let path = input
            .to_file_path()
            .map_err(|_| Error::invalid_params("SyncTeX requires local files"))?;
        let metadata = std::fs::metadata(&path).map_err(|_| {
            Error::invalid_params("Project inputs changed; compile again before querying SyncTeX")
        })?;
        if metadata
            .modified()
            .map_or(true, |modified| modified > recorded)
        {
            return Err(Error::invalid_params(
                "Project inputs changed; compile again before querying SyncTeX",
            ));
        }
        if let Some(doc) = workspace
            .documents
            .get(&input)
            .filter(|doc| doc.version.is_some())
        {
            if std::fs::read_to_string(path).ok().as_deref() != Some(doc.text.source.as_str()) {
                return Err(Error::invalid_params(
                    "Save project inputs and compile again before querying SyncTeX",
                ));
            }
        }
    }
    Ok(Project {
        root,
        pdf,
        directory,
        artifacts,
    })
}

fn forward(project: &Project, request: &ForwardRequest, cancel: &Cancellation) -> Result<Value> {
    let source = request
        .uri
        .to_file_path()
        .map_err(|_| Error::invalid_params("SyncTeX requires local files"))?;
    let text = std::fs::read_to_string(&source)
        .map_err(|_| Error::invalid_params("Save the source file before querying SyncTeX"))?;
    if crate::text::Text::new(text)
        .offset(request.position)
        .is_none()
    {
        return Err(Error::invalid_params("Invalid UTF-16 source position"));
    }
    // Engines normally record lines without reliable character columns. Use
    // SyncTeX's documented column=0 rather than passing a UTF-16 offset as bytes.
    let input = format!(
        "{}:0:{}",
        u64::from(request.position.line) + 1,
        source.display()
    );
    let output = query(
        project,
        &[
            "view".into(),
            "-i".into(),
            input,
            "-o".into(),
            project.pdf.to_string_lossy().into_owned(),
        ],
        cancel,
    )?;
    let locations = parse_forward(&output);
    Ok(
        json!({"rootUri":project.root,"pdfUri":Url::from_file_path(&project.pdf).ok(),"locations":locations}),
    )
}

fn inverse(project: &Project, request: &InverseRequest, cancel: &Cancellation) -> Result<Value> {
    if request.page == 0
        || !request.x.is_finite()
        || !request.y.is_finite()
        || request.x < 0.0
        || request.y < 0.0
    {
        return Err(Error::invalid_params(
            "Expected a one-based PDF page and finite nonnegative x/y coordinates",
        ));
    }
    let output = query(
        project,
        &[
            "edit".into(),
            "-o".into(),
            format!(
                "{}:{}:{}:{}",
                request.page,
                request.x,
                request.y,
                project.pdf.display()
            ),
        ],
        cancel,
    )?;
    let locations = parse_inverse(project, &output);
    Ok(
        json!({"rootUri":project.root,"pdfUri":Url::from_file_path(&project.pdf).ok(),"locations":locations}),
    )
}

fn query(project: &Project, arguments: &[String], cancel: &Cancellation) -> Result<String> {
    let mut command = Command::new("synctex");
    command
        .args(arguments)
        .current_dir(&project.directory)
        // The CLI otherwise executes commands from these environment variables.
        .env_remove("SYNCTEX_VIEWER")
        .env_remove("SYNCTEX_EDITOR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| {
        Error::invalid_params(format!(
            "Cannot start synctex: {error}; install the SyncTeX command-line tool"
        ))
    })?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout = std::thread::spawn(move || crate::build::capture(stdout));
    let stderr = std::thread::spawn(move || crate::build::capture(stderr));
    let started = Instant::now();
    let result = loop {
        if cancel.cancelled() {
            break Err(Error::request_cancelled());
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => {
                break Err(Error::invalid_params(format!(
                    "Cannot wait for synctex: {error}"
                )))
            }
            _ if started.elapsed() > Duration::from_secs(5) => {
                break Err(Error::invalid_params("SyncTeX query timed out"))
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    crate::build::kill_group(&mut child);
    let _ = child.wait();
    let output = stdout.join().unwrap_or_default();
    let errors = stderr.join().unwrap_or_default();
    let status = result?;
    if !status.success() {
        let message: String = String::from_utf8_lossy(&errors)
            .chars()
            .take(1024)
            .collect();
        return Err(Error::invalid_params(format!(
            "SyncTeX query failed ({status}): {message}"
        )));
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn records(output: &str) -> Vec<BTreeMap<&str, &str>> {
    let mut records = Vec::new();
    let mut record = BTreeMap::new();
    let mut active = false;
    for line in output.lines().take(32768) {
        if line == "SyncTeX result begin" {
            active = true;
        } else if line == "SyncTeX result end" {
            if !record.is_empty() {
                records.push(std::mem::take(&mut record));
            }
            active = false;
        } else if active {
            if let Some((key, value)) = line.split_once(':') {
                if matches!(key, "Output" | "Page" | "Input") && record.contains_key(key) {
                    records.push(std::mem::take(&mut record));
                }
                record.insert(key, value.trim());
            }
        }
        if records.len() >= 256 {
            break;
        }
    }
    // Ignore incomplete records if the process output was truncated.
    records
}

fn parse_forward(output: &str) -> Vec<Value> {
    records(output)
        .into_iter()
        .filter_map(|record| {
            let page: u32 = record.get("Page")?.parse().ok()?;
            let x: f64 = record.get("x")?.parse().ok()?;
            let y: f64 = record.get("y")?.parse().ok()?;
            if page == 0 || !x.is_finite() || !y.is_finite() {
                return None;
            }
            let mut result = json!({"page":page,"x":x,"y":y});
            for (key, field) in [("h", "h"), ("v", "v"), ("W", "width"), ("H", "height")] {
                if let Some(value) = record
                    .get(key)
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite())
                {
                    result[field] = json!(value);
                }
            }
            Some(result)
        })
        .collect()
}

fn parse_inverse(project: &Project, output: &str) -> Vec<Location> {
    let mut locations = Vec::new();
    for record in records(output) {
        let Some(line) = record
            .get("Line")
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|line| *line > 0)
        else {
            continue;
        };
        let Some(input) = record.get("Input") else {
            continue;
        };
        let path = project
            .directory
            .join(input)
            .canonicalize()
            .ok()
            .or_else(|| project.pdf.parent()?.join(input).canonicalize().ok());
        let Some(uri) = path
            .filter(|path| path.is_file())
            .and_then(|path| Url::from_file_path(path).ok())
        else {
            continue;
        };
        if uri
            .to_file_path()
            .ok()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .is_none_or(|source| line as usize > crate::text::Text::new(source).text_line_count())
        {
            continue;
        }
        let position = Position::new(line - 1, 0);
        let location = Location::new(uri, Range::new(position, position));
        if !locations.contains(&location) {
            locations.push(location);
        }
    }
    locations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_records_keep_multiple_hits_and_reject_malformed_or_incomplete_data() {
        let output = "banner\nSyncTeX result begin\nOutput:/tmp/a.pdf\nPage:1\nx:12.5\ny:34\nh:10\nv:40\nW:100\nH:9\nOutput:/tmp/a.pdf\nPage:2\nx:20\ny:45\nOutput:/tmp/a.pdf\nPage:0\nx:1\ny:2\nOutput:/tmp/a.pdf\nPage:3\nx:NaN\ny:2\nSyncTeX result end\nSyncTeX result begin\nPage:4\nx:1\ny:2\n";
        let hits = parse_forward(output);
        assert_eq!(
            hits,
            vec![
                json!({"page":1,"x":12.5,"y":34.0,"h":10.0,"v":40.0,"width":100.0,"height":9.0}),
                json!({"page":2,"x":20.0,"y":45.0})
            ]
        );
    }

    #[test]
    fn inverse_records_resolve_relative_paths_deduplicate_and_validate_source_lines() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("chapter 中文.tex"), "First\nSecond\n").unwrap();
        let project = Project {
            root: Url::from_file_path(directory.path().join("main.tex")).unwrap(),
            pdf: directory.path().join("main.pdf"),
            directory: directory.path().into(),
            artifacts: Vec::new(),
        };
        let output = "SyncTeX result begin\nInput:./chapter 中文.tex\nLine:2\nColumn:15\nInput:chapter 中文.tex\nLine:2\nInput:missing.tex\nLine:1\nInput:chapter 中文.tex\nLine:999\nSyncTeX result end\n";
        let hits = parse_inverse(&project, output);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].uri,
            Url::from_file_path(directory.path().join("chapter 中文.tex")).unwrap()
        );
        assert_eq!(
            hits[0].range,
            Range::new(Position::new(1, 0), Position::new(1, 0))
        );
    }

    #[test]
    fn prepare_requires_artifacts_saved_sources_and_fresh_recording() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.tex");
        let uri = Url::from_file_path(&path).unwrap();
        std::fs::write(&path, "\\documentclass{article}\n").unwrap();
        let mut workspace = Workspace::default();
        workspace.add_root(directory.path().into());
        assert!(prepare(&workspace, &uri).is_err());
        let output = directory.path().join("build/latex-lsp");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("main.pdf"), "%PDF").unwrap();
        std::fs::write(output.join("main.synctex.gz"), "SyncTeX data").unwrap();
        let recorded = SystemTime::now() + Duration::from_secs(10);
        std::fs::File::open(output.join("main.synctex.gz"))
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(recorded))
            .unwrap();
        assert!(prepare(&workspace, &uri).is_ok());
        workspace.open(uri.clone(), "Unsaved".into(), 1);
        assert!(prepare(&workspace, &uri)
            .unwrap_err()
            .message
            .contains("Save"));
        workspace.documents.remove(&uri);
        workspace.reload(&uri);
        std::fs::File::open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(recorded + Duration::from_secs(1)))
            .unwrap();
        assert!(prepare(&workspace, &uri)
            .unwrap_err()
            .message
            .contains("changed"));
    }

    #[test]
    fn prepare_checks_static_and_recorded_images_without_indexing_binary_data() {
        for recorded_image in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            let uri = Url::from_file_path(root.join("main.tex")).unwrap();
            let image = root.join("plot.png");
            std::fs::write(&image, [0, 255, 1]).unwrap();
            let source = if recorded_image {
                "\\documentclass{article}\n"
            } else {
                "\\documentclass{article}\n\\includegraphics{plot}\n"
            };
            std::fs::write(root.join("main.tex"), source).unwrap();
            let mut workspace = Workspace::default();
            workspace.add_root(root.into());
            let image_uri = Url::from_file_path(&image).unwrap();
            if recorded_image {
                workspace
                    .recorded_inputs
                    .insert(uri.clone(), [uri.clone(), image_uri.clone()].into());
                workspace.refresh_project(&uri);
            }
            assert!(!workspace.documents.contains_key(&image_uri));
            let output = root.join("build/latex-lsp");
            std::fs::create_dir_all(&output).unwrap();
            std::fs::write(output.join("main.pdf"), "%PDF").unwrap();
            std::fs::write(output.join("main.synctex.gz"), "SyncTeX data").unwrap();
            let recorded = SystemTime::now() + Duration::from_secs(10);
            std::fs::File::open(output.join("main.synctex.gz"))
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(recorded))
                .unwrap();
            assert!(prepare(&workspace, &uri).is_ok());
            std::fs::File::open(&image)
                .unwrap()
                .set_times(
                    std::fs::FileTimes::new().set_modified(recorded + Duration::from_secs(1)),
                )
                .unwrap();
            assert!(prepare(&workspace, &uri)
                .unwrap_err()
                .message
                .contains("changed"));
            if recorded_image {
                std::fs::remove_file(&image).unwrap();
                assert!(prepare(&workspace, &uri)
                    .unwrap_err()
                    .message
                    .contains("changed"));
            }
        }
    }
}
