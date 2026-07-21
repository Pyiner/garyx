# Task Notification Structured Presentation

Status: revision 9 for adversarial review
(r1 3B+8M; r2 1B+7M+3m; r3 2B+5M+2m; r4 3B+3M+1m; r5 3B+1M; r6 4B+4M;
r7 5B+3M+1m; r8 5B+2M. This HEAD is the complete self-contained contract.
Closed and unchanged: §1, §3.4 UI contract, six-producer inventory,
enqueue convergence, flattened presentation + unknown-kind lossy decode,
prose-parser deletion, no-retroactive-trust, capability-block
precondition, dual render_schema=2 gates, owner-approved breaking set.)
Owner: Gary (orchestrator thread)
Scope: garyx-models, garyx-router, garyx-bridge, garyx-gateway, garyx
(plugin host + CLI), garyx-channels, desktop renderer + desktop
main-process gateway client, iOS

## 1. Problem

(unchanged) 73/252 live committed task-notification records lose their
semantics in the busy-thread queue allowlist and render as raw XML; four
client prose parsers reverse-parse templates; provider runtime material
leaks into committed transcripts past a two-key denylist; several
ingresses accept arbitrary metadata.

## 2. Design principles

1. **The write path is the root of trust.** Internal dispatchers alone
   attach provenance; untrusted input paths structurally cannot; the
   store write surface is typed Trusted/Untrusted — including pendings
   and the live-run write API (§3.1).
2. **Run-scoped provider runtime input never persists.** Three types:
   `ExternalRunOverrides` (one-shot, runtime), `ThreadRuntimeConfig`
   (persistent product state), `ResolvedRunAttribution` (server-derived
   `effective_*` audit; names never collide with request fields).
3. **Structure is never round-tripped through prose.**
4. **No retroactive trust, no dual mechanisms, no shims.** Owner-approved
   breaking set is final: plugin metadata field deletion, historical
   loop-summary degradation, history envelope hard cut, render_schema=2
   hard cutover, no history upgrade.

## 3. Changes

### 3.1 Typed dispatch metadata and provenance

#### Override/config/audit types (r7 B2, unchanged)

- `ExternalRunOverrides` (chat only): `model`, `model_reasoning_effort`,
  `model_service_tier`, `provider_type`, `workspace_path`,
  `system_prompt`, `remote_mcp_servers` (URL-only),
  `garyx_mcp_headers`. Feeds `RuntimeMetadata`; never persisted.
- `ThreadRuntimeConfig` (Create/Update thread + internal agent binding):
  `model`, reasoning, tier, session resume/fork,
  `sdk_session_provider_hint`, agent snapshot. Thread-record product
  state.
- `ResolvedRunAttribution` (server-derived, durable): `effective_model`,
  `effective_reasoning_effort`, `effective_service_tier`,
  `effective_provider_type`, `effective_workspace_dir`, `agent_id`,
  `agent_display_name`, `sdk_session_id`.

#### Core types (r8 confirmed, restated)

```rust
pub struct DispatchMetadata {
    provenance: Option<ProvenanceRecord>,
    pub durable: DurableMetadata,
    runtime: RuntimeMetadata,
}
impl DispatchMetadata {
    pub fn external(durable: DurableMetadata, runtime: RuntimeMetadata) -> Self;
    pub fn internal(_: &DispatchAuthority, provenance: ProvenanceRecord,
                    durable: DurableMetadata, runtime: RuntimeMetadata) -> Self;
    pub fn provider_view(&self) -> ProviderMetadataView<'_>;
}

pub enum ProvenanceRecord { // serde tag = "kind"; no payload field named kind
    TaskNotification { event: String, status: String, task_id: String,
                       title: String, task_thread_id: Option<String>,
                       source_run_id: Option<String> },
    RestartWake { id: String, wake_kind: String, target: Option<String>,
                  all: bool, attempt: u32 },
    Followup { job_id: String, scheduled_at: String, scheduled_for: String,
               reason: Option<String>, originating_run_id: Option<String> },
    Automation { automation_id: String, cron_action: String },
    Cron { cron_job_id: String, cron_action: String },
    TaskAutoStart { task_id: String, dispatch_reason: String },
}
```

