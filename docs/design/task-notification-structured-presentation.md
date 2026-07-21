# Task Notification Structured Presentation — Slice A

Status: final scope per owner decision 2026-07-21 ("做A就行了").
Nine adversarial review rounds explored a much larger surface; the owner
cut scope back to the original need. Everything outside this document is
recorded in `task-notification-review-debt.md` (slice B security debt /
slice C architecture) — explicitly **out of scope** here.
Owner: Gary (orchestrator thread)
Scope: garyx-bridge (queue metadata), garyx-gateway (producer + strip),
garyx-models (presentation), desktop renderer, iOS

## 1. The need (original, unchanged)

1. Task notifications render as raw XML on Mac/iOS in ~29% of live cases
   (73/252 measured): the busy-thread queue path filters dispatch
   metadata through a five-key allowlist that drops the notification
   semantics, so the reducer never marks `presentation`.
2. Both clients regex-parse the English prose template word-for-word —
   any template edit silently breaks them.
3. Product requirements: the card is not full-width — user-side
   (trailing) at the ordinary user-bubble width; body collapsed to ~10
   lines; tap to expand — modal dialog on Mac, full screen on iOS.

## 2. Changes

### 2.1 Bug fix: the notification object survives the queue

- The queue keeps its **bounded projection** (the current allowlist
  mechanism) — full-map pass-through is rejected for slice A: with
  plugin/CreateThread ingress hardening deferred to slice B, copying
  arbitrary caller metadata into pendings would let externally forged
  `task_notification` objects reach committed records on the busy path
  and be upgraded into trusted-looking cards, a *new* forgery surface
  the allowlist incidentally prevents today. It would also record
  busy-request fields (model/provider/workspace) that never applied to
  the already-running provider as if they had.
- `DISPATCH_ATTRIBUTION_METADATA_KEYS` gains exactly one entry:
  `task_notification`. No other source keys are added in slice A.
- The helper's second job is kept explicitly: the queue projection
  still writes `origin_run_id = requested_run_id` (the notify-dispatch
  attribution — the full metadata never contains it and the pending's
  `bridge_run_id` is the active run, so deleting the helper would lose
  it permanently). ACK asserts `origin_run_id` is the requested run,
  not the active run.
- `RUNTIME_ONLY_METADATA_KEYS` and the direct persistence path are
  untouched (the direct-path secret leak is pre-existing slice B1 debt;
  slice A must not widen or narrow it).
- `acknowledge_pending_input` merge semantics unchanged (bookkeeping
  keys win).

### 2.2 Structured notification metadata + envelope

- `deliver_task_review_handoff` emits one cohesive metadata object,
  replacing the scattered bool/string keys (task_hooks' reader moves to
  the object; the legacy scattered shape has no remaining writer):

  ```json
  "task_notification": { "event": "ready_for_review",
                          "status": "in_review",
                          "task_id": "#TASK-42",
                          "title": "..." }
  ```

- Message text restructure — the card body must be pure handoff, and the
  agent tutorial is part of the *letter*, not the *notification*:

  ```
  <garyx_task_notification event="ready_for_review" task_id="#TASK-42"
      status="in_review" title="...">
  {final_message}
  </garyx_task_notification>

  View details: garyx task get #TASK-42
  Review next: … (unchanged tutorial text)
  ```

  The tutorial block moves **outside** the XML envelope (agents still
  read it; the envelope strip never includes it; no prompt changes
  needed). `title` joins `task_id` under `xml_attr` escaping, extended
  to encode CR/LF/TAB as numeric character references (multiline-title
  test). Existing body close-tag neutralization stays.
- **Forgery mitigation in scope**: `task_notification` becomes a
  reserved key stripped at **all four** raw-metadata ingresses — chat
  (`prepare.rs`, next to the existing agent-identity strip), atomic
  dispatch (`create_dispatch.rs`), `CreateThreadBody.metadata` (thread
  metadata is later copied into dispatches), and the plugin host's
  `extra_metadata` intake. Each is a one-key strip at an existing
  boundary — this is not the slice-B ingress refactor, which remains
  deferred.

