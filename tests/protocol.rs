use std::{
    cell::RefCell,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use serde_json::{json, Value};
use tower_lsp::lsp_types::Url;

struct Server {
    child: Child,
    stdin: ChildStdin,
    messages: mpsc::Receiver<Value>,
    pending: RefCell<Vec<Value>>,
}

impl Server {
    fn start() -> Self {
        Self::start_with_env(&[])
    }

    fn start_with_env(environment: &[(&str, &str)]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_latex-lsp"))
            .arg("--stdio")
            .envs(environment.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut length = None;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 {
                        return;
                    }
                    if header == "\r\n" {
                        break;
                    }
                    if let Some(value) = header.strip_prefix("Content-Length:") {
                        length = value.trim().parse::<usize>().ok();
                    }
                }
                let Some(length) = length else {
                    return;
                };
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                if sender.send(serde_json::from_slice(&body).unwrap()).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin,
            messages,
            pending: RefCell::new(Vec::new()),
        }
    }

    fn send(&mut self, message: Value) {
        let mut message = message;
        // Methods with no parameters omit the field, as required by the protocol.
        if message.get("params") == Some(&Value::Null) {
            message.as_object_mut().unwrap().remove("params");
        }
        let bytes = serde_json::to_vec(&message).unwrap();
        write!(self.stdin, "Content-Length: {}\r\n\r\n", bytes.len()).unwrap();
        self.stdin.write_all(&bytes).unwrap();
        self.stdin.flush().unwrap();
    }

    fn until(&self, matches: impl Fn(&Value) -> bool) -> Value {
        let existing = self.pending.borrow().iter().position(&matches);
        if let Some(index) = existing {
            return self.pending.borrow_mut().remove(index);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("LSP server timed out or closed stdout");
            if matches(&message) {
                return message;
            }
            assert!(message.get("error").is_none(), "LSP error: {message}");
            self.pending.borrow_mut().push(message);
        }
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}));
        let message = self.until(|message| message["id"] == id);
        assert!(message.get("error").is_none(), "LSP error: {message}");
        message["result"].clone()
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc":"2.0", "method":method, "params":params}));
    }

    fn wait_exit(&mut self, expected: i32) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(expected));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "server did not exit after exit notification"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(unix)]
fn fake_build_server(script: &str, on_save: bool) -> (Server, tempfile::TempDir, Url) {
    fake_build_server_capabilities(script, on_save, json!({}))
}

#[cfg(unix)]
fn fake_build_server_capabilities(
    script: &str,
    on_save: bool,
    capabilities: Value,
) -> (Server, tempfile::TempDir, Url) {
    let directory = tempfile::tempdir().unwrap();
    let root = Url::from_directory_path(directory.path()).unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let source = "\\documentclass{article}\n\\section{Title}\n";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    let executable = directory.path().join("compiler.sh");
    std::fs::write(&executable, script).unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":root,"capabilities":capabilities,"initializationOptions":{"build":{"executable":"/bin/sh","arguments":[executable,"%o","%f"],"onSave":on_save,"timeoutSeconds":5}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":source}}),
    );
    server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    (server, directory, uri)
}

#[test]
#[cfg(unix)]
fn unchanged_pdf_older_than_an_included_source_is_rejected_but_fresh_noop_builds_succeed() {
    let (mut server, directory, uri) = fake_build_server("exit 0\n", false);
    let source = "\\documentclass{article}\n\\input{chapter}\n";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    std::fs::write(directory.path().join("chapter.tex"), "Chapter\n").unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":source}]}),
    );
    let output = directory.path().join("build/latex-lsp");
    std::fs::create_dir_all(&output).unwrap();
    let pdf = output.join("main.pdf");
    std::fs::write(&pdf, "%PDF-1.4\n").unwrap();
    let now = std::time::SystemTime::now();
    std::fs::File::open(uri.to_file_path().unwrap())
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(now - Duration::from_secs(120)))
        .unwrap();
    std::fs::File::open(&pdf)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(now - Duration::from_secs(60)))
        .unwrap();
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let outdated = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(outdated["params"]["status"], "error", "{outdated}");
    assert_eq!(outdated["params"]["stale"], false);
    assert!(outdated["params"]["pdfUri"].is_null());
    assert!(outdated["params"]["message"]
        .as_str()
        .unwrap()
        .contains("chapter.tex"));
    let diagnostic = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item["message"]
                            .as_str()
                            .is_some_and(|message| message.contains("outdated PDF"))
                    })
                })
    });
    assert_eq!(diagnostic["params"]["version"], 2);
    std::fs::write(&pdf, "%PDF-1.4\nFresh\n").unwrap();
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let fresh = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(fresh["params"]["status"], "success", "{fresh}");
    assert_eq!(fresh["params"]["stale"], false);
    assert_eq!(fresh["params"]["diagnosticsCount"], 0);
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn unchanged_pdf_checks_prefer_recorded_inputs_over_statically_excluded_files() {
    let script = "if [ ! -f \"$1/main.pdf\" ]; then\n printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n printf 'PWD %s\\nINPUT main.tex\\nINPUT used.tex\\n' \"$PWD\" > \"$1/main.fls\"\nfi\n";
    let (mut server, directory, uri) = fake_build_server(script, false);
    let source =
        "\\documentclass{article}\n\\includeonly{used}\n\\include{used}\n\\include{excluded}\n";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    std::fs::write(directory.path().join("used.tex"), "Used\n").unwrap();
    std::fs::write(directory.path().join("excluded.tex"), "Excluded\n").unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    std::fs::write(
        directory.path().join("excluded.tex"),
        "Changed but still excluded\n",
    )
    .unwrap();
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let second = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(second["params"]["status"], "success", "{second}");
    assert_eq!(second["params"]["stale"], false);
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn slow_synctex_queries_do_not_block_edits_and_discard_results_after_source_changes() {
    let script = "touch ready\nwhile [ ! -f release ]; do sleep 0.02; done\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n";
    let (mut server, directory, uri) = fake_synctex_server(script);
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":uri,"position":{"line":1,"character":0}}]}}));
    wait_for_file(&directory.path().join("ready"));
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"\\documentclass{article}\n\\section{Changed}\n"}]}));
    let symbols = server.request(3, "workspace/symbol", json!({"query":"Changed"}));
    assert_eq!(symbols.as_array().unwrap().len(), 1);
    std::fs::write(directory.path().join("release"), "").unwrap();
    let stale = server.until(|message| message["id"] == 2);
    assert!(
        stale["error"]["message"].as_str().unwrap().contains("Save"),
        "{stale}"
    );
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[cfg(unix)]
fn fake_synctex_server(script: &str) -> (Server, tempfile::TempDir, Url) {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("synctex");
    std::fs::write(&executable, format!("#!/bin/sh\n{script}")).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let source = "\\documentclass{article}\n\\section{Title}\n";
    let main = directory.path().join("main.tex");
    std::fs::write(&main, source).unwrap();
    let output = directory.path().join("build/latex-lsp");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("main.pdf"), "%PDF-1.4").unwrap();
    std::fs::write(output.join("main.synctex"), "SyncTeX data").unwrap();
    let path = format!(
        "{}:{}",
        directory.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut server = Server::start_with_env(&[("PATH", &path)]);
    let uri = Url::from_file_path(main).unwrap();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(directory.path()).unwrap(),"capabilities":{}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":source}}),
    );
    server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    (server, directory, uri)
}

