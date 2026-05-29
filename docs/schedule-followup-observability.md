# `schedule_followup` observability

`schedule_followup` (see [schedule-followup.md](./schedule-followup.md)) creates
its cron jobs with `system: true`, which keeps them out of the user-facing
automation list (`GET /api/cron/jobs` filters `system` jobs out). That is the
right default — these are internal self-wake schedules, not automations a user
manages — but it leaves a blind spot during incidents like *"the agent promised
a followup and it never came back."*

The debug endpoint `GET /api/debug/system-cron-jobs` closes that gap: it lists
the system cron jobs (including `schedule_followup`-created followups) together
with each job's recent `RunRecord` history, and offers a system-only manual
fire.

## Authorization

The debug routes live under the gateway's **protected** router, so they inherit
`enforce_gateway_auth`:

- Requests from **loopback** (`127.0.0.1` / `::1`) pass without a token. On the
  gateway host, `curl http://127.0.0.1:<port>/api/debug/system-cron-jobs` just
  works.
- Any **non-loopback** request must carry a valid gateway token (the same token
  used for the rest of the gateway API — `Authorization: Bearer <token>`, the
  `x-garyx-token` header, or `?token=`). No token / wrong token → `401`.

This reuses the existing gateway auth token rather than introducing a separate
debug-token config surface. The endpoint is never exposed unauthenticated to
external callers.

## `GET /api/debug/system-cron-jobs`

Lists every cron job with `system == true`, plus each job's recent runs.

### Query parameters

| Param | Type | Default | Notes |
|---|---|---|---|
| `thread_id` | string | — | Exact match on the job's `thread_id`. Empty / whitespace-only is ignored (returns all system jobs). |
| `since` | string | — | Lower bound on the job's `created_at`. Accepts a **unix-second timestamp** (all digits) or an **RFC3339** datetime. Jobs created strictly before this instant are filtered out. A value that parses as neither form returns `400 invalid_since` — it is never silently treated as "no filter". |
| `runs_limit` | integer | `20` | Max recent `RunRecord`s attached per job, most-recent-first. |

### Response

```json
{
  "jobs": [
    {
      "id": "followup_4d7c5b8f12ab9e3a",
      "label": "schedule_followup(thread::abc)",
      "kind": {
        "type": "internal_dispatch",
        "reason": "background build finished",
        "originating_run_id": "run-...",
        "scheduled_at": "2026-05-29T07:25:00+00:00",
        "delay_seconds_requested": 300
      },
      "schedule": { "once": { "at": "2026-05-29T07:30:00+00:00" } },
      "thread_id": "thread::abc",
      "agent_id": null,
      "enabled": true,
      "system": true,
      "delete_after_run": true,
      "next_run": "2026-05-29T07:30:00+00:00",
      "last_status": "never_run",
      "run_count": 0,
      "created_at": "2026-05-29T07:25:00+00:00",
      "last_run_at": null,
      "recent_runs": [
        {
          "run_id": "...",
          "job_id": "followup_4d7c5b8f12ab9e3a",
          "status": "failed",
          "started_at": "2026-05-29T07:30:00+00:00",
          "finished_at": "2026-05-29T07:30:00+00:00",
          "duration_ms": 12,
          "thread_id": "thread::abc",
          "error": "thread not found"
        }
      ]
    }
  ],
  "count": 1,
  "thread_id": null,
  "since": null,
  "runs_limit": 20,
  "service_available": true
}
```

When the cron service is not running, the endpoint returns `200` with
`{"jobs": [], "count": 0, "service_available": false}` (mirroring
`GET /api/cron/jobs`), so a probe never 500s just because cron is disabled.

### Reading a "followup never fired" incident

1. Filter to the thread: `GET /api/debug/system-cron-jobs?thread_id=thread::abc`.
2. If the job is **absent**, it already fired and self-deleted
   (`delete_after_run: true`) — check `GET /api/cron/runs` or the gateway logs
   for its terminal `RunRecord`.
3. If the job is **present** with `last_status: never_run` and a future
   `next_run`, it is still pending — the delay simply has not elapsed.
4. If `recent_runs` shows a `failed` record, the `error` field explains why the
   dispatch did not reach the thread (e.g. the thread was deleted or had no
   provider attached).

## `POST /api/debug/system-cron-jobs/{id}/run`

Manually fires a system cron job immediately — a system-only wrapper around
`CronService::run_now`. The debug channel must never be a back door to trigger
user-visible automations, so:

- A **missing** job → `404 not_found`.
- A job that exists but is **not** `system` → `404 not_found` (same shape as
  missing; the debug channel does not enumerate or fire user automations).
- A system job that **cannot run right now** (disabled or already running) →
  `409 not_runnable`.
- Otherwise → `200` with the resulting `RunRecord`:

```json
{ "ran": true, "run": { "run_id": "...", "job_id": "...", "status": "success", "...": "..." } }
```

## Implementation notes

- `GET` reuses `CronService::list_all` (the unfiltered list — `list()` hides
  system jobs) and `CronService::list_runs_for_job`. It is strictly read-only
  and does not repair or mutate any cron state.
- `POST .../run` reuses `CronService::get` (to enforce the system-only guard)
  and `CronService::run_now`.
- Handlers: `garyx-gateway/src/api.rs`
  (`debug_system_cron_jobs` / `debug_run_system_cron_job`); routes registered in
  `garyx-gateway/src/route_graph.rs` under `operations_routes()`.
- The default `GET /api/cron/jobs` and `GET /api/cron/runs` behavior is
  unchanged — these debug routes are additive.
