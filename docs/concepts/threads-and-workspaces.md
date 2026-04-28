# Threads & workspaces

A **thread** is the unit of conversation in Garyx. It owns a transcript, a
binding to one or more channel endpoints, and the provider it routes runs
to. Every message — DM, @-mention, CLI send, MCP tool call — happens inside
exactly one thread.

## The anatomy of a thread

Each thread carries:

| Field | Meaning |
| --- | --- |
| `thread_id` | Stable identifier, e.g. `thread::<uuid>`. Used in URLs and the CLI. |
| `agent_id` | Which agent (or team) handles runs on this thread. |
| `provider_type` | Which provider currently backs the agent (`claude_code`, `codex_app_server`, `gemini_cli`, `agent_team`). |
| `workspace_dir` | Filesystem root the agent operates in. May be `null` for chat-only threads. |
| `channel_bindings` | Channel endpoints attached to this thread (Telegram chat id, Feishu chat id, etc.). |
| `recent_run_id` | The last agent run dispatched to this thread; useful for live debugging. |

Threads are persisted under `~/.garyx/data/threads/`. Transcripts live in
per-thread files and are migrated on upgrade — see
[`garyx migrate thread-transcripts`](/reference/cli#migrate).

## How a chat becomes a thread

When a message arrives on a channel, Garyx looks up the right thread by:

1. **Endpoint binding key** — for example, on Feishu the binding key is the
   chat id. If a thread is already bound to that endpoint, the message is
   routed there.
2. **Account default** — otherwise a fresh thread is created and bound,
   inheriting `agent_id` and `workspace_dir` from the channel account.

The same thread can be bound to multiple endpoints. The Garyx desktop app
reuses one thread across DMs and group mentions when you want continuity;
each WeChat / Telegram bot uses its own thread per conversation by default.

## Workspaces

`workspace_dir` is what the agent actually sees as its working directory
when it executes tool calls. For Claude Code or Codex, this is the cwd
passed to the SDK; for Gemini, the project root.

A few ways `workspace_dir` gets set:

- on the channel **account** (every new thread for that account inherits it)
- explicitly when calling [`garyx thread create`](/reference/cli#thread)
- via the desktop app when you pick a folder in the thread sidebar

::: info
A workspace is a *development context*, not a security boundary. Garyx does
not sandbox the agent — if it has shell access it can wander out of
`workspace_dir`. Use OS-level controls if you need real isolation.
:::

## Sessions vs threads

A **provider session** is the runtime handle the SDK keeps to a long-lived
agent process. Garyx maintains a 1:N relationship: one thread can resume
the same provider session across many runs (preserving conversation
context), and Garyx automatically creates a fresh session if the previous
one is invalidated (token rotation, provider upgrade, stale resume token).

You usually never see this layer; it shows up in the logs as
`provider run completed via run graph run_id=… session_id=…`.

## Where to go next

- [Channels](/concepts/channels) — what binds to a thread on the inbound side
- [Providers](/concepts/providers) — what runs the agent on the outbound side
- [Configuration](/configuration) — disk layout and overrides
