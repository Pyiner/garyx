# Task Notification Structured Presentation

Status: revision 8 for adversarial review
(r1 3B+8M; r2 1B+7M+3m; r3 2B+5M+2m; r4 3B+3M+1m; r5 3B+1M; r6 4B+4M;
r7 5B+3M+1m. This HEAD is the complete self-contained contract.
Closed by prior rounds and unchanged here: §1 problem statement, §3.4
client/UI contract (zero findings in r7), six-producer inventory,
single-PendingUserInput-constructor convergence, flattened presentation
choice, prose-parser deletion, no-retroactive-trust, capability-block
precondition, owner-approved breaking decisions.)
Owner: Gary (orchestrator thread)
Scope: garyx-models, garyx-router, garyx-bridge, garyx-gateway, garyx
(plugin host + CLI), garyx-channels, desktop renderer **and desktop
main-process gateway client**, iOS

## 1. Problem

Mac and iOS render `<garyx_task_notification>` as raw XML in a large
fraction of cases (live 2026-07-21: 73/252 committed notification records
unmarked — the busy-thread queue path filters dispatch metadata through a
five-key allowlist, dropping all semantics). Four client prose parsers
reverse-parse English templates. Dispatch metadata is one bare
`HashMap<String, Value>`; the direct path persists it minus a two-key
denylist, so provider runtime material already leaks into committed
transcripts. External callers can inject arbitrary keys at several
ingresses.

## 2. Design principles

1. **The write path is the root of trust — not the data shape.** Internal
   dispatchers are the only code paths that attach provenance; every
   untrusted-input path (ingress DTOs, imports, raw record writes)
   structurally cannot carry it; the store write surface is typed
   Trusted/Untrusted.
2. **Run-scoped provider runtime input never enters persistable dispatch
   metadata.** Three distinct data kinds, three types (§3.1): one-shot
   external run overrides (runtime, never persisted), persistent typed
   thread configuration (ordinary product state), and server-derived run
   attribution (durable audit fields that never share names with request
   fields).
3. **Structure is never round-tripped through prose.**
4. **No retroactive trust, no dual mechanisms, no compatibility shims**
   (owner-approved breaking decisions are final: plugin metadata
   pass-through removal, historical loop-summary degradation, history
   envelope hard cut, render_schema=2 hard cutover, no history upgrade).

## 3. Changes

### 3.1 Typed dispatch metadata and provenance

#### The three runtime/config/audit types (r7 B2)

- **`ExternalRunOverrides`** — one-shot, run-scoped, external:
  `model`, `model_reasoning_effort`, `model_service_tier`,
  `provider_type`, `workspace_path`, `system_prompt`,
  `remote_mcp_servers` (URL-only), `garyx_mcp_headers`. Parsed from the
  chat request only (§3.2), feeds `RuntimeMetadata`, **never persisted**.
- **`ThreadRuntimeConfig`** — persistent typed thread configuration
  written by Create/Update thread APIs and internal agent binding:
  `model`, reasoning, service tier, session resume/fork fields,
  `sdk_session_provider_hint`, agent snapshot. Ordinary product state in
  the thread record; not dispatch metadata at all.
- **`ResolvedRunAttribution`** — server-derived audit of what actually
  ran: `effective_model`, `effective_reasoning_effort`,
  `effective_service_tier`, `effective_provider_type`,
  `effective_workspace_dir`, `agent_id`, `agent_display_name`,
  `sdk_session_id`. Durable; field names never collide with request
  fields, so a persisted value can never be mistaken for caller input.

#### Types

