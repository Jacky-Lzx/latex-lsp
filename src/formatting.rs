//! Run formatters against editor text without writing the source file.
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::{FormattingOptions, TextEdit},
};

use crate::{
    config::{Formatter, FormattingConfig},
    text::Text,
};

pub(crate) fn start(
    tasks: &crate::tasks::Tasks,
    config: FormattingConfig,
    options: FormattingOptions,
    source: String,
    directory: Option<PathBuf>,
) -> Result<crate::tasks::Task<String>> {
    validate_options(&options)?;
    tasks.start(move |cancel| run(&config, &options, source, directory, || cancel.cancelled()))
}

pub(crate) fn validate_options(options: &FormattingOptions) -> Result<()> {
    if !(1..=256).contains(&options.tab_size) {
        return Err(Error::invalid_params(
            "Formatting tabSize must be between 1 and 256",
        ));
    }
    Ok(())
}

pub(crate) fn failure(message: impl Into<String>) -> Error {
    Error {
        code: tower_lsp::jsonrpc::ErrorCode::ServerError(-32803),
        message: message.into().into(),
        data: None,
    }
}

fn arguments(formatter: Formatter, options: &FormattingOptions) -> Vec<String> {
    match formatter {
        Formatter::Latexindent => {
            let indent = if options.insert_spaces {
                " ".repeat(options.tab_size as usize)
            } else {
                "\t".into()
            };
            vec![
                "-l".into(),
                format!(
                    "-y=defaultIndent: {}",
                    serde_json::to_string(&indent).unwrap()
                ),
                "-".into(),
            ]
        }
        Formatter::TexFmt => {
            let mut arguments = vec![
                "--stdin".into(),
                "--tabsize".into(),
                options.tab_size.to_string(),
            ];
            if !options.insert_spaces {
                arguments.push("--usetabs".into());
            }
            arguments
        }
    }
}

fn capture(reader: impl Read, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut output)?;
    Ok(output)
}

