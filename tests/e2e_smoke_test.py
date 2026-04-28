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
import os
import subprocess
import sys
import tempfile
import time
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
# 1. Team CRUD
# ---------------------------------------------------------------------------

def test_team_crud():
    print("\n=== 1. Team CRUD ===")
    tid = f"smoke-test-{uuid.uuid4().hex[:8]}"
    payload = {
        "team_id": tid,
        "display_name": "Smoke Test Team",
        "leader_agent_id": "agent-a",
        "member_agent_ids": ["agent-a", "agent-b"],
        "workflow_text": "Plan -> Execute -> Review",
    }

    try:
        # CREATE
        status, body = _req("POST", "/api/teams", payload)
        check("Create team -> 201", status == 201, f"got {status}")
        check("Create team returns team_id", body and body.get("team_id") == tid)

        # READ
        status, body = _req("GET", f"/api/teams/{tid}")
        check("Get team -> 200", status == 200, f"got {status}")
        check("Get team display_name matches", body and body.get("display_name") == "Smoke Test Team")

        # UPDATE
        payload["display_name"] = "Updated Smoke Team"
        status, body = _req("PUT", f"/api/teams/{tid}", payload)
        check("Update team -> 200", status == 200, f"got {status}")
        check("Update team name reflected", body and body.get("display_name") == "Updated Smoke Team")

        # Verify update persisted
        status, body = _req("GET", f"/api/teams/{tid}")
        check("Re-read after update shows new name", body and body.get("display_name") == "Updated Smoke Team")

        # DELETE
        status, _ = _req("DELETE", f"/api/teams/{tid}")
        check("Delete team -> 204", status == 204, f"got {status}")

        # Verify gone
        status, _ = _req("GET", f"/api/teams/{tid}")
        check("Get after delete -> 404", status == 404, f"got {status}")
    except Exception as e:
        check("Team CRUD unexpected error", False, str(e))
    finally:
        # Best-effort cleanup in case test failed mid-way
        try:
            _req("DELETE", f"/api/teams/{tid}")
        except Exception:
            pass

    return tid


# ---------------------------------------------------------------------------
# 2. MCP public tool surface
# ---------------------------------------------------------------------------

def test_mcp_tool_surface():
    print("\n=== 2. MCP public tool surface ===")
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
# 3. AgentTeam provider E2E chat flow
# ---------------------------------------------------------------------------
#
# Exercises the AgentTeam provider flow:
#
#   - create a team from two existing standalone agents,
#   - create a thread bound to that team (provider resolves to agent_team),
#   - send a default-routed user message (goes to leader),
#   - send an `@[Coder](<coder_id>)` message (fans out to coder),
#   - verify group transcript shape, per-child catch-up slices, and that
#     the workspace_dir is identical across group + leader-child +
#     coder-child threads.
#
# Notes / dev-environment caveats:
#
#   - The gateway at --base-url must have >=2 standalone custom agents
#     registered. If fewer, the test prints a SKIP line and returns
#     cleanly (it does NOT add to FAIL_COUNT). Builtin agents include
#     `claude`, `codex`, `gemini` if their providers are configured.
#   - Chat dispatch is driven via the `garyx` CLI (`garyx thread send
#     --json`), which reuses the gateway's WebSocket path internally. By
#     default we call the `garyx` binary on PATH; override with the
#     `GARYX_BIN` env var (e.g. `GARYX_BIN=target/release/garyx`). If
#     the binary is missing or the subprocess times out, the whole
#     chat-flow test SKIPs rather than FAILs (same policy as the
#     <2-standalone-agents case).
#   - If the only standalone agents available are ones that can't run
#     headlessly in this environment (e.g. `claude_code` without an API
#     key, or `gemini` without OAuth), the CLI will hang and eventually
#     hit the timeout. That's a host-config issue, not a test bug —
#     rerun with agents whose providers are known working.


class CliSkip(Exception):
    """Raised when the CLI is unusable (missing / timed out) and the
    whole AgentTeam chat-flow test should SKIP rather than FAIL."""


