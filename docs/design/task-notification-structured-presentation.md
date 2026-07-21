# Task Notification Structured Presentation

Status: draft for adversarial review
Owner: Gary (orchestrator thread)
Scope: garyx-bridge, garyx-gateway, garyx-models, desktop renderer, iOS

## 1. Problem

Mac and iOS render `<garyx_task_notification>` messages as raw XML text in a
large fraction of cases. Measured on live data (2026-07-21, last 80
transcripts): 252 committed task-notification user records, **73 (29%) missing
the `task_notification` metadata marker**, concentrated in busy orchestrator
threads — exactly the threads the user watches.

Two independent defects share one root theme: **structure is thrown away and
then badly reconstructed downstream.**

### Defect A — queue path drops semantic metadata (the live bug)

- Idle target thread: internal dispatch starts a run; the committed user
  record carries full dispatch metadata including `task_notification: true`
  and `task_notification_event: "ready_for_review"`. The `garyx-models`
  reducer marks `presentation: task_notification`; both clients render the
  card. Works.
- Busy target thread (`QueuedToActiveRun`): the message enters the pending
  input queue through `dispatch_attribution_metadata`
  (`garyx-bridge/src/multi_provider/run_management.rs:107`), which filters
  metadata through the allowlist `DISPATCH_ATTRIBUTION_METADATA_KEYS =
  [source, automation_id, cron_job_id, cron_action, internal_dispatch]`.
  Every `task_notification*` key is dropped. The committed record has no
  marker, the reducer never sets `presentation`, both clients fall back to
  raw-XML markdown.

This allowlist contradicts the codebase's own single-chokepoint contract:
`persistence.rs:1147` declares `RUNTIME_ONLY_METADATA_KEYS =
["garyx_mcp_auth_token", "remote_mcp_servers"]` as "the single persistence
chokepoint — every dispatch source funnels through it", with **denylist**
semantics (persist everything except runtime wiring). The queue path invented
a second, opposite-polarity filter. Two filters drift; this bug is the drift.

Product owner decision (2026-07-21): internal dispatch messages must persist
with the same metadata logic as ordinary user messages. No parallel filter.

### Defect B — prose round-trip (the architectural defect)

`format_task_ready_notification` (gateway) receives structured fields
(`task_id`, `title`, `final_message`), flattens them into an English prose
template plus CLI usage instructions, wraps them in XML — and then desktop
(`task-notification.ts`, TS regex) and iOS
(`GaryxTaskNotificationPresentation.swift`, NSRegularExpression) each maintain
a parser that reverse-engineers the prose, coupled word-for-word to the
template ("Task X is ready for review:", "\nView details:"). Any template
edit silently breaks both parsers back to raw XML. This violates the
repository direction that render semantics are server-owned and clients must
not recompute them.

Product owner decision (2026-07-21): parsing collapses into the server side /
render_state; clients dumb-render structured fields. No client prose regex.

## 2. Design principles

1. **One metadata persistence semantic.** Queued and direct paths persist the
   same dispatch metadata, cleaned by the same `RUNTIME_ONLY_METADATA_KEYS`
   denylist. The attribution allowlist is deleted, not extended.
2. **Structure is never round-tripped through prose.** The producer already
   holds the structured fields; they ride the message as metadata and are
   projected by the reducer. Nobody — server or client — regex-parses the
   prose body at runtime.
3. **The prose/XML body remains the agent-facing surface.** Receiving agents
   read the text. Human UI reads the projection. Both are generated from the
   same source at the same commit point, so they cannot drift.

## 3. Changes

### 3.1 Bridge: delete the attribution allowlist

- Delete `DISPATCH_ATTRIBUTION_METADATA_KEYS` and
  `dispatch_attribution_metadata`.
- At the two queue call sites (`run_management.rs:1117`, `:1132`), pass the
  full dispatch `metadata` minus `RUNTIME_ONLY_METADATA_KEYS`, plus the
  existing `origin_run_id` (the requested run id recording which dispatch
  request queued this input — keep it).
- The strip must happen **at enqueue time**: `PendingUserInput` is persisted
  inside the run record, so the gateway bearer token and managed MCP
  definitions must never enter it. Move/export `RUNTIME_ONLY_METADATA_KEYS`
  so both persistence and enqueue reference the same constant (single truth
  source).
- `acknowledge_pending_input` already merges `pending_input.metadata` into
  the committed record (queue bookkeeping keys win on conflict). Unchanged.
- Resulting committed shape for a queued internal dispatch:
  `{queued_input_id, queued_at, origin_id?, origin_run_id, internal_dispatch,
  task_notification…, requested_provider_type, …}` — i.e. exactly the
  dispatch metadata, like a direct-path record, minus run-runtime context
  that only a fresh run would add. Note this also fixes the same latent drop
  for every other internal dispatch flavor (followups, automation prompts
  queued into busy threads).

### 3.2 Gateway: structured metadata object, restructured envelope

- `deliver_task_review_handoff` puts one structured object on the dispatch:

  ```json
  "task_notification": {
    "event": "ready_for_review",
    "status": "in_review",
    "task_id": "#TASK-42",
    "title": "<task title>"
  }
  ```

  replacing the scattered `task_notification: bool` +
  `task_notification_event` + `task_notification_source_run_id` keys.
  `task_id` / `task_thread_id` / source run id fold into or alongside this
  object (reviewer to confirm final key layout; no boolean+string scatter).
