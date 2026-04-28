#!/usr/bin/env python3
"""Lifecycle-capable JSON-RPC channel plugin for the respawn contract
test. Unlike `fake_plugin.py` (which only serves the dry-run preflight
handshake), this one answers `initialize(dry_run=false)` + `start` +
`stop` + `dispatch_outbound` + `shutdown`, which is what
`ChannelPluginManager::register_subprocess_plugin` and `respawn_plugin`
drive.

Environment knobs:
  FAKE_PLUGIN_ID        — plugin id reported from initialize (default
                          "fake-lifecycle-plugin").
  FAKE_PLUGIN_LABEL     — free-form string baked into
                          dispatch_outbound message_ids so the test can
                          distinguish responses from OLD vs. NEW
                          subprocess incarnations.
  FAKE_HANG_ON_STOP_MS  — milliseconds to sleep inside `stop` before
                          answering (0 = answer immediately). Used to
                          drive the drain/escalation path.
"""
import json
import os
import sys
import time


PLUGIN_ID = os.environ.get("FAKE_PLUGIN_ID", "fake-lifecycle-plugin")
# Defaults to the PID so respawned children are distinguishable from
# their predecessors even when the host reuses the same spawn_options.
PLUGIN_LABEL = os.environ.get("FAKE_PLUGIN_LABEL", f"pid-{os.getpid()}")
PLUGIN_VERSION = "0.1.0"
PROTOCOL_VERSIONS = [1]
HANG_ON_STOP_MS = int(os.environ.get("FAKE_HANG_ON_STOP_MS", "0"))
# When set, initialize replies with JSON-RPC error `code`, message
# "forced by test". Used to exercise the host's InitializeRejected vs.
# LifecycleRpc split.
FAIL_INIT_CODE = os.environ.get("FAKE_FAIL_INIT_CODE")
# If set to a filesystem path that EXISTS at startup, initialize fails
# with `FAKE_FAIL_INIT_CODE` (or ConfigRejected if unset). Used by the
# respawn-failure test: it creates the file between register and
# respawn so the NEW child fails while the OLD child is already live.
FAIL_INIT_IF_FILE = os.environ.get("FAKE_FAIL_INIT_IF_FILE")
# When set, dispatch_outbound parks forever — used to simulate a
# straggler that outlasts `stop_grace_ms`.
HANG_DISPATCH = os.environ.get("FAKE_HANG_DISPATCH") == "1"


def read_frame(stream):
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


CAPS = {
    "outbound": True,
    "inbound": True,
    "streaming": False,
    "images": False,
    "files": False,
}

# Initialized accounts; stashed so the host's respawn test can probe
# which account set the NEW subprocess saw.
STATE = {"accounts": []}


def handle(req):
    method = req.get("method")
    req_id = req.get("id")
    if method == "initialize":
        params = req.get("params") or {}
        # Real lifecycle path: dry_run MUST be false for
        # register_subprocess_plugin / respawn_plugin.
        if params.get("dry_run") is True:
            return make_error(
                req_id,
                -32602,
                "fake-lifecycle plugin only answers non-dry-run initialize",
            )
        if FAIL_INIT_CODE is not None:
            return make_error(req_id, int(FAIL_INIT_CODE), "forced by test")
        if FAIL_INIT_IF_FILE is not None and os.path.exists(FAIL_INIT_IF_FILE):
            return make_error(req_id, -32005, "forced by test via trigger file")
        STATE["accounts"] = list(params.get("accounts") or [])
        return make_ok(
            req_id,
            {
                "plugin": {"id": PLUGIN_ID, "version": PLUGIN_VERSION},
                "capabilities": CAPS,
            },
        )
    if method == "start":
        return make_ok(req_id, {})
    if method == "stop":
        if HANG_ON_STOP_MS > 0:
            time.sleep(HANG_ON_STOP_MS / 1000.0)
        return make_ok(req_id, {})
    if method == "describe":
        return make_ok(
            req_id,
            {
                "plugin": {"id": PLUGIN_ID, "version": PLUGIN_VERSION},
                "protocol_versions": PROTOCOL_VERSIONS,
                "schema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                },
                "auth_flows": [],
                "capabilities": CAPS,
            },
        )
    if method == "dispatch_outbound":
        params = req.get("params") or {}
        if HANG_DISPATCH:
            # Never answer. The host-side drain will time out, and the
            # respawn path aborts stragglers with HostAborted.
            while True:
                time.sleep(60)
        # Emit a deterministic id that embeds both the plugin-label
        # (so the test distinguishes OLD vs. NEW subprocess) and the
        # caller's chat_id + account_id so replays are distinguishable.
        chat = params.get("chat_id", "")
        account = params.get("account_id", "")
        mid = f"{PLUGIN_LABEL}:{account}:{chat}"
        return make_ok(req_id, {"message_ids": [mid]})
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
            continue
        resp = handle(req)
        write_frame(stdout, resp)
        if req.get("method") == "shutdown":
            return


if __name__ == "__main__":
    main()
