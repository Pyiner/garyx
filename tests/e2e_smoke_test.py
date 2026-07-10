#!/usr/bin/env python3
"""
End-to-end HTTP smoke test for Garyx gateway.

Usage:
    python3 tests/e2e_smoke_test.py [--base-url http://localhost:31337]

Requires: Python 3.10+ (stdlib only, no pip dependencies).
The gateway must be running at the given base URL before you start.
"""

import argparse
import json
import sys
import urllib.request
import urllib.error
import uuid
from typing import Optional

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

BASE_URL = "http://localhost:31337"
HTTP_TIMEOUT = 15  # seconds
PASS_COUNT = 0
FAIL_COUNT = 0


def _req(method: str, path: str, body=None, headers=None, expected_status=None):
    """Fire an HTTP request and return (status, parsed_json | raw_bytes)."""
    url = f"{BASE_URL}{path}"
    data = json.dumps(body).encode() if body is not None else None
    hdrs = {"Content-Type": "application/json", "Accept": "application/json"}
    if headers:
        hdrs.update(headers)
    req = urllib.request.Request(url, data=data, headers=hdrs, method=method)
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT) as resp:
            raw = resp.read()
            status = resp.status
    except urllib.error.HTTPError as e:
        raw = e.read()
        status = e.code

    try:
        parsed = json.loads(raw) if raw else None
    except (json.JSONDecodeError, ValueError):
        parsed = raw

    if expected_status is not None and status != expected_status:
        raise AssertionError(
            f"Expected HTTP {expected_status}, got {status}. Body: {parsed}"
        )
    return status, parsed


class McpSession:
    """Manages an MCP Streamable HTTP session with proper handshake."""

    def __init__(self, base_url: str):
        self.mcp_url = f"{base_url}/mcp"
        self.session_id: Optional[str] = None
        self._initialized = False

    def _post(self, payload: dict, extra_headers: Optional[dict] = None) -> tuple:
        """POST to /mcp with session headers; returns (status, parsed, resp_headers)."""
        hdrs = {
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
        }
        if self.session_id:
            hdrs["Mcp-Session-Id"] = self.session_id
        if extra_headers:
            hdrs.update(extra_headers)

        data = json.dumps(payload).encode()
        req = urllib.request.Request(self.mcp_url, data=data, headers=hdrs, method="POST")
        resp_headers = {}
        try:
            with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT) as resp:
                raw = resp.read()
                status = resp.status
                resp_headers = dict(resp.headers)
        except urllib.error.HTTPError as e:
            raw = e.read()
            status = e.code
            resp_headers = dict(e.headers)

        parsed = self._parse_response(raw, resp_headers.get("Content-Type", ""))
        return status, parsed, resp_headers

    @staticmethod
    def _parse_response(raw: bytes, content_type: str):
        """Parse JSON or SSE response."""
        if "text/event-stream" in content_type:
            for line in raw.decode("utf-8", errors="replace").splitlines():
                if line.startswith("data:"):
                    payload = line[len("data:"):].strip()
                    if payload:
                        try:
                            return json.loads(payload)
                        except (json.JSONDecodeError, ValueError):
                            continue
            return None
        try:
            return json.loads(raw) if raw else None
        except (json.JSONDecodeError, ValueError):
            return raw

    def initialize(self):
        """Perform MCP initialize + notifications/initialized handshake."""
        if self._initialized:
            return
        # Step 1: initialize
        status, parsed, headers = self._post({
            "jsonrpc": "2.0",
            "id": str(uuid.uuid4()),
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "e2e-smoke-test", "version": "0.1.0"},
            },
        })
        if status != 200:
            raise AssertionError(f"MCP initialize failed: HTTP {status}")

        # Extract session id (case-insensitive header)
        for k, v in headers.items():
            if k.lower() == "mcp-session-id":
                self.session_id = v
                break

        # Step 2: notifications/initialized
        self._post({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        })
        self._initialized = True

    def call_tool(self, tool_name: str, arguments: dict, extra_headers: Optional[dict] = None):
        """Call an MCP tool; returns the parsed tool result content."""
        self.initialize()

        rpc_body = {
            "jsonrpc": "2.0",
            "id": str(uuid.uuid4()),
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments,
            },
        }
        status, parsed, _ = self._post(rpc_body, extra_headers)
        if status != 200:
            raise AssertionError(f"MCP tools/call HTTP {status}")

        if parsed and isinstance(parsed, dict):
            if "error" in parsed:
                raise AssertionError(f"MCP error: {parsed['error']}")
            if "result" in parsed and "content" in parsed["result"]:
                for item in parsed["result"]["content"]:
                    if item.get("type") == "text":
                        try:
                            return json.loads(item["text"])
                        except (json.JSONDecodeError, ValueError):
                            return item["text"]
        return parsed

    def list_tools(self):
        """Return the MCP tool definitions advertised by the server."""
        self.initialize()
        status, parsed, _ = self._post({
            "jsonrpc": "2.0",
            "id": str(uuid.uuid4()),
            "method": "tools/list",
        })
        if status != 200:
            raise AssertionError(f"MCP tools/list HTTP {status}")
        if parsed and isinstance(parsed, dict):
            if "error" in parsed:
                raise AssertionError(f"MCP error: {parsed['error']}")
            return parsed.get("result", {}).get("tools", [])
        return []


