# Repository Contracts

These contracts describe source-of-truth boundaries that should not be
reinterpreted in feature code.

## Source Of Truth

- Config: `~/.garyx/garyx.json`.
- Channel accounts: `channels.<channel_id>.accounts[...]`
  (`channels.api.accounts[...]` for API).
- Configured bot account `config` is ordinary application state. Mobile and
  desktop should not add token-specific merge, redaction, or preservation paths
  beyond keeping real secrets out of committed fixtures.
- Thread records and known endpoint state: `garyx-router`.
- MCP schema and tool behavior: `garyx-gateway/src/mcp.rs`.
- Provider session behavior: `garyx-bridge`.
- Channel/plugin stream presentation policy, buffering, and tool-call display
  helpers: `garyx-channels/src/plugin_tools.rs`.

## Thread Queries Never Enumerate Record Files

- In steady state the thread-record filesystem serves exactly two shapes:
  point reads/writes of a single known `thread_id`, and transcript
  append/range access. Every "find threads by condition" query — by channel
  binding, team, task number, recency, or a bare count — must be answered by
  a SQLite projection maintained from the write path.
- Do not add code that calls `list_keys` and then `get`s record bodies to
  filter them. Thread records carry their full legacy `messages` history
  (multi-MB for busy threads); enumerating them materializes gigabytes and
  was the root of the gateway's 5.5GB startup memory peaks. If a projection
  lacks the column a query needs, add the column and its write-path update —
  do not fall back to scanning files.
- The only legitimate full walk over record files is a projection
  repair/migration (startup reconciliation after a projection version bump
  or loss), and even that path should avoid materializing `messages` when it
  only needs metadata.

## Recent Threads And Active Runs

- Mobile recent-thread lists read the gateway SQLite `recent_threads`
  projection only.
- Keep `recent_threads` current by writing it from the thread-store write path;
  do not make `GET /api/recent-threads` rescan router/thread files.
- Provider bridge run persistence must use the same projecting thread store as
  the gateway/router so active run state updates are dual-written into
  `recent_threads` at write time.
- Do not repair stale `active_run_id` or `run_state` in read routes.
- Startup reconciliation may repair historically stale active-run projection
  rows as a data migration, but steady-state correctness must come from the
  thread-store write path.

## Transcript Rendering

- The committed/control transcript ledger is the only source for rendered chat
  structure. `garyx-models` owns the `TranscriptRenderState` reducer; desktop
  and mobile must not reimplement user-turn grouping, tool grouping, tail
  thinking, filtered placeholders, or final-answer placement.
- Gateway per-thread SSE sends `thread_render_frame`:
  `{ events, render_state }`. `events` synchronize caches, run state, and
  cursors; `render_state` renders rows.
- The sequence rule is write-then-derive: commit transcript records first, then
  derive `render_state` from committed records up to the frame sequence.
  `render_state.based_on_seq` must match the frame sequence for normal replay
  and live frames.
- `render_state.rows` may be narrowed by a client-declared `render_floor`.
  `render_state.based_on_seq` remains the committed window tail, and event
  delivery is still governed only by the SSE cursor and committed ledger.
- A caught-up per-thread stream connection still sends a snapshot-only frame
  with `events: []`; its SSE id and replay cursor are
  `render_state.based_on_seq`, clamped to the actual committed ledger tail.
- Desktop and mobile may keep optimistic local user rows, pending-ack chrome,
  local selection state, pagination cursors, and presentation adapters. They
  must dumb-render `render_state.rows`, `render_state.tailActivity`, and
  `render_state.activeToolGroupId`.
- Transport state (`LiveStreamStatus`, WebSocket/SSE connection state, cached
  `activeRun` projections) is not a transcript render source. It may only drive
  retry/reconnect behavior or local business gates defined in
  `docs/agents/conversation-state.md`.

## Agent And Team Write Concurrency

- `POST /api/custom-agents` and `POST /api/teams` are strict creates: an
  existing id is a 409, never a silent overwrite.
