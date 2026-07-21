# Task Notification Structured Presentation

Status: revision 5 for adversarial review
(r1 FAIL 3B+8M; r2 FAIL 1B+7M+3m; r3 FAIL 2B+5M+2m; r4 FAIL 3B+3M+1m —
confirmed and frozen through r4: root cause & principles, flattened
presentation payload, all client/UI decisions incl. shared width owner,
clamp model, iOS occurrence owner + snapshot selection, prose-parser
deletion, range-rewrite marker skeleton, capability-block precondition,
stream-only schema negotiation + client cache invalidation, rollout &
TestFlight contract, single-PendingUserInput-constructor convergence)
Owner: Gary (orchestrator thread)
Scope: garyx-models, garyx-router, garyx-bridge, garyx-gateway, garyx
(channel plugin host), **garyx-channels** (Discord/Telegram/Feishu/Weixin
inbound), desktop renderer, iOS

## 1. Problem

(unchanged; confirmed r3/r4) 73 of 252 live committed task-notification
records lack the metadata marker (busy-thread queue allowlist drops it);
four client prose parsers reverse-engineer templates; provider runtime
secrets already leak into committed transcripts on the direct path;
message semantics, runtime configuration, and external caller input share
one untyped `HashMap` namespace guarded by drifting string filters.

## 2. Design principles

1. **Provenance is a type reachable only from trusted code paths.**
   Internal dispatch flavor (including task notification and restart wake)
   is one sealed value. External ingress DTOs do not have the field;
   construction requires a runtime authority capability; store
   deserialization happens only inside the trust boundary.
2. **Runtime configuration is a type, not a set of keys.** Provider runtime
   input never travels in persistable metadata. External runtime overrides
   are explicit, per-ingress, and strictly narrower-or-equal to today's
   actually-consumed surface — never a capability expansion.
3. **Structure is never round-tripped through prose.** Producers persist
   structured objects; the reducer projects them; clients dumb-render.
4. **No retroactive trust.** History that cannot be proven server-originated
   is never upgraded to trusted presentation.

## 3. Changes

### 3.1 Typed dispatch metadata

#### Provenance: sealed by module privacy + authority capability

```rust
// garyx-models, module `dispatch_provenance`
pub struct InternalDispatchProvenance { inner: ProvenanceInner /* private */ }
// read access: pub fn kind(&self) -> ProvenanceKind + typed accessors.
// serde: Serialize always; Deserialize is NOT derived publicly — the
// store-side decode lives behind `pub(crate)`/store-only API so external
// input can never deserialize into it.

enum ProvenanceInner {                 // private enum, private variants
    TaskNotification(TaskNotificationProvenance),
    RestartWake(RestartWakeProvenance),
    Followup(FollowupProvenance),      // quota resend rides this
    Automation { automation_id: String, cron_action: String },
    Cron { cron_job_id: String, cron_action: String },
    TaskAutoStart { task_id: String, dispatch_reason: String },
}

/// Unforgeable constructor capability. One value, created by the gateway
/// composition root at startup, handed only to the internal dispatchers.
pub struct DispatchAuthority { _private: () }
impl InternalDispatchProvenance {
    pub fn task_notification(_: &DispatchAuthority, f: TaskNotificationFields) -> Self { … }
    // one constructor per variant, all requiring &DispatchAuthority
}
```

- Six live variants. `LoopContinuation` has no production producer (r4):
  it is **not** a sealed variant; its legacy keys (`internal_kind`,
  `loop_origin`) are migration/normalization input only, and existing
  committed rows keep rendering as they do today.
- `persistence.rs` internal marking and `task_hooks.rs` lifecycle gating
  derive from provenance presence/kind, never from metadata keys.
- Honest boundary statement: `Deserialize` must exist for the store read
  path, so the guarantee is (a) no external ingress DTO has a provenance
  field, (b) the decode API is store-scoped, (c) literal construction
  requires `&DispatchAuthority`. trybuild probes: literal construction from
  an external crate **and** from a sibling module inside garyx-bridge
  orchestration both fail; `DispatchAuthority` cannot be constructed
  outside the composition root.

#### Canonical committed-record layout (r4 MAJOR 2)

Provenance is a **top-level message field**, not a metadata key:

```jsonc
// direct-path committed user record (golden JSON #1)
{
  "role": "user",
  "content": "...",
  "timestamp": "...",
  "provenance": { "kind": "task_notification", "event": "ready_for_review",
                   "status": "in_review", "task_id": "#TASK-42",
                   "title": "...", "task_thread_id": "thread::…",
                   "source_run_id": "…" },        // internally tagged: "kind"
  "metadata": {
    /* durable attribution scalars + conversation context, flat */
    "agent_id": "…", "model": "…", "channel": "…", …,
    "external": { /* extension namespace: verbatim untrusted keys */ }
  }
}
```

