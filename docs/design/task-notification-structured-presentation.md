# Task Notification Structured Presentation

Status: revision 3 for adversarial review
(round 1: FAIL 3B+8M; round 2: FAIL 1B+7M+3m — direction confirmed:
flattened payload, shared user-role width owner, always-attached iOS owner,
range-rewrite migration)
Owner: Gary (orchestrator thread)
Scope: garyx-models, garyx-router, garyx-bridge, garyx-gateway, desktop
renderer, iOS

## 1. Problem

Mac and iOS render `<garyx_task_notification>` messages as raw XML text in a
large fraction of cases. Measured on live data (2026-07-21, last 80
transcripts): 252 committed task-notification user records, **73 (29%) missing
the `task_notification` metadata marker**, concentrated in busy orchestrator
threads.

Root theme: **structure is thrown away and then badly reconstructed
downstream**, and metadata durability/secrecy is decided by accident of code
path.

### Defect A — queue path drops semantic metadata (the live bug)

Idle-thread dispatches commit full dispatch metadata; busy-thread dispatches
(`QueuedToActiveRun`) pass `dispatch_attribution_metadata`
(`run_management.rs:107`), whose five-key allowlist drops every
`task_notification*` key. Unmarked records get no `presentation` and render
as raw XML on both clients.

### Defect B — prose round-trip (the architectural defect)

`format_task_ready_notification` flattens structured fields into an English
prose template + CLI tutorial, wraps them in XML; desktop
(`task-notification.ts`) and iOS (`GaryxTaskNotificationPresentation.swift`)
regex-parse the prose back, coupled word-for-word to the template. Restart
notices (`<garyx_restarted>`) have the same disease: two more client prose
sniffers, no server presentation at all.

### Defect C — runtime secrets have no structural boundary

Dispatch metadata is one bare `HashMap` from plan to persistence. The
run-metadata backfill injects `provider_env` (may contain tokens),
`garyx_mcp_headers`, `desktop_antigravity_env`, `system_prompt`,
`developer_instructions` into the same map that persistence later reads with
only a two-key denylist — some of this **already leaks into committed
transcripts on the direct path**. Mixing happens at the sources
(`agent_reference.rs` snapshot, router `execution.rs` bulk thread-metadata
copy, `internal_inbound.rs` MCP wiring), so no persistence-side filter can be
sound.

## 2. Design principles

1. **Typed metadata separation, end to end.** Two containers with different
   lifecycles travel the whole dispatch chain; the split happens **at the
   sources**, not at persistence (§3.1).
2. **Structure is never round-tripped through prose.** Producers persist
   structured objects; the reducer projects them; clients dumb-render. This
   applies to task notifications **and** restart notices — no second
   mechanism survives.
3. **The prose/XML body remains the agent-facing surface.** Agents read the
   text; human UI reads the projection; both are generated from the same
   source at the same commit point.
4. **One trust boundary for externally supplied metadata.** Every external
   ingress passes `UntrustedMetadata` sanitization (§3.2); server-owned keys
   are constructible only by internal dispatchers.

## 3. Changes

### 3.1 Typed durable/runtime metadata across the dispatch chain

#### Types

```rust
/// Persistable message metadata. Freely constructible; the only metadata
/// type persistence APIs accept.
pub struct DurableMetadata(serde_json::Map<String, Value>);

/// Provider runtime configuration. Private field, opaque constructors
/// (one per legitimate producer need). Deliberately implements neither
/// Serialize nor Deserialize: a compile-fail (trybuild) probe pins that
/// it cannot be serialized or passed to run-record/transcript APIs, and
/// that no caller can forge a DurableMetadata from it without going
/// through the explicit audit projection.
pub struct RuntimeMetadata { inner: serde_json::Map<String, Value> }

/// What travels the dispatch chain.
pub struct DispatchMetadata {
    pub durable: DurableMetadata,
    runtime: RuntimeMetadata,
}

impl DispatchMetadata {
    /// Read-only combined view for provider assembly (prompt, env, MCP
    /// wiring). Providers cannot obtain an owned persistable map from it.
    pub fn provider_view(&self) -> ProviderMetadataView<'_> { … }
}
```

- `DispatchPlan`, `AgentRunRequest.metadata`, `ProviderRunOptions.metadata`
  change from `HashMap<String, Value>` to `DispatchMetadata`
  (`garyx-models/src/provider.rs:88`, `:989`; router
  `run/execution.rs`). Every producer that today inserts into the bare map
  chooses a container at the type level; there is no default and no
  string-keyed filter anywhere.