- Envelope restructure: all structured fields become XML attributes; the body
  becomes the pure handoff message:

  ```
  <garyx_task_notification event="ready_for_review" task_id="#TASK-42"
      status="in_review" title="...">
  {final_message}
  </garyx_task_notification>
  ```

  The "Task X is ready for review:" headline and the "View details / Review
  next / garyx task update …" CLI tutorial block are **deleted from the
  body**. Receiving agents get the identity from attributes and already know
  the review workflow from their own prompts/skills; both clients already
  refused to render the tutorial block. Existing `xml_attr` escaping and
  `neutralize_task_notification_tag` body guard stay.
- Bot-channel delivery (`send_notification_message`) sends the same text as
  today (now shorter). Human readability of raw XML on Telegram/Discord is a
  known, separate issue — out of scope here.

### 3.3 Models: presentation carries the payload

- `RenderMessagePresentation` becomes a tagged enum with payload:

  ```rust
  #[serde(tag = "kind", rename_all = "snake_case")]
  pub enum RenderMessagePresentation {
      TaskNotification(TaskNotificationPresentation),
  }

  pub struct TaskNotificationPresentation {
      pub event: String,
      pub status: String,
      pub task_id: String,
      pub title: String,
  }
  ```

  Wire: `"presentation": {"kind": "task_notification", "event": …,
  "status": …, "task_id": …, "title": …}` (flattened or nested — reviewer
  picks one; must be a single discriminated object so identity and payload
  cannot desync).
- `render_message_presentation` reads the structured `task_notification`
  metadata object. **No text parsing in the reducer.** A record whose
  metadata object is missing/malformed gets no presentation and renders as
  ordinary text (today's fallback semantics).
- Payload is small fixed fields only. `final_message` is **not** copied into
  render rows (projection-lightweight contract): clients derive the card body
  from the message text they already cache, via the structural envelope strip
  (3.4).

### 3.4 Clients: delete the prose parsers

- Desktop: delete `parseTaskNotificationText` prose/regex logic and the
  `detailCommand`/`reviewCommands` fields (already unrendered dead weight).
  `TaskNotificationCard` reads `event/status/task_id/title` from the row's
  presentation payload. Card body = message text with the XML envelope
  stripped — a purely structural rule (first `>` of the opening tag … last
  `</garyx_task_notification>`), independent of wording, tolerant of the
  legacy body shape.
- iOS: same. `GaryxTaskNotificationPresentation.parse` regexes are deleted;
  keep only the envelope strip + `statusLabel` formatting. The mapper feeds
  payload fields from the snapshot ref.
- Both clients keep the existing "no presentation → ordinary text" path.
  No client-side sniffing of `<garyx_task_notification` to resurrect cards.

### 3.5 History: versioned transcript metadata backfill

One versioned one-shot migration (durable marker, same regime as
`recent_task_thread_kind_v1`):

- Records with legacy scattered keys (`task_notification: true` +
  `task_notification_event`) → rewrite metadata to the structured object,
  extracting `task_id`/`title` from the legacy prose (regex against the
  frozen legacy template is acceptable **inside a one-shot migration**; it is
  not a runtime path).
- The 73 dropped-marker records: identified by
  `origin_run_id` prefix `task-notify-` **and** body shape; backfill the same
  structured object.
- Message text is history and is not rewritten. Legacy bodies will show the
  headline + tutorial block inside the card body; acceptable, still far
  better than raw XML.
- Open question for review: is a transcript-jsonl metadata rewrite acceptable
  under the committed-event identity rule `(seq, payload)`? Proposed answer:
  the migration runs at boot before serving; connected clients reseed via
  snapshot replay; `render_state` is derived, not stored. If the reviewer
  finds a hard conflict, the fallback is: no backfill, history stays plain
  text (bug only fixed forward). Decide explicitly.

## 4. Non-goals

- Restart notices use the same client-side prose-sniffing pattern (both
  clients regex the text, no server presentation at all). Same disease,
  separate surgery: migrate it onto this presentation mechanism in a
  follow-up, not in this change.
- Bot-channel (Telegram/Discord) human-readable rendering of notifications.
- Any change to task lifecycle, notification targets, or delivery routing.

## 5. Validation

- **Bridge**: test that a `task_ready_for_review` dispatch queued into an
  active run commits a user record whose metadata contains the structured
  `task_notification` object and `internal_dispatch`, and does **not**
  contain `garyx_mcp_auth_token` / `remote_mcp_servers`. Drive with the real
  queue path (busy thread), not a direct-commit shortcut.
- **Models**: reducer tests with captured record fixtures (both the new
  structured shape and post-migration legacy shape) asserting the wire
  presentation object; absence test for malformed metadata.
- **Desktop/iOS**: headless mapper/render tests from real captured
  render_state snapshots; card asserts fields from payload, body from
  envelope strip; zero regex on template wording (grep-level check: the
  "is ready for review" literal appears in no client source).
- **Migration**: fixture DB with all three historical shapes (legacy keys,
  dropped-marker, already-new) → single boot migration → assert projected
  presentation for all three; marker prevents re-run.
- **End to end**: with gateway running, create a task with
  `--notify current-thread` into a busy thread; observe the committed record
  and the rendered card on desktop; confirm the same record renders on iOS
  via SwiftPM mapper test against the captured frame.

## 6. Rollout

Single change set, all ends in one merge (desktop and gateway ship together;
iOS targets the same wire). No compatibility shims, no dual-reading of the
legacy scattered keys at runtime — the migration is the only consumer of the
legacy shape.
