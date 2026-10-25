#!/usr/bin/env python3
"""Replay an editor session against a real stdio language-server process (stdlib only)."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import queue
import statistics
import subprocess
import sys
import tempfile
import threading
import time


SCALES = {"small": (1, 16), "medium": (4, 32), "large": (8, 64)}


class Server:
    def __init__(self, binary, cwd, timeout):
        self.process = subprocess.Popen(
            [str(binary), "--stdio"], cwd=cwd, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        )
        self.timeout = timeout
        self.messages = queue.Queue()
        self.next_id = 0
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self):
        try:
            while True:
                headers = {}
                while True:
                    line = self.process.stdout.readline()
                    if not line:
                        raise EOFError("server closed stdout")
                    if line == b"\r\n":
                        break
                    name, value = line.decode("ascii").split(":", 1)
                    headers[name.lower()] = value.strip()
                length = int(headers["content-length"])
                body = self.process.stdout.read(length)
                self.messages.put(json.loads(body))
        except Exception as error:
            self.messages.put(error)

    def send(self, message):
        body = json.dumps({"jsonrpc": "2.0", **message}, ensure_ascii=False).encode()
        self.process.stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
        self.process.stdin.flush()

    def notify(self, method, params):
        self.send({"method": method, "params": params})

    def request(self, method, params):
        self.next_id += 1
        request_id = self.next_id
        self.send({"id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + self.timeout
        while True:
            try:
                message = self.messages.get(timeout=max(0, deadline - time.monotonic()))
            except queue.Empty as error:
                raise TimeoutError(f"{method}: no response within {self.timeout}s") from error
            if isinstance(message, Exception):
                raise message
            if "method" in message:
                if "id" in message:
                    # This client advertises no dynamic registration/refresh support.
                    self.send({"id": message["id"], "result": None})
                continue
            if message.get("id") != request_id:
                raise RuntimeError(f"unexpected response: {message}")
            if "error" in message:
                raise RuntimeError(f"{method}: {message['error']}")
            return message["result"]

    def close(self):
        try:
            if self.process.poll() is None:
                self.request("shutdown", None)
                self.notify("exit", None)
                self.process.wait(timeout=self.timeout)
                if self.process.returncode:
                    raise RuntimeError(f"server exited with {self.process.returncode}")
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait()
            self.process.stdin.close()
            self.reader.join(timeout=1)
            self.process.stdout.close()


def position(source, offset):
    prefix = source[:offset]
    return {"line": prefix.count("\n"),
            "character": len(prefix.rsplit("\n", 1)[-1].encode("utf-16-le")) // 2}


def fixture(root, scale, hooks=False, hook_wrappers=False):
    chapters, sections = SCALES[scale]
    main = ("\\documentclass{article}\n\\usepackage{amsmath,biblatex}\n"
            "\\addbibresource{references.bib}\n\\newcommand{\\projectterm}[1]{\\textbf{#1}}\n"
            "\\begin{document}\n")
    if hooks:
        main = main.replace("\\begin{document}\n", (
            "\\NewHookWithArguments{benchmark}{1}\n"
            "\\AddToHookWithArguments{benchmark}{\\typeout{#1}}\n"
            "\\begin{document}\n\\UseHookWithArguments{benchmark}{0}{Data}\n"
        ))
    if hook_wrappers:
        main = main.replace("\\begin{document}\n", (
            "\\newcommand\\benchreader[1]{\\UseHook{benchmark}{#1}}\n"
            "\\newcommand\\benchforward[1]{\\benchreader{#1}}\n"
            "\\begin{document}\n"
        ))
    for chapter in range(chapters):
        source = "% !TeX root = main.tex\n"
        for section in range(sections):
            key = f"{chapter}:{section}"
            source += (
                f"\\section{{Experiment {key}}}\\label{{sec:{key}}}\n"
                f"中文 😀 Results use \\projectterm{{measurements}}; see \\ref{{sec:0:0}} "
                f"and \\cite{{paper{section}}}.\n"
                "\\begin{equation}\nE = mc^2 + \\frac{a}{b}\\label{eq:" + key + "}\n"
                "\\end{equation}\n"
                "The experiment compares latency, throughput, and repeatability. "
                "Each observation includes an independent reference measurement.\n"
            )
            if hook_wrappers:
                source += "\\benchforward{Literal data}\n"
        # Stable query locations followed by a region edited throughout the session.
        source += ("\\ref{sec:0:0}\n\\cite{paper0}\n\\projectterm{value}\n"
                   "\\textbf{probe}\n中文 😀 EDIT-HERE\nREFERENCE-HERE\n")
        (root / f"chapter{chapter}.tex").write_text(source)
        main += f"\\input{{chapter{chapter}}}\n"
    main += "\\printbibliography\n\\end{document}\n"
    (root / "main.tex").write_text(main)
    bib = "".join(
        f"@article{{paper{i}, author={{Author, Alice}}, title={{Measurement {i}}}, "
        f"journal={{Experiments}}, year={{2024}}}}\n" for i in range(sections * 4)
    )
    (root / "references.bib").write_text(bib)
    files = sorted(root.iterdir())
    digest = hashlib.sha256()
    for path in files:
        digest.update(path.name.encode() + b"\0" + path.read_bytes())
    return {"files": len(files), "bytes": sum(p.stat().st_size for p in files),
            "sha256": digest.hexdigest()}


def session(binary, scale, iterations, warmup, timeout, record, hooks=False, hook_wrappers=False):
    with tempfile.TemporaryDirectory(prefix="latex-lsp-benchmark-") as directory:
        root = Path(directory)
        metadata = fixture(root, scale, hooks, hook_wrappers)
        server = Server(binary, root, timeout)
        sources = {p.as_uri(): p.read_text() for p in root.glob("*.tex")}
        versions = {uri: 1 for uri in sources}
        uri = (root / "chapter0.tex").as_uri()
        main_uri = (root / "main.tex").as_uri()
        document = {"textDocument": {"uri": uri}}

        def timed(name, operation, measured=True):
            start = time.perf_counter_ns()
            result = operation()
            elapsed = (time.perf_counter_ns() - start) / 1e6
            if measured:
                record(name, elapsed)
            return result

        def fence():
            status = server.request("workspace/executeCommand", {"command": "latex.indexingStatus"})
            assert status["files"] == metadata["files"] and not status["truncated"], status

        def edit(target, start, end, replacement):
            source = sources[target]
            change = {"range": {"start": position(source, start), "end": position(source, end)},
                      "text": replacement}
            sources[target] = source[:start] + replacement + source[end:]
            versions[target] += 1
            server.notify("textDocument/didChange", {
                "textDocument": {"uri": target, "version": versions[target]},
                "contentChanges": [change],
            })

        def at(needle, delta=0):
            source = sources[uri]
            return {**document, "position": position(source, source.rindex(needle) + delta)}

        try:
            timed("initialize", lambda: server.request("initialize", {
                "processId": None, "rootUri": root.as_uri(),
                "capabilities": {"textDocument": {
                    "diagnostic": {}, "documentSymbol": {"hierarchicalDocumentSymbolSupport": True},
                }},
                "initializationOptions": {"distribution": {"enabled": False}},
            }))
            server.notify("initialized", {})
            for target in [main_uri, uri]:
                server.notify("textDocument/didOpen", {"textDocument": {
                    "uri": target, "languageId": "latex", "version": 1, "text": sources[target],
                }})
            timed("open", fence)
            for step in range(warmup + iterations):
                measured = step >= warmup
                # Insert/delete at a UTF-16 position following CJK and a surrogate pair.
                # Every fourth edit pastes/removes multiple lines and a new declaration.
                marker = sources[uri].index("EDIT-HERE")
                added = "输入" if step % 4 < 2 else "\n\\subsection{Draft}\\label{draft:new}\n"
                def change_prose():
                    if step % 2 == 0:
                        edit(uri, marker, marker, added)
                    else:
                        edit(uri, marker - len(added), marker, "")
                    fence()
                timed("edit.prose", change_prose, measured)

                # Change the project preamble without saving; child queries must see it.
                old, new = ("[1]", "[2]") if step % 2 == 0 else ("[2]", "[1]")
                def change_preamble():
                    start = sources[main_uri].index(old)
                    edit(main_uri, start, start + len(old), new)
                    fence()
                timed("edit.preamble", change_preamble, measured)

                def query(name, method, params):
                    return timed(name, lambda: server.request(method, params), measured)

                completion = query("completion.labels", "textDocument/completion", at("sec:0:0", 4))
                items = completion["items"] if isinstance(completion, dict) else completion
                assert any(item["label"] == "sec:0:0" for item in items), items
                completion = query("completion.citations", "textDocument/completion", at("paper0", 3))
                items = completion["items"] if isinstance(completion, dict) else completion
                assert any(item["label"] == "paper0" for item in items), items
                completion = query("completion.commands", "textDocument/completion", at("\\textbf", 4))
                items = completion["items"] if isinstance(completion, dict) else completion
                assert any(item["label"] == "textbf" for item in items), items
                assert query("hover", "textDocument/hover", at("\\textbf", 3))
                definitions = query("definition", "textDocument/definition", at("sec:0:0", 4))
                assert len(definitions) == 1 and definitions[0]["uri"] == uri, definitions
                declaration = sources[uri].index(r"\label{sec:0:0}") + len(r"\label{")
                assert definitions[0]["range"] == {
                    "start": position(sources[uri], declaration),
                    "end": position(sources[uri], declaration + len("sec:0:0")),
                }, definitions
                references = query("references", "textDocument/references", {
                    **at("sec:0:0", 4), "context": {"includeDeclaration": True},
                })
                chapters, sections = SCALES[scale]
                expected_references = chapters * (sections + 1) + 1
                assert len(references) == expected_references, (len(references), expected_references)
                assert {item["uri"] for item in references} == {
                    (root / f"chapter{chapter}.tex").as_uri() for chapter in range(chapters)
                }, references
                rename = query("rename", "textDocument/rename", {
                    **at("sec:0:0", 4), "newName": "sec:renamed",
                })
                changes = rename["changes"]
                assert set(changes) == {item["uri"] for item in references}, changes
                assert sum(len(edits) for edits in changes.values()) == expected_references, changes
                assert all(edit["newText"] == "sec:renamed"
                           for edits in changes.values() for edit in edits), changes
                signature = query("signature", "textDocument/signatureHelp", at("{value}", 2))
                assert len(signature["signatures"][0]["parameters"]) == (2 if step % 2 == 0 else 1), signature
                if hooks:
                    hook_source = sources[main_uri]
                    signature = query("signature.hook", "textDocument/signatureHelp", {
                        "textDocument": {"uri": main_uri},
                        "position": position(hook_source, hook_source.index("{Data}") + 2),
                    })
                    assert signature["signatures"][0]["label"] == (
                        r"\UseHookWithArguments{hook}{argument count}{argument 1}"
                    ), signature
                    assert signature["activeParameter"] == 2, signature
                assert query("symbols", "textDocument/documentSymbol", document)
                symbols = query("workspace.symbols.exact", "workspace/symbol", {"query": "sec:0:0"})
                assert symbols and symbols[0]["name"] == "sec:0:0", symbols
                assert symbols[0]["location"]["uri"] == uri, symbols
                symbols = query("workspace.symbols.all", "workspace/symbol", {"query": ""})
                assert symbols and len(symbols) <= 256, symbols
                assert all(symbol["kind"] != 7 for symbol in symbols), symbols
                assert query("workspace.symbols.missing", "workspace/symbol", {
                    "query": "absent:benchmark:symbol",
                }) == []
                assert query("folding", "textDocument/foldingRange", document)
                tokens = query("semantic.full", "textDocument/semanticTokens/full", document)
                assert tokens["data"], tokens
                delta = query("semantic.delta.cached", "textDocument/semanticTokens/full/delta", {
                    **document, "previousResultId": tokens["resultId"],
                })
                assert "edits" in delta or "data" in delta, delta

                # Invalidate diagnostics with an unresolved reference, then remove it next step.
                marker = sources[uri].index("REFERENCE-HERE")
                missing = "\\ref{missing:benchmark}\n"
                def change_reference():
                    if step % 2 == 0:
                        edit(uri, marker, marker, missing)
                    else:
                        edit(uri, marker - len(missing), marker, "")
                    fence()
                timed("edit.reference", change_reference, measured)
                report = query("diagnostic", "textDocument/diagnostic", document)
                assert report["kind"] == "full", report
                found = any("missing:benchmark" in item["message"] for item in report["items"])
                assert found == (step % 2 == 0), report
                cached = query("diagnostic.cached", "textDocument/diagnostic", {
                    **document, "previousResultId": report["resultId"],
                })
                assert cached["kind"] == "unchanged", cached

                # Eight notifications queued without a barrier simulate rapid typing.
                def burst():
                    for index in range(8):
                        marker = sources[uri].index("EDIT-HERE")
                        if index % 2 == 0:
                            edit(uri, marker, marker, "x")
                        else:
                            edit(uri, marker - 1, marker, "")
                    result = server.request("textDocument/completion", at("sec:0:0", 4))
                    assert result
                timed("burst8.to_completion", burst, measured)
        finally:
            server.close()
        return metadata


def summary(values):
    ordered = sorted(values)
    return {"count": len(values), "p50_ms": statistics.median(values),
            "p95_ms": ordered[math.ceil(len(values) * 0.95) - 1], "max_ms": max(values)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, default=Path("target/release/latex-lsp"))
    parser.add_argument("--scales", nargs="+", choices=SCALES, default=list(SCALES))
    parser.add_argument("--iterations", type=int, default=20)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--hooks", action="store_true", help="include an argument-taking kernel hook in the main document")
    parser.add_argument("--hook-wrappers", action="store_true", help="include nested hook wrappers and one call per section (implies --hooks)")
    args = parser.parse_args()
    args.hooks = args.hooks or args.hook_wrappers
    if args.iterations < 1 or args.runs < 1 or args.warmup < 0 or args.timeout <= 0:
        parser.error("iterations/runs/timeout must be positive; warmup must be nonnegative")
    binary = args.server.resolve(strict=True)
    baseline = json.loads(args.baseline.read_text()) if args.baseline else None
    output = {"schema": 1, "workload_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "environment": {
        "platform": platform.platform(), "cpu_count": os.cpu_count(),
        "python": platform.python_version(), "server": str(binary),
        "server_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    }, "parameters": {"iterations": args.iterations, "warmup": args.warmup, "runs": args.runs,
                      "hooks": args.hooks, "hook_wrappers": args.hook_wrappers},
        "scenarios": {}}
    if baseline and (baseline["parameters"] != output["parameters"]
                     or baseline["workload_sha256"] != output["workload_sha256"]):
        parser.error("baseline must use the same script, iterations, warmup, and runs")
    for scale in args.scales:
        samples = {}
        for run in range(args.runs):
            print(f"{scale}: run {run + 1}/{args.runs}", file=sys.stderr, flush=True)
            metadata = session(binary, scale, args.iterations, args.warmup, args.timeout,
                               lambda name, ms: samples.setdefault(name, []).append(ms), args.hooks, args.hook_wrappers)
        metrics = {name: summary(values) for name, values in samples.items()}
        output["scenarios"][scale] = {"fixture": metadata, "metrics": metrics, "samples_ms": samples}
        if baseline and baseline["scenarios"][scale]["fixture"] != metadata:
            raise RuntimeError(f"{scale}: baseline fixture differs")
        for name, metric in metrics.items():
            comparison = ""
            if baseline:
                before = baseline["scenarios"][scale]["metrics"][name]["p50_ms"]
                comparison = f"  {before / metric['p50_ms']:.2f}x"
            print(f"{scale:6} {name:24} p50 {metric['p50_ms']:9.3f} ms  "
                  f"p95 {metric['p95_ms']:9.3f} ms{comparison}", flush=True)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