```rust
// garyx-models
pub struct DispatchMetadata {
    provenance: Option<ProvenanceRecord>,   // set only via internal()
    pub durable: DurableMetadata,
    runtime: RuntimeMetadata,
}
impl DispatchMetadata {
    pub fn external(durable: DurableMetadata, runtime: RuntimeMetadata) -> Self { … }
    /// Called only by the six internal dispatchers (all in garyx-gateway).
    /// DispatchAuthority (one value, composition root) is a misuse
    /// tripwire — hygiene, not the security boundary.
    pub fn internal(_: &DispatchAuthority, provenance: ProvenanceRecord,
                    durable: DurableMetadata, runtime: RuntimeMetadata) -> Self { … }
    pub fn provider_view(&self) -> ProviderMetadataView<'_> { … }
}

/// Wire/record data (NOT a credential): internally tagged "kind",
/// public read accessors, Serialize + Deserialize.
/// No payload field is named `kind` (serde internal-tag conflict, r7 B1):
/// the restart wake flavor field is `wake_kind`.
pub enum ProvenanceRecord {
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

Serde roundtrip tests cover every variant (restart golden included).

- Producer inventory (legacy keys are migration input only): task
  notification (`task_notification`, `task_notification_event`,
  `task_id`, `task_thread_id`, `task_notification_source_run_id?`);
  restart wake (`restart_wake`, `restart_wake_id/_kind/_target/_all/
  _attempt`); followup — quota resend rides it (`schedule_followup`,
  `_job_id`, `_scheduled_at`, `_scheduled_for`, `_reason?`,
  `_originating_run_id?`); automation (`source=automation`,
  `automation_id`, `cron_action`); cron (`source=cron`, `cron_job_id`,
  `cron_action`); task auto-start (`task_auto_start`,
  `task_dispatch_reason`, `task_id`). Common front door adds
  `internal_dispatch` + MCP token/servers + optional requested provider;
  provider resolution later adds the runtime set. No `LoopContinuation`
  variant (no live producer; legacy keys are migration input).
- **`DurableMetadata`** (typed constructors are the authoritative
  inventory): `ResolvedRunAttribution`; run identifiers
  (`client_run_id`/`run_id`/`bridge_run_id`), `client_intent_id`,
  `origin_id`; conversation context (`channel`, `account_id`, `from_id`,
  `chat_id`, `client`, `is_group`, `thread_binding_key`,
  `delivery_thread_id`, `display_label`, `delivery_target_type`,
  `delivery_target_id`, `resolved_thread_id`, `attachments` refs);
  session forking (`fork_from_thread_id`, `fork_from_sdk_session_id`,
  `fork_from_provider_type`); `runtime_context` rebuilt from
  authoritative typed fields; `external` extension namespace (the only
  home for generic untrusted keys). Request-shaped
  model/provider/workspace fields do **not** exist in durable form.
- **`RuntimeMetadata`** (opaque: private field, per-producer
  constructors, no serde): `ExternalRunOverrides` content plus
  server-side runtime (`garyx_mcp_auth_token`, managed
  `remote_mcp_servers` incl. stdio shapes, `provider_env`,
  `developer_instructions`, `desktop_antigravity_env`,
  `sdk_session_fork`). Providers read `provider_view()`. Persistence
  accepts `&DurableMetadata` (+ provenance); `ProviderRunOptions` loses
  bulk serde for an explicit `PersistedRunOptions` projection; both
  legacy filter constants deleted.

#### Controlled store writes — messages AND thread records

- `TrustedCommittedMessage`: produced only by bridge persistence from a
  live run's `DispatchMetadata`; the only provenance-writing type.
- `UntrustedImportedMessage` (legacy boot import, **provider-session
  import (a live runtime path — `rewrite_from_messages`)**, raw
  append/rewrite/replace): the constructor strips top-level `provenance`,
  legacy internal keys, **and every runtime-owned field**, and rebuilds a
  safe `runtime_context` (r7 B3 — boot scrub cannot cover runtime-era
  imports). Public raw-`Value` message write APIs go private.
- `UntrustedImportedThreadRecord`: every raw thread-record
  set/patch/import sanitizes embedded `pending_user_inputs` the same way
  (provenance, legacy keys, runtime fields).
- Pending trust boundary: persisted pendings deserialize as
  `UntrustedPendingUserInput` and are re-sanitized; ACK accepts only
  trusted pendings from this process's single live constructor.
- The reducer reads `message.provenance` from committed records — safe
  because of this write-surface split.

#### Provenance attribution rules (r7 M1)

- Direct dispatch: provenance attaches to the originating user row only.
- Queued dispatch: provenance attaches to that pending's ACK user row,
  **taking precedence over the active run's own provenance** for that
  row.
- Assistant/tool/control rows never inherit user provenance —
  `build_run_messages`' bulk metadata merge is replaced by per-row typed
  attribution. Test: internal run A active, internal notification B
  queued, multiple ACKs — every row carries exactly its own provenance.

#### Canonical record + history wire (complete goldens, r7 M1)

Golden #1 — direct-path committed user record:

```jsonc
{ "role": "user", "content": "…", "timestamp": "…",
  "provenance": { "kind": "task_notification",
                   "event": "ready_for_review", "status": "in_review",
                   "task_id": "#TASK-42", "title": "…",
                   "task_thread_id": "thread::aaaa", "source_run_id": "run-1" },
  "metadata": { "effective_model": "m", "effective_provider_type": "claude_code",
                 "agent_id": "gary", "channel": "api", "account_id": "main",
                 "resolved_thread_id": "thread::bbbb",
                 "runtime_context": { /* safe schema */ },
                 "external": { } } }
