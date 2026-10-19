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
