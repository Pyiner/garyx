# Task Notification Structured Presentation

Status: revision 7 for adversarial review
(r1 3B+8M; r2 1B+7M+3m; r3 2B+5M+2m; r4 3B+3M+1m; r5 3B+1M; r6 4B+4M.
r6 lesson applied: the HEAD document is the complete contract — nothing
"frozen" lives only in git history. This revision is self-contained.)
Owner: Gary (orchestrator thread)
Scope: garyx-models, garyx-router, garyx-bridge, garyx-gateway, garyx
(plugin host + CLI), garyx-channels, desktop renderer, iOS

## 1. Problem

Mac and iOS render `<garyx_task_notification>` as raw XML in a large
fraction of cases (live 2026-07-21: 73/252 committed notification records
unmarked — the busy-thread queue path filters dispatch metadata through a
five-key allowlist, `run_management.rs:107`, dropping all semantics).
Four client prose parsers (task + restart, desktop + iOS) reverse-parse
English templates word-for-word. Dispatch metadata is one bare
`HashMap<String, Value>` from plan to persistence; the direct path
persists it minus a two-key denylist, so provider runtime material
(`provider_env`, `system_prompt`, MCP token/headers/servers, antigravity
env, `sdk_session_fork`) **already leaks into committed transcripts**.
External callers can inject arbitrary keys at several ingresses.

## 2. Design principles

1. **The write path is the root of trust — not the data shape.** Serde
   types cannot authenticate origin (no friend crates; `Deserialize` is
   not visibility-scoped). Trust holds because: internal dispatchers are
   the only code paths that attach provenance; every untrusted-input path
   (ingress DTOs, imports, raw record writes) structurally cannot carry
   it; the store write surface is typed Trusted/Untrusted.
2. **Run-scoped provider runtime input never enters persistable dispatch
   metadata** (transcripts, run records, pending inputs). Typed
   thread/agent *configuration* (model, reasoning, tier on a thread) is
   ordinary persistent product state and is out of this rule's scope.
3. **Structure is never round-tripped through prose.** Producers persist
   structured objects; the reducer projects them; clients dumb-render.
4. **No retroactive trust, no dual mechanisms.** History is never
   upgraded; no legacy field survives as a parallel signal.

## 3. Changes

### 3.1 Typed dispatch metadata and provenance

#### Types

```rust
// garyx-models
pub struct DispatchMetadata {
    provenance: Option<ProvenanceRecord>,  // read accessor; set only via internal()
    pub durable: DurableMetadata,
    runtime: RuntimeMetadata,
}
impl DispatchMetadata {
    /// Every external ingress parse path. No provenance parameter exists.
    pub fn external(durable: DurableMetadata,
                    runtime: RuntimeMetadata) -> Self { … }
    /// Called only by the internal dispatchers (all in garyx-gateway).
    /// DispatchAuthority (one value, composition root) is a misuse
    /// tripwire — hygiene, not the security boundary.
    pub fn internal(_: &DispatchAuthority, provenance: ProvenanceRecord,
                    durable: DurableMetadata, runtime: RuntimeMetadata) -> Self { … }
    pub fn provider_view(&self) -> ProviderMetadataView<'_> { … } // read-only combine
}

/// Wire/record data (NOT a credential): internally tagged "kind",
/// public read accessors, Serialize + Deserialize.
pub enum ProvenanceRecord {
    TaskNotification { event: String, status: String, task_id: String,
                       title: String, task_thread_id: Option<String>,
                       source_run_id: Option<String> },
    RestartWake { id: String, kind: String, target: Option<String>,
                  all: bool, attempt: u32 },
    Followup { job_id: String, scheduled_at: String, scheduled_for: String,
               reason: Option<String>, originating_run_id: Option<String> },
    Automation { automation_id: String, cron_action: String },
    Cron { cron_job_id: String, cron_action: String },
    TaskAutoStart { task_id: String, dispatch_reason: String },
}
```