```

Golden #2a — persisted pending (inside the run record):

```jsonc
{ "id": "queued_input:u1", "bridge_run_id": "run-A",
  "origin_run_id": "task-notify-TASK-42-…", "queued_at": "…",
  "origin_id": null, "status": "queued",
  "provenance": { "kind": "task_notification", /* as #1 */ },
  "durable_metadata": { /* as #1 metadata */ } }
```

Golden #2b — ACK committed user record: exactly #1 plus
`"queued_input_id": "queued_input:u1"`, `"queued_at": "…"`, optional
`"origin_id"`, and `"origin_run_id": "task-notify-TASK-42-…"` in
`metadata`; `provenance` is the pending's own (never run A's).

Golden #3 — migrated legacy record: original `role/content/timestamp/seq`
untouched; `provenance` absent; metadata scrubbed of runtime fields and
all legacy scattered keys; `runtime_context` rebuilt safe; `external`
preserved verbatim.

Golden #4 — history response element (envelope carries the same message
objects; `provenance` at the same top level of each message;
`internal`/`internal_kind` absent):

```jsonc
{ "messages": [ { "role": "user", "content": "…", "timestamp": "…",
                   "provenance": { "kind": "restart_notice_or_task…" },
                   "metadata": { /* as stored */ } } ],
  "render_schema": 2, "next_cursor": "…" }
```

### 3.2 External ingress: complete field-level matrix

| Surface | Wire fields kept (typed) | Notes |
|---|---|---|
| chat API | `ExternalRunOverrides`: `model`, reasoning, tier, `system_prompt` (request > thread > agent precedence), `workspace_path`, `provider_type`, `remote_mcp_servers` (URL-only), `garyx_mcp_headers` | run-scoped, never persisted; generic metadata → `external` |
| `AtomicDispatchBody` | message, attachments, account/from, generic metadata only — **exactly today's surface; no run overrides added** (r7 M2). Thread-level fields belong to the enclosing `CreateThreadBody`. | generic metadata → `external` |
| `CreateThreadBody` | `ThreadRuntimeConfig` product fields: `model`, reasoning, tier, session resume, fork, `sdk_session_provider_hint` | persistent thread state; no run-scoped overrides; `merge_thread_agent_runtime_snapshot` bare-metadata pickup retired |
| `UpdateThreadBody` | `ThreadRuntimeConfig` updates: `model`, reasoning, tier | same |
| Telegram group/topic prompt | server-side config → trusted `ChannelRuntimeConfig` | not external input |
| built-in channels | prompt attachments, native command text, routing identity as typed `InboundRequest` fields | providers never reinterpret extension keys |
| plugin `deliver_inbound` | none (owner-approved breaking removal of the metadata pass-through) | extension namespace only |

#### Garyx MCP identity: capability, not header filtering (r7 B4)

Header stripping cannot secure the managed MCP context: the Garyx MCP
route derives run/thread identity from the URL path, and loopback
requests bypass gateway auth — an external `remote_mcp_servers` entry
pointing at `http://127.0.0.1:…/mcp/<victim-thread>/<forged-run>` would
inherit a forged identity regardless of headers. And stripping
`Authorization` from third-party servers would break their legitimate
auth. Therefore:

- **Managed Garyx MCP**: the gateway mints an unforgeable, run/thread-
  bound capability token per run and injects it server-side; the MCP
  route validates the capability on **every** request — including
  loopback — and ignores path-supplied identity that lacks a matching
  capability. Mutation tools (schedule/capsule/…) are reachable only
  with a valid capability.