Producer inventory and legacy-key mapping unchanged (six live producers;
`LoopContinuation` legacy keys are migration input only). Serde roundtrip
tests for all six variants.

- `DurableMetadata` (typed constructors = inventory):
  `ResolvedRunAttribution`; run identifiers; `client_intent_id`,
  `origin_id`; conversation context (`channel`, `account_id`, `from_id`,
  `chat_id`, `client`, `is_group`, `thread_binding_key`,
  `delivery_thread_id`, `display_label`, `delivery_target_type`,
  `delivery_target_id`, `resolved_thread_id`, `attachments` refs);
  `fork_from_*`; `DurableRuntimeContext` (below); `external` extension
  namespace. Queue-commit keys are **not** part of this inventory — they
  come from `QueueCommitAttribution` (below).
- **`DurableRuntimeContext`** (r8 B3): a fully typed schema — explicit
  fields for thread identity, binding/channel labels, workspace, and
  task linkage, each rebuilt from authoritative typed state
  (thread record, binding, task projection). No `#[serde(flatten)]`, no
  seeding from any prior/caller `runtime_context` object.
- `RuntimeMetadata` (opaque, no serde): `ExternalRunOverrides` content +
  server runtime (`ManagedMcpCapability` (§3.2), managed
  `remote_mcp_servers` incl. stdio, `provider_env`,
  `developer_instructions`, `desktop_antigravity_env`,
  `sdk_session_fork`). The global gateway auth token is not a run
  credential and never enters dispatch metadata.

#### Trusted pending & the live-run store API (r8 B1)

The general `ThreadStore` raw-record surface (`set`, `update`, `patch`,
`update_many_atomic`, import) accepts **only** untrusted record types and
always sanitizes embedded pendings. Live-run persistence therefore gets
its own typed API — the contradiction "live writes go through raw
set/patch" is resolved by removing that path, not by exempting it:

- `TrustedPendingUserInput`: created only by the single live enqueue
  constructor; carries `provenance` + `durable_metadata` + identity
  fields.
- `TrustedPendingInputsPatch`: the only way bridge persistence writes
  pendings into the run record — a dedicated store method
  (`apply_live_run_patch`) that accepts typed trusted values and touches
  nothing else.
- `UntrustedPendingUserInput`: every read-back of persisted pendings
  (restart, import, merge) deserializes here and is re-sanitized
  (provenance, legacy keys, runtime fields stripped). **Trust is never
  restored across a process restart**: ACK accepts only
  `TrustedPendingUserInput` values created by this process; after
  restart, a restored pending re-commits through the ACK path as
  sanitized durable data with its persisted provenance **only if** it
  re-enters via `apply_live_run_patch` replay from the trusted run
  record — a raw-imported pending never does.
- `QueueCommitAttribution { queued_input_id, queued_at, origin_id,
  origin_run_id }`: typed; attached exclusively by the ACK
  committed-message constructor; this is the sole source of those keys
  in committed metadata.
- Tests: unrelated `patch`/`update`/`update_many_atomic` calls neither
  corrupt nor re-trust live pendings; raw multi-record updates cannot
  inject pendings with provenance; ACK after restart rejects
  raw-imported pendings.

#### Message write surface (r8 confirmed, restated)

- `TrustedCommittedMessage`: bridge persistence only; sole
  provenance-writing type.
- `UntrustedImportedMessage` (boot import, provider-session import via
  `rewrite_from_messages` — a live runtime path, raw append/rewrite):
  strips top-level provenance, legacy internal keys, **all runtime-owned
  and request-shaped fields, and all `ResolvedRunAttribution` fields**
  (`effective_*`, `sdk_session_id` — server-derived values are never
  accepted from imports), then rebuilds `DurableRuntimeContext` from
  authoritative state. Unknown legacy keys move into `external`
  (inert, preserved); known dangerous keys are deleted.