- Producer inventory (replaces these live legacy keys, which remain only
  as migration input): task notification (`task_notification`,
  `task_notification_event`, `task_id`, `task_thread_id`,
  `task_notification_source_run_id?`); restart wake (`restart_wake`,
  `restart_wake_id/_kind/_target/_all/_attempt`); followup — quota resend
  rides it (`schedule_followup`, `_job_id`, `_scheduled_at`,
  `_scheduled_for`, `_reason?`, `_originating_run_id?`); automation
  (`source=automation`, `automation_id`, `cron_action`); cron
  (`source=cron`, `cron_job_id`, `cron_action`); task auto-start
  (`task_auto_start`, `task_dispatch_reason`, `task_id`). Common front
  door adds `internal_dispatch` + MCP token/servers + optional requested
  provider (`internal_inbound.rs:169`); provider resolution later adds
  the runtime set. There is no `LoopContinuation` variant (no live
  producer); its legacy keys are migration input only.
- `DurableMetadata` (typed constructors = authoritative inventory, doc
  comments in code): attribution scalars (`agent_id`,
  `agent_display_name`, `model`, `model_reasoning_effort`,
  `model_service_tier`, `requested_provider_type`, run identifiers,
  `client_intent_id`, `origin_id`, `sdk_session_id`); conversation
  context (`channel`, `account_id`, `from_id`, `chat_id`, `client`,
  `is_group`, `thread_binding_key`, `delivery_thread_id`,
  `display_label`, `delivery_target_type`, `delivery_target_id`,
  `resolved_thread_id`, `workspace_dir`, `attachments` refs); session
  forking (`fork_from_thread_id`, `fork_from_sdk_session_id`,
  `fork_from_provider_type`); `runtime_context` rebuilt from
  authoritative typed fields (never cloned from caller input);
  `external` — the extension namespace, the only place generic untrusted
  keys live.
- `RuntimeMetadata` (opaque: private field, per-producer constructors, no
  serde): `garyx_mcp_auth_token`, `remote_mcp_servers`,
  `garyx_mcp_headers`, `provider_env`, `system_prompt`,
  `developer_instructions`, `desktop_antigravity_env`,
  `sdk_session_fork`. Providers read `provider_view()`. Persistence APIs
  accept `&DurableMetadata` (+ provenance); `ProviderRunOptions` loses
  its bulk serde derive — run records serialize an explicit
  `PersistedRunOptions` projection. `RUNTIME_ONLY_METADATA_KEYS` and
  `DISPATCH_ATTRIBUTION_METADATA_KEYS` are deleted.

#### Controlled store writes — messages AND thread records (r6 B1)

- `TrustedCommittedMessage`: produced only by bridge persistence from a
  live run's `DispatchMetadata`; the only type whose top-level
  `provenance` is written through.
- `UntrustedImportedMessage`: legacy boot import, provider-session
  import, raw append/rewrite/replace. Constructor unconditionally strips
  top-level `provenance` + legacy scattered keys. Public raw-`Value`
  message write APIs go private / are re-exposed only as this type.
- **`UntrustedImportedThreadRecord`**: every raw thread-record
  `set`/patch/import path (legacy boot import hands whole records to
  `ThreadStore::set(Value)`) passes here first; it sanitizes embedded
  `pending_user_inputs` — stripping pending `provenance`, legacy internal
  keys, and runtime fields.
- **Pending trust boundary**: when bridge persistence merges previously
  persisted pending inputs (`pending_inputs_from_value`), they
  deserialize as `UntrustedPendingUserInput` and are re-sanitized;
  **ACK accepts only trusted pendings created by this process's single
  live constructor** (the one convergence point of all four enqueue
  paths — Legacy, Durable exact, `execute_durable_stream_input`, public
  `add_streaming_input`). A restored/imported pending re-enters as
  untrusted data and can never mint provenance.
- The reducer reads `message.provenance` from committed records — safe
  because of this write-surface split, not because of the value's type.

#### Canonical record + history wire — no legacy aliases (r6 B4)

- **Stored record**: top-level `provenance` is the only stored truth.
  New records never store `internal` / `internal_kind`; nothing reads
  them.
