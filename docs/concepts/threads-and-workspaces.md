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
| `agent_id` | Which agent handles runs on this thread. |
| `provider_type` | Which provider currently backs the agent (`claude_code`, `codex_app_server`, `traex`, `antigravity`, or a native model provider). |
| `workspace_dir` | Filesystem root the agent operates in. May be `null` for chat-only threads. |
| `channel_bindings` | Channel endpoints attached to this thread (Telegram chat id, Feishu chat id, etc.). |
| `recent_run_id` | The last agent run dispatched to this thread; useful for live debugging. |

Threads are persisted under `~/.garyx/data/threads/`. Transcripts live in
per-thread files.

## Recent active threads

Garyx also maintains a gateway-local SQLite projection of visible thread
metadata for compact clients such as the mobile app. The projection is derived
from canonical thread records at the thread-store write boundary: creating,
updating, hiding, deleting, or changing run state on a thread updates the
projection as part of the same gateway write path. It stores denormalized
display metadata:
`thread_id`, title, `workspace_dir`, thread type, provider/agent hints, message
count, the latest preview, recent/active run ids, and a coarse `run_state`
(`running`, `completed`, or `idle`).

Clients read the recency-ordered view through `GET /api/recent-threads`. The
endpoint reads only the SQLite projection and returns pagination metadata
(`total`, `offset`, and `has_more`) alongside the requested page. It must not
rescan router thread files on the read path.

The optional `tasks` query parameter selects the filtering domain before
pagination:

- `tasks=include` includes task and non-task threads and is the default when
  the parameter is omitted.
- `tasks=exclude` returns only non-task threads.
- `tasks=only` returns only task threads.

Unknown values return HTTP 400. `total`, `offset`, and `has_more` always
describe the selected domain. Existing clients that omit `tasks` retain the
same member set, ordering, pagination, envelope, and row schema.

Channel bots use this same projection for thread management. The
`/threads [page|next|prev]` command browses recent non-task threads in pages of
10, and `/bindthread <n>` binds an absolute row number from pages that endpoint
has already seen. `/newthread` creates a fresh thread. The former
`/threadprev` and `/threadnext` commands are hidden compatibility commands that
now point users to the browse-then-bind flow.

## How a chat becomes a thread

When a message arrives on a channel, Garyx looks up the right thread by:

1. **Endpoint binding key** — for example, on Feishu the binding key is the
   chat id. If a thread is already bound to that endpoint, the message is
   routed there.
2. **Account default** — otherwise a fresh thread is created and bound,
   inheriting `agent_id` from the channel account and resolving
   `workspace_dir` from the channel account first, then the selected Agent's
   `default_workspace_dir`, then the provider's normal home/root fallback.

The same thread can be bound to multiple endpoints. The Garyx desktop app
reuses one thread across DMs and group mentions when you want continuity;
each WeChat / Telegram bot uses its own thread per conversation by default.

## Workspace directories

`workspace_dir` is what the agent actually sees as its working directory
when it executes tool calls. CLI provider transports receive it as their cwd;
native model providers use the same path for tool execution.

Garyx does not treat a workspace as a separate domain entity. A
`workspace_dir` is just a directory path recorded on the thread (or supplied
as a default when the thread is created). Desktop folder groups are derived
from these paths for navigation only.

Once a thread has a `workspace_dir`, that execution directory is immutable.
Create a new thread when you want to work from a different directory.

A few ways `workspace_dir` gets set:

- on the channel **account** (every new thread for that account inherits it)
- on the **Agent** as `default_workspace_dir` (used only when the new
  bot/task thread has no explicit or account workspace)
- explicitly when calling [`garyx thread create`](/reference/cli#thread)
- explicitly when calling [`garyx task create --workspace-dir <path>`](/reference/cli#tasks)
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
