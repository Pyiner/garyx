# Garyx Native Agent Loop

Status: planned implementation for the `garyx_native` provider.

## Goals

Garyx needs a first-party provider that runs the agent loop inside the bridge
instead of delegating it to Claude Code, Codex app-server, or Gemini CLI. The
provider should:

- use Garyx's existing streaming, transcript, interruption, task, and channel
  delivery pipeline;
- use Codex-compatible OpenAI authentication so users who already logged in to
  Codex do not need to duplicate API keys in Garyx;
- expose enough local tools for a model to inspect and modify the workspace;
- persist provider messages so a thread can continue with useful context;
- support `/goal` as a persistent objective layer on top of normal chat.

Non-goals for the first version:

- replacing existing providers;
- adding a new secret store;
- implementing a separate desktop UI before the command/channel path works.

## Provider Boundary

The provider slug is `garyx_native`; the built-in agent id is `garyx`.

The implementation lives in `garyx-bridge` and implements
`AgentLoopProvider`. It consumes the existing `ProviderRunOptions` and emits the
existing `StreamEvent` variants:

- `SessionBound` when a Garyx-native session id is created or restored;
- `Delta` for assistant text;
- `ToolUse` and `ToolResult` for normalized transcript persistence;
- `Boundary::UserAck` when queued streaming input is accepted;
- `Done` when the run is complete.

The bridge persists these messages through the existing persistence worker. To
support restart/resume quality without changing the public provider request
shape, the bridge places normalized recent session messages in
`ProviderRunOptions.metadata.garyx_session_messages`. Other providers ignore
the metadata key; `garyx_native` uses it to rebuild the next model request.

## Authentication

The provider uses a `codex` auth source by default. Resolution order follows the
practical Codex path:

1. `CODEX_API_KEY` from the provider env or process env;
2. `OPENAI_API_KEY` from the provider env or process env;
3. `$CODEX_HOME/auth.json`, or `~/.codex/auth.json` when `CODEX_HOME` is unset.

Supported credential forms:

- `OPENAI_API_KEY` in `auth.json`: call the OpenAI Responses API at
  `https://api.openai.com/v1/responses`;
- `tokens.access_token` in `auth.json`: call the ChatGPT Codex backend at
  `https://chatgpt.com/backend-api/codex/responses` and forward
  `ChatGPT-Account-ID` when present.

Codex agent identity signing is intentionally not duplicated in this pass. If
Codex only has `agent_identity`, Garyx reports a clear configuration error.

## Loop

Each run performs the same basic cycle used by Codex and Claude Code:

1. Add the user input to the session transcript.
2. Send transcript, system instructions, optional goal instructions, and tool
   schemas to the model.
3. Stream assistant text to Garyx as it arrives.
4. If the model asks for tool calls, execute them, append tool results, and
   sample again.
5. Stop when a model turn returns assistant text without tool calls, when the
   run is interrupted, or when the configured tool-iteration budget is reached.

The provider has an in-memory session map keyed by Garyx `thread_id`. On each
run it seeds that map from the restored `sdk_session_id` and persisted
`session_messages` if needed.

## Tools

The first native tool set is deliberately small but complete enough for coding
work:

- `exec_command`: run a shell command in the active workspace with optional
  timeout;
- `read_file`: read a UTF-8 text file;
- `write_file`: write a UTF-8 text file;
- `list_dir`: list directory entries;
- `get_goal`: inspect the current thread goal;
- `update_goal`: mark the current goal `active`, `paused`, or `completed`.

Tool outputs are truncated before sending them back to the model and before
persisting to the transcript.

## Goal State

`/goal <objective>` is a router-native command. It resolves or creates the
current thread, writes the goal both at the thread top level and under
`metadata.goal`, and enables loop mode:

```json
{
  "goal": {
    "objective": "...",
    "status": "active",
    "created_at": "...",
    "updated_at": "..."
  },
  "metadata": {
    "goal": {
      "objective": "...",
      "status": "active",
      "created_at": "...",
      "updated_at": "..."
    }
  },
  "loop_enabled": true,
  "loop_iteration_count": 0
}
```

`/goal off`, `/goal clear`, or `/goal done` clears the goal and disables loop.

Bridge run metadata is enriched from thread metadata, so the native provider
receives the goal object on each turn. The provider turns it into hidden system
context and exposes the goal tools. When `update_goal` marks the goal
`completed`, the provider returns metadata indicating completion; the bridge
turns off loop mode for that thread.

This does not mathematically guarantee the objective is achieved. The guarantee
is operational: the objective is durable, every turn sees it, the model has a
goal-completion tool, the loop can continue automatically, and Garyx stops only
when the goal is completed, paused, cleared, interrupted, or exhausted by
budget/error.

## Tests

The implementation is test-driven in vertical slices:

1. provider/model config round-trips and built-in `garyx` agent profile;
2. `/goal` command parsing and thread mutation;
3. bridge option hydration with persisted `session_messages`;
4. native loop with fake model: assistant-only turn;
5. native loop with fake model: tool call followed by second model request;
6. native loop with queued streaming input and interrupt;
7. auth resolution from env and Codex auth file;
8. focused package tests for touched crates.
