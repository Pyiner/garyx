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
  `<data_dir>/threads` and `<data_dir>/sessions` survives only as the one-shot
  boot-import source for pre-SQLite upgrades. There is no runtime JSON backend
  or dual-write mirror; recovery uses the archived backup plus an explicitly
  forced fresh import.
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
- `ThreadStore::get/set/patch/delete/exists` are point operations on one
  known key; `list_keys` is a key listing. Do not build condition queries
  by listing keys and fetching bodies — that is a projection's job.
- `ThreadStore` write shapes are exactly `set` (full replace), `patch`
  (validated `ThreadRecordPatch` witness), and `update_many_atomic` (the
  privileged all-or-nothing batch — the only shape allowed to change
  `channel_bindings`). There is no raw top-level merge method; do not
  reintroduce one. The binding privilege is enforced at entry construction:
  `AtomicRecordMerge::new` rejects the protected field, and binding-carrying
  entries exist only through `AtomicRecordMerge::channel_bindings_merge`
  under the `ChannelBindingsMergeAuthority` witness. The witness has no
  public constructor: it is provided by the `EndpointBindingMutator` trait
  itself (implementing the trait is the declaration of being the serialized
  binding mutator), plus the `test-seams`-gated
  `ChannelBindingsMergeAuthority::test_authority` seam for fixtures that
  inject binding state from tests. Store-owned runtime domains (run
  coordinator, projection read seams) live on the `ThreadStoreDomains`
  supertrait, and
  `garyx_router::store_contract` is the executable contract every backend
  and delegating wrapper must run from its tests.
- The only full walk over the legacy JSON archive is the one-shot boot
  import into `thread_records` (and it streams one record at a time). The
  archive is not written by any primary path. After a successful import its
  `threads/` and `sessions/` directories are moved to
  `<data_dir>/backups/legacy-archive-v1/`; keeping the backup under the data
  directory makes each retirement a same-filesystem atomic rename. For the
  default data directory this is
  `~/.garyx/data/backups/legacy-archive-v1/`.
- A failed legacy import aborts startup and retries on the next boot; no
  failure is recorded as complete. Recovery intent with no restored archive,
  or with only a partially moved-back archive, also aborts startup.
- Manual legacy-import recovery is: move (do not copy) the backup `threads/`
  and `sessions/` directories back under `<data_dir>`, clear only the
  `thread_records_import` projection-state row, then reboot. The importer
  atomically restores the import marker, advances the monotonic import
  generation, and clears the prior retirement marker; generation-dependent
  SQL cutovers then run once against the restored records. Archived-thread
  tombstones always win over restored records and transcripts, so recovery
  cannot resurrect their record, projections, or conversation. False-success
  markers written by older binaries are not repaired automatically.
- A full `<data_dir>` backup restore or clone is a different operation: stop
  every gateway using that directory, restore it, then run
  `garyx gateway rotate-store-incarnation` with the restored configuration
  before the first serving boot. The command takes the same per-data-dir lock
  as the gateway and rotates the favorites CAS identity. Normal reopen/restart
  never rotates it. Legacy-import recovery rotates the identity atomically in
  the import-marker transaction and does not need this extra command.

## Thread Lifecycle And Terminal Tombstones

- The row in `archived_threads` is the canonical durable terminal tombstone.
  Its `kind` is either `archived` or `deleted`; both kinds fence resurrection.
  The distinction exists for lifecycle result-matrix decisions and diagnostics,
  not to permit writes to archived threads.
- The tombstone is the only terminal-state truth source. Coordinator state is a
  read-through cache that must calibrate from the tombstone before admission or
  mutation decisions and may install a terminal state only after the matching
  SQLite commit. A missing process-local coordinator entry never means `Live`.
- Every production thread-record write path — including set, update, batch,
  bridge persistence, and legacy import — must check for either tombstone kind
  in the same SQLite transaction as the write and reject the write. Do not add
  an archived-only check, a preflight-only check, or a write path that bypasses
  the canonical store. Historical bare deletes from before this cutover remain
  accepted history; reads must not synthesize tombstones for them.
- Archive and delete are replay-safe lifecycle operations. The expected store
  incarnation is checked before completed lookup or in-flight registration;
  the durable ledger key is `(store_incarnation, operation_id)`, and a replay
  must match the complete canonical request fingerprint. Completed success
  replays return the original result payload, including the first detached
  endpoint-key set.
- Deterministic rejection is committed in a decision transaction before it is
  published. Internal or transient failure is not written as a completed
  decision and remains eligible for a same-operation-id retry.
