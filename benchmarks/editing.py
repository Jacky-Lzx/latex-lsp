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
import re
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


def fixture(root, scale, hooks=False, hook_wrappers=False, hook_environments=False, hook_interfaces=False, hook_labels=False, lifecycle_labels=False, global_hook_rules=False, default_hook_labels=False, document_hook_slots=False, relative_hook_labels=False):
    chapters, sections = SCALES[scale]
    label_hook = "begindocument" if lifecycle_labels else "benchmark"
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
    if hook_environments:
        environments = "".join(
            f"\\NewDocumentEnvironment{{benchenv{i}}}{{m}}{{}}{{\\UseHook{{benchmark}}{{#1}}}}\n"
            for i in range(sections)
        )
        main = main.replace("\\begin{document}\n", environments + "\\begin{document}\n")
    if hook_interfaces:
        main = main.replace("\\begin{document}\n", (
            "\\newenvironment{benchinterface}{\\UseHook}{}\n\\begin{document}\n"
        ))
    if hook_labels:
        main = main.replace("\\begin{document}\n", (
            f"\\AddToHook{{{label_hook}}}[contribution]{{\\typeout{{Contribution}}}}\n"
            "\\NewHook{benchmark.other}\n"
            "\\AddToHook{benchmark.other}[contrast]{\\typeout{Other}}\n"
            "\\begin{document}\n"
        ))
    if document_hook_slots:
        main = main.replace("\\begin{document}\n", (
            f"\\AddToHook{{{label_hook}}}[top-level]{{\\typeout{{Document slot}}}}\n"
            "\\AddToHook{benchmark.other}[top-level]{\\typeout{Other slot}}\n"
            "\\begin{document}\n"
        ))
    if relative_hook_labels:
        main = main.replace("\\begin{document}\n", (
            f"\\AddToHook{{{label_hook}}}[relative/tail]{{\\typeout{{Relative explicit}}}}\n"
            "\\AddToHook{benchmark.other}[relative/tail]{\\typeout{Other relative}}\n"
            "\\begin{document}\n"
        ))
    if lifecycle_labels:
        main = main.replace("\\begin{document}\n", (
            "\\AtEndDocument[contrast]{\\typeout{End}}\n\\begin{document}\n"
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
            if hook_labels:
                source += ("\\AtBeginDocument[contribution]{\\typeout{Contribution}}\n"
                           if lifecycle_labels else
                           "\\AddToHook{benchmark}[contribution]{\\typeout{Contribution}}\n")
            if relative_hook_labels:
                relative_add = "\\AtBeginDocument" if lifecycle_labels else "\\AddToHook{benchmark}"
                source += ("\\PushDefaultHookLabel{relative}" + relative_add
                           + "[./tail]{\\typeout{Relative contribution}}\\PopDefaultHookLabel\n")
            if document_hook_slots:
                source += ("\\AtBeginDocument[top-level]{\\typeout{Document slot}}\n"
                           if lifecycle_labels else
                           "\\AddToHook{benchmark}[top-level]{\\typeout{Document slot}}\n")
                source += "\\AddToHook{benchmark.other}[top-level]{\\typeout{Other slot}}\n"
            if global_hook_rules:
                source += "\\AddToHook{benchmark.other}[contribution]{\\typeout{Global contribution}}\n"
            if default_hook_labels:
                source += ("\\PushDefaultHookLabel{fixture}\n"
                           "\\SetDefaultHookLabel{contribution}\n"
                           "\\AddToHook{benchmark.other}{\\typeout{Implicit contribution}}\n"
                           "\\PopDefaultHookLabel\n")
            if hook_wrappers:
                source += "\\benchforward{Literal data}\n"
            if hook_environments:
                source += f"\\begin{{benchenv{section}}}{{Literal data}}\\end{{benchenv{section}}}\n"
            if hook_interfaces:
                source += "\\benchinterface{benchmark}{Literal data}\n"
        # Stable query locations followed by a region edited throughout the session.
        source += ("\\ref{sec:0:0}\n\\cite{paper0}\n\\projectterm{value}\n"
                   "\\textbf{probe}\n中文 😀 EDIT-HERE\nREFERENCE-HERE\n")
        if hook_labels:
            source += (f"\\RemoveFromHook{{{label_hook}}}[contribution]\n"
                       f"\\RemoveFromHook{{{label_hook}}}[conTRIB]\n")
        if relative_hook_labels:
            source += (f"\\PushDefaultHookLabel{{relative}}\\RemoveFromHook{{{label_hook}}}[./tail]\\PopDefaultHookLabel\n"
                       f"\\PushDefaultHookLabel{{relative}}\\RemoveFromHook{{{label_hook}}}[./taFAKE]\\PopDefaultHookLabel\n")
        if document_hook_slots:
            source += (f"\\RemoveFromHook{{{label_hook}}}[top-level]\n"
                       f"\\RemoveFromHook{{{label_hook}}}[toFAKE]\n")
        if global_hook_rules:
            source += ("\\DeclareDefaultHookRule{contribution}{before}{contrast}\n"
                       "\\DeclareDefaultHookRule{conTRIB}{before}{contrast}\n")
        if default_hook_labels:
            source += ("\\PushDefaultHookLabel{fixture}\\SetDefaultHookLabel{contribution}\\PopDefaultHookLabel\n"
                       "\\PushDefaultHookLabel{fixture}\\SetDefaultHookLabel{conTRIB}\\PopDefaultHookLabel\n")
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


def hook_name_edits(sources, new_name="benchmark.renamed"):
    """Exact hook edits for this fixture, excluding its unrelated missing keys."""
    return {
        uri: [{"range": {"start": position(source, match.start() + 1),
                         "end": position(source, match.start() + 1 + len("benchmark"))},
               "newText": new_name}
              for match in re.finditer(r"\{benchmark\}", source)]
        for uri, source in sources.items() if "{benchmark}" in source
    }


def hook_label_locations(sources, declarations_only=True, lifecycle=False, global_rules=False):
    """Exact literal code-label ranges, independently of the server index."""
    commands = "AddToHook" if declarations_only else "(?:AddToHook|RemoveFromHook)"
    hook = "begindocument" if lifecycle else "benchmark"
    patterns = [re.compile(r"\\" + commands + r"\{" + hook + r"\}\[(contribution)\]")]
    if lifecycle:
        patterns.append(re.compile(r"\\AtBeginDocument\[(contribution)\]"))
    if global_rules and not declarations_only:
        patterns.append(re.compile(r"\\DeclareDefaultHookRule\{(contribution)\}"))
    if not declarations_only:
        patterns.append(re.compile(r"\\(?:Push|Set)DefaultHookLabel\{(contribution)\}"))
    return [{"uri": uri, "range": {"start": position(source, match.start(1)),
                                  "end": position(source, match.end(1))}}
            for uri, source in sorted(sources.items())
            for match in sorted((match for pattern in patterns for match in pattern.finditer(source)),
                                key=lambda match: match.start(1))]


def global_hook_label_locations(sources, declarations_only=True):
    """Explicit labels across fixture hooks and global ordering fields."""
    commands = "AddToHook" if declarations_only else "(?:AddToHook|RemoveFromHook)"
    patterns = [re.compile(r"\\" + commands + r"\{[^{}]+\}\[(contribution)\]"),
                re.compile(r"\\At(?:Begin|End)Document\[(contribution)\]")]
    if not declarations_only:
        patterns.append(re.compile(r"\\DeclareDefaultHookRule\{(contribution)\}"))
        patterns.append(re.compile(r"\\(?:Push|Set)DefaultHookLabel\{(contribution)\}"))
    return [{"uri": uri, "range": {"start": position(source, match.start(1)),
                                  "end": position(source, match.end(1))}}
            for uri, source in sorted(sources.items())
            for match in sorted((match for pattern in patterns for match in pattern.finditer(source)),
                                key=lambda match: match.start(1))]



def document_hook_slot_locations(sources, declarations_only=True, lifecycle=False):
    """Exact local document-slot ranges; other hooks and ordinary keys are excluded."""
    commands = "AddToHook" if declarations_only else "(?:AddToHook|RemoveFromHook)"
    hook = "begindocument" if lifecycle else "benchmark"
    patterns = [re.compile(r"\\" + commands + r"\{" + hook + r"\}\[(top-level)\]")]
    if lifecycle:
        patterns.append(re.compile(r"\\AtBeginDocument\[(top-level)\]"))
    return [{"uri": uri, "range": {"start": position(source, match.start(1)),
                                  "end": position(source, match.end(1))}}
            for uri, source in sorted(sources.items())
            for match in sorted((match for pattern in patterns for match in pattern.finditer(source)),
                                key=lambda match: match.start(1))]


def relative_hook_label_locations(sources, declarations_only=True, lifecycle=False):
    """Ranges of the fixture's one explicit label and adjacent Push relative fields."""
    hook = "begindocument" if lifecycle else "benchmark"
    patterns = [re.compile(r"\\AddToHook\{" + hook + r"\}\[(relative/tail)\]")]
    add = r"AtBeginDocument" if lifecycle else r"AddToHook\{" + hook + r"\}"
    commands = add if declarations_only else "(?:" + add + r"|RemoveFromHook\{" + hook + r"\})"
    patterns.append(re.compile(r"\\PushDefaultHookLabel\{relative\}\\" + commands + r"\[(\./tail)\]"))
    return [{"uri": uri, "range": {"start": position(source, match.start(1)),
                                  "end": position(source, match.end(1))}}
            for uri, source in sorted(sources.items())
            for match in sorted((match for pattern in patterns for match in pattern.finditer(source)),
                                key=lambda match: match.start(1))]

def session(binary, scale, iterations, warmup, timeout, record, hooks=False, hook_wrappers=False, hook_environments=False, hook_interfaces=False, hook_renames=False, hook_labels=False, lifecycle_labels=False, global_hook_rules=False, default_hook_labels=False, document_hook_slots=False, relative_hook_labels=False):
    with tempfile.TemporaryDirectory(prefix="latex-lsp-benchmark-") as directory:
        root = Path(directory)
        metadata = fixture(root, scale, hooks, hook_wrappers, hook_environments, hook_interfaces, hook_labels, lifecycle_labels, global_hook_rules, default_hook_labels, document_hook_slots, relative_hook_labels)
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
                if hook_labels:
                    params = at("[contribution]", 4)
                    definitions = query("definition.hook_label", "textDocument/definition", params)
                    assert definitions == hook_label_locations(sources, lifecycle=lifecycle_labels), definitions
                    references = query("references.hook_label", "textDocument/references", {
                        **params, "context": {"includeDeclaration": True},
                    })
                    assert references == hook_label_locations(sources, False, lifecycle_labels, global_hook_rules), references
                    hover = query("hover.hook_label", "textDocument/hover", params)
                    assert "contribution" in hover["contents"]["value"], hover
                    label_hook = "begindocument" if lifecycle_labels else "benchmark"
                    assert f"Hook: {label_hook}" in hover["contents"]["value"], hover
                    items = query("completion.hook_label", "textDocument/completion", at("[conTRIB]", 4))
                    assert len(items) == 1 and items[0]["label"] == "contribution", items
                    start = sources[uri].rindex("[conTRIB]") + 1
                    assert items[0]["textEdit"] == {"range": {
                        "start": position(sources[uri], start),
                        "end": position(sources[uri], start + len("conTRIB"))},
                        "newText": "contribution"}, items
                if global_hook_rules:
                    params = at("{contribution}{before}", 4)
                    definitions = query("definition.global_hook_label", "textDocument/definition", params)
                    assert definitions == global_hook_label_locations(sources), definitions
                    references = query("references.global_hook_label", "textDocument/references", {
                        **params, "context": {"includeDeclaration": True},
                    })
                    assert references == global_hook_label_locations(sources, False), references
                    hover = query("hover.global_hook_label", "textDocument/hover", params)
                    assert "All hooks (default ordering rule)" in hover["contents"]["value"], hover
                    items = query("completion.global_hook_label", "textDocument/completion", at("{conTRIB}{before}", 4))
                    assert [item["label"] for item in items] == ["contrast", "contribution"], items
                    start = sources[uri].rindex("{conTRIB}{before}") + 1
                    for item in items:
                        assert item["textEdit"] == {"range": {
                            "start": position(sources[uri], start),
                            "end": position(sources[uri], start + len("conTRIB"))},
                            "newText": item["label"]}, items
                if default_hook_labels:
                    params = at("\\SetDefaultHookLabel{contribution}", len("\\SetDefaultHookLabel{") + 3)
                    definitions = query("definition.default_hook_label", "textDocument/definition", params)
                    # The current feature links explicit registrations only;
                    # never count omitted registrations as certified results.
                    assert definitions == global_hook_label_locations(sources), definitions
                    references = query("references.default_hook_label", "textDocument/references", {
                        **params, "context": {"includeDeclaration": True},
                    })
                    assert references == global_hook_label_locations(sources, False), references
                    hover = query("hover.default_hook_label", "textDocument/hover", params)
                    assert "All hooks (default code label)" in hover["contents"]["value"], hover
                    needle = "\\SetDefaultHookLabel{conTRIB}"
                    items = query("completion.default_hook_label", "textDocument/completion", at(needle, len("\\SetDefaultHookLabel{") + 3))
                    assert [item["label"] for item in items] == ["contrast", "contribution"], items
                    start = sources[uri].rindex(needle) + len("\\SetDefaultHookLabel{")
                    for item in items:
                        assert item["textEdit"] == {"range": {
                            "start": position(sources[uri], start),
                            "end": position(sources[uri], start + len("conTRIB"))},
                            "newText": item["label"]}, items
                if document_hook_slots:
                    params = at("[top-level]", 4)
                    definitions = query("definition.document_hook_slot", "textDocument/definition", params)
                    assert definitions == document_hook_slot_locations(sources, lifecycle=lifecycle_labels), definitions
                    references = query("references.document_hook_slot", "textDocument/references", {
                        **params, "context": {"includeDeclaration": True},
                    })
                    assert references == document_hook_slot_locations(sources, False, lifecycle_labels), references
                    hover = query("hover.document_hook_slot", "textDocument/hover", params)
                    assert "Document-level code slot" in hover["contents"]["value"], hover
                    label_hook = "begindocument" if lifecycle_labels else "benchmark"
                    assert f"Hook: {label_hook}" in hover["contents"]["value"], hover
                    items = query("completion.document_hook_slot", "textDocument/completion", at("[toFAKE]", 3))
                    assert [item["label"] for item in items] == ["top-level"], items
                    start = sources[uri].rindex("[toFAKE]") + 1
                    assert items[0]["textEdit"] == {"range": {
                        "start": position(sources[uri], start),
                        "end": position(sources[uri], start + len("toFAKE"))},
                        "newText": "top-level"}, items
                if relative_hook_labels:
                    params = at("[./tail]", 4)
                    definitions = query("definition.relative_hook_label", "textDocument/definition", params)
                    assert definitions == relative_hook_label_locations(sources, lifecycle=lifecycle_labels), definitions
                    references = query("references.relative_hook_label", "textDocument/references", {
                        **params, "context": {"includeDeclaration": True},
                    })
                    assert references == relative_hook_label_locations(sources, False, lifecycle_labels), references
                    hover = query("hover.relative_hook_label", "textDocument/hover", params)
                    assert "relative/tail" in hover["contents"]["value"], hover
                    label_hook = "begindocument" if lifecycle_labels else "benchmark"
                    assert f"Hook: {label_hook}" in hover["contents"]["value"], hover
                    items = query("completion.relative_hook_label", "textDocument/completion", at("[./taFAKE]", 5))
                    assert [item["label"] for item in items] == ["./tail"], items
                    start = sources[uri].rindex("[./taFAKE]") + 1
                    assert items[0]["textEdit"] == {"range": {
                        "start": position(sources[uri], start),
                        "end": position(sources[uri], start + len("./taFAKE"))},
                        "newText": "./tail"}, items
                if hook_renames:
                    hook_source = sources[main_uri]
                    result = query("rename.hook", "textDocument/rename", {
                        "textDocument": {"uri": main_uri},
                        "position": position(hook_source, hook_source.index("{benchmark}") + 2),
                        "newName": "benchmark.renamed",
                    })
                    assert result == {"changes": hook_name_edits(sources)}, result
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
    parser.add_argument("--hook-environments", action="store_true", help="include xparse environment variants passing their arguments to a hook in end code (implies --hooks)")
    parser.add_argument("--hook-interfaces", action="store_true", help="directly call an environment-generated control forwarding a hook once per section (implies --hooks)")
    parser.add_argument("--hook-renames", action="store_true", help="measure and verify every custom hook name edit (implies --hooks)")
    parser.add_argument("--hook-labels", action="store_true", help="verify explicit hook code-label definitions, references, hover, and suffix completion (implies --hooks)")
    parser.add_argument("--lifecycle-labels", action="store_true", help="mix AtBeginDocument code labels with direct registration and AtEndDocument interference (implies --hook-labels)")
    parser.add_argument("--global-hook-rules", action="store_true", help="verify global default-rule labels across multiple hooks (implies --hook-labels)")
    parser.add_argument("--default-hook-labels", action="store_true", help="verify literal default-label settings across explicit hook registrations (implies --hook-labels)")
    parser.add_argument("--document-hook-slots", action="store_true", help="verify explicit document-level hook slots independently of rules and default settings (implies --hook-labels)")
    parser.add_argument("--relative-hook-labels", action="store_true", help="verify explicit relative labels following an adjacent literal Push (implies --hook-labels)")
    args = parser.parse_args()
    args.hook_labels = args.hook_labels or args.lifecycle_labels or args.global_hook_rules or args.default_hook_labels or args.document_hook_slots or args.relative_hook_labels
    args.hooks = args.hooks or args.hook_wrappers or args.hook_environments or args.hook_interfaces or args.hook_renames or args.hook_labels
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
                      "hooks": args.hooks, "hook_wrappers": args.hook_wrappers,
                      "hook_environments": args.hook_environments,
                      "hook_interfaces": args.hook_interfaces, "hook_renames": args.hook_renames,
                      "hook_labels": args.hook_labels, "lifecycle_labels": args.lifecycle_labels,
                      "global_hook_rules": args.global_hook_rules,
                      "default_hook_labels": args.default_hook_labels,
                      "document_hook_slots": args.document_hook_slots,
                      "relative_hook_labels": args.relative_hook_labels},
        "scenarios": {}}
    if baseline and (baseline["parameters"] != output["parameters"]
                     or baseline["workload_sha256"] != output["workload_sha256"]):
        parser.error("baseline must use the same script, iterations, warmup, and runs")
    for scale in args.scales:
        samples = {}
        for run in range(args.runs):
            print(f"{scale}: run {run + 1}/{args.runs}", file=sys.stderr, flush=True)
            metadata = session(binary, scale, args.iterations, args.warmup, args.timeout,
                               lambda name, ms: samples.setdefault(name, []).append(ms), args.hooks, args.hook_wrappers, args.hook_environments, args.hook_interfaces, args.hook_renames, args.hook_labels, args.lifecycle_labels, args.global_hook_rules, args.default_hook_labels, args.document_hook_slots, args.relative_hook_labels)
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