- `PersistedRun` / run-record / transcript persistence APIs accept
  `&DurableMetadata` only. `RUNTIME_ONLY_METADATA_KEYS` and
  `DISPATCH_ATTRIBUTION_METADATA_KEYS` are both deleted (superseded).
- `PendingUserInput` keeps its own identity fields (`id`, `bridge_run_id`,
  `queued_at`, `origin_id`, `status`) and gains `durable_metadata:
  DurableMetadata`; queue bookkeeping is not re-expressible by callers
  through a metadata map. `acknowledge_pending_input` merge semantics
  (bookkeeping wins) unchanged.
- All four enqueue entrances (Legacy, Durable exact,
  `execute_durable_stream_input`, public `add_streaming_input`) already
  converge on the single `PendingUserInput` constructor
  (`add_streaming_input_with_metadata_mode`) — confirmed in review; the
  typed split makes that point structurally safe rather than filtered.

#### Field ownership table

Runtime (never persisted; seeds the source-migration, enforced by type not
by list):

| Key | Producer |
|---|---|
| `garyx_mcp_auth_token` | internal_inbound / chat pipeline |
| `remote_mcp_servers` | managed MCP injection |
| `garyx_mcp_headers` | provider_common MCP HTTP headers |
| `provider_env` | agent snapshot (`agent_provider_env_metadata`) |
| `desktop_antigravity_env` | antigravity provider |
| `system_prompt` | agent snapshot / thread snapshot merge |
| `developer_instructions` | codex provider input |
| `sdk_session_fork` | run-derivation control |

Durable — semantic identity (server-owned, reserved):

| Key | Producer |
|---|---|
| `task_notification` (object, §3.2) | task notification dispatcher |
| `restart_wake` (object, §3.3) | restart wake dispatcher |

Durable — source-specific semantic keys (exact inventory per source):

| Source | Keys |
|---|---|
| Task notification | `task_notification` object |
| Followup (`schedule_followup`) | `schedule_followup`, `schedule_followup_job_id`, `scheduled_at`, `scheduled_for`, `reason?`, `originating_run_id?` |
| Quota resend | same shape as followup (no separate quota keys) |
| Automation | `source="automation"`, `automation_id`, `cron_action` |
| Cron | `source="cron"`, `cron_job_id`, `cron_action` |
| Task auto-start | `task_auto_start`, `task_dispatch_reason`, `task_id` |
| Restart wake | `restart_wake` object |
| All internal dispatch | `internal_dispatch`, `internal_kind?`, `loop_origin?` |

Durable — attribution/audit safe scalars (explicitly projected, never
bulk-copied): `agent_id`, `agent_display_name`, `model`,
`model_reasoning_effort`, `model_service_tier`, `requested_provider_type`,
`client_run_id` / `run_id` / `bridge_run_id`, `client_intent_id`,
`origin_id`, `sdk_session_id`.

Durable — conversation context (existing committed behavior, no secret
material): `channel`, `account_id`, `from_id`, `chat_id`, `client`,
`is_group`, `thread_binding_key`, `delivery_thread_id`,
`resolved_thread_id`, `workspace_dir`, `runtime_context` (thread/channel
context object; if the implementation inventory finds any embedded secret
field it is removed field-wise, not by reclassifying the object),
`attachments` (references).

New keys introduced later must pick a container at the type level; the table
is the initial authoritative inventory, kept in the code as doc comments on
the two constructors.

### 3.2 Gateway: reserved keys behind one trust boundary; envelope; prompt

#### Structured object

`deliver_task_review_handoff` emits one cohesive durable object — no
top-level aliases; the scattered bool/string keys are deleted;
`task_hooks.rs` reads the new object; the legacy shape is read only by the
migration:

```json
"task_notification": {
  "event": "ready_for_review",
  "status": "in_review",
  "task_id": "#TASK-42",
  "title": "...",
  "task_thread_id": "thread::...",
  "source_run_id": "..."
}
```

`task_thread_id` and `source_run_id` are audit fields, optional in the
schema (§3.5 explains why they cannot always exist).

#### `UntrustedMetadata` boundary (all external ingresses)

A single sanitization type wraps every externally supplied metadata map and
strips reserved keys (`task_notification`, `restart_wake`) alongside the
existing agent-identity strip. Covered ingresses:

- chat API (`prepare.rs`),
- `AtomicDispatchBody.metadata` (`create_dispatch.rs`),
- `CreateThreadBody.metadata` (`routes/threads.rs`) — thread metadata is
  later bulk-copied into dispatch metadata by the router, so create-only
  injection must die at the write point,
- plugin inbound `extra_metadata` (`channel_plugin_host.rs`),
- admission fingerprints (`conversation_admission.rs`) — computed over the
  sanitized map so a forged key cannot even perturb idempotency.

Internal dispatchers construct trusted `DurableMetadata` directly and never
pass through sanitization. §5 adds per-ingress forgery tests and a
fingerprint-collision test.

#### Envelope restructure

Attributes carry all structured fields; body is the pure handoff:

```
<garyx_task_notification event="ready_for_review" task_id="#TASK-42"
    status="in_review" title="...">
{final_message}
</garyx_task_notification>
```

`xml_attr` is extended to encode CR/LF/TAB as numeric character references
(current helper only escapes `& " < >`); multiline-title test required.
Existing body tag-neutralization stays.

#### Task lifecycle capability block (precondition for tutorial deletion)

The review confirmed no existing prompt/skill depends on the notification
body tutorial, but `GARY_BASE_INSTRUCTIONS` does not reach custom agents:
Claude custom standalone uses only the agent prompt (pinned by test), and
Codex custom agents with a blank prompt get no developer instructions at
all. Therefore:

- Introduce a **brand-free, persona-free task lifecycle capability block**
  (no Garyx identity statements) injected for every combination of
  provider × default/custom agent × prompt present/blank.
- Content: approve → `garyx task update <id> --status done`; needs changes →
  send feedback with `garyx thread send task '<id>' "<feedback>"` — which
  itself wakes the task and flips `in_review → in_progress`; do **not**
  attempt a manual status update (the CLI rejects it).
- Provider-envelope tests assert the block for the full matrix. Only after
  that lands is the "View details / Review next" tutorial deleted from the
  notification body. One mechanism: prompt.

### 3.3 Models: flattened presentation payload; restart notice variant

Wire (flattened discriminated object; golden JSON tests lock both kinds):

```json
"presentation": { "kind": "task_notification", "event": "ready_for_review",
                  "status": "in_review", "task_id": "#TASK-42", "title": "..." }
```

```json
"presentation": { "kind": "restart_notice" }
```

- Rust: enum with struct variants, serde `tag = "kind"`.
- `render_message_presentation` reads the structured durable objects only.
  Missing/malformed → no presentation → ordinary text. **Presentation kind
  is never derived from message text** — not in the reducer, not in
  clients.
- Restart canonical shape decision (producer currently emits `restart_wake:
  true` plus five scattered siblings — that shape is retired): one nested
  object, all top-level aliases deleted; producer, reserved-key
  enforcement, reducer, and tests recognize only this:

  ```json
  "restart_wake": { "id": "...", "kind": "...", "target": "...",
                     "all": false, "attempt": 0 }
  ```

- Payload carries small fixed fields only; body text stays out of rows
  (projection-lightweight contract); clients derive card/expanded body from
  their cached message text via the structural envelope strip.
- Impact closure: desktop `transcript.ts` string enum → discriminated
  object (all comparison sites updated); iOS decoder single-string → object
  and message signature hashes the complete payload; test that a
  title/status-only change produces a rows-hash delta upsert; Storybook
  scenario becomes a real user render row with a real presentation object.

### 3.4 Clients: delete all four prose parsers; shared layout; collapse

- Delete desktop `parseTaskNotificationText` + `restart-notice.ts`; delete
  iOS `GaryxTaskNotificationPresentation` regex parsing +
  `GaryxRestartNoticePresentation` sniffing. One shared **structural
  envelope decoder** per platform (find opening tag's `>`, last close tag;
  wording-independent; tolerant of legacy bodies) is invoked **only when
  the server presentation is present** — never to decide what a message is.
  `statusLabel`-style formatting stays client-side.

#### Alignment and width: user-role layout owner

A task notification is a user-role row (role guaranteed by the committed
record/reducer — the reducer marks user rows only; clients never infer
placement from presentation kind):

- **iOS**: extract the existing user-role container — trailing alignment,
  Dynamic Type width policy (`0.77 ×` screen normal, `0.94` at
  `xxxLarge+`), min leading spacer 60/12, trailing menu edge — into a
  shared layout owner used by both plain user bubbles and the card. The
  full-width leading `taskNotificationRow` branch is deleted. Copying
  `0.77` or minting a notification-specific constant is forbidden.