- **History response**: the message envelope adds `provenance` and
  **drops `internal`/`internal_kind` entirely** — no derived-alias
  projection (that was a dual mechanism). Desktop, iOS, and the CLI's
  task-progress annotation switch to reading provenance in the same
  change set (desktop+gateway atomic; CLI same repo; iOS under the v2
  minimum build). Historical loop-continuation rows lose their summary
  and render as plain text — accepted product cost, consistent with
  no-retroactive-trust. Old clients decode the absent fields as nil and
  degrade to plain display without crashing; the additive+subtractive
  envelope change is documented in the golden fixtures (stored trio +
  full history response).

Golden JSON #1 (direct-path committed record):

```jsonc
{ "role": "user", "content": "…", "timestamp": "…",
  "provenance": { "kind": "task_notification",
                   "event": "ready_for_review", "status": "in_review",
                   "task_id": "#TASK-42", "title": "…",
                   "task_thread_id": "thread::…", "source_run_id": "…" },
  "metadata": { "agent_id": "…", "model": "…", "channel": "…",
                 "external": { } } }
```

Golden #2 (queued): same + `queued_input_id`, `queued_at`, `origin_id?`,
`origin_run_id`. Golden #3 (migrated legacy): scrubbed metadata, no
provenance, no legacy keys. Golden #4: full history response envelope.

### 3.2 External ingress: complete field-level matrix (r6 B2)

| Surface | Wire fields kept (typed) | Rejected / removed |
|---|---|---|
| chat / `AtomicDispatchBody` | `model`, `model_reasoning_effort`, `model_service_tier`, `system_prompt` (request > thread > agent precedence), `workspace_path` (cwd selection), `provider_type` (provider selection), `remote_mcp_servers` (URL-only schema), `garyx_mcp_headers` (minus reserved) | generic metadata → `external` namespace; stdio MCP shapes rejected |
| `CreateThreadBody` | `model`, reasoning, service tier, session resume, fork, `sdk_session_provider_hint` (persistent typed thread configuration — legitimate product state per §2.2) | run-scoped runtime overrides rejected; agent snapshot via internal-only `ThreadRuntimeConfig`; `merge_thread_agent_runtime_snapshot` bare-metadata pickup retired |
| `UpdateThreadBody` | `model`, reasoning, service tier (existing typed updates) | same as create |
| Telegram group/topic prompt | server-side channel configuration → trusted `ChannelRuntimeConfig` (not external input, not extension) | — |
| built-in channels (`garyx-channels`) | prompt attachments, native command text, routing identity as dedicated typed `InboundRequest` fields | providers never reinterpret extension keys |
| plugin `deliver_inbound` | none | **breaking removal, owner-accepted**: today's arbitrary `extra_metadata` pass-through to router/provider ends; plugins get the extension namespace only. Stated explicitly — this *is* a shipped-capability removal, done deliberately as a security hard-cut. |

Header hard rules (both header-bearing paths: `garyx_mcp_headers` and
per-server `headers` inside URL-only `remote_mcp_servers` entries — the
per-entry headers pass the **same** filter, closing the bypass):

- Reserved set (server writes last; external values discarded):
  `Authorization`, `X-Run-Id`, `X-Thread-Id`, `X-Session-Key`,
  `X-Garyx-Token`, `X-Mcp-Token`, `X-Channel`, `X-Account-Id`.
- Matching is **ASCII case-insensitive** and applied after merging, so
  `x-run-id`/`X-RUN-ID` aliases cannot coexist-and-win in the
  intermediate `HashMap`.
- `provider_env`, `developer_instructions`, `desktop_antigravity_env`
  stay server-owned everywhere.
- No ingress DTO has a provenance field; forged provenance/internal
  shaped JSON lands inert in `metadata.external` (no lifecycle flip —
  task `in_review→in_progress` wake still fires; no internal marking; no
  presentation).
- Admission fingerprints computed over the typed request form.

#### Envelope + producer (unchanged since r3, restated)

`deliver_task_review_handoff` constructs the TaskNotification provenance
and body `<garyx_task_notification event=… task_id=… status=… title=…>
{final_message}</garyx_task_notification>`; `xml_attr` escapes
`& " < >` **and CR/LF/TAB as numeric character references** (multiline
title test); body close-tag neutralization stays. The prose headline and
CLI tutorial block are deleted from the body **only after** the
brand-free task-lifecycle capability block ships in base instructions for
provider × default/custom × prompt present/blank: approve →
`garyx task update <id> --status done`; needs changes →
`garyx thread send task '<id>' "<feedback>"` (auto-wakes and flips
`in_review → in_progress`; manual status updates are rejected by the
CLI). Provider-envelope tests across the matrix.