#[cfg(unix)]
fn wait_for_file(path: &std::path::Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "subprocess failed to create {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[cfg(unix)]
fn synctex_request_cancellation_shutdown_and_timeouts_stop_the_process_group() {
    for scenario in ["cancel", "shutdown", "timeout"] {
        let (mut server, directory, uri) =
            fake_synctex_server("sleep 30 &\nprintf '%s' \"$!\" > child-pid\ntouch ready\nwait\n");
        server.send(json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":uri,"position":{"line":1,"character":0}}]}}));
        wait_for_file(&directory.path().join("ready"));
        let child: i32 = std::fs::read_to_string(directory.path().join("child-pid"))
            .unwrap()
            .parse()
            .unwrap();
        let started = std::time::Instant::now();
        match scenario {
            "cancel" => server.notify("$/cancelRequest", json!({"id":2})),
            "shutdown" => server.send(json!({"jsonrpc":"2.0","id":3,"method":"shutdown"})),
            _ => {}
        }
        let mut shutdown_received = false;
        let rejected = loop {
            let response = server.until(|message| message["id"] == 2 || message["id"] == 3);
            if response["id"] == 2 {
                break response;
            }
            assert!(response.get("error").is_none(), "{response}");
            shutdown_received = true;
        };
        if scenario == "timeout" {
            assert!(
                rejected["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("timed out"),
                "{rejected}"
            );
        } else {
            assert_eq!(rejected["error"]["code"], -32800, "{rejected}");
            assert!(started.elapsed() < Duration::from_secs(3));
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            // An orphaned child can briefly remain a zombie before init reaps it.
            let alive = unsafe { libc::kill(child, 0) } == 0;
            let zombie = std::fs::read_to_string(format!("/proc/{child}/stat"))
                .ok()
                .is_some_and(|stat| {
                    stat.rsplit_once(')')
                        .is_some_and(|(_, state)| state.trim_start().starts_with('Z'))
                });
            if !alive || zombie {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "SyncTeX descendant survived {scenario}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        if scenario != "shutdown" {
            server.request(3, "shutdown", Value::Null);
        } else if !shutdown_received {
            server.until(|message| message["id"] == 3);
        }
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn concurrent_synctex_queries_are_bounded_and_leave_normal_requests_responsive() {
    let (mut server, directory, uri) = fake_synctex_server("touch \"ready-$$\"\nwhile [ ! -f release ]; do sleep 0.02; done\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n");
    for id in 2..6 {
        server.send(json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":uri,"position":{"line":1,"character":0}}]}}));
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::fs::read_dir(directory.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("ready-"))
        .count()
        < 4
    {
        assert!(
            std::time::Instant::now() < deadline,
            "queries failed to run concurrently"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    server.send(json!({"jsonrpc":"2.0","id":6,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":uri,"position":{"line":1,"character":0}}]}}));
    let rejected = server.until(|message| message["id"] == 6);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Too many"),
        "{rejected}"
    );
    let symbols = server.request(7, "workspace/symbol", json!({"query":"Title"}));
    assert_eq!(symbols.as_array().unwrap().len(), 1);
    std::fs::write(directory.path().join("release"), "").unwrap();
    for id in 2..6 {
        let response = server.until(|message| message["id"] == id);
        assert_eq!(response["result"]["locations"][0]["page"], 1, "{response}");
    }
    server.request(8, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn standard_progress_completes_and_cancellation_uses_the_build_token() {
    for cancel in [false, true] {
        let script = if cancel {
            "sleep 30\n"
        } else {
            "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n"
        };
        let (mut server, _directory, uri) = fake_build_server_capabilities(
            script,
            false,
            json!({"window":{"workDoneProgress":true}}),
        );
        let started = server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[uri]}),
        );
        let create = server.until(|message| message["method"] == "window/workDoneProgress/create");
        let token = create["params"]["token"].clone();
        assert_eq!(token, format!("latex-build-{}", started["buildId"]));
        server.send(json!({"jsonrpc":"2.0","id":create["id"],"result":null}));
        let begin = server.until(|message| {
            message["method"] == "$/progress" && message["params"]["value"]["kind"] == "begin"
        });
        assert_eq!(begin["params"]["token"], token);
        assert_eq!(begin["params"]["value"]["cancellable"], true);
        if cancel {
            server.notify("window/workDoneProgress/cancel", json!({"token":token}));
        }
        let end = server.until(|message| {
            message["method"] == "$/progress" && message["params"]["value"]["kind"] == "end"
        });
        assert_eq!(end["params"]["token"], token);
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            finished["params"]["status"],
            if cancel { "cancelled" } else { "success" }
        );
        server.request(3, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn rejected_or_unanswered_progress_requests_do_not_prevent_compilation() {
    for reply in [false, true] {
        let (mut server, _directory, uri) = fake_build_server_capabilities(
            "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n",
            false,
            json!({"window":{"workDoneProgress":true}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[uri]}),
        );
        let create = server.until(|message| message["method"] == "window/workDoneProgress/create");
        if reply {
            server.send(json!({"jsonrpc":"2.0","id":create["id"],"error":{"code":-32601,"message":"unsupported"}}));
        }
        let hover = server.request(
            3,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":1,"character":3}}),
        );
        assert!(hover["contents"].is_object());
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "success");
        server.request(4, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn background_build_keeps_requests_responsive_and_publishes_compiler_results() {
    let (mut server, directory, uri) = fake_build_server("sleep 0.3\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: example warning.\\n' \"$2\" > \"$1/main.log\"\n", false);
    let started = server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    assert_eq!(started["status"], "started");
    let hover = server.request(
        3,
        "textDocument/hover",
        json!({"textDocument":{"uri":uri},"position":{"line":1,"character":3}}),
    );
    assert!(hover["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("section"));
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success");
    assert_eq!(finished["params"]["buildId"], started["buildId"]);
    assert!(finished["params"]["pdfUri"]
        .as_str()
        .unwrap()
        .ends_with("main.pdf"));
    let diagnostics = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|diagnostics| {
                    diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic["source"] == "latex-build")
                })
    });
    assert_eq!(
        diagnostics["params"]["diagnostics"][0]["range"]["start"]["line"],
        1
    );
    std::fs::write(
        directory.path().join("compiler.sh"),
        "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n",
    )
    .unwrap();
    let next = server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let finished = server.until(|message| {
        message["method"] == "latex/buildFinished"
            && message["params"]["buildId"] == next["buildId"]
    });
    assert_eq!(finished["params"]["status"], "success");
    assert_eq!(finished["params"]["diagnosticsCount"], 0);
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(Vec::is_empty)
    });
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn moving_build_inputs_cancels_the_old_build_before_handling_new_requests() {
    let (mut server, directory, uri) = fake_build_server("sleep 30\n", false);
    let started = server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let renamed = Url::from_file_path(directory.path().join("renamed.tex")).unwrap();
    std::fs::rename(uri.to_file_path().unwrap(), renamed.to_file_path().unwrap()).unwrap();
    server.notify(
        "workspace/didRenameFiles",
        json!({"files":[{"oldUri":uri,"newUri":renamed}]}),
    );
    let symbols = server.request(3, "workspace/symbol", json!({"query":"Title"}));
    assert_eq!(symbols[0]["location"]["uri"], renamed.as_str());
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["buildId"], started["buildId"]);
    assert_eq!(finished["params"]["status"], "cancelled");
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn unsaved_local_packages_are_rejected_and_package_changes_make_build_results_stale() {
    let (mut server, directory, uri) = fake_build_server(
        "sleep 0.2\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n",
        false,
    );
    let source = "\\documentclass{article}\n\\usepackage{local}\n";
    std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    let package_path = directory.path().join("local.sty");
    std::fs::write(&package_path, "\\newcommand{\\hello}{Hello}\n").unwrap();
    let package = Url::from_file_path(&package_path).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.notify("textDocument/didOpen", json!({"textDocument":{"uri":package,"languageId":"latex","version":1,"text":"\\newcommand{\\hello}{Unsaved}\n"}}));
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"latex.build","arguments":[uri]}}));
    let rejected = server.until(|message| message["id"] == 2);
    assert!(rejected["error"]["message"]
        .as_str()
        .unwrap()
        .contains("local.sty"));
    server.notify(
        "textDocument/didClose",
        json!({"textDocument":{"uri":package}}),
    );
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    std::fs::write(&package_path, "\\newcommand{\\hello}{Changed}\n").unwrap();
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["stale"], true);
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn repeated_saves_during_a_build_compile_the_latest_saved_content_once_more() {
    let script = "count=0\n[ ! -f \"$1/count\" ] || count=$(cat \"$1/count\")\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"$1/count\"\nif [ \"$count\" = 1 ]; then\n  touch \"$1/ready\"\n  while [ ! -f \"$1/release\" ]; do sleep 0.02; done\nfi\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: build %s.\\n' \"$2\" \"$count\" > \"$1/main.log\"\n";
    let (mut server, directory, uri) = fake_build_server(script, true);
    let started = server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let output = directory.path().join("build/latex-lsp");
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !output.join("ready").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "compiler failed to start"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    for version in [2, 3] {
        let source = format!("\\documentclass{{article}}\n\\section{{Saved {version}}}\n");
        std::fs::write(uri.to_file_path().unwrap(), &source).unwrap();
        server.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":source}]}));
        server.notify("textDocument/didSave", json!({"textDocument":{"uri":uri}}));
    }
    let status = server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[uri]}),
    );
    assert_eq!(status["queued"], true);
    std::fs::write(output.join("release"), "").unwrap();
    let first = server.until(|message| {
        message["method"] == "latex/buildFinished"
            && message["params"]["buildId"] == started["buildId"]
    });
    assert_eq!(first["params"]["stale"], true);
    let second = server.until(|message| {
        message["method"] == "latex/buildFinished"
            && message["params"]["buildId"] != started["buildId"]
    });
    assert_eq!(second["params"]["status"], "success");
    assert_eq!(second["params"]["stale"], false);
    assert_eq!(std::fs::read_to_string(output.join("count")).unwrap(), "2");
    let diagnostics = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|diagnostics| {
                    diagnostics.iter().any(|diagnostic| {
                        diagnostic["message"]
                            .as_str()
                            .is_some_and(|message| message.contains("build 2"))
                    })
                })
    });
    assert_eq!(diagnostics["params"]["version"], 3);
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn cancelling_or_shutting_down_discards_the_queued_save() {
    for shutdown in [false, true] {
        let (mut server, _directory, uri) = fake_build_server("sleep 30\n", true);
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[uri]}),
        );
        server.notify("textDocument/didSave", json!({"textDocument":{"uri":uri}}));
        let status = server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.buildStatus","arguments":[uri]}),
        );
        assert_eq!(status["queued"], true);
        if shutdown {
            server.request(4, "shutdown", Value::Null);
        } else {
            let cancelled = server.request(
                4,
                "workspace/executeCommand",
                json!({"command":"latex.cancelBuild","arguments":[uri]}),
            );
            assert_eq!(cancelled["cancelled"], 1);
        }
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "cancelled");
        if !shutdown {
            let status = server.request(
                5,
                "workspace/executeCommand",
                json!({"command":"latex.buildStatus","arguments":[uri]}),
            );
            assert_eq!(status["status"], "cancelled");
            server.request(6, "shutdown", Value::Null);
        }
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn builds_are_deduplicated_cancelled_and_reported_through_status() {
    let (mut server, _directory, uri) = fake_build_server("sleep 30\n", false);
    let started = server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[{"uri":uri}]}),
    );
    let again = server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    assert_eq!(again["status"], "running");
    assert_eq!(again["buildId"], started["buildId"]);
    let cancelled = server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.cancelBuild","arguments":[]}),
    );
    assert_eq!(cancelled["cancelled"], 1);
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "cancelled");
    let status = server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[uri]}),
    );
    assert_eq!(status["status"], "cancelled");
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn configuration_updates_enable_build_on_save_and_invalid_updates_preserve_settings() {
    let (mut server, directory, uri) =
        fake_build_server("printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n", false);
    server.notify("workspace/didChangeConfiguration", json!({"settings":{"latex":{"build":{"executable":"/bin/sh","arguments":[directory.path().join("compiler.sh"),"%o","%f"],"onSave":true}}}}));
    server.notify("textDocument/didSave", json!({"textDocument":{"uri":uri}}));
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success");
    server.notify(
        "workspace/didChangeConfiguration",
        json!({"settings":{"latex":{"build":{"timeoutSeconds":0}}}}),
    );
    server.notify("textDocument/didSave", json!({"textDocument":{"uri":uri}}));
    let again = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(again["params"]["status"], "success");
    assert_ne!(again["params"]["buildId"], finished["params"]["buildId"]);
    server.request(2, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn automatic_build_on_save_does_not_deadlock_and_stale_results_are_discarded() {
    let (mut server, _directory, uri) = fake_build_server("sleep 0.2\nprintf '%s:2: Undefined control sequence.\\n' \"$2\" > \"$1/main.log\"\nexit 1\n", true);
    server.notify("textDocument/didSave", json!({"textDocument":{"uri":uri}}));
    server.until(|message| message["method"] == "latex/buildStarted");
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"\\documentclass{article}\n\\section{New title}\n"}]}));
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics" && message["params"]["version"] == 2
    });
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["stale"], true);
    assert_eq!(finished["params"]["status"], "error");
    assert!(!server
        .pending
        .borrow()
        .iter()
        .any(
            |message| message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["diagnostics"]
                    .as_array()
                    .is_some_and(|diagnostics| diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic["source"] == "latex-build"))
        ));
    server.request(2, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn unsaved_documents_are_rejected_and_shutdown_cancels_active_builds() {
    let (mut server, _directory, uri) = fake_build_server("sleep 30\n", false);
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"unsaved"}]}),
    );
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"latex.build","arguments":[uri]}}));
    let rejected = server.until(|message| message["id"] == 2);
    assert_eq!(rejected["error"]["code"], -32602);
    assert!(rejected["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Save"));
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":3},"contentChanges":[{"text":"\\documentclass{article}\n\\section{Title}\n"}]}));
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn real_build_command_compiles_the_root_from_an_included_document() {
    let directory = tempfile::tempdir().unwrap();
    let root = Url::from_directory_path(directory.path()).unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let chapter = Url::from_file_path(directory.path().join("chapter.tex")).unwrap();
    std::fs::write(
        main.to_file_path().unwrap(),
        "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(chapter.to_file_path().unwrap(), "Hello, world.\n").unwrap();
    let mut server = Server::start();
    server.request(
        1,
        "initialize",
        json!({"processId":null,"rootUri":root,"capabilities":{}}),
    );
    server.notify("initialized", json!({}));
    let started = server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[chapter]}),
    );
    assert_eq!(started["rootUri"], main.as_str());
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    let pdf = Url::parse(finished["params"]["pdfUri"].as_str().unwrap())
        .unwrap()
        .to_file_path()
        .unwrap();
    assert!(std::fs::read(pdf).unwrap().starts_with(b"%PDF"));
    server.request(3, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn recorder_indexes_dynamic_inputs_and_uses_them_for_later_builds() {
    let script = "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'PWD %s\\nINPUT main.tex\\nINPUT .inputs/dynamic.ltx\\n' \"$PWD\" > \"$1/main.fls\"\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    std::fs::create_dir(directory.path().join(".inputs")).unwrap();
    let dynamic = Url::from_file_path(directory.path().join(".inputs/dynamic.ltx")).unwrap();
    let source = "\\label{dynamic:discovered}\n";
    std::fs::write(dynamic.to_file_path().unwrap(), source).unwrap();
    assert_eq!(
        server.request(2, "workspace/symbol", json!({"query":"dynamic:discovered"})),
        json!([])
    );
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["stale"], false, "{finished}");
    assert_eq!(finished["params"]["dependenciesCount"], 2);
    let symbols = server.request(4, "workspace/symbol", json!({"query":"dynamic:discovered"}));
    assert_eq!(symbols[0]["location"]["uri"], dynamic.as_str());
    let started = server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[dynamic]}),
    );
    assert_eq!(started["rootUri"], main.as_str());
    server.until(|message| message["method"] == "latex/buildFinished");
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":dynamic,"languageId":"latex","version":1,"text":"Unsaved"}}),
    );
    server.send(json!({"jsonrpc":"2.0","id":6,"method":"workspace/executeCommand","params":{"command":"latex.build","arguments":[main]}}));
    let rejected = server.until(|message| message["id"] == 6);
    assert!(rejected["error"]["message"]
        .as_str()
        .unwrap()
        .contains("dynamic.ltx"));
    server.request(7, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn newly_recorded_inputs_discard_stale_diagnostics_but_unrelated_buffers_do_not() {
    for scenario in ["unsaved", "changed", "unrelated"] {
        let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'PWD %s\\nINPUT main.tex\\nINPUT dynamic.tex\\n' \"$PWD\" > \"$1/main.fls\"\nprintf '%s:2: LaTeX Warning: recorder warning.\\n' \"$2\" > \"$1/main.log\"\n";
        let (mut server, directory, main) = fake_build_server(script, false);
        let dynamic_path = directory.path().join("dynamic.tex");
        std::fs::write(&dynamic_path, "Saved\n").unwrap();
        let opened = directory.path().join(if scenario == "unrelated" {
            "other.tex"
        } else {
            "dynamic.tex"
        });
        if scenario != "changed" {
            server.notify("textDocument/didOpen", json!({"textDocument":{"uri":Url::from_file_path(opened).unwrap(),"languageId":"latex","version":1,"text":"Unsaved\n"}}));
        }
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !output.join("ready").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "compiler failed to start"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        if scenario == "changed" {
            std::fs::write(&dynamic_path, "Changed during compilation\n").unwrap();
        }
        std::fs::write(output.join("release"), "").unwrap();
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            finished["params"]["stale"],
            scenario != "unrelated",
            "{scenario}: {finished}"
        );
        if scenario != "unrelated" {
            assert!(!server.pending.borrow().iter().any(|message| {
                message["method"] == "textDocument/publishDiagnostics"
                    && message["params"]["diagnostics"]
                        .as_array()
                        .is_some_and(|items| {
                            items.iter().any(|item| item["source"] == "latex-build")
                        })
            }));
        }
        server.request(3, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn unchanged_recorder_files_are_not_treated_as_current_inputs() {
    let (mut server, directory, main) =
        fake_build_server("printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n", false);
    let output = directory.path().join("build/latex-lsp");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(directory.path().join("old.tex"), "Old\n").unwrap();
    std::fs::write(output.join("main.fls"), "INPUT main.tex\nINPUT old.tex\n").unwrap();
    server.notify("textDocument/didOpen", json!({"textDocument":{"uri":Url::from_file_path(directory.path().join("old.tex")).unwrap(),"languageId":"latex","version":1,"text":"Unsaved\n"}}));
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["stale"], false);
    assert_eq!(finished["params"]["dependenciesCount"], 1);
    server.request(3, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn real_recorder_discovers_macro_expanded_input_without_indexing_tex_distribution() {
    let directory = tempfile::tempdir().unwrap();
    let root = Url::from_directory_path(directory.path()).unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let dynamic = Url::from_file_path(directory.path().join("dynamic.ltx")).unwrap();
    std::fs::write(main.to_file_path().unwrap(), "\\documentclass{article}\n\\def\\chapterfile{dynamic.ltx}\n\\begin{document}\n\\input{\\chapterfile}\n\\end{document}\n").unwrap();
    std::fs::write(
        dynamic.to_file_path().unwrap(),
        "\\section{Discovered chapter}\\label{dynamic:real}\nHello.\n",
    )
    .unwrap();
    let mut server = Server::start();
    server.request(
        1,
        "initialize",
        json!({"processId":null,"rootUri":root,"capabilities":{}}),
    );
    server.notify("initialized", json!({}));
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], false, "{finished}");
    assert_eq!(finished["params"]["dependenciesCount"], 2, "{finished}");
    let symbols = server.request(3, "workspace/symbol", json!({"query":"dynamic:real"}));
    assert_eq!(symbols[0]["location"]["uri"], dynamic.as_str());
    let started = server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[dynamic]}),
    );
    assert_eq!(started["rootUri"], main.as_str());
    server.until(|message| message["method"] == "latex/buildFinished");
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires latexmk, SyncTeX and a TeX installation"]
fn real_synctex_round_trip_handles_spaces_and_rejects_unsaved_or_stale_sources() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("project with spaces");
    std::fs::create_dir(&folder).unwrap();
    let root = Url::from_directory_path(&folder).unwrap();
    let main = Url::from_file_path(folder.join("main.tex")).unwrap();
    let chapter = Url::from_file_path(folder.join("chapter file.tex")).unwrap();
    let source = "Hello SyncTeX.\nSecond line.\n% 😀\n";
    std::fs::write(
        main.to_file_path().unwrap(),
        "\\documentclass{article}\n\\begin{document}\n\\input{chapter file}\n\\end{document}\n",
    )
    .unwrap();
    std::fs::write(chapter.to_file_path().unwrap(), source).unwrap();
    let marker = directory.path().join("unexpected-launch");
    let external = format!("touch {}", marker.display());
    let mut server =
        Server::start_with_env(&[("SYNCTEX_VIEWER", &external), ("SYNCTEX_EDITOR", &external)]);
    let initialized = server.request(
        1,
        "initialize",
        json!({"processId":null,"rootUri":root,"capabilities":{}}),
    );
    let commands = initialized["capabilities"]["executeCommandProvider"]["commands"]
        .as_array()
        .unwrap();
    assert!(commands.contains(&json!("latex.forwardSearch")));
    assert!(commands.contains(&json!("latex.inverseSearch")));
    server.notify("initialized", json!({}));
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":chapter,"position":{"line":0,"character":6}}]}}));
    let missing = server.until(|message| message["id"] == 2);
    assert!(missing["error"]["message"]
        .as_str()
        .unwrap()
        .contains("missing"));
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    let forward = server.request(4, "workspace/executeCommand", json!({"command":"latex.forwardSearch","arguments":[{"uri":chapter,"position":{"line":0,"character":6}}]}));
    assert_eq!(forward["rootUri"], main.as_str());
    assert_eq!(forward["pdfUri"], finished["params"]["pdfUri"]);
    let hit = &forward["locations"][0];
    assert_eq!(hit["page"], 1, "{forward}");
    assert!(hit["x"].as_f64().unwrap().is_finite());
    let inverse = server.request(5, "workspace/executeCommand", json!({"command":"latex.inverseSearch","arguments":[{"uri":main,"page":hit["page"],"x":hit["x"],"y":hit["y"]}]}));
    assert!(
        inverse["locations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|location| location["uri"] == chapter.as_str()
                && location["range"]["start"]["line"] == 0),
        "{inverse}"
    );
    assert!(!marker.exists(), "SyncTeX launched an external command");
    for (id, command, argument, expected) in [
        (
            6,
            "latex.forwardSearch",
            json!({"uri":chapter,"position":{"line":2,"character":3}}),
            "UTF-16",
        ),
        (
            7,
            "latex.inverseSearch",
            json!({"uri":main,"page":0,"x":0,"y":0}),
            "one-based",
        ),
        (
            8,
            "latex.inverseSearch",
            json!({"uri":main,"page":1,"x":-1,"y":0}),
            "nonnegative",
        ),
    ] {
        server.send(json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":{"command":command,"arguments":[argument]}}));
        let rejected = server.until(|message| message["id"] == id);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .unwrap()
                .contains(expected),
            "{rejected}"
        );
    }
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":chapter,"languageId":"latex","version":1,"text":"Unsaved\n"}}),
    );
    server.send(json!({"jsonrpc":"2.0","id":9,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":chapter,"position":{"line":0,"character":1}}]}}));
    let unsaved = server.until(|message| message["id"] == 9);
    assert!(unsaved["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Save"));
    server.notify(
        "textDocument/didClose",
        json!({"textDocument":{"uri":chapter}}),
    );
    std::fs::write(chapter.to_file_path().unwrap(), "Changed\n").unwrap();
    server.send(json!({"jsonrpc":"2.0","id":10,"method":"workspace/executeCommand","params":{"command":"latex.inverseSearch","arguments":[{"uri":main,"page":1,"x":0,"y":0}]}}));
    let stale = server.until(|message| message["id"] == 10);
    assert!(stale["error"]["message"]
        .as_str()
        .unwrap()
        .contains("changed"));
    server.request(11, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn bibliography_relations_support_navigation_completion_and_versioned_rename_over_stdio() {
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}}}}));
    server.notify("initialized", json!({}));
    let uri = "file:///bibliography-protocol/refs.bib";
    let source = "@book{parent, title={Parent}}\n@incollection{child, crossref={parent}}\n";
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"bibtex","version":7,"text":source}}),
    );
    server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    let position = latex_lsp::text::Text::new(source.into())
        .position(source.find("crossref={par").unwrap() + "crossref={par".len());
    let target = json!({"textDocument":{"uri":uri},"position":position});
    let definitions = server.request(2, "textDocument/definition", target.clone());
    assert_eq!(definitions[0]["range"]["start"]["line"], 0);
    let completions = server.request(3, "textDocument/completion", target.clone());
    assert!(completions
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["label"] == "parent"));
    let highlights = server.request(4, "textDocument/documentHighlight", target);
    assert_eq!(highlights.as_array().unwrap().len(), 2);
    let edits = server.request(
        5,
        "textDocument/rename",
        json!({"textDocument":{"uri":uri},"position":position,"newName":"renamed"}),
    );
    assert_eq!(edits["documentChanges"][0]["textDocument"]["version"], 7);
    assert_eq!(
        edits["documentChanges"][0]["edits"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    server.send(json!({"jsonrpc":"2.0","id":6,"method":"textDocument/rename","params":{"textDocument":{"uri":uri},"position":position,"newName":"*"}}));
    let rejected = server.until(|message| message["id"] == 6);
    assert!(rejected.get("error").is_some());
    server.request(7, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires latexmk, XeLaTeX, LuaLaTeX and a TeX installation"]
fn real_engine_directives_select_the_root_engine_and_explicit_configuration_overrides_them() {
    for (program, engine, guard) in [
        ("xelatex", "xelatex", "ifXeTeX"),
        ("lualatex", "lualatex", "ifLuaTeX"),
        ("xelatex", "pdflatex", "ifPDFTeX"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = Url::from_directory_path(directory.path()).unwrap();
        let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
        let chapter = Url::from_file_path(directory.path().join("chapter.tex")).unwrap();
        let source = format!("% !TeX program = {program}\n\\documentclass{{article}}\n\\usepackage{{iftex}}\n\\{guard}\\else\\errmessage{{Wrong engine}}\\fi\n\\begin{{document}}\n\\input{{chapter}}\n\\end{{document}}\n");
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        std::fs::write(
            chapter.to_file_path().unwrap(),
            "% !TeX program = ignored\nEngine selected.\n",
        )
        .unwrap();
        let cache = directory.path().join(".tex-cache");
        std::fs::create_dir(&cache).unwrap();
        let cache = cache.to_str().unwrap();
        let mut server = Server::start_with_env(&[("TEXMFVAR", cache), ("TEXMFCACHE", cache)]);
        let options = if engine == "pdflatex" {
            json!({"build":{"engine":engine}})
        } else {
            json!({})
        };
        server.request(1, "initialize", json!({"processId":null,"rootUri":root,"capabilities":{},"initializationOptions":options}));
        server.notify("initialized", json!({}));
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[chapter]}),
        );
        let started = server.until(|message| message["method"] == "latex/buildStarted");
        assert_eq!(started["params"]["engine"], engine);
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            finished["params"]["status"], "success",
            "{engine}: {finished}"
        );
        assert_eq!(finished["params"]["engine"], engine);
        assert_eq!(finished["params"]["executable"], "latexmk");
        assert_eq!(finished["params"]["stale"], false, "{finished}");
        assert_eq!(finished["params"]["dependenciesCount"], 2, "{finished}");
        let pdf = Url::parse(finished["params"]["pdfUri"].as_str().unwrap())
            .unwrap()
            .to_file_path()
            .unwrap();
        assert!(std::fs::read(pdf).unwrap().starts_with(b"%PDF"));
        server.request(3, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
fn ltx_def_and_clo_sources_are_indexed_and_included_in_dynamic_watch_registration() {
    let directory = tempfile::tempdir().unwrap();
    for extension in ["ltx", "def", "clo"] {
        std::fs::write(
            directory.path().join(format!("source.{extension}")),
            format!("\\label{{source:{extension}}}"),
        )
        .unwrap();
    }
    let mut server = Server::start();
    server.request(1,"initialize",json!({"processId":null,"rootUri":Url::from_directory_path(directory.path()).unwrap(),"capabilities":{"workspace":{"didChangeWatchedFiles":{"dynamicRegistration":true}}}}));
    server.notify("initialized", json!({}));
    let registration = server.until(|message| message["method"] == "client/registerCapability");
    let pattern = registration["params"]["registrations"][0]["registerOptions"]["watchers"][0]
        ["globPattern"]
        .as_str()
        .unwrap();
    for extension in ["tex", "ltx", "bib", "sty", "cls", "def", "clo"] {
        assert!(pattern.contains(extension));
    }
    server.send(json!({"jsonrpc":"2.0","id":registration["id"],"result":null}));
    let symbols = server.request(2, "workspace/symbol", json!({"query":"source:"}));
    assert_eq!(symbols.as_array().unwrap().len(), 3);
    let definitions = directory.path().join("source.def");
    std::fs::write(&definitions, "\\label{updated:definitions}").unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":Url::from_file_path(&definitions).unwrap(),"type":2}]}),
    );
    assert!(server
        .request(3, "workspace/symbol", json!({"query":"source:def"}))
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        server
            .request(
                4,
                "workspace/symbol",
                json!({"query":"updated:definitions"})
            )
            .as_array()
            .unwrap()
            .len(),
        1
    );
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires latexmk and a TeX installation"]
fn real_ltx_root_and_dynamically_input_definition_files_compile_and_keep_root_detection() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.ltx")).unwrap();
    let definitions = Url::from_file_path(directory.path().join("commands.def")).unwrap();
    std::fs::write(main.to_file_path().unwrap(), "\\documentclass{article}\n\\def\\definitionsfile{commands.def}\n\\input{\\definitionsfile}\n\\begin{document}\n\\hellocommand\n\\end{document}\n").unwrap();
    std::fs::write(
        definitions.to_file_path().unwrap(),
        "\\newcommand{\\hellocommand}{Hello source files}\n",
    )
    .unwrap();
    let mut server = Server::start();
    server.request(1,"initialize",json!({"processId":null,"rootUri":Url::from_directory_path(directory.path()).unwrap(),"capabilities":{}}));
    server.notify("initialized", json!({}));
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], false);
    assert_eq!(finished["params"]["dependenciesCount"], 2);
    let next = server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[definitions]}),
    );
    assert_eq!(next["rootUri"], main.as_str());
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn dynamic_watch_registration_and_closed_file_updates() {
    let directory = tempfile::tempdir().unwrap();
    let root = Url::from_directory_path(directory.path()).unwrap();
    let chapter = directory.path().join("chapter.tex");
    std::fs::write(&chapter, "\\label{old}").unwrap();
    let chapter_uri = Url::from_file_path(&chapter).unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":root,"capabilities":{"workspace":{"didChangeWatchedFiles":{"dynamicRegistration":true}}}}));
    server.notify("initialized", json!({}));
    let registration = server.until(|message| message["method"] == "client/registerCapability");
    assert_eq!(
        registration["params"]["registrations"][0]["method"],
        "workspace/didChangeWatchedFiles"
    );
    server.send(json!({"jsonrpc":"2.0", "id":registration["id"], "result":null}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":"\\ref{new}"}}),
    );
    server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    let position = json!({"textDocument":{"uri":uri},"position":{"line":0,"character":7}});
    assert!(server
        .request(2, "textDocument/definition", position.clone())
        .as_array()
        .unwrap()
        .is_empty());
    std::fs::write(&chapter, "\\label{new}").unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":chapter_uri,"type":2}]}),
    );
    let definitions = server.request(3, "textDocument/definition", position.clone());
    assert_eq!(definitions[0]["uri"], chapter_uri.as_str());
    std::fs::remove_file(chapter).unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":chapter_uri,"type":3}]}),
    );
    assert!(server
        .request(4, "textDocument/definition", position)
        .as_array()
        .unwrap()
        .is_empty());
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn exit_without_shutdown_returns_failure() {
    let mut server = Server::start();
    server.notify("exit", Value::Null);
    server.wait_exit(1);
}

