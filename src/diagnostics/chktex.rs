//! Check the editor snapshot over stdin; never follow input files or write source.
use crate::rpc::{Error, Result};
use crate::{
    config::ChktexConfig,
    process::tasks::{Cancellation, Task, Tasks},
    text::Text,
};
use lsp_types::*;
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) fn start(
    tasks: &Tasks,
    config: ChktexConfig,
    source: String,
    directory: Option<PathBuf>,
) -> Result<Task<Vec<Diagnostic>>> {
    tasks.start(move |cancel| run(&config, source, directory, cancel))
}

fn run(
    config: &ChktexConfig,
    source: String,
    directory: Option<PathBuf>,
    cancel: Cancellation,
) -> Result<Vec<Diagnostic>> {
    let mut command = Command::new(&config.executable);
    // Extra arguments can select warnings and rc files. Protocol switches last
    // ensure the parser continues to receive the expected, snapshot-only output.
    command
        .args(&config.arguments)
        .args(["-q", "-I0", "-S", "TabSize=1", "-f%l:%c:%d:%k:%n:%m\n"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|err| Error::invalid_params(format!("Cannot start ChkTeX: {err}")))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let text = Text::new(source.clone());
    let writer = std::thread::spawn(move || stdin.write_all(source.as_bytes()));
    let output = std::thread::spawn(move || {
        let mut data = Vec::new();
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map(|_| data)
    });
    let errors = std::thread::spawn(move || {
        let mut data = Vec::new();
        stderr.take(64 * 1024).read_to_end(&mut data).map(|_| data)
    });
    let started = Instant::now();
    let status = loop {
        if cancel.cancelled() {
            break Err(Error::request_cancelled());
        }
        if started.elapsed() > Duration::from_secs(config.timeout_seconds) {
            break Err(Error::invalid_params("ChkTeX timed out"));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(err) => {
                break Err(Error::invalid_params(format!(
                    "Cannot wait for ChkTeX: {err}"
                )))
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    crate::process::kill_group(&mut child);
    let _ = child.wait();
    let sent = writer.join();
    let output = output.join().ok().and_then(|value| value.ok());
    let errors = errors
        .join()
        .ok()
        .and_then(|value| value.ok())
        .unwrap_or_default();
    let status = status?;
    // ChkTeX returns 1/2/3 for warnings/errors, including valid diagnostics.
    if !matches!(status.code(), Some(0..=3)) {
        return Err(Error::invalid_params(format!(
            "ChkTeX failed: {}",
            String::from_utf8_lossy(&errors)
                .chars()
                .take(1024)
                .collect::<String>()
        )));
    }
    sent.map_err(|_| Error::invalid_params("Cannot send ChkTeX text"))?
        .map_err(|err| Error::invalid_params(err.to_string()))?;
    let output = output
        .filter(|data| data.len() <= 1024 * 1024)
        .ok_or_else(|| Error::invalid_params("ChkTeX output exceeds 1 MiB or cannot be read"))?;
    let output = String::from_utf8(output)
        .map_err(|_| Error::invalid_params("ChkTeX returned invalid UTF-8"))?;
    if output.trim().is_empty() && !status.success() && !errors.is_empty() {
        return Err(Error::invalid_params(
            String::from_utf8_lossy(&errors)
                .chars()
                .take(1024)
                .collect::<String>(),
        ));
    }
    Ok(parse(&text, &output))
}

fn parse(text: &Text, output: &str) -> Vec<Diagnostic> {
    let lines: Vec<_> = text
        .source
        .split_inclusive('\n')
        .scan(0, |offset, line| {
            let current = *offset;
            *offset += line.len();
            Some((current, line.trim_end_matches(['\r', '\n'])))
        })
        .collect();
    output
        .lines()
        .take(4096)
        .filter_map(|line| {
            let fields: Vec<_> = line.splitn(6, ':').collect();
            if fields.len() != 6 {
                return None;
            }
            let line = fields[0].parse::<usize>().ok()?.checked_sub(1)?;
            let column = fields[1].parse::<usize>().ok()?.checked_sub(1)?;
            let length = fields[2].parse::<usize>().ok()?;
            let (offset, content) = *lines.get(line)?;
            if column > content.len() {
                return None;
            }
            let mut start = column;
            while !content.is_char_boundary(start) {
                start -= 1;
            }
            let mut end = column.saturating_add(length).min(content.len());
            while !content.is_char_boundary(end) {
                end += 1;
            }
            Some(Diagnostic {
                range: text.range(offset + start, offset + end),
                source: Some("chktex".into()),
                code: Some(NumberOrString::String(fields[4].into())),
                severity: Some(match fields[3] {
                    "Error" => DiagnosticSeverity::ERROR,
                    "Message" => DiagnosticSeverity::INFORMATION,
                    _ => DiagnosticSeverity::WARNING,
                }),
                message: fields[5].into(),
                ..Default::default()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chktex_byte_columns_are_converted_to_utf16_and_bad_lines_are_ignored() {
        let text = Text::new("😀 hello...\r\nnext\n".into());
        let result = parse(
            &text,
            "1:11:3:Warning:11:Use dots: here\n0:1:1:Warning:1:bad\n2:99:1:Warning:1:bad\nnoise\n",
        );
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].range,
            Range::new(Position::new(0, 8), Position::new(0, 11))
        );
        assert_eq!(result[0].message, "Use dots: here");
    }
}