### 3.3 Models: flattened presentation payload

Why flattened (decision rationale): `kind` directly forms the TS/Swift
discriminated union, avoids a second `payload` wrapper layer, and every
payload field naturally participates in `Hash`/row signatures.

```json
"presentation": { "kind": "task_notification",
                  "event": "ready_for_review", "status": "in_review",
                  "task_id": "#TASK-42", "title": "…" }
```

```json
"presentation": { "kind": "restart_notice" }
```

- All four presentation fields required for `task_notification`; unknown
  `kind` values ⇒ clients treat the row as having no presentation
  (forward-safe).
- Rust: enum with struct variants, serde `tag = "kind"`; golden JSON
  locks both kinds. Reducer projects from `message.provenance`
  (TaskNotification / RestartWake), user-role rows only;
  missing/malformed/other ⇒ no presentation ⇒ ordinary text; kind is
  never derived from message text.
- Impact closure: desktop `transcript.ts` string enum → discriminated
  object (every comparison site updated); iOS decoder single-string →
  object; iOS message signature hashes the complete payload; server
  rows_hash test that a title/status-only change produces a delta
  upsert; delta roundtrip + same-seq reseed with object presentations;
  Storybook scenario becomes a real user render row with a real
  presentation object.

### 3.4 Clients: full UI contract

- **Delete all four prose parsers** (desktop `task-notification.ts` +
  `restart-notice.ts`; iOS `GaryxTaskNotificationPresentation` regexes +
  `GaryxRestartNoticePresentation`). One structural envelope decoder per
  platform (after opening tag's `>` … before last close tag;
  wording-independent; legacy-body tolerant), reachable **only** behind a
  server presentation. `statusLabel` formatting stays client-side.
- **User-role layout owner** (role guaranteed by committed record +
  reducer; clients never infer placement from presentation kind). The
  card aligns **trailing** under the same owner as ordinary user bubbles:
  - iOS: extract the existing user-role container — trailing alignment,
    Dynamic Type width `0.77 ×` screen normal / `0.94` at `xxxLarge+`,
    min leading spacer 60/12, trailing menu edge — into a shared owner
    used by plain user bubbles and the card. The current full-width
    leading `taskNotificationRow` branch is deleted. Copying `0.77` or
    minting a card-specific constant is forbidden.
  - Desktop: `.message-bubble.user { max-width: 77%; align-self:
    flex-end }` is the owner; the card's current
    `align-self: flex-start; max-width: 100%` overrides are deleted; the
    card may fill the shared cap but declares no constants.
  - Width rules bind the **collapsed transcript card only**; the expanded
    surfaces are not constrained by them.
- **Collapsed card**: header (task id, status, title) + body viewport
  clamped to the height of **10 line-boxes at the current body font**.
  Overflow decision: pure function `overflows(naturalHeight, clampHeight,
  ε)` with ε an injected parameter; measured **after** the shared
  user-role width applies; re-measured on content, width, font scale,
  Dynamic Type, and intrinsic settling (image load, font resolution,
  markdown table/code re-layout, width reflow). The pure decision lives
  in `GaryxMobileCore` (SwiftUI supplies only a measurement adapter);
  desktop mirrors the same split. Overflow ⇒ expand affordance + a11y
  action; fits ⇒ neither (no dead interaction). Expand activation
  excludes interactive descendants (links, copy/select/share, long-press
  menus work in both states).
- **Expanded**: header + complete body from the **same structured payload
  and the same single envelope strip** captured as an immutable
  header/body snapshot at activation:
  - Desktop: controlled Radix `Dialog`, stable owner, focus trap, Escape,
    focus returns to the card on close; standard large-dialog sizing.
  - iOS: full-screen presentation owned by the stable conversation/route
    occurrence owner via one always-attached
    `.garyxFullScreenCover(item:)` (capsule-preview pattern); rows only
    send selection actions; selection identity is message seq/id (never
    task_id — one task can notify repeatedly); the snapshot keeps the
    body complete even if the row is evicted by the render window;
    selection clears on thread/gateway/occurrence change; modifiers are
    never conditionally attached.

