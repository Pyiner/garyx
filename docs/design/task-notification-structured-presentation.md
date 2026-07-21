# Task Notification Structured Presentation

Status: revision 4 for adversarial review
(r1 FAIL 3B+8M; r2 FAIL 1B+7M+3m; r3 FAIL 2B+5M+2m — confirmed and frozen:
root cause, flattened presentation payload, shared user-role width owner,
clamp/measurement model, always-attached iOS owner + snapshot selection,
range-rewrite marker skeleton, capability-block precondition, stream-only
schema negotiation, rollout/TestFlight contract)
Owner: Gary (orchestrator thread)
Scope: garyx-models, garyx-router, garyx-bridge, garyx-gateway, **garyx**
(channel plugin host), desktop renderer, iOS

## 1. Problem

Mac and iOS render `<garyx_task_notification>` messages as raw XML text in a
large fraction of cases (live measurement 2026-07-21: 73 of 252 committed
notification records lack the metadata marker — the busy-thread queue path
drops it). Two client prose parsers per platform reverse-engineer an English
template word-for-word; restart notices have two more. And dispatch metadata
is one bare `HashMap` from plan to persistence, so provider runtime secrets
(`provider_env`, `system_prompt`, MCP headers/token/servers, antigravity
env) **already leak into committed transcripts** on the direct path, guarded
only by a two-key denylist, while the queue path loses semantics through a
five-key allowlist.

Root theme: message semantics, provider runtime configuration, and external
caller input all share one untyped namespace; every boundary downstream is a
string filter that drifts.

## 2. Design principles

1. **Provenance is a type, not a set of keys.** Everything "the gateway did
   this" — internal dispatch flavor, task notification, restart wake — is
   one sealed type constructible only by internal dispatchers. External
   ingresses have no way to express it; nothing needs to be stripped.
2. **Runtime configuration is a type, not a set of keys.** Provider runtime
   input never travels in persistable metadata. External runtime overrides
   enter through explicit typed wire fields, not through generic metadata.
3. **Structure is never round-tripped through prose.** Producers persist
   structured objects; the reducer projects them; clients dumb-render. Task
   notifications and restart notices both; no prose parser survives.
4. **The prose/XML body remains the agent-facing surface**, generated from
   the same structured source at the same commit point.

## 3. Changes

### 3.1 Typed dispatch metadata

#### The shape

```rust
/// Travels the whole dispatch chain:
/// DispatchPlan → AgentRunRequest → ProviderRunOptions / persistence.
pub struct DispatchMetadata {
    /// Sealed: constructible only inside gateway/router dispatchers.
    /// Persisted with the message; the reducer projects presentation
    /// from it; task hooks and persistence derive "internal" from it.
    pub provenance: Option<InternalDispatchProvenance>,
    /// Typed persistable attribution + conversation context (see table).
    pub durable: DurableMetadata,
    /// Opaque provider runtime configuration. No Serialize/Deserialize;
    /// private field; per-producer constructors. Compile-fail (trybuild)
    /// probes pin: cannot reach persistence APIs, cannot be converted
    /// into DurableMetadata.
    runtime: RuntimeMetadata,
}

/// One variant per internal source — the complete closed inventory.
/// Serialization of this enum (serde-tagged) IS the durable wire/record
/// shape; no hand-maintained key tables to drift.
pub enum InternalDispatchProvenance {
    TaskNotification(TaskNotificationProvenance), // event/status/task_id/
                                                  // title/task_thread_id?/
                                                  // source_run_id?
    RestartWake(RestartWakeProvenance),           // id/kind/target/all/attempt
    Followup(FollowupProvenance),                 // job_id/scheduled_at/
                                                  // scheduled_for/reason?/
                                                  // originating_run_id?
                                                  // (quota resend rides this)
    Automation { automation_id: String, cron_action: String },
    Cron { cron_job_id: String, cron_action: String },
    TaskAutoStart { task_id: String, dispatch_reason: String },
    LoopContinuation { kind: Option<String>, origin: Option<String> },
}
```

- `persistence.rs::is_internal_dispatch` / the `internal` record field and
  `task_hooks.rs`'s internal-dispatch gate derive from `provenance`, never
  from metadata keys. An external message **cannot** flip lifecycle
  behavior or mark itself internal — the field does not exist on the
  untrusted path (`MAJOR 2`, structural fix instead of a wider strip list).
