use std::{
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
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("LSP server timed out or closed stdout");
            assert!(message.get("error").is_none(), "LSP error: {message}");
            if matches(&message) {
                return message;
            }
        }
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}));
        self.until(|message| message["id"] == id)["result"].clone()
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
