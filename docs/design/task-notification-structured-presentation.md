# Task Notification Structured Presentation

Status: revision 2 for adversarial review (round 1: FAIL, 3 BLOCKER + 8 MAJOR)
Owner: Gary (orchestrator thread)
Scope: garyx-bridge, garyx-gateway, garyx-models, garyx-router boot, desktop
renderer, iOS

## 1. Problem

Mac and iOS render `<garyx_task_notification>` messages as raw XML text in a
large fraction of cases. Measured on live data (2026-07-21, last 80
transcripts): 252 committed task-notification user records, **73 (29%) missing
the `task_notification` metadata marker**, concentrated in busy orchestrator
threads — exactly the threads the user watches.

Root theme: **structure is thrown away and then badly reconstructed
downstream**, and metadata durability is decided by accident of code path.

### Defect A — queue path drops semantic metadata (the live bug)

- Idle target thread: internal dispatch starts a run; the committed user
  record carries the dispatch metadata including `task_notification: true` /
  `task_notification_event`. The reducer marks `presentation`; clients render
  the card.
- Busy target thread (`QueuedToActiveRun`): the message enters the pending
  input queue through `dispatch_attribution_metadata`
  (`run_management.rs:107`), whose allowlist drops every `task_notification*`
  key. The committed record has no marker; both clients fall back to raw-XML
  markdown.

### Defect B — prose round-trip (the architectural defect)

`format_task_ready_notification` receives structured fields, flattens them
into an English prose template plus CLI tutorial text, wraps them in XML —
and desktop (`task-notification.ts`) and iOS
(`GaryxTaskNotificationPresentation.swift`) each maintain a regex parser
coupled word-for-word to the template. Any template edit silently breaks both
parsers. Restart notices (`<garyx_restarted>`) have the same disease: two
client prose sniffers, no server presentation at all.

### Defect C — runtime secrets have no structural boundary (found in review)

Dispatch metadata is one bare `HashMap` mixing durable semantic keys with
provider runtime configuration. Consequences today:

- The direct persistence path relies on a two-key denylist
  (`RUNTIME_ONLY_METADATA_KEYS`) while the run-metadata backfill injects far
  more runtime material — `provider_env` (may contain tokens),
  `garyx_mcp_headers` (goes into MCP HTTP headers), `desktop_antigravity_env`
  (arbitrary process env), `system_prompt` — some of which **already leaks
  into committed transcripts** on the direct path.
- `PendingUserInput.metadata` is persisted inside the run record and later
  serialized as the committed message metadata without passing any cleaner.

So neither "extend the allowlist" nor "share the denylist" is sound. The
boundary must be structural.

## 2. Design principles

1. **Typed metadata separation.** Dispatch metadata is split into two
   containers with different lifecycles:
   - `durable` — semantic message metadata; persists with the message
     everywhere (direct commit, pending queue, run record, transcript).
   - `runtime` — provider runtime configuration (tokens, MCP servers/headers,
     provider env, antigravity env, system prompt, managed wiring); consumed
     by provider assembly, **never persisted** into pending inputs, run
     records, or transcripts.
   The type system enforces the boundary: `PendingUserInput` and transcript
   persistence APIs accept only the durable container. No filter lists on the
   persistence side. Fields needed for audit (e.g. model, agent id) are
   explicitly projected as safe scalars into `durable`, never bulk-copied.
2. **Structure is never round-tripped through prose.** The producer already
   holds structured fields; they ride the message as durable metadata and are
   projected by the reducer. Nobody regex-parses prose at runtime — for task
   notifications **and** restart notices (no second mechanism left behind).
3. **The prose/XML body remains the agent-facing surface.** Receiving agents
   read the text; human UI reads the projection; both generate from the same
   source at the same commit point.
4. **Server-owned reserved keys.** `task_notification` and `restart_wake`
   metadata are producible only by the gateway's own dispatchers. The chat
   ingress strips them from external requests (alongside the existing agent
   identity strip in `prepare.rs`), so external callers cannot forge cards.

## 3. Changes

### 3.1 Bridge: typed containers replace both filters

- Delete `DISPATCH_ATTRIBUTION_METADATA_KEYS` and
  `dispatch_attribution_metadata`.
- Introduce the durable/runtime split at the type level on the dispatch
  path. The single enforcement point is the one place that constructs
  `PendingUserInput` (`add_streaming_input_with_metadata_mode`) plus the run
  persistence entry — not per-call-site cleaning. All enqueue entrances
  (Legacy, Durable exact, `execute_durable_stream_input`, public
  `add_streaming_input` used by channels/chat control) converge there and
  get the same guarantee.