- The legacy scattered keys (`task_notification`+`task_notification_event`+
  siblings; `restart_wake`+`restart_wake_id/_kind/_target/_all/_attempt`;
  `schedule_followup`+`schedule_followup_job_id/_scheduled_at/
  _scheduled_for/_reason/_originating_run_id`; `task_auto_start`/
  `task_dispatch_reason`/`task_id`; `internal_dispatch`/`internal_kind`/
  `loop_origin`) exist after this change **only** as the migration's input
  shape (§3.5) — the enum's serde form replaces them everywhere live.
- `DurableMetadata` is typed, not a bare map: attribution/audit scalars
  (`agent_id`, `agent_display_name`, `model`, `model_reasoning_effort`,
  `model_service_tier`, `requested_provider_type`, run identifiers,
  `client_intent_id`, `origin_id`, `sdk_session_id`), conversation context
  (`channel`, `account_id`, `from_id`, `chat_id`, `client`, `is_group`,
  `thread_binding_key`, `delivery_thread_id`, `display_label`,
  `delivery_target_type`, `delivery_target_id`, `resolved_thread_id`,
  `workspace_dir`, `attachments` refs), session forking
  (`fork_from_thread_id`, `fork_from_sdk_session_id`,
  `fork_from_provider_type`), a rebuilt-from-authoritative-fields
  `runtime_context` object (§3.2), and an **extension namespace** for
  generic external metadata (the only place untrusted keys can live).
  Field ownership is expressed by these constructors in code — the
  authoritative inventory is the type definitions, and §5 retention tests
  assert against the constructors' serde output, not a hand-written table
  (r3 MAJOR 1: the r3 table had wrong followup key names and missing
  fork/delivery fields precisely because it was hand-written).
- `RuntimeMetadata` producers: MCP wiring (`garyx_mcp_auth_token`,
  `remote_mcp_servers`, `garyx_mcp_headers`), agent snapshot
  (`provider_env`, `system_prompt`), codex `developer_instructions`,
  `desktop_antigravity_env`, `sdk_session_fork` control. Providers read a
  combined read-only view (`provider_view()`).
- `PendingUserInput` keeps its identity fields and carries
  `provenance` + `durable_metadata`; the queued requested run id is kept as
  an explicit `origin_run_id` field on the queue record (attribution
  preserved without a metadata key). All four enqueue entrances converge on
  the single constructor (confirmed r2/r3); `RUNTIME_ONLY_METADATA_KEYS`
  and `DISPATCH_ATTRIBUTION_METADATA_KEYS` are deleted.

### 3.2 External ingress: typed overrides, no reserved-key stripping

External callers today legitimately use metadata for provider runtime
overrides. That capability becomes explicit typed wire fields —
`ExternalProviderRuntimeOverrides { system_prompt?, provider_env?,
remote_mcp_servers?, garyx_mcp_headers?, … }` — accepted by chat,
`AtomicDispatchBody`, `CreateThreadBody`, and plugin `deliver_inbound`
(`garyx/src/channel_plugin_host.rs` — the `garyx` crate is in scope). The
ingress constructs `RuntimeMetadata` from them directly. Remaining generic
metadata goes into the durable **extension namespace** verbatim; it cannot
express provenance (no field), cannot reach runtime (wrong type), and needs
no string filtering (r3 BLOCKER 1: classification is by wire position, not
by key name).

- `runtime_context` is rebuilt from authoritative typed fields
  (`garyx-router/src/runtime_context.rs` stops cloning the caller's object;
  no "remove secrets when found" repair).
- Admission fingerprints are computed over the typed request form, so
  forged keys cannot perturb idempotency.
- Thread metadata copy-through (`create-only` → later dispatch) carries only
  the typed/extension form — provenance can never enter via a thread
  record.
- §5: per-ingress tests that runtime-shaped JSON in generic metadata (top
  level or nested, e.g. inside a caller-supplied `runtime_context`) never
  reaches providers or persistence outside the extension namespace, and
  that provenance-shaped JSON stays inert extension data with no lifecycle
  or presentation effect.

#### Envelope restructure + notification producer

`deliver_task_review_handoff` constructs
`InternalDispatchProvenance::TaskNotification { event, status, task_id,
title, task_thread_id, source_run_id }` (audit fields optional) and the
agent-facing text:

```
<garyx_task_notification event="ready_for_review" task_id="#TASK-42"
    status="in_review" title="...">
{final_message}
</garyx_task_notification>
```