# Global MCP session (lazy-initialized on first tool call)
_mcp_session: Optional[McpSession] = None


def get_mcp_session() -> McpSession:
    global _mcp_session
    if _mcp_session is None:
        _mcp_session = McpSession(BASE_URL)
    return _mcp_session


def mcp_call(tool_name: str, arguments: dict, thread_id: Optional[str] = None):
    """Call an MCP tool via the shared session."""
    extra = {}
    if thread_id:
        extra["X-Thread-Id"] = thread_id
    return get_mcp_session().call_tool(tool_name, arguments, extra_headers=extra or None)


def check(label: str, condition: bool, detail: str = ""):
    global PASS_COUNT, FAIL_COUNT
    if condition:
        PASS_COUNT += 1
        print(f"  PASS  {label}")
    else:
        FAIL_COUNT += 1
        msg = f"  FAIL  {label}"
        if detail:
            msg += f"  ({detail})"
        print(msg)


# ---------------------------------------------------------------------------
# 1. MCP public tool surface
# ---------------------------------------------------------------------------

def test_mcp_tool_surface():
    print("\n=== 1. MCP public tool surface ===")
    try:
        tools = get_mcp_session().list_tools()
        tool_names = {
            tool.get("name")
            for tool in tools
            if isinstance(tool, dict) and isinstance(tool.get("name"), str)
        }
        check("tools/list exposes status", "status" in tool_names)

    except Exception as e:
        check("MCP public tool surface unexpected error", False, str(e))


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    global BASE_URL, _mcp_session
    parser = argparse.ArgumentParser(description="Garyx E2E smoke test")
    parser.add_argument("--base-url", default="http://localhost:31337",
                        help="Gateway base URL (default: http://localhost:31337)")
    args = parser.parse_args()
    BASE_URL = args.base_url.rstrip("/")
    _mcp_session = None  # reset so it picks up new BASE_URL

    print(f"Garyx E2E Smoke Test -- target: {BASE_URL}")
    print("=" * 60)

    # Quick connectivity check
    try:
        _req("GET", "/api/status")
    except Exception as e:
        print(f"\nFAIL: Cannot reach gateway at {BASE_URL}: {e}")
        print("   Make sure Garyx is running and try again.")
        sys.exit(1)

    test_mcp_tool_surface()

    print("\n" + "=" * 60)
    print(f"Results: {PASS_COUNT} passed, {FAIL_COUNT} failed")
    if FAIL_COUNT > 0:
        print("SOME TESTS FAILED")
        sys.exit(1)
    else:
        print("ALL TESTS PASSED")
        sys.exit(0)


if __name__ == "__main__":
    main()