### 2.3 Models: flattened presentation payload

```json
"presentation": { "kind": "task_notification",
                  "event": "ready_for_review", "status": "in_review",
                  "task_id": "#TASK-42", "title": "..." }
```

- Rust: struct-variant enum, serde `tag = "kind"`; all four fields
  required; golden JSON locks the wire.
- Reducer: `render_message_presentation` reads the structured
  `task_notification` object (user-role rows only). Missing/malformed →
  no presentation → ordinary text. Kind is never derived from message
  text. Legacy scattered-key records (all history) get no presentation
  and render as plain text — **no migration** (owner-accepted).
- Impact closure: desktop `transcript.ts` string enum → discriminated
  object (all comparison sites); iOS decoder single-string → object via
  a lossy decoder (unknown `kind` degrades that row, never the frame —
  dedicated unknown-kind tests both platforms); iOS message signature
  hashes the full payload; rows-hash test for a title/status-only delta
  upsert; Storybook scenario becomes a real user render row.
- Wire cutover: desktop ships atomically with the gateway (same repo,
  same release). iOS ships via the normal release flow. **Honest
  old-iOS failure mode** (owner-accepted, no compatibility logic): the
  current decoder reads presentation as a single String, so against a
  new gateway an old app does not "degrade gracefully" — a full
  snapshot drops the affected row entirely (lossy row array), and a
  delta upsert containing an object presentation decodes as a malformed
  delta and triggers the gap/replay path. Validation covers these two
  real failure shapes, and the rollout note tells the owner to update
  iOS promptly after the gateway ships. No schema negotiation (slice
  C).

### 2.4 Clients: delete the task prose parsers; card UI

(The UI contract below passed review rounds 7–9 with zero findings.)

- Delete desktop `parseTaskNotificationText` (and its dead
  `detailCommand`/`reviewCommands` fields) and iOS
  `GaryxTaskNotificationPresentation.parse` regexes. One structural
  envelope decoder per platform — take the text between the opening
  tag's `>` and the last `</garyx_task_notification>`; wording-
  independent; tolerant of legacy bodies — reachable **only** when the
  server presentation is present. `statusLabel` formatting stays
  client-side. (Restart notices keep their current parsers; folding
  them into this mechanism = slice B/C.)
- **Alignment/width**: the card is a user-role row, trailing-aligned
  under the same layout owner as ordinary user bubbles:
  - iOS: extract the existing user-role container — trailing alignment,
    Dynamic Type width `0.77 ×` screen normal / `0.94` at `xxxLarge+`,
    min leading spacer 60/12, trailing menu edge — into a shared owner
    used by both plain user bubbles and the card; delete the full-width
    leading `taskNotificationRow` branch; copying `0.77` or minting a
    card constant is forbidden.
  - Desktop: `.message-bubble.user { max-width: 77%; align-self:
    flex-end }` is the owner; delete the card's
    `align-self`/`max-width`/736px overrides; the card may fill the
    shared cap but declares no constants. The existing test asserting
    full-width is inverted.
  - Width rules bind the collapsed transcript card only.
- **Collapsed card**: header (task id, status pill, title) + body
  clamped to the height of **10 line-boxes at the current body font**
  (not 10 source lines). Overflow = pure function
  `overflows(naturalHeight, clampHeight, ε)` with injected ε, measured
  after the shared width applies, re-measured on content, width, font
  scale, Dynamic Type, and intrinsic settling (image load, font
  resolution, markdown table/code re-layout). Decision lives in
  `GaryxMobileCore` (SwiftUI supplies a measurement adapter; desktop
  mirrors the split). Overflow ⇒ expand affordance + accessibility
  action; fits ⇒ neither. Expand activation excludes interactive
  descendants (links, copy/select/share, long-press menus keep working
  in both states).