- `UntrustedImportedThreadRecord`: raw record set/patch/import sanitizes
  embedded pendings identically.
- Reducer reads `message.provenance` from committed records.

#### Provenance attribution rules (r8 confirmed)

Direct → originating user row. Queued → that pending's ACK user row,
taking precedence over the active run's provenance.
Assistant/tool/control rows never inherit; `build_run_messages` bulk
merge is replaced by per-row typed attribution. Test: run A active +
internal B queued + multiple ACKs.

#### Goldens (checked-in JSON fixtures are authoritative; the design
copies are complete, legal JSON — no comments, no ellipses; r8 M1)

Golden #1 — direct-path committed transcript record:

```json
{ "seq": 41, "thread_id": "thread::aaaa", "run_id": "run-direct-1",
  "timestamp": "2026-07-21T06:00:00Z",
  "message": {
    "role": "user",
    "content": "<garyx_task_notification event=\"ready_for_review\" task_id=\"#TASK-42\" status=\"in_review\" title=\"T\">Body</garyx_task_notification>",
    "timestamp": "2026-07-21T06:00:00Z",
    "provenance": { "kind": "task_notification", "event": "ready_for_review",
                     "status": "in_review", "task_id": "#TASK-42", "title": "T",
                     "task_thread_id": "thread::bbbb", "source_run_id": "run-src-1" },
    "metadata": { "effective_model": "m1", "effective_provider_type": "claude_code",
                   "effective_reasoning_effort": "high", "agent_id": "gary",
                   "agent_display_name": "Gary", "sdk_session_id": "sess-1",
                   "channel": "api", "account_id": "main", "from_id": "loop",
                   "resolved_thread_id": "thread::aaaa",
                   "runtime_context": { "thread_id": "thread::aaaa",
                                         "channel": "api", "account_id": "main",
                                         "workspace_dir": "/Users/test/repo",
                                         "task_id": "#TASK-42" },
                   "external": {} } } }
```

Golden #2a — persisted pending inside the run record:

```json
{ "id": "queued_input:u1", "bridge_run_id": "run-A",
  "origin_run_id": "task-notify-TASK-42-x1",
  "queued_at": "2026-07-21T06:01:00Z", "origin_id": null,
  "status": "queued",
  "text": "<garyx_task_notification event=\"ready_for_review\" task_id=\"#TASK-42\" status=\"in_review\" title=\"T\">Body</garyx_task_notification>",
  "content": [ { "type": "text", "text": "<garyx_task_notification event=\"ready_for_review\" task_id=\"#TASK-42\" status=\"in_review\" title=\"T\">Body</garyx_task_notification>" } ],
  "raw_content_type": "array",
  "provenance": { "kind": "task_notification", "event": "ready_for_review",
                   "status": "in_review", "task_id": "#TASK-42", "title": "T",
                   "task_thread_id": "thread::bbbb", "source_run_id": "run-src-1" },
  "durable_metadata": { "effective_model": "m1",
                         "effective_provider_type": "claude_code",
                         "agent_id": "gary", "channel": "api",
                         "account_id": "main",
                         "resolved_thread_id": "thread::aaaa",
                         "runtime_context": { "thread_id": "thread::aaaa",
                                               "channel": "api",
                                               "account_id": "main",
                                               "workspace_dir": "/Users/test/repo",
                                               "task_id": "#TASK-42" },
                         "external": {} } }
```

Golden #2b — ACK committed transcript record (provenance is the
pending's own, never run A's; queue keys from `QueueCommitAttribution`):