- Golden JSON #2 (queued): same layout plus `queued_input_id`,
  `queued_at`, `origin_id?`, `origin_run_id` bookkeeping keys.
- Golden JSON #3 (migrated/normalized legacy record): scrubbed metadata,
  **no** provenance (see §3.5), legacy scattered keys removed.
- The reducer reads `message.provenance`; raw `/api/threads/history`
  returns the record as stored (provenance visible; wire documented).
- `ProviderRunOptions` currently derives Serialize/Deserialize; that derive
  is **removed**. Run-record persistence serializes an explicit
  `PersistedRunOptions` projection (durable + provenance only). Compile
  fails if anyone reintroduces a bulk serialize of the runtime container.
- `PendingUserInput` carries `provenance` + `durable_metadata` + its own
  identity fields incl. explicit `origin_run_id` (confirmed r3/r4; single
  constructor; both legacy filter constants deleted).

#### DurableMetadata / RuntimeMetadata

Unchanged from r4 (typed constructors are the authoritative field
inventory; runtime producers: MCP token/servers/headers, `provider_env`,
`system_prompt`, `developer_instructions`, `desktop_antigravity_env`,
`sdk_session_fork` — with §3.5 now scrubbing and §3.2 re-homing the
persisted-fork dependency).

### 3.2 External ingress: per-ingress explicit surface, no expansion

Closed allow/deny table (r4 BLOCKER 3 — no ellipses; **no key becomes
externally settable that is not consumed from external input today**; the
implementation's first step is an evidence pass confirming today's
actually-consumed set, and the table below may only shrink from it):