- **Third-party remote MCP** (external URL-only entries): their
  `Authorization`/custom headers pass through untouched — they cannot
  reach the managed context because they hold no capability, not because
  of header hygiene. Reserved Garyx headers (`X-Garyx-Token`,
  `X-Mcp-Token`, `X-Run-Id`, `X-Thread-Id`, `X-Session-Key`,
  `X-Channel`, `X-Account-Id`, `Authorization` toward *managed*
  endpoints) are still written last by the server on managed
  connections, with ASCII case-insensitive matching applied after
  merging (both `garyx_mcp_headers` and per-server `headers` entries).
- Tests: forged path identity via localhost, IPv4/IPv6 loopback,
  redirects; capability-less mutation calls rejected; third-party bearer
  auth preserved; header aliases/mixed case on both paths. No URL
  blocklists.

Other rules (unchanged): `provider_env`, `developer_instructions`,
`desktop_antigravity_env` server-owned everywhere; no ingress DTO has a
provenance field; forged provenance/internal-shaped JSON lands inert in
`metadata.external` (no lifecycle flip, no internal marking, no
presentation); admission fingerprints computed over the typed form;
stdio MCP shapes rejected externally.

#### Envelope + producer (unchanged)

`<garyx_task_notification event=… task_id=… status=… title=…>
{final_message}</garyx_task_notification>`; `xml_attr` escapes `& " < >`
and CR/LF/TAB as numeric character references (multiline-title test);
close-tag neutralization stays. Headline + CLI tutorial deleted from the
body only after the brand-free lifecycle capability block ships for
provider × default/custom × prompt present/blank (approve →
`garyx task update <id> --status done`; needs changes →
`garyx thread send task '<id>' "<feedback>"`, which auto-wakes and flips
`in_review → in_progress`; manual status updates are CLI-rejected).

### 3.3 Models: flattened presentation payload

Rationale: `kind` directly forms the TS/Swift discriminated union; no
second `payload` wrapper; all payload fields participate in `Hash`/row
signatures.

```json
"presentation": { "kind": "task_notification",
                  "event": "ready_for_review", "status": "in_review",
                  "task_id": "#TASK-42", "title": "…" }
```

```json
"presentation": { "kind": "restart_notice" }
```

- All four `task_notification` fields required; unknown `kind` ⇒ the row
  renders with no presentation. **Forward-safety is an explicit decoder
  contract** (r7 minor): iOS uses a lossy/custom decoder so an unknown
  discriminator degrades that row only — never fails the frame; desktop
  narrows unknown kinds to undefined. Dedicated unknown-kind frame tests
  on both platforms (not folded into malformed tests).
- Rust enum with struct variants, serde `tag = "kind"`; golden JSON locks
  both kinds; reducer projects from `message.provenance` for user-role
  rows; kind never derived from text.
- Impact closure: desktop `transcript.ts` discriminated union; iOS
  object decoder + full-payload signature hash; title/status-only
  rows-hash delta upsert; delta roundtrip; same-seq reseed; real
  Storybook user render row.

### 3.4 Clients: full UI contract (r7: zero findings; unchanged)

- Delete all four prose parsers; one structural envelope decoder per
  platform, reachable only behind a server presentation; `statusLabel`
  formatting client-side.
- User-role trailing alignment under the shared owner: iOS extracts the
  existing user container (0.77×/0.94× Dynamic Type widths, spacer
  60/12, trailing menu edge) for both bubbles and card — the full-width
  leading card branch is deleted; desktop deletes the card's
  `align-self:flex-start; max-width:100%` overrides and inherits
  `.message-bubble.user { max-width:77%; align-self:flex-end }`; no
  card-local constants; width rules bind the collapsed card only.
- Collapsed card: header (task id, status, title) + body clamped to 10
  line-boxes at the current body font; pure
  `overflows(naturalHeight, clampHeight, ε)` with injected ε, measured
  after shared width, re-measured on content/width/font/Dynamic Type and
  intrinsic settling (image load, font resolution, markdown re-layout,
  width reflow); decision in `GaryxMobileCore` (desktop mirrors the
  split); overflow ⇒ affordance + a11y action, fits ⇒ neither; expand
  activation excludes interactive descendants.
- Expanded: same payload + same single envelope strip captured as an
  immutable header/body snapshot at activation. Desktop: controlled
  Radix Dialog, stable owner, focus trap/Escape/focus return. iOS:
  always-attached `.garyxFullScreenCover(item:)` on the stable
  occurrence owner; selection identity = message seq/id; snapshot
  survives render-window eviction; selection clears on
  thread/gateway/occurrence change; modifiers never conditionally
  attached.