```json
{ "seq": 57, "thread_id": "thread::aaaa", "run_id": "run-A",
  "timestamp": "2026-07-21T06:02:00Z",
  "message": {
    "role": "user",
    "content": "<garyx_task_notification event=\"ready_for_review\" task_id=\"#TASK-42\" status=\"in_review\" title=\"T\">Body</garyx_task_notification>",
    "timestamp": "2026-07-21T06:02:00Z",
    "provenance": { "kind": "task_notification", "event": "ready_for_review",
                     "status": "in_review", "task_id": "#TASK-42", "title": "T",
                     "task_thread_id": "thread::bbbb", "source_run_id": "run-src-1" },
    "metadata": { "queued_input_id": "queued_input:u1",
                   "queued_at": "2026-07-21T06:01:00Z",
                   "origin_run_id": "task-notify-TASK-42-x1",
                   "effective_model": "m1", "effective_provider_type": "claude_code",
                   "agent_id": "gary", "channel": "api", "account_id": "main",
                   "resolved_thread_id": "thread::aaaa",
                   "runtime_context": { "thread_id": "thread::aaaa",
                                         "channel": "api", "account_id": "main",
                                         "workspace_dir": "/Users/test/repo",
                                         "task_id": "#TASK-42" },
                   "external": {} } } }
```

Golden #3 — migrated legacy record (scrubbed; no provenance; unknown
legacy key preserved in external):

```json
{ "seq": 12, "thread_id": "thread::cccc", "run_id": "run-old-1",
  "timestamp": "2026-05-01T01:00:00Z",
  "message": {
    "role": "user",
    "content": "<garyx_task_notification event=\"ready_for_review\" task_id=\"#TASK-9\" status=\"in_review\">Task #TASK-9 is ready for review: Old\n\nBody</garyx_task_notification>",
    "timestamp": "2026-05-01T01:00:00Z",
    "metadata": { "channel": "api", "account_id": "main",
                   "resolved_thread_id": "thread::cccc",
                   "runtime_context": { "thread_id": "thread::cccc",
                                         "channel": "api", "account_id": "main",
                                         "workspace_dir": "/Users/test/repo",
                                         "task_id": null },
                   "external": { "some_unknown_legacy_key": "preserved" } } } }
```

Golden #4 — history response (complete real envelope; provenance is
projected onto the **outer envelope, which is the only place clients
read**; the nested `message` is the as-stored record body, present for
raw access but never consumed for behavior; `internal`, `internal_kind`,
and `loop_origin` are deleted from the envelope — full retirement set):

```json
{ "ok": true,
  "thread": { "thread_id": "thread::aaaa" },
  "session": { "thread_id": "thread::aaaa" },
  "thread_runtime": { "provider_type": "claude_code" },
  "render_schema": 2,
  "messages": [
    { "index": 41, "role": "user", "kind": "user_query",
      "tool_related": false, "likely_user_visible": true,
      "timestamp": "2026-07-21T06:00:00Z",
      "text": "Body",
      "content": "Body",
      "raw_content_type": "string",
      "provenance": { "kind": "task_notification",
                       "event": "ready_for_review", "status": "in_review",
                       "task_id": "#TASK-42", "title": "T",
                       "task_thread_id": "thread::bbbb",
                       "source_run_id": "run-src-1" },
      "message": { "role": "user", "content": "Body",
                    "timestamp": "2026-07-21T06:00:00Z",
                    "provenance": { "kind": "task_notification",
                                     "event": "ready_for_review",
                                     "status": "in_review",
                                     "task_id": "#TASK-42", "title": "T",
                                     "task_thread_id": "thread::bbbb",
                                     "source_run_id": "run-src-1" },
                    "metadata": { "external": {} } } } ],
  "pending_user_inputs": [],
  "message_stats": { "total_messages_in_thread": 100,
                      "total_messages_in_session": 100,
                      "committed_message_count": 100,
                      "returned_messages": 1, "returned_user_queries": 1,
                      "returned_start_index": 41, "returned_end_index": 41,
                      "has_more_before": true, "next_before_index": 40,
                      "has_more_after": false, "next_after_index": null,
                      "reset": false, "user_query_limit": 20,
                      "tool_related_count": 0,
                      "likely_user_visible_count": 1,
                      "pending_user_input_count": 0,
                      "active_pending_user_input_count": 0,
                      "kind_counts": {} } }
```

Golden #5 — history upgrade-required (returned before any `messages`
field is produced):

```json
{ "ok": false, "error": "upgrade_required",
  "required_render_schema": 2, "received_render_schema": null }
```