- **Desktop**: `.message-bubble.user { max-width: 77%; align-self: flex-end }`
  is the owner; task-card overrides (align-self, 736px/full-width) are
  deleted; the card may fill the shared cap but declares no constants.
- Width rules bind the **collapsed transcript card only**; expanded desktop
  dialog uses standard large-dialog sizing; expanded iOS view is
  full-screen.

#### Collapsed card + expand

- **Collapsed**: header (task id, status, title) + body viewport clamped to
  the height of **10 line-boxes at the current body font** (not 10 source
  lines). Overflow decision `naturalRenderedHeight > clampHeight + ε`,
  with `ε` an injected parameter of the measurement adapter (no hard-coded
  physical pixel), measured **after** the shared user-role width applies.
  Re-measure on content, width, font scale, Dynamic Type, **and**
  intrinsic-size settling: image load, font resolution, markdown
  table/code re-layout.
- Clamp decision in a testable layer: measurement adapter supplies
  width/scale/natural height; a pure function decides `fits/overflows`.
  Overflow ⇒ expand affordance + accessibility action; fits ⇒ neither.
  Expand activation excludes interactive descendants (links, copy/select/
  share, long-press menus keep working in both states).
- **Expanded**: header + the same envelope-stripped body (single strip,
  shared string):
  - Desktop: controlled Radix `Dialog`, stable owner, focus trap, Escape,
    focus returns to the card.
  - iOS: full-screen presentation owned by the stable conversation/route
    occurrence owner via one always-attached `.garyxFullScreenCover(item:)`
    (capsule-preview pattern). The selection item carries an **immutable
    header/body snapshot** taken at activation; `seq`/message id serve as
    identity only (a row evicted by the render window while open must not
    blank the presentation; one task may notify repeatedly). Selection
    clears on thread/gateway/occurrence change; modifiers never
    conditionally attached.

### 3.5 History: boot-time migration via the range-rewrite protocol

One boot-time migration, reason `structured_presentation_metadata_v1`,
covering **both** legacy task-notification metadata **and** legacy restart
scattered metadata (restart history must not regress to raw XML once the
prose parsers are gone).

1. Runs in the boot orchestrator that owns both the DB and
   `ThreadTranscriptStore`, after legacy import, before listener bind.
2. Per transcript, under the store lock: rewrite message metadata only —
   legacy scattered task keys → `task_notification` object; legacy restart
   keys → `restart_wake` object; the 73 dropped-marker records (identified
   by `origin_run_id` prefix `task-notify-` + envelope shape) get the
   object reconstructed. `seq`/`thread_id`/`run_id`/`timestamp` untouched;
   message text is history and is not rewritten.
3. **Field availability rules** (per review): `event`, `status`, `task_id`,
   `title` are required for presentation — recoverable from envelope
   attributes and the frozen legacy prose headline (regex acceptable inside
   a one-shot migration). `task_thread_id` is best-effort via the task
   projection (absent when the task is deleted). `source_run_id` is
   unrecoverable for dropped-marker records and stays absent —
   `origin_run_id` is the notify-dispatch id, **never** substituted for it.
   A fixture matching the exact measured dropped-marker shape proves a
   card renders without the audit fields.
4. Each changed transcript gets, in the same atomic file replacement, an
   appended new-seq `range_rewrite` control record. **Marker identity is
   `(migration_version, import_generation, thread_id)`**: dedup applies
   only within the same generation; a later import generation rewrites
   again with a new marker. A crash-retry that finds the file already
   rewritten still repairs the SQL projections before recording
   completion.
5. Thread-record count/tail projections fixed in the same pass; migration
   must not bump user-facing activity ordering.
6. Durable global marker keyed to `(migration_version, import_generation)`
   written last. File-atomic per transcript; idempotent end-to-end; any
   failure aborts startup and retries next boot.
7. Fault-injection coverage: crash after N of M transcripts; crash after
   file rename before SQL repair; crash after all per-thread work before
   the global marker. Plus a rehearsal on an isolated copy of the real
   production data directory.

### 3.6 Wire cutover: schema negotiation on the render-state stream only