### 3.5 History: one boot migration, complete protocol

Reason `structured_presentation_metadata_v1`; per-thread marker identity
`(migration_version, import_generation, thread_id)`.

**Ordering**: legacy boot import → this migration → gateway begins
accepting SSE/history clients. No client is served mid-migration.

Per transcript, under the store lock, in one atomic file replacement:

1. **Secret scrub**: remove runtime-owned fields (`provider_env`,
   `system_prompt`, `garyx_mcp_auth_token`, `remote_mcp_servers`,
   `garyx_mcp_headers`, `desktop_antigravity_env`,
   `developer_instructions`, `sdk_session_fork`) from every committed
   message **and** embedded `pending_user_inputs`; rebuild legacy
   `runtime_context` to the safe schema. First-fork behavior re-homes
   onto typed `fork_from_*` + current session binding (validation:
   transcripts contain no `sdk_session_fork`; a fresh fork still
   resolves).
2. **Provenance strip**: delete any pre-cutover top-level `provenance`
   from committed messages and pendings. Later import generations strip
   at the import boundary (`UntrustedImportedMessage` /
   `UntrustedImportedThreadRecord`) — no recurring global rescans.
3. **Normalization without trust upgrade**: legacy scattered
   task/restart/internal keys removed (incl. `internal`,
   `internal_kind`, `loop_origin`). **No record is upgraded to
   provenance** — there is no provable ledger (task events die with task
   deletion; restart pending files are deleted on success; plugins could
   supply arbitrary metadata and self-chosen `task-notify-*` run ids).
   Historical task/restart/loop rows keep plain-text rendering
   permanently.
4. Original `seq`/`thread_id`/`run_id`/`timestamp` untouched; message
   text is history and never rewritten.
5. **Derived-state repair, same pass**: SQL projections and thread-record
   count/tail rebuilt for changed transcripts; user-facing activity
   ordering not bumped. Because payloads changed at existing seqs, the
   appended new-seq `range_rewrite` control record (same atomic
   replacement) is what forces every client to drop and refetch — and
   client-side both the transcript/message body caches **and** the render
   snapshot caches are versioned and hard-dropped (§3.6), not just the
   render snapshots.
6. **Markers & crash safety**: per-thread marker dedups only within the
   same import generation (a later generation rewrites again under a new
   marker); a crash retry that finds a file already rewritten still
   repairs SQL projections before recording completion; the global marker
   keyed to `(migration_version, import_generation)` is written last;
   any failure aborts startup and retries next boot. Fault injection: (a)
   crash after N of M transcripts, (b) crash after file rename before SQL
   repair, (c) crash after per-thread work before the global marker.
   Rehearsal on an isolated copy of the real production data directory,
   asserting sentinel secrets absent from transcript jsonl **and**
   `/api/threads/history`.

### 3.6 Wire cutover

- `render_schema = 2` declared by clients on the render-state stream;
  frames echo it; v2 gateway answers missing/old schema with structured
  upgrade-required before SSE; v2 clients fail fast on schema-less
  frames. No string-presentation wire is ever emitted by a v2 gateway.
- History envelope change (add `provenance`, drop
  `internal`/`internal_kind`) ships in the same cutover; consumers
  (desktop, iOS, CLI) switch in the same change set.
- **Client persistent caches**: desktop transcript cache and iOS
  `GaryxTranscriptCache` bump their schema version and hard-drop v1-era
  **message bodies and render snapshots together** on first v2 launch
  (offline cold-open shows the loading path until refetch). Tests:
  upgrade path, offline cold-open, range_rewrite-marker refetch.
- Desktop ships atomically with the gateway; iOS sets a v2 minimum
  supported build.

## 4. Non-goals

Bot-channel human rendering; task lifecycle semantics; notification
targets; delivery routing. Restart notices are in scope. Retroactive
certification is rejected, not deferred.

## 5. Validation (executable matrix)