def _pick_seed_agent(agents: list) -> Optional[dict]:
    """Pick one standalone leaf agent whose provider is most likely to work
    headlessly for deterministic temporary-agent smoke tests."""
    leaves = [
        agent for agent in agents
        if isinstance(agent, dict)
        and agent.get("standalone", True) is True
        and isinstance(agent.get("agent_id"), str)
        and agent.get("provider_type") != "agent_team"
    ]
    if not leaves:
        return None

    provider_rank = {
        "claude_code": 0,
        "codex_app_server": 1,
        "gemini": 2,
        "gemini_cli": 3,
        "openai_responses": 4,
    }
    leaves.sort(
        key=lambda agent: (
            provider_rank.get(str(agent.get("provider_type") or ""), 99),
            str(agent.get("display_name") or agent.get("agent_id") or ""),
        )
    )
    return leaves[0]


def _create_temp_custom_agent(
    agent_id: str,
    display_name: str,
    provider_type: str,
    model: str,
    system_prompt: str,
) -> bool:
    status, body = _req("POST", "/api/custom-agents", {
        "agent_id": agent_id,
        "display_name": display_name,
        "provider_type": provider_type,
        "model": model,
        "system_prompt": system_prompt,
    })
    check(f"Create temp agent {agent_id} -> 201", status == 201,
          f"status={status} body={body!r}")
    return status == 201


def _delete_custom_agent(agent_id: str):
    try:
        _req("DELETE", f"/api/custom-agents/{agent_id}")
    except Exception:
        pass


def _cli_thread_send(thread_id: str, message: str, workspace_dir: str,
                     timeout_secs: int = 60) -> list:
    """Drive `garyx thread send --json` and return the list of parsed
    event dicts printed on stdout (one JSON object per line).

    Env:
      GARYX_BIN — path to the garyx binary (default: "garyx" on PATH).

    Raises CliSkip if the binary is missing or the subprocess times out.
    """
    garyx_bin = os.environ.get("GARYX_BIN", "garyx")
    source_config = os.environ.get("GARYX_CONFIG", os.path.expanduser("~/.garyx/garyx.json"))
    temp_config_path = None
    try:
        try:
            with open(source_config, "r", encoding="utf-8") as fh:
                config = json.load(fh)
        except Exception:
            config = {}
        if not isinstance(config, dict):
            config = {}
        gateway = config.get("gateway")
        if not isinstance(gateway, dict):
            gateway = {}
        gateway["public_url"] = BASE_URL
        config["gateway"] = gateway
        with tempfile.NamedTemporaryFile(
            "w", encoding="utf-8", suffix=".json", delete=False
        ) as fh:
            json.dump(config, fh)
            temp_config_path = fh.name

        proc = subprocess.run(
            [garyx_bin, "-c", temp_config_path, "thread", "send",
             thread_id, message,
             "--workspace-dir", workspace_dir,
             "--timeout", str(timeout_secs),
             "--json"],
            capture_output=True, text=True,
            timeout=timeout_secs + 10,
        )
    except FileNotFoundError as exc:
        raise CliSkip(f"garyx binary not found ({garyx_bin!r}): {exc}") from exc
    except subprocess.TimeoutExpired as exc:
        raise CliSkip(
            f"garyx thread send timed out after {timeout_secs + 10}s "
            f"(provider likely hung; check host config)"
        ) from exc
    finally:
        if temp_config_path:
            try:
                os.unlink(temp_config_path)
            except OSError:
                pass

    if proc.returncode != 0:
        raise AssertionError(
            f"garyx thread send exited {proc.returncode}: "
            f"stdout={proc.stdout!r} stderr={proc.stderr!r}"
        )

    events = []
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return events


def _terminal_event_type(events: list) -> str:
    """Return the last seen terminal event type ('done'/'complete'/'error')
    or 'unknown' if none appeared."""
    for ev in reversed(events):
        if not isinstance(ev, dict):
            continue
        t = ev.get("type")
        if t in ("done", "complete", "error"):
            return t
    return "unknown"