- **Expanded view** — same header + the complete envelope-stripped body
  (single strip, shared string, captured as an immutable snapshot at
  activation):
  - Desktop: controlled Radix `Dialog`, stable owner, focus trap,
    Escape, focus returns to the card on close; standard large-dialog
    sizing.
  - iOS: full-screen presentation owned by the stable conversation/route
    occurrence owner via one always-attached
    `.garyxFullScreenCover(item:)` (capsule-preview pattern); rows send
    selection actions only; selection identity is message seq/id (one
    task can notify repeatedly); the snapshot keeps the body complete if
    the row is evicted by the render window; selection clears on
    thread/gateway/occurrence change; modifiers never conditionally
    attached.

## 3. Non-goals (owner-cut scope — see the debt document)

Typed metadata containers; trusted/untrusted store write surfaces;
provenance types; MCP capability identity; plugin/channel ingress
hardening; external override typing; restart-notice mechanism change;
history migration of any kind; render-schema negotiation; client cache
versioning; internal/internal_kind retirement; history envelope changes.

## 4. Validation

- **Queue fix**: busy-thread dispatch (real `QueuedToActiveRun` path, no
  direct-commit shortcut) commits a user record whose metadata contains
  the `task_notification` object, `internal_dispatch`, and
  `origin_run_id` = the requested (notify-dispatch) run id, never the
  active run's. **Pending-at-rest assertion**: block the provider ACK,
  read the persisted thread record, and assert the pending's metadata
  key set is exactly the bounded projection (allowlist keys +
  `task_notification` + `origin_run_id`) — in particular none of the
  eight runtime keys — then release the ACK and assert the committed
  transcript. Run once through the Legacy queue path and once through
  the Durable exact path, plus a focused unit test on the single
  enqueue constructor.
- **Producer**: golden text fixture — envelope attributes escaped
  (multiline title), body = pure final_message, tutorial outside the
  envelope; task_hooks reads the object; forgery negatives at all four
  strip points — chat, atomic dispatch, CreateThread metadata (including
  the copy-through into a later dispatch), and plugin `extra_metadata` —
  each carrying `task_notification`, asserted on both the direct path
  and the busy-queue path: the key never commits and no card renders.
- **Models**: golden presentation JSON; reducer fixtures (new object
  shape → presentation; measured legacy dropped-marker and scattered-key
  shapes → none); malformed negatives; unknown-kind row degradation on
  both platforms; title/status-only rows-hash delta upsert; delta
  roundtrip; same-seq reseed.
- **Clients**: mapper tests from captured frames; grep guard — no client
  code path derives the card from text ("is ready for review" appears in
  no client source; envelope decoder reachable only behind
  presentation); negative: identical unmarked text renders plain.
  Width/alignment real-layout tests (long user text vs long card share
  trailing edge and max width across resize and both iOS Dynamic Type
  branches; no card-local constants). Clamp matrix: exact-fit with
  injected ε, one long wrapping line, 11 explicit lines,
  list/code/table/image bodies, overflow→fit on widening, Dynamic Type
  change, late image/font settling; no affordance on short bodies;
  gesture coexistence; dialog focus trap/Escape/focus return; snapshot
  survives row eviction; occurrence/gateway-switch dismissal.
- **E2E**: with the gateway running, a task with
  `--notify current-thread` into a busy thread → committed record shape
  → desktop packaged-renderer collapsed card → dialog (app.asar hash
  verified against the worktree build; renderer `--app-path` checked);
  iOS SwiftPM mapper tests from the captured frame + simulator flow
  (collapsed → full screen → complete body; carrier hash verified).

## 5. Rollout

One change set; desktop + gateway ship together; iOS through its normal
release flow (TestFlight upload only with explicit owner approval in
that turn). No migration, no data backfill. Historical notifications
render as plain text — owner-accepted.