- `PendingUserInput.metadata` carries the full durable container plus queue
  bookkeeping (`queued_input_id`, `queued_at`, `origin_id`, `origin_run_id`);
  `acknowledge_pending_input` merge semantics unchanged.
- Runtime keys at minimum: `garyx_mcp_auth_token`, `remote_mcp_servers`,
  `provider_env`, `garyx_mcp_headers`, `desktop_antigravity_env`,
  `system_prompt`. Classification is by container, not by name — this list
  seeds the migration of existing call sites, it is not a runtime filter.
- This unifies direct and queued persistence: the existing direct-path
  `provider_env`/`system_prompt` leak is fixed by the same boundary.
  `RUNTIME_ONLY_METADATA_KEYS` disappears with it (superseded by the type
  split).
- Internal-dispatch source inventory (all seven flavors keep their semantic
  keys in `durable`): task notification, `schedule_followup`, quota resend
  (rides `schedule_followup`), automation, cron, task auto-start
  (`task_auto_start`, `task_dispatch_reason`, `task_id`), restart wake
  (`restart_wake`, `restart_wake_id`, `kind`, `target`, `all`, `attempt`).

### 3.2 Gateway: one structured object, reserved keys, restructured envelope

- `deliver_task_review_handoff` emits a single cohesive durable object — no
  top-level aliases:

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

  The scattered `task_notification: bool` / `task_notification_event` /
  `task_thread_id` / `task_notification_source_run_id` keys are deleted.
  `task_hooks.rs`'s runtime read of the legacy bool moves to the new object;
  the legacy shape is read only by the history migration.
- Reserved-key enforcement: chat ingress (`prepare.rs`) strips
  `task_notification` and `restart_wake` from externally supplied metadata.
- Envelope restructure: attributes carry all structured fields (all escaped
  via `xml_attr`, including `title` — covering `" & < >`, newlines, embedded
  close tags on top of the existing body tag neutralization); the body is
  the pure handoff:

  ```
  <garyx_task_notification event="ready_for_review" task_id="#TASK-42"
      status="in_review" title="...">
  {final_message}
  </garyx_task_notification>
  ```

- **Prompt before deletion**: the approve/reject lifecycle moves into the
  base instructions injected for every provider
  (`gary_prompt.rs::GARY_BASE_INSTRUCTIONS`): approve → `--status done`;
  needs changes → feedback via `garyx thread send task` (wake) and status
  back to in_progress. A provider-envelope test asserts the injected prompt
  carries both rules. Only then is the "View details / Review next" tutorial
  block deleted from the notification body. One mechanism: prompt.
- Restart notices: the producer (`restart_wake.rs`) already attaches
  structured `restart_wake` metadata; no producer change beyond reserved-key
  protection.

### 3.3 Models: presentation carries the payload (flattened), plus restart

- `RenderMessagePresentation` becomes a discriminated object, **flattened**
  (reviewer decision — kind and fields cannot desync, shallower wire):

  ```json
  "presentation": {
    "kind": "task_notification",
    "event": "ready_for_review",
    "status": "in_review",
    "task_id": "#TASK-42",
    "title": "..."
  }
  ```

  ```json
  "presentation": { "kind": "restart_notice" }
  ```

  Rust: enum with struct variants, serde `tag = "kind"`, golden JSON tests
  lock the exact wire shape.
- `render_message_presentation` reads the structured durable objects
  (`task_notification`, `restart_wake`). No text parsing in the reducer.
  Missing/malformed object → no presentation → ordinary text (today's
  fallback semantics).
- Payload is small fixed fields only; `final_message` stays out of rows
  (projection-lightweight contract). Card/expanded body derive from the
  cached message text via structural envelope strip.
- Impact closure for the type change:
  - Desktop `transcript.ts` contract: string enum → discriminated object;
    all string-comparison sites updated.
  - iOS decoder: single-string → object decode; message signature hashes the
    complete payload (not `rawValue`) so presentation-only updates rebuild
    rows (`GaryxMobileToolTraceBuilder.swift:108`).
  - Server `rows_hash` already hashes whole rows; add a test that a
    title/status-only change produces a delta upsert.
  - Storybook scenario becomes a real user render row with a real
    presentation object (current fixture is assistant prose and proves
    nothing).

### 3.4 Clients: delete all four prose parsers, shared user-role layout