`presentation` string → object is a breaking change **of the render-state
wire**, which is emitted only by the per-thread SSE full/delta frames. The
raw committed-message history API (`/api/threads/history`) does not carry
`RenderSnapshot` and keeps its wire unchanged — CLI/desktop/iOS history
consumers are unaffected (this is not dual reading; it is a different
endpoint with a different payload).

- `render_schema = 2` is declared by clients on the stream request; full
  and delta frames echo it.
- The v2 gateway answers a missing/older schema with a structured
  upgrade-required error before establishing the SSE stream; it never
  emits the legacy string wire.
- v2 clients hitting a frame without the schema (old gateway) fail fast
  with an upgrade notice; no string fallback.
- Desktop ships atomically with the gateway. iOS sets a minimum supported
  build for v2.

## 4. Non-goals

- Bot-channel (Telegram/Discord) human-readable rendering.
- Task lifecycle semantics, notification targets, delivery routing.
- Restart notices are **in scope** (§3.3/§3.4/§3.5).

## 5. Validation

- **Metadata boundary**: trybuild compile-fail probes (RuntimeMetadata into
  persistence APIs; forging durable from runtime). Sentinel-secret tests
  for every runtime-table key across direct, Legacy queue, Durable exact
  queue, and public `add_streaming_input` paths — absent from run records
  and transcripts. Retention tests assert the **exact source-specific keys
  from the §3.1 table** for all seven sources through the busy-thread queue
  path (real queue, no direct-commit shortcut).
- **Forgery**: per-ingress tests (chat, atomic dispatch, create-thread —
  including the thread-metadata copy-through on the next message, plugin
  inbound) that reserved keys are stripped and no presentation appears;
  fingerprint computed post-sanitization (collision test); assistant-role
  record with notification metadata gets no presentation.
- **Models**: golden JSON for both flattened kinds; reducer tests from
  captured fixtures (new shape + post-migration shape + measured
  dropped-marker shape); absence tests for malformed objects and for
  unmarked text that merely looks like an envelope; presentation-only
  change → rows-hash delta upsert; delta roundtrip; same-seq reseed.
- **Clients**: headless mapper tests from captured v2 frames. Guard rule:
  no client code path decides presentation kind from text (the structural
  envelope decoder is reachable only behind a server presentation);
  negative test that unmarked identical text renders as a plain message.
  Storybook uses a real user render row.
- **Width/alignment**: real-layout tests (browser; both iOS Dynamic Type
  branches) that long user text and a long card share trailing edge and
  max width across resize; architecture guard that card CSS/SwiftUI
  declares no width/alignment constants; the existing full-width desktop
  assertion is inverted.
- **Clamp/expand**: fixtures at exactly-fits (ε injected), one long
  wrapping line, 11 explicit lines, list/code/table/image bodies;
  overflow→fit on widening; Dynamic Type change; late image/font settling
  triggers re-measure; no affordance on short bodies; gesture coexistence;
  dialog focus trap/Escape/focus return; iOS snapshot-carrying selection
  survives row eviction; occurrence/gateway switch closes the
  presentation.
- **Prompt**: provider-envelope tests that the capability block (both
  approve and feedback rules) is present for provider × default/custom ×
  prompt present/blank.
- **Migration**: fixture data dir with all historical shapes (legacy task
  keys, legacy restart keys, dropped-marker, already-new); crash/restart
  idempotence at the three §3.5 injection points; per-generation marker
  dedup and regeneration on a new import generation; old-cursor client
  refetch via the range_rewrite marker; rehearsal on a copied real data
  dir.
- **Cutover**: v2 client × old gateway → upgrade notice; old client × v2
  gateway → upgrade-required before SSE; raw history endpoint unchanged
  for old and new clients; no frame ever emitted in legacy shape by a v2
  gateway.
- **End to end**: busy-thread `--notify current-thread` task → committed
  record shape → desktop packaged-renderer check (collapsed card → dialog)
  → iOS `xcodebuild` + simulator flow (collapsed card → full-screen →
  complete body, close on occurrence switch), plus SwiftPM mapper tests
  from the captured frame.

## 6. Rollout

Implementation and schema cutover land as one change set; desktop and
gateway ship together; the migration runs on first boot of the new gateway.
iOS ships through its normal release flow with the v2 minimum build; any
TestFlight upload is a separate release action taken only with explicit
user approval in that turn, per the repository release contract, and the
hard-cutover window is coordinated at that point. The legacy metadata
shapes and the legacy string presentation wire each have exactly one
consumer: the migration and the upgrade-required error path.
