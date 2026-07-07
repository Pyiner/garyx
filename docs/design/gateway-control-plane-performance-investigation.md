# Gateway Control Plane Performance Investigation

Date: 2026-07-07

## Context

Local CLI commands intermittently failed with messages like:

```text
gateway not reachable at http://127.0.0.1:31337
```

The immediate trigger was a `garyx thread send task '#TASK-0000' ...` call. The
gateway was not consistently down: it was listening on port `31337`, but some
lightweight HTTP requests were delayed long enough for short-lived CLI requests
to fail.

## Observed Symptoms

- `garyx status` sometimes reported the gateway as running at `[::]:31337`.
- `lsof` showed `garyx` listening on TCP port `31337`.
- `curl` could establish TCP connections to `127.0.0.1:31337`, but `/health`
  occasionally returned no HTTP response within 3 seconds.
- A repeated `/health` probe showed mostly sub-200ms responses, but also
  intermittent 3-second timeouts and one response around 830ms.
- A later probe saw `/health` take about 3.75 seconds while the gateway process
  remained alive.
- `launchctl print ... com.garyx.agent` showed the managed gateway had restarted:
  `runs = 10`, current PID `89341`, and the previous termination signal was
  `Killed: 9`.
- Gateway sampling showed Tokio runtime worker threads active or waiting around
  common run paths and mutex waits.
- The gateway process physical footprint had reached a sampled peak around
  5.5GB and later settled much lower.

## What "Control Plane Saturation" Means

This is not primarily a port-listening problem.

There are two layers:

1. The operating system accepts TCP connections because the gateway process is
   alive and listening on `31337`.
2. The Garyx application must schedule and execute the route handler for
   `/health`, `/api/tasks/{id}`, `/api/threads`, and similar routes.

The failure mode observed here is: TCP connect succeeds, but the application
does not produce an HTTP response promptly. That indicates request handling is
queued behind busy runtime work, blocked on shared resources, or both.

## Runtime And Worker Count

The gateway CLI entrypoint uses a bare `#[tokio::main]` in
`garyx/src/main.rs`.

Tokio's default for `#[tokio::main]` is a multi-thread runtime. On this machine:

```text
hw.logicalcpu = 16
```

So the runtime is expected to use roughly 16 async worker threads, plus auxiliary
threads. The gateway is asynchronous, but async does not make CPU work, blocking
filesystem calls, SQLite mutex contention, or synchronous locks disappear.

## Likely Contributors

### Restart Wake Load

After the gateway restarted at about 15:38, it dispatched a pending restart wake:

```text
pending restart wake dispatched ... thread_id=thread::<synthetic-thread-id>
```

That run completed after about 93 seconds and logged approximately 1.68 million
input tokens. This is a large resume workload inside the same gateway process
that also serves lightweight HTTP and desktop/mobile control-plane requests.

### Shared Runtime For Heavy Runs And Light HTTP

Provider runs, MCP handling, event ledger writes, render/projection updates, SSE
fanout, workflow operations, and HTTP control-plane routes all share the gateway
process and Tokio runtime. When a large run is active, lightweight requests can
still be delayed by worker scheduling, shared locks, DB access, or CPU-heavy
serialization.

### Synchronous Or Single-Point Resources

The codebase contains several places that can reduce async concurrency benefits:

- `rusqlite::Connection` wrapped behind mutexes in gateway DB layers.
- `std::sync::Mutex` / `std::sync::RwLock` in some shared state.
- synchronous filesystem operations in request-adjacent paths.
- large JSON/transcript/render work that can consume a worker until it reaches
  an `.await` or completes.

These are not necessarily bugs individually, but under large run pressure they
can turn 16 async workers into a queue behind a few shared bottlenecks.

### Workflow Definition Warning Noise

The local `development-loop` workflow package had a manifest pointing to
`workflow.mjs`, while current gateway workflow definition discovery requires a
fixed root `workflow.ts`.

That produced repeated warnings:

```text
workflow.ts is required in workflow package
```

This has been fixed locally by renaming `workflow.mjs` to `workflow.ts` and
removing the stale `entrypoint` field from the manifest. After the change,
`garyx workflow definition list` includes `development-loop` and no new warning
lines were emitted during verification.

## CLI Amplification

`garyx thread send task '#TASK-0000' ...` first resolves the task with:

```text
GET /api/tasks/{task_id}
```

That request uses a short timeout. If it lands during a gateway restart window or
control-plane saturation window, the command can fail before it ever opens the
chat WebSocket or sends the user message.

This makes a transient backend delay appear as a hard gateway failure.

## Recommended Fix Plan

### Phase 1: Protect Lightweight Control Plane

- Make `/health` strictly lightweight: no DB, no bridge locks, no expensive
  state construction.
- Add runtime lag reporting to `/health` or `/health/detailed`, such as last
  watchdog tick delay, active run count, queued restart wakes, and DB lock wait
  counters.
- Add retry with short exponential backoff to CLI gateway JSON helpers,
  especially task lookup before `thread send task`.
- Add a concurrency cap for restart-wake dispatch and provider runs.
- Change restart behavior so wake-all is explicit or heavily throttled.

### Phase 2: Move Blocking Work Off Async Workers

- Audit request paths for synchronous `std::fs` and move request-time filesystem
  scans to cache plus background refresh.
- Use `spawn_blocking` for known blocking SQLite or filesystem-heavy operations
  where they remain on request paths.
- Shorten lock critical sections and avoid holding locks across `.await`.
- Identify hot `std::sync::Mutex` / `RwLock` paths and split or replace them
  where they block Tokio workers under load.

### Phase 3: Separate Control Plane From Heavy Runtime Work

Longer-term architecture:

```text
Gateway control plane
  /health
  /api/tasks
  /api/threads
  desktop/mobile API
  SSE read paths

Agent/runtime data plane
  provider runs
  MCP calls
  transcript ingestion
  render/projection jobs
  workflow child runs
```

The first step does not need to be a separate process. It can start as separate
queues, semaphores, and cached read models within the same process. If pressure
continues, move provider/runtime execution into a worker process and let the
gateway only submit work and read committed state.

## Immediate Local State

- `development-loop` workflow package has been repaired locally.
- The gateway still deserves a retry/priority/observability pass; the workflow
  fix removes repeated warning noise but does not address the larger runtime
  contention issue.