| Ingress | Runtime overrides accepted (typed fields) | Everything else |
|---|---|---|
| chat API (`prepare.rs`) | `remote_mcp_servers`, `garyx_mcp_headers` (today's working remote-MCP surface) | durable extension namespace |
| `AtomicDispatchBody` | same as chat | extension namespace |
| `CreateThreadBody` (create-only, no run) | **none** — run-scoped overrides rejected; long-lived thread runtime state lives in typed `ThreadRuntimeConfig` written only by gateway-internal paths (agent binding); the bulk `merge_thread_agent_runtime_snapshot` from bare thread metadata is retired with it | extension namespace |
| plugin `deliver_inbound` (`garyx` crate) | **none** | extension namespace |
| built-in channels (`garyx-channels`: Discord/Telegram/Feishu/Weixin `InboundRequest.extra_metadata`) | **none** | migrated to the provenance-free external constructor; in the §5 matrix |

- `provider_env`, `system_prompt`, `developer_instructions`,
  `desktop_antigravity_env` remain server-owned everywhere (r4: opening
  `provider_env` would have been an unapproved security expansion — it is
  stripped at every caller boundary today and stays that way).
- `runtime_context` rebuilt from authoritative typed fields (no cloning of
  caller objects). Admission fingerprints computed over the typed request
  form. Thread-metadata copy-through carries only typed/extension forms.
- External DTOs have no provenance field; forged provenance-shaped JSON in
  generic metadata lands inert in `metadata.external` with no lifecycle,
  internal-marking, or presentation effect (§5 per-ingress tests, incl.
  the built-in channels).

Envelope restructure, `xml_attr` CR/LF/TAB encoding, and the task
lifecycle capability block precondition are unchanged (confirmed r3/r4).

### 3.3 Models: flattened presentation payload (confirmed; unchanged)

`presentation: { "kind": "task_notification", event, status, task_id,
title }` / `{ "kind": "restart_notice" }`, projected by the reducer from
`message.provenance` for user-role rows; golden JSON both kinds; desktop
discriminated union; iOS object decoder + full-payload signature;
title/status-only rows-hash upsert test; real Storybook row.

### 3.4 Clients (confirmed; unchanged)

Four prose parsers deleted; structural envelope decoder only behind a
server presentation; shared user-role width owner (iOS 0.77/0.94 contract
extraction, desktop inherits `.message-bubble.user`); 10-line-box clamp
with injected ε, post-width measurement, intrinsic-settling re-measure,
pure decision in GaryxMobileCore; expand = desktop controlled Dialog / iOS
always-attached cover on the occurrence owner with immutable snapshot
selection; collapsed-only width rules.

### 3.5 History: scrub and normalize — never re-trust (r4 BLOCKER 5)

The boot migration (range-rewrite skeleton confirmed: marker identity
`(migration_version, import_generation, thread_id)`, store lock, original
seqs untouched, crash-retry SQL repair, global marker last, three-point
fault injection, real-copy rehearsal) does:

1. **Secret scrub**: remove historical runtime-owned fields —
   `provider_env`, `system_prompt`, `garyx_mcp_auth_token`,
   `remote_mcp_servers`, `garyx_mcp_headers`, `desktop_antigravity_env`,
   `developer_instructions`, **`sdk_session_fork`** (r4 MAJOR 6) — from
   every committed message and persisted `pending_user_inputs`; rebuild
   legacy `runtime_context` to the safe schema. Rehearsal asserts sentinel
   secrets absent from transcript jsonl **and** `/api/threads/history`.
   The session resolver's dependency on the persisted fork flag is
   re-homed: first-fork behavior derives from typed `fork_from_*` fields
   plus current session binding state (validation: transcripts contain no
   `sdk_session_fork`, and a fresh fork still resolves correctly).
2. **Normalization without trust upgrade**: legacy scattered
   task/restart/internal keys are removed from metadata (golden JSON #3).
   **No legacy record is upgraded to provenance.** There is no immutable
   dispatch ledger that can prove server origin: task "history" is just
   the current task object's events and dies with task deletion; restart
   pending files are deleted after successful wake; plugins can supply
   arbitrary `extra_metadata` and self-chosen `run_id`s, so any
   correlation predicate (task id exists + envelope shape + `task-notify-`
   prefix) is attacker-satisfiable. Historical task/restart messages
   therefore keep plain-text rendering permanently; the four prose parsers
   are still deleted (accepted product cost, stated to the owner); all
   new notifications get cards from provenance.
3. **Forged-history fixtures**: negative fixtures use a *real* task id,
   matching notification thread, legal envelope, and a plugin-controlled
   `task-notify-*` run id — and assert the migration still does not mint
   provenance. (A future dispatch ledger cannot retroactively certify old
   records either; out of scope.)

### 3.6 Wire cutover (confirmed; unchanged)

`render_schema = 2` negotiated on the render-state stream only; raw
history wire unchanged; upgrade-required / fail-fast both directions;
desktop+gateway atomic; iOS v2 minimum build; both persistent client
caches bump schema and hard-drop v1 render snapshots (upgrade, offline
cold-open, marker-refetch tests).

## 4. Non-goals

Bot-channel human rendering; task lifecycle semantics; notification
targets; delivery routing. Restart notices in scope. Retroactive
certification of historical records is explicitly rejected, not deferred.

## 5. Validation

- **Types**: trybuild — external-crate and sibling-module literal
  construction of provenance fail; `DispatchAuthority` unconstructible
  outside the composition root; runtime container into persistence APIs
  fails; reintroduced bulk serialize of run options fails.
- **Boundary**: sentinel-secret absence for every runtime key (incl.
  `sdk_session_fork`) across direct, Legacy queue, Durable exact queue,
  public `add_streaming_input`; provenance retention for all six variants
  through the real busy-thread queue path, asserted against constructor
  serde output; golden JSON #1/#2/#3.
- **Ingress**: per-ingress tests for chat, atomic dispatch, create-thread
  (+copy-through), plugin inbound, **and each built-in channel**:
  runtime-shaped generic metadata stays in `metadata.external`;
  provenance-shaped input is inert (no internal marking, no lifecycle
  flip — task wake still fires; no presentation); typed-form fingerprints
  (collision test); create-thread run-scoped override rejection;
  `ThreadRuntimeConfig` written only by internal paths.
- **Models/clients**: unchanged confirmed matrix (golden kinds, captured
  fixtures, malformed/unmarked negatives, rows-hash upsert, delta
  roundtrip, same-seq reseed, mapper tests, no-text-kind guard, Storybook,
  width/alignment real-layout, clamp/expand incl. eviction-surviving
  snapshot and occurrence-switch dismissal).
- **Prompt**: capability block across provider × default/custom × prompt
  present/blank.
- **Migration**: all historical shapes; forged-history negatives per §3.5
  item 3; secret-scrub assertions on jsonl + history API of a real-copy
  rehearsal; first-fork-after-scrub correctness; crash/restart idempotence
  at the three injection points; per-generation marker dedup/regeneration;
  old-cursor refetch.
- **Cutover**: v2×old, old×v2, history-wire stability, cache-schema
  upgrade + offline cold-open.
- **End-to-end evidence attribution**: desktop — worktree build ↔
  installed `app.asar` hash equality verified before and after, **and**
  the attached renderer's `--app-path` confirmed to point at the
  just-installed app; iOS — worktree build code carrier ↔ installed
  carrier hash equality (Debug: `<Product>.debug.dylib`, else executable)
  before and after; busy-thread `--notify current-thread` E2E through
  committed record → desktop collapsed card → dialog; iOS
  `xcodebuild`+simulator collapsed → full-screen → complete body →
  occurrence-switch dismissal; SwiftPM mapper tests from the captured
  frame.

## 6. Rollout

(confirmed; unchanged) One change set; desktop+gateway together; migration
on first boot; iOS via normal release flow with v2 minimum build;
TestFlight only with explicit user approval in that turn. Legacy shapes
and the legacy string wire each have one consumer: the migration and the
upgrade-required path.