### 3.5 History: one boot migration, complete protocol

Reason `structured_presentation_metadata_v1`; per-thread marker identity
`(migration_version, import_generation, thread_id)`.

**Ordering**: legacy boot import → migration → gateway serves clients
(no SSE/history before completion).

Per transcript, under the store lock, one atomic file replacement:

1. **Secret scrub**: remove runtime-owned fields (`provider_env`,
   `system_prompt`, `garyx_mcp_auth_token`, `remote_mcp_servers`,
   `garyx_mcp_headers`, `desktop_antigravity_env`,
   `developer_instructions`, `sdk_session_fork`) from every committed
   message and embedded pending; rebuild legacy `runtime_context`.
   First-fork re-homes onto typed `fork_from_*` + session binding
   (tests: no `sdk_session_fork` in transcripts; fresh fork resolves).
2. **Provenance strip**: delete pre-cutover top-level `provenance` from
   messages and pendings; later generations strip at the import boundary.
3. **Normalization without trust upgrade**: legacy scattered keys removed
   (incl. `internal`, `internal_kind`, `loop_origin`); no record upgraded
   to provenance; historical task/restart/loop rows keep plain text.
4. Original `seq`/`thread_id`/`run_id`/`timestamp` untouched; text never
   rewritten.
5. **Derived-state repair, same pass**: SQL projections and thread-record
   count/tail rebuilt; appended new-seq `range_rewrite` marker (same
   atomic replacement) forces client refetch.
6. **Activity preservation algorithm** (r7 M3 — the default marker path
   would bubble every migrated thread to the top): the `range_rewrite`
   record's timestamp serves the ledger/cursor only;
   `last_message_at` / `last_active_at` / `activity_seq` are rebuilt from
   the last non-control user-visible content — or explicitly restored to
   their pre-migration values — in the same transaction that appends the
   marker and repairs projections. Assertion: recent-thread ordering is
   byte-identical before/after migration on the rehearsal copy.
7. **Markers & crash safety**: per-generation dedup; crash-retry on an
   already-rewritten file still repairs SQL before completion; global
   marker `(migration_version, import_generation)` last; failure aborts
   startup and retries. Fault injection: crash after N of M transcripts;
   after file rename before SQL repair; after per-thread work before the
   global marker. Rehearsal on an isolated real-data copy asserting
   sentinel secrets absent from jsonl and history API.

### 3.6 Wire cutover: versioned on both surfaces (r7 B5)

- **Render-state stream**: clients declare `render_schema = 2`; frames
  echo it; v2 gateway answers missing/old schema with structured
  upgrade-required before SSE; v2 clients fail fast on schema-less
  frames.
- **History endpoint**: also versioned — history requests carry
  `render_schema=2`; the v2 gateway returns a structured
  upgrade-required error (not messages) for missing/old schema; v2
  clients fail fast against old gateways. Rationale: the desktop main
  process decodes `internal`/`internal_kind` as required fields today
  and history can be fetched before any SSE negotiation, so an
  unversioned subtractive change would crash old clients mid-fetch —
  the hard cut needs an explicit gate, not luck. `old × v2` tests assert
  explicit rejection, not silent degradation. The desktop main-process
  gateway client is in scope and switches to provenance in the same
  change set (with renderer, iOS, CLI task-progress annotation).
- **Client persistent caches**: desktop and iOS bump cache schema and
  hard-drop v1-era **message bodies and render snapshots together** on
  first v2 launch; offline cold-open shows the loading path until
  refetch. Tests: upgrade, offline cold-open, marker refetch.
- Desktop ships atomically with the gateway; iOS sets a v2 minimum
  supported build.

## 4. Non-goals

Bot-channel human rendering; task lifecycle semantics; notification
targets; delivery routing. Restart notices in scope. Retroactive
certification rejected, not deferred.

## 5. Validation (executable matrix)

- **Types**: trybuild — `internal()` without authority; ingress DTOs
  have no provenance slot; RuntimeMetadata into persistence; bulk
  run-options serialize. Serde roundtrip for all six provenance variants
  (incl. RestartWake `wake_kind` golden).