def _find_user_message_matching(messages: list, predicate) -> Optional[dict]:
    """Return the first user-role message whose text matches `predicate`,
    or None. Operates on the `messages[]` envelope returned by
    /api/threads/history."""
    for msg in messages:
        if not isinstance(msg, dict):
            continue
        if (msg.get("role") or "").lower() != "user":
            continue
        text = msg.get("text") or ""
        if predicate(text):
            return msg
    return None


def _agent_team_child_metadata(detail: dict) -> Optional[dict]:
    """Return `metadata.agent_team_child` from a thread metadata payload."""
    if not isinstance(detail, dict):
        return None
    metadata = detail.get("metadata")
    if not isinstance(metadata, dict):
        return None
    child = metadata.get("agent_team_child")
    return child if isinstance(child, dict) else None


def test_agent_team_chat_flow():
    print("\n=== 3. AgentTeam chat flow ===")

    suffix = uuid.uuid4().hex[:8]
    team_id = f"smoke-team-{suffix}"
    leader_id = f"e2e-leader-{suffix}"
    coder_id = f"e2e-coder-{suffix}"
    workspace_dir = f"/tmp/garyx-e2e-smoke-{suffix}"
    leader_display = "E2E Leader"
    coder_display = "E2E Coder"
    leader_token = f"E2E_LEADER_{suffix}"
    leader_fallback = f"E2E_LEADER_FALLBACK_{suffix}"
    coder_handoff = f"E2E_CODER_HANDOFF_{suffix}"
    coder_direct = f"E2E_CODER_DIRECT_{suffix}"
    coder_fallback = f"E2E_CODER_FALLBACK_{suffix}"
    handoff_token = f"E2E_HANDOFF_{suffix}"
    team_created = False
    leader_created = False
    coder_created = False

    try:
        # --- Step 1: Discover a standalone leaf agent to clone ----------
        # The gateway exposes `/api/custom-agents` (not `/api/agents`);
        # response shape is `{"agents": [CustomAgentProfile, ...]}`.
        status, body = _req("GET", "/api/custom-agents")
        if status != 200 or not isinstance(body, dict):
            check("Discover agents: GET /api/custom-agents", False,
                  f"status={status} body={body!r}")
            return
        agents = body.get("agents") or []
        seed = _pick_seed_agent(agents)
        if not isinstance(seed, dict):
            print("  SKIP: need at least one standalone non-team agent")
            return
        provider_type = str(seed.get("provider_type") or "")
        model = str(seed.get("model") or "")
        print(
            f"  seed_agent={seed.get('agent_id')!r} "
            f"provider_type={provider_type!r} model={model!r}"
        )

        # --- Step 2: Create deterministic temporary agents --------------
        coder_prompt = (
            "You are a deterministic end-to-end smoke-test coder.\n"
            "Follow these rules exactly.\n"
            "- If the input contains \"please ship it\", reply with "
            f"exactly \"{coder_direct}\".\n"
            f"- Otherwise, if the input contains \"{handoff_token}\", reply with exactly "
            f"\"{coder_handoff}\".\n"
            f"- Otherwise reply with exactly \"{coder_fallback}\".\n"
            "Do not add any extra words, punctuation, markdown, explanation, "
            "or code fences."
        )
        coder_created = _create_temp_custom_agent(
            coder_id,
            coder_display,
            provider_type,
            model,
            coder_prompt,
        )
        if not coder_created:
            return

        leader_prompt = (
            "You are a deterministic end-to-end smoke-test leader.\n"
            "Follow these rules exactly.\n"
            "- If the input contains \"hello team\", reply with exactly "
            f"\"{leader_token} @[{coder_display}]({coder_id}) {handoff_token}\".\n"
            f"- Otherwise reply with exactly \"{leader_fallback}\".\n"
            "Do not add any extra words, punctuation, markdown, explanation, "
            "or code fences."
        )
        leader_created = _create_temp_custom_agent(
            leader_id,
            leader_display,
            provider_type,
            model,
            leader_prompt,
        )
        if not leader_created:
            return

        print(f"  temp leader={leader_id!r}  temp coder={coder_id!r}")

        # --- Step 3: Create team ---------------------------------------
        status, body = _req("POST", "/api/teams", {
            "team_id": team_id,
            "display_name": "E2E Team",
            "leader_agent_id": leader_id,
            "member_agent_ids": [leader_id, coder_id],
            "workflow_text": "smoke test",
        })
        check("Create team -> 201", status == 201,
              f"status={status} body={body!r}")
        if status != 201:
            return
        team_created = True

        # --- Step 4: Create thread bound to the team ---------------------
        status, body = _req("POST", "/api/threads", {
            "label": "E2E Team Thread",
            "workspaceDir": workspace_dir,
            "agentId": team_id,
        })
        check("Create team-bound thread -> 201", status == 201,
              f"status={status} body={body!r}")
        if status != 201 or not isinstance(body, dict):
            return
        group_thread_id = body.get("thread_id") or body.get("thread_key")
        check("Create thread returns thread_id",
              isinstance(group_thread_id, str) and bool(group_thread_id))
        if not isinstance(group_thread_id, str):
            return

        # provider_type is exposed on the summary; if not, fetch detail.
        provider_type = body.get("provider_type")
        if provider_type in (None, "", "null"):
            status, detail = _req("GET", f"/api/threads/{group_thread_id}")
            if status == 200 and isinstance(detail, dict):
                provider_type = detail.get("provider_type")
        check("Thread resolves to provider_type=agent_team",
              provider_type == "agent_team",
              f"got provider_type={provider_type!r}")

        # --- Step 5: First turn — default-route to leader, then hand off
        #             from leader -> coder in the same group turn ----------
        try:
            first_events = _cli_thread_send(
                group_thread_id,
                "hello team",
                workspace_dir,
                timeout_secs=60,
            )
        except CliSkip as exc:
            print(f"  SKIP turn 1: {exc}")
            return
        first_terminal = _terminal_event_type(first_events)
        check("Turn 1 completed without error",
              first_terminal in ("done", "complete"),
              f"terminal={first_terminal!r}")

        # --- Step 6: Verify group transcript after first turn ------------
        status, hist = _req(
            "GET",
            f"/api/threads/history?thread_id={group_thread_id}&include_tool_messages=false",
        )
        check("History (turn 1) -> 200", status == 200, f"got {status}")
        check("History ok == true",
              isinstance(hist, dict) and hist.get("ok") is True)
        messages = (hist or {}).get("messages") or []
        user_messages = [m for m in messages if (m.get("role") or "").lower() == "user"]
        assistant_messages = [m for m in messages if (m.get("role") or "").lower() == "assistant"]
        check("Turn 1: exactly one user message 'hello team'",
              len(user_messages) == 1
              and (user_messages[0].get("text") or "").strip() == "hello team",
              f"user_messages={[m.get('text') for m in user_messages]!r}")
        assistant_texts = [m.get("text") or "" for m in assistant_messages]
        check("Turn 1: group has leader + coder assistant messages",
              len(assistant_messages) >= 2,
              f"count={len(assistant_messages)}")
        check("Turn 1: leader handoff token persisted in group history",
              any(leader_token in text and handoff_token in text for text in assistant_texts),
              f"assistant_texts={assistant_texts!r}")
        check("Turn 1: coder handoff reply persisted in group history",
              any(coder_handoff in text for text in assistant_texts),
              f"assistant_texts={assistant_texts!r}")

        team_block = (hist or {}).get("team") or {}
        check("team block present", isinstance(team_block, dict) and bool(team_block),
              f"team={team_block!r}")
        check("team.team_id matches",
              isinstance(team_block, dict) and team_block.get("team_id") == team_id,
              f"got {team_block.get('team_id') if isinstance(team_block, dict) else team_block!r}")
        child_map = team_block.get("child_thread_ids") or {} if isinstance(team_block, dict) else {}
        leader_child_id = child_map.get(leader_id)
        coder_child_id = child_map.get(coder_id)
        check(f"team.child_thread_ids has leader entry ({leader_id})",
              isinstance(leader_child_id, str) and bool(leader_child_id),
              f"map={child_map!r}")
        check(f"team.child_thread_ids has coder entry ({coder_id}) after leader handoff",
              isinstance(coder_child_id, str) and bool(coder_child_id),
              f"map={child_map!r}")

        # --- Verify coder child got both user turn and leader handoff ---
        if isinstance(coder_child_id, str) and coder_child_id:
            status, coder_detail1 = _req("GET", f"/api/threads/{coder_child_id}")
            check("Coder-child detail after turn 1 -> 200",
                  status == 200, f"got {status}")
            coder_child_meta1 = _agent_team_child_metadata(coder_detail1)
            check("Coder child persists metadata.agent_team_child",
                  isinstance(coder_child_meta1, dict),
                  f"detail={coder_detail1!r}")
            check("Coder child metadata stores parent team id",
                  isinstance(coder_child_meta1, dict)
                  and coder_child_meta1.get("team_id") == team_id,
                  f"metadata={coder_child_meta1!r}")
            check("Coder child metadata stores group thread id",
                  isinstance(coder_child_meta1, dict)
                  and coder_child_meta1.get("group_thread_id") == group_thread_id,
                  f"metadata={coder_child_meta1!r}")
            check("Coder child metadata stores child agent id",
                  isinstance(coder_child_meta1, dict)
                  and coder_child_meta1.get("child_agent_id") == coder_id,
                  f"metadata={coder_child_meta1!r}")
            check("Coder child metadata records one-time injection timestamp",
                  isinstance(coder_child_meta1, dict)
                  and isinstance(coder_child_meta1.get("initial_context_injected_at"), str)
                  and bool(coder_child_meta1.get("initial_context_injected_at")),
                  f"metadata={coder_child_meta1!r}")

            status, coder_hist1 = _req(
                "GET",
                f"/api/threads/history?thread_id={coder_child_id}&include_tool_messages=false",
            )
            check("Coder-child history after turn 1 -> 200",
                  status == 200, f"got {status}")
            coder_msgs1 = (coder_hist1 or {}).get("messages") or []
            catch_up = _find_user_message_matching(
                coder_msgs1,
                lambda t: "<group_activity from=" in t and "hello team" in t,
            )
            check("Coder child received hello-team catch-up envelope",
                  catch_up is not None,
                  f"user_msgs={[m.get('text') for m in coder_msgs1 if (m.get('role') or '').lower() == 'user']!r}")
            leader_handoff = _find_user_message_matching(
                coder_msgs1,
                lambda t: f"<group_activity from=\"{leader_id}\"" in t and handoff_token in t,
            )
            check("Coder child received leader handoff envelope in same run",
                  leader_handoff is not None,
                  f"user_msgs={[m.get('text') for m in coder_msgs1 if (m.get('role') or '').lower() == 'user']!r}")
            if catch_up is not None:
                catch_up_text = catch_up.get("text") or ""
                check("Coder child catch-up includes team_context exactly once",
                      catch_up_text.count("<team_context>") == 1,
                      f"text={catch_up_text!r}")
                check("Coder child catch-up includes Claude @ handoff syntax rule",
                      "@[DisplayName](agent_id)" in catch_up_text,
                      f"text={catch_up_text!r}")
                check("Coder child catch-up includes team workflow text",
                      "smoke test" in catch_up_text,
                      f"text={catch_up_text!r}")
                check("Coder child catch-up warns against SendMessage/message tools",
                      "SendMessage/message tools" in catch_up_text,
                      f"text={catch_up_text!r}")
            coder_assistants1 = [
                m for m in coder_msgs1 if (m.get("role") or "").lower() == "assistant"
            ]
            check("Coder child emitted handoff reply",
                  any(coder_handoff in (m.get("text") or "") for m in coder_assistants1),
                  f"assistant_msgs={[m.get('text') for m in coder_assistants1]!r}")

        # --- Step 7: Second turn — explicit @coder ----------------------
        second_msg = f"@[{coder_display}]({coder_id}) please ship it"
        try:
            second_events = _cli_thread_send(
                group_thread_id,
                second_msg,
                workspace_dir,
                timeout_secs=90,
            )
        except CliSkip as exc:
            print(f"  SKIP turn 2: {exc}")
            return
        second_terminal = _terminal_event_type(second_events)
        check("Turn 2 completed without error",
              second_terminal in ("done", "complete"),
              f"terminal={second_terminal!r}")

        # --- Step 8: Verify explicit user @ only wakes coder ------------
        status, hist2 = _req(
            "GET",
            f"/api/threads/history?thread_id={group_thread_id}&include_tool_messages=false",
        )
        check("History (turn 2) -> 200", status == 200, f"got {status}")
        messages2 = (hist2 or {}).get("messages") or []
        user_messages2 = [m for m in messages2 if (m.get("role") or "").lower() == "user"]
        assistant_messages2 = [m for m in messages2 if (m.get("role") or "").lower() == "assistant"]
        check("Turn 2: group has at least 2 user messages",
              len(user_messages2) >= 2, f"count={len(user_messages2)}")
        check("Turn 2: explicit user @coder message persisted in group history",
              any((m.get("text") or "").strip() == second_msg for m in user_messages2),
              f"user_messages={[m.get('text') for m in user_messages2]!r}")
        check("Turn 2: group has >=3 assistant messages",
              len(assistant_messages2) >= 3, f"count={len(assistant_messages2)}")
        assistant_texts2 = [m.get("text") or "" for m in assistant_messages2]
        check("Turn 2: coder direct reply persisted in group history",
              any(coder_direct in text for text in assistant_texts2),
              f"assistant_texts={assistant_texts2!r}")

        team_block2 = (hist2 or {}).get("team") or {}
        child_map2 = team_block2.get("child_thread_ids") or {} if isinstance(team_block2, dict) else {}
        check(f"team.child_thread_ids has leader entry ({leader_id}) after turn 2",
              isinstance(child_map2.get(leader_id), str),
              f"map={child_map2!r}")
        check(f"team.child_thread_ids has coder entry ({coder_id}) after turn 2",
              isinstance(child_map2.get(coder_id), str),
              f"map={child_map2!r}")

        coder_child_id = child_map2.get(coder_id)

        # --- Verify coder child thread received catch-up + live turn ---
        if isinstance(coder_child_id, str) and coder_child_id:
            status, coder_hist = _req(
                "GET",
                f"/api/threads/history?thread_id={coder_child_id}&include_tool_messages=false",
            )
            check("Coder-child history -> 200", status == 200, f"got {status}")
            coder_msgs = (coder_hist or {}).get("messages") or []

            catch_up = _find_user_message_matching(
                coder_msgs, lambda t: "<group_activity from=" in t
            )
            check("Coder child received <group_activity ...> catch-up envelope",
                  catch_up is not None,
                  f"user_msgs={[m.get('text') for m in coder_msgs if (m.get('role') or '').lower() == 'user']!r}")

            live_turn = _find_user_message_matching(
                coder_msgs, lambda t: "please ship it" in t
            )
            check("Coder child received live '@' turn ending 'please ship it'",
                  live_turn is not None)
            if live_turn is not None:
                live_turn_text = live_turn.get("text") or ""
                check("Coder live '@' turn does not repeat team_context",
                      "<team_context>" not in live_turn_text,
                      f"text={live_turn_text!r}")

            coder_assistants = [
                m for m in coder_msgs if (m.get("role") or "").lower() == "assistant"
            ]
            check("Coder child has >=1 assistant reply",
                  len(coder_assistants) >= 1,
                  f"count={len(coder_assistants)}")
            check("Coder child has direct-reply token for explicit user @",
                  any(coder_direct in (m.get("text") or "") for m in coder_assistants),
                  f"assistant_msgs={[m.get('text') for m in coder_assistants]!r}")

        # --- Verify leader child thread got at least the turn-1 content ---
        leader_child_id2 = child_map2.get(leader_id)
        if isinstance(leader_child_id2, str) and leader_child_id2:
            status, leader_detail = _req("GET", f"/api/threads/{leader_child_id2}")
            check("Leader-child detail -> 200", status == 200, f"got {status}")
            leader_child_meta = _agent_team_child_metadata(leader_detail)
            check("Leader child persists metadata.agent_team_child",
                  isinstance(leader_child_meta, dict),
                  f"detail={leader_detail!r}")
            check("Leader child metadata stores child agent id",
                  isinstance(leader_child_meta, dict)
                  and leader_child_meta.get("child_agent_id") == leader_id,
                  f"metadata={leader_child_meta!r}")
            check("Leader child metadata records one-time injection timestamp",
                  isinstance(leader_child_meta, dict)
                  and isinstance(leader_child_meta.get("initial_context_injected_at"), str)
                  and bool(leader_child_meta.get("initial_context_injected_at")),
                  f"metadata={leader_child_meta!r}")

            status, leader_hist = _req(
                "GET",
                f"/api/threads/history?thread_id={leader_child_id2}&include_tool_messages=false",
            )
            check("Leader-child history -> 200", status == 200, f"got {status}")
            leader_msgs = (leader_hist or {}).get("messages") or []
            leader_turn1 = _find_user_message_matching(
                leader_msgs, lambda t: "hello team" in t
            )
            check("Leader child has turn-1 'hello team' user content",
                  leader_turn1 is not None,
                  f"user_msgs={[m.get('text') for m in leader_msgs if (m.get('role') or '').lower() == 'user']!r}")
            if leader_turn1 is not None:
                leader_turn1_text = leader_turn1.get("text") or ""
                check("Leader child first wake-up includes team_context exactly once",
                      leader_turn1_text.count("<team_context>") == 1,
                      f"text={leader_turn1_text!r}")
                check("Leader child first wake-up includes Claude @ handoff syntax rule",
                      "@[DisplayName](agent_id)" in leader_turn1_text,
                      f"text={leader_turn1_text!r}")
                check("Leader child first wake-up includes team workflow text",
                      "smoke test" in leader_turn1_text,
                      f"text={leader_turn1_text!r}")
                check("Leader child first wake-up warns against SendMessage/message tools",
                      "SendMessage/message tools" in leader_turn1_text,
                      f"text={leader_turn1_text!r}")
            leader_turn2 = _find_user_message_matching(
                leader_msgs, lambda t: second_msg in t
            )
            check("Leader child did not receive explicit user @coder turn",
                  leader_turn2 is None,
                  f"user_msgs={[m.get('text') for m in leader_msgs if (m.get('role') or '').lower() == 'user']!r}")

        # --- Step 9: Shared workspace across all three threads ----------
        for label, tid in (
            ("group thread", group_thread_id),
            ("leader child", leader_child_id2 if isinstance(leader_child_id2, str) else None),
            ("coder child", coder_child_id if isinstance(coder_child_id, str) else None),
        ):
            if not tid:
                continue
            status, detail = _req("GET", f"/api/threads/{tid}")
            if status != 200 or not isinstance(detail, dict):
                check(f"{label}: GET /api/threads/:id", False,
                      f"status={status} body={detail!r}")
                continue
            # The gateway returns snake_case `workspace_dir` on thread
            # detail; allow `workspaceDir` as a defensive fallback.
            ws = detail.get("workspace_dir") or detail.get("workspaceDir")
            check(f"{label}: workspace_dir == {workspace_dir!r}",
                  ws == workspace_dir, f"got {ws!r}")

    except Exception as e:
        check("AgentTeam chat flow unexpected error", False, str(e))
    finally:
        # Plan §7.5: only clean up the team registration; leave threads
        # behind (matches the existing smoke-test pattern).
        if team_created:
            try:
                _req("DELETE", f"/api/teams/{team_id}")
            except Exception:
                pass
        if leader_created:
            _delete_custom_agent(leader_id)
        if coder_created:
            _delete_custom_agent(coder_id)


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
        _req("GET", "/api/teams")
    except Exception as e:
        print(f"\nFAIL: Cannot reach gateway at {BASE_URL}: {e}")
        print("   Make sure Garyx is running and try again.")
        sys.exit(1)

    test_team_crud()
    test_mcp_tool_surface()
    test_agent_team_chat_flow()

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