Grep guards across gateway, desktop renderer, desktop main process, iOS,
CLI: no consumer of `internal`, `internal_kind`, `loop_origin` remains.

### 3.2 External ingress: complete field-level matrix

| Surface | Wire fields kept (typed) | Notes |
|---|---|---|
| chat API | `ExternalRunOverrides` (full set §3.1) | run-scoped; **busy-thread conflict: if admission resolves to `QueuedToActiveRun` and overrides are non-empty, the request is rejected with a structured conflict error** (overrides cannot apply to an already-running provider and must not persist); idle positive + busy rejection tests |
| `AtomicDispatchBody` | `message`, `attachments`, `images`, `files`, `account_id`, `from_id`, generic metadata — exactly today's wire, attachments capability intact | no run overrides added; generic metadata → `external` |
| `CreateThreadBody` | `ThreadRuntimeConfig` fields (model, reasoning, tier, resume, fork, `sdk_session_provider_hint`) | persistent product state; `merge_thread_agent_runtime_snapshot` retired |
| `UpdateThreadBody` | `ThreadRuntimeConfig` updates (model, reasoning, tier) | same |
| Telegram group/topic prompt | server config → trusted `ChannelRuntimeConfig` | not external input |
| built-in channels | attachments, native command text, routing identity as typed `InboundRequest` fields | no extension reinterpretation |
| plugin `deliver_inbound` | **the `extra_metadata` wire field is deleted from the plugin protocol**; a payload carrying it is a protocol error (owner-approved breaking removal — nothing maps into `external`) | — |

#### Managed MCP capability — full security contract (r8 B4)

- **`ManagedMcpCapability`**: a distinct opaque runtime type; never the
  global gateway auth token. Raw token: CSPRNG, ≥256-bit. It implements
  neither `Serialize` nor `Debug`; it exists only in `RuntimeMetadata`
  and is injected only into the provider's MCP transport for that run.
  It never appears in transcripts, run records, thread records, logs,
  error messages, or telemetry (leak tests grep captured logs and
  URLs).
- **Registry**: the gateway holds an ephemeral in-memory map
  `token_digest → { thread_id, run_id, generation }`. MCP request
  context is derived **only** from this mapping. Registry entries are
  revoked on run terminal, abort, and handoff failure; a gateway restart
  clears the registry (all outstanding capabilities die with it).
- **Mismatch = reject**: a valid capability presented with a path or
  header identity that disagrees with its mapping is rejected — the
  request never falls back to path/header/caller identity. Loopback,
  IPv4/IPv6, and redirected requests all validate the same way; no URL
  blocklists.
- Third-party remote MCP servers (external URL-only entries) keep their
  own `Authorization`/custom headers untouched; they can never reach the
  managed context because they hold no capability. Reserved Garyx
  headers (`X-Garyx-Token`, `X-Mcp-Token`, `X-Run-Id`, `X-Thread-Id`,
  `X-Session-Key`, `X-Channel`, `X-Account-Id`, and `Authorization`
  toward managed endpoints) are written last by the server on managed
  connections, ASCII case-insensitively, on both `garyx_mcp_headers` and
  per-server `headers` entries.
- Tests: replay after terminal/abort/restart fails; capability A + path
  identity B rejected; two concurrent runs isolate; forged loopback
  paths without capability rejected; mutation tools unreachable without
  capability; third-party bearer auth preserved; no token in URLs/logs.

Other rules unchanged: `provider_env` / `developer_instructions` /
`desktop_antigravity_env` server-owned; no ingress DTO has a provenance
field; forged provenance/internal-shaped JSON inert in
`metadata.external`; typed-form admission fingerprints (collision test);
stdio MCP shapes rejected externally; envelope restructure + `xml_attr`
control-character encoding + capability-block precondition as before.

### 3.3 Models: flattened presentation payload (confirmed; unchanged)

