#!/usr/bin/env python3
"""Check the installed server's initialize/shutdown/exit lifecycle over stdio."""

import argparse
import json
import queue
import subprocess
import tempfile
import threading
import time


def frame(message):
    body = json.dumps(message).encode()
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    stderr_file = tempfile.TemporaryFile()
    process = subprocess.Popen(
        [args.binary, "--stdio"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=stderr_file, bufsize=0,
    )
    chunks = queue.Queue()

    # Windows select() only accepts sockets, not subprocess pipes.
    def read_stdout():
        try:
            while True:
                data = process.stdout.read(65536)
                chunks.put(data)
                if not data:
                    return
        except OSError as error:
            chunks.put(error)

    reader = threading.Thread(target=read_stdout, daemon=True)
    reader.start()
    pending = b""

    def send(message):
        process.stdin.write(frame(message))
        process.stdin.flush()

    def receive(request_id):
        nonlocal pending
        deadline = time.monotonic() + 30
        while True:
            if b"\r\n\r\n" in pending:
                headers, body = pending.split(b"\r\n\r\n", 1)
                length = next(int(line.split(b":", 1)[1]) for line in headers.split(b"\r\n")
                              if line.lower().startswith(b"content-length:"))
                if len(body) >= length:
                    pending = body[length:]
                    message = json.loads(body[:length])
                    if message.get("id") == request_id:
                        return message["result"]
                    continue
            timeout = deadline - time.monotonic()
            if timeout <= 0:
                raise TimeoutError(f"No LSP response for request {request_id}")
            try:
                data = chunks.get(timeout=timeout)
            except queue.Empty:
                raise TimeoutError(f"No LSP response for request {request_id}")
            if isinstance(data, OSError):
                raise data
            if not data:
                raise RuntimeError("LSP server closed stdout before responding")
            pending += data

    try:
        send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "processId": None, "rootUri": None, "capabilities": {},
        }})
        initialization = receive(1)
        assert initialization["serverInfo"] == {"name": "texquill", "version": args.version}
        assert initialization["capabilities"]["completionProvider"]
        send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
        send({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": None})
        assert receive(2) is None
        send({"jsonrpc": "2.0", "method": "exit"})
        process.stdin.close()
        process.wait(timeout=10)
        if process.returncode:
            stderr_file.seek(0)
            raise RuntimeError(f"LSP server exited {process.returncode}: {stderr_file.read().decode()}")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        reader.join(timeout=5)
        process.stdout.close()
        if not process.stdin.closed:
            process.stdin.close()
        stderr_file.close()
    print(f"LSP lifecycle passed: texquill {args.version}")


if __name__ == "__main__":
    main()