- An applied lifecycle transaction atomically commits its terminal tombstone,
  removes the canonical record and its projections/pin/favorite state, performs
  persistent endpoint detaches, records the completed ledger outcome, and
  inserts all required volatile-cleanup jobs. No read route, boot reconcile, or
  process-local cache update may substitute for this transaction boundary.

### Lifecycle Cleanup Outbox

- `cleanup_outbox` is the durable handoff for post-commit volatile cleanup:
  conditional endpoint-runtime invalidation, provider/runtime teardown,
  transcript removal, and thread-log removal. A successful lifecycle response
  means the durable transaction committed; it does not mean every outbox step
  already ran.
- The worker starts once during gateway boot and resumes all pending jobs from
  SQLite, so a crash after commit or before the initial wake cannot lose
  cleanup. Ready jobs from different threads may progress independently, but a
  later job for one thread must never overtake that thread's earlier pending
  job. Read routes never drain or repair the outbox.
- Cleanup steps are replay-safe. A failed step stays pending with a persisted
  attempt count and bounded backoff. Endpoint invalidation carries the expected
  owner and must not erase a later rebound owner. Transcript/log deletion and
  already-absent provider state are successful replays.
- Runtime teardown is ordered: provider clear must reach `Cleared` or
  `AlreadyAbsent` before local affinity/workspace state is dropped and the job
  is marked done. `RetryableFailure` retains that local routing state and keeps
  the job pending; this retain-until-cleared behavior must not be reverted.

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
- The server may emit same-seq render-state overwrite frames. Clients reject a
  full snapshot only when `based_on_seq` is below their accepted render
  cursor; accepted snapshots are compared by full value, excluding
  `rows_hash` (a delta-chain transport token), never by cursor equality alone.
- Committed-event identity is `(seq, payload)`: an equal same-seq payload is a
  silent duplicate, while a distinct same-seq payload is an overwrite whose
  body or rewrite/reset semantics must apply exactly once for that payload.
- A logical stream request id gates connection-scoped state only (render
  snapshots, windowed-replay markers, errors, and expansion settles). It must
  never filter committed ledger events, including events queued by a
  superseded logical request.
- A client that narrows structure with `render_floor` owns a demand-convergent
  invariant: its effective render window covers the minimum seq among loaded
  committed bodies (or is the full window). When demand extends loaded bodies
  below the window, the client widens structure by resuming with a lower
  `render_floor`; bounded retry may hold only until the next demand trigger.
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

## Structural Guards And Test Seams

- Architecture guards are structural, never textual: no test may walk or
  regex-scan source files to enforce a boundary. Enforce boundaries with
  visibility, typestate witnesses (e.g. `DrainedDeleteReservation`, minted
  only by consuming a delete reservation through the coordinator's
  abort/drain barrier and consumed again by settlement), `cfg(test)`-only
  helpers for test seeding (`archive_thread_record`), and in-module tests
  that import reviewed contract constants directly (the patch-field
  allowlists).
- Test seams are additive injection points. `cfg(test)` must never replace
  production behavior. The production side of a seam is wired explicitly at
  the composition root — e.g. the runtime assembly passes
  `AppStateBuilder::with_restart_requester(process_restart_requester())`
  while the builder default stays inert — matching the
  `with_persistent_local_stores` opt-in pattern.
- Contracts formerly pinned by retired source-scan guards, now owned by
  review: provider SPI `run_streaming` is invoked only by the bridge run
  graph (`garyx-bridge/src/run_graph.rs`); bridge raw run entrypoints stay
  `pub(crate)` behind `MultiProviderBridge`; automation-engine logging goes
  through the `engine/log.rs` `cron_*` wrappers (the runtime tracing-target
  test remains); the subprocess plugin host dispatches inbound work only
  through `InboundPipeline` (primary enforcement stays compile-level via
  `pub(crate)` in garyx-channels); direct `UPDATE recent_threads` writes
  stay confined to activity-seq allocation plus the reviewed pre-bind-only
  exceptions documented at their call sites; the channel dispatcher stays
  channel-blind — no built-in channel-name literals or downcast vocabulary
  in `dispatcher.rs` (the privacy seal in `outbound_registry` remains the
  compile-level boundary).
- Startup ordering that used to be text-pinned is structurally funneled:
  `garyx_db` obtains an on-disk SQLite connection under the data-dir lock
  only through `lock.rs::acquire_locked_database`, whose body is
  lock -> pre-R5 parent handoff (a barrier private to `lock.rs`) -> open;
  the fail-closed behavior test pins the observable property. Migration
  dependency edges are runtime preconditions checked by the migrations
  themselves (and exercised by the full-runner tests); registration order
  between migrations without a precondition is not a contract — introduce a
  new dependency as a precondition, never as ordering convention.

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
