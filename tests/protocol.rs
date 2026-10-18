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
        let mut child = Command::new(env!("CARGO_BIN_EXE_latex-lsp"))
            .arg("--stdio")
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