#[test]
fn rename_prepares_ranges_and_returns_versioned_cross_file_edits() {
    let directory = tempfile::tempdir().unwrap();
    let root = Url::from_directory_path(directory.path()).unwrap();
    let chapter = Url::from_file_path(directory.path().join("chapter.tex")).unwrap();
    std::fs::write(chapter.to_file_path().unwrap(), r"\label{old}").unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let mut server = Server::start();
    let init = server.request(1, "initialize", json!({"processId":null,"rootUri":root,"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}}}}));
    assert_eq!(
        init["capabilities"]["renameProvider"]["prepareProvider"],
        true
    );
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":9,"text":"\\ref{old}"}}),
    );
    server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    let params = json!({"textDocument":{"uri":uri},"position":{"line":0,"character":6}});
    let prepared = server.request(2, "textDocument/prepareRename", params.clone());
    assert_eq!(prepared["placeholder"], "old");
    assert_eq!(prepared["range"]["start"]["character"], 5);
    let mut params = params;
    params["newName"] = json!("sec:new");
    let edit = server.request(3, "textDocument/rename", params);
    let documents = edit["documentChanges"].as_array().unwrap();
    assert_eq!(documents.len(), 2);
    let open = documents
        .iter()
        .find(|edit| edit["textDocument"]["uri"] == uri.as_str())
        .unwrap();
    assert_eq!(open["textDocument"]["version"], 9);
    assert_eq!(open["edits"][0]["newText"], "sec:new");
    let closed = documents
        .iter()
        .find(|edit| edit["textDocument"]["uri"] == chapter.as_str())
        .unwrap();
    assert!(closed["textDocument"]["version"].is_null());
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn file_rename_requests_update_references_and_preserve_open_document_versions() {
    let directory = tempfile::tempdir().unwrap();
    let root = Url::from_directory_path(directory.path()).unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let child = Url::from_file_path(directory.path().join("child.tex")).unwrap();
    let renamed = Url::from_file_path(directory.path().join("renamed.tex")).unwrap();
    let source = "\\documentclass{article}\n\\input{child}\n\\ref{sec:x}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(child.to_file_path().unwrap(), "\\label{sec:x}").unwrap();
    let mut server = Server::start();
    let init = server.request(1,"initialize",json!({"processId":null,"rootUri":root,"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true},"fileOperations":{"willRename":true,"didRename":true}}}}));
    assert_eq!(
        init["capabilities"]["workspace"]["fileOperations"]["willRename"]["filters"][0]["scheme"],
        "file"
    );
    server.notify("initialized", json!({}));
    for (uri, source, version) in [(&main, source, 1), (&child, "\\label{sec:x}", 4)] {
        server.notify("textDocument/didOpen",json!({"textDocument":{"uri":uri,"languageId":"latex","version":version,"text":source}}));
        server.until(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri.as_str()
        });
    }
    let files = json!({"files":[{"oldUri":child,"newUri":renamed}]});
    let edit = server.request(2, "workspace/willRenameFiles", files.clone());
    assert_eq!(
        edit["documentChanges"][0]["textDocument"]["uri"],
        main.as_str()
    );
    assert_eq!(edit["documentChanges"][0]["textDocument"]["version"], 1);
    assert_eq!(edit["documentChanges"][0]["edits"][0]["newText"], "renamed");
    let source = source.replace("{child}", "{renamed}");
    std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    std::fs::rename(
        child.to_file_path().unwrap(),
        renamed.to_file_path().unwrap(),
    )
    .unwrap();
    server.notify("workspace/didRenameFiles", files);
    let symbols = server.request(3, "workspace/symbol", json!({"query":"sec:x"}));
    assert_eq!(symbols[0]["location"]["uri"], renamed.as_str());
    let rename = server.request(4,"textDocument/rename",json!({"textDocument":{"uri":renamed},"position":{"line":0,"character":9},"newName":"sec:new"}));
    let documents = rename["documentChanges"].as_array().unwrap();
    assert_eq!(
        documents
            .iter()
            .find(|doc| doc["textDocument"]["uri"] == renamed.as_str())
            .unwrap()["textDocument"]["version"],
        4
    );
    let definition = server.request(
        5,
        "textDocument/definition",
        json!({"textDocument":{"uri":main},"position":{"line":2,"character":8}}),
    );
    assert_eq!(definition[0]["uri"], renamed.as_str());
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn command_only_code_actions_request_client_edits_and_reject_stale_versions() {
    let mut server = Server::start();
    let uri = Url::parse("file:///editing-protocol/legacy.tex").unwrap();
    server.request(
        1,
        "initialize",
        json!({"processId":null,"capabilities":{"workspace":{"applyEdit":true}}}),
    );
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":"{broken"}}),
    );
    let diagnostics =
        server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    let diagnostic = diagnostics["params"]["diagnostics"][0].clone();
    let actions = server.request(2, "textDocument/codeAction", json!({"textDocument":{"uri":uri},"range":diagnostic["range"],"context":{"diagnostics":[diagnostic]}}));
    assert_eq!(actions[0]["command"], "latex.applyQuickFix");
    server.send(json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":{"command":actions[0]["command"],"arguments":actions[0]["arguments"]}}));
    let apply = server.until(|message| message["method"] == "workspace/applyEdit");
    assert_eq!(
        apply["params"]["edit"]["changes"][uri.as_str()][0]["newText"],
        "}"
    );
    server.send(json!({"jsonrpc":"2.0","id":apply["id"],"result":{"applied":true}}));
    let response = server.until(|message| message["id"] == 3);
    assert_eq!(response["result"]["applied"], true);
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"{still broken"}]}),
    );
    server.send(json!({"jsonrpc":"2.0","id":4,"method":"workspace/executeCommand","params":{"command":actions[0]["command"],"arguments":actions[0]["arguments"]}}));
    let response = server.until(|message| message["id"] == 4);
    assert_eq!(response["error"]["code"], -32602);
    assert!(response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("outdated"));
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn quick_fixes_highlights_and_selection_ranges_work_over_stdio() {
    let mut server = Server::start();
    let uri = Url::parse("file:///editing-protocol/main.tex").unwrap();
    let init = server.request(1, "initialize", json!({"processId":null,"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}},"textDocument":{"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix"]}}}}}}));
    assert_eq!(
        init["capabilities"]["codeActionProvider"]["codeActionKinds"][0],
        "quickfix"
    );
    assert_eq!(init["capabilities"]["selectionRangeProvider"], true);
    assert_eq!(init["capabilities"]["documentHighlightProvider"], true);
    server.notify("initialized", json!({}));
    let source = "\\label{x}\n\\ref{x}\n\\begin{equation}\nx=1\n";
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":3,"text":source}}),
    );
    let diagnostics =
        server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    let diagnostic = diagnostics["params"]["diagnostics"][0].clone();
    let actions = server.request(2, "textDocument/codeAction", json!({"textDocument":{"uri":uri},"range":diagnostic["range"],"context":{"diagnostics":[diagnostic],"only":["quickfix"]}}));
    assert_eq!(actions.as_array().unwrap().len(), 1);
    assert_eq!(actions[0]["title"], "Close environment `equation`");
    assert_eq!(
        actions[0]["edit"]["documentChanges"][0]["textDocument"]["version"],
        3
    );
    let highlights = server.request(
        3,
        "textDocument/documentHighlight",
        json!({"textDocument":{"uri":uri},"position":{"line":1,"character":6}}),
    );
    assert_eq!(highlights.as_array().unwrap().len(), 2);
    let selections = server.request(4, "textDocument/selectionRange", json!({"textDocument":{"uri":uri},"positions":[{"line":1,"character":6},{"line":3,"character":1}]}));
    assert_eq!(selections.as_array().unwrap().len(), 2);
    assert!(selections[0]["parent"].is_object());
    let repaired = format!("{source}\\end{{equation}}\n");
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":4},"contentChanges":[{"text":repaired}]}),
    );
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics" && message["params"]["version"] == 4
    });
    let actions = server.request(5, "textDocument/codeAction", json!({"textDocument":{"uri":uri},"range":diagnostic["range"],"context":{"diagnostics":[diagnostic]}}));
    assert!(actions.as_array().unwrap().is_empty());
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn navigation_capabilities_and_requests_work_over_stdio() {
    let directory = tempfile::tempdir().unwrap();
    let root = Url::from_directory_path(directory.path()).unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    std::fs::write(directory.path().join("child.tex"), "\\label{sec:child}").unwrap();
    let source = "\\section{Main}\n\\input{child}\n\\begin{itemize}\n\\item text\n\\end{itemize}\n";
    std::fs::write(directory.path().join("main.tex"), source).unwrap();
    let mut server = Server::start();
    let init = server.request(1, "initialize", json!({"processId":null,"rootUri":root,"capabilities":{"textDocument":{"foldingRange":{"rangeLimit":1,"lineFoldingOnly":true}}}}));
    assert_eq!(init["capabilities"]["workspaceSymbolProvider"], true);
    assert_eq!(init["capabilities"]["foldingRangeProvider"], true);
    assert!(init["capabilities"]["documentLinkProvider"].is_object());
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":source}}),
    );
    server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    let symbols = server.request(2, "workspace/symbol", json!({"query":"sch"}));
    assert_eq!(symbols[0]["name"], "sec:child");
    let folds = server.request(
        3,
        "textDocument/foldingRange",
        json!({"textDocument":{"uri":uri}}),
    );
    assert_eq!(folds.as_array().unwrap().len(), 1);
    assert_eq!(folds[0]["startLine"], 0);
    let links = server.request(
        4,
        "textDocument/documentLink",
        json!({"textDocument":{"uri":uri}}),
    );
    assert!(links[0]["target"].as_str().unwrap().ends_with("child.tex"));
    let completions = server.request(
        5,
        "textDocument/completion",
        json!({"textDocument":{"uri":uri},"position":{"line":1,"character":9}}),
    );
    let item = completions
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "child.tex")
        .unwrap();
    assert_eq!(item["textEdit"]["newText"], "child");
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn document_symbols_follow_the_clients_hierarchy_capability() {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    for hierarchy in [false, true] {
        let mut server = Server::start();
        server.request(1, "initialize", json!({"processId":null,"capabilities":{"textDocument":{"documentSymbol":{"hierarchicalDocumentSymbolSupport":hierarchy}}}}));
        server.notify("initialized", json!({}));
        server.notify("textDocument/didOpen", json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":"\\section{Main}\n\\subsection{Child}\n\\label{sec:child}"}}));
        server.until(|message| message["method"] == "textDocument/publishDiagnostics");
        let symbols = server.request(
            2,
            "textDocument/documentSymbol",
            json!({"textDocument":{"uri":uri}}),
        );
        if hierarchy {
            assert_eq!(symbols.as_array().unwrap().len(), 1);
            assert_eq!(symbols[0]["children"][0]["name"], "Child");
            assert_eq!(
                symbols[0]["children"][0]["children"][0]["name"],
                "sec:child"
            );
            assert!(symbols[0].get("location").is_none());
        } else {
            assert_eq!(symbols.as_array().unwrap().len(), 3);
            assert_eq!(symbols[0]["location"]["uri"], uri.as_str());
            assert_eq!(symbols[1]["containerName"], "Main");
            assert_eq!(symbols[2]["containerName"], "Child");
        }
        server.request(3, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
fn stdio_lifecycle_incremental_edits_navigation_and_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let root = Url::from_directory_path(directory.path()).unwrap();
    std::fs::write(directory.path().join("chapter.tex"), "\\label{sec:intro}").unwrap();
    let uri = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let mut server = Server::start();
    let initialized = server.request(
        1,
        "initialize",
        json!({"processId":null,"rootUri":root,"capabilities":{}}),
    );
    assert_eq!(initialized["capabilities"]["positionEncoding"], "utf-16");
    assert_eq!(initialized["capabilities"]["textDocumentSync"]["change"], 2);
    server.notify("initialized", json!({}));
    server.notify("textDocument/didOpen", json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":"😀 \\ref{sec:intro}\n\\section{中文}\n}"}}));
    let diagnostics =
        server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    assert_eq!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        diagnostics["params"]["diagnostics"][0]["range"]["start"]["line"],
        2
    );
    let definition = server.request(
        2,
        "textDocument/definition",
        json!({"textDocument":{"uri":uri},"position":{"line":0,"character":12}}),
    );
    assert!(definition[0]["uri"]
        .as_str()
        .unwrap()
        .ends_with("chapter.tex"));
    let hover = server.request(
        3,
        "textDocument/hover",
        json!({"textDocument":{"uri":uri},"position":{"line":1,"character":4}}),
    );
    assert!(hover["contents"]["value"]
        .as_str()
        .unwrap()
        .contains("section"));
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"range":{"start":{"line":2,"character":0},"end":{"line":2,"character":1}},"text":"\\sec"}]}));
    let diagnostics =
        server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    assert_eq!(diagnostics["params"]["version"], 2);
    assert!(diagnostics["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .is_empty());
    // An edit inside a UTF-16 surrogate pair must not corrupt the document.
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":3},"contentChanges":[{"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":2}},"text":"bad"}]}));
    // Stale versions must not replace newer text.
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":1},"contentChanges":[{"text":"stale"}]}),
    );
    let completions = server.request(
        4,
        "textDocument/completion",
        json!({"textDocument":{"uri":uri},"position":{"line":2,"character":4}}),
    );
    assert!(completions
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["label"] == "section"));
    let symbols = server.request(
        5,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":uri}}),
    );
    assert_eq!(symbols[0]["name"], "中文");
    server.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}));
    let cleared = server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    assert!(cleared["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(server.request(6, "shutdown", Value::Null).is_null());
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}