`xml_attr` extended to encode CR/LF/TAB as numeric character references;
multiline-title test. Body tag-neutralization stays.

#### Task lifecycle capability block (precondition for tutorial deletion)

Unchanged from r3 (confirmed): brand-free, persona-free block injected for
provider × default/custom × prompt present/blank; approve →
`garyx task update <id> --status done`; needs changes →
`garyx thread send task '<id>' "<feedback>"` (auto-wakes, flips
`in_review → in_progress`; no manual status update). Provider-envelope
tests across the matrix; only then the tutorial block is deleted.

### 3.3 Models: flattened presentation payload (confirmed r3)

```json
"presentation": { "kind": "task_notification", "event": "ready_for_review",
                  "status": "in_review", "task_id": "#TASK-42", "title": "..." }
```

```json
"presentation": { "kind": "restart_notice" }
```

Projected by the reducer **from `provenance`** (TaskNotification and
RestartWake variants; user-role rows only). Missing/other provenance → no
presentation → ordinary text. Kind is never derived from message text.
Golden JSON locks both kinds; desktop contract goes discriminated-object;
iOS decoder + full-payload signature hash; title/status-only change →
rows-hash delta upsert test; Storybook uses a real user render row.

### 3.4 Clients (confirmed r3; unchanged)

Delete all four prose parsers; one structural envelope decoder per
platform, reachable only behind a server presentation; negative test that
unmarked identical text stays a plain message. Shared user-role layout
owner (iOS extracts the `0.77/0.94` + spacer + trailing-menu contract, no
copied constants; desktop deletes the task-card width/alignment overrides
and inherits `.message-bubble.user`). Collapsed card clamps to 10
line-boxes with injected-ε overflow decision measured after shared width,
re-measured on content/width/font/Dynamic Type and intrinsic settling
(image load, font resolution, table/code re-layout); the pure
`fits/overflows` decision lives in `GaryxMobileCore` (SwiftUI keeps only
the measurement adapter). Expand: desktop controlled Radix Dialog (focus
trap/Escape/focus return); iOS always-attached
`.garyxFullScreenCover(item:)` on the stable occurrence owner, selection
item carries an immutable header/body snapshot (seq/id as identity only),
cleared on thread/gateway/occurrence change. Width rules bind the
collapsed transcript card only.

### 3.5 History: one boot migration = full dispatch-metadata separation

One boot-time range-rewrite migration, reason
`structured_presentation_metadata_v1`, marker identity
`(migration_version, import_generation, thread_id)` (r3-confirmed
skeleton: boot orchestrator owning DB + transcript store, store lock,
original seq/timestamps untouched, per-generation dedup, crash-retry still
repairs SQL projections, global marker last, three-point fault injection,
rehearsal on a copied real data dir). Its content upgrades from
"presentation metadata backfill" to the **complete separation cleanup**
(r3 BLOCKER 2):

1. **Secret scrub**: remove all historical runtime-owned top-level fields
   from every committed message metadata (`provider_env`, `system_prompt`,
   `garyx_mcp_auth_token`, `remote_mcp_servers`, `garyx_mcp_headers`,
   `desktop_antigravity_env`, `developer_instructions`); rebuild legacy
   `runtime_context` values into the authoritative safe schema; normalize
   persisted `pending_user_inputs` the same way. Rehearsal asserts
   sentinel secrets appear in neither transcript jsonl nor
   `/api/threads/history` responses afterward.
2. **Provenance-gated presentation backfill** (r3 MAJOR 3 — historical
   metadata was forgeable): a legacy record is converted to a trusted
   `TaskNotification` provenance/object only if a **legacy producer
   predicate** holds: the notification's `task_id` correlates with the
   authoritative task ledger/projection history (task events exist for
   that id; for the 73 dropped-marker records additionally `origin_run_id`
   prefix `task-notify-` + envelope shape). Restart records likewise must
   correlate with restart-wake ledger evidence. Records that cannot prove
   server origin keep plain text — no presentation, ever. Negative
   fixtures with forged legacy keys/prefixes/bodies prove they stay plain.