- **Types**: trybuild — `internal()` without authority fails; ingress
  DTOs have no provenance slot; RuntimeMetadata into persistence fails;
  bulk run-options serialize fails.
- **Store surface**: `UntrustedImportedMessage` strips provenance across
  boot import / provider import / raw append / rewrite;
  `UntrustedImportedThreadRecord` sanitizes embedded pendings on
  set/patch/import; persistence merge of restored pendings re-sanitizes;
  ACK rejects non-live pendings; no public raw-`Value` write API remains.
- **Boundary**: sentinel secrets (every RuntimeMetadata key) absent from
  run records and transcripts across direct, Legacy queue, Durable exact
  queue, public `add_streaming_input`; provenance retained for all six
  variants through the real busy-thread queue; goldens #1–#4 exact.
- **Ingress**: per surface (chat, atomic, create-thread + copy-through,
  update-thread, each built-in channel, plugin): typed fields reach the
  provider and never persist; stdio MCP rejected; reserved-header spoof
  (every alias, mixed case, both header paths incl. per-server entries,
  local Garyx URLs) ends with server values; Telegram server-config
  prompt reaches the provider via ChannelRuntimeConfig; CreateThread /
  UpdateThread typed fields keep working from Mac/iOS; plugin metadata
  pass-through removal verified (breaking, owner-accepted); forged
  provenance/internal keys inert (wake still fires; no internal marking;
  no presentation); typed-form fingerprint collision test.
- **Legacy-field retirement**: no stored `internal`/`internal_kind`; no
  consumer left (grep guard across gateway, desktop, iOS, CLI); history
  envelope golden shows both fields absent.
- **Models**: golden JSON both kinds; captured-fixture reducer tests
  (new, post-migration, measured dropped-marker shape); malformed/
  unmarked negatives; title/status-only rows-hash delta upsert; delta
  roundtrip; same-seq reseed.
- **Clients**: mapper tests from captured v2 frames; no-text-kind guard +
  negative (identical unmarked text renders plain); real Storybook row;
  width/alignment real-layout on both platforms (long user text vs long
  card: same trailing edge and max width, across resize and both iOS
  Dynamic Type branches; no card-local constants; existing full-width
  desktop assertion inverted); clamp matrix (exact-fit with injected ε,
  one long wrapping line, 11 explicit lines, list/code/table/image,
  overflow→fit on widening, Dynamic Type change, late image/font
  settling); no affordance on short bodies; gesture coexistence; dialog
  focus trap/Escape/focus return; snapshot-carrying selection survives
  row eviction; occurrence/gateway switch dismissal.
- **Prompt**: capability block for provider × default/custom × prompt
  present/blank; approve + feedback rules asserted per provider envelope.
- **Migration**: all historical shapes; forged-history negatives (real
  task id, matching thread, legal envelope, plugin-chosen run id,
  canonical top-level provenance via every import path — none mint
  provenance); secret scrub asserted on jsonl + history API of the
  real-copy rehearsal; pre-cutover provenance strip; first-fork
  correctness; three-point fault injection; per-generation marker dedup
  and regeneration; old-cursor refetch via the marker.
- **Cutover**: v2×old-gateway and old×v2-gateway; history envelope
  golden; cache-schema upgrade (bodies + snapshots dropped together);
  offline cold-open.
- **E2E evidence attribution**: desktop worktree ↔ installed `app.asar`
  hash equality before/after + renderer `--app-path` check; iOS worktree
  ↔ installed carrier (debug.dylib / executable) hash equality
  before/after; busy-thread `--notify current-thread` E2E → committed
  record → collapsed card → dialog; iOS xcodebuild + simulator collapsed
  → full-screen → complete body → occurrence-switch dismissal; SwiftPM
  mapper tests from the captured frame.

## 6. Rollout

One change set; desktop + gateway ship together; migration on first boot
before serving; CLI ships from the same repo; iOS via its normal release
flow with the v2 minimum build; TestFlight upload only with explicit user
approval in that turn. Owner-communicated product costs: historical
task/restart notifications and historical loop-continuation rows render
as plain text; plugin arbitrary-metadata pass-through is removed.
