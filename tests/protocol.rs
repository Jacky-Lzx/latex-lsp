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
    stdin: Option<ChildStdin>,
    messages: mpsc::Receiver<Value>,
    pending: RefCell<Vec<Value>>,
}

#[test]
#[cfg(unix)]
fn active_external_root_builds_can_be_queried_and_cancelled_by_the_reported_uri() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let folder = root.join("workspace/a/b/c");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        root.join("workspace/a/main.tex"),
        "\\documentclass{article}",
    )
    .unwrap();
    std::fs::write(root.join("main.tex"), "\\documentclass{report}").unwrap();
    std::fs::write(folder.join("chapter.tex"), "Chapter.").unwrap();
    let source = Url::from_file_path(folder.join("chapter.tex")).unwrap();
    let main = Url::from_file_path(root.join("workspace/a/main.tex")).unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(folder).unwrap(),"capabilities":{},"initializationOptions":{"build":{"executable":"sh","arguments":["-c","sleep 30"],"rootFile":"../../main.tex","timeoutSeconds":5}}}));
    server.notify("initialized", json!({}));
    let started = server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[source]}),
    );
    assert_eq!(started["rootUri"], json!(main));
    let status = server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[started["rootUri"]]}),
    );
    assert_eq!(status["status"], "running");
    assert_eq!(status["buildId"], started["buildId"]);
    let cancelled = server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.cancelBuild","arguments":[started["rootUri"]]}),
    );
    assert_eq!(cancelled["cancelled"], 1);
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "cancelled", "{finished}");
    assert_eq!(finished["params"]["rootUri"], json!(main));
    let status = server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[main]}),
    );
    assert_eq!(status["status"], "cancelled");
    assert_eq!(status["buildId"], started["buildId"]);
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires pdflatex"]
fn external_relative_roots_return_uris_that_work_for_build_status_and_cancellation() {
    for overlap in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let folder = root.join("workspace/a/b/c");
        std::fs::create_dir_all(&folder).unwrap();
        let main_path = root.join("workspace/a/main.tex");
        std::fs::write(
            &main_path,
            "\\documentclass{article}\\begin{document}\\input{b/c/chapter}\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("main.tex"),
            "\\documentclass{article}\\begin{document}Wrong project.\\end{document}\n",
        )
        .unwrap();
        std::fs::write(folder.join("chapter.tex"), "Expected project.\n").unwrap();
        let source = Url::from_file_path(folder.join("chapter.tex")).unwrap();
        let main = Url::from_file_path(&main_path).unwrap();
        let mut folders =
            vec![json!({"uri":Url::from_directory_path(&folder).unwrap(),"name":"nested"})];
        if overlap {
            folders.push(json!({"uri":Url::from_directory_path(root.join("workspace/a")).unwrap(),"name":"outer"}));
        }
        let mut server = Server::start();
        server.request(1, "initialize", json!({"processId":null,"workspaceFolders":folders,"capabilities":{},"initializationOptions":{"build":{"executable":"pdflatex","rootFile":"../../main.tex"}}}));
        server.notify("initialized", json!({}));
        let started = server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[source]}),
        );
        assert_eq!(started["rootUri"], json!(main));
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "success", "{finished}");
        assert_eq!(finished["params"]["rootUri"], json!(main));
        let status = server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.buildStatus","arguments":[finished["params"]["rootUri"]]}),
        );
        assert_eq!(status["status"], "success", "{overlap}: {status}");
        assert_eq!(status["buildId"], started["buildId"]);
        assert_eq!(status["rootUri"], json!(main));
        let cancelled = server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.cancelBuild","arguments":[started["rootUri"]]}),
        );
        assert_eq!(cancelled["cancelled"], 0);
        let again = server.request(
            5,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[started["rootUri"]]}),
        );
        assert_eq!(again["rootUri"], json!(main));
        let repeated = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(repeated["params"]["status"], "success", "{repeated}");
        assert_eq!(repeated["params"]["rootUri"], json!(main));
        let from_source = server.request(
            6,
            "workspace/executeCommand",
            json!({"command":"latex.buildStatus","arguments":[source]}),
        );
        assert_eq!(from_source["buildId"], again["buildId"]);
        assert_eq!(from_source["rootUri"], json!(main));
        server.request(7, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_builds_from_sources_ignore_deleted_roots_and_document_examples_in_packages() {
    for package in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let main_name = if package { "main.tex" } else { "b.tex" };
        let source_name = if package { "local.sty" } else { "chapter.tex" };
        let main_source = if package {
            "\\documentclass{article}\\usepackage{local}\\begin{document}\\fromChild\\end{document}\n"
        } else {
            "\\documentclass{article}\\begin{document}\\input{chapter}\\end{document}\n"
        };
        let source = if package {
            std::fs::write(
                root.join("chapter.tex"),
                "\\newcommand{\\fromChild}{Hello.}\n",
            )
            .unwrap();
            "\\ProvidesPackage{local}\\newcommand{\\beginLater}{\\begin{document}}\\input{chapter}\n"
        } else {
            std::fs::write(root.join("a.tex"), main_source).unwrap();
            "Hello.\n"
        };
        std::fs::write(root.join(main_name), main_source).unwrap();
        std::fs::write(root.join(source_name), source).unwrap();
        let folder = Url::from_directory_path(root).unwrap();
        let main = Url::from_file_path(root.join(main_name)).unwrap();
        let source_uri = Url::from_file_path(root.join(source_name)).unwrap();
        let mut server = Server::start();
        server.request(1, "initialize", json!({"processId":null,"rootUri":folder,"capabilities":{},"initializationOptions":{"build":{"executable":"pdflatex"}}}));
        server.notify("initialized", json!({}));
        server.notify("textDocument/didOpen", json!({"textDocument":{"uri":source_uri,"languageId":"latex","version":1,"text":source}}));
        if !package {
            // No file notification: keep the removed main in the server's index.
            std::fs::remove_file(root.join("a.tex")).unwrap();
        }
        let started = server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[source_uri]}),
        );
        assert_eq!(started["rootUri"], json!(main), "{started}");
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            finished["params"]["status"], "success",
            "{package}: {finished}"
        );
        assert_eq!(finished["params"]["rootUri"], json!(main));
        assert_eq!(finished["params"]["stale"], false);
        assert_eq!(finished["params"]["diagnosticsCount"], 0);
        let stem = std::path::Path::new(main_name)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        assert!(root
            .join("build/latex-lsp")
            .join(format!("{stem}.pdf"))
            .is_file());
        server.request(3, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
fn build_code_lenses_advertise_resolved_commands_and_follow_root_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["a.tex", "b.ltx"] {
        std::fs::write(root.join(name), "\\documentclass{article}\\input{章节}").unwrap();
    }
    std::fs::write(root.join("章节.tex"), "Chapter.").unwrap();
    let folder = Url::from_directory_path(root).unwrap();
    let child = Url::from_file_path(root.join("章节.tex")).unwrap();
    let first = Url::from_file_path(root.join("a.tex")).unwrap();
    let second = Url::from_file_path(root.join("b.ltx")).unwrap();
    let mut server = Server::start();
    let init = server.request(
        1,
        "initialize",
        json!({"processId":null,"rootUri":folder,"capabilities":{"textDocument":{"codeLens":{}}}}),
    );
    assert_eq!(
        init["capabilities"]["codeLensProvider"]["resolveProvider"],
        false
    );
    assert!(init["capabilities"]["executeCommandProvider"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command == "latex.build"));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":child,"languageId":"latex","version":1,"text":"Chapter."}}),
    );
    let result = server.request(
        2,
        "textDocument/codeLens",
        json!({"textDocument":{"uri":child}}),
    );
    assert_eq!(result.as_array().unwrap().len(), 2);
    for (lens, root) in result.as_array().unwrap().iter().zip([&first, &second]) {
        assert_eq!(lens["command"]["command"], "latex.build");
        assert_eq!(lens["command"]["arguments"], json!([root]));
        assert_eq!(
            lens["range"],
            json!({"start":{"line":0,"character":0},"end":{"line":0,"character":0}})
        );
    }
    server.notify(
        "workspace/didChangeConfiguration",
        json!({"settings":{"build":{"rootFile":"b.ltx"}}}),
    );
    let selected = server.request(
        3,
        "textDocument/codeLens",
        json!({"textDocument":{"uri":child}}),
    );
    assert_eq!(selected.as_array().unwrap().len(), 1);
    assert_eq!(selected[0]["command"]["arguments"], json!([child]));
    let missing = server.request(
        4,
        "textDocument/codeLens",
        json!({"textDocument":{"uri":"untitled:missing.tex"}}),
    );
    assert_eq!(missing, json!([]));
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn semantic_tokens_advertise_full_and_range_and_follow_incremental_changes() {
    let mut server = Server::start();
    let init = server.request(1, "initialize", json!({"processId":null,"capabilities":{"textDocument":{"semanticTokens":{"requests":{"full":true,"range":true},"tokenTypes":["macro","variable","type","enumMember","string","comment"],"tokenModifiers":["declaration"],"formats":["relative"]}}}}));
    let provider = &init["capabilities"]["semanticTokensProvider"];
    assert_eq!(provider["full"]["delta"], true);
    assert_eq!(provider["range"], true);
    assert_eq!(provider["legend"]["tokenModifiers"], json!(["declaration"]));
    assert_eq!(
        provider["legend"]["tokenTypes"],
        json!([
            "macro",
            "variable",
            "type",
            "enumMember",
            "string",
            "comment"
        ])
    );
    server.notify("initialized", json!({}));
    let uri = "file:///semantic-protocol/main.tex";
    server.notify("textDocument/didOpen", json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":"😀 \\label{one}\\ref{one}\r\n% 中文😀\r\n"}}));
    let full = server.request(
        2,
        "textDocument/semanticTokens/full",
        json!({"textDocument":{"uri":uri}}),
    );
    assert_eq!(
        full["data"],
        json!([0, 3, 6, 0, 0, 0, 7, 3, 3, 1, 0, 4, 4, 0, 0, 0, 5, 3, 3, 0, 1, 0, 6, 5, 0])
    );
    let range = server.request(3, "textDocument/semanticTokens/range", json!({"textDocument":{"uri":uri},"range":{"start":{"line":1,"character":2},"end":{"line":1,"character":6}}}));
    assert_eq!(range["data"], json!([1, 2, 4, 5, 0]));
    server.send(json!({"jsonrpc":"2.0","id":4,"method":"textDocument/semanticTokens/range","params":{"textDocument":{"uri":uri},"range":{"start":{"line":1,"character":5},"end":{"line":1,"character":6}}}}));
    assert_eq!(
        server.until(|message| message["id"] == 4)["error"]["code"],
        -32602
    );
    // No semantic cache may retain the old line layout or deleted comment.
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"range":{"start":{"line":1,"character":0},"end":{"line":2,"character":0}},"text":"\\ref{two}\n"}]}));
    let updated = server.request(
        5,
        "textDocument/semanticTokens/full",
        json!({"textDocument":{"uri":uri}}),
    );
    let expected = json!([
        0, 3, 6, 0, 0, 0, 7, 3, 3, 1, 0, 4, 4, 0, 0, 0, 5, 3, 3, 0, 1, 0, 4, 0, 0, 0, 5, 3, 3, 0
    ]);
    assert_eq!(updated["data"], expected);
    let missing = server.request(
        6,
        "textDocument/semanticTokens/full",
        json!({"textDocument":{"uri":"file:///semantic-protocol/missing.tex"}}),
    );
    assert!(missing.is_null());
    server.request(7, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn semantic_token_deltas_use_flat_array_offsets_and_fall_back_after_close_or_stale_ids() {
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"capabilities":{"textDocument":{"semanticTokens":{"requests":{"full":{"delta":true},"range":true},"tokenTypes":["macro","enumMember"],"tokenModifiers":["declaration"],"formats":["relative"]}}}}));
    server.notify("initialized", json!({}));
    let uri = "file:///semantic-delta/main.tex";
    let source = "\\label{key}\\ref{key}\\ref{key}\\ref{key}\n";
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":source}}),
    );
    let initial = server.request(
        2,
        "textDocument/semanticTokens/full",
        json!({"textDocument":{"uri":uri}}),
    );
    let initial_id = initial["resultId"].as_str().unwrap();
    // A range response has its own absolute positions and must not replace
    // the full snapshot used by the client's next delta request.
    let range = server.request(3, "textDocument/semanticTokens/range", json!({"textDocument":{"uri":uri},"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":6}}}));
    assert!(range.get("resultId").is_none());
    let unchanged = server.request(
        4,
        "textDocument/semanticTokens/full/delta",
        json!({"textDocument":{"uri":uri},"previousResultId":initial_id}),
    );
    assert_eq!(unchanged["edits"], json!([]));
    let next_id = unchanged["resultId"].as_str().unwrap();
    assert_ne!(next_id, initial_id);
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"text":"\n"}]}));
    let delta = server.request(
        5,
        "textDocument/semanticTokens/full/delta",
        json!({"textDocument":{"uri":uri},"previousResultId":next_id}),
    );
    assert_eq!(
        delta["edits"],
        json!([{"start":0,"deleteCount":5,"data":[1,0,6,0,0]}])
    );
    let mut reconstructed = initial["data"].as_array().unwrap().clone();
    reconstructed.splice(0..5, delta["edits"][0]["data"].as_array().unwrap().clone());
    let verified = server.request(
        6,
        "textDocument/semanticTokens/full",
        json!({"textDocument":{"uri":uri}}),
    );
    assert_eq!(verified["data"], Value::Array(reconstructed));
    // An older result or another file's result must receive complete data.
    let stale = server.request(
        7,
        "textDocument/semanticTokens/full/delta",
        json!({"textDocument":{"uri":uri},"previousResultId":next_id}),
    );
    assert_eq!(stale["data"], verified["data"]);
    let other = "file:///semantic-delta/other.tex";
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":other,"languageId":"latex","version":1,"text":source}}),
    );
    let isolated = server.request(
        8,
        "textDocument/semanticTokens/full/delta",
        json!({"textDocument":{"uri":other},"previousResultId":stale["resultId"]}),
    );
    assert_eq!(isolated["data"], initial["data"]);
    server.notify("textDocument/didClose", json!({"textDocument":{"uri":uri}}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":source}}),
    );
    let reopened = server.request(
        9,
        "textDocument/semanticTokens/full/delta",
        json!({"textDocument":{"uri":uri},"previousResultId":stale["resultId"]}),
    );
    assert_eq!(reopened["data"], initial["data"]);
    server.request(10, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn destination_diagnostics_exclude_chapters_only_probed_by_the_tex_recorder() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let active = Url::from_file_path(root.join("active.tex")).unwrap();
    let excluded = Url::from_file_path(root.join("excluded.tex")).unwrap();
    let main_source = "\\documentclass{article}\\usepackage{hyperref}\\includeonly{active}\n\\begin{document}\\hypertarget{last}{Candidate}\\hypertarget{section.1}{Explicit}\\section{Automatic}\\include{active}\\include{excluded}\\end{document}\n";
    let active_source = "\\hyperlink{lost}{Active}\\hyperdef{}{section.1}{Active target}\n";
    let excluded_source = "\\hyperlink{lost}{Excluded}\\hyperdef{}{section.1}{Excluded target}\\hypertarget{loft}{Excluded candidate}\n";
    for (uri, source) in [
        (&main, main_source),
        (&active, active_source),
        (&excluded, excluded_source),
    ] {
        std::fs::write(uri.to_file_path().unwrap(), source).unwrap();
    }
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{"textDocument":{"publishDiagnostics":{"relatedInformation":true},"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix"]}}}}},"initializationOptions":{"build":{"executable":"pdflatex","arguments":["-interaction=nonstopmode","-recorder","-output-directory=%o","%f"]}}}));
    server.notify("initialized", json!({}));
    for (uri, text) in [
        (&main, main_source),
        (&active, active_source),
        (&excluded, excluded_source),
    ] {
        server.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":text}}),
        );
    }
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], false, "{finished}");
    for (uri, other) in [(&main, &active), (&active, &main)] {
        let published = server.until(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri.as_str()
                && message["params"]["diagnostics"]
                    .as_array()
                    .is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item["code"] == "duplicate-pdf-destination")
                    })
        });
        let duplicate = published["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["code"] == "duplicate-pdf-destination")
            .unwrap();
        let related = duplicate["relatedInformation"].as_array().unwrap();
        assert_eq!(related.len(), 1, "{duplicate}");
        assert_eq!(related[0]["location"]["uri"], other.as_str());
        if uri == &active {
            assert!(
                published["params"]["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["code"] == "undefined-pdf-destination"),
                "{published}"
            );
        }
    }
    let range = latex_lsp::text::Text::new(excluded_source.into()).range(11, 15);
    let actions = server.request(
        3,
        "textDocument/codeAction",
        json!({"textDocument":{"uri":excluded},"range":range,"context":{"diagnostics":[]}}),
    );
    assert!(actions.as_array().unwrap().is_empty(), "{actions}");
    let range = latex_lsp::text::Text::new(active_source.into()).range(11, 15);
    let actions = server.request(
        4,
        "textDocument/codeAction",
        json!({"textDocument":{"uri":active},"range":range,"context":{"diagnostics":[]}}),
    );
    assert_eq!(actions.as_array().unwrap().len(), 1, "{actions}");
    assert_eq!(
        actions[0]["edit"]["changes"][active.as_str()][0]["newText"],
        "last"
    );
    let all_chapters =
        main_source.replace("\\includeonly{active}", "\\includeonly{active,excluded}");
    std::fs::write(root.join("main.tex"), &all_chapters).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":all_chapters}]}),
    );
    server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    let published = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == excluded.as_str()
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item["code"] == "duplicate-pdf-destination")
                })
    });
    let duplicate = published["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "duplicate-pdf-destination")
        .unwrap();
    assert_eq!(duplicate["relatedInformation"].as_array().unwrap().len(), 2);
    let actions = server.request(
        6,
        "textDocument/codeAction",
        json!({"textDocument":{"uri":excluded},"range":range,"context":{"diagnostics":[]}}),
    );
    assert_eq!(actions.as_array().unwrap().len(), 2, "{actions}");
    server.request(7, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn duplicate_compiler_destinations_link_declarations_and_clear_after_resolving_then_renaming() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let child = Url::from_file_path(root.join("chapter.tex")).unwrap();
    let main_source = "\\documentclass{article}\\usepackage{hyperref}\\begin{document}\n\\hypertarget{section.1}{Explicit}\\section{Automatic}\n\\def\\chapterfile{chapter}\\input{\\chapterfile}\\end{document}\n";
    let child_source = "\\hyperdef[bookmark-label]{}{section.1}{Other}\n\\hyperlink{section.1}{Link}\\label{section.1}\n";
    std::fs::write(root.join("main.tex"), main_source).unwrap();
    std::fs::write(root.join("chapter.tex"), child_source).unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}},"textDocument":{"publishDiagnostics":{"relatedInformation":true},"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix"]}}}}},"initializationOptions":{"build":{"executable":"pdflatex","arguments":["-interaction=nonstopmode","-recorder","-output-directory=%o","%f"]}}}));
    server.notify("initialized", json!({}));
    for (uri, text) in [(&main, main_source), (&child, child_source)] {
        server.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":text}}),
        );
    }
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], false, "{finished}");
    for (id, uri, other) in [(3, &main, &child), (4, &child, &main)] {
        let published = server.until(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri.as_str()
                && message["params"]["diagnostics"]
                    .as_array()
                    .is_some_and(|items| {
                        items
                            .iter()
                            .any(|item| item["code"] == "duplicate-pdf-destination")
                    })
        });
        let duplicates: Vec<_> = published["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["code"] == "duplicate-pdf-destination")
            .collect();
        assert_eq!(duplicates.len(), 1, "{published}");
        let diagnostic = duplicates[0];
        assert_eq!(diagnostic["data"]["destination"], "section.1");
        assert_eq!(
            diagnostic["relatedInformation"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            diagnostic["relatedInformation"][0]["location"]["uri"],
            other.as_str()
        );
        let actions = server.request(id, "textDocument/codeAction", json!({"textDocument":{"uri":uri},"range":diagnostic["range"],"context":{"diagnostics":[diagnostic]}}));
        assert!(actions.as_array().unwrap().is_empty(), "{actions}");
    }
    let position = latex_lsp::text::Text::new(main_source.into())
        .position(main_source.find("section.1").unwrap() + 2);
    server.send(json!({"jsonrpc":"2.0","id":5,"method":"textDocument/rename","params":{"textDocument":{"uri":main},"position":position,"newName":"custom.section"}}));
    let rejected = server.until(|message| message["id"] == 5);
    assert!(rejected.get("error").is_some(), "{rejected}");
    let child_repaired = "\\hyperlink{section.1}{Link}\\label{section.1}\n";
    std::fs::write(root.join("chapter.tex"), child_repaired).unwrap();
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":child,"version":2},"contentChanges":[{"text":child_repaired}]}));
    let edit = server.request(
        6,
        "textDocument/rename",
        json!({"textDocument":{"uri":main},"position":position,"newName":"custom.section"}),
    );
    let changes = edit["documentChanges"].as_array().unwrap();
    assert_eq!(changes.len(), 2, "{edit}");
    for change in changes {
        let uri: Url = serde_json::from_value(change["textDocument"]["uri"].clone()).unwrap();
        let (source, version) = if uri == main {
            (main_source, 2)
        } else {
            (child_repaired, 3)
        };
        let mut edits: Vec<tower_lsp::lsp_types::TextEdit> =
            serde_json::from_value(change["edits"].clone()).unwrap();
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
        let changed = latex_lsp::text::Text::new(source.into())
            .changed(
                edits
                    .into_iter()
                    .map(
                        |edit| tower_lsp::lsp_types::TextDocumentContentChangeEvent {
                            range: Some(edit.range),
                            range_length: None,
                            text: edit.new_text,
                        },
                    )
                    .collect(),
            )
            .unwrap()
            .source;
        if uri == main {
            assert!(changed.contains("\\hypertarget{custom.section}{Explicit}\\section{Automatic}"));
        } else {
            assert_eq!(
                changed,
                "\\hyperlink{custom.section}{Link}\\label{section.1}\n"
            );
        }
        std::fs::write(uri.to_file_path().unwrap(), &changed).unwrap();
        server.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":changed}]}));
    }
    for id in [7, 8] {
        server.request(
            id,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[child]}),
        );
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "success", "{finished}");
        if id == 8 {
            assert_eq!(finished["params"]["diagnosticsCount"], 0, "{finished}");
        }
    }
    server.request(9, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires pdflatex and hyperref"]
fn compiler_destination_diagnostics_follow_recorded_children_and_offer_preserving_fixes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let child = Url::from_file_path(root.join("chapter.tex")).unwrap();
    let main_source = "\\documentclass{article}\\usepackage{hyperref}\n\\begin{document}\\section{Title}\\hyperdef{section}{intro}{Target}\n\\def\\chapterfile{chapter}\\input{\\chapterfile}\\end{document}\n";
    let source = "\\hyperlink{section.itnro}{Direct}\\label{section.itnro}\n\\hyperref{}{section}% keep\n{itnro}{Compound}\n";
    std::fs::write(root.join("main.tex"), main_source).unwrap();
    std::fs::write(root.join("chapter.tex"), source).unwrap();
    std::fs::write(
        root.join("unrelated.tex"),
        "\\hyperlink{section.itnro}{Other}",
    )
    .unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}},"textDocument":{"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix"]}}}}},"initializationOptions":{"build":{"executable":"pdflatex","arguments":["-interaction=nonstopmode","-recorder","-output-directory=%o","%f"]}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":main,"languageId":"latex","version":1,"text":main_source}}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], false, "{finished}");
    let published = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == child.as_str()
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| {
                    items.len() == 2
                        && items
                            .iter()
                            .all(|item| item["code"] == "undefined-pdf-destination")
                })
    });
    let mut edits = Vec::new();
    for (i, diagnostic) in published["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        assert_eq!(diagnostic["data"]["destination"], "section.itnro");
        let actions = server.request(3 + i as u64, "textDocument/codeAction", json!({"textDocument":{"uri":child},"range":diagnostic["range"],"context":{"diagnostics":[diagnostic]}}));
        let actions = actions.as_array().unwrap();
        assert_eq!(actions.len(), 1, "{actions:?}");
        assert_eq!(
            actions[0]["title"],
            "Replace with PDF destination `section.intro`"
        );
        let change = &actions[0]["edit"]["documentChanges"][0];
        assert_eq!(change["textDocument"]["uri"], child.as_str());
        assert!(change["textDocument"]["version"].is_null());
        edits.push(
            serde_json::from_value::<tower_lsp::lsp_types::TextEdit>(change["edits"][0].clone())
                .unwrap(),
        );
    }
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let repaired = latex_lsp::text::Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .map(
                    |edit| tower_lsp::lsp_types::TextDocumentContentChangeEvent {
                        range: Some(edit.range),
                        range_length: None,
                        text: edit.new_text,
                    },
                )
                .collect(),
        )
        .unwrap()
        .source;
    assert_eq!(repaired, "\\hyperlink{section.intro}{Direct}\\label{section.itnro}\n\\hyperref{}{section}% keep\n{intro}{Compound}\n");
    std::fs::write(root.join("chapter.tex"), &repaired).unwrap();
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":child,"languageId":"latex","version":1,"text":repaired}}),
    );
    for id in [5, 6] {
        server.request(
            id,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[child]}),
        );
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "success", "{finished}");
        assert_eq!(finished["params"]["diagnosticsCount"], 0, "{finished}");
    }
    server.request(7, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

fn lowercase_uri_escapes(uri: &Url) -> String {
    let mut bytes = uri.as_str().as_bytes().to_vec();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' {
            bytes[i + 1].make_ascii_lowercase();
            bytes[i + 2].make_ascii_lowercase();
            i += 3;
        } else {
            i += 1;
        }
    }
    String::from_utf8(bytes).unwrap()
}