- Desktop deletes `parseTaskNotificationText` and `restart-notice.ts`; iOS
  deletes `GaryxTaskNotificationPresentation.parse` regexes and
  `GaryxRestartNoticePresentation` sniffing. Cards read presentation payload
  fields; body = envelope strip (structural rule only: after the opening
  tag's `>` … before the last close tag; tolerant of the legacy body shape).
  `statusLabel`-style display formatting stays client-side.
- No client-side sniffing to resurrect cards for unmarked records.

#### Alignment and width: the user-role layout owner rules

A task notification is a user-role row. The card aligns to the user side
(trailing) under the **same layout owner** as ordinary user bubbles — not a
copied constant:

- **iOS**: extract the existing user-role container (trailing alignment,
  Dynamic Type width policy `0.77 × screen` normal / `0.94` at
  `xxxLarge+`, min leading spacer 60/12, trailing menu edge) into a shared
  layout owner used by both the plain user body and the task card. The
  current `taskNotificationRow` full-width leading branch is deleted; the
  card's interaction menu switches to the trailing edge with the container.
  Copying `0.77` or minting a notification-specific width constant is
  forbidden.
- **Desktop**: `.message-bubble.user { max-width: 77%; align-self: flex-end }`
  is the owner. The task-card CSS overrides (`align-self`, 736px/full-width)
  are deleted; the card may `width: 100%` within the shared cap but declares
  no width/alignment constants of its own.
- Role comes from the committed record/reducer (user-role rows only);
  clients never infer user-side placement from the presentation kind. An
  assistant-role record with forged notification metadata gets no user-side
  card (reducer only marks user rows; see §5).
- These width rules apply to the **collapsed transcript card only**; the
  desktop expanded dialog uses standard large-dialog sizing and the iOS
  expanded view is full-screen — the 77% cap never propagates into the
  expanded body.

#### Collapsed card + expand

Notification handoffs are often multi-KB review verdicts; in-transcript cards
are compact summaries:

- **Collapsed (default)**: header (task id, status, title) + body viewport
  clamped to **the height of 10 line-boxes at the current body font** (not
  10 source lines — markdown blocks wrap and vary in height). Overflow
  decision: `naturalRenderedHeight > clampHeight + ε`, measured **after** the
  shared user-role width is applied, re-measured when content, width, font
  scale, or Dynamic Type change.
- The clamp decision lives in a testable layer: a measurement adapter feeds
  actual width / font scale / natural height; a Core/pure function decides
  `fits/overflows`. When it overflows, the card shows the expand affordance
  and an expand accessibility action; when it fits, neither exists (no dead
  interaction).
- Expand activation excludes interactive descendants (links, copy/select/
  share controls, long-press menus keep working in both states).
- **Expanded view** renders header + the same envelope-stripped body (single
  strip, shared string, no refetch):
  - **Desktop**: controlled Radix `Dialog` with a stable owner; focus trap,
    Escape, focus returns to the card on close.
  - **iOS**: full-screen presentation owned by the stable conversation/route
    occurrence owner (capsule-preview pattern): one always-attached
    `.garyxFullScreenCover(item:)`; rows only send a selection action;
    selection identity is the message seq/id (never `task_id` — one task can
    notify repeatedly); selection clears on thread/gateway/occurrence change;
    modifiers are never conditionally attached.

### 3.5 History: boot-time migration via the range-rewrite protocol

Silent same-seq rewrites violate the `(seq, payload)` committed-event
identity: SSE replay only covers `seq > cursor`, so caught-up clients would
keep stale committed caches. The migration therefore uses the repository's
existing rewrite-marker protocol (`reconcile.rs` `range_rewrite`):

1. Runs in the boot orchestrator that owns both the DB and
   `ThreadTranscriptStore`, after legacy import, before listener bind — not
   inside the SQL-only startup migration set.
2. Per transcript, under the store lock: read all records; rewrite only
   message metadata (legacy scattered keys → structured object; the 73
   dropped-marker records identified by `origin_run_id` prefix
   `task-notify-` + body shape get the object backfilled from attributes/
   legacy prose — regex against the frozen legacy template is acceptable
   inside a one-shot migration). `seq`/`thread_id`/`run_id`/`timestamp`
   untouched. Message text is history and is not rewritten (legacy bodies
   show the old headline/tutorial inside the card; acceptable).
3. Each changed transcript gets, in the same atomic file replacement, an
   appended new-seq `range_rewrite` control record with reason
   `task_notification_metadata_v1` — desktop and iOS sync planners already
   drop the window and refetch authoritatively on that marker.
4. Fix thread-record transcript count/tail projections in the same pass;
   migration must not bump user-facing activity ordering.
5. A durable global marker keyed to the import generation is written last,
   after all file and SQLite work. File-atomic per transcript, idempotent
   end-to-end (already-converted metadata / existing markers are recognized,
   no duplicate markers), any failure aborts startup and retries next boot.
6. Fault-injection tests around the non-atomic seam (after file rename,
   before SQL), plus a rehearsal against an isolated copy of the real
   production data directory.

### 3.6 Wire cutover: explicit schema boundary, no dual reading

`presentation` changing from string to object is a breaking render-state
change. No dual-read shim; an explicit protocol version instead:

- Define `render_schema = 2` for the render-state wire.
- Clients declare the schema on stream/history requests; every frame carries
  it back.
- The v2 gateway answers missing/older schema with an explicit
  upgrade-required error — it never emits the legacy string wire.
- v2 clients hitting an old gateway (no schema) fail fast with an upgrade
  notice — no string fallback path.
- Desktop ships atomically with the gateway (same repo, same release). iOS
  sets a minimum supported build; the forced-upgrade window is accepted
  (explicitly approved scope — iOS-26-only, no legacy compatibility policy).

## 4. Non-goals

- Bot-channel (Telegram/Discord) human-readable rendering of notifications.
- Any change to task lifecycle semantics, notification targets, or delivery
  routing.
- (Restart notices are **in scope** — see §3.3/§3.4; leaving them on prose
  parsing was rejected in review as a retained dual mechanism.)

## 5. Validation

- **Metadata boundary (bridge)**: sentinel-secret tests for
  `garyx_mcp_auth_token`, `remote_mcp_servers`, `provider_env`,
  `garyx_mcp_headers`, `desktop_antigravity_env` proving absence from run
  records and transcripts across **direct, Legacy queue, Durable exact
  queue, and public `add_streaming_input`** paths; semantic-key retention
  tests for all seven internal-dispatch sources through the busy-thread
  queue path (real queue, no direct-commit shortcut).
- **Forgery**: external chat requests carrying `task_notification` /
  `restart_wake` metadata commit without those keys and get no presentation;
  an assistant-role record with notification metadata gets no presentation.
- **Models**: golden JSON for the flattened serde wire (both kinds);
  reducer tests from captured fixtures (new shape + post-migration shape);
  absence test for malformed objects; presentation-only change → row-hash
  delta upsert test; delta roundtrip and same-seq reseed with object
  presentations.
- **Desktop/iOS mapping**: headless mapper tests from real captured v2
  frames; grep-level guard that no client source matches the template
  wording ("is ready for review") or `<garyx_restarted` sniffing; Storybook
  scenario uses a real user render row with presentation object.
- **Width/alignment**: real-layout tests (browser for desktop; both Dynamic
  Type branches for iOS) that a long user message and a long notification
  card share the same trailing edge and max width, across window resize;
  architecture guard that task-card CSS/SwiftUI declares no width/alignment
  constants (the shared owner does); the existing desktop test asserting
  full-width is inverted.
- **Clamp/expand**: fixtures at exactly-fits, one long wrapping line, 11
  explicit lines, list/code/table/image bodies; overflow→fit on widening;
  iOS Dynamic Type change re-measurement; expand affordance absent on short
  bodies; gesture coexistence (copy/select/share + expand); desktop dialog
  focus trap/Escape/focus return; iOS always-attached cover owner with
  seq-based selection identity.
- **Prompt**: provider-envelope test that base instructions include both
  approve and reject/feedback lifecycle rules for every provider type.
- **Migration**: fixture data dir with all three historical shapes; crash/
  restart idempotence; fault injection at the rename/SQL seam; import-
  generation marker; old-cursor client refetch driven by the range_rewrite
  marker; rehearsal on a copied real data dir.
- **Cutover**: v2 client × old gateway → explicit upgrade notice; old
  client × v2 gateway → upgrade-required error; no frame ever emitted in
  legacy shape by a v2 gateway.
- **End to end**: task with `--notify current-thread` into a busy thread;
  observe committed record shape, desktop card (collapsed → dialog), iOS
  mapper output from the captured frame.

## 6. Rollout

Single change set; desktop and gateway ship together; iOS follows via
TestFlight under the standing release authorization with the v2 minimum
build. The migration runs on first boot of the new gateway. The legacy
scattered metadata shape and legacy string presentation wire have exactly
one consumer each: the migration and the upgrade-required error path.