- `PUT /api/custom-agents/{id}` and `PUT /api/teams/{id}` are strict
  conditional updates: the request must carry `expected_updated_at` (the
  `updated_at` of the profile the edit was based on). A missing token is a
  400, a missing target is a 404 (deleted profiles are not resurrected), and
  a moved token is a 409 with `current_updated_at` so the client can re-read
  and retry. There is no unconditional HTTP write path.
- Every client edit flow (desktop, mobile, CLI) must base its update on a
  freshly fetched profile and send that profile's `updated_at` back; the CLI's
  update commands do the GET-merge-PUT internally.

## Provider And Channel Behavior

- Telegram uses the throttled plugin stream policy: assistant text can stream
  through 300ms edit coalescing while top-level tool calls flush immediately.
- Discord uses the buffered plugin stream policy: assistant text deltas wait
  until a top-level tool call starts or the run finishes; rapid tool-call
  placeholder updates are coalesced with a one-second minimum interval.
- When a queued user message is acknowledged mid-stream, Discord finalizes the
  current reply segment and starts subsequent assistant output in a new message.
- Discord REST writes retry 429, transient network, and 5xx responses with
  backoff.
- Garyx in-process native model providers load Garyx-managed Skills from
  `~/.garyx/skills` and managed MCP from gateway-injected
  `remote_mcp_servers`; they should not read downstream Claude/Codex Skill or
  MCP config files.

## Workflows

- Task execution is selected by `ThreadTask.executor`, whose product-level
  variants are Agent, Agent Team, and Workflow. New task creation paths should
  set `executor`; `assignee` remains a compatibility/ownership field and should
  not be used as the execution selector for new Agent/Team/Workflow UI or CLI
  flows.
- Workflow source code executes in the caller's process through SDKs such as
  `@garyx/workflow`; gateway must not own a workflow language runtime or parse
  arbitrary workflow scripts as the primary execution model.
- Reusable workflow definitions are file-backed packages rooted by
  `garyx.workflow.json`. Gateway list/get/install APIs should read and copy
  those packages; do not store workflow definitions as runtime database rows.
- Workflow task input is a single plain-text string in every surface. The CLI
  offers one `--input <text>` flag; a workflow that needs structured data
  parses that text in its own first step. Do not add per-source input flags
  (`--input-file` / `--input-json`) or treat input as the source for generated
  product forms.
- Gateway workflow APIs provide observability, durable run/event storage, hidden
  child-thread execution, and structured child results for Task-backed workflow
  runs. SDKs connect those APIs with ordinary user code through explicit options
  or `GARYX_*` environment variables such as `GARYX_TASK_ID`,
  `GARYX_TASK_THREAD_ID`, `GARYX_WORKFLOW_RUN_ID`, and
  `GARYX_PARENT_THREAD_ID`.
- Structured child results are implemented as a generic thread-run capability:
  the child thread metadata carries the result schema, and the MCP server exposes
  a dynamic `submit_result` tool for the current thread with direct schema-field
  arguments. Do not route structured results through workflow-only tokens or a
  generic `{ payload: ... }` wrapper.

## Time And Timezone

- Storage, HTTP API contracts, and scheduling baselines are UTC: RFC3339 with
  trailing `Z` (`Utc::now().to_rfc3339()`) or epoch values. Do not localize
  persisted or wire timestamps; clients localize at render time.
- Human-readable sinks render gateway-machine local wall-clock time in the
  unified `YYYY-MM-DD HH:MM:SS` style — no `T` separator, no offset suffix,
  the machine timezone is implicit (sub-second precision allowed for logs):
  the tracing log timer (`main.rs` `ChronoLocal`), thread-log line stamps,
  CLI list/detail timestamps (`commands/shared.rs::format_local_timestamp`),
  and agent-facing strings (`schedule_followup` responses, followup metadata,
  and the `current_time` line in `<garyx_thread_metadata>`, which carries the
  IANA zone once as context). Machine-facing `unix_ts` fields stay
  timezone-neutral.
- Bare cron expressions without an explicit timezone are interpreted in the
  gateway machine's local timezone, not UTC. Product automation schedules
  (Daily/Monthly) always carry an explicit IANA timezone; the CLI
  `--daily-time` default is the machine's IANA timezone via `iana-time-zone`,
  never a hard-coded region.