#[test]
fn equivalent_file_uris_share_open_versions_navigation_rename_watch_and_close_state() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let main_encoded = main.as_str().replace("main.tex", "%6dain.tex");
    let child = Url::from_file_path(root.join("章节.tex")).unwrap();
    let child_lower = lowercase_uri_escapes(&child);
    let main_source = "\\documentclass{article}\\input{章节}\\ref{draft}\n";
    let disk_source = "\\label{saved}\\ref{saved}\n";
    let open_source = "😀 \\label{draft}\\ref{draft}\n";
    std::fs::write(root.join("main.tex"), main_source).unwrap();
    std::fs::write(root.join("章节.tex"), disk_source).unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}}}}));
    server.notify("initialized", json!({}));
    server.notify("textDocument/didOpen", json!({"textDocument":{"uri":main_encoded,"languageId":"latex","version":4,"text":main_source}}));
    server.notify("textDocument/didOpen", json!({"textDocument":{"uri":child_lower,"languageId":"latex","version":1,"text":open_source}}));
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":child,"type":2}]}),
    );
    let symbols = server.request(
        2,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":child}}),
    );
    assert_eq!(symbols[0]["name"], "draft", "{symbols}");
    let main_position = latex_lsp::text::Text::new(main_source.into())
        .position(main_source.find("draft").unwrap() + 1);
    let definitions = server.request(
        3,
        "textDocument/definition",
        json!({"textDocument":{"uri":main_encoded},"position":main_position}),
    );
    assert_eq!(definitions.as_array().unwrap().len(), 1, "{definitions}");
    assert_eq!(definitions[0]["uri"], child.as_str());
    let changed = "😀 \\label{draft}\\ref{draft}\\ref{draft}\n";
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":child,"version":2},"contentChanges":[{"text":changed}]}),
    );
    let position =
        latex_lsp::text::Text::new(changed.into()).position(changed.find("draft").unwrap() + 1);
    let references = server.request(4, "textDocument/references", json!({"textDocument":{"uri":child_lower},"position":position,"context":{"includeDeclaration":true}}));
    assert_eq!(references.as_array().unwrap().len(), 4, "{references}");
    let edit = server.request(
        5,
        "textDocument/rename",
        json!({"textDocument":{"uri":child_lower},"position":position,"newName":"release"}),
    );
    let changes = edit["documentChanges"].as_array().unwrap();
    assert_eq!(changes.len(), 2, "{edit}");
    let child_change = changes
        .iter()
        .find(|change| change["textDocument"]["uri"] == child.as_str())
        .unwrap();
    assert_eq!(child_change["textDocument"]["version"], 2);
    assert_eq!(child_change["edits"].as_array().unwrap().len(), 3);
    let main_change = changes
        .iter()
        .find(|change| change["textDocument"]["uri"] == main.as_str())
        .unwrap();
    assert_eq!(main_change["textDocument"]["version"], 4);
    let workspace_symbols = server.request(6, "workspace/symbol", json!({"query":"draft"}));
    assert_eq!(
        workspace_symbols.as_array().unwrap().len(),
        1,
        "{workspace_symbols}"
    );
    server.notify(
        "textDocument/didClose",
        json!({"textDocument":{"uri":child_lower}}),
    );
    let symbols = server.request(
        7,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":child_lower}}),
    );
    assert_eq!(symbols[0]["name"], "saved", "{symbols}");
    server.request(8, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires pdflatex"]
fn lowercase_unicode_uris_keep_root_detection_build_diagnostics_and_status() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let child = Url::from_file_path(root.join("章节.tex")).unwrap();
    let child_lower = lowercase_uri_escapes(&child);
    let main_source = "\\documentclass{article}\\begin{document}\\input{章节}\\end{document}\n";
    let source = "\\firstbad and \\secondbad.\n";
    std::fs::write(root.join("main.tex"), main_source).unwrap();
    std::fs::write(root.join("章节.tex"), source).unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{},"initializationOptions":{"build":{"executable":"pdflatex","arguments":["-interaction=nonstopmode","-file-line-error","-output-directory=%o","%f"]}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":child_lower,"languageId":"latex","version":1,"text":source}}),
    );
    let build = server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[child_lower]}),
    );
    assert_eq!(build["rootUri"], main.as_str(), "{build}");
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["errorsCount"], 2, "{finished}");
    assert_eq!(finished["params"]["stale"], false);
    let published = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == child.as_str()
            && message["params"]["version"] == 1
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| items.len() == 2)
    });
    assert_eq!(
        published["params"]["diagnostics"][0]["code"],
        "undefined-control-sequence"
    );
    let status = server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[{"uri":child_lower}]}),
    );
    assert_eq!(status["buildId"], finished["params"]["buildId"]);
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":child_lower,"version":2},"contentChanges":[{"text":"Corrected.\n"}]}));
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == child.as_str()
            && message["params"]["version"] == 2
            && message["params"]["diagnostics"] == json!([])
    });
    server.send(json!({"jsonrpc":"2.0","id":4,"method":"workspace/executeCommand","params":{"command":"latex.build","arguments":[child_lower]}}));
    let response = server.until(|message| message["id"] == 4);
    assert_eq!(response["error"]["code"], -32602, "{response}");
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires pdflatex"]
fn real_command_spelling_fixes_preserve_arguments_and_compile_after_repair() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let source = "\\documentclass{article}\\input{definitions}\n\\begin{document}\n\\sectoin{Intro} \\reporttitel{Existing argument}\n\\end{document}\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(
        root.join("definitions.tex"),
        "\\newcommand{\\reporttitle}[1]{\\textbf{#1}}\n",
    )
    .unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}},"textDocument":{"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix"]}}}}},"initializationOptions":{"build":{"executable":"pdflatex","arguments":["-interaction=nonstopmode","-file-line-error","-output-directory=%o","%f"]}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":source}}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["errorsCount"], 2, "{finished}");
    let published = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == uri.as_str()
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| {
                    items.len() == 2
                        && items
                            .iter()
                            .all(|item| item["code"] == "undefined-control-sequence")
                })
    });
    let mut edits = Vec::new();
    for (index, diagnostic) in published["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let actions = server.request(3 + index as u64, "textDocument/codeAction", json!({"textDocument":{"uri":uri},"range":diagnostic["range"],"context":{"diagnostics":[diagnostic],"only":["quickfix"]}}));
        let replacement = if diagnostic["data"]["command"] == "sectoin" {
            "\\section"
        } else {
            "\\reporttitle"
        };
        let action = actions
            .as_array()
            .unwrap()
            .iter()
            .find(|action| {
                action["edit"]["documentChanges"][0]["edits"][0]["newText"] == replacement
            })
            .unwrap_or_else(|| panic!("{actions}"));
        assert_eq!(action["isPreferred"], false);
        let change = &action["edit"]["documentChanges"][0];
        assert_eq!(change["textDocument"]["version"], 1);
        edits.push(
            serde_json::from_value::<tower_lsp::lsp_types::TextEdit>(change["edits"][0].clone())
                .unwrap(),
        );
    }
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let repaired = latex_lsp::text::Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .map(
                    |edit| tower_lsp::lsp_types::TextDocumentContentChangeEvent {
                        range: Some(edit.range),
                        range_length: None,
                        text: edit.new_text,
                    },
                )
                .collect(),
        )
        .unwrap()
        .source;
    assert!(repaired.contains("\\section{Intro} \\reporttitle{Existing argument}"));
    std::fs::write(root.join("main.tex"), &repaired).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":repaired}]}),
    );
    server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["pages"], 1);
    assert_eq!(finished["params"]["diagnosticsCount"], 0);
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
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
            stdin: Some(stdin),
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
        let stdin = self.stdin.as_mut().unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n", bytes.len()).unwrap();
        stdin.write_all(&bytes).unwrap();
        stdin.flush().unwrap();
    }

    fn until(&self, matches: impl Fn(&Value) -> bool) -> Value {
        self.until_with_timeout(Duration::from_secs(10), matches)
    }

    fn until_with_timeout(&self, timeout: Duration, matches: impl Fn(&Value) -> bool) -> Value {
        let existing = self.pending.borrow().iter().position(&matches);
        if let Some(index) = existing {
            return self.pending.borrow_mut().remove(index);
        }
        let deadline = std::time::Instant::now() + timeout;
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

#[test]
fn duplicate_project_diagnostics_refresh_related_open_documents_after_edits_and_disk_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let child = Url::from_file_path(root.join("chapter.tex")).unwrap();
    let main_source = "\\documentclass{article}\n\\input{chapter}\n\\label{shared}\n";
    let child_source = "😀 \\label{shared}\n";
    std::fs::write(root.join("main.tex"), main_source).unwrap();
    std::fs::write(root.join("chapter.tex"), child_source).unwrap();
    let mut server = Server::start();
    server.request(1,"initialize",json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{"textDocument":{"publishDiagnostics":{"relatedInformation":true}}}}));
    server.notify("initialized", json!({}));
    for (uri, source) in [(&main, main_source), (&child, child_source)] {
        server.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"latex","version":1,"text":source}}),
        );
        let diagnostics = server.until(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri.as_str()
                && message["params"]["version"] == 1
        });
        let diagnostics = diagnostics["params"]["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0]["code"], "duplicate-definition");
        let other = if uri == &main { &child } else { &main };
        assert_eq!(
            diagnostics[0]["relatedInformation"][0]["location"]["uri"],
            other.as_str()
        );
    }
    // Drain initial notifications before checking a same-version refresh.
    server.request(
        2,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":main}}),
    );
    server.pending.borrow_mut().clear();
    server.notify("textDocument/didChange",json!({"textDocument":{"uri":child,"version":2},"contentChanges":[{"text":"\\label{unique}\n"}]}));
    for (uri, version) in [(&main, 1), (&child, 2)] {
        server.until(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri.as_str()
                && message["params"]["version"] == version
                && message["params"]["diagnostics"] == json!([])
        });
    }
    // Closing a changed buffer restores its saved declaration and the conflict.
    server.notify(
        "textDocument/didClose",
        json!({"textDocument":{"uri":child}}),
    );
    let diagnostics = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == main.as_str()
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
    });
    assert_eq!(
        diagnostics["params"]["diagnostics"][0]["code"],
        "duplicate-definition"
    );
    std::fs::write(root.join("chapter.tex"), "\\label{disk:unique}\n").unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":child,"type":2}]}),
    );
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == main.as_str()
            && message["params"]["diagnostics"] == json!([])
    });
    server.request(3, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn builds_with_the_same_job_name_cannot_write_the_same_artifacts_concurrently() {
    let (mut server, directory, main) = fake_build_server("touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/$3.pdf\"\n", false);
    let root = directory.path();
    let other = Url::from_file_path(root.join("other.tex")).unwrap();
    std::fs::write(root.join("other.tex"), "\\documentclass{article}Other.").unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":other,"type":1}]}),
    );
    server.notify("workspace/didChangeConfiguration",json!({"settings":{"build":{"executable":"/bin/sh","jobName":"draft","arguments":[root.join("compiler.sh"),"%o","%f","%j"]}}}));
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let started = server.until(|message| message["method"] == "latex/buildStarted");
    assert_eq!(started["params"]["jobName"], "draft");
    let ready = root.join("build/latex-lsp/ready");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    server.send(json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":{"command":"latex.build","arguments":[other]}}));
    let rejected = server.until(|message| message["id"] == 3);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("same output file"),
        "{rejected}"
    );
    std::fs::write(root.join("build/latex-lsp/release"), "").unwrap();
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[other]}),
    );
    let second = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(second["params"]["status"], "success", "{second}");
    assert_eq!(second["params"]["rootUri"], other.as_str());
    assert_eq!(second["params"]["jobName"], "draft");
    assert_eq!(
        second["params"]["pdfUri"],
        Url::from_file_path(root.join("build/latex-lsp/draft.pdf"))
            .unwrap()
            .as_str()
    );
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn single_quoted_package_citation_warnings_offer_precise_spelling_fixes() {
    let script = "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf \"Package biblatex Warning: Citation 'smoth2026' undefined on input line 3.\\n\" > \"$1/main.log\"\n";
    let (mut server, directory, main) = fake_build_server_capabilities(
        script,
        false,
        json!({"textDocument":{"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix"]}}}}}),
    );
    let source = "\\documentclass{article}\n\\addbibresource{refs.bib}\n\\autocite{smoth2026}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(
        directory.path().join("refs.bib"),
        "@book{smith2026,title={Example}}\n",
    )
    .unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    let message = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|diagnostic| diagnostic["source"] == "latex-build")
                })
    });
    let diagnostic = message["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["source"] == "latex-build")
        .unwrap();
    assert_eq!(
        diagnostic["range"],
        json!({"start":{"line":2,"character":10},"end":{"line":2,"character":19}})
    );
    let actions = server.request(3,"textDocument/codeAction",json!({"textDocument":{"uri":main},"range":diagnostic["range"],"context":{"diagnostics":[diagnostic],"only":["quickfix"]}}));
    assert_eq!(actions.as_array().unwrap().len(), 1, "{actions}");
    assert_eq!(
        actions[0]["edit"]["changes"][main.as_str()][0]["newText"],
        "smith2026"
    );
    assert_eq!(
        actions[0]["edit"]["changes"][main.as_str()][0]["range"],
        diagnostic["range"]
    );
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn missing_included_sources_publish_path_warnings_and_clear_after_the_next_successful_build() {
    let script = "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nif [ ! -f 'missing chapter.tex' ]; then\n printf 'No file main.aux.\\nNo file missing chapter.tex.\\n' > \"$1/main.log\"\nelse\n : > \"$1/main.log\"\nfi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let source = "\\documentclass{article}\n\\include{missing chapter}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success");
    assert_eq!(first["params"]["diagnosticsCount"], 1);
    let diagnostics = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == main.as_str()
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|diagnostics| {
                    diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic["code"] == "missing-include")
                })
    });
    assert_eq!(diagnostics["params"]["version"], 2);
    assert_eq!(
        diagnostics["params"]["diagnostics"][0]["range"],
        json!({"start":{"line":1,"character":9},"end":{"line":1,"character":24}})
    );
    server.request(
        3,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":main}}),
    );
    server.pending.borrow_mut().clear();
    let restored = Url::from_file_path(directory.path().join("missing chapter.tex")).unwrap();
    std::fs::write(restored.to_file_path().unwrap(), "Restored chapter.\n").unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":restored,"type":1}]}),
    );
    server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let repaired = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(repaired["params"]["status"], "success");
    assert_eq!(repaired["params"]["diagnosticsCount"], 0);
    assert_eq!(repaired["params"]["dependenciesCount"], 2);
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == main.as_str()
            && message["params"]["diagnostics"] == json!([])
    });
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn local_bibtex_styles_require_saving_and_edits_during_compilation_make_results_stale() {
    let (mut server,directory,main) = fake_build_server("touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n",false);
    let root = directory.path();
    let style = Url::from_file_path(root.join("local.bst")).unwrap();
    let source = "\\documentclass{article}\n\\bibliographystyle{local}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(style.to_file_path().unwrap(), "Original style\n").unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.notify("textDocument/didOpen",json!({"textDocument":{"uri":style,"languageId":"bst","version":1,"text":"Unsaved style with }\n"}}));
    let diagnostics = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == style.as_str()
    });
    assert_eq!(diagnostics["params"]["diagnostics"], json!([]));
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"latex.build","arguments":[main]}}));
    let rejected = server.until(|message| message["id"] == 2);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("local.bst"),
        "{rejected}"
    );
    std::fs::write(style.to_file_path().unwrap(), "Unsaved style with }\n").unwrap();
    server.notify(
        "textDocument/didSave",
        json!({"textDocument":{"uri":style}}),
    );
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[style]}),
    );
    let ready = root.join("build/latex-lsp/ready");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    server.notify("textDocument/didChange",json!({"textDocument":{"uri":style,"version":2},"contentChanges":[{"text":"Changed during compilation\n"}]}));
    // Wait for the edit to be applied before releasing the fake compiler.
    server.request(
        4,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":style}}),
    );
    std::fs::write(root.join("build/latex-lsp/release"), "").unwrap();
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["rootUri"], main.as_str());
    assert_eq!(finished["params"]["stale"], true, "{finished}");
    assert_eq!(finished["params"]["dependenciesCount"], 2);
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires pdflatex and synctex"]
fn real_custom_job_names_preserve_build_analysis_recorded_dependencies_and_synctex_round_trips() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let child = Url::from_file_path(root.join("chapter.tex")).unwrap();
    std::fs::write(root.join("main.tex"), "\\documentclass{article}\n\\newcommand{\\chapterfile}{chapter}\n\\begin{document}\n\\input{\\chapterfile}\n\\end{document}\n").unwrap();
    std::fs::write(
        root.join("chapter.tex"),
        "Chapter text.\\PackageWarning{example}{Named job warning}\n",
    )
    .unwrap();
    let mut server = Server::start();
    server.request(1,"initialize",json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{},"initializationOptions":{"build":{"executable":"pdflatex","jobName":"论文 draft.v2"}}}));
    server.notify("initialized", json!({}));
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    let result = &finished["params"];
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["stale"], false);
    assert_eq!(result["jobName"], "论文 draft.v2");
    assert_eq!(result["dependenciesCount"], 2);
    let pdf = Url::from_file_path(root.join("build/latex-lsp/论文 draft.v2.pdf")).unwrap();
    assert_eq!(result["pdfUri"], pdf.as_str());
    assert_eq!(
        result["logUri"],
        Url::from_file_path(root.join("build/latex-lsp/论文 draft.v2.log"))
            .unwrap()
            .as_str()
    );
    let diagnostics = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == child.as_str()
    });
    assert!(diagnostics["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("Named job warning"))));
    let forward = server.request(3,"workspace/executeCommand",json!({"command":"latex.forwardSearch","arguments":[{"uri":child,"position":{"line":0,"character":4}}]}));
    assert_eq!(forward["pdfUri"], pdf.as_str());
    let hit = &forward["locations"][0];
    assert!(hit["page"].as_u64().unwrap() > 0);
    let inverse = server.request(4,"workspace/executeCommand",json!({"command":"latex.inverseSearch","arguments":[{"uri":main,"page":hit["page"],"x":hit["x"],"y":hit["y"]}]}));
    assert_eq!(inverse["pdfUri"], pdf.as_str());
    assert!(inverse["locations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|location| location["uri"] == child.as_str()));
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
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
    let pulls_diagnostics = capabilities.pointer("/textDocument/diagnostic").is_some();
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
    if pulls_diagnostics {
        server.request(
            0,
            "textDocument/documentSymbol",
            json!({"textDocument":{"uri":uri}}),
        );
    } else {
        server.until(|message| message["method"] == "textDocument/publishDiagnostics");
    }
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
fn bibliography_styles_invalidate_old_pdfs_even_when_missing_from_tex_recorder() {
    let script = "if [ ! -f \"$1/main.pdf\" ]; then\n printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n printf 'INPUT main.tex\\n' > \"$1/main.fls\"\nfi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let source = "\\documentclass{article}\n\\bibliographystyle{local}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    let style = directory.path().join("local.bst");
    std::fs::write(&style, "Original style\n").unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    let pdf = directory.path().join("build/latex-lsp/main.pdf");
    let pdf_time = std::fs::metadata(&pdf).unwrap().modified().unwrap();
    std::fs::write(&style, "Changed style\n").unwrap();
    std::fs::File::open(&style)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(pdf_time + Duration::from_secs(1)))
        .unwrap();
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let outdated = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(outdated["params"]["status"], "error", "{outdated}");
    assert_eq!(outdated["params"]["stale"], false);
    assert!(outdated["params"]["message"]
        .as_str()
        .unwrap()
        .contains("local.bst"));
    assert!(outdated["params"]["pdfUri"].is_null());
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
fn cancelling_before_progress_acknowledgement_preserves_previous_build_artifacts() {
    for existing in [false, true] {
        for progress_cancel in [false, true] {
            let (mut server, directory, uri) = fake_build_server_capabilities(
                "touch compiler-started\nprintf 'new log' > \"$1/main.log\"\n",
                false,
                json!({"window":{"workDoneProgress":true}}),
            );
            let output = directory.path().join("build/latex-lsp");
            let extensions = ["log", "pdf", "fls", "blg", "aux", "synctex.gz"];
            if existing {
                std::fs::create_dir_all(&output).unwrap();
                for extension in extensions {
                    std::fs::write(output.join(format!("main.{extension}")), "previous build")
                        .unwrap();
                }
            }
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[uri]}),
            );
            let create =
                server.until(|message| message["method"] == "window/workDoneProgress/create");
            if progress_cancel {
                server.notify(
                    "window/workDoneProgress/cancel",
                    json!({"token":create["params"]["token"]}),
                );
            } else {
                let result = server.request(
                    3,
                    "workspace/executeCommand",
                    json!({"command":"latex.cancelBuild","arguments":[uri]}),
                );
                assert_eq!(result["cancelled"], 1);
            }
            // This response establishes wire order before acknowledging progress.
            server.request(4, "workspace/symbol", json!({"query":"Title"}));
            server.send(json!({"jsonrpc":"2.0","id":create["id"],"result":null}));
            let finished = server.until(|message| message["method"] == "latex/buildFinished");
            let summary = &finished["params"];
            assert_eq!(summary["status"], "cancelled", "{finished}");
            assert_eq!(output.exists(), existing);
            if existing {
                for extension in extensions {
                    assert_eq!(
                        std::fs::read_to_string(output.join(format!("main.{extension}"))).unwrap(),
                        "previous build"
                    );
                }
            }
            assert_eq!(summary["exitCode"], Value::Null);
            assert_eq!(summary["logUri"], Value::Null);
            assert_eq!(summary["pdfUri"], Value::Null);
            assert_eq!(summary["diagnosticsCount"], 0);
            assert_eq!(summary["output"], "");
            assert!(!directory.path().join("compiler-started").exists());
            server.request(5, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
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
fn build_summaries_use_final_logs_actual_pdf_sizes_and_clear_artifacts_after_errors() {
    let script = "printf 'Output written on old.pdf (9 pages, 900 bytes).\\n'\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: final pass warning.\\nOutput written on main.pdf (2 pages, 999 bytes).\\n' \"$2\" > \"$1/main.log\"\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    let summary = &finished["params"];
    assert_eq!(summary["status"], "success");
    assert_eq!(summary["pages"], 2);
    assert_eq!(summary["pdfSize"], 9);
    assert_eq!(summary["diagnosticsCount"], 1);
    assert_eq!(summary["errorsCount"], 0);
    assert_eq!(summary["warningsCount"], 1);
    assert_eq!(summary["diagnosticsTruncated"], false);
    let status = server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[main]}),
    );
    assert_eq!(&status, summary);
    std::fs::write(directory.path().join("compiler.sh"), "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: Undefined control sequence.\\nOutput written on main.pdf (3 pages, 999 bytes).\\n' \"$2\" > \"$1/main.log\"\n").unwrap();
    server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let failed = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(failed["params"]["status"], "error", "{failed}");
    assert!(failed["params"]["pages"].is_null());
    assert!(failed["params"]["pdfSize"].is_null());
    assert_eq!(failed["params"]["errorsCount"], 1);
    assert_eq!(failed["params"]["warningsCount"], 0);
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn build_summaries_report_truncated_diagnostics_only_when_the_authoritative_log_is_partial() {
    use std::io::{Seek, SeekFrom, Write};
    let script =
        "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\ncat \"${2%/*}/large.log\" > \"$1/main.log\"\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let mut log = std::fs::File::create(directory.path().join("large.log")).unwrap();
    writeln!(log, "LaTeX Warning: Beginning on input line 1.").unwrap();
    log.set_len(10 * 1024 * 1024).unwrap();
    log.seek(SeekFrom::End(0)).unwrap();
    writeln!(
        log,
        "\nLaTeX Warning: Ending on input line 2.\nOutput written on main.pdf (2 pages, 9 bytes)."
    )
    .unwrap();
    drop(log);
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    let summary = &finished["params"];
    assert_eq!(summary["status"], "success");
    assert_eq!(summary["diagnosticsTruncated"], true);
    assert_eq!(summary["warningsCount"], 2);
    assert_eq!(summary["pages"], 2);
    let status = server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[main]}),
    );
    assert_eq!(&status, summary);
    // Large output from an earlier pass does not make a complete final log
    // incomplete. Its final counts and diagnostics must take precedence.
    std::fs::write(directory.path().join("compiler.sh"), "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\ncat \"${2%/*}/large.log\"\nprintf 'LaTeX Warning: Final pass on input line 2.\\nOutput written on main.pdf (3 pages, 9 bytes).\\n' > \"$1/main.log\"\n").unwrap();
    server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["diagnosticsTruncated"], false);
    assert_eq!(finished["params"]["pages"], 3);
    assert_eq!(finished["params"]["warningsCount"], 1);
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
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
#[ignore = "requires latexmk and BibTeX"]
fn real_bibliography_logs_discover_macro_expanded_inputs_and_enable_project_key_renames() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join(".inputs")).unwrap();
    let main = Url::from_file_path(root.join("main.tex")).unwrap();
    let bib = Url::from_file_path(root.join(".inputs/refs.bib")).unwrap();
    let style = Url::from_file_path(root.join(".inputs/local.bst")).unwrap();
    std::fs::write(main.to_file_path().unwrap(), "\\documentclass{article}\n\\def\\database{.inputs/refs}\n\\def\\stylename{.inputs/local}\n\\begin{document}\n\\cite{dynamic}\n\\bibliographystyle{\\stylename}\n\\bibliography{\\database}\n\\end{document}\n").unwrap();
    std::fs::write(
        bib.to_file_path().unwrap(),
        "@book{dynamic,title={Discovered}}\n",
    )
    .unwrap();
    std::fs::write(
        style.to_file_path().unwrap(),
        r#"ENTRY { title } {} {}
FUNCTION {book} { "\bibitem{" write$ cite$ write$ "}" write$ newline$ title write$ newline$ }
READ
FUNCTION {begin.bib} { "\begin{thebibliography}{9}" write$ newline$ }
EXECUTE {begin.bib}
ITERATE {call.type$}
FUNCTION {end.bib} { "\end{thebibliography}" write$ newline$ }
EXECUTE {end.bib}
"#,
    )
    .unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{}}));
    server.notify("initialized", json!({}));
    for (id, input) in [(2, &main), (3, &style)] {
        server.request(
            id,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[input]}),
        );
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "success", "{finished}");
        assert_eq!(finished["params"]["stale"], false, "{finished}");
        assert_eq!(finished["params"]["rootUri"], main.as_str());
        assert_eq!(finished["params"]["dependenciesCount"], 3, "{finished}");
    }
    let edit = server.request(4, "textDocument/rename", json!({"textDocument":{"uri":main},"position":{"line":4,"character":8},"newName":"renamed"}));
    let changes = edit["changes"].as_object().unwrap();
    assert_eq!(changes.len(), 2, "{edit}");
    assert_eq!(changes[main.as_str()][0]["newText"], "renamed");
    assert_eq!(changes[bib.as_str()][0]["newText"], "renamed");
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn biber_logged_sources_keep_diagnostics_navigation_and_rename_on_the_complete_database() {
    for quote in ["'", "\"", ""] {
        for suffix in [
            ":copy.bib",
            ";copy.bib",
            ")copy.bib",
            "]copy.bib",
            "}copy.bib",
            ",copy.bib",
            " line 700 copy.bib",
            ": copy.bib",
            "; copy.bib",
            ") copy.bib",
            ", line 700 copy.bib",
            ", in file 'wrong.bib' copy.bib",
        ] {
            let script = "cp \"${2%/*}/fixture.blg\" \"$1/main.blg\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nexit 1\n";
            let (mut server, directory, main) = fake_build_server_capabilities(
                script,
                false,
                json!({"textDocument":{"diagnostic":{}},"workspace":{"workspaceEdit":{"documentChanges":true}}}),
            );
            let root = directory.path();
            std::fs::create_dir(root.join(".inputs")).unwrap();
            let name = format!(".inputs/refs.bib{quote}{suffix}");
            let bib = Url::from_file_path(root.join(&name)).unwrap();
            let decoy = Url::from_file_path(root.join(".inputs/refs.bib")).unwrap();
            std::fs::write(
                bib.to_file_path().unwrap(),
                "% 中文 😀 header\n\n@book{entry,title={Actual}}\n",
            )
            .unwrap();
            std::fs::write(
                decoy.to_file_path().unwrap(),
                "@book{entry,title={Wrong}}\n",
            )
            .unwrap();
            let source = "\\documentclass{article}\n\\cite{entry}\n% \\cite{entry}\n";
            std::fs::write(main.to_file_path().unwrap(), source).unwrap();
            server.notify(
                "textDocument/didChange",
                json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
            );
            server.notify("textDocument/didSave", json!({"textDocument":{"uri":main}}));
            let fixture = root.join("fixture.blg");
            let input = if quote != "\"" {
                bib.to_file_path().unwrap().to_str().unwrap().to_owned()
            } else {
                name.clone()
            };
            std::fs::write(&fixture, format!("INFO - Found BibTeX data source '{input}'\nERROR - syntax error in 'parser'; in file {quote}{name}{quote}, line 3\nWARN - Missing field in 'parser'; for entry 'entry' (file {quote}{name}{quote})\n")).unwrap();
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let finished = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(finished["params"]["status"], "error", "{finished}");
            let diagnostics = server.request(
                3,
                "textDocument/diagnostic",
                json!({"textDocument":{"uri":bib}}),
            );
            let items: Vec<_> = diagnostics["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["source"] == "latex-build")
                .collect();
            assert_eq!(items.len(), 2, "{name}: {diagnostics}");
            assert!(
                items.iter().all(|item| item["range"]["start"]["line"] == 2),
                "{name}: {diagnostics}"
            );
            let warning = items.iter().find(|item| item["severity"] == 2).unwrap();
            assert_eq!(
                warning["range"],
                json!({"start":{"line":2,"character":6},"end":{"line":2,"character":11}})
            );
            let target = json!({"textDocument":{"uri":main},"position":{"line":1,"character":8}});
            let definitions = server.request(4, "textDocument/definition", target.clone());
            assert_eq!(
                definitions.as_array().unwrap().len(),
                1,
                "{name}: {definitions}"
            );
            assert_eq!(definitions[0]["uri"], json!(bib));
            let edit = server.request(5, "textDocument/rename", json!({"textDocument":{"uri":main},"position":target["position"],"newName":"renamed"}));
            let changes = edit["documentChanges"].as_array().unwrap();
            assert_eq!(changes.len(), 2, "{name}: {edit}");
            for change in changes {
                let uri = &change["textDocument"]["uri"];
                assert!(
                    uri == main.as_str() || uri == bib.as_str(),
                    "{name}: {edit}"
                );
                assert_eq!(
                    change["textDocument"]["version"],
                    if uri == main.as_str() {
                        json!(2)
                    } else {
                        Value::Null
                    }
                );
                assert_eq!(change["edits"].as_array().unwrap().len(), 1);
                assert_eq!(change["edits"][0]["newText"], "renamed");
                assert_eq!(
                    change["edits"][0]["range"]["start"]["line"],
                    if uri == main.as_str() { 1 } else { 2 }
                );
            }
            std::fs::write(
                &fixture,
                format!("INFO - Found BibTeX data source '{name}'\n"),
            )
            .unwrap();
            server.request(
                6,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            server.until(|message| message["method"] == "latex/buildFinished");
            for (index, uri) in [&main, &bib, &decoy].into_iter().enumerate() {
                let diagnostics = server.request(
                    7 + index as u64,
                    "textDocument/diagnostic",
                    json!({"textDocument":{"uri":uri}}),
                );
                assert!(
                    diagnostics["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|item| !item["message"].as_str().unwrap().starts_with("Biber:")),
                    "{name}: {diagnostics}"
                );
            }
            server.request(10, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn biber_unreported_unquoted_sources_restrict_diagnostics_to_the_complete_explicit_file() {
    for name in ["with spaces.bib", "refs.bib line 700 copy.bib"] {
        for missing in [false, true] {
            let script = "cp \"${2%/*}/fixture.blg\" \"$1/main.blg\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nexit 1\n";
            let (mut server, directory, main) = fake_build_server_capabilities(
                script,
                false,
                json!({"textDocument":{"diagnostic":{}}}),
            );
            let root = directory.path();
            std::fs::create_dir(root.join(".inputs")).unwrap();
            let path = root.join(".inputs").join(name);
            let bib = Url::from_file_path(&path).unwrap();
            let decoy = Url::from_file_path(root.join(".inputs/refs.bib")).unwrap();
            let wrong = Url::from_file_path(root.join("wrong.bib")).unwrap();
            std::fs::write(
                decoy.to_file_path().unwrap(),
                "@book{entry,title={Decoy}}\n",
            )
            .unwrap();
            std::fs::write(
                wrong.to_file_path().unwrap(),
                "@book{entry,title={Wrong}}\n",
            )
            .unwrap();
            if !missing {
                std::fs::write(&path, "% header\n\n@book{entry,title={Explicit}}\n").unwrap();
            }
            let fixture = root.join("fixture.blg");
            let input = "INFO - Found BibTeX data source '.inputs/refs.bib'\n";
            let error = format!("Unexpected token 'wrong.bib' in file .inputs/{name}, line 3");
            let warning = format!("Missing field for entry 'entry' (file .inputs/{name})");
            std::fs::write(
                &fixture,
                format!("{input}ERROR - {error}\nWARN - {warning}\n"),
            )
            .unwrap();
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let finished = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(finished["params"]["status"], "error", "{finished}");
            let target = if missing { &main } else { &bib };
            let mut id = 3;
            for uri in [&main, &bib, &decoy, &wrong] {
                let diagnostics = server.request(
                    id,
                    "textDocument/diagnostic",
                    json!({"textDocument":{"uri":uri}}),
                );
                id += 1;
                let items: Vec<_> = diagnostics["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|item| item["message"].as_str().unwrap().starts_with("Biber:"))
                    .collect();
                if uri == target {
                    assert_eq!(items.len(), 2, "{name}: {diagnostics}");
                    assert!(
                        items
                            .iter()
                            .all(|item| item["range"]["start"]["line"]
                                == if missing { 0 } else { 2 }),
                        "{name}: {diagnostics}"
                    );
                    if missing {
                        assert!(items
                            .iter()
                            .all(|item| item["data"]["unresolvedSource"] == true));
                    } else {
                        let warning = items.iter().find(|item| item["severity"] == 2).unwrap();
                        assert_eq!(
                            warning["range"],
                            json!({"start":{"line":2,"character":6},"end":{"line":2,"character":11}})
                        );
                    }
                } else {
                    assert!(items.is_empty(), "{name}: {diagnostics}");
                }
            }
            std::fs::write(&fixture, input).unwrap();
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            id += 1;
            server.until(|message| message["method"] == "latex/buildFinished");
            for uri in [&main, &bib, &decoy, &wrong] {
                let diagnostics = server.request(
                    id,
                    "textDocument/diagnostic",
                    json!({"textDocument":{"uri":uri}}),
                );
                id += 1;
                assert!(
                    diagnostics["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|item| !item["message"].as_str().unwrap().starts_with("Biber:")),
                    "{name}: {diagnostics}"
                );
            }
            server.request(id, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn bibliography_logs_discover_inputs_without_replacing_or_losing_tex_recorder_dependencies() {
    let script = "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT main.tex\\nINPUT .inputs/chapter.tex\\n' > \"$1/main.fls\"\nif [ ! -f \"$1/main.blg\" ]; then\n printf 'The top-level auxiliary file: build/latex-lsp/main.aux\\nDatabase file #1: .inputs/refs.bib\\nThe style file: .inputs/local.bst\\n' > \"$1/main.blg\"\nfi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let root = directory.path();
    std::fs::create_dir(root.join(".inputs")).unwrap();
    for (name, source) in [
        ("chapter.tex", "\\label{dynamic:chapter}\n"),
        ("refs.bib", "@book{dynamic,title={Discovered}}\n"),
        ("local.bst", "Original style\n"),
    ] {
        std::fs::write(root.join(".inputs").join(name), source).unwrap();
    }
    for id in [2, 3] {
        server.request(
            id,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "success", "{finished}");
        assert_eq!(finished["params"]["stale"], false, "{finished}");
        assert_eq!(finished["params"]["dependenciesCount"], 4, "{finished}");
    }
    let symbols = server.request(4, "workspace/symbol", json!({"query":"dynamic"}));
    assert_eq!(symbols.as_array().unwrap().len(), 2, "{symbols}");
    // A later no-op build must still check the bibliography discovered earlier.
    std::fs::write(root.join("compiler.sh"), "exit 0\n").unwrap();
    let bib = root.join(".inputs/refs.bib");
    let pdf_time = std::fs::metadata(root.join("build/latex-lsp/main.pdf"))
        .unwrap()
        .modified()
        .unwrap();
    std::fs::File::open(bib)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(pdf_time + Duration::from_secs(1)))
        .unwrap();
    server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let outdated = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(outdated["params"]["status"], "error", "{outdated}");
    assert!(
        outdated["params"]["message"]
            .as_str()
            .unwrap()
            .contains("refs.bib"),
        "{outdated}"
    );
    let style = Url::from_file_path(root.join(".inputs/local.bst")).unwrap();
    server.notify("textDocument/didOpen", json!({"textDocument":{"uri":style,"languageId":"bst","version":1,"text":"Unsaved style\n"}}));
    server.send(json!({"jsonrpc":"2.0","id":6,"method":"workspace/executeCommand","params":{"command":"latex.build","arguments":[style]}}));
    let rejected = server.until(|message| message["id"] == 6);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("local.bst"),
        "{rejected}"
    );
    server.request(7, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn newly_discovered_bibliography_inputs_discard_results_for_unsaved_or_changed_sources() {
    for name in ["refs.bib", "local.bst"] {
        for changed_during_build in [false, true] {
            let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'Database file #1: .inputs/refs.bib\\nThe style file: .inputs/local.bst\\n' > \"$1/main.blg\"\n";
            let (mut server, directory, main) = fake_build_server(script, false);
            let root = directory.path();
            std::fs::create_dir(root.join(".inputs")).unwrap();
            std::fs::write(root.join(".inputs/refs.bib"), "@book{example}\n").unwrap();
            std::fs::write(root.join(".inputs/local.bst"), "Original style\n").unwrap();
            let path = root.join(".inputs").join(name);
            let uri = Url::from_file_path(&path).unwrap();
            if !changed_during_build {
                server.notify("textDocument/didOpen", json!({"textDocument":{"uri":uri,"languageId":"bib","version":1,"text":"Unsaved\n"}}));
            }
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            wait_for_file(&root.join("build/latex-lsp/ready"));
            if changed_during_build {
                std::fs::write(&path, "Changed during compilation\n").unwrap();
            }
            std::fs::write(root.join("build/latex-lsp/release"), "").unwrap();
            let finished = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(finished["params"]["stale"], true, "{name}: {finished}");
            assert_eq!(finished["params"]["dependenciesCount"], 3, "{finished}");
            server.request(3, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
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
    let image_directory = tempfile::tempdir().unwrap();
    let image_source = image_directory.path().join("figure.tex");
    std::fs::write(
        &image_source,
        "\\documentclass{article}\\begin{document}Image\\end{document}",
    )
    .unwrap();
    let image_build =
        latex_lsp::build::run(&image_source, &Default::default(), &Default::default());
    assert_eq!(image_build.status, "success", "{}", image_build.output);
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("project with spaces");
    std::fs::create_dir(&folder).unwrap();
    let root = Url::from_directory_path(&folder).unwrap();
    let main = Url::from_file_path(folder.join("main.tex")).unwrap();
    let chapter = Url::from_file_path(folder.join("chapter file.tex")).unwrap();
    let source = "Hello SyncTeX.\nSecond line.\n% 😀\n";
    std::fs::create_dir(folder.join("assets")).unwrap();
    let image = folder.join("assets/plot.pdf");
    std::fs::copy(image_build.pdf.unwrap(), &image).unwrap();
    std::fs::write(
        main.to_file_path().unwrap(),
        "\\documentclass{article}\n\\usepackage{graphicx}\n\\graphicspath{{assets/}}\n\\begin{document}\n\\input{chapter file}\n\\includegraphics[width=1cm]{plot}\n\\end{document}\n",
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
    assert_eq!(finished["params"]["dependenciesCount"], 3, "{finished}");
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
    let original_time = std::fs::metadata(&image).unwrap().modified().unwrap();
    let recorded_time = std::fs::metadata(folder.join("build/latex-lsp/main.synctex.gz"))
        .unwrap()
        .modified()
        .unwrap();
    std::fs::File::open(&image)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(recorded_time + Duration::from_secs(1)))
        .unwrap();
    server.send(json!({"jsonrpc":"2.0","id":10,"method":"workspace/executeCommand","params":{"command":"latex.inverseSearch","arguments":[{"uri":main,"page":1,"x":0,"y":0}]}}));
    let image_stale = server.until(|message| message["id"] == 10);
    assert!(
        image_stale["error"]["message"]
            .as_str()
            .unwrap()
            .contains("changed"),
        "{image_stale}"
    );
    std::fs::File::open(&image)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(original_time))
        .unwrap();
    std::fs::write(chapter.to_file_path().unwrap(), "Changed\n").unwrap();
    server.send(json!({"jsonrpc":"2.0","id":11,"method":"workspace/executeCommand","params":{"command":"latex.inverseSearch","arguments":[{"uri":main,"page":1,"x":0,"y":0}]}}));
    let stale = server.until(|message| message["id"] == 11);
    assert!(stale["error"]["message"]
        .as_str()
        .unwrap()
        .contains("changed"));
    server.request(12, "shutdown", Value::Null);
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
        // A fresh LuaTeX font cache can exceed the ordinary LSP message
        // budget when other real-engine tests are running concurrently.
        let finished = server.until_with_timeout(Duration::from_secs(30), |message| {
            message["method"] == "latex/buildFinished"
        });
        assert_eq!(
            finished["params"]["status"], "success",
            "{engine}: {finished}"
        );
        assert_eq!(finished["params"]["engine"], engine);
        assert_eq!(finished["params"]["executable"], "latexmk");
        assert_eq!(finished["params"]["stale"], false, "{finished}");
        // LuaTeX can additionally read distribution text data tables.
        assert!(
            finished["params"]["dependenciesCount"]
                .as_u64()
                .is_some_and(|count| count >= 2),
            "{finished}"
        );
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
    let watchers = registration["params"]["registrations"][0]["registerOptions"]["watchers"]
        .as_array()
        .unwrap();
    assert!(
        watchers
            .iter()
            .any(|watcher| watcher["globPattern"] == "**/*" && watcher["kind"] == 5),
        "{registration}"
    );
    let image = watchers
        .iter()
        .find(|watcher| {
            watcher["kind"] == 7
                && watcher["globPattern"]
                    .as_str()
                    .is_some_and(|pattern| pattern.contains("[pP][nN][gG]"))
        })
        .expect("case-insensitive image updates must be registered");
    for extension in [
        "[pP][dD][fF]",
        "[jJ][pP][gG]",
        "[jJ][pP][eE][gG]",
        "[eE][pP][sS]",
        "[sS][vV][gG]",
        "[cC][sS][vV]",
        "[tT][sS][vV]",
        "[dD][aA][tT]",
        "[jJ][sS][oO][nN]",
        "[xX][mM][lL]",
        "[yY][aA][mM][lL]",
        "[yY][mM][lL]",
        "[tT][xX][tT]",
    ] {
        assert!(image["globPattern"].as_str().unwrap().contains(extension));
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

#[test]
fn signature_help_tracks_incremental_edits_and_custom_macros_over_stdio() {
    let mut server = Server::start();
    let init = server.request(1, "initialize", json!({"processId":null,"capabilities":{}}));
    assert!(
        init["capabilities"]["signatureHelpProvider"]["triggerCharacters"]
            .as_array()
            .unwrap()
            .contains(&json!("{"))
    );
    server.notify("initialized", json!({}));
    let macros = "file:///signature-protocol/macros.sty";
    let main = "file:///signature-protocol/main.tex";
    server.notify("textDocument/didOpen",json!({"textDocument":{"uri":macros,"languageId":"latex","version":1,"text":"\\newcommand{\\pair}[2][blue]{#1:#2}"}}));
    server.notify("textDocument/didOpen",json!({"textDocument":{"uri":main,"languageId":"latex","version":1,"text":"\\pair{value}"}}));
    let help = server.request(
        2,
        "textDocument/signatureHelp",
        json!({"textDocument":{"uri":main},"position":{"line":0,"character":10}}),
    );
    assert_eq!(help["signatures"][0]["label"], "\\pair[#1=blue]{#2}");
    assert_eq!(help["activeParameter"], 1);
    server.notify("textDocument/didChange",json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":"\\frac{one}{two}"}]}));
    let help = server.request(
        3,
        "textDocument/signatureHelp",
        json!({"textDocument":{"uri":main},"position":{"line":0,"character":13}}),
    );
    assert_eq!(
        help["signatures"][0]["label"],
        "\\frac{numerator}{denominator}"
    );
    assert_eq!(help["activeParameter"], 1);
    server.notify("textDocument/didChange",json!({"textDocument":{"uri":macros,"version":2},"contentChanges":[{"text":"\\newenvironment{panel}[2][blue]{#1#2}{}"}]}));
    server.notify("textDocument/didChange",json!({"textDocument":{"uri":main,"version":3},"contentChanges":[{"text":"\\begin{panel}{title}\\end{panel}"}]}));
    let help = server.request(
        4,
        "textDocument/signatureHelp",
        json!({"textDocument":{"uri":main},"position":{"line":0,"character":18}}),
    );
    assert_eq!(
        help["signatures"][0]["label"],
        "\\begin{panel}[#1=blue]{#2}"
    );
    assert_eq!(help["activeParameter"], 2);
    server.notify("textDocument/didChange",json!({"textDocument":{"uri":macros,"version":3},"contentChanges":[{"text":"\\newenvironment{panel}[1]{#1}{}"}]}));
    let help = server.request(
        5,
        "textDocument/signatureHelp",
        json!({"textDocument":{"uri":main},"position":{"line":0,"character":18}}),
    );
    assert_eq!(help["signatures"][0]["label"], "\\begin{panel}{#1}");
    assert_eq!(help["activeParameter"], 1);
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn signature_help_triggers_follow_custom_delimiters_flags_and_completed_arguments() {
    let mut server = Server::start();
    let init = server.request(1, "initialize", json!({"processId":null,"capabilities":{}}));
    let options = &init["capabilities"]["signatureHelpProvider"];
    for character in ["{", "[", ",", "<", "(", "|", "*", "+"] {
        assert!(options["triggerCharacters"]
            .as_array()
            .unwrap()
            .contains(&json!(character)));
    }
    for character in ["}", "]", ">", ")", "|"] {
        assert!(options["retriggerCharacters"]
            .as_array()
            .unwrap()
            .contains(&json!(character)));
    }
    server.notify("initialized", json!({}));
    let main = "file:///signature-triggers/main.tex";
    server.notify("textDocument/didOpen", json!({"textDocument":{"uri":"file:///signature-triggers/macros.sty","languageId":"latex","version":1,"text":"\\NewDocumentCommand\\probe{d<>r()t+m}{}\\NewDocumentCommand\\pipes{r||m}{}"}}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":main,"languageId":"latex","version":1,"text":""}}),
    );
    let cases = [
        (r"\probe", "<", Some(0), false),
        (r"\probe<x", ">", Some(1), true),
        (r"\probe", "(", Some(1), false),
        (r"\probe(req", ")", Some(2), true),
        (r"\probe(req)", "+", Some(3), false),
        (r"\probe(req)+", "{", Some(3), false),
        (r"\probe(req)+{BODY", "}", None, true),
        (r"\pipes", "|", Some(0), false),
        (r"\pipes|one", "|", Some(1), true),
        (r"\section", "*", Some(1), false),
        (r"\cite{one", ",", Some(2), false),
        (r"\sqrt", "[", Some(0), false),
        (r"\sqrt[3", "]", Some(1), true),
    ];
    let mut version = 1;
    let mut id = 1;
    for prefix in ["", "😀 ", "% ignored < + *\n"] {
        for (base, character, active, retrigger) in cases {
            let mut text = format!("{prefix}{base}");
            version += 1;
            server.notify("textDocument/didChange", json!({"textDocument":{"uri":main,"version":version},"contentChanges":[{"text":text}]}));
            let at = latex_lsp::text::Text::new(text.clone()).position(text.len());
            version += 1;
            server.notify("textDocument/didChange", json!({"textDocument":{"uri":main,"version":version},"contentChanges":[{"range":{"start":at,"end":at},"text":character}]}));
            text.push_str(character);
            let position = latex_lsp::text::Text::new(text.clone()).position(text.len());
            id += 1;
            let help = server.request(id, "textDocument/signatureHelp", json!({"textDocument":{"uri":main},"position":position,"context":{"triggerKind":2,"triggerCharacter":character,"isRetrigger":retrigger}}));
            if let Some(active) = active {
                assert_eq!(help["activeParameter"], active, "{text}; {help}");
            } else {
                assert!(help.is_null(), "{text}; {help}");
            }
        }
    }
    server.request(id + 1, "shutdown", Value::Null);
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
    assert!(initialized["capabilities"]
        .get("diagnosticProvider")
        .is_none());
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

#[test]
#[cfg(unix)]
fn recorded_images_remain_binary_and_invalidate_unchanged_pdf_artifacts() {
    let script = "if [ ! -f \"$1/main.pdf\" ]; then\n printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n printf 'PWD %s\\nINPUT main.tex\\nINPUT assets/plot.png\\n' \"$PWD\" > \"$1/main.fls\"\nfi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    std::fs::create_dir(directory.path().join("assets")).unwrap();
    let image = directory.path().join("assets/plot.png");
    // Even an ASCII image containing TeX-looking text must stay outside the index.
    std::fs::write(&image, "\\label{image:should-not-be-indexed}").unwrap();
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    assert_eq!(first["params"]["dependenciesCount"], 2);
    assert_eq!(
        server.request(
            3,
            "workspace/symbol",
            json!({"query":"image:should-not-be-indexed"})
        ),
        json!([])
    );
    let pdf = directory.path().join("build/latex-lsp/main.pdf");
    let old_pdf = std::fs::read(&pdf).unwrap();
    let newer = std::fs::metadata(&pdf).unwrap().modified().unwrap() + Duration::from_secs(1);
    std::fs::File::open(&image)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(newer))
        .unwrap();
    server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let changed = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(changed["params"]["status"], "error", "{changed}");
    assert_eq!(changed["params"]["stale"], false);
    assert!(changed["params"]["message"]
        .as_str()
        .unwrap()
        .contains("plot.png"));
    assert!(changed["params"]["pdfUri"].is_null());
    assert_eq!(std::fs::read(&pdf).unwrap(), old_pdf);
    std::fs::remove_file(&image).unwrap();
    server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let deleted = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(deleted["params"]["status"], "error", "{deleted}");
    assert_eq!(deleted["params"]["stale"], false);
    assert!(deleted["params"]["pdfUri"].is_null());
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn static_graphicspath_images_changed_during_build_discard_diagnostics_without_a_recorder() {
    let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: old image.\\n' \"$2\" > \"$1/main.log\"\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    std::fs::create_dir(directory.path().join("assets")).unwrap();
    let image = directory.path().join("assets/plot.png");
    std::fs::write(&image, [0, 255, 1]).unwrap();
    let source = "\\documentclass{article}\n\\graphicspath{{assets/}}\n\\includegraphics{plot}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let output = directory.path().join("build/latex-lsp");
    wait_for_file(&output.join("ready"));
    let modified = std::fs::metadata(&image).unwrap().modified().unwrap();
    std::fs::write(&image, [0, 255, 1, 2, 3, 4]).unwrap();
    // Fingerprints detect length changes even when a tool preserves timestamps.
    std::fs::File::open(&image)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    std::fs::write(output.join("release"), "").unwrap();
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], true, "{finished}");
    assert_eq!(finished["params"]["dependenciesCount"], 2);
    assert!(finished["params"]["message"]
        .as_str()
        .unwrap()
        .contains("diagnostics discarded"));
    server.request(3, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn conditional_files_changed_during_build_discard_diagnostics_without_file_events() {
    for (command, name, existed) in [
        ("IfFileExists", "optional.bin", false),
        ("IfFileExists", "optional.bin", true),
        ("InputIfFileExists", "optional.tex", false),
    ] {
        let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: previous file condition.\\n' \"$2\" > \"$1/main.log\"\n";
        let (mut server, directory, main) = fake_build_server(script, false);
        let target = directory.path().join(name);
        if existed {
            std::fs::write(&target, "Data.").unwrap();
        }
        let source =
            format!("\\documentclass{{article}}\n\\{command}{{{name}}}{{Found.}}{{Missing.}}\n");
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        wait_for_file(&output.join("ready"));
        if existed {
            std::fs::remove_file(&target).unwrap();
        } else {
            std::fs::write(&target, "New data.").unwrap();
        }
        std::fs::write(output.join("release"), "").unwrap();
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "success", "{finished}");
        assert_eq!(
            finished["params"]["stale"], true,
            "{command}, {existed}: {finished}"
        );
        server.request(3, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn conditional_first_stale_pdf_cannot_be_accepted_by_a_later_noop_build() {
    let script = "if [ ! -f \"$1/main.pdf\" ]; then\ntouch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nfi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let source = "\\documentclass{article}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let output = directory.path().join("build/latex-lsp");
    wait_for_file(&output.join("ready"));
    std::fs::write(directory.path().join("optional.bin"), "Created.").unwrap();
    std::fs::write(output.join("release"), "").unwrap();
    let stale = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(stale["params"]["status"], "success", "{stale}");
    assert_eq!(stale["params"]["stale"], true, "{stale}");
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let noop = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(noop["params"]["status"], "error", "{noop}");
    assert!(
        noop["params"]["message"]
            .as_str()
            .unwrap()
            .contains("file condition changed"),
        "{noop}"
    );
    assert!(noop["params"]["pdfUri"].is_null());
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn missing_include_creation_invalidates_results_and_old_pdfs_unless_the_chapter_is_excluded() {
    for (selection, stale) in [
        ("", true),
        ("\\includeonly{chapter}\n", true),
        ("\\includeonly{chapter.tex}\n", true),
        ("\\includeonly{ chapter.tex, other }\n", true),
        ("\\includeonly{other}\n", false),
        ("\\includeonly{}\n", false),
        ("\\includeonly{\\selected}\n", false),
        ("\\includeonly{other}\n\\includeonly{chapter}\n", false),
    ] {
        let script = "if [ ! -f \"$1/main.pdf\" ]; then\ntouch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nfi\n";
        let (mut server, directory, main) = fake_build_server(script, false);
        let source = format!("\\documentclass{{article}}\n{selection}\\include{{chapter}}\n");
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        wait_for_file(&output.join("ready"));
        std::fs::write(directory.path().join("chapter.tex"), "New chapter.").unwrap();
        std::fs::write(output.join("release"), "").unwrap();
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["stale"], stale, "{selection}: {first}");
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let noop = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            noop["params"]["status"],
            if stale { "error" } else { "success" },
            "{selection}: {noop}"
        );
        if stale {
            assert!(noop["params"]["pdfUri"].is_null());
            assert!(
                noop["params"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("file condition changed"),
                "{noop}"
            );
        }
        server.request(4, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn missing_required_inputs_created_during_failed_builds_discard_obsolete_errors() {
    for (command, name) in [
        ("\\input{child}", "child.tex"),
        ("\\input child", "child.tex"),
        ("\\input \"my file\"", "my file.tex"),
        ("\\input{child.ltx}", "child.ltx"),
        ("\\subfile{child}", "child.tex"),
    ] {
        let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%s:2: Undefined control sequence.\\n' \"$2\" > \"$1/main.log\"\nexit 1\n";
        let (mut server, directory, main) = fake_build_server(script, false);
        let source = format!("\\documentclass{{article}}\n{command}\n");
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        wait_for_file(&output.join("ready"));
        std::fs::write(directory.path().join(name), "New input.").unwrap();
        std::fs::write(output.join("release"), "").unwrap();
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "error", "{finished}");
        assert_eq!(finished["params"]["stale"], true, "{command}: {finished}");
        assert!(finished["params"]["message"]
            .as_str()
            .unwrap()
            .contains("diagnostics discarded"));
        server.request(3, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn stale_builds_clear_previously_published_errors_without_file_notifications() {
    let script = "if [ -f \"$1/first\" ]; then\ntouch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nelse touch \"$1/first\"; fi\nprintf '%s:2: Undefined control sequence.\\n' \"$2\" > \"$1/main.log\"\nexit 1\n";
    let (mut server, directory, main) =
        fake_build_server_capabilities(script, false, json!({"textDocument":{"diagnostic":{}}}));
    let source = "\\documentclass{article}\n\\input{child}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "error", "{first}");
    assert_eq!(first["params"]["stale"], false, "{first}");
    let old = server.request(
        3,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main}}),
    );
    assert!(
        old["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["source"] == "latex-build"),
        "{old}"
    );
    server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let output = directory.path().join("build/latex-lsp");
    wait_for_file(&output.join("ready"));
    std::fs::write(directory.path().join("child.tex"), "Restored input.").unwrap();
    std::fs::write(output.join("release"), "").unwrap();
    let stale = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(stale["params"]["stale"], true, "{stale}");
    let cleared = server.request(
        5,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
    );
    assert_eq!(cleared["kind"], "full", "{cleared}");
    assert_eq!(cleared["items"], json!([]), "{cleared}");
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn required_input_filename_selection_changes_invalidate_results_and_unchanged_pdfs() {
    for symlink in [false, true] {
        let script = "if [ ! -f \"$1/main.pdf\" ]; then\ntouch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nfi\n";
        let (mut server, directory, main) = fake_build_server(script, false);
        let original = directory.path().join("original.tex");
        let replacement = directory.path().join("replacement.tex");
        std::fs::write(&original, "Original.").unwrap();
        std::fs::write(&replacement, "Replacement.").unwrap();
        let name = if symlink { "child.tex" } else { "child.ltx" };
        let input = directory.path().join(name);
        if symlink {
            std::os::unix::fs::symlink(&original, &input).unwrap();
        } else {
            std::fs::write(&input, "Original.").unwrap();
        }
        let source = format!("\\documentclass{{article}}\n\\input{{{name}}}\n");
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        wait_for_file(&output.join("ready"));
        if symlink {
            std::fs::remove_file(&input).unwrap();
            std::os::unix::fs::symlink(&replacement, &input).unwrap();
        } else {
            std::fs::write(directory.path().join("child.ltx.tex"), "Preferred suffix.").unwrap();
        }
        std::fs::write(output.join("release"), "").unwrap();
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["stale"], true, "{symlink}: {first}");
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let noop = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(noop["params"]["status"], "error", "{noop}");
        assert!(
            noop["params"]["message"]
                .as_str()
                .unwrap()
                .contains("file condition changed"),
            "{noop}"
        );
        assert!(noop["params"]["pdfUri"].is_null());
        server.request(4, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn required_input_creation_notifications_cancel_builds_and_queues_despite_includeonly() {
    for command in ["input", "subfile"] {
        let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n";
        let (mut server, directory, main) = fake_build_server(script, true);
        let source =
            format!("\\documentclass{{article}}\n\\includeonly{{other}}\n\\{command}{{my file}}\n");
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        wait_for_file(&directory.path().join("build/latex-lsp/ready"));
        server.notify("textDocument/didSave", json!({"textDocument":{"uri":main}}));
        assert_eq!(
            server.request(
                3,
                "workspace/executeCommand",
                json!({"command":"latex.buildStatus","arguments":[main]})
            )["queued"],
            true
        );
        let target = directory.path().join("my file.tex");
        std::fs::write(&target, "New source.").unwrap();
        let target = Url::from_file_path(&target).unwrap();
        if command == "input" {
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":target,"type":1}]}),
            );
        } else {
            server.notify(
                "workspace/didCreateFiles",
                json!({"files":[{"uri":target}]}),
            );
        }
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            finished["params"]["status"], "cancelled",
            "{command}: {finished}"
        );
        let status = server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.buildStatus","arguments":[main]}),
        );
        assert_eq!(status["buildId"], finished["params"]["buildId"]);
        assert_ne!(status["queued"], true, "{status}");
        server.request(5, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex and xelatex"]
fn real_required_input_restoration_discards_missing_file_errors_and_rebuilds_each_filename_form() {
    for engine in ["pdflatex", "xelatex"] {
        for (command, name) in [
            ("\\input{child}", "child.tex"),
            ("\\input child", "child.tex"),
            ("\\input \"my file\"", "my file.tex"),
            ("\\input{输入 file}", "输入 file.tex"),
            ("\\input{child.ltx}", "child.ltx"),
        ] {
            let script = format!("{engine} -interaction=nonstopmode -halt-on-error -recorder -synctex=1 -output-directory=\"$1\" \"$2\"\nresult=$?\ntouch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nexit \"$result\"\n");
            let (mut server, directory, main) = fake_build_server(&script, false);
            let source = format!("\\documentclass{{article}}\n\\begin{{document}}Hello.\n{command}\n\\end{{document}}\n");
            std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
            server.notify(
                "textDocument/didChange",
                json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
            );
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let output = directory.path().join("build/latex-lsp");
            wait_for_file(&output.join("ready"));
            std::fs::write(
                directory.path().join(name),
                "\\typeout{NEW-INPUT-READ}Restored input.\n",
            )
            .unwrap();
            std::fs::write(output.join("release"), "").unwrap();
            let failed = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(
                failed["params"]["status"], "error",
                "{engine}, {command}: {failed}"
            );
            assert_eq!(
                failed["params"]["stale"], true,
                "{engine}, {command}: {failed}"
            );
            server.request(
                3,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(
                rebuilt["params"]["status"], "success",
                "{engine}, {command}: {rebuilt}"
            );
            assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
            assert!(
                rebuilt["params"]["output"]
                    .as_str()
                    .unwrap()
                    .contains("NEW-INPUT-READ"),
                "{rebuilt}"
            );
            assert_eq!(rebuilt["params"]["pages"], 1);
            assert_eq!(rebuilt["params"]["dependenciesCount"], 2);
            server.request(4, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn missing_include_directory_creation_cancels_selected_builds_and_their_queue_only() {
    for selected in [false, true] {
        let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n";
        let (mut server, directory, main) = fake_build_server(script, true);
        let settings = directory.path().join("settings.tex");
        std::fs::write(
            &settings,
            if selected {
                "\\includeonly{chapters/missing.tex}\n"
            } else {
                "\\includeonly{other}\n"
            },
        )
        .unwrap();
        let source = "\\documentclass{article}\n\\input{settings}\n\\include{chapters/missing}\n";
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        wait_for_file(&directory.path().join("build/latex-lsp/ready"));
        server.notify("textDocument/didSave", json!({"textDocument":{"uri":main}}));
        assert_eq!(
            server.request(
                3,
                "workspace/executeCommand",
                json!({"command":"latex.buildStatus","arguments":[main]})
            )["queued"],
            true
        );
        let folder = directory.path().join("chapters");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("missing.tex"), "New chapter.").unwrap();
        server.notify(
            "workspace/didChangeWatchedFiles",
            json!({"changes":[{"uri":Url::from_file_path(&folder).unwrap(),"type":1}]}),
        );
        let status = server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.buildStatus","arguments":[main]}),
        );
        if selected {
            assert_eq!(status["status"], "cancelled", "{status}");
            assert_ne!(status["queued"], true, "{status}");
        } else {
            assert_eq!(status["status"], "running", "{status}");
            assert_eq!(status["queued"], true, "{status}");
            server.request(
                5,
                "workspace/executeCommand",
                json!({"command":"latex.cancelBuild","arguments":[main]}),
            );
        }
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "cancelled", "{finished}");
        server.request(6, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn missing_include_warning_clears_when_the_chapter_is_created_and_synctex_rejects_the_old_pdf() {
    let script = "if [ ! -f \"$1/main.pdf\" ]; then\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX test data\\n' > \"$1/main.synctex\"\nprintf 'No file chapter.tex.\\n' > \"$1/main.log\"\nfi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let source = "\\documentclass{article}\n\\include{chapter}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    assert_eq!(first["params"]["stale"], false, "{first}");
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["code"] == "missing-include"))
    });
    let chapter = directory.path().join("chapter.tex");
    std::fs::write(&chapter, "New chapter.").unwrap();
    // Restoring an old timestamp cannot make the old mapping trustworthy.
    let old = std::fs::metadata(main.to_file_path().unwrap())
        .unwrap()
        .modified()
        .unwrap()
        - Duration::from_secs(60);
    std::fs::File::open(&chapter)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
    server.send(json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]}}));
    let rejected = server.until(|message| message["id"] == 3);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("File conditions changed"),
        "{rejected}"
    );
    server.notify(
        "workspace/didCreateFiles",
        json!({"files":[{"uri":Url::from_file_path(&chapter).unwrap()}]}),
    );
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == main.as_str()
            && message["params"]["diagnostics"] == json!([])
    });
    server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let noop = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(noop["params"]["status"], "error", "{noop}");
    assert!(
        noop["params"]["message"]
            .as_str()
            .unwrap()
            .contains("file condition changed"),
        "{noop}"
    );
    assert!(noop["params"]["pdfUri"].is_null());
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex and xelatex"]
fn real_missing_include_creation_discards_the_incomplete_pdf_and_respects_includeonly() {
    for engine in ["pdflatex", "xelatex"] {
        for selected in [false, true] {
            let script = format!("if [ -f \"$1/noop\" ]; then exit 0; fi\n{engine} -interaction=nonstopmode -halt-on-error -recorder -synctex=1 -output-directory=\"$1\" \"$2\"\nresult=$?\ntouch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nexit \"$result\"\n");
            let (mut server, directory, main) = fake_build_server(&script, false);
            let selection = if selected {
                "chapters/新章节.tex"
            } else {
                "other"
            };
            // This custom shell wrapper must prepare its own auxiliary folder.
            std::fs::create_dir_all(directory.path().join("build/latex-lsp/chapters")).unwrap();
            let source = format!("\\documentclass{{article}}\n\\includeonly{{{selection}}}\n\\begin{{document}}Hello.\n\\include{{chapters/新章节}}\n\\end{{document}}\n");
            std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
            server.notify(
                "textDocument/didChange",
                json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
            );
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let output = directory.path().join("build/latex-lsp");
            wait_for_file(&output.join("ready"));
            std::fs::create_dir(directory.path().join("chapters")).unwrap();
            std::fs::write(
                directory.path().join("chapters/新章节.tex"),
                "\\typeout{NEW-INCLUDE-LOADED}New chapter.\n",
            )
            .unwrap();
            std::fs::write(output.join("release"), "").unwrap();
            let first = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(
                first["params"]["status"], "success",
                "{engine}, {selected}: {first}"
            );
            assert_eq!(
                first["params"]["stale"], selected,
                "{engine}, {selected}: {first}"
            );
            assert!(!first["params"]["output"]
                .as_str()
                .unwrap()
                .contains("NEW-INCLUDE-LOADED"));
            std::fs::write(output.join("noop"), "").unwrap();
            server.request(
                3,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(
                noop["params"]["status"],
                if selected { "error" } else { "success" },
                "{engine}, {selected}: {noop}"
            );
            std::fs::remove_file(output.join("noop")).unwrap();
            server.request(
                4,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(
                rebuilt["params"]["status"], "success",
                "{engine}, {selected}: {rebuilt}"
            );
            assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
            assert_eq!(
                rebuilt["params"]["output"]
                    .as_str()
                    .unwrap()
                    .contains("NEW-INCLUDE-LOADED"),
                selected,
                "{rebuilt}"
            );
            server.request(5, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn conditional_lookup_tracks_suffix_shadows_and_root_priority_without_other_project_checks() {
    for (root_exists, changed, expected_stale) in [
        (true, "optional.bin.tex", true),
        (true, "chapters/optional.bin", false),
        (false, "optional.bin", true),
        (false, "chapters/optional.bin", true),
        (false, "other/optional.bin", false),
    ] {
        let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n";
        let (mut server, directory, main) = fake_build_server(script, false);
        std::fs::create_dir(directory.path().join("chapters")).unwrap();
        std::fs::create_dir(directory.path().join("other")).unwrap();
        let source = "\\documentclass{article}\n\\input{chapters/child}\n";
        std::fs::write(
            directory.path().join("chapters/child.tex"),
            "\\IfFileExists{optional.bin}{Found.}{Missing.}\n",
        )
        .unwrap();
        std::fs::write(
            directory.path().join("other/main.tex"),
            "\\documentclass{article}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n",
        )
        .unwrap();
        if root_exists {
            std::fs::write(directory.path().join("optional.bin"), "Root file.").unwrap();
        }
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        wait_for_file(&output.join("ready"));
        std::fs::write(directory.path().join(changed), "New candidate.").unwrap();
        std::fs::write(output.join("release"), "").unwrap();
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "success", "{finished}");
        assert_eq!(
            finished["params"]["stale"], expected_stale,
            "{root_exists}, {changed}: {finished}"
        );
        server.request(3, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn conditional_symlink_retargets_are_stale_and_canonical_target_deletion_cancels_builds() {
    for notify in [false, true] {
        let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n";
        let (mut server, directory, main) = fake_build_server(script, false);
        let first = directory.path().join("first.bin");
        let second = directory.path().join("second.bin");
        let link = directory.path().join("optional.bin");
        std::fs::write(&first, "First.").unwrap();
        std::fs::write(&second, "Second.").unwrap();
        std::os::unix::fs::symlink(&first, &link).unwrap();
        let source = "\\documentclass{article}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n";
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        wait_for_file(&output.join("ready"));
        if notify {
            std::fs::remove_file(&first).unwrap();
            server.notify(
                "workspace/didDeleteFiles",
                json!({"files":[{"uri":Url::from_file_path(&first).unwrap()}]}),
            );
        } else {
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink(&second, &link).unwrap();
            std::fs::write(output.join("release"), "").unwrap();
        }
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            finished["params"]["status"],
            if notify { "cancelled" } else { "success" },
            "{finished}"
        );
        if !notify {
            assert_eq!(finished["params"]["stale"], true, "{finished}");
        }
        server.request(3, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn conditional_file_rename_updates_the_reference_and_preserves_old_pdf_conditions() {
    let script = "if [ ! -f \"$1/main.pdf\" ]; then\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nfi\n";
    let (mut server, directory, main) = fake_build_server_capabilities(
        script,
        false,
        json!({"workspace":{"workspaceEdit":{"documentChanges":true},"fileOperations":{"willRename":true,"didRename":true}}}),
    );
    let original = directory.path().join("optional.bin");
    let renamed = directory.path().join("renamed flag.bin");
    std::fs::write(&original, "Data.").unwrap();
    let source = "\\documentclass{article}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    let original_time = std::fs::metadata(main.to_file_path().unwrap())
        .unwrap()
        .modified()
        .unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    let files = json!({"files":[{"oldUri":Url::from_file_path(&original).unwrap(),"newUri":Url::from_file_path(&renamed).unwrap()}]});
    let edit = server.request(3, "workspace/willRenameFiles", files.clone());
    assert_eq!(edit["documentChanges"][0]["textDocument"]["version"], 2);
    assert_eq!(
        edit["documentChanges"][0]["edits"][0]["newText"],
        "renamed flag.bin"
    );
    let updated = source.replace("optional.bin", "renamed flag.bin");
    std::fs::write(main.to_file_path().unwrap(), &updated).unwrap();
    // A preserved source mtime must not bypass the historical file check.
    std::fs::File::open(main.to_file_path().unwrap())
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(original_time))
        .unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":3},"contentChanges":[{"text":updated}]}),
    );
    std::fs::rename(original, renamed).unwrap();
    server.notify("workspace/didRenameFiles", files);
    server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let noop = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(noop["params"]["status"], "error", "{noop}");
    assert!(
        noop["params"]["message"]
            .as_str()
            .unwrap()
            .contains("file condition changed"),
        "{noop}"
    );
    assert!(noop["params"]["pdfUri"].is_null());
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn conditional_file_events_cancel_active_and_queued_builds_but_unrelated_files_do_not() {
    for existed in [false, true] {
        let script="touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n";
        let (mut server, directory, main) = fake_build_server(script, true);
        let flag = directory.path().join("flags/optional.bin");
        std::fs::create_dir(flag.parent().unwrap()).unwrap();
        if existed {
            std::fs::write(&flag, "Data.").unwrap();
        }
        let source =
            "\\documentclass{article}\n\\IfFileExists{flags/optional.bin}{Found.}{Missing.}\n";
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        wait_for_file(&directory.path().join("build/latex-lsp/ready"));
        let other = directory.path().join("unrelated.bin");
        std::fs::write(&other, "Other.").unwrap();
        server.notify(
            "workspace/didCreateFiles",
            json!({"files":[{"uri":Url::from_file_path(&other).unwrap()}]}),
        );
        assert_eq!(
            server.request(
                3,
                "workspace/executeCommand",
                json!({"command":"latex.buildStatus","arguments":[main]})
            )["status"],
            "running"
        );
        server.notify("textDocument/didSave", json!({"textDocument":{"uri":main}}));
        assert_eq!(
            server.request(
                4,
                "workspace/executeCommand",
                json!({"command":"latex.buildStatus","arguments":[main]})
            )["queued"],
            true
        );
        if existed {
            std::fs::remove_dir_all(flag.parent().unwrap()).unwrap();
            server.notify(
                "workspace/didDeleteFiles",
                json!({"files":[{"uri":Url::from_file_path(flag.parent().unwrap()).unwrap()}]}),
            );
        } else {
            std::fs::write(&flag, "Created.").unwrap();
            server.notify(
                "workspace/didCreateFiles",
                json!({"files":[{"uri":Url::from_file_path(&flag).unwrap()}]}),
            );
        }
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(finished["params"]["status"], "cancelled", "{finished}");
        let status = server.request(
            5,
            "workspace/executeCommand",
            json!({"command":"latex.buildStatus","arguments":[main]}),
        );
        assert_eq!(status["status"], "cancelled", "{status}");
        assert_eq!(status["buildId"], finished["params"]["buildId"]);
        server.request(6, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn conditional_data_content_and_unsaved_buffers_do_not_change_existence_or_import_symbols() {
    let script="touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let flag = directory.path().join("optional.bin");
    std::fs::write(&flag, "Data.").unwrap();
    let source =
        "\\documentclass{article}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\\ref{hidden}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.notify("textDocument/didOpen",json!({"textDocument":{"uri":Url::from_file_path(&flag).unwrap(),"languageId":"plaintext","version":9,"text":"\\label{hidden}Unsaved data."}}));
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let output = directory.path().join("build/latex-lsp");
    wait_for_file(&output.join("ready"));
    std::fs::write(&flag, "Different disk contents, still present.").unwrap();
    std::fs::write(output.join("release"), "").unwrap();
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], false, "{finished}");
    assert_eq!(finished["params"]["dependenciesCount"], 1);
    let at = source.find("hidden").unwrap();
    let position = latex_lsp::text::Text::new(source.into()).position(at);
    assert_eq!(
        server.request(
            3,
            "textDocument/definition",
            json!({"textDocument":{"uri":main},"position":position})
        ),
        json!([])
    );
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn conditional_file_events_clear_diagnostics_and_reject_old_pdf_and_synctex_until_rebuilt() {
    let script="if [ ! -f \"$1/main.pdf\" ] || [ -f \"$1/rebuild\" ]; then\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX test data\\n' > \"$1/main.synctex\"\nfi\nprintf '%s:2: LaTeX Warning: condition-specific warning.\\n' \"$2\" > \"$1/main.log\"\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let source = "\\documentclass{article}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item["message"]
                            .as_str()
                            .is_some_and(|text| text.contains("condition-specific"))
                    })
                })
    });
    let flag = directory.path().join("optional.bin");
    std::fs::write(&flag, "Created.").unwrap();
    server.send(json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]}}));
    let rejected = server.until(|message| message["id"] == 3);
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("File conditions changed"),
        "{rejected}"
    );
    server.notify(
        "workspace/didCreateFiles",
        json!({"files":[{"uri":Url::from_file_path(&flag).unwrap()}]}),
    );
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == main.as_str()
            && message["params"]["diagnostics"] == json!([])
    });
    for id in [4, 5] {
        server.request(
            id,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let old = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(old["params"]["status"], "error", "{old}");
        assert!(
            old["params"]["message"]
                .as_str()
                .unwrap()
                .contains("file condition changed"),
            "{old}"
        );
        assert!(old["params"]["pdfUri"].is_null());
        assert_eq!(old["params"]["stale"], false);
    }
    std::fs::write(directory.path().join("build/latex-lsp/rebuild"), "").unwrap();
    server.request(
        6,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
    assert_eq!(rebuilt["params"]["stale"], false);
    server.request(7, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn conditional_generated_artifacts_and_include_auxiliaries_do_not_make_builds_stale() {
    let script="mkdir -p \"$1/chapters\"\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'Auxiliary\\n' > \"$1/main.aux\"\nprintf 'Child auxiliary\\n' > \"$1/chapters/child.aux\"\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    std::fs::create_dir(directory.path().join("chapters")).unwrap();
    std::fs::write(directory.path().join("chapters/child.tex"), "Chapter.").unwrap();
    let source="\\documentclass{article}\n\\IfFileExists{build/latex-lsp/main.pdf}{}{}\n\\IfFileExists{build/latex-lsp/main.aux}{}{}\n\\IfFileExists{build/latex-lsp/chapters/child.aux}{}{}\n\\include{chapters/child.tex}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], false, "{finished}");
    server.request(3, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn watched_file_changes_clear_root_build_warnings_and_keep_data_out_of_the_symbol_index() {
    for conditional in [false, true] {
        let script = "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: outdated project warning.\\n' \"$2\" > \"$1/main.log\"\n";
        let (mut server, directory, main) = fake_build_server(script, false);
        let name = if conditional {
            "optional.bin"
        } else {
            "child.tex"
        };
        let target = directory.path().join(name);
        if !conditional {
            std::fs::write(&target, "Old child.").unwrap();
        }
        let source = if conditional {
            "\\documentclass{article}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n"
        } else {
            "\\documentclass{article}\n\\input{child}\n"
        };
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        server.until(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["diagnostics"]
                    .as_array()
                    .is_some_and(|items| {
                        items.iter().any(|item| {
                            item["message"]
                                .as_str()
                                .is_some_and(|text| text.contains("outdated project warning"))
                        })
                    })
        });
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.buildStatus","arguments":[main]}),
        );
        server.pending.borrow_mut().clear();
        std::fs::write(&target, "\\label{watched-data-label}New contents.").unwrap();
        server.notify("workspace/didChangeWatchedFiles", json!({"changes":[{"uri":Url::from_file_path(&target).unwrap(),"type":if conditional {1} else {2}}]}));
        server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.buildStatus","arguments":[main]}),
        );
        let published = server.until(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == main.as_str()
        });
        assert_eq!(
            published["params"]["diagnostics"],
            json!([]),
            "conditional={conditional}: {published}"
        );
        let symbols = server.request(5, "workspace/symbol", json!({"query":"watched-data-label"}));
        assert_eq!(
            symbols.as_array().unwrap().len(),
            usize::from(!conditional),
            "{symbols}"
        );
        server.request(6, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn watched_files_cancel_changed_project_inputs_but_preserve_existence_only_content_edits() {
    for kind in ["source", "image", "create", "delete", "data-content"] {
        let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n";
        let (mut server, directory, main) = fake_build_server(script, true);
        let (name, source) = match kind {
            "source" => ("child.tex", "\\documentclass{article}\n\\input{child}\n"),
            "image" => (
                "plot.PnG",
                "\\documentclass{article}\n\\includegraphics{plot.PnG}\n",
            ),
            _ => (
                "optional.bin",
                "\\documentclass{article}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n",
            ),
        };
        let target = directory.path().join(name);
        if kind != "create" {
            std::fs::write(&target, "Original.").unwrap();
        }
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        wait_for_file(&output.join("ready"));
        if kind != "data-content" {
            server.notify("textDocument/didSave", json!({"textDocument":{"uri":main}}));
            assert_eq!(
                server.request(
                    3,
                    "workspace/executeCommand",
                    json!({"command":"latex.buildStatus","arguments":[main]})
                )["queued"],
                true
            );
        }
        if kind == "delete" {
            std::fs::remove_file(&target).unwrap();
        } else {
            let modified = std::fs::metadata(&target)
                .ok()
                .and_then(|metadata| metadata.modified().ok());
            std::fs::write(
                &target,
                if kind == "data-content" {
                    "\\label{watched-data-label}Different."
                } else {
                    "Different"
                },
            )
            .unwrap();
            if kind == "source" {
                // Both the original and replacement have nine bytes.
                std::fs::File::open(&target)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(modified.unwrap()))
                    .unwrap();
            }
        }
        server.notify("workspace/didChangeWatchedFiles", json!({"changes":[{"uri":Url::from_file_path(&target).unwrap(),"type":match kind {"create"=>1,"delete"=>3,_=>2}}]}));
        let status = server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.buildStatus","arguments":[main]}),
        );
        if kind == "data-content" {
            assert_eq!(status["status"], "running", "{status}");
            std::fs::write(output.join("release"), "").unwrap();
        }
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            finished["params"]["status"],
            if kind == "data-content" {
                "success"
            } else {
                "cancelled"
            },
            "{kind}: {finished}"
        );
        if kind == "data-content" {
            assert_eq!(finished["params"]["stale"], false, "{finished}");
            assert_eq!(
                server.request(5, "workspace/symbol", json!({"query":"watched-data-label"})),
                json!([])
            );
        } else {
            assert_ne!(status["queued"], true, "{status}");
        }
        server.request(6, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn watched_delayed_notifications_do_not_cancel_a_build_of_the_same_saved_inputs() {
    let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let source = "\\documentclass{article}\n\\input{child}\n\\includegraphics{plot.png}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n";
    let child = directory.path().join("child.tex");
    let image = directory.path().join("plot.png");
    std::fs::write(&child, "Saved source.").unwrap();
    std::fs::write(&image, [0, 255, 1]).unwrap();
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let output = directory.path().join("build/latex-lsp");
    wait_for_file(&output.join("ready"));
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[
            {"uri":main,"type":2},
            {"uri":Url::from_file_path(&child).unwrap(),"type":1},
            {"uri":Url::from_file_path(&image).unwrap(),"type":2},
            {"uri":Url::from_file_path(directory.path().join("optional.bin")).unwrap(),"type":3}
        ]}),
    );
    let status = server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[main]}),
    );
    assert_eq!(status["status"], "running", "{status}");
    std::fs::write(output.join("release"), "").unwrap();
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], false, "{finished}");
    server.request(4, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn watched_conditional_file_changes_refresh_pulled_project_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let main = Url::from_file_path(directory.path().join("main.tex")).unwrap();
    let source = "\\documentclass{article}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    let script = directory.path().join("compiler.sh");
    std::fs::write(&script, "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: outdated pull condition.\\n' \"$2\" > \"$1/main.log\"\n").unwrap();
    let mut server = Server::start();
    server.request(1, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(directory.path()).unwrap(),"capabilities":{"textDocument":{"diagnostic":{}},"workspace":{"diagnostic":{"refreshSupport":true}}},"initializationOptions":{"build":{"executable":"/bin/sh","arguments":[script,"%o","%f"]}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":main,"languageId":"latex","version":1,"text":source}}),
    );
    // Acknowledge the opening refresh before testing the build refresh.
    let refresh = server.until(|message| message["method"] == "workspace/diagnostic/refresh");
    server.send(json!({"jsonrpc":"2.0","id":refresh["id"],"result":null}));
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    let refresh = server.until(|message| message["method"] == "workspace/diagnostic/refresh");
    server.send(json!({"jsonrpc":"2.0","id":refresh["id"],"result":null}));
    let diagnostics = server.request(
        3,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main}}),
    );
    assert!(
        diagnostics["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["message"]
                .as_str()
                .is_some_and(|text| text.contains("outdated pull condition"))),
        "{diagnostics}"
    );
    let flag = directory.path().join("optional.bin");
    std::fs::write(&flag, "\\label{data-should-stay-unindexed}").unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":Url::from_file_path(&flag).unwrap(),"type":1}]}),
    );
    let refresh = server.until(|message| message["method"] == "workspace/diagnostic/refresh");
    server.send(json!({"jsonrpc":"2.0","id":refresh["id"],"result":null}));
    let cleared = server.request(
        4,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main},"previousResultId":diagnostics["resultId"]}),
    );
    assert_eq!(cleared["kind"], "full", "{cleared}");
    assert_eq!(cleared["items"], json!([]), "{cleared}");
    assert_ne!(cleared["resultId"], diagnostics["resultId"]);
    assert_eq!(
        server.request(
            5,
            "workspace/symbol",
            json!({"query":"data-should-stay-unindexed"})
        ),
        json!([])
    );
    assert!(!server
        .pending
        .borrow()
        .iter()
        .any(|message| message["method"] == "textDocument/publishDiagnostics"));
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
#[ignore = "requires pdflatex and xelatex"]
fn real_conditional_file_changes_after_tex_runs_discard_old_results_and_rebuild_the_right_branch() {
    for engine in ["pdflatex", "xelatex"] {
        let script=format!("{engine} -interaction=nonstopmode -halt-on-error -recorder -synctex=1 -output-directory=\"$1\" \"$2\"\nresult=$?\ntouch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nexit \"$result\"\n");
        let (mut server, directory, main) = fake_build_server(&script, false);
        let source="\\documentclass{article}\\begin{document}\n\\IfFileExists{状态.flag}{\\typeout{CONDITION-BRANCH:FOUND}Found.}{\\typeout{CONDITION-BRANCH:MISSING}Missing.}\n\\end{document}\n";
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        let output = directory.path().join("build/latex-lsp");
        for (id, second) in [(2, false), (3, true)] {
            if second {
                std::fs::remove_file(output.join("ready")).unwrap();
                std::fs::remove_file(output.join("release")).unwrap();
            }
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            wait_for_file(&output.join("ready"));
            if !second {
                std::fs::write(
                    directory.path().join("状态.flag"),
                    "\\undefinedcheckedcommand",
                )
                .unwrap();
            }
            std::fs::write(output.join("release"), "").unwrap();
            let finished = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(finished["params"]["status"], "success", "{finished}");
            assert_eq!(finished["params"]["stale"], !second, "{finished}");
            assert!(
                finished["params"]["output"]
                    .as_str()
                    .unwrap()
                    .contains(if second {
                        "CONDITION-BRANCH:FOUND"
                    } else {
                        "CONDITION-BRANCH:MISSING"
                    }),
                "{finished}"
            );
        }
        server.request(4, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[ignore = "requires latexmk, BibTeX and a TeX installation"]
fn real_compiler_reference_quick_fixes_remove_label_and_citation_typos_after_rebuilding() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let uri = Url::from_file_path(root.join("main.tex")).unwrap();
    let source = "\\documentclass{article}\n\\begin{document}\n\\input{chapter}\nSee \\ref{sec:itnro} and \\cite{smith2025}.\n\\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}\n";
    std::fs::write(root.join("main.tex"), source).unwrap();
    std::fs::write(
        root.join("chapter.tex"),
        "\\section{Intro}\\label{sec:intro}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("refs.bib"),
        "@book{smith2026,author={Smith},title={Example},year={2026},publisher={P}}\n",
    )
    .unwrap();
    let mut server = Server::start();
    server.request(1,"initialize",json!({"processId":null,"rootUri":Url::from_directory_path(root).unwrap(),"capabilities":{"workspace":{"workspaceEdit":{"documentChanges":true}},"textDocument":{"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix"]}}}}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"version":1,"languageId":"latex","text":source}}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    let diagnostics = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == uri.as_str()
            && message["params"]["diagnostics"]
                .as_array()
                .is_some_and(|diagnostics| {
                    diagnostics.iter().any(|diagnostic| {
                        diagnostic["message"]
                            .as_str()
                            .is_some_and(|message| message.contains("Reference `sec:itnro'"))
                    })
                })
    });
    let diagnostics: Vec<_> = diagnostics["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| {
            diagnostic["message"].as_str().is_some_and(|message| {
                message.contains("undefined")
                    && (message.contains("Reference `sec:itnro'")
                        || message.contains("Citation `smith2025'"))
            })
        })
        .cloned()
        .collect();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    let mut edits = Vec::new();
    for (index, diagnostic) in diagnostics.into_iter().enumerate() {
        let actions = server.request(3 + index as u64,"textDocument/codeAction",json!({"textDocument":{"uri":uri},"range":diagnostic["range"],"context":{"diagnostics":[diagnostic],"only":["quickfix"]}}));
        assert_eq!(actions.as_array().unwrap().len(), 1, "{actions}");
        let edit = &actions[0]["edit"]["documentChanges"][0];
        assert_eq!(edit["textDocument"]["version"], 1);
        edits.push(
            serde_json::from_value::<tower_lsp::lsp_types::TextEdit>(edit["edits"][0].clone())
                .unwrap(),
        );
    }
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    let repaired = latex_lsp::text::Text::new(source.into())
        .changed(
            edits
                .into_iter()
                .map(
                    |edit| tower_lsp::lsp_types::TextDocumentContentChangeEvent {
                        range: Some(edit.range),
                        range_length: None,
                        text: edit.new_text,
                    },
                )
                .collect(),
        )
        .unwrap()
        .source;
    assert!(repaired.contains("\\ref{sec:intro} and \\cite{smith2026}"));
    std::fs::write(root.join("main.tex"), &repaired).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":repaired}]}),
    );
    server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let second = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(second["params"]["status"], "success", "{second}");
    assert_eq!(second["params"]["stale"], false);
    assert_eq!(second["params"]["diagnosticsCount"], 0, "{second}");
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

fn assert_no_pushed_diagnostics(server: &Server) {
    assert!(
        server
            .pending
            .borrow()
            .iter()
            .all(|message| { message["method"] != "textDocument/publishDiagnostics" }),
        "pull client received pushed diagnostics"
    );
}

#[test]
fn pull_diagnostics_refresh_without_blocking_and_keep_result_ids_scoped_to_document_versions() {
    let directory = tempfile::tempdir().unwrap();
    let uri = Url::from_file_path(directory.path().join("章节.tex")).unwrap();
    let alternate = uri
        .as_str()
        .replace("%E7%AB%A0%E8%8A%82", "%e7%ab%a0%e8%8a%82");
    let mut server = Server::start();
    let init = server.request(1000, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(directory.path()).unwrap(),"capabilities":{"textDocument":{"diagnostic":{}},"workspace":{"diagnostic":{"refreshSupport":true}}}}));
    assert_eq!(
        init["capabilities"]["diagnosticProvider"],
        json!({"identifier":"latex-lsp","interFileDependencies":true,"workspaceDiagnostics":true})
    );
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":alternate,"languageId":"latex","version":1,"text":"😀{"}}),
    );
    let refresh = server.until(|message| message["method"] == "workspace/diagnostic/refresh");
    // Pull before acknowledging refresh: the server's wire-order queue must be free.
    let first = server.request(
        1001,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":alternate},"identifier":"latex-lsp"}),
    );
    assert_eq!(first["kind"], "full");
    assert_eq!(first["items"].as_array().unwrap().len(), 1, "{first}");
    assert_eq!(first["items"][0]["range"]["start"]["character"], 2);
    let unchanged = server.request(
        1002,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":uri},"previousResultId":first["resultId"]}),
    );
    assert_eq!(
        unchanged,
        json!({"kind":"unchanged","resultId":first["resultId"]})
    );
    let full = server.request(
        1003,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":uri},"previousResultId":"unknown"}),
    );
    assert_eq!(full, first);
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":alternate,"version":2},"contentChanges":[{"text":"😀{}"}]}),
    );
    let cleared = server.request(
        1004,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":uri},"previousResultId":first["resultId"]}),
    );
    assert_eq!(cleared["kind"], "full");
    assert_eq!(cleared["items"], json!([]));
    assert_ne!(cleared["resultId"], first["resultId"]);
    server.send(json!({"jsonrpc":"2.0","id":refresh["id"],"result":null}));
    // A second invalidation survives even though the earlier refresh was pending.
    let second_refresh =
        server.until(|message| message["method"] == "workspace/diagnostic/refresh");
    server.send(json!({"jsonrpc":"2.0","id":second_refresh["id"],"result":null}));
    server.notify(
        "textDocument/didClose",
        json!({"textDocument":{"uri":alternate}}),
    );
    let removed = server.request(
        1005,
        "workspace/diagnostic",
        json!({"previousResultIds":[{"uri":alternate,"value":cleared["resultId"]}]}),
    );
    assert_eq!(removed["items"].as_array().unwrap().len(), 1);
    assert_eq!(removed["items"][0]["uri"], uri.as_str());
    assert_eq!(removed["items"][0]["kind"], "full");
    assert_eq!(removed["items"][0]["items"], json!([]));
    assert_eq!(removed["items"][0]["version"], Value::Null);
    server.send(json!({"jsonrpc":"2.0","id":1006,"method":"textDocument/diagnostic","params":{"textDocument":{"uri":uri},"identifier":"another-provider"}}));
    let error = server.until(|message| message["id"] == 1006);
    assert_eq!(error["error"]["code"], -32602);
    server.request(1007, "shutdown", Value::Null);
    assert_no_pushed_diagnostics(&server);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn workspace_pull_diagnostics_follow_project_dependencies_deletions_and_renames() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    std::fs::write(
        path.join("main.tex"),
        r"\documentclass{article}\input{a}\input{b}",
    )
    .unwrap();
    std::fs::write(path.join("a.tex"), r"\label{same}").unwrap();
    std::fs::write(path.join("b.tex"), r"\label{same}").unwrap();
    let a = Url::from_file_path(path.join("a.tex")).unwrap();
    let b = Url::from_file_path(path.join("b.tex")).unwrap();
    let moved = Url::from_file_path(path.join("moved.tex")).unwrap();
    let mut server = Server::start();
    server.request(1100, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(path).unwrap(),"capabilities":{"textDocument":{"diagnostic":{}}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":a,"languageId":"latex","version":7,"text":"\\label{same}"}}),
    );
    let first = server.request(
        1101,
        "workspace/diagnostic",
        json!({"previousResultIds":[]}),
    );
    let items = first["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    for uri in [&a, &b] {
        let item = items
            .iter()
            .find(|item| item["uri"] == uri.as_str())
            .unwrap();
        assert_eq!(item["items"].as_array().unwrap().len(), 1, "{item}");
        assert_eq!(item["items"][0]["code"], "duplicate-definition");
        assert_eq!(
            item["version"],
            if uri == &a { json!(7) } else { Value::Null }
        );
    }
    let previous: Vec<_> = items
        .iter()
        .map(|item| json!({"uri":item["uri"],"value":item["resultId"]}))
        .collect();
    let unchanged = server.request(
        1102,
        "workspace/diagnostic",
        json!({"previousResultIds":previous}),
    );
    assert!(unchanged["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["kind"] == "unchanged"));
    std::fs::remove_file(path.join("b.tex")).unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":b,"type":3}]}),
    );
    let cleared = server.request(
        1103,
        "workspace/diagnostic",
        json!({"previousResultIds":previous}),
    );
    for uri in [&a, &b] {
        let item = cleared["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["uri"] == uri.as_str())
            .unwrap();
        assert_eq!(item["kind"], "full", "{item}");
        assert_eq!(item["items"], json!([]));
    }
    let previous: Vec<_> = cleared["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| json!({"uri":item["uri"],"value":item["resultId"]}))
        .collect();
    std::fs::rename(path.join("a.tex"), path.join("moved.tex")).unwrap();
    server.notify(
        "workspace/didRenameFiles",
        json!({"files":[{"oldUri":a,"newUri":moved}]}),
    );
    let renamed = server.request(
        1104,
        "workspace/diagnostic",
        json!({"previousResultIds":previous}),
    );
    let items = renamed["items"].as_array().unwrap();
    let old = items.iter().find(|item| item["uri"] == a.as_str()).unwrap();
    assert_eq!(old["kind"], "full");
    assert_eq!(old["version"], Value::Null);
    assert_eq!(old["items"], json!([]));
    let new = items
        .iter()
        .find(|item| item["uri"] == moved.as_str())
        .unwrap();
    assert_eq!(new["kind"], "full");
    assert_eq!(new["version"], 7);
    assert_eq!(new["items"], json!([]));
    server.request(1105, "shutdown", Value::Null);
    assert_no_pushed_diagnostics(&server);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[ignore = "requires pdflatex"]
fn compiler_pull_diagnostics_refresh_and_offer_versioned_spelling_fixes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.tex");
    let uri = Url::from_file_path(&path).unwrap();
    let source = "\\documentclass{article}\n\\begin{document}\n$\\epsilonn$\n\\end{document}\n";
    std::fs::write(&path, source).unwrap();
    let mut server = Server::start();
    server.request(1200, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(directory.path()).unwrap(),"capabilities":{"textDocument":{"diagnostic":{},"codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix"]}}}},"workspace":{"workspaceEdit":{"documentChanges":true},"diagnostic":{"refreshSupport":true}}},"initializationOptions":{"build":{"executable":"pdflatex"}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":3,"text":source}}),
    );
    let refresh = server.until(|message| message["method"] == "workspace/diagnostic/refresh");
    server.send(json!({"jsonrpc":"2.0","id":refresh["id"],"result":null}));
    let before = server.request(
        1201,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":uri}}),
    );
    assert_eq!(before["items"], json!([]));
    server.request(
        1202,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "error");
    let refresh = server.until(|message| message["method"] == "workspace/diagnostic/refresh");
    let built = server.request(
        1203,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":uri},"previousResultId":before["resultId"]}),
    );
    assert_eq!(built["kind"], "full");
    let diagnostic = built["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "undefined-control-sequence")
        .unwrap();
    assert_eq!(diagnostic["data"]["command"], "epsilonn");
    assert_eq!(
        diagnostic["range"]["start"],
        json!({"line":2,"character":1})
    );
    let actions = server.request(1204, "textDocument/codeAction", json!({"textDocument":{"uri":uri},"range":diagnostic["range"],"context":{"diagnostics":[diagnostic],"only":["quickfix"]}}));
    let fix = actions
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["title"] == "Replace with command `\\epsilon`")
        .unwrap();
    assert_eq!(
        fix["edit"]["documentChanges"][0]["textDocument"]["version"],
        3
    );
    let fixed = source.replace("epsilonn", "epsilon");
    std::fs::write(&path, &fixed).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":4},"contentChanges":[{"text":fixed}]}),
    );
    let cleared = server.request(
        1205,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":uri},"previousResultId":built["resultId"]}),
    );
    assert_eq!(cleared["kind"], "full");
    assert_eq!(cleared["items"], json!([]));
    server.send(json!({"jsonrpc":"2.0","id":refresh["id"],"result":null}));
    let refresh = server.until(|message| message["method"] == "workspace/diagnostic/refresh");
    server.send(json!({"jsonrpc":"2.0","id":refresh["id"],"result":null}));
    server.request(
        1206,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[uri]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    let unchanged = server.request(
        1207,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":uri},"previousResultId":cleared["resultId"]}),
    );
    assert_eq!(unchanged["kind"], "unchanged");
    let workspace = server.request(
        1208,
        "workspace/diagnostic",
        json!({"previousResultIds":[]}),
    );
    let report = workspace["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["uri"] == uri.as_str())
        .unwrap();
    assert_eq!(report["version"], 4);
    assert_eq!(report["items"], json!([]));
    server.request(1209, "shutdown", Value::Null);
    assert_no_pushed_diagnostics(&server);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
fn file_event_notifications_refresh_navigation_and_duplicate_diagnostics_without_watchers() {
    for pull in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let main = Url::from_file_path(path.join("main.tex")).unwrap();
        let folder = Url::from_file_path(path.join("chapters")).unwrap();
        let child = Url::from_file_path(path.join("chapters/child.tex")).unwrap();
        let source = r"\documentclass{article}\input{chapters/child}\label{shared}\ref{shared}";
        std::fs::write(path.join("main.tex"), source).unwrap();
        let capabilities = if pull {
            json!({"textDocument":{"diagnostic":{}}})
        } else {
            json!({})
        };
        let mut server = Server::start();
        let init = server.request(1300, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(path).unwrap(),"capabilities":capabilities}));
        for event in ["didCreate", "didDelete"] {
            assert_eq!(
                init["capabilities"]["workspace"]["fileOperations"][event]["filters"][0]["scheme"],
                "file"
            );
        }
        server.notify("initialized", json!({}));
        server.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":main,"languageId":"latex","version":3,"text":source}}),
        );
        assert_eq!(
            server
                .request(1301, "workspace/symbol", json!({"query":"shared"}))
                .as_array()
                .unwrap()
                .len(),
            1
        );
        std::fs::create_dir(path.join("chapters")).unwrap();
        std::fs::write(path.join("chapters/child.tex"), r"\label{shared}").unwrap();
        // Reject the whole invalid batch before indexing its valid first item.
        server.notify(
            "workspace/didCreateFiles",
            json!({"files":[{"uri":folder},{"uri":"https://example.test/remote.tex"}]}),
        );
        assert_eq!(
            server
                .request(1302, "workspace/symbol", json!({"query":"shared"}))
                .as_array()
                .unwrap()
                .len(),
            1
        );
        server.notify(
            "workspace/didCreateFiles",
            json!({"files":[{"uri":folder}]}),
        );
        let symbols = server.request(1303, "workspace/symbol", json!({"query":"shared"}));
        assert_eq!(symbols.as_array().unwrap().len(), 2);
        assert!(symbols
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["location"]["uri"] == child.as_str()));
        let diagnostic = server.request(
            1304,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main}}),
        );
        assert_eq!(
            diagnostic["items"].as_array().unwrap().len(),
            1,
            "{diagnostic}"
        );
        assert_eq!(diagnostic["items"][0]["code"], "duplicate-definition");
        std::fs::remove_dir_all(path.join("chapters")).unwrap();
        server.notify(
            "workspace/didDeleteFiles",
            json!({"files":[{"uri":folder}]}),
        );
        let cleared = server.request(
            1305,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main},"previousResultId":diagnostic["resultId"]}),
        );
        assert_eq!(cleared["kind"], "full");
        assert_eq!(cleared["items"], json!([]));
        let workspace = server.request(
            1306,
            "workspace/diagnostic",
            json!({"previousResultIds":[]}),
        );
        let main_report = workspace["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["uri"] == main.as_str())
            .unwrap();
        assert_eq!(main_report["version"], 3);
        assert_eq!(
            server
                .request(1307, "workspace/symbol", json!({"query":"shared"}))
                .as_array()
                .unwrap()
                .len(),
            1
        );
        server.request(1308, "shutdown", Value::Null);
        if pull {
            assert_no_pushed_diagnostics(&server);
        }
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[ignore = "requires pdflatex"]
fn real_file_events_clear_missing_input_errors_and_reindex_deleted_chapters() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let main = Url::from_file_path(path.join("main.tex")).unwrap();
    let folder = Url::from_file_path(path.join("chapters")).unwrap();
    let source =
        "\\documentclass{article}\n\\begin{document}\n\\input{chapters/child}\n\\end{document}\n";
    std::fs::write(path.join("main.tex"), source).unwrap();
    let mut server = Server::start();
    server.request(1400, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(path).unwrap(),"capabilities":{"textDocument":{"diagnostic":{}}},"initializationOptions":{"build":{"executable":"pdflatex"}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":main,"languageId":"latex","version":1,"text":source}}),
    );
    server.request(
        1401,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let failed = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(failed["params"]["status"], "error");
    let missing = server.request(
        1402,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main}}),
    );
    assert!(
        missing["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["message"].as_str().unwrap().contains("chapters/child")),
        "{missing}"
    );
    std::fs::create_dir(path.join("chapters")).unwrap();
    std::fs::write(
        path.join("chapters/child.tex"),
        r"\newcommand{\fresh}{Created source.}\fresh",
    )
    .unwrap();
    server.notify(
        "workspace/didCreateFiles",
        json!({"files":[{"uri":folder}]}),
    );
    let cleared = server.request(
        1403,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main},"previousResultId":missing["resultId"]}),
    );
    assert_eq!(cleared["kind"], "full");
    assert_eq!(cleared["items"], json!([]));
    assert_eq!(
        server
            .request(1404, "workspace/symbol", json!({"query":"fresh"}))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    server.request(
        1405,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let built = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(built["params"]["status"], "success", "{built}");
    std::fs::remove_dir_all(path.join("chapters")).unwrap();
    server.notify(
        "workspace/didDeleteFiles",
        json!({"files":[{"uri":folder}]}),
    );
    assert_eq!(
        server.request(1406, "workspace/symbol", json!({"query":"fresh"})),
        json!([])
    );
    server.request(
        1407,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let deleted = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(deleted["params"]["status"], "error");
    server.request(1408, "shutdown", Value::Null);
    assert_no_pushed_diagnostics(&server);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn deleted_input_directories_cancel_active_and_queued_builds_but_unrelated_files_do_not() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    std::fs::create_dir(path.join("chapters")).unwrap();
    std::fs::write(path.join("chapters/child.tex"), r"\label{chapter}").unwrap();
    let source = r"\documentclass{article}\input{chapters/child}";
    std::fs::write(path.join("main.tex"), source).unwrap();
    let main = Url::from_file_path(path.join("main.tex")).unwrap();
    let folder = Url::from_file_path(path.join("chapters")).unwrap();
    let unrelated = Url::from_file_path(path.join("other.tex")).unwrap();
    let mut server = Server::start();
    server.request(1500, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(path).unwrap(),"capabilities":{},"initializationOptions":{"build":{"executable":"sh","arguments":["-c","sleep 30"],"onSave":true,"timeoutSeconds":5}}}));
    server.notify("initialized", json!({}));
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":main,"languageId":"latex","version":1,"text":source}}),
    );
    server.request(
        1501,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    server.until(|message| message["method"] == "latex/buildStarted");
    std::fs::write(path.join("other.tex"), "Unrelated.").unwrap();
    server.notify(
        "workspace/didCreateFiles",
        json!({"files":[{"uri":unrelated}]}),
    );
    let running = server.request(
        1502,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[main]}),
    );
    assert_eq!(running["status"], "running");
    server.notify("textDocument/didSave", json!({"textDocument":{"uri":main}}));
    let queued = server.request(
        1503,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[main]}),
    );
    assert_eq!(queued["queued"], true);
    std::fs::remove_dir_all(path.join("chapters")).unwrap();
    server.notify(
        "workspace/didDeleteFiles",
        json!({"files":[{"uri":folder}]}),
    );
    let cancelled = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(cancelled["params"]["status"], "cancelled", "{cancelled}");
    let status = server.request(
        1504,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[main]}),
    );
    assert_eq!(status["status"], "cancelled");
    assert_eq!(
        server.request(1505, "workspace/symbol", json!({"query":"chapter"})),
        json!([])
    );
    server.request(1506, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[cfg(unix)]
struct OwnedProcessGroup(i32);

#[cfg(unix)]
impl Drop for OwnedProcessGroup {
    fn drop(&mut self) {
        // All groups used here were created by this test's compiler/query.
        // A failed regression must not leave its sleep processes behind.
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
fn assert_process_stopped(pid: i32) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .is_some_and(|stat| {
                stat.rsplit_once(')')
                    .is_some_and(|(_, state)| state.trim_start().starts_with('Z'))
            });
        if !alive || zombie {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "external process {pid} survived disconnect"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
impl Server {
    fn terminate_transport(mut self, mode: &str) -> std::process::ExitStatus {
        if mode != "eof" {
            self.notify("exit", Value::Null);
        }
        if mode != "exit" {
            drop(self.stdin.take());
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("server did not terminate promptly after {mode}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
#[cfg(unix)]
fn disconnects_reap_compilers_and_descendants_without_waiting_for_their_timeout() {
    for mode in ["exit", "eof", "exit-eof"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        std::fs::write(path.join("main.tex"), r"\documentclass{article}").unwrap();
        let uri = Url::from_file_path(path.join("main.tex")).unwrap();
        let script = "printf '%s' \"$$\" > compiler-parent\nsleep 30 &\nprintf '%s' \"$!\" > compiler-child\ntouch compiler-ready\nwait\n";
        let mut server = Server::start();
        server.request(1600, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(path).unwrap(),"capabilities":{},"initializationOptions":{"build":{"executable":"sh","arguments":["-c",script],"timeoutSeconds":30}}}));
        server.notify("initialized", json!({}));
        server.request(
            1601,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[uri]}),
        );
        wait_for_file(&path.join("compiler-ready"));
        let parent: i32 = std::fs::read_to_string(path.join("compiler-parent"))
            .unwrap()
            .parse()
            .unwrap();
        let descendant: i32 = std::fs::read_to_string(path.join("compiler-child"))
            .unwrap()
            .parse()
            .unwrap();
        let _group = OwnedProcessGroup(parent);
        let status = server.terminate_transport(mode);
        assert_eq!(
            status.code(),
            Some(if mode == "eof" { 0 } else { 1 }),
            "{mode}"
        );
        assert_process_stopped(parent);
        assert_process_stopped(descendant);
    }
}

#[test]
#[cfg(unix)]
fn disconnects_cancel_simultaneous_synctex_queries_and_builds_and_reap_both_groups() {
    for mode in ["exit", "eof", "exit-eof"] {
        let script = "printf '%s' \"$$\" > query-parent\nsleep 30 &\nprintf '%s' \"$!\" > query-child\ntouch query-ready\nwait\n";
        let (mut server, directory, uri) = fake_synctex_server(script);
        let path = directory.path();
        server.send(json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":uri,"position":{"line":1,"character":0}}]}}));
        wait_for_file(&path.join("query-ready"));
        let query: i32 = std::fs::read_to_string(path.join("query-parent"))
            .unwrap()
            .parse()
            .unwrap();
        let query_child: i32 = std::fs::read_to_string(path.join("query-child"))
            .unwrap()
            .parse()
            .unwrap();
        let _query_group = OwnedProcessGroup(query);
        let script = "printf '%s' \"$$\" > compiler-parent\nsleep 30 &\nprintf '%s' \"$!\" > compiler-child\ntouch compiler-ready\nwait\n";
        server.notify("workspace/didChangeConfiguration", json!({"settings":{"build":{"executable":"sh","arguments":["-c",script],"timeoutSeconds":30}}}));
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[uri]}),
        );
        wait_for_file(&path.join("compiler-ready"));
        let compiler: i32 = std::fs::read_to_string(path.join("compiler-parent"))
            .unwrap()
            .parse()
            .unwrap();
        let compiler_child: i32 = std::fs::read_to_string(path.join("compiler-child"))
            .unwrap()
            .parse()
            .unwrap();
        let _compiler_group = OwnedProcessGroup(compiler);
        let status = server.terminate_transport(mode);
        assert_eq!(
            status.code(),
            Some(if mode == "eof" { 0 } else { 1 }),
            "{mode}"
        );
        for pid in [query, query_child, compiler, compiler_child] {
            assert_process_stopped(pid);
        }
    }
}

#[test]
#[cfg(unix)]
fn disconnects_do_not_wait_for_unacknowledged_client_registration_requests() {
    for mode in ["exit", "eof", "exit-eof"] {
        let directory = tempfile::tempdir().unwrap();
        let mut server = Server::start();
        server.request(1700, "initialize", json!({"processId":null,"rootUri":Url::from_directory_path(directory.path()).unwrap(),"capabilities":{"workspace":{"didChangeWatchedFiles":{"dynamicRegistration":true}}}}));
        server.notify("initialized", json!({}));
        server.until(|message| message["method"] == "client/registerCapability");
        assert_eq!(
            server.terminate_transport(mode).code(),
            Some(if mode == "eof" { 0 } else { 1 })
        );
    }
}
#[test]
#[cfg(unix)]
fn missing_inputs_under_symlink_directories_match_canonical_creation_notifications() {
    for (command, selected) in [
        (
            r"\IfFileExists{alias/deep/nested/optional.bin}{Found.}{Missing.}",
            true,
        ),
        (
            r"\InputIfFileExists{alias/deep/nested/optional.bin}{}{}",
            true,
        ),
        (r"\input{alias/deep/nested/optional.bin}", true),
        (r"\subfile{alias/deep/nested/optional.bin}", true),
        (r"\include{alias/deep/nested/optional}", true),
        (
            r"\includeonly{other}\include{alias/deep/nested/optional}",
            false,
        ),
    ] {
        for (watcher, directory_event) in [(true, false), (true, true), (false, true)] {
            let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\n";
            let (mut server, directory, main) = fake_build_server(script, true);
            let physical = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(physical.path(), directory.path().join("alias")).unwrap();
            let source = format!("\\documentclass{{article}}\n{command}\n");
            std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
            server.notify(
                "textDocument/didChange",
                json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
            );
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let output = directory.path().join("build/latex-lsp");
            wait_for_file(&output.join("ready"));
            server.notify("textDocument/didSave", json!({"textDocument":{"uri":main}}));
            let queued = server.request(
                3,
                "workspace/executeCommand",
                json!({"command":"latex.buildStatus","arguments":[main]}),
            );
            assert_eq!(queued["queued"], true, "{queued}");
            std::fs::create_dir_all(physical.path().join("deep/nested")).unwrap();
            let target = physical.path().join(if command.contains("\\include") {
                "deep/nested/optional.tex"
            } else {
                "deep/nested/optional.bin"
            });
            std::fs::write(&target, "Created externally.\n").unwrap();
            let changed = if directory_event {
                physical.path().join("deep")
            } else {
                target
            };
            let event = Url::from_file_path(changed).unwrap();
            if watcher {
                server.notify(
                    "workspace/didChangeWatchedFiles",
                    json!({"changes":[{"uri":event,"type":1}]}),
                );
            } else {
                server.notify("workspace/didCreateFiles", json!({"files":[{"uri":event}]}));
            }
            let status = server.request(
                4,
                "workspace/executeCommand",
                json!({"command":"latex.buildStatus","arguments":[main]}),
            );
            std::fs::write(output.join("release"), "").unwrap();
            let finished = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(
                finished["params"]["status"],
                if selected { "cancelled" } else { "success" },
                "{command}, {watcher}, {directory_event}: {finished}"
            );
            if selected {
                assert_ne!(status["queued"], true, "{status}");
            } else {
                assert_eq!(finished["params"]["stale"], false, "{finished}");
            }
            server.request(5, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn canonical_creation_events_clear_completed_diagnostics_for_missing_symlink_inputs() {
    let script = "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: missing symlink input.\\n' \"$2\" > \"$1/main.log\"\n";
    let (mut server, directory, main) =
        fake_build_server_capabilities(script, false, json!({"textDocument":{"diagnostic":{}}}));
    let physical = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(physical.path(), directory.path().join("alias")).unwrap();
    let source =
        "\\documentclass{article}\n\\IfFileExists{alias/nested/optional.bin}{Found.}{Missing.}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    let old = server.request(
        3,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main}}),
    );
    assert!(
        old["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["message"]
                .as_str()
                .is_some_and(|text| text.contains("missing symlink input"))),
        "{old}"
    );
    std::fs::create_dir(physical.path().join("nested")).unwrap();
    let target = physical.path().join("nested/optional.bin");
    std::fs::write(&target, "Created.\n").unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":Url::from_file_path(target).unwrap(),"type":1}]}),
    );
    let cleared = server.request(
        4,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
    );
    assert_eq!(cleared["kind"], "full", "{cleared}");
    assert_eq!(cleared["items"], json!([]), "{cleared}");
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn repeated_watched_events_preserve_current_build_diagnostics_for_all_input_kinds() {
    let script = "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: current inputs warning.\\n' \"$2\" > \"$1/main.log\"\nprintf 'INPUT %s\\nINPUT %s/child.tex\\nINPUT %s/dynamic.ltx\\nINPUT %s/plot.png\\nINPUT %s/refs.bib\\n' \"$2\" \"${2%/*}\" \"${2%/*}\" \"${2%/*}\" \"${2%/*}\" > \"$1/main.fls\"\n";
    let (mut server, directory, main) =
        fake_build_server_capabilities(script, false, json!({"textDocument":{"diagnostic":{}}}));
    let source = "\\documentclass{article}\n\\input{child}\n\\includegraphics{plot.png}\n\\bibliography{refs}\n\\IfFileExists{optional.bin}{Found.}{Missing.}\n";
    for (name, contents) in [
        ("child.tex", "Old child.\n"),
        ("dynamic.ltx", "Dynamic.\n"),
        ("refs.bib", "@book{book,title={Example}}\n"),
    ] {
        std::fs::write(directory.path().join(name), contents).unwrap();
    }
    std::fs::write(directory.path().join("plot.png"), [0, 255, 1]).unwrap();
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let finished = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(finished["params"]["status"], "success", "{finished}");
    assert_eq!(finished["params"]["stale"], false, "{finished}");
    let old = server.request(
        3,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main}}),
    );
    assert!(
        old["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["message"]
                .as_str()
                .is_some_and(|text| text.contains("current inputs warning"))),
        "{old}"
    );
    for (id, name, kind) in [
        (4, "main.tex", 2),
        (5, "child.tex", 1),
        (6, "dynamic.ltx", 2),
        (7, "plot.png", 2),
        (8, "refs.bib", 2),
        (9, "optional.bin", 3),
        (10, "", 1),
    ] {
        server.notify("workspace/didChangeWatchedFiles", json!({"changes":[{"uri":Url::from_file_path(directory.path().join(name)).unwrap(),"type":kind}]}));
        let current = server.request(
            id,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
        );
        assert_eq!(current["kind"], "unchanged", "{name}: {current}");
        assert_eq!(current["resultId"], old["resultId"]);
    }
    let image = directory.path().join("plot.png");
    let image_time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1);
    std::fs::File::open(&image)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(image_time))
        .unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":Url::from_file_path(&image).unwrap(),"type":2}]}),
    );
    let touched = server.request(
        20,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
    );
    assert_eq!(touched["kind"], "unchanged", "{touched}");
    let child = directory.path().join("child.tex");
    let modified = std::fs::metadata(&child).unwrap().modified().unwrap();
    std::fs::write(&child, "New child.\n").unwrap();
    std::fs::File::open(&child)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":Url::from_file_path(child).unwrap(),"type":2}]}),
    );
    let cleared = server.request(
        11,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
    );
    assert_eq!(cleared["kind"], "full", "{cleared}");
    assert_eq!(cleared["items"], json!([]), "{cleared}");
    server.request(
        21,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
    let current = server.request(
        22,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main}}),
    );
    assert!(
        current["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["source"] == "latex-build"),
        "{current}"
    );
    std::fs::write(&image, [1, 255, 0]).unwrap();
    std::fs::File::open(&image)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(image_time))
        .unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":Url::from_file_path(image).unwrap(),"type":2}]}),
    );
    let cleared_image = server.request(
        23,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main},"previousResultId":current["resultId"]}),
    );
    assert_eq!(cleared_image["kind"], "full", "{cleared_image}");
    assert_eq!(cleared_image["items"], json!([]), "{cleared_image}");
    server.request(24, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn watched_shared_inputs_compare_each_roots_published_build_instead_of_the_latest_read() {
    let script = "name=${2##*/}\nname=${name%.tex}\nprintf '%%PDF-1.4\\n' > \"$1/$name.pdf\"\nprintf '%s:2: LaTeX Warning: project-specific input warning.\\n' \"$2\" > \"$1/$name.log\"\n";
    let (mut server, directory, main) =
        fake_build_server_capabilities(script, false, json!({"textDocument":{"diagnostic":{}}}));
    let source = "\\documentclass{article}\n\\input{shared}\n";
    let shared = directory.path().join("shared.tex");
    let other = Url::from_file_path(directory.path().join("other.tex")).unwrap();
    std::fs::write(&shared, "Old.\n").unwrap();
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    std::fs::write(other.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":other,"languageId":"latex","version":1,"text":source}}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    std::fs::write(&shared, "New.\n").unwrap();
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[other]}),
    );
    let second = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(second["params"]["status"], "success", "{second}");
    let old_main = server.request(
        4,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main}}),
    );
    let old_other = server.request(
        5,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":other}}),
    );
    for old in [&old_main, &old_other] {
        assert!(
            old["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["source"] == "latex-build"),
            "{old}"
        );
    }
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":Url::from_file_path(shared).unwrap(),"type":2}]}),
    );
    let cleared = server.request(
        6,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main},"previousResultId":old_main["resultId"]}),
    );
    assert_eq!(cleared["kind"], "full", "{cleared}");
    assert_eq!(cleared["items"], json!([]), "{cleared}");
    let retained = server.request(
        7,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":other},"previousResultId":old_other["resultId"]}),
    );
    assert_eq!(retained["kind"], "unchanged", "{retained}");
    server.request(8, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn delayed_watchers_preserve_active_saved_builds_while_clearing_older_published_diagnostics() {
    let script = "if [ -f \"$1/hold\" ]; then touch \"$1/ready\"; while [ ! -f \"$1/release\" ]; do sleep 0.02; done; fi\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: warning for compiled inputs.\\n' \"$2\" > \"$1/main.log\"\n";
    let (mut server, directory, main) =
        fake_build_server_capabilities(script, false, json!({"textDocument":{"diagnostic":{}}}));
    let source = "\\documentclass{article}\n\\input{child}\n";
    let child = directory.path().join("child.tex");
    std::fs::write(&child, "Old.\n").unwrap();
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    let old = server.request(
        3,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main}}),
    );
    assert!(
        old["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["source"] == "latex-build"),
        "{old}"
    );
    std::fs::write(&child, "New.\n").unwrap();
    let output = directory.path().join("build/latex-lsp");
    std::fs::write(output.join("hold"), "").unwrap();
    server.request(
        4,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    wait_for_file(&output.join("ready"));
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes":[{"uri":Url::from_file_path(child).unwrap(),"type":2}]}),
    );
    let status = server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.buildStatus","arguments":[main]}),
    );
    let cleared = server.request(
        6,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
    );
    std::fs::write(output.join("release"), "").unwrap();
    assert_eq!(status["status"], "running", "{status}");
    assert_eq!(cleared["kind"], "full", "{cleared}");
    assert_eq!(cleared["items"], json!([]), "{cleared}");
    let second = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(second["params"]["status"], "success", "{second}");
    assert_eq!(second["params"]["stale"], false, "{second}");
    server.request(7, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn binary_input_content_changes_are_detected_even_with_preserved_size_and_timestamp() {
    for notify in [false, true] {
        let script = "touch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf '%s:2: LaTeX Warning: old image warning.\\n' \"$2\" > \"$1/main.log\"\n";
        let (mut server, directory, main) = fake_build_server(script, true);
        let source = "\\documentclass{article}\n\\includegraphics{plot.png}\n";
        let image = directory.path().join("plot.png");
        let mut pixels = vec![0; 128 * 1024 + 17];
        pixels[0] = 255;
        std::fs::write(&image, &pixels).unwrap();
        let modified = std::fs::metadata(&image).unwrap().modified().unwrap();
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        wait_for_file(&output.join("ready"));
        if notify {
            server.notify("textDocument/didSave", json!({"textDocument":{"uri":main}}));
            let queued = server.request(
                3,
                "workspace/executeCommand",
                json!({"command":"latex.buildStatus","arguments":[main]}),
            );
            assert_eq!(queued["queued"], true, "{queued}");
        }
        pixels[64 * 1024 + 7] = 1;
        std::fs::write(&image, &pixels).unwrap();
        std::fs::File::open(&image)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        if notify {
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":Url::from_file_path(&image).unwrap(),"type":2}]}),
            );
            server.request(
                4,
                "workspace/executeCommand",
                json!({"command":"latex.buildStatus","arguments":[main]}),
            );
        }
        std::fs::write(output.join("release"), "").unwrap();
        let finished = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            finished["params"]["status"],
            if notify { "cancelled" } else { "success" },
            "{notify}: {finished}"
        );
        if !notify {
            assert_eq!(finished["params"]["stale"], true, "{finished}");
        }
        server.request(5, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn old_pdf_input_contents_remain_protected_through_noop_errors_until_regenerated() {
    for (name, command, original, replacement) in [
        (
            "child.tex",
            r"\input{child}",
            b"Old.\n".as_slice(),
            b"New.\n".as_slice(),
        ),
        (
            "plot.png",
            r"\includegraphics{plot.png}",
            &[0, 255, 1],
            &[1, 255, 0],
        ),
        (
            "refs.bib",
            r"\bibliography{refs}",
            b"@book{book,title={Old}}\n",
            b"@book{book,title={New}}\n",
        ),
        (
            "local.bst",
            r"\bibliographystyle{local}",
            b"Old.\n",
            b"New.\n",
        ),
        ("dynamic.ltx", r"\input{\chosen}", b"Old.\n", b"New.\n"),
    ] {
        let script = format!("if [ ! -f \"$1/main.pdf\" ] || [ -f \"$1/rebuild\" ]; then\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nif [ -f \"$1/rebuild\" ]; then printf 'Regenerated\\n' >> \"$1/main.pdf\"; fi\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\nINPUT %s/{name}\\n' \"$2\" \"${{2%/*}}\" > \"$1/main.fls\"\nfi\n");
        let (mut server, directory, main) = fake_build_server(&script, false);
        let input = directory.path().join(name);
        std::fs::write(&input, original).unwrap();
        let modified = std::fs::metadata(&input).unwrap().modified().unwrap();
        let source = format!("\\documentclass{{article}}\n{command}\n");
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{name}: {first}");
        assert_eq!(first["params"]["stale"], false, "{first}");
        assert_eq!(original.len(), replacement.len());
        std::fs::write(&input, replacement).unwrap();
        std::fs::File::open(&input)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        server.send(json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":{"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]}}));
        let rejected = server.until(|message| message["id"] == 3);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Input contents changed")),
            "{name}: {rejected}"
        );
        for id in [4, 5] {
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(noop["params"]["status"], "error", "{name}: {noop}");
            assert_eq!(noop["params"]["stale"], false, "{noop}");
            assert!(
                noop["params"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("input content changed")),
                "{noop}"
            );
            assert!(noop["params"]["pdfUri"].is_null());
            assert!(noop["params"]["pages"].is_null());
        }
        std::fs::write(directory.path().join("build/latex-lsp/rebuild"), "").unwrap();
        server.request(
            6,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{name}: {rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        server.request(7, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn first_stale_pdf_retains_original_binary_input_contents_across_repeated_noops() {
    let script = "if [ ! -f \"$1/main.pdf\" ]; then\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\ntouch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nfi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let input = directory.path().join("plot.png");
    std::fs::write(&input, [0, 255, 1]).unwrap();
    let modified = std::fs::metadata(&input).unwrap().modified().unwrap();
    let source = "\\documentclass{article}\n\\includegraphics{plot.png}\n";
    std::fs::write(main.to_file_path().unwrap(), source).unwrap();
    server.notify(
        "textDocument/didChange",
        json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
    );
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let output = directory.path().join("build/latex-lsp");
    wait_for_file(&output.join("ready"));
    std::fs::write(&input, [1, 255, 0]).unwrap();
    std::fs::File::open(&input)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    std::fs::write(output.join("release"), "").unwrap();
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    assert_eq!(first["params"]["stale"], true, "{first}");
    for id in [3, 4] {
        server.request(
            id,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let noop = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(noop["params"]["status"], "error", "{noop}");
        assert!(
            noop["params"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("input content changed")),
            "{noop}"
        );
        assert!(noop["params"]["pdfUri"].is_null());
    }
    server.request(5, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn stale_pdf_history_uses_disk_contents_for_initially_unsaved_dynamic_inputs() {
    for save_during in [false, true] {
        let script = "if [ ! -f \"$1/main.pdf\" ] || [ -f \"$1/rebuild\" ]; then\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nif [ -f \"$1/rebuild\" ]; then printf 'New contents\\n' >> \"$1/main.pdf\"; fi\nprintf 'INPUT %s\\nINPUT %s/child.ltx\\n' \"$2\" \"${2%/*}\" > \"$1/main.fls\"\ntouch \"$1/ready\"\nwhile [ ! -f \"$1/release\" ]; do sleep 0.02; done\nfi\n";
        let (mut server, directory, main) = fake_build_server(script, false);
        let child = Url::from_file_path(directory.path().join("child.ltx")).unwrap();
        std::fs::write(child.to_file_path().unwrap(), "Old.\n").unwrap();
        let modified = std::fs::metadata(child.to_file_path().unwrap())
            .unwrap()
            .modified()
            .unwrap();
        let source = "\\documentclass{article}\n\\def\\chosen{child.ltx}\\input{\\chosen}\n";
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        server.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":child,"languageId":"latex","version":1,"text":"New.\n"}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let output = directory.path().join("build/latex-lsp");
        wait_for_file(&output.join("ready"));
        if save_during {
            std::fs::write(child.to_file_path().unwrap(), "New.\n").unwrap();
            std::fs::File::open(child.to_file_path().unwrap())
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
        }
        std::fs::write(output.join("release"), "").unwrap();
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["stale"], true, "{first}");
        std::fs::write(child.to_file_path().unwrap(), "New.\n").unwrap();
        std::fs::File::open(child.to_file_path().unwrap())
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        for id in [3, 4] {
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(noop["params"]["status"], "error", "{noop}");
            assert!(
                noop["params"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("input content changed")),
                "{noop}"
            );
            assert!(noop["params"]["pdfUri"].is_null());
        }
        std::fs::write(directory.path().join("build/latex-lsp/rebuild"), "").unwrap();
        server.request(
            5,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        server.request(6, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn overwritten_pdfs_and_failed_driver_outputs_cannot_reuse_successful_synctex_history() {
    for failed_driver in [false, true] {
        let (mut server, directory, main) = fake_synctex_server("touch queried\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n");
        let root = directory.path();
        let compiler = root.join("compiler.sh");
        std::fs::write(&compiler, "if [ -f \"$1/noop\" ]; then exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nif [ -f \"$1/fail\" ]; then printf '%%PDF-1.4\\nREPLACED\\n' > \"$1/main.pdf\"; exit 1; fi\n").unwrap();
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        let params = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
        let original = server.request(3, "workspace/executeCommand", params.clone());
        assert_eq!(original["locations"][0]["page"], 1, "{original}");
        let output = root.join("build/latex-lsp");
        let pdf = output.join("main.pdf");
        let modified = std::fs::metadata(&pdf).unwrap().modified().unwrap();
        std::fs::File::open(&pdf)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified + Duration::from_secs(1)))
            .unwrap();
        // Metadata-only changes leave the PDF's contents and mapping valid.
        let touched = server.request(4, "workspace/executeCommand", params.clone());
        assert_eq!(touched["locations"][0]["page"], 1, "{touched}");
        std::fs::remove_file(root.join("queried")).unwrap();
        if failed_driver {
            std::fs::write(output.join("fail"), "").unwrap();
            server.request(
                5,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let failed = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(failed["params"]["status"], "error", "{failed}");
            std::fs::remove_file(output.join("fail")).unwrap();
        } else {
            std::fs::write(&pdf, b"%PDF-1.4\nREPLACED\n").unwrap();
        }
        // The overwritten artifact also retains its original length and mtime.
        std::fs::File::open(&pdf)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        server.send(json!({"jsonrpc":"2.0","id":6,"method":"workspace/executeCommand","params":params.clone()}));
        let rejected = server.until(|message| message["id"] == 6);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .is_some_and(|text| text.contains("Build artifacts changed")),
            "{failed_driver}: {rejected}"
        );
        assert!(!root.join("queried").exists());
        std::fs::write(output.join("noop"), "").unwrap();
        for id in [7, 8] {
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(noop["params"]["status"], "error", "{noop}");
            assert!(
                noop["params"]["message"]
                    .as_str()
                    .is_some_and(|text| text.contains("build artifacts changed")),
                "{noop}"
            );
            assert!(noop["params"]["pdfUri"].is_null());
        }
        std::fs::remove_file(output.join("noop")).unwrap();
        server.request(
            9,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        let restored = server.request(10, "workspace/executeCommand", params);
        assert_eq!(restored["locations"][0]["page"], 1, "{restored}");
        server.request(11, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn successful_noop_builds_keep_the_actual_pdf_input_scope_when_recorder_output_changes() {
    let script = "if [ ! -f \"$1/main.pdf\" ]; then\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\nINPUT %s/dynamic.ltx\\n' \"$2\" \"${2%/*}\" > \"$1/main.fls\"\nelif [ -f \"$1/rebuild\" ]; then\nprintf '%%PDF-1.4\\nRegenerated\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nelif [ -f \"$1/omit\" ]; then\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nfi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let dynamic = directory.path().join("dynamic.ltx");
    std::fs::write(&dynamic, "Old.\n").unwrap();
    let modified = std::fs::metadata(&dynamic).unwrap().modified().unwrap();
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    std::fs::write(directory.path().join("build/latex-lsp/omit"), "").unwrap();
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let noop = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(noop["params"]["status"], "success", "{noop}");
    std::fs::write(&dynamic, "New.\n").unwrap();
    std::fs::File::open(&dynamic)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    for id in [4, 5] {
        server.request(
            id,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rejected = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rejected["params"]["status"], "error", "{rejected}");
        assert!(
            rejected["params"]["message"]
                .as_str()
                .is_some_and(|text| text.contains("input content changed")),
            "{rejected}"
        );
        assert!(rejected["params"]["pdfUri"].is_null());
    }
    let output = directory.path().join("build/latex-lsp");
    std::fs::write(output.join("rebuild"), "").unwrap();
    server.request(
        6,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
    std::fs::remove_file(output.join("rebuild")).unwrap();
    std::fs::write(&dynamic, "Old.\n").unwrap();
    std::fs::File::open(&dynamic)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    server.request(
        7,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let unused = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(unused["params"]["status"], "success", "{unused}");
    server.request(8, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn replaced_synctex_data_cannot_be_adopted_by_successful_noop_builds() {
    for compressed in [false, true] {
        for scenario in ["overwrite", "failed", "delete", "alternate"] {
            let (mut server, directory, main) = fake_synctex_server("touch queried\nif [ \"$1\" = edit ]; then\nprintf 'SyncTeX result begin\\nInput:%s/main.tex\\nLine:2\\nColumn:0\\nSyncTeX result end\\n' \"$PWD\"\nelse\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\nfi\n");
            let root = directory.path();
            let extension = if compressed { "synctex.gz" } else { "synctex" };
            let other = if compressed { "synctex" } else { "synctex.gz" };
            let compiler = root.join("compiler.sh");
            std::fs::write(&compiler, format!("if [ -f \"$1/noop\" ]; then exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nrm -f \"$1/main.synctex\" \"$1/main.synctex.gz\"\nprintf 'MAPS-OLD\\n' > \"$1/main.{extension}\"\nif [ -f \"$1/fail\" ]; then printf 'MAPS-NEW\\n' > \"$1/main.{extension}\"; exit 1; fi\n")).unwrap();
            server.notify(
                "workspace/didChangeConfiguration",
                json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
            );
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let first = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(first["params"]["status"], "success", "{first}");
            let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
            let inverse = json!({"command":"latex.inverseSearch","arguments":[{"uri":main,"page":1,"x":10,"y":20}]});
            let original = server.request(3, "workspace/executeCommand", forward.clone());
            assert_eq!(original["locations"][0]["page"], 1, "{original}");
            let output = root.join("build/latex-lsp");
            let mapping = output.join(format!("main.{extension}"));
            let modified = std::fs::metadata(&mapping).unwrap().modified().unwrap();
            std::fs::File::open(&mapping)
                .unwrap()
                .set_times(
                    std::fs::FileTimes::new().set_modified(modified + Duration::from_secs(1)),
                )
                .unwrap();
            let touched = server.request(4, "workspace/executeCommand", forward.clone());
            assert_eq!(touched["locations"][0]["page"], 1, "{touched}");
            std::fs::remove_file(root.join("queried")).unwrap();
            match scenario {
                "failed" => {
                    std::fs::write(output.join("fail"), "").unwrap();
                    server.request(
                        5,
                        "workspace/executeCommand",
                        json!({"command":"latex.build","arguments":[main]}),
                    );
                    let failed = server.until(|message| message["method"] == "latex/buildFinished");
                    assert_eq!(failed["params"]["status"], "error", "{failed}");
                    std::fs::remove_file(output.join("fail")).unwrap();
                }
                "overwrite" => std::fs::write(&mapping, "MAPS-NEW\n").unwrap(),
                "delete" => std::fs::remove_file(&mapping).unwrap(),
                "alternate" => {
                    std::fs::write(output.join(format!("main.{other}")), "OTHER-MAP\n").unwrap()
                }
                _ => unreachable!(),
            }
            if mapping.exists() {
                std::fs::File::open(&mapping)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(modified))
                    .unwrap();
            }
            let mut id = 6;
            for params in [&forward, &inverse] {
                server.send(json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":params}));
                let rejected = server.until(|message| message["id"] == id);
                assert!(
                    rejected["error"]["message"]
                        .as_str()
                        .is_some_and(|text| text.contains("Build artifacts changed")),
                    "{compressed}/{scenario}: {rejected}"
                );
                id += 1;
            }
            assert!(!root.join("queried").exists());
            std::fs::write(output.join("noop"), "").unwrap();
            for _ in 0..2 {
                server.request(
                    id,
                    "workspace/executeCommand",
                    json!({"command":"latex.build","arguments":[main]}),
                );
                id += 1;
                let noop = server.until(|message| message["method"] == "latex/buildFinished");
                assert_eq!(
                    noop["params"]["status"], "success",
                    "{compressed}/{scenario}: {noop}"
                );
                assert!(noop["params"]["pdfUri"].as_str().is_some());
                server.send(json!({"jsonrpc":"2.0","id":id,"method":"workspace/executeCommand","params":forward}));
                let rejected = server.until(|message| message["id"] == id);
                assert!(
                    rejected["error"]["message"]
                        .as_str()
                        .is_some_and(|text| text.contains("Build artifacts changed")),
                    "{compressed}/{scenario}: {rejected}"
                );
                id += 1;
            }
            assert!(!root.join("queried").exists());
            std::fs::remove_file(output.join("noop")).unwrap();
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            id += 1;
            let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
            for params in [forward, inverse] {
                let restored = server.request(id, "workspace/executeCommand", params);
                assert!(
                    !restored["locations"].as_array().unwrap().is_empty(),
                    "{restored}"
                );
                id += 1;
            }
            server.request(id, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn equal_size_and_timestamp_artifact_replacements_during_synctex_queries_are_rejected() {
    for extension in ["pdf", "synctex", "synctex.gz"] {
        for inverse in [false, true] {
            let (mut server, directory, main) = fake_synctex_server("touch ready\nwhile [ ! -f release ]; do sleep 0.02; done\nif [ \"$1\" = edit ]; then\nprintf 'SyncTeX result begin\\nInput:%s/main.tex\\nLine:2\\nColumn:0\\nSyncTeX result end\\n' \"$PWD\"\nelse\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\nfi\n");
            let root = directory.path();
            let output = root.join("build/latex-lsp");
            if extension == "synctex.gz" {
                std::fs::rename(output.join("main.synctex"), output.join("main.synctex.gz"))
                    .unwrap();
            }
            let artifact = output.join(format!("main.{extension}"));
            let modified = std::fs::metadata(&artifact).unwrap().modified().unwrap();
            let params = if inverse {
                json!({"command":"latex.inverseSearch","arguments":[{"uri":main,"page":1,"x":10,"y":20}]})
            } else {
                json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]})
            };
            server.send(
                json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":params}),
            );
            wait_for_file(&root.join("ready"));
            let mut replacement = std::fs::read(&artifact).unwrap();
            *replacement.last_mut().unwrap() ^= 1;
            std::fs::write(&artifact, replacement).unwrap();
            std::fs::File::open(&artifact)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
            std::fs::write(root.join("release"), "").unwrap();
            let rejected = server.until(|message| message["id"] == 2);
            assert!(
                rejected["error"]["message"].as_str().is_some_and(
                    |text| text.contains("Build artifacts changed during the SyncTeX query")
                ),
                "{extension}/{inverse}: {rejected}"
            );
            server.request(3, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn recorded_external_images_keep_pdf_history_and_remain_binary() {
    for dynamic in [false, true] {
        let external = tempfile::tempdir().unwrap();
        let image = external.path().join("plot.png");
        std::fs::write(&image, "\\label{binary-source-old}\n").unwrap();
        let modified = std::fs::metadata(&image).unwrap().modified().unwrap();
        let (mut server, directory, main) = fake_synctex_server("touch queried\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n");
        let source = if dynamic {
            format!("\\documentclass{{article}}\n\\def\\selected{{{}}}\\includegraphics{{\\selected}}\n", image.display())
        } else {
            format!(
                "\\documentclass{{article}}\n\\includegraphics{{{}}}\n",
                image.display()
            )
        };
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        let compiler = directory.path().join("compiler.sh");
        std::fs::write(&compiler, format!("if [ -f \"$1/noop\" ]; then exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\nINPUT {}\\n' \"$2\" > \"$1/main.fls\"\n", image.display())).unwrap();
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["stale"], false, "{first}");
        assert_eq!(
            first["params"]["dependenciesCount"], 2,
            "{dynamic}: {first}"
        );
        let symbols = server.request(3, "workspace/symbol", json!({"query":"binary-source"}));
        assert!(symbols.as_array().unwrap().is_empty(), "{symbols}");
        let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
        let valid = server.request(4, "workspace/executeCommand", forward.clone());
        assert_eq!(valid["locations"][0]["page"], 1, "{valid}");
        std::fs::remove_file(directory.path().join("queried")).unwrap();
        std::fs::write(&image, "\\label{binary-source-new}\n").unwrap();
        std::fs::File::open(&image)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        server.send(
            json!({"jsonrpc":"2.0","id":5,"method":"workspace/executeCommand","params":forward}),
        );
        let rejected = server.until(|message| message["id"] == 5);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Input contents changed")),
            "{dynamic}: {rejected}"
        );
        assert!(!directory.path().join("queried").exists());
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("noop"), "").unwrap();
        for id in [6, 7] {
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(noop["params"]["status"], "error", "{dynamic}: {noop}");
            assert!(noop["params"]["pdfUri"].is_null());
            assert!(
                noop["params"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("input content changed")),
                "{noop}"
            );
        }
        std::fs::remove_file(output.join("noop")).unwrap();
        server.request(
            8,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        let restored = server.request(9, "workspace/executeCommand", forward);
        assert_eq!(restored["locations"][0]["page"], 1, "{restored}");
        server.request(10, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn changed_recorded_external_images_invalidate_active_builds_with_or_without_watcher_events() {
    for notify in [false, true] {
        let external = tempfile::tempdir().unwrap();
        let image = external.path().join("plot.png");
        std::fs::write(&image, b"OLD\n").unwrap();
        let modified = std::fs::metadata(&image).unwrap().modified().unwrap();
        let script = format!("printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\nINPUT {}\\n' \"$2\" > \"$1/main.fls\"\nprintf '%s:2: LaTeX Warning: old image.\\n' \"$2\" > \"$1/main.log\"\nif [ -f \"$1/hold\" ]; then touch \"$1/ready\"; while [ ! -f \"$1/release\" ]; do sleep 0.02; done; fi\n", image.display());
        let (mut server, directory, main) = fake_build_server_capabilities(
            &script,
            false,
            json!({"textDocument":{"diagnostic":{}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["dependenciesCount"], 2, "{first}");
        let old = server.request(
            10,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main}}),
        );
        assert!(
            old["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["source"] == "latex-build"),
            "{old}"
        );
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("hold"), "").unwrap();
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        wait_for_file(&output.join("ready"));
        std::fs::write(&image, b"NEW\n").unwrap();
        std::fs::File::open(&image)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        if notify {
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":Url::from_file_path(&image).unwrap(),"type":2}]}),
            );
        } else {
            std::fs::write(output.join("release"), "").unwrap();
        }
        let result = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            result["params"]["status"],
            if notify { "cancelled" } else { "success" },
            "{result}"
        );
        if !notify {
            assert_eq!(result["params"]["stale"], true, "{result}");
        }
        let cleared = server.request(
            11,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
        );
        assert_eq!(cleared["kind"], "full", "{cleared}");
        assert_eq!(cleared["items"], json!([]), "{cleared}");
        std::fs::write(output.join("release"), "").unwrap();
        std::fs::remove_file(output.join("hold")).unwrap();
        server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        assert!(
            rebuilt["params"]["warningsCount"].as_u64().unwrap() > 0,
            "{rebuilt}"
        );
        server.request(5, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn content_updates_to_pdf_and_recorder_are_accepted_with_preserved_output_metadata() {
    let script = "if [ -f \"$1/noop\" ]; then exit 0; fi\nif [ -f \"$1/new\" ]; then chosen=other; content=NEW; else chosen=first; content=OLD; fi\nprintf '%%PDF-1.4\\n%s\\n' \"$content\" > \"$1/main.pdf\"\nprintf 'INPUT %s\\nINPUT %s/.inputs/%s.ltx\\n' \"$2\" \"${2%/*}\" \"$chosen\" > \"$1/main.fls\"\nif [ -f \"$1/clock\" ]; then touch -r \"$1/clock\" \"$1/main.pdf\" \"$1/main.fls\"; fi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let inputs = directory.path().join(".inputs");
    std::fs::create_dir(&inputs).unwrap();
    std::fs::write(inputs.join("first.ltx"), "\\label{first-child}\n").unwrap();
    std::fs::write(inputs.join("other.ltx"), "\\label{other-child}\n").unwrap();
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    let output = directory.path().join("build/latex-lsp");
    let future = std::time::SystemTime::now() + Duration::from_secs(120);
    std::fs::write(output.join("clock"), "").unwrap();
    let paths = [output.join("main.pdf"), output.join("main.fls")];
    let lengths = paths
        .each_ref()
        .map(|path| std::fs::metadata(path).unwrap().len());
    for path in paths.iter().chain([output.join("clock")].iter()) {
        std::fs::File::open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(future))
            .unwrap();
    }
    std::fs::write(output.join("new"), "").unwrap();
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
    for (path, length) in paths.iter().zip(lengths) {
        let metadata = std::fs::metadata(path).unwrap();
        assert_eq!(metadata.len(), length);
        assert_eq!(metadata.modified().unwrap(), future);
    }
    assert!(std::fs::read_to_string(&paths[0]).unwrap().contains("NEW"));
    assert!(std::fs::read_to_string(&paths[1])
        .unwrap()
        .contains("other.ltx"));
    assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
    assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
    let symbols = server.request(4, "workspace/symbol", json!({"query":"other-child"}));
    assert_eq!(symbols.as_array().unwrap().len(), 1, "{symbols}");
    assert_eq!(
        symbols[0]["location"]["uri"],
        json!(Url::from_file_path(inputs.join("other.ltx")).unwrap())
    );
    std::fs::write(output.join("noop"), "").unwrap();
    server.request(
        5,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let noop = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(noop["params"]["status"], "success", "{noop}");
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn bibliography_log_content_updates_are_analyzed_with_preserved_output_metadata() {
    let script = "if [ -f \"$1/noop\" ]; then exit 0; fi\nif [ -f \"$1/new\" ]; then chosen=new; else chosen=old; fi\nprintf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'Database file #1: .inputs/%srefs.bib\\nWarning--empty author in %s\\n' \"$chosen\" \"$chosen\" > \"$1/main.blg\"\nif [ -f \"$1/clock\" ]; then touch -r \"$1/clock\" \"$1/main.blg\"; fi\n";
    let (mut server, directory, main) = fake_build_server(script, false);
    let inputs = directory.path().join(".inputs");
    std::fs::create_dir(&inputs).unwrap();
    for name in ["old", "new"] {
        std::fs::write(
            inputs.join(format!("{name}refs.bib")),
            format!("@book{{{name},title={{Title}}}}\n"),
        )
        .unwrap();
    }
    server.request(
        2,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let first = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(first["params"]["status"], "success", "{first}");
    assert_eq!(first["params"]["warningsCount"], 1, "{first}");
    let output = directory.path().join("build/latex-lsp");
    let log = output.join("main.blg");
    let length = std::fs::metadata(&log).unwrap().len();
    let future = std::time::SystemTime::now() + Duration::from_secs(120);
    std::fs::write(output.join("clock"), "").unwrap();
    for path in [&log, &output.join("clock")] {
        std::fs::File::open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(future))
            .unwrap();
    }
    std::fs::write(output.join("new"), "").unwrap();
    server.request(
        3,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
    let metadata = std::fs::metadata(&log).unwrap();
    assert_eq!(metadata.len(), length);
    assert_eq!(metadata.modified().unwrap(), future);
    assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
    assert_eq!(rebuilt["params"]["warningsCount"], 1, "{rebuilt}");
    let bibliography = Url::from_file_path(inputs.join("newrefs.bib")).unwrap();
    let diagnostics = server.request(
        4,
        "textDocument/diagnostic",
        json!({"textDocument":{"uri":bibliography}}),
    );
    assert!(
        diagnostics["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["source"] == "latex-build"
                && item["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("author in new"))),
        "{diagnostics}"
    );
    let symbols = server.request(5, "workspace/symbol", json!({"query":"new"}));
    assert!(
        symbols
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["location"]["uri"] == json!(bibliography)),
        "{symbols}"
    );
    std::fs::write(output.join("noop"), "").unwrap();
    server.request(
        6,
        "workspace/executeCommand",
        json!({"command":"latex.build","arguments":[main]}),
    );
    let noop = server.until(|message| message["method"] == "latex/buildFinished");
    assert_eq!(noop["params"]["status"], "success", "{noop}");
    assert_eq!(noop["params"]["warningsCount"], 0, "{noop}");
    server.request(7, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}

#[test]
#[cfg(unix)]
fn recorded_data_inputs_keep_pdf_history_without_latex_indexing() {
    for extension in ["csv", "TSV", "dat", "json", "xml", "yaml", "yml", "txt"] {
        let external = tempfile::tempdir().unwrap();
        let data = external.path().join(format!("table.{extension}"));
        std::fs::write(&data, "\\label{data-source-old}\n").unwrap();
        let modified = std::fs::metadata(&data).unwrap().modified().unwrap();
        let (mut server, directory, main) = fake_synctex_server("touch queried\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n");
        let source = format!(
            "\\documentclass{{article}}\n\\newread\\dataset\\openin\\dataset={}\\relax\n",
            data.display()
        );
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        let compiler = directory.path().join("compiler.sh");
        std::fs::write(&compiler, format!("if [ -f \"$1/noop\" ]; then exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\nINPUT {}\\n' \"$2\" > \"$1/main.fls\"\n", data.display())).unwrap();
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["stale"], false, "{first}");
        assert_eq!(
            first["params"]["dependenciesCount"], 2,
            "{extension}: {first}"
        );
        let symbols = server.request(3, "workspace/symbol", json!({"query":"data-source"}));
        assert!(symbols.as_array().unwrap().is_empty(), "{symbols}");
        let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
        let valid = server.request(4, "workspace/executeCommand", forward.clone());
        assert_eq!(valid["locations"][0]["page"], 1, "{valid}");
        std::fs::remove_file(directory.path().join("queried")).unwrap();
        std::fs::write(&data, "\\label{data-source-new}\n").unwrap();
        std::fs::File::open(&data)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        server.send(
            json!({"jsonrpc":"2.0","id":5,"method":"workspace/executeCommand","params":forward}),
        );
        let rejected = server.until(|message| message["id"] == 5);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Input contents changed")),
            "{extension}: {rejected}"
        );
        assert!(!directory.path().join("queried").exists());
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("noop"), "").unwrap();
        for id in [6, 7] {
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(noop["params"]["status"], "error", "{extension}: {noop}");
            assert!(noop["params"]["pdfUri"].is_null());
            assert!(
                noop["params"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("input content changed")),
                "{noop}"
            );
        }
        std::fs::remove_file(output.join("noop")).unwrap();
        server.request(
            8,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        let restored = server.request(9, "workspace/executeCommand", forward);
        assert_eq!(restored["locations"][0]["page"], 1, "{restored}");
        server.request(10, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn changed_recorded_data_invalidates_active_builds_with_or_without_watcher_events() {
    for notify in [false, true] {
        let external = tempfile::tempdir().unwrap();
        let data = external.path().join("table.CSV");
        std::fs::write(&data, b"OLD\n").unwrap();
        let modified = std::fs::metadata(&data).unwrap().modified().unwrap();
        let script = format!("printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\nINPUT {}\\n' \"$2\" > \"$1/main.fls\"\nprintf '%s:2: LaTeX Warning: old data.\\n' \"$2\" > \"$1/main.log\"\nif [ -f \"$1/hold\" ]; then touch \"$1/ready\"; while [ ! -f \"$1/release\" ]; do sleep 0.02; done; fi\n", data.display());
        let (mut server, directory, main) = fake_build_server_capabilities(
            &script,
            false,
            json!({"textDocument":{"diagnostic":{}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["dependenciesCount"], 2, "{first}");
        let old = server.request(
            10,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main}}),
        );
        assert!(
            old["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["source"] == "latex-build"),
            "{old}"
        );
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("hold"), "").unwrap();
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        wait_for_file(&output.join("ready"));
        std::fs::write(&data, b"NEW\n").unwrap();
        std::fs::File::open(&data)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        if notify {
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":Url::from_file_path(&data).unwrap(),"type":2}]}),
            );
        } else {
            std::fs::write(output.join("release"), "").unwrap();
        }
        let result = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            result["params"]["status"],
            if notify { "cancelled" } else { "success" },
            "{result}"
        );
        if !notify {
            assert_eq!(result["params"]["stale"], true, "{result}");
        }
        let cleared = server.request(
            11,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
        );
        assert_eq!(cleared["kind"], "full", "{cleared}");
        assert_eq!(cleared["items"], json!([]), "{cleared}");
        std::fs::write(output.join("release"), "").unwrap();
        std::fs::remove_file(output.join("hold")).unwrap();
        server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        assert!(
            rebuilt["params"]["warningsCount"].as_u64().unwrap() > 0,
            "{rebuilt}"
        );
        server.request(5, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn recorded_binary_aliases_keep_pdf_history_and_do_not_become_latex_sources() {
    for (extension, target) in [
        ("png", "blob"),
        ("png", "tex"),
        ("CSV", "payload"),
        ("csv", "tex"),
    ] {
        let external = tempfile::tempdir().unwrap();
        let data = external.path().join(format!("table.{target}"));
        let alias = external.path().join(format!("alias.{extension}"));
        std::os::unix::fs::symlink(&data, &alias).unwrap();
        std::fs::write(&data, "\\label{aliased-source-old}\n").unwrap();
        let modified = std::fs::metadata(&data).unwrap().modified().unwrap();
        let (mut server, directory, main) = fake_synctex_server("touch queried\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n");
        let source = if extension.eq_ignore_ascii_case("png") {
            format!("\\documentclass{{article}}\n\\def\\selected{{{}}}\\includegraphics{{\\selected}}\n", alias.display())
        } else {
            format!(
                "\\documentclass{{article}}\n\\newread\\dataset\\openin\\dataset={}\\relax\n",
                alias.display()
            )
        };
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        let compiler = directory.path().join("compiler.sh");
        std::fs::write(&compiler, format!("if [ -f \"$1/noop\" ]; then exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\nINPUT {}\\n' \"$2\" > \"$1/main.fls\"\n", alias.display())).unwrap();
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["stale"], false, "{first}");
        assert_eq!(
            first["params"]["dependenciesCount"], 2,
            "{extension}: {first}"
        );
        let symbols = server.request(3, "workspace/symbol", json!({"query":"aliased-source"}));
        assert!(symbols.as_array().unwrap().is_empty(), "{symbols}");
        let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
        let valid = server.request(4, "workspace/executeCommand", forward.clone());
        assert_eq!(valid["locations"][0]["page"], 1, "{valid}");
        std::fs::remove_file(directory.path().join("queried")).unwrap();
        std::fs::write(&data, "\\label{aliased-source-new}\n").unwrap();
        std::fs::File::open(&data)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        server.send(
            json!({"jsonrpc":"2.0","id":5,"method":"workspace/executeCommand","params":forward}),
        );
        let rejected = server.until(|message| message["id"] == 5);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Input contents changed")),
            "{extension}: {rejected}"
        );
        assert!(!directory.path().join("queried").exists());
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("noop"), "").unwrap();
        for id in [6, 7] {
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(noop["params"]["status"], "error", "{extension}: {noop}");
            assert!(noop["params"]["pdfUri"].is_null());
            assert!(
                noop["params"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("input content changed")),
                "{noop}"
            );
        }
        std::fs::remove_file(output.join("noop")).unwrap();
        server.request(
            8,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        let restored = server.request(9, "workspace/executeCommand", forward);
        assert_eq!(restored["locations"][0]["page"], 1, "{restored}");
        server.request(10, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn changed_recorded_binary_aliases_invalidate_active_builds_with_or_without_watcher_events() {
    for notify in [false, true] {
        let external = tempfile::tempdir().unwrap();
        let data = external.path().join("table.tex");
        let alias = external.path().join("alias.CSV");
        std::os::unix::fs::symlink(&data, &alias).unwrap();
        std::fs::write(&data, b"OLD\n").unwrap();
        let modified = std::fs::metadata(&data).unwrap().modified().unwrap();
        let script = format!("printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\nINPUT {}\\n' \"$2\" > \"$1/main.fls\"\nprintf '%s:2: LaTeX Warning: old data.\\n' \"$2\" > \"$1/main.log\"\nif [ -f \"$1/hold\" ]; then touch \"$1/ready\"; while [ ! -f \"$1/release\" ]; do sleep 0.02; done; fi\n", alias.display());
        let (mut server, directory, main) = fake_build_server_capabilities(
            &script,
            false,
            json!({"textDocument":{"diagnostic":{}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["dependenciesCount"], 2, "{first}");
        let old = server.request(
            10,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main}}),
        );
        assert!(
            old["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["source"] == "latex-build"),
            "{old}"
        );
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("hold"), "").unwrap();
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        wait_for_file(&output.join("ready"));
        std::fs::write(&data, b"NEW\n").unwrap();
        std::fs::File::open(&data)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        if notify {
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":Url::from_file_path(&data).unwrap(),"type":2}]}),
            );
        } else {
            std::fs::write(output.join("release"), "").unwrap();
        }
        let result = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            result["params"]["status"],
            if notify { "cancelled" } else { "success" },
            "{result}"
        );
        if !notify {
            assert_eq!(result["params"]["stale"], true, "{result}");
        }
        let cleared = server.request(
            11,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
        );
        assert_eq!(cleared["kind"], "full", "{cleared}");
        assert_eq!(cleared["items"], json!([]), "{cleared}");
        std::fs::write(output.join("release"), "").unwrap();
        std::fs::remove_file(output.join("hold")).unwrap();
        server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        assert!(
            rebuilt["params"]["warningsCount"].as_u64().unwrap() > 0,
            "{rebuilt}"
        );
        server.request(5, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn recorded_alias_target_changes_reject_old_pdfs_and_allow_restored_or_rebuilt_bindings() {
    for (extension, remove, rewrite_recorder) in [
        ("png", false, false),
        ("csv", false, false),
        ("png", true, false),
        ("csv", true, false),
        ("png", false, true),
        ("csv", false, true),
    ] {
        let external = tempfile::tempdir().unwrap();
        let data = external.path().join(format!("first.{extension}"));
        let other = external.path().join(format!("second.{extension}"));
        let alias = external.path().join(format!("alias.{extension}"));
        std::fs::write(&other, "\\label{data-source-new}\n").unwrap();
        std::os::unix::fs::symlink(&data, &alias).unwrap();
        std::fs::write(&data, "\\label{data-source-old}\n").unwrap();
        let (mut server, directory, main) = fake_synctex_server("touch queried\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n");
        let source = format!(
            "\\documentclass{{article}}\n\\newread\\dataset\\openin\\dataset={}\\relax\n",
            alias.display()
        );
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        let compiler = directory.path().join("compiler.sh");
        let noop_recorder = if rewrite_recorder {
            "printf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"; "
        } else {
            ""
        };
        std::fs::write(&compiler, format!("if [ -f \"$1/noop\" ]; then {noop_recorder}exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\nINPUT {}\\n' \"$2\" > \"$1/main.fls\"\n", alias.display())).unwrap();
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["stale"], false, "{first}");
        assert_eq!(
            first["params"]["dependenciesCount"], 2,
            "{extension}: {first}"
        );
        let symbols = server.request(3, "workspace/symbol", json!({"query":"data-source"}));
        assert!(symbols.as_array().unwrap().is_empty(), "{symbols}");
        let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
        let valid = server.request(4, "workspace/executeCommand", forward.clone());
        assert_eq!(valid["locations"][0]["page"], 1, "{valid}");
        std::fs::remove_file(directory.path().join("queried")).unwrap();
        std::fs::remove_file(&alias).unwrap();
        if !remove {
            std::os::unix::fs::symlink(&other, &alias).unwrap();
        }
        server.send(
            json!({"jsonrpc":"2.0","id":5,"method":"workspace/executeCommand","params":forward}),
        );
        let rejected = server.until(|message| message["id"] == 5);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("File conditions changed")),
            "{extension}: {rejected}"
        );
        assert!(!directory.path().join("queried").exists());
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("noop"), "").unwrap();
        for id in [6, 7] {
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(noop["params"]["status"], "error", "{extension}: {noop}");
            assert!(noop["params"]["pdfUri"].is_null());
            assert!(
                noop["params"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("file condition changed")),
                "{noop}"
            );
        }
        if alias.exists() {
            std::fs::remove_file(&alias).unwrap();
        }
        std::os::unix::fs::symlink(&data, &alias).unwrap();
        server.request(
            81,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let restored_noop = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            restored_noop["params"]["status"], "success",
            "{restored_noop}"
        );
        let restored = server.request(80, "workspace/executeCommand", forward.clone());
        assert_eq!(restored["locations"][0]["page"], 1, "{restored}");
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&other, &alias).unwrap();
        std::fs::remove_file(output.join("noop")).unwrap();
        server.request(
            8,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        let restored = server.request(9, "workspace/executeCommand", forward);
        assert_eq!(restored["locations"][0]["page"], 1, "{restored}");
        server.request(10, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn changed_alias_targets_invalidate_active_builds_with_or_without_watcher_events() {
    for notify in [false, true] {
        let external = tempfile::tempdir().unwrap();
        let data = external.path().join("first.CSV");
        let other = external.path().join("second.CSV");
        let alias = external.path().join("alias.CSV");
        std::fs::write(&other, b"NEW\n").unwrap();
        std::os::unix::fs::symlink(&data, &alias).unwrap();
        std::fs::write(&data, b"OLD\n").unwrap();
        let script = format!("printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\nINPUT {}\\n' \"$2\" > \"$1/main.fls\"\nprintf '%s:2: LaTeX Warning: old data.\\n' \"$2\" > \"$1/main.log\"\nif [ -f \"$1/hold\" ]; then touch \"$1/ready\"; while [ ! -f \"$1/release\" ]; do sleep 0.02; done; fi\n", alias.display());
        let (mut server, directory, main) = fake_build_server_capabilities(
            &script,
            false,
            json!({"textDocument":{"diagnostic":{}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["dependenciesCount"], 2, "{first}");
        let old = server.request(
            10,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main}}),
        );
        assert!(
            old["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["source"] == "latex-build"),
            "{old}"
        );
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("hold"), "").unwrap();
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        wait_for_file(&output.join("ready"));
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&other, &alias).unwrap();
        if notify {
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":Url::from_file_path(&alias).unwrap(),"type":2}]}),
            );
        } else {
            std::fs::write(output.join("release"), "").unwrap();
        }
        let result = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(
            result["params"]["status"],
            if notify { "cancelled" } else { "success" },
            "{result}"
        );
        if !notify {
            assert_eq!(result["params"]["stale"], true, "{result}");
        }
        let cleared = server.request(
            11,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
        );
        assert_eq!(cleared["kind"], "full", "{cleared}");
        assert_eq!(cleared["items"], json!([]), "{cleared}");
        std::fs::write(output.join("release"), "").unwrap();
        std::fs::remove_file(output.join("hold")).unwrap();
        server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        assert!(
            rebuilt["params"]["warningsCount"].as_u64().unwrap() > 0,
            "{rebuilt}"
        );
        server.request(5, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn new_pdf_binding_histories_drop_unused_aliases_and_preserve_uncertain_provenance() {
    for omit in [true, false] {
        let external = tempfile::tempdir().unwrap();
        let first = external.path().join("first.csv");
        let second = external.path().join("second.csv");
        let alias = external.path().join("alias.csv");
        std::fs::write(&first, "OLD\n").unwrap();
        std::fs::write(&second, "NEW\n").unwrap();
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let (mut server, directory, main) = fake_synctex_server(
            "printf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n",
        );
        let compiler = directory.path().join("compiler.sh");
        std::fs::write(&compiler, format!("if [ -f \"$1/noop\" ]; then exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nif [ ! -f \"$1/omit\" ]; then printf 'INPUT {}\\n' >> \"$1/main.fls\"; fi\nif [ -f \"$1/hold\" ]; then touch \"$1/ready\"; while [ ! -f \"$1/release\" ]; do sleep 0.02; done; fi\n", alias.display())).unwrap();
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let original = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(original["params"]["status"], "success", "{original}");
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join(if omit { "omit" } else { "hold" }), "").unwrap();
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        if !omit {
            wait_for_file(&output.join("ready"));
        }
        let completed = if omit {
            server.until(|message| message["method"] == "latex/buildFinished")
        } else {
            std::fs::remove_file(&alias).unwrap();
            std::os::unix::fs::symlink(&second, &alias).unwrap();
            std::fs::write(output.join("release"), "").unwrap();
            server.until(|message| message["method"] == "latex/buildFinished")
        };
        assert_eq!(completed["params"]["status"], "success", "{completed}");
        assert_eq!(completed["params"]["stale"], !omit, "{completed}");
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(if omit { &second } else { &first }, &alias).unwrap();
        let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
        server.send(
            json!({"jsonrpc":"2.0","id":4,"method":"workspace/executeCommand","params":forward}),
        );
        let checked = server.until(|message| message["id"] == 4);
        if omit {
            assert_eq!(checked["result"]["locations"][0]["page"], 1, "{checked}");
        } else {
            assert!(
                checked["error"]["message"]
                    .as_str()
                    .is_some_and(|text| text.contains("File conditions changed")),
                "{checked}"
            );
            std::fs::remove_file(output.join("hold")).unwrap();
        }
        std::fs::write(output.join("noop"), "").unwrap();
        for id in [5, 6] {
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|message| message["method"] == "latex/buildFinished");
            assert_eq!(
                noop["params"]["status"],
                if omit { "success" } else { "error" },
                "{noop}"
            );
        }
        std::fs::remove_file(output.join("noop")).unwrap();
        server.request(
            7,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|message| message["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        let valid = server.request(8, "workspace/executeCommand", forward);
        assert_eq!(valid["locations"][0]["page"], 1, "{valid}");
        server.request(9, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn bibliography_alias_history_survives_noop_log_updates_and_recovers_after_restoration() {
    for (extension, remove, drop_log) in [
        ("bib", false, false),
        ("bst", false, false),
        ("bib", true, false),
        ("bst", true, false),
        ("bib", false, true),
        ("bst", false, true),
    ] {
        let (mut server, directory, main) = fake_synctex_server(
            "printf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n",
        );
        let inputs = directory.path().join(".inputs");
        std::fs::create_dir(&inputs).unwrap();
        let first = inputs.join(format!("first.{extension}"));
        let second = inputs.join(format!("second.{extension}"));
        let alias = inputs.join(format!("refs.{extension}"));
        std::fs::write(&first, "@book{entry,title={Old}}\n").unwrap();
        std::fs::write(&second, "@book{entry,title={New}}\n").unwrap();
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let record = if extension == "bib" {
            "Database file #1:"
        } else {
            "The style file:"
        };
        let noop_log = if drop_log {
            "printf 'BibTeX finished\\n' > \"$1/main.blg\";"
        } else {
            ""
        };
        let compiler = directory.path().join("compiler.sh");
        std::fs::write(&compiler, format!("if [ -f \"$1/noop\" ]; then printf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"; {noop_log} exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nprintf '{record} {}\\n' > \"$1/main.blg\"\n", alias.display())).unwrap();
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let built = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(built["params"]["status"], "success", "{built}");
        assert_eq!(built["params"]["stale"], false, "{built}");
        assert_eq!(built["params"]["dependenciesCount"], 2, "{built}");
        let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
        assert_eq!(
            server.request(3, "workspace/executeCommand", forward.clone())["locations"][0]["page"],
            1
        );
        std::fs::remove_file(&alias).unwrap();
        if !remove {
            std::os::unix::fs::symlink(&second, &alias).unwrap();
        }
        server.send(
            json!({"jsonrpc":"2.0","id":4,"method":"workspace/executeCommand","params":forward}),
        );
        let rejected = server.until(|m| m["id"] == 4);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("File conditions changed")),
            "{extension}: {rejected}"
        );
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("noop"), "").unwrap();
        for id in [5, 6] {
            server.request(
                id,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let noop = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(noop["params"]["status"], "error", "{noop}");
            assert!(noop["params"]["pdfUri"].is_null(), "{noop}");
            assert!(
                noop["params"]["message"]
                    .as_str()
                    .is_some_and(|s| s.contains("file condition changed")),
                "{noop}"
            );
        }
        if alias.exists() {
            std::fs::remove_file(&alias).unwrap();
        }
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        server.request(
            7,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let restored = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(restored["params"]["status"], "success", "{restored}");
        assert_eq!(
            server.request(8, "workspace/executeCommand", forward.clone())["locations"][0]["page"],
            1
        );
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&second, &alias).unwrap();
        std::fs::remove_file(output.join("noop")).unwrap();
        server.request(
            9,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        assert_eq!(
            server.request(10, "workspace/executeCommand", forward)["locations"][0]["page"],
            1
        );
        server.request(11, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn bibliography_alias_changes_invalidate_active_builds_even_if_only_tex_refreshes_its_log() {
    for (extension, notify, keep_log, remove) in [
        ("bib", false, false, false),
        ("bib", true, false, false),
        ("bst", false, false, false),
        ("bst", true, false, false),
        ("bib", false, true, false),
        ("bst", false, true, false),
        ("bib", false, false, true),
        ("bib", true, false, true),
        ("bst", false, false, true),
        ("bst", true, false, true),
    ] {
        let record = if extension == "bib" {
            "Database file #1:"
        } else {
            "The style file:"
        };
        let script = format!("printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nif [ ! -f \"$1/keep-blg\" ]; then printf '{record} .inputs/refs.{extension}\\n' > \"$1/main.blg\"; fi\nprintf '%s:2: LaTeX Warning: bibliography warning.\\n' \"$2\" > \"$1/main.log\"\nif [ -f \"$1/hold\" ]; then touch \"$1/ready\"; while [ ! -f \"$1/release\" ]; do sleep 0.02; done; fi\n");
        let (mut server, directory, main) = fake_build_server_capabilities(
            &script,
            false,
            json!({"textDocument":{"diagnostic":{}}}),
        );
        let inputs = directory.path().join(".inputs");
        std::fs::create_dir(&inputs).unwrap();
        let first = inputs.join(format!("first.{extension}"));
        let second = inputs.join(format!("second.{extension}"));
        let alias = inputs.join(format!("refs.{extension}"));
        std::fs::write(&first, "@book{entry,title={Old}}\n").unwrap();
        std::fs::write(&second, "@book{entry,title={New}}\n").unwrap();
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let built = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(built["params"]["status"], "success", "{built}");
        assert_eq!(built["params"]["dependenciesCount"], 2, "{built}");
        let old = server.request(
            3,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main}}),
        );
        assert!(
            old["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["source"] == "latex-build"),
            "{old}"
        );
        let output = directory.path().join("build/latex-lsp");
        std::fs::write(output.join("hold"), "").unwrap();
        if keep_log {
            std::fs::write(output.join("keep-blg"), "").unwrap();
        }
        server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        wait_for_file(&output.join("ready"));
        std::fs::remove_file(&alias).unwrap();
        if !remove {
            std::os::unix::fs::symlink(&second, &alias).unwrap();
        }
        if notify {
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":Url::from_file_path(&alias).unwrap(),"type":2}]}),
            );
        } else {
            std::fs::write(output.join("release"), "").unwrap();
        }
        let changed = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(
            changed["params"]["status"],
            if notify { "cancelled" } else { "success" },
            "{changed}"
        );
        if !notify {
            assert_eq!(changed["params"]["stale"], true, "{changed}");
        }
        let cleared = server.request(
            5,
            "textDocument/diagnostic",
            json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
        );
        assert_eq!(cleared["kind"], "full", "{cleared}");
        assert_eq!(cleared["items"], json!([]), "{cleared}");
        std::fs::write(output.join("release"), "").unwrap();
        std::fs::remove_file(output.join("hold")).unwrap();
        if keep_log {
            std::fs::remove_file(output.join("keep-blg")).unwrap();
        }
        if remove {
            std::os::unix::fs::symlink(&second, &alias).unwrap();
        }
        server.request(
            6,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        server.request(7, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn known_external_aliases_remain_observed_when_the_new_target_is_not_indexed() {
    for extension in ["tex", "csv", "bib", "bst"] {
        for (notify, remove) in [(false, false), (true, false), (false, true), (true, true)] {
            let external = tempfile::tempdir().unwrap();
            let first = external.path().join(format!("first.{extension}"));
            let second = external.path().join(format!("second.{extension}"));
            let alias = external.path().join(format!("refs.{extension}"));
            std::fs::write(&first, "% OLD\n").unwrap();
            std::fs::write(&second, "% NEW\n").unwrap();
            std::os::unix::fs::symlink(&first, &alias).unwrap();
            let (mut server, directory, main) = fake_build_server_capabilities(
                "exit 1\n",
                false,
                json!({"textDocument":{"diagnostic":{}}}),
            );
            let command = match extension {
                "tex" | "csv" => "input",
                "bib" => "bibliography",
                _ => "bibliographystyle",
            };
            let source = format!(
                "\\documentclass{{article}}\n\\{command}{{{}}}\n",
                alias.display()
            );
            std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
            server.notify(
                "textDocument/didChange",
                json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
            );
            let record = match extension {
                "tex" | "csv" => format!("INPUT {}", alias.display()),
                "bib" => format!("Database file #1: {}", alias.display()),
                _ => format!("The style file: {}", alias.display()),
            };
            let log = if matches!(extension, "tex" | "csv") {
                "fls"
            } else {
                "blg"
            };
            std::fs::write(directory.path().join("compiler.sh"), format!(
                "printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nprintf '{record}\\n' >> \"$1/main.{log}\"\nprintf '%s:2: LaTeX Warning: prior warning.\\n' \"$2\" > \"$1/main.log\"\nif [ -f \"$1/hold\" ]; then touch \"$1/ready\"; while [ ! -f \"$1/release\" ]; do sleep 0.02; done; fi\n"
            )).unwrap();
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let built = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(built["params"]["status"], "success", "{extension}: {built}");
            assert_eq!(built["params"]["stale"], false, "{extension}: {built}");
            assert_eq!(
                built["params"]["dependenciesCount"], 2,
                "{extension}: {built}"
            );
            let old = server.request(
                3,
                "textDocument/diagnostic",
                json!({"textDocument":{"uri":main}}),
            );
            assert!(
                old["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["source"] == "latex-build"),
                "{old}"
            );
            let output = directory.path().join("build/latex-lsp");
            std::fs::write(output.join("hold"), "").unwrap();
            server.request(
                4,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            wait_for_file(&output.join("ready"));
            std::fs::remove_file(&alias).unwrap();
            if !remove {
                std::os::unix::fs::symlink(&second, &alias).unwrap();
            }
            if notify {
                server.notify(
                    "workspace/didChangeWatchedFiles",
                    json!({"changes":[{"uri":Url::from_file_path(&alias).unwrap(),"type":2}]}),
                );
            } else {
                std::fs::write(output.join("release"), "").unwrap();
            }
            let changed = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(
                changed["params"]["status"],
                if notify { "cancelled" } else { "success" },
                "{extension}: {changed}"
            );
            if !notify {
                assert_eq!(changed["params"]["stale"], true, "{extension}: {changed}");
            }
            let cleared = server.request(
                5,
                "textDocument/diagnostic",
                json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
            );
            assert_eq!(cleared["kind"], "full", "{cleared}");
            assert_eq!(cleared["items"], json!([]), "{cleared}");
            std::fs::write(output.join("release"), "").unwrap();
            std::fs::remove_file(output.join("hold")).unwrap();
            if remove {
                std::os::unix::fs::symlink(&second, &alias).unwrap();
            }
            server.request(
                6,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let rebuilt = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(
                rebuilt["params"]["status"], "success",
                "{extension}: {rebuilt}"
            );
            assert_eq!(rebuilt["params"]["stale"], false, "{extension}: {rebuilt}");
            server.request(7, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn bibliography_and_tex_bindings_keep_independent_scopes_and_reject_unrerun_tool_results() {
    for omit_tex in [true, false] {
        let (mut server, directory, main) = fake_synctex_server(
            "printf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n",
        );
        let inputs = directory.path().join(".inputs");
        std::fs::create_dir(&inputs).unwrap();
        let tex_first = inputs.join("first.tex");
        let tex_second = inputs.join("second.tex");
        let tex_alias = inputs.join("chapter.tex");
        let bib_first = inputs.join("first.bib");
        let bib_second = inputs.join("second.bib");
        let bib_alias = inputs.join("refs.bib");
        for path in [&tex_first, &tex_second] {
            std::fs::write(path, "Chapter.\n").unwrap();
        }
        for path in [&bib_first, &bib_second] {
            std::fs::write(path, "@book{entry,title={Title}}\n").unwrap();
        }
        std::os::unix::fs::symlink(&tex_first, &tex_alias).unwrap();
        std::os::unix::fs::symlink(&bib_first, &bib_alias).unwrap();
        let compiler = directory.path().join("compiler.sh");
        std::fs::write(&compiler, "printf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nif [ ! -f \"$1/keep-tex\" ]; then printf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"; if [ ! -f \"$1/omit-tex\" ]; then printf 'INPUT .inputs/chapter.tex\\n' >> \"$1/main.fls\"; fi; fi\nif [ ! -f \"$1/keep-bib\" ]; then printf 'BibTeX completed\\n' > \"$1/main.blg\"; if [ ! -f \"$1/omit-bib\" ]; then printf 'Database file #1: .inputs/refs.bib\\n' >> \"$1/main.blg\"; fi; fi\n").unwrap();
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let first = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(first["params"]["status"], "success", "{first}");
        assert_eq!(first["params"]["dependenciesCount"], 3, "{first}");
        let output = directory.path().join("build/latex-lsp");
        let (
            omit,
            keep,
            removed_alias,
            removed_original,
            removed_new,
            retained_alias,
            retained_original,
            retained_new,
        ) = if omit_tex {
            (
                "omit-tex",
                "keep-bib",
                &tex_alias,
                &tex_first,
                &tex_second,
                &bib_alias,
                &bib_first,
                &bib_second,
            )
        } else {
            (
                "omit-bib",
                "keep-tex",
                &bib_alias,
                &bib_first,
                &bib_second,
                &tex_alias,
                &tex_first,
                &tex_second,
            )
        };
        std::fs::write(output.join(omit), "").unwrap();
        std::fs::write(output.join(keep), "").unwrap();
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let reduced = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(reduced["params"]["status"], "success", "{reduced}");
        assert_eq!(reduced["params"]["stale"], false, "{reduced}");
        std::fs::remove_file(removed_alias).unwrap();
        std::os::unix::fs::symlink(removed_new, removed_alias).unwrap();
        std::fs::write(removed_original, "Unrelated old input changed.\n").unwrap();
        let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
        let valid = server.request(4, "workspace/executeCommand", forward.clone());
        assert_eq!(valid["locations"][0]["page"], 1, "{valid}");
        std::fs::remove_file(retained_alias).unwrap();
        std::os::unix::fs::symlink(retained_new, retained_alias).unwrap();
        server.request(
            5,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let stale = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(stale["params"]["status"], "success", "{stale}");
        assert_eq!(stale["params"]["stale"], true, "{stale}");
        std::fs::remove_file(retained_alias).unwrap();
        std::os::unix::fs::symlink(retained_original, retained_alias).unwrap();
        server.send(
            json!({"jsonrpc":"2.0","id":6,"method":"workspace/executeCommand","params":forward}),
        );
        let uncertain = server.until(|m| m["id"] == 6);
        assert!(
            uncertain["error"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("File conditions changed")),
            "{uncertain}"
        );
        std::fs::remove_file(output.join(keep)).unwrap();
        server.request(
            7,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let rebuilt = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
        assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
        assert_eq!(
            server.request(8, "workspace/executeCommand", forward)["locations"][0]["page"],
            1
        );
        server.request(9, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn tex_only_builds_do_not_adopt_changed_bibliography_contents_or_partial_tool_logs() {
    for extension in ["bib", "bst"] {
        for partial in [false, true] {
            let (mut server, directory, main) = fake_synctex_server(
                "printf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n",
            );
            let inputs = directory.path().join(".inputs");
            std::fs::create_dir(&inputs).unwrap();
            let bibliography = inputs.join(format!("refs.{extension}"));
            let original = "@book{entry,title={OLD}}\n";
            let replacement = "@book{entry,title={NEW}}\n";
            std::fs::write(&bibliography, original).unwrap();
            let modified = std::fs::metadata(&bibliography)
                .unwrap()
                .modified()
                .unwrap();
            let record = if extension == "bib" {
                "Database file #1:"
            } else {
                "The style file:"
            };
            let compiler = directory.path().join("compiler.sh");
            std::fs::write(&compiler, format!(
                "if [ -f \"$1/noop\" ]; then exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nif [ ! -f \"$1/keep-bib\" ]; then printf '{record} .inputs/refs.{extension}\\n' > \"$1/main.blg\"; elif [ -f \"$1/partial\" ]; then cp \"${{2%/*}}/partial-template\" \"$1/main.blg\"; fi\nprintf '%s:2: LaTeX Warning: bibliography warning.\\n' \"$2\" > \"$1/main.log\"\n"
            )).unwrap();
            server.notify("workspace/didChangeConfiguration", json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}));
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let first = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(first["params"]["status"], "success", "{first}");
            assert_eq!(first["params"]["stale"], false, "{first}");
            let old = server.request(
                3,
                "textDocument/diagnostic",
                json!({"textDocument":{"uri":main}}),
            );
            assert!(
                old["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["source"] == "latex-build"),
                "{old}"
            );
            let output = directory.path().join("build/latex-lsp");
            std::fs::write(output.join("keep-bib"), "").unwrap();
            if partial {
                std::fs::write(
                    directory.path().join("partial-template"),
                    format!(
                        "{record} .inputs/refs.{extension}\n{}\n",
                        " ".repeat(9 * 1024 * 1024)
                    ),
                )
                .unwrap();
                std::fs::write(output.join("partial"), "").unwrap();
            }
            std::fs::write(&bibliography, replacement).unwrap();
            std::fs::File::open(&bibliography)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
            server.request(
                4,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let reused = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(
                reused["params"]["status"], "success",
                "{extension}, partial={partial}: {reused}"
            );
            assert_eq!(
                reused["params"]["stale"], true,
                "{extension}, partial={partial}: {reused}"
            );
            assert_eq!(
                reused["params"]["diagnosticsTruncated"], partial,
                "{reused}"
            );
            let cleared = server.request(
                5,
                "textDocument/diagnostic",
                json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
            );
            assert_eq!(cleared["kind"], "full", "{cleared}");
            assert_eq!(cleared["items"], json!([]), "{cleared}");
            // Restoring the data cannot establish which contents produced this
            // new PDF; only a complete bibliography rerun can replace history.
            std::fs::write(&bibliography, original).unwrap();
            std::fs::File::open(&bibliography)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
            let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
            server.send(json!({"jsonrpc":"2.0","id":6,"method":"workspace/executeCommand","params":forward}));
            let uncertain = server.until(|m| m["id"] == 6);
            assert!(
                uncertain["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("File conditions changed")),
                "{uncertain}"
            );
            std::fs::write(output.join("noop"), "").unwrap();
            for id in [7, 8] {
                server.request(
                    id,
                    "workspace/executeCommand",
                    json!({"command":"latex.build","arguments":[main]}),
                );
                let noop = server.until(|m| m["method"] == "latex/buildFinished");
                assert_eq!(noop["params"]["status"], "error", "{noop}");
                assert!(noop["params"]["pdfUri"].is_null(), "{noop}");
            }
            std::fs::write(&bibliography, replacement).unwrap();
            std::fs::remove_file(output.join("noop")).unwrap();
            std::fs::remove_file(output.join("keep-bib")).unwrap();
            if partial {
                std::fs::remove_file(output.join("partial")).unwrap();
            }
            server.request(
                9,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let rebuilt = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
            assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
            let valid = server.request(10, "workspace/executeCommand", forward);
            assert_eq!(valid["locations"][0]["page"], 1, "{valid}");
            server.request(11, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn known_external_bindings_track_unindexed_target_contents_and_active_retargets() {
    for extension in ["tex", "bib"] {
        for notify in [false, true] {
            let external = tempfile::tempdir().unwrap();
            let first = external.path().join(format!("first.{extension}"));
            let second = external.path().join(format!("second.{extension}"));
            let third = external.path().join(format!("third.{extension}"));
            let alias = external.path().join(format!("alias.{extension}"));
            let original = if extension == "tex" {
                "OLD \\label{old-entry}\n"
            } else {
                "@book{old-entry,title={OLD}}\n"
            };
            let replacement = if extension == "tex" {
                "NEW \\label{unindexed-entry}\n"
            } else {
                "@book{unindexed-entry,title={NEW}}\n"
            };
            std::fs::write(&first, original).unwrap();
            std::fs::write(&second, replacement).unwrap();
            std::fs::write(
                &third,
                replacement.replace("unindexed-entry", "third-entry"),
            )
            .unwrap();
            std::os::unix::fs::symlink(&first, &alias).unwrap();
            let (mut server, directory, main) = fake_synctex_server("touch queried\nprintf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n");
            let command = if extension == "tex" {
                "input"
            } else {
                "bibliography"
            };
            let source = format!(
                "\\documentclass{{article}}\n\\def\\selected{{{}}}\\{command}{{\\selected}}\n",
                alias.display()
            );
            std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
            server.notify(
                "textDocument/didChange",
                json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
            );
            let first_uri = Url::from_file_path(&first).unwrap();
            server.notify("textDocument/didOpen", json!({"textDocument":{"uri":first_uri,"languageId":if extension=="tex" {"latex"} else {"bibtex"},"version":1,"text":original}}));
            server.request(
                0,
                "textDocument/documentSymbol",
                json!({"textDocument":{"uri":first_uri}}),
            );
            let record = if extension == "tex" {
                format!("printf 'INPUT {}\\n' >> \"$1/main.fls\"\n", alias.display())
            } else {
                format!(
                    "printf 'Database file #1: {}\\n' > \"$1/main.blg\"\n",
                    alias.display()
                )
            };
            let compiler = directory.path().join("compiler.sh");
            std::fs::write(&compiler, format!("if [ -f \"$1/noop\" ]; then exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\n{record}printf '%s:2: LaTeX Warning: dependency warning.\\n' \"$2\" > \"$1/main.log\"\nif [ -f \"$1/hold\" ]; then touch \"$1/ready\"; while [ ! -f \"$1/release\" ]; do sleep 0.02; done; fi\n")).unwrap();
            server.notify("workspace/didChangeConfiguration", json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}));
            server.request(
                2,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let first_build = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(first_build["params"]["status"], "success", "{first_build}");
            std::fs::remove_file(&alias).unwrap();
            std::os::unix::fs::symlink(&second, &alias).unwrap();
            server.request(
                3,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let selected = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(
                selected["params"]["status"], "success",
                "{extension}: {selected}"
            );
            assert_eq!(selected["params"]["stale"], false, "{selected}");
            let symbols = server.request(4, "workspace/symbol", json!({"query":"unindexed-entry"}));
            assert_eq!(symbols, json!([]), "{symbols}");
            let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
            assert_eq!(
                server.request(5, "workspace/executeCommand", forward.clone())["locations"][0]
                    ["page"],
                1
            );
            std::fs::remove_file(directory.path().join("queried")).unwrap();
            let modified = std::fs::metadata(&second).unwrap().modified().unwrap();
            std::fs::write(&second, replacement.replace("NEW", "ALT")).unwrap();
            std::fs::File::open(&second)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
            server.send(json!({"jsonrpc":"2.0","id":6,"method":"workspace/executeCommand","params":forward}));
            let rejected = server.until(|m| m["id"] == 6);
            assert!(
                rejected["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("Input contents changed")),
                "{rejected}"
            );
            assert!(!directory.path().join("queried").exists());
            let output = directory.path().join("build/latex-lsp");
            std::fs::write(output.join("noop"), "").unwrap();
            for id in [7, 8] {
                server.request(
                    id,
                    "workspace/executeCommand",
                    json!({"command":"latex.build","arguments":[main]}),
                );
                let noop = server.until(|m| m["method"] == "latex/buildFinished");
                assert_eq!(noop["params"]["status"], "error", "{noop}");
                assert!(noop["params"]["pdfUri"].is_null(), "{noop}");
            }
            std::fs::write(&second, replacement).unwrap();
            std::fs::File::open(&second)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
            server.request(
                91,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let restored = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(restored["params"]["status"], "success", "{restored}");
            assert_eq!(
                server.request(9, "workspace/executeCommand", forward.clone())["locations"][0]
                    ["page"],
                1
            );
            std::fs::remove_file(output.join("noop")).unwrap();
            // Resolve the known raw name at launch, rather than snapshotting
            // its previous target. The new target remains outside the index.
            std::fs::remove_file(&alias).unwrap();
            std::os::unix::fs::symlink(&third, &alias).unwrap();
            std::fs::write(output.join("hold"), "").unwrap();
            server.request(
                10,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            wait_for_file(&output.join("ready"));
            let modified = std::fs::metadata(&third).unwrap().modified().unwrap();
            std::fs::write(
                &third,
                replacement
                    .replace("unindexed-entry", "third-entry")
                    .replace("NEW", "ALT"),
            )
            .unwrap();
            std::fs::File::open(&third)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
            if notify {
                server.notify(
                    "workspace/didChangeWatchedFiles",
                    json!({"changes":[{"uri":Url::from_file_path(&third).unwrap(),"type":2}]}),
                );
            } else {
                std::fs::write(output.join("release"), "").unwrap();
            }
            let changed = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(
                changed["params"]["status"],
                if notify { "cancelled" } else { "success" },
                "{changed}"
            );
            if !notify {
                assert_eq!(changed["params"]["stale"], true, "{changed}");
            }
            std::fs::write(output.join("release"), "").unwrap();
            std::fs::remove_file(output.join("hold")).unwrap();
            server.request(
                11,
                "workspace/executeCommand",
                json!({"command":"latex.build","arguments":[main]}),
            );
            let rebuilt = server.until(|m| m["method"] == "latex/buildFinished");
            assert_eq!(rebuilt["params"]["status"], "success", "{rebuilt}");
            assert_eq!(rebuilt["params"]["stale"], false, "{rebuilt}");
            let old = server.request(
                12,
                "textDocument/diagnostic",
                json!({"textDocument":{"uri":main}}),
            );
            assert!(
                old["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["source"] == "latex-build"),
                "{old}"
            );
            std::fs::write(
                &third,
                replacement.replace("unindexed-entry", "third-entry"),
            )
            .unwrap();
            std::fs::File::open(&third)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
            server.notify(
                "workspace/didChangeWatchedFiles",
                json!({"changes":[{"uri":Url::from_file_path(&third).unwrap(),"type":2}]}),
            );
            let cleared = server.request(
                13,
                "textDocument/diagnostic",
                json!({"textDocument":{"uri":main},"previousResultId":old["resultId"]}),
            );
            assert_eq!(cleared["kind"], "full", "{cleared}");
            assert_eq!(cleared["items"], json!([]), "{cleared}");
            server.request(14, "shutdown", Value::Null);
            server.notify("exit", Value::Null);
            server.wait_exit(0);
        }
    }
}

#[test]
#[cfg(unix)]
fn compiler_input_roles_survive_open_edits_close_and_versioned_cross_file_rename() {
    for (database, extension) in [
        (true, "payload"),
        (true, "tex"),
        (true, "csv"),
        (true, "png"),
        (true, "bst"),
        (false, "bib"),
        (false, "bst"),
        (false, "png"),
    ] {
        let (mut server, directory, main) = fake_build_server_capabilities(
            "exit 1\n",
            false,
            json!({"workspace":{"workspaceEdit":{"documentChanges":true}}}),
        );
        let inputs = directory.path().join(".inputs");
        std::fs::create_dir(&inputs).unwrap();
        let physical = inputs.join(format!("physical.{extension}"));
        let alias = inputs.join(if database { "alias.bib" } else { "alias.sty" });
        let contents = if database {
            "@book{entry,title={OLD}}\n"
        } else {
            "\\newcommand{\\fromalias}{OLD}\n"
        };
        std::fs::write(&physical, contents).unwrap();
        std::os::unix::fs::symlink(&physical, &alias).unwrap();
        let data_uri = Url::from_file_path(&physical).unwrap();
        let source = if database {
            "\\documentclass{article}\n\\def\\selected{.inputs/alias}\\bibliography{\\selected}\\cite{entry}\n"
        } else {
            "\\documentclass{article}\n\\def\\selected{.inputs/alias}\\usepackage{\\selected}\\fromalias\n"
        };
        let at = latex_lsp::text::Text::new(source.into()).position(if database {
            source.find("cite{entry}").unwrap() + 6
        } else {
            source.find("\\fromalias").unwrap() + 3
        });
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        let extra = if database {
            "printf 'Database file #1: .inputs/alias.bib\\n' > \"$1/main.blg\"\n"
        } else {
            "printf 'INPUT .inputs/alias.sty\\n' >> \"$1/main.fls\"\n"
        };
        std::fs::write(directory.path().join("compiler.sh"), format!("printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\n{extra}")).unwrap();
        assert_eq!(
            server.request(
                2,
                "textDocument/definition",
                json!({"textDocument":{"uri":main},"position":at})
            ),
            json!([])
        );
        server.request(
            3,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let built = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(
            built["params"]["status"], "success",
            "{database}/{extension}: {built}"
        );
        let definitions = server.request(
            4,
            "textDocument/definition",
            json!({"textDocument":{"uri":main},"position":at}),
        );
        assert_eq!(definitions[0]["uri"], data_uri.as_str(), "{definitions}");
        let references = server.request(5, "textDocument/references", json!({"textDocument":{"uri":main},"position":at,"context":{"includeDeclaration":true}}));
        assert_eq!(references.as_array().unwrap().len(), 2, "{references}");
        if database {
            let completion = server.request(
                6,
                "textDocument/completion",
                json!({"textDocument":{"uri":main},"position":at}),
            );
            assert!(
                completion
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["label"] == "entry"),
                "{completion}"
            );
        }
        // The confirmed input role wins over the physical extension and stays
        // attached to the editor buffer through full and incremental updates.
        server.notify("textDocument/didOpen", json!({"textDocument":{"uri":data_uri,"languageId":"plaintext","version":1,"text":contents}}));
        let changed = contents.replace("OLD", "NEW");
        std::fs::write(&physical, &changed).unwrap();
        server.notify("textDocument/didChange", json!({"textDocument":{"uri":data_uri,"version":2},"contentChanges":[{"text":changed}]}));
        let new_name = if database {
            "renamedentry"
        } else {
            "renamedalias"
        };
        let renamed = server.request(
            7,
            "textDocument/rename",
            json!({"textDocument":{"uri":main},"position":at,"newName":new_name}),
        );
        let edits = renamed["documentChanges"].as_array().unwrap();
        assert_eq!(edits.len(), 2, "{renamed}");
        assert!(
            edits
                .iter()
                .any(|edit| edit["textDocument"]["uri"] == data_uri.as_str()
                    && edit["textDocument"]["version"] == 2),
            "{renamed}"
        );
        for edit in edits {
            let uri: Url = serde_json::from_value(edit["textDocument"]["uri"].clone()).unwrap();
            let path = uri.to_file_path().unwrap();
            let changes: Vec<tower_lsp::lsp_types::TextEdit> =
                serde_json::from_value(edit["edits"].clone()).unwrap();
            let text = latex_lsp::text::Text::new(std::fs::read_to_string(&path).unwrap())
                .changed(
                    changes
                        .into_iter()
                        .map(
                            |edit| tower_lsp::lsp_types::TextDocumentContentChangeEvent {
                                range: Some(edit.range),
                                range_length: None,
                                text: edit.new_text,
                            },
                        )
                        .collect(),
                )
                .unwrap()
                .source;
            std::fs::write(&path, &text).unwrap();
            server.notify(
                "textDocument/didChange",
                json!({"textDocument":{"uri":uri,"version":3},"contentChanges":[{"text":text}]}),
            );
        }
        let definitions = server.request(
            8,
            "textDocument/definition",
            json!({"textDocument":{"uri":main},"position":at}),
        );
        assert_eq!(definitions[0]["uri"], data_uri.as_str(), "{definitions}");
        server.notify(
            "textDocument/didClose",
            json!({"textDocument":{"uri":data_uri}}),
        );
        server.notify(
            "workspace/didChangeWatchedFiles",
            json!({"changes":[{"uri":data_uri,"type":2}]}),
        );
        let closed = server.request(
            9,
            "textDocument/definition",
            json!({"textDocument":{"uri":main},"position":at}),
        );
        assert_eq!(closed[0]["uri"], data_uri.as_str(), "{closed}");
        server.request(10, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn bibliography_alias_data_suffixes_do_not_bypass_external_source_index_scope() {
    for extension in ["csv", "png"] {
        let external = tempfile::tempdir().unwrap();
        let physical = external.path().join(format!("database.{extension}"));
        let alias = external.path().join("refs.bib");
        let contents = "@book{external-entry,title={Book}}\n";
        std::fs::write(&physical, contents).unwrap();
        std::os::unix::fs::symlink(&physical, &alias).unwrap();
        let (mut server, directory, main) = fake_build_server("exit 1\n", false);
        let source = format!("\\documentclass{{article}}\n\\def\\selected{{{}}}\\bibliography{{\\selected}}\\cite{{external-entry}}\n", alias.display());
        std::fs::write(main.to_file_path().unwrap(), &source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        std::fs::write(directory.path().join("compiler.sh"), format!("printf '%%PDF-1.4\\n' > \"$1/main.pdf\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\nprintf 'Database file #1: {}\\n' > \"$1/main.blg\"\n", alias.display())).unwrap();
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let built = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(built["params"]["status"], "success", "{built}");
        assert_eq!(built["params"]["dependenciesCount"], 1, "{built}");
        assert_eq!(
            server.request(3, "workspace/symbol", json!({"query":"external-entry"})),
            json!([])
        );
        let physical_uri = Url::from_file_path(&physical).unwrap();
        server.notify("textDocument/didOpen", json!({"textDocument":{"uri":physical_uri,"languageId":"bibtex","version":1,"text":contents}}));
        server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let opened = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(opened["params"]["status"], "success", "{opened}");
        assert_eq!(opened["params"]["dependenciesCount"], 2, "{opened}");
        let at = latex_lsp::text::Text::new(source.clone())
            .position(source.find("cite{external-entry}").unwrap() + 7);
        let definitions = server.request(
            5,
            "textDocument/definition",
            json!({"textDocument":{"uri":main},"position":at}),
        );
        assert_eq!(
            definitions[0]["uri"],
            physical_uri.as_str(),
            "{definitions}"
        );
        server.request(6, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
#[cfg(unix)]
fn static_bibliography_alias_roles_protect_old_pdfs_without_bibliography_logs() {
    for style in [false, true] {
        let (mut server, directory, main) = fake_synctex_server(
            "printf 'SyncTeX result begin\\nPage:1\\nx:10\\ny:20\\nSyncTeX result end\\n'\n",
        );
        let database = directory.path().join("database.payload");
        let stylesheet = directory.path().join("style.tex");
        std::fs::write(&database, "@book{entry,title={OLD}}\n").unwrap();
        std::fs::write(&stylesheet, "OLD STYLE\n").unwrap();
        std::os::unix::fs::symlink(&database, directory.path().join("refs.bib")).unwrap();
        std::os::unix::fs::symlink(&stylesheet, directory.path().join("chosen.bst")).unwrap();
        let source = "\\documentclass{article}\n\\bibliographystyle{chosen}\\bibliography{refs}\\cite{entry}\n";
        std::fs::write(main.to_file_path().unwrap(), source).unwrap();
        server.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":main,"version":2},"contentChanges":[{"text":source}]}),
        );
        let compiler = directory.path().join("compiler.sh");
        std::fs::write(&compiler, "if [ -f \"$1/noop\" ]; then exit 0; fi\nprintf '%%PDF-1.4\\nCOMPILED\\n' > \"$1/main.pdf\"\nprintf 'SyncTeX data\\n' > \"$1/main.synctex\"\nprintf 'INPUT %s\\n' \"$2\" > \"$1/main.fls\"\n").unwrap();
        server.notify(
            "workspace/didChangeConfiguration",
            json!({"settings":{"build":{"executable":"/bin/sh","arguments":[compiler,"%o","%f"]}}}),
        );
        server.request(
            2,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let built = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(built["params"]["status"], "success", "{built}");
        let input = if style { &stylesheet } else { &database };
        let original = std::fs::read_to_string(input).unwrap();
        let modified = std::fs::metadata(input).unwrap().modified().unwrap();
        std::fs::write(input, original.replace("OLD", "NEW")).unwrap();
        std::fs::File::open(input)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let forward = json!({"command":"latex.forwardSearch","arguments":[{"uri":main,"position":{"line":1,"character":0}}]});
        server.send(
            json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":forward}),
        );
        let rejected = server.until(|m| m["id"] == 3);
        assert!(
            rejected["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Input contents changed")),
            "{rejected}"
        );
        std::fs::write(directory.path().join("build/latex-lsp/noop"), "").unwrap();
        server.request(
            4,
            "workspace/executeCommand",
            json!({"command":"latex.build","arguments":[main]}),
        );
        let noop = server.until(|m| m["method"] == "latex/buildFinished");
        assert_eq!(noop["params"]["status"], "error", "{noop}");
        assert!(noop["params"]["pdfUri"].is_null(), "{noop}");
        server.request(5, "shutdown", Value::Null);
        server.notify("exit", Value::Null);
        server.wait_exit(0);
    }
}

#[test]
fn environment_linked_editing_tracks_incremental_changes_and_validates_positions_over_stdio() {
    use latex_lsp::text::Text;
    let mut server = Server::start();
    let uri = Url::parse("file:///linked-editing-protocol/main.tex").unwrap();
    let alias = "file:///linked-editing-protocol/%6dain.tex";
    let initialized = server.request(1, "initialize", json!({"processId":null,"capabilities":{"textDocument":{"linkedEditingRange":{"dynamicRegistration":false}}}}));
    assert_eq!(
        initialized["capabilities"]["linkedEditingRangeProvider"],
        true
    );
    server.notify("initialized", json!({}));
    let source = "😀 \\begin{quote}\r\n\\begin{quote}Inner\\end{quote}\r\n\\end{quote}\r\n";
    let text = Text::new(source.into());
    server.notify(
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri,"languageId":"latex","version":7,"text":source}}),
    );
    let begin = source.find("\n\\begin{quote}").unwrap() + 1 + "\\begin{".len();
    let end = source.find("\\end{quote}").unwrap() + "\\end{".len();
    let ranges = [text.range(begin, begin + 5), text.range(end, end + 5)];
    let result = server.request(
        2,
        "textDocument/linkedEditingRange",
        json!({"textDocument":{"uri":alias},"position":text.position(begin + 2)}),
    );
    assert_eq!(result["ranges"], json!(ranges));
    assert!(result["wordPattern"].as_str().is_some());
    server.notify("textDocument/didChange", json!({"textDocument":{"uri":alias,"version":8},"contentChanges":[{"range":ranges[1],"text":"center"},{"range":ranges[0],"text":"center"}]}));
    let changed = text
        .changed(vec![
            tower_lsp::lsp_types::TextDocumentContentChangeEvent {
                range: Some(ranges[1]),
                range_length: None,
                text: "center".into(),
            },
            tower_lsp::lsp_types::TextDocumentContentChangeEvent {
                range: Some(ranges[0]),
                range_length: None,
                text: "center".into(),
            },
        ])
        .unwrap();
    let begin = changed.source.find("\\begin{center}").unwrap() + "\\begin{".len();
    let end = changed.source.find("\\end{center}").unwrap() + "\\end{".len();
    let updated = server.request(
        3,
        "textDocument/linkedEditingRange",
        json!({"textDocument":{"uri":uri},"position":changed.position(end + 2)}),
    );
    assert_eq!(
        updated["ranges"],
        json!([changed.range(begin, begin + 6), changed.range(end, end + 6)])
    );
    let missing = server.request(4, "textDocument/linkedEditingRange", json!({"textDocument":{"uri":"file:///linked-editing-protocol/missing.tex"},"position":{"line":0,"character":0}}));
    assert!(missing.is_null());
    server.send(json!({"jsonrpc":"2.0","id":5,"method":"textDocument/linkedEditingRange","params":{"textDocument":{"uri":uri},"position":{"line":0,"character":1}}}));
    let invalid = server.until(|message| message["id"] == 5);
    assert_eq!(invalid["error"]["code"], -32602);
    server.request(6, "shutdown", Value::Null);
    server.notify("exit", Value::Null);
    server.wait_exit(0);
}
