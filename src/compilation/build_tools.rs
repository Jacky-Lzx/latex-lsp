//! Shared lifecycle for explicit build-tool stages.
use crate::compilation::file_checks::InputSnapshot;
use lsp_types::Url;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub(crate) struct Failure {
    pub status: &'static str,
    pub exit_code: Option<i32>,
    pub message: String,
    pub output: String,
}
impl Failure {
    pub(crate) fn error(message: String) -> Self {
        Self {
            status: "error",
            exit_code: None,
            message,
            output: String::new(),
        }
    }
}
#[derive(Clone)]
pub(crate) struct Watched {
    path: PathBuf,
    target: PathBuf,
    contents: InputSnapshot,
}
impl Watched {
    pub(crate) fn snapshot(&self) -> Option<(Url, InputSnapshot)> {
        Some((
            Url::from_file_path(&self.target).ok()?,
            self.contents.clone(),
        ))
    }
    pub(crate) fn matches_path(&self, path: &Path) -> bool {
        let path = crate::project::workspace::normalize_path(path);
        path == crate::project::workspace::normalize_path(&self.path) || path == self.target
    }
    pub(crate) fn capture(path: &Path) -> Result<Self, Failure> {
        let target = path
            .canonicalize()
            .map_err(|error| Failure::error(format!("Cannot read {}: {error}", path.display())))?;
        let contents = InputSnapshot::file(path);
        if matches!(contents, InputSnapshot::File(None)) {
            return Err(Failure::error(format!(
                "Build tool input is not a readable regular file: {}",
                path.display()
            )));
        }
        Ok(Self {
            path: path.to_owned(),
            target,
            contents,
        })
    }
    pub(crate) fn unchanged(&self) -> bool {
        self.path.canonicalize().ok().as_ref() == Some(&self.target)
            && Url::from_file_path(&self.path).is_ok_and(|uri| !self.contents.changed(&uri))
    }
}
pub(crate) fn cancelled(executable: &str) -> Failure {
    Failure {
        status: "cancelled",
        exit_code: None,
        message: format!("{executable} cancelled"),
        output: String::new(),
    }
}
pub(crate) fn execute(
    mut command: Command,
    executable: &str,
    timeout_seconds: u64,
    cancel: &AtomicBool,
) -> Result<String, Failure> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled(executable));
    }
    command
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
                    "Cannot start {executable}: {error}"
                )))
            }
        }
    };
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout = std::thread::spawn(move || crate::compilation::build::capture(stdout));
    let stderr = std::thread::spawn(move || crate::compilation::build::capture(stderr));
    let started = Instant::now();
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(cancelled(executable));
        }
        if started.elapsed() >= Duration::from_secs(timeout_seconds) {
            break Err(Failure {
                status: "timeout",
                exit_code: None,
                message: format!("{} timed out", executable),
                output: String::new(),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(status)) => {
                break Err(Failure {
                    status: "error",
                    exit_code: status.code(),
                    message: format!("{} exited with {status}", executable),
                    output: String::new(),
                })
            }
            Err(error) => {
                break Err(Failure::error(format!(
                    "Cannot wait for {}: {error}",
                    executable
                )))
            }
            _ => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    crate::process::kill_group(&mut child);
    let _ = child.wait();
    let output = format!(
        "{}\n{}",
        crate::compilation::build_log::decode_output(&stdout.join().unwrap_or_default()),
        crate::compilation::build_log::decode_output(&stderr.join().unwrap_or_default())
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
