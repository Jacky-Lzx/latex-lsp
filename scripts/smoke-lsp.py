#!/usr/bin/env python3
"""Check the installed server's initialize/shutdown/exit lifecycle over stdio."""

import argparse
import json
import os
import select
import subprocess
import time


def frame(message):
    body = json.dumps(message).encode()
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    process = subprocess.Popen(
        [args.binary, "--stdio"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, bufsize=0,
    )
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
            if timeout <= 0 or not select.select([process.stdout], [], [], timeout)[0]:
                raise TimeoutError(f"No LSP response for request {request_id}")
            data = os.read(process.stdout.fileno(), 65536)
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
        _, stderr = process.communicate(timeout=10)
        if process.returncode:
            raise RuntimeError(f"LSP server exited {process.returncode}: {stderr.decode()}")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
    print(f"LSP lifecycle passed: texquill {args.version}")


if __name__ == "__main__":
    main()
