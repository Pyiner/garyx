# `schedule_followup` — delayed self-wake for long-running tasks

`schedule_followup` is an MCP tool exposed by the Garyx gateway as
`mcp__garyx__schedule_followup`. When an agent is partway through a task that
depends on external progress (a remote build, a queued job, "sleep N minutes
then poll"), it can use this
tool to park the turn cleanly and have the gateway re-wake the same
thread later with a synthetic user message — instead of leaving the
conversation silent until the user re-engages.

The synthesized re-wake is delivered through the same path as
restart-wake and task-ready notifications
(`dispatch_internal_message_to_thread`), so it appears to the agent
exactly like a normal user turn — except it carries a
`<garyx_followup_metadata>` header so the agent can recognize it as
its own scheduled followup.

## Tool contract

### Input

| Field | Type | Required | Notes |
|---|---|---|---|
| `delay_seconds` | integer (u64) | yes | Wall-clock delay before the re-wake. Must be in `60..=86400`. Out-of-range values are **rejected** with `out_of_range` — never silently clamped. |
| `prompt` | string | yes | Verbatim text that will be injected back into the thread as the synthetic user turn body, after the metadata header. |
| `reason` | string | no | Free-text telemetry note; echoed into the metadata header for the agent to see when it resumes. |

### Output

```json
{
  "tool": "schedule_followup",
  "status": "ok",
  "schedule_id": "followup_4d7c5b8f12ab9e3a",
  "scheduled_for_iso": "2026-05-26T07:30:00+00:00",
  "scheduled_for_unix_ts": 1779783000,
  "thread_id": "thread::...",
  "originating_run_id": "run-...",
  "delay_seconds_requested": 300,
  "reason": "background build finished",
  "replaced_previous": null
}
```

When a second call from the same `(thread_id, run_id)` replaces an
earlier schedule, `replaced_previous` is non-null and carries the
prior job's `schedule_id`, `was_scheduled_for_iso`,
`was_scheduled_for_unix_ts`, and (if recoverable from the persisted
payload) `delay_seconds_requested`, `reason`, `originating_run_id`,
and `scheduled_at`.

### Dedupe semantics

Dedupe key is `(thread_id, originating_run_id)`. The job id is a
deterministic 16-hex-char FNV-1a hash of those two values, prefixed
with `followup_`. Two calls from the same agent run on the same
thread therefore **replace** each other; calls from a later run on
the same thread create a **new** independent schedule.

This matches the typical use case where the agent ratchets its own
delay forward as new information arrives ("actually wait another 60s
because the queue depth went up"), without leaking stale schedules.

## Synthetic user turn shape

When the cron service fires the followup, it dispatches the following
body into the thread:

```
<garyx_followup_metadata>
schedule_id: followup_4d7c5b8f12ab9e3a
scheduled_at: 2026-05-26T07:25:00+00:00
scheduled_for: 2026-05-26T07:30:00+00:00
delay_seconds_requested: 300
reason: background build finished
originating_run_id: run-...
</garyx_followup_metadata>

<the verbatim prompt the agent originally supplied>
```

The metadata block:

- `scheduled_at` is the wall-clock time the agent called
  `schedule_followup`.
- `scheduled_for` is the wall-clock time the cron tick actually
  fired — equal to `scheduled_at + delay_seconds_requested` unless a
  later call bumped the schedule.
- `reason` and `originating_run_id` are only present if they were
  supplied / known.

Additionally, the dispatch attaches structured `extra_metadata`
fields to the synthetic user turn so non-agent surfaces can filter
followup-driven runs distinctly from organic user input:
`schedule_followup`, `schedule_followup_job_id`,
`schedule_followup_scheduled_at`, `schedule_followup_scheduled_for`,
`schedule_followup_reason`, `schedule_followup_originating_run_id`.

## What this tool is *not* for

- **Recurring tasks**: use a regular gateway automation
  (`CronJobKind::AutomationPrompt`). `schedule_followup` jobs are
  one-shot — they self-delete after firing.
- **Polling externally without an agent on the other end**: the
  followup re-wakes whatever thread the schedule was created on; if
  that thread has no provider attached, the dispatch will be a no-op.
- **Very short or very long delays**: under 60 seconds belongs in the
  agent's in-loop heartbeat; over 24 hours should be a regular
  automation that the user can see and edit.

## Under the hood

- The MCP tool validates `delay_seconds` and builds an
  `InternalDispatchJobPayload`.
- It calls `CronService::upsert` with a `CronJobConfig` whose `kind`
  is `CronJobKind::InternalDispatch { payload }`, `system: true`,
  `delete_after_run: true`, and `schedule:
  CronSchedule::Once { at: now + delay }`.
- `system: true` keeps the job out of the user-facing automation list
  (`CronService::list`). It still shows up in `list_all` for internal
  consumers and in `get(id)` for direct lookup.
- On tick, `CronService::execute_job` notices the
  `CronJobKind::InternalDispatch` variant, renders the synthetic body
  via `build_followup_body`, and dispatches through the
  `AutomationDispatchPort` of its injected execution environment
  (`composition::automation_wiring` implements the port over a
  `Weak<AppState>` and calls `dispatch_internal_message_to_thread`).
  The engine itself holds no `AppState` reference, so no circular
  `Arc` is formed between `AppState` and `CronService`.

## Boundary handling & retries

When a followup fires, the originating thread or the dispatch itself may be in a
state that prevents delivery. These cases are handled explicitly rather than
dropped silently:

- **Thread deleted.** If the originating thread is no longer in the thread store
  by the time the followup fires, the run is recorded as `failed_dropped` with a
  `thread not found: <id>` reason. This is non-retryable — retrying cannot bring
  the thread back.
- **Transient dispatch failure.** A network/internal error during dispatch is
  retried up to `FOLLOWUP_MAX_RETRIES` (3) times with exponential backoff
  (≈200ms / 400ms / 800ms). If a retry succeeds, the run is `success` and the
  intermediate failures are logged at `warn`. If the budget is exhausted, the
  run is recorded as `failed_dropped` with the concrete underlying error.
- **Terminal drops are final.** A `failed_dropped` outcome disables the one-shot
  job (and honors `delete_after_run`) so a dropped followup never re-fires on a
  later tick.
- **Thread stopped / cancelled.** There is no dedicated dispatch-time signal for
  a stopped-but-still-present thread today, so such a thread still receives the
  injected turn (which starts a fresh turn). The drop classifier is structured
  to accommodate such a signal if one is added later.

Every drop path emits a `tracing::warn`, and the persisted run record carries
the run `status` and reason, so drops are observable through the debug cron
inspection routes and never silent.

The `RunRecord.status` enum gains a `failed_dropped` value
(`JobRunStatus::FailedDropped`) distinct from `failed`. It is additive: older
persisted run records without it still deserialize.

## Observability

`schedule_followup` jobs are `system: true`, so they do not show up in the
user-facing automation list. To inspect them during an incident — list the
pending followups, see each job's `RunRecord` history, or manually fire one —
use the debug endpoint documented in
[schedule-followup-observability.md](./schedule-followup-observability.md).

## Backwards compatibility

The `CronJobConfig.system` field and the `CronJobKind::InternalDispatch`
variant both use `#[serde(default)]` / externally-tagged JSON. Cron job
state written by older gateway builds (which never set either) round-
trips as `system = false` + `kind = "automation_prompt"`, so an
in-place upgrade does not invalidate persisted automations.
