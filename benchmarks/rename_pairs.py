#!/usr/bin/env python3
"""Compare validated rename requests in alternating resident release processes."""

import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import sys
import tempfile
import time

import editing


def cpu_seconds(process):
    if not Path("/proc/self/stat").exists():
        return None
    stat = Path(f"/proc/{process.pid}/stat").read_text()
    fields = stat[stat.rfind(")") + 1:].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


def replay(before, after, scale, args, cpu):
    wall = {"before": [], "after": []}
    processor = {"before": [], "after": []}
    for run in range(args.runs):
        with tempfile.TemporaryDirectory(prefix="latex-lsp-rename-pairs-") as directory, ExitStack() as stack:
            root = Path(directory)
            launchers = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix="latex-lsp-rename-launch-")))
            metadata = editing.fixture(root, scale, args.hooks, args.hook_wrappers)
            main, chapter = root / "main.tex", root / "chapter0.tex"
            source = chapter.read_text()
            params = {"textDocument": {"uri": chapter.as_uri()},
                      "position": editing.position(source, source.rindex("sec:0:0") + 4),
                      "newName": "sec:renamed"}
            chapters, sections = editing.SCALES[scale]
            expected_files = {(root / f"chapter{i}.tex").as_uri() for i in range(chapters)}
            expected_edits = chapters * (sections + 1) + 1
            servers = {}
            creation_order = [("before", before), ("after", after)]
            if run % 2:
                creation_order.reverse()
            for label, binary in creation_order:
                launch = binary
                if cpu is not None:
                    # Set affinity before Tokio can create its worker threads.
                    launch = launchers / label
                    launch.write_text(
                        f"#!{sys.executable}\nimport os, sys\n"
                        f"os.sched_setaffinity(0, {{{cpu}}})\n"
                        f"os.execv({str(binary)!r}, [{str(binary)!r}] + sys.argv[1:])\n"
                    )
                    launch.chmod(0o700)
                server = editing.Server(launch, root, args.timeout)
                stack.callback(server.close)
                servers[label] = server
                server.request("initialize", {
                    "processId": None, "rootUri": root.as_uri(),
                    "capabilities": {"textDocument": {"diagnostic": {}}},
                    "initializationOptions": {"distribution": {"enabled": False}},
                })
                server.notify("initialized", {})
                for path in [main, chapter]:
                    server.notify("textDocument/didOpen", {"textDocument": {
                        "uri": path.as_uri(), "languageId": "latex", "version": 1,
                        "text": path.read_text(),
                    }})
                status = server.request("workspace/executeCommand", {"command": "latex.indexingStatus"})
                assert status["files"] == metadata["files"] and not status["truncated"], status
                if cpu is not None:
                    for task in Path(f"/proc/{server.process.pid}/task").iterdir():
                        assert os.sched_getaffinity(int(task.name)) == {cpu}, task
            for step in range(args.warmup + args.iterations):
                order = ["before", "after"] if (run + step) % 2 == 0 else ["after", "before"]
                for label in order:
                    server = servers[label]
                    start_cpu = cpu_seconds(server.process)
                    start = time.perf_counter_ns()
                    result = server.request("textDocument/rename", params)
                    elapsed = (time.perf_counter_ns() - start) / 1e6
                    end_cpu = cpu_seconds(server.process)
                    changes = result["changes"]
                    assert set(changes) == expected_files, changes
                    assert sum(map(len, changes.values())) == expected_edits, changes
                    assert all(edit["newText"] == "sec:renamed"
                               for edits in changes.values() for edit in edits), changes
                    if step >= args.warmup:
                        wall[label].append(elapsed)
                        if start_cpu is not None and end_cpu is not None:
                            processor[label].append((end_cpu - start_cpu) * 1000)
    return {
        "fixture": metadata,
        "wall_ms": {label: editing.summary(values) for label, values in wall.items()},
        "cpu_mean_ms": {label: statistics.mean(values) if values else None
                        for label, values in processor.items()},
        "paired_before_over_after_median": statistics.median(
            a / b for a, b in zip(wall["before"], wall["after"])),
        "wall_samples_ms": wall, "cpu_samples_ms": processor,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--scales", nargs="+", choices=editing.SCALES, default=list(editing.SCALES))
    parser.add_argument("--iterations", type=int, default=20)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--cpu", type=int, help="pin both servers to this allowed CPU (Linux default: first allowed CPU)")
    parser.add_argument("--hooks", action="store_true")
    parser.add_argument("--hook-wrappers", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.iterations < 1 or args.runs < 1 or args.warmup < 0 or args.timeout <= 0:
        parser.error("iterations/runs/timeout must be positive; warmup must be nonnegative")
    affinity = os.sched_getaffinity(0) if hasattr(os, "sched_getaffinity") else set()
    if args.cpu is not None and args.cpu not in affinity:
        parser.error("cpu must belong to the current allowed CPU set")
    cpu = args.cpu if args.cpu is not None else min(affinity, default=None)
    args.hooks = args.hooks or args.hook_wrappers
    before, after = args.before.resolve(strict=True), args.after.resolve(strict=True)
    report = {
        "schema": 1, "kind": "paired_rename",
        "workload_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "fixture_script_sha256": hashlib.sha256(Path(editing.__file__).read_bytes()).hexdigest(),
        "environment": {"platform": platform.platform(), "python": platform.python_version(),
                        "cpu": cpu, "affinity_scope": "all server threads (set before exec, verified after open)" if cpu is not None else None,
                        "cpu_tick_ms": 1000 / os.sysconf("SC_CLK_TCK") if Path("/proc/self/stat").exists() else None},
        "servers": {label: {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
                    for label, binary in [("before", before), ("after", after)]},
        "parameters": {"iterations": args.iterations, "warmup": args.warmup, "runs": args.runs,
                       "hooks": args.hooks, "hook_wrappers": args.hook_wrappers},
        "scenarios": {},
    }
    for scale in args.scales:
        result = replay(before, after, scale, args, cpu)
        report["scenarios"][scale] = result
        print(f"{scale}: wall p50 {result['wall_ms']['before']['p50_ms']:.3f} -> "
              f"{result['wall_ms']['after']['p50_ms']:.3f} ms; "
              f"paired median {result['paired_before_over_after_median']:.2f}x; "
              f"server CPU means {result['cpu_mean_ms']}", flush=True)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
