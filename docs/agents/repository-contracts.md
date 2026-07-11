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
- Thread records: the `thread_records` table in
  `<data_dir>/garyx-db.sqlite3` (truth source; #TASK-1864). Record bodies
  never contain the retired `messages` snapshot; conversation content
  lives in the transcript jsonl files. Known endpoint state and the
  `ThreadStore` trait: `garyx-router`; the production store is
  `SqliteThreadStore` (garyx-gateway). The former JSON archive under
  `~/.garyx/data/threads` survives only as the boot-import source and the
  optional dual-write mirror (`sessions.thread_store=sqlite`, the
  emergency mode).
- MCP schema and tool behavior: `garyx-gateway/src/mcp.rs`.
- Provider session behavior: `garyx-bridge`.
- Channel/plugin stream presentation policy, buffering, and tool-call display
  helpers: `garyx-channels/src/plugin_tools.rs`.

## Thread Queries Go Through SQL

- Every "find threads by condition" query — by channel binding, task
  number, recency, or a bare count — must be answered by a SQLite
  projection table. Projections derive inside the same transaction as
  every record write, so they are structurally current: there is no
  backfill, prune, reconcile, or read-time repair layer, and none should
  be reintroduced. If a projection lacks the column a query needs, add the
  column and its write-path derivation.
- `ThreadStore::get/set/update/delete/exists` are point operations on one
  known key; `list_keys` is a key listing. Do not build condition queries
  by listing keys and fetching bodies — that is a projection's job.
- The only full walk over the legacy JSON archive is the one-shot boot
  import into `thread_records` (and it streams one record at a time). The
  archive is not written by any primary path and is retired to
  `~/.garyx/backups/` after the switchover.

## Recent Threads And Active Runs

- Mobile recent-thread lists read the gateway SQLite `recent_threads`
  projection only.
- A task backing thread durably carries `thread_kind="task"` from creation
  through task overlay deletion. `recent_threads.thread_type` and
  `thread_meta.thread_type` derive from that canonical kind and therefore
  remain `"task"` after the task itself is deleted. Do not infer task identity
  from a live task projection, a title, or a title prefix.
- Keep `recent_threads` current by writing it from the thread-store write path;
  do not make `GET /api/recent-threads` rescan router/thread files.
- Bot `/threads` and `/bindthread` reads go through the injected
  `RecentThreadPageReader`, backed by the same SQLite projection. A missing or
  failed reader is an explicit temporary-unavailable result; never add a
  thread-store scan fallback.
- Provider bridge run persistence must use the same thread store as the
  gateway/router (the `SqliteThreadStore`) so active run state derives into
  `recent_threads` inside the same write transaction.
- Do not repair stale `active_run_id` or `run_state` in read routes.
- The only recurring startup recovery is `clear_stale_active_runs`: one SQL
  pass that settles running rows orphaned by the previous process (the bridge
  run index is empty at boot). Versioned, transactional one-shot cutovers such
  as `recent_task_thread_kind_v1` and `endpoint_holder_dedup_v1` run after the
  boot import and record a durable marker; they are not read-time or recurring
  reconciliation.

## Endpoint Binding Ownership

- Each endpoint key has at most one holder in canonical `thread_records`.
  `endpoint_holder_dedup_v1` established this invariant for legacy data; every
  later bind or detach must preserve it through the serialized
  `EndpointBindingMutator` service.
- Runtime bind/detach paths point-read the previous owner from
  `thread_channel_endpoints`, validate known target records, and point-write
  only the known previous/target records. HTTP bind/detach, `/newthread`, and
  `/bindthread` must share this service. Do not reintroduce `list_keys` or
  record-body scans to find endpoint holders.
- When an endpoint has no projected owner, construct its first binding from
  inbound or known-endpoint metadata. Persist binding-related delivery context
  using the already-known record ids; absence of an owner is not permission to
  scan.

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

## Agent Write Concurrency

- `POST /api/custom-agents` is a strict create: an
  existing id is a 409, never a silent overwrite.
- `PUT /api/custom-agents/{id}` is a strict
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