`presentation: {"kind":"task_notification", event, status, task_id,
title}` / `{"kind":"restart_notice"}`; required fields; unknown kind ⇒
row-level degradation via explicit lossy decoders (dedicated frame tests
both platforms); reducer projects from `message.provenance`, user rows
only; kind never from text; discriminated unions both clients;
full-payload signatures; title/status-only rows-hash upsert; delta
roundtrip; same-seq reseed; real Storybook row.

### 3.4 Clients: full UI contract (r7/r8: zero findings; unchanged)

Four prose parsers deleted; structural envelope decoder only behind
server presentation; trailing user-side card under the shared width
owner (iOS 0.77/0.94 extraction, desktop `.message-bubble.user`,
collapsed-only, no card constants); 10-line-box clamp with injected ε,
post-width measurement, settling re-measure, pure decision in
GaryxMobileCore; expand = controlled Radix Dialog / always-attached
`.garyxFullScreenCover(item:)` on the occurrence owner with immutable
snapshot selection (survives eviction; clears on occurrence change).

### 3.5 History: one boot migration, complete protocol

Reason `structured_presentation_metadata_v1`; marker identity
`(migration_version, import_generation, thread_id)`; ordering: legacy
boot import → migration → serving (no SSE/history before completion).

Per transcript, under the store lock, one atomic file replacement:

1. **Typed-decoder scrub** (r8 B3 — a versioned decoder classifies
   fields; no second hand-written filter table): delete all
   runtime-owned fields (`provider_env`, `system_prompt`,
   `garyx_mcp_auth_token`, `remote_mcp_servers`, `garyx_mcp_headers`,
   `desktop_antigravity_env`, `developer_instructions`,
   `sdk_session_fork`) **and all request-shaped fields and their legacy
   aliases** (`model`, `model_reasoning_effort`, `model_service_tier`,
   `provider_type`, `requested_provider_type`, `workspace_path` and
   message-level workspace aliases) **and all imported
   `ResolvedRunAttribution` names** (`effective_*`, `sdk_session_id` —
   server-derived values are never accepted from history) from every
   committed message and embedded pending. Rebuild
   `DurableRuntimeContext` from authoritative typed state. Unknown
   legacy keys move to `external`; nested sentinel fixtures lock the
   classification. (Legitimate thread configuration lives in the thread
   record as `ThreadRuntimeConfig` — not in message metadata.)
2. **Provenance strip**: delete pre-cutover top-level `provenance` from
   messages and pendings; later generations strip at the import
   boundary.
3. **Normalization without trust upgrade**: legacy scattered keys
   removed (incl. `internal`, `internal_kind`, `loop_origin`); no record
   upgraded; historical task/restart/loop rows stay plain text.
4. Original `seq`/`thread_id`/`run_id`/`timestamp` untouched; text never
   rewritten. First-fork re-homes onto typed `fork_from_*` + session
   binding (tests: no `sdk_session_fork` in transcripts; fresh fork
   resolves).
5. **Derived-state repair, same pass**: SQL projections and thread-record
   count/tail/preview rebuilt **in place**; the appended new-seq
   `range_rewrite` marker forces client refetch.
6. **Activity preservation — exact values** (r8 B5, single decision):
   `activity_seq`, `last_active_at`, `last_message_at`, and
   `recent_threads_meta.activity_seq` keep their **exact pre-migration
   values** via in-place UPDATE (the repository's existing
   restore-in-place precedent). The migration never calls any upsert
   that allocates a new activity sequence; the `range_rewrite` record's
   timestamp serves the ledger/cursor only. Crash retry is idempotent
   and never re-allocates. Rehearsal asserts full SQL rows, the global
   activity counter, keyset cursor values, and multi-page recent-thread
   query results are all byte-identical before/after.
7. **Markers & crash safety** (unchanged): per-generation dedup;
   crash-retry on a rewritten file still repairs SQL; global marker
   last; failure aborts startup. Fault injection at the three seams;
   rehearsal on an isolated real-data copy asserting sentinel secrets
   absent from jsonl and history API.

### 3.6 Wire cutover (confirmed; unchanged)