- **Store surface**: `UntrustedImportedMessage` strips provenance +
  legacy keys + **runtime fields** and rebuilds `runtime_context` across
  boot import / provider-session import (`rewrite_from_messages`) / raw
  append / rewrite — sentinel secrets per path;
  `UntrustedImportedThreadRecord` sanitizes embedded pendings on
  set/patch/import; persistence merge re-sanitizes restored pendings;
  ACK rejects non-live pendings; no public raw-`Value` write API remains.
- **Boundary**: sentinel secrets (every RuntimeMetadata key) absent from
  run records and transcripts across direct, Legacy queue, Durable exact
  queue, public `add_streaming_input`; provenance retained for all six
  variants through the real busy-thread queue; goldens #1/#2a/#2b/#3/#4
  exact. Chat/atomic overrides never persist (positive: they reach the
  provider); Create/Update config persists to the thread record
  (positive product test); `effective_*` attribution fields present and
  distinct from request names. Attribution: run A active + internal B
  queued + multiple ACKs — per-row provenance exact; assistant/tool/
  control rows carry none.
- **Ingress**: per surface — typed fields work; atomic dispatch has no
  overrides (golden matches today's wire); stdio MCP rejected; managed
  MCP capability validated on loopback/IPv4/IPv6/redirect forged paths,
  capability-less mutation rejected, third-party bearer auth untouched;
  reserved-header aliases/mixed case on both header paths end with
  server values; Telegram config prompt via ChannelRuntimeConfig;
  plugin pass-through removal verified; forged provenance/internal inert
  (wake fires, no marking, no presentation); typed-form fingerprint
  collision test.
- **Legacy-field retirement**: no stored `internal`/`internal_kind`; no
  consumer in gateway, desktop renderer, desktop main process, iOS, CLI
  (grep guard); history golden shows both absent.
- **Models**: golden JSON both presentation kinds; captured-fixture
  reducer tests (new, post-migration, measured dropped-marker);
  malformed negatives; **dedicated unknown-kind frame tests on both
  platforms (row degrades, frame survives)**; title/status-only
  rows-hash delta upsert; delta roundtrip; same-seq reseed.
- **Clients**: mapper tests from captured v2 frames; no-text-kind guard;
  unmarked-identical-text renders plain; real Storybook row;
  width/alignment real-layout both platforms (same trailing edge/max
  width vs long user text, across resize and both Dynamic Type branches;
  no card-local constants; full-width assertion inverted); clamp matrix
  (exact-fit injected ε, long wrapping line, 11 lines,
  list/code/table/image, overflow→fit on widening, Dynamic Type, late
  settling); no affordance on short bodies; gesture coexistence; dialog
  focus trap/Escape/return; snapshot survives row eviction;
  occurrence/gateway-switch dismissal.
- **Prompt**: capability block for provider × default/custom × prompt
  present/blank, approve + feedback rules per provider envelope.
- **Migration**: all historical shapes; forged-history negatives (real
  task id, matching thread, legal envelope, plugin-chosen run id,
  canonical provenance via every import path); secret scrub on jsonl +
  history API (rehearsal); pre-cutover provenance strip; first-fork
  correctness; three-point fault injection; per-generation marker
  dedup/regeneration; old-cursor refetch; **recent-thread ordering
  byte-identical before/after on the rehearsal copy**.
- **Cutover**: v2×old and old×v2 on **both** stream and history
  (explicit structured rejection, no silent degradation); history
  envelope golden; cache upgrade drops bodies + snapshots together;
  offline cold-open.
- **E2E evidence attribution**: desktop worktree ↔ installed `app.asar`
  hash before/after + renderer `--app-path` check; iOS worktree ↔
  installed carrier (debug.dylib/executable) hash before/after;
  busy-thread `--notify current-thread` E2E → committed record →
  collapsed card → dialog; iOS xcodebuild + simulator collapsed →
  full-screen → complete body → occurrence-switch dismissal; SwiftPM
  mapper tests from the captured frame.

## 6. Rollout

One change set; desktop (renderer + main process) + gateway + CLI ship
together; migration on first boot before serving; iOS via normal release
flow with the v2 minimum build; TestFlight only with explicit user
approval in that turn. Owner-communicated product costs: historical
task/restart notifications and loop-continuation rows render as plain
text; plugin arbitrary-metadata pass-through removed.
