//! Explicit index/glossary stages owned by the normal build task.
use crate::{
    config::{BuildConfig, IndexToolConfig},
    file_checks::InputSnapshot,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tower_lsp::lsp_types::Url;

pub(crate) struct Failure {
    pub status: &'static str,
    pub exit_code: Option<i32>,
    pub message: String,
    pub output: String,
}
impl Failure {
    fn error(message: String) -> Self {
        Self {
            status: "error",
            exit_code: None,
            message,
            output: String::new(),
        }
    }
}
#[derive(Clone)]
struct Watched {
    path: PathBuf,
    target: PathBuf,
    contents: InputSnapshot,
}
impl Watched {
    fn capture(path: &Path) -> Result<Self, Failure> {
        let target = path
            .canonicalize()
            .map_err(|error| Failure::error(format!("Cannot read {}: {error}", path.display())))?;
        let contents = InputSnapshot::file(path);
        if matches!(contents, InputSnapshot::File(None)) {
            return Err(Failure::error(format!(
                "Index input is not a readable regular file: {}",
                path.display()
            )));
        }
        Ok(Self {
            path: path.to_owned(),
            target,
            contents,
        })
    }
    fn unchanged(&self) -> bool {
        self.path.canonicalize().ok().as_ref() == Some(&self.target)
            && Url::from_file_path(&self.path).is_ok_and(|uri| !self.contents.changed(&uri))
    }
}
struct Stage {
    config: IndexToolConfig,
    input: PathBuf,
    output: PathBuf,
    dependencies: Vec<Watched>,
}
pub(crate) struct Plan {
    root: PathBuf,
    output: PathBuf,
    stages: Vec<Stage>,
}
pub(crate) struct Processed {
    pub output: String,
    pub tools: Vec<serde_json::Value>,
    inputs: Vec<Watched>,
}
impl Processed {
    pub(crate) fn inputs_unchanged(&self) -> bool {
        self.inputs.iter().all(Watched::unchanged)
    }
}

fn artifact(
    root: &Path,
    build: &BuildConfig,
    tool: &IndexToolConfig,
    extension: &str,
) -> Option<PathBuf> {
    let mut selected = build.clone();
    if tool.job_name.is_some() {
        selected.job_name.clone_from(&tool.job_name);
    }
    selected.artifact_path(root, extension)
}
fn transcript_path(output: &Path, tool: &IndexToolConfig) -> PathBuf {
    output.with_extension(match tool.input_extension.as_str() {
        "glo" => "glg",
        "acn" => "alg",
        "nlo" => "nlg",
        _ => "ilg",
    })
}
pub(crate) fn outputs(root: &Path, config: &BuildConfig) -> Vec<PathBuf> {
    config
        .index_tools
        .iter()
        .flat_map(|tool| {
            artifact(root, config, tool, &tool.output_extension)
                .into_iter()
                .flat_map(|path| {
                    let log = tool.is_makeindex().then(|| transcript_path(&path, tool));
                    [Some(path), log].into_iter().flatten()
                })
        })
        .collect()
}

pub(crate) fn dependencies(root: &Path, config: &BuildConfig) -> Vec<PathBuf> {
    let Some(parent) = root.parent() else {
        return vec![];
    };
    config
        .index_tools
        .iter()
        .flat_map(|tool| {
            tool.dependencies
                .iter()
                .map(|dependency| parent.join(dependency))
        })
        .collect()
}

impl Plan {
    pub(crate) fn prepare(root: &Path, build: &BuildConfig) -> Result<Self, Failure> {
        let output = build
            .output_path(root)
            .ok_or_else(|| Failure::error("Invalid build output directory".into()))?;
        let mut names = BTreeSet::new();
        let mut stages = vec![];
        for config in &build.index_tools {
            config.validate().map_err(Failure::error)?;
            let input = artifact(root, build, config, &config.input_extension)
                .ok_or_else(|| Failure::error("Invalid index input name".into()))?;
            let output = artifact(root, build, config, &config.output_extension)
                .ok_or_else(|| Failure::error("Invalid index output name".into()))?;
            if !names.insert(crate::workspace::normalize_path(&output)) {
                return Err(Failure::error(format!(
                    "Multiple index tools write {}",
                    output.display()
                )));
            }
            let dependencies = config
                .dependencies
                .iter()
                .map(|dependency| Watched::capture(&root.parent().unwrap().join(dependency)))
                .collect::<Result<_, _>>()?;
            stages.push(Stage {
                config: config.clone(),
                input,
                output,
                dependencies,
            });
        }
        Ok(Self {
            root: root.to_owned(),
            output,
            stages,
        })
    }
    pub(crate) fn run(
        &self,
        build: &BuildConfig,
        cancel: &AtomicBool,
    ) -> Result<Processed, Failure> {
        let mut output = String::new();
        let mut tools = vec![];
        let mut inputs = vec![];
        for stage in &self.stages {
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            let input = Watched::capture(&stage.input)?;
            if stage
                .dependencies
                .iter()
                .any(|dependency| !dependency.unchanged())
            {
                return Err(Failure::error(
                    "Index tool dependencies changed during compilation; build again".into(),
                ));
            }
            if std::fs::symlink_metadata(&stage.output)
                .is_ok_and(|metadata| !metadata.file_type().is_file())
            {
                return Err(Failure::error(format!(
                    "Index output must be a regular file: {}",
                    stage.output.display()
                )));
            }
            let temporary = staged_path(&self.output, &stage.config.output_extension)?;
            let transcript = staged_path(&self.output, "ilg")?;
            let mut selected = build.clone();
            if stage.config.job_name.is_some() {
                selected.job_name.clone_from(&stage.config.job_name);
            }
            let job = selected.job_name(&self.root).unwrap().to_string_lossy();
            let root = self.root.to_string_lossy();
            let parent = self.root.parent().unwrap().to_string_lossy();
            let directory = self.output.to_string_lossy();
            let relative_input = Path::new(".").join(stage.input.file_name().unwrap());
            let input_name = relative_input.to_string_lossy();
            let temporary_name = temporary.file_name().unwrap().to_string_lossy();
            let transcript_name = transcript.file_name().unwrap().to_string_lossy();
            let substitutions = [
                ('l', transcript_name.as_ref()),
                ('f', root.as_ref()),
                ('r', parent.as_ref()),
                ('o', directory.as_ref()),
                ('j', job.as_ref()),
                ('i', input_name.as_ref()),
                ('t', temporary_name.as_ref()),
            ];
            let arguments = stage
                .config
                .arguments
                .iter()
                .map(|argument| expand(argument, &substitutions))
                .collect::<Vec<_>>();
            let stage_output = execute(&stage.config, &arguments, &self.output, cancel)?;
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            if !input.unchanged()
                || stage
                    .dependencies
                    .iter()
                    .any(|dependency| !dependency.unchanged())
            {
                return Err(Failure::error(format!(
                    "Index input or dependencies changed while processing {}; build again",
                    stage.input.display()
                )));
            }
            if !std::fs::symlink_metadata(&temporary)
                .is_ok_and(|metadata| metadata.file_type().is_file())
            {
                let mut failure = Failure::error(format!(
                    "{} exited successfully without producing the requested index output",
                    stage.config.executable
                ));
                failure.output = stage_output;
                return Err(failure);
            }
            let transcript_output = transcript_path(&stage.output, &stage.config);
            if stage.config.is_makeindex() {
                let report = std::fs::symlink_metadata(&transcript)
                    .is_ok_and(|metadata| metadata.file_type().is_file())
                    .then(|| std::fs::File::open(&transcript).ok())
                    .flatten()
                    .map(crate::build::capture)
                    .map(|bytes| crate::build_log::decode_output(&bytes).into_owned());
                let rejected = report.as_deref().and_then(rejected_entries);
                if rejected != Some(0) {
                    let mut failure = Failure::error(rejected.map_or_else(
                        || "makeindex did not produce a complete processing transcript".into(),
                        |count| {
                            format!("makeindex rejected {count} index entries; see build output")
                        },
                    ));
                    failure.output = format!("{}\n{}", stage_output, report.unwrap_or_default());
                    return Err(failure);
                }
            }
            temporary
                .persist(&stage.output)
                .map_err(|error| Failure::error(format!("Cannot install index output: {error}")))?;
            if stage.config.is_makeindex() {
                transcript.persist(&transcript_output).map_err(|error| {
                    Failure::error(format!("Cannot install index transcript: {error}"))
                })?;
            }
            output.push_str(&format!(
                "\n{} {}\n{}",
                stage.config.executable,
                stage.input.display(),
                stage_output
            ));
            inputs.push(input);
            inputs.extend(stage.dependencies.iter().cloned());
            tools.push(serde_json::json!({"executable":stage.config.executable,"inputUri":Url::from_file_path(&stage.input).ok(),"outputUri":Url::from_file_path(&stage.output).ok(),"transcriptUri":stage.config.is_makeindex().then(|| Url::from_file_path(&transcript_output).ok()).flatten()}));
        }
        Ok(Processed {
            output,
            tools,
            inputs,
        })
    }
}
fn staged_path(directory: &Path, extension: &str) -> Result<tempfile::TempPath, Failure> {
    let path = tempfile::Builder::new()
        .prefix("latex-lsp-index-")
        .suffix(&format!(".{extension}"))
        .tempfile_in(directory)
        .map_err(|error| Failure::error(format!("Cannot stage index output: {error}")))?
        .into_temp_path();
    std::fs::remove_file(&path)
        .map_err(|error| Failure::error(format!("Cannot prepare index output: {error}")))?;
    Ok(path)
}
fn rejected_entries(report: &str) -> Option<u64> {
    let counts: Option<Vec<u64>> = report
        .lines()
        .filter(|line| line.starts_with("Scanning input file ") || line.starts_with("done ("))
        .filter_map(|line| line.rsplit_once("done (").map(|(_, summary)| summary))
        .map(|summary| {
            summary
                .split_once(" entries accepted, ")?
                .1
                .strip_suffix(" rejected).")?
                .parse()
                .ok()
        })
        .collect();
    let counts = counts?;
    if counts.is_empty() {
        return None;
    }
    counts.into_iter().try_fold(0u64, u64::checked_add)
}

fn cancelled() -> Failure {
    Failure {
        status: "cancelled",
        exit_code: None,
        message: "Index processing cancelled".into(),
        output: String::new(),
    }
}
fn expand(template: &str, substitutions: &[(char, &str)]) -> String {
    let mut expanded = String::new();
    let mut tail = template;
    while let Some(at) = tail.find('%') {
        expanded.push_str(&tail[..at]);
        let marker = tail.as_bytes().get(at + 1).copied().map(char::from);
        if let Some((_, value)) = substitutions.iter().find(|(key, _)| Some(*key) == marker) {
            expanded.push_str(value);
            tail = &tail[at + 2..];
        } else {
            expanded.push('%');
            tail = &tail[at + 1..];
        }
    }
    expanded.push_str(tail);
    expanded
}
fn execute(
    config: &IndexToolConfig,
    arguments: &[String],
    directory: &Path,
    cancel: &AtomicBool,
) -> Result<String, Failure> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    let mut command = Command::new(&config.executable);
    command
        .args(arguments)
        .current_dir(directory)
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
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy && retries < 3 => {
                std::thread::sleep(Duration::from_millis(10 << retries));
                retries += 1;
            }
            Err(error) => {
                return Err(Failure::error(format!(
                    "Cannot start {}: {error}",
                    config.executable
                )))
            }
        }
    };
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout = std::thread::spawn(move || crate::build::capture(stdout));
    let stderr = std::thread::spawn(move || crate::build::capture(stderr));
    let started = Instant::now();
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(cancelled());
        }
        if started.elapsed() >= Duration::from_secs(config.timeout_seconds) {
            break Err(Failure {
                status: "timeout",
                exit_code: None,
                message: format!("{} timed out", config.executable),
                output: String::new(),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(status)) => {
                break Err(Failure {
                    status: "error",
                    exit_code: status.code(),
                    message: format!("{} exited with {status}", config.executable),
                    output: String::new(),
                })
            }
            Err(error) => {
                break Err(Failure::error(format!(
                    "Cannot wait for {}: {error}",
                    config.executable
                )))
            }
            _ => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    crate::build::kill_group(&mut child);
    let _ = child.wait();
    let output = format!(
        "{}\n{}",
        crate::build_log::decode_output(&stdout.join().unwrap_or_default()),
        crate::build_log::decode_output(&stderr.join().unwrap_or_default())
    );
    let output: String = output
        .chars()
        .rev()
        .take(8192)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    result.map(|()| output.clone()).map_err(|mut failure| {
        failure.output = output;
        failure
    })
}