`render_schema = 2` on **both** the render-state stream and the history
endpoint (structured upgrade-required before any payload; v2 clients
fail fast against old gateways; `old × v2` asserts explicit rejection).
Desktop main-process client, renderer, iOS, CLI switch to provenance in
the same change set. Client caches bump schema and hard-drop v1 bodies +
snapshots together; upgrade/offline-cold-open/marker-refetch tests.
Desktop ships atomically with the gateway; iOS sets a v2 minimum build.

## 4. Non-goals

(unchanged) Bot-channel human rendering; task lifecycle semantics;
notification targets; delivery routing. Restart notices in scope;
retroactive certification rejected.

## 5. Validation (executable matrix)

- **Types**: trybuild (`internal()` sans authority; no provenance slot in
  ingress DTOs; RuntimeMetadata into persistence; bulk run-options
  serialize; ManagedMcpCapability serialize/Debug). Serde roundtrip all
  six variants.
- **Store surface**: general `set/update/patch/update_many_atomic/import`
  accept only untrusted types and sanitize embedded pendings (provenance,
  legacy keys, runtime + request-shaped + `effective_*` fields);
  `apply_live_run_patch` is the only trusted pending write; unrelated
  patches don't corrupt or re-trust live pendings; raw multi-record
  updates can't inject provenance; ACK-after-restart rejects
  raw-imported pendings; `UntrustedImportedMessage` per-path sentinel
  tests (boot import, provider-session import, raw append, rewrite);
  no public raw-`Value` write API remains.
- **Boundary**: sentinel secrets (every RuntimeMetadata key) absent
  across direct, Legacy queue, Durable exact queue, public
  `add_streaming_input`; provenance retained for six variants through
  the real busy queue; goldens #1/#2a/#2b/#3/#4/#5 as checked-in
  fixtures, byte-exact; chat overrides never persist (and reach the
  provider when idle); busy + non-empty overrides → structured conflict;
  Create/Update config persists to the thread record; `effective_*`
  present and distinct from request names; attribution test (A active +
  B queued + multi-ACK; assistant/tool/control rows carry none).
- **Ingress**: per-surface typed-field tests incl. atomic
  images/files/attachments intact; plugin payload with `extra_metadata`
  is a protocol error; stdio MCP rejected; the full §3.2 capability test
  set; reserved-header aliases/case both paths; Telegram config prompt
  via ChannelRuntimeConfig; forged provenance/internal inert (wake still
  fires; no marking; no presentation); typed-form fingerprint collision.
- **Legacy-field retirement**: no stored or served `internal` /
  `internal_kind` / `loop_origin`; grep guards across all five
  consumers; history golden shows absence.
- **Models/clients/prompt**: as §3.3/§3.4 (unknown-kind frame tests;
  captured-fixture reducer tests incl. measured dropped-marker shape;
  width/alignment real-layout; clamp matrix; dialog/cover contracts;
  capability block provider × default/custom × prompt present/blank).
- **Migration**: historical shapes + forged-history negatives (real task
  id, matching thread, legal envelope, plugin-chosen run id, canonical
  provenance via every import path); typed-decoder classification incl.
  nested sentinels and unknown-key→external; secret scrub on jsonl +
  history API (rehearsal); pre-cutover provenance strip; first-fork;
  three-point fault injection; per-generation markers; old-cursor
  refetch; activity: full-row/counter/cursor/multi-page byte-identical.
- **Cutover**: v2×old and old×v2 on stream and history (explicit
  rejection); history golden; upgrade-required golden #5; cache upgrade
  drops bodies + snapshots; offline cold-open.
- **E2E evidence attribution** (unchanged): desktop `app.asar` hash
  before/after + `--app-path`; iOS carrier hash before/after;
  busy-thread E2E to collapsed card → dialog; iOS simulator flow;
  SwiftPM mapper tests from the captured frame.

## 6. Rollout

(unchanged) One change set; desktop (renderer + main) + gateway + CLI
together; migration on first boot before serving; iOS normal release
flow with v2 minimum build; TestFlight only with explicit approval in
that turn. Owner-communicated costs: historical notifications and loop
rows render plain; plugin `extra_metadata` field deleted.