3. **Field recovery with deterministic fallback** (r3 MAJOR 4 — title is
   not universally recoverable: the legacy envelope has no title
   attribute, titles may contain blank lines, the task may be deleted):
   `event/status/task_id` from envelope attributes; `title` from the task
   ledger when available, else synthesized from the prose headline/first
   paragraph, else `task_id` — with a recorded recovery reason; never
   abort boot over an unrecoverable title. `task_thread_id` best-effort
   via task projection; `source_run_id` absent when unknowable —
   `origin_run_id` (the notify-dispatch id) is never substituted.
   Fixtures: multiline title with embedded blank segment + deleted task;
   the exact measured dropped-marker shape.

### 3.6 Wire cutover: stream negotiation + client cache invalidation

- `render_schema = 2` negotiated on the per-thread render-state stream
  only; full and delta frames echo it; v2 gateway answers missing/old
  schema with upgrade-required before SSE; v2 clients fail fast on
  schema-less frames. Raw `/api/threads/history` wire unchanged
  (CLI/desktop/iOS history consumers unaffected).
- **Persistent render caches** (r3 MAJOR 5): desktop transcript cache and
  iOS `GaryxTranscriptCache` bump their cache schema version and hard-drop
  v1-era cached render snapshots on first v2 launch (no dual reading of
  cached string presentations; offline cold-open shows the loading path
  until refetch). Tests: upgrade path, offline cold-open after upgrade,
  and the range_rewrite-marker refetch interaction.
- Desktop ships atomically with the gateway; iOS sets a v2 minimum
  supported build.

## 4. Non-goals

Bot-channel human rendering; task lifecycle semantics; notification
targets; delivery routing. Restart notices are in scope.

## 5. Validation

- **Types**: trybuild compile-fail — RuntimeMetadata into persistence,
  RuntimeMetadata→DurableMetadata conversion, external construction of
  `InternalDispatchProvenance`.
- **Boundary**: sentinel-secret absence (every runtime producer key)
  across direct, Legacy queue, Durable exact queue, public
  `add_streaming_input`; provenance retention for all seven variants
  through the real busy-thread queue path, asserted against the enum's
  serde output.
- **Ingress**: per-ingress (chat, atomic dispatch, create-thread +
  copy-through, plugin inbound) — runtime-shaped generic metadata never
  reaches providers/persistence outside the extension namespace;
  provenance-shaped metadata is inert (no lifecycle flip: task
  `in_review→in_progress` wake still fires for a forged
  `internal_dispatch`; no `internal` record marking; no presentation);
  fingerprints computed on the typed form (collision test).
- **Models/clients**: golden JSON both kinds; captured-fixture reducer
  tests (new, post-migration, measured dropped-marker); malformed/unmarked
  negatives; rows-hash upsert; delta roundtrip; same-seq reseed; mapper
  tests from captured v2 frames; no-text-kind-derivation guard; real
  Storybook row; width/alignment real-layout on both platforms (long user
  text vs long card share trailing edge/max width; no card-local
  constants; the existing full-width assertion inverted); clamp/expand
  matrix incl. ε-injected exact-fit, intrinsic settling, gesture
  coexistence, dialog focus/Escape/return, snapshot-carrying selection
  surviving row eviction, occurrence/gateway-switch dismissal.
- **Prompt**: capability block present for provider × default/custom ×
  prompt present/blank.
- **Migration**: all historical shapes incl. forged-legacy negatives,
  multiline-title/deleted-task recovery, secret-scrub assertions on jsonl
  + history API of a real-copy rehearsal; crash/restart idempotence at the
  three injection points; per-generation marker dedup/regeneration;
  old-cursor refetch via the marker.
- **Cutover**: v2×old-gateway, old×v2-gateway, history-wire stability,
  cache-schema upgrade + offline cold-open.
- **End-to-end evidence attribution** (per garyx-review contract): busy
  thread `--notify current-thread` → committed record → desktop
  packaged-renderer check with `app.asar` hash verified against the
  worktree build before and after; iOS `xcodebuild` + simulator flow
  (collapsed → full-screen → complete body; occurrence switch closes)
  with the installed code carrier hashed (Debug: `<Product>.debug.dylib`,
  else executable) before and after; SwiftPM mapper tests from the
  captured frame.

## 6. Rollout

Implementation and schema cutover land as one change set; desktop and
gateway ship together; the migration runs on first boot. iOS ships through
its normal release flow with the v2 minimum build; any TestFlight upload
is a separate release action taken only with explicit user approval in
that turn, per the repository release contract. Legacy metadata shapes and
the legacy string presentation wire each have exactly one consumer: the
migration and the upgrade-required error path.
