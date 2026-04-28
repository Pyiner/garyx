#!/usr/bin/env python3
"""Minimal JSON-RPC channel plugin used by Rust integration tests.

Speaks the LSP-style `Content-Length:` framed JSON-RPC 2.0 protocol
documented by the subprocess plugin tests
`initialize`, `describe`, and `shutdown`; rejects everything else so
tests notice when the host asks for something unexpected.
"""
import json
import os
import sys


PLUGIN_ID = os.environ.get("FAKE_PLUGIN_ID", "fake-plugin")
PLUGIN_VERSION = "0.1.0"
PROTOCOL_VERSIONS = [1]


def read_frame(stream):
    """Read one Content-Length framed JSON-RPC payload from `stream`.

    Returns the decoded dict, or None on clean EOF.
    """
    headers = {}
    while True:
        line = stream.readline()
        if not line:
            return None
        line = line.rstrip(b"\r\n")
        if not line:
            break
        if b":" not in line:
            continue
        key, _, value = line.partition(b":")
        headers[key.strip().lower()] = value.strip()
    length = int(headers.get(b"content-length", b"0"))
    body = b""
    remaining = length
    while remaining > 0:
        chunk = stream.read(remaining)
        if not chunk:
            return None
        body += chunk
        remaining -= len(chunk)
    return json.loads(body.decode("utf-8"))


def write_frame(stream, obj):
    body = json.dumps(obj, separators=(",", ":")).encode("utf-8")
    header = f"Content-Length: {len(body)}\r\n\r\n".encode("ascii")
    stream.write(header)
    stream.write(body)
    stream.flush()


def make_error(req_id, code, message):
    return {
        "jsonrpc": "2.0",
        "id": req_id,
        "error": {"code": code, "message": message},
    }


def make_ok(req_id, result):
    return {"jsonrpc": "2.0", "id": req_id, "result": result}


def handle(req):
    method = req.get("method")
    req_id = req.get("id")
    if method == "initialize":
        # Host contract per §6.3a of the protocol doc: for preflight
        # (dry_run) the host MUST pass dry_run=true and an empty
        # accounts list. The fake plugin fails preflight if the host
        # regresses on that, which lets `preflight_contract.rs` catch
        # it.
        params = req.get("params") or {}
        if params.get("dry_run") is not True:
            return make_error(
                req_id,
                -32602,
                "fake plugin requires dry_run=true during preflight",
            )
        if params.get("accounts", []) != []:
            return make_error(
                req_id,
                -32602,
                "fake plugin requires empty accounts during preflight",
            )
        return make_ok(
            req_id,
            {
                "plugin": {"id": PLUGIN_ID, "version": PLUGIN_VERSION},
                "capabilities": {
                    "outbound": True,
                    "inbound": True,
                    "streaming": False,
                    "images": False,
                    "files": False,
                },
            },
        )
    if method == "describe":
        return make_ok(
            req_id,
            {
                "plugin": {"id": PLUGIN_ID, "version": PLUGIN_VERSION},
                "protocol_versions": PROTOCOL_VERSIONS,
                "schema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "required": ["token"],
                    "properties": {"token": {"type": "string"}},
                },
                "auth_flows": [
                    {"id": "device_code", "label": "Device code", "prompt": "opens browser"}
                ],
                "capabilities": {
                    "outbound": True,
                    "inbound": True,
                    "streaming": False,
                    "images": False,
                    "files": False,
                },
            },
        )
    if method == "shutdown":
        return make_ok(req_id, {})
    return make_error(req_id, -32601, f"method not found: {method}")


def main():
    stdin = sys.stdin.buffer
    stdout = sys.stdout.buffer
    while True:
        req = read_frame(stdin)
        if req is None:
            return
        if req.get("method") is None or req.get("id") is None:
            # Notification or malformed: ignore silently.
            continue
        resp = handle(req)
        write_frame(stdout, resp)
        if req.get("method") == "shutdown":
            return


if __name__ == "__main__":
    main()