fn run(
    config: &FormattingConfig,
    options: &FormattingOptions,
    source: String,
    directory: Option<PathBuf>,
    cancelled: impl Fn() -> bool,
) -> Result<String> {
    let nonempty = !source.trim().is_empty();
    if cancelled() {
        return Err(Error::request_cancelled());
    }
    let executable = config
        .executable
        .as_deref()
        .unwrap_or(match config.formatter {
            Formatter::Latexindent => "latexindent",
            Formatter::TexFmt => "tex-fmt",
        });
    // latexindent can create logs and other auxiliary files even with stdin.
    // Keep them in a private directory that survives until the process is reaped.
    let scratch = if config.formatter == Formatter::Latexindent && config.arguments.is_none() {
        Some(tempfile::tempdir().map_err(|error| {
            failure(format!(
                "Cannot create formatter temporary directory: {error}"
            ))
        })?)
    } else {
        None
    };
    let mut args = config
        .arguments
        .clone()
        .unwrap_or_else(|| arguments(config.formatter, options));
    if let Some(scratch) = &scratch {
        args.insert(
            0,
            format!("-g={}", scratch.path().join("indent.log").display()),
        );
        args.insert(0, format!("-c={}", scratch.path().display()));
    }
    let mut command = Command::new(executable);
    command
        .args(args)
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
    let mut child = command.spawn().map_err(|error| failure(format!("Cannot start formatter {executable}: {error}; install it or configure formatting.executable")))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let input = std::thread::spawn(move || stdin.write_all(source.as_bytes()));
    const LIMIT: usize = 16 * 1024 * 1024;
    let output = std::thread::spawn(move || capture(stdout, LIMIT));
    let errors = std::thread::spawn(move || capture(stderr, 64 * 1024));
    let started = Instant::now();
    let result = loop {
        if cancelled() {
            break Err(Error::request_cancelled());
        }
        if started.elapsed() >= Duration::from_secs(config.timeout_seconds) {
            break Err(failure(format!("Formatter {executable} timed out")));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => break Err(failure(format!("Cannot wait for formatter: {error}"))),
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    crate::build::kill_group(&mut child);
    let _ = child.wait();
    let input = input.join();
    let output = output.join();
    let errors = errors
        .join()
        .ok()
        .and_then(|result| result.ok())
        .unwrap_or_default();
    let status = result?;
    let output = output
        .map_err(|_| failure("Cannot capture formatter output"))?
        .map_err(|error| failure(format!("Cannot read formatter output: {error}")))?;
    if output.len() > LIMIT {
        return Err(failure("Formatter output exceeds 16 MiB"));
    }
    if !status.success() {
        let message: String = String::from_utf8_lossy(&errors)
            .chars()
            .take(1024)
            .collect();
        return Err(failure(format!(
            "Formatter {executable} failed ({status}): {message}"
        )));
    }
    input
        .map_err(|_| failure("Cannot send text to formatter"))?
        .map_err(|error| failure(format!("Cannot send text to formatter: {error}")))?;
    if nonempty && output.is_empty() {
        return Err(failure(
            "Formatter returned empty output for a nonempty document",
        ));
    }
    String::from_utf8(output).map_err(|_| failure("Formatter returned invalid UTF-8"))
}

/// One contiguous edit, excluding the unchanged prefix/suffix, on UTF-16 and CRLF boundaries.
pub fn edits(text: &Text, formatted: String) -> Vec<TextEdit> {
    let formatted = formatted
        .replace("\r\n", "\n")
        .replace('\n', text.newline());
    replacement(text, 0..text.source.len(), &formatted)
        .into_iter()
        .collect()
}

/// A minimal edit for an already newline-normalized replacement.
pub(crate) fn replacement(
    text: &Text,
    span: std::ops::Range<usize>,
    formatted: &str,
) -> Option<TextEdit> {
    let source = &text.source[span.clone()];
    if source == formatted {
        return None;
    }
    let mut start: usize = source
        .chars()
        .zip(formatted.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    if source.as_bytes().get(start) == Some(&b'\n')
        && start > 0
        && source.as_bytes()[start - 1] == b'\r'
    {
        start -= 1;
    }
    let suffix: usize = source[start..]
        .chars()
        .rev()
        .zip(formatted[start..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum();
    let mut end = source.len() - suffix;
    let mut new_end = formatted.len() - suffix;
    if source.as_bytes().get(end) == Some(&b'\n') && end > 0 && source.as_bytes()[end - 1] == b'\r'
    {
        end += 1;
        new_end += 1;
    }
    Some(TextEdit {
        range: text.range(span.start + start, span.start + end),
        new_text: formatted[start..new_end].into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use serde_json::json;
    use tower_lsp::lsp_types::TextDocumentContentChangeEvent;

    #[test]
    fn configuration_defaults_to_tex_fmt_and_validates_overrides() {
        let config = Config::parse(json!({})).unwrap().formatting;
        assert_eq!(config.formatter, Formatter::TexFmt);
        let options = FormattingOptions {
            tab_size: 4,
            insert_spaces: false,
            ..Default::default()
        };
        assert_eq!(
            arguments(config.formatter, &options),
            ["--stdin", "--tabsize", "4", "--usetabs"]
        );
        let config = Config::parse(json!({"latex":{"formatting":{"formatter":"latexindent","executable":"/tools/latexindent","arguments":["-","-m"],"timeoutSeconds":3}}})).unwrap().formatting;
        assert_eq!(config.formatter, Formatter::Latexindent);
        assert_eq!(config.arguments.unwrap(), ["-", "-m"]);
        assert!(arguments(Formatter::Latexindent, &options)
            .contains(&"-y=defaultIndent: \"\\t\"".into()));
        for value in [
            json!({"formatter":"unknown"}),
            json!({"timeoutSeconds":0}),
            json!({"timeoutSeconds":3601}),
            json!({"executable":" "}),
            json!({"unknown":true}),
        ] {
            assert!(Config::parse(json!({"formatting":value})).is_err());
        }
    }

    #[test]
    fn edits_preserve_unicode_line_endings_and_unchanged_text() {
        for (source, formatted, expected) in [
            ("😀中 a\r\n尾\r\n", "😀中  a\n尾\n", "😀中  a\r\n尾\r\n"),
            ("中\r\n", "中\n\n", "中\r\n\r\n"),
            ("中\r\n\r\n", "中\n", "中\r\n"),
            ("😀x", "😀", "😀"),
            ("", "中\n", "中\n"),
        ] {
            let text = Text::new(source.into());
            let edits = edits(&text, formatted.into());
            assert_eq!(edits.len(), 1);
            let changes = edits
                .into_iter()
                .map(|edit| TextDocumentContentChangeEvent {
                    range: Some(edit.range),
                    range_length: None,
                    text: edit.new_text,
                })
                .collect();
            assert_eq!(text.changed(changes).unwrap().source, expected);
        }
        assert!(edits(&Text::new("中\r\n".into()), "中\n".into()).is_empty());
    }
}
