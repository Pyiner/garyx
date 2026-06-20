# TASK-971 Bound Delivery Realtime Replay

## Reproduction

Focused regression:

```text
cargo test -p garyx-channels committed_replay::tests::spawn_backfills_initial_lag_before_matching_row_reaches_terminal -- --exact --nocapture
```

Current failure:

```text
left: []
right: ["first ", "second "]
```

The test commits target-run rows into the durable tail reader, then forces a
global broadcast lag before the replay task has observed any matching row. The
next retained bus line is for another run. Current replay records the lag, but
`backfill` is a no-op because `CommittedReplayState.thread_id` is still `None`.
Target rows remain durable-only until a later target-row or terminal record lets
the state learn the thread id.

The reproduction uses a trivial Vec-collecting consumer, so it proves hypothesis
A is sufficient without any Telegram HTTP, throttle timer, or channel-worker
backpressure in the loop.

## Root Cause

This is hypothesis A, narrowed to the initial-lag/unknown-thread branch.
`committed_replay` uses a contiguous frontier and correctly avoids forwarding
out-of-order rows. However, it only learns `thread_id` from a matching bus line.
For bound delivery from `garyx-gateway/src/application/chat/delivery.rs`, the
caller already knows the canonical `thread_id`, but `committed_callback` accepts
only `run_id`. A dropped target-run prefix can therefore make the first lag
backfill unable to query the durable transcript.

Hypothesis B is not supported by the code path: Telegram `ScheduleFlush` spawns a
Tokio sleep in `garyx-channels/src/telegram/streaming.rs`, and the committed
replay consumer calls the Telegram callback, which only enqueues into an
unbounded worker channel. Telegram HTTP sends do not synchronously block the
committed replay receiver.

Hypothesis C is not the primary root cause for this reproduction: bound delivery
fanout is synchronous only over callback invocation; the Telegram callback itself
queues work into its streaming worker.

## Fix Plan

Add a replay constructor/callback variant that accepts a known initial thread id.
Use it from gateway bound delivery, where `thread_id` is already available before
dispatch. Keep existing committed replay entry points for inbound channel runs
that do not know the canonical thread until routing resolves.

With the thread id seeded, a broadcast `Lagged` before the first matching row can
immediately read `records_after_seq(thread_id, 0)` or the run-scoped fallback and
forward committed rows in order during the run. The contiguous frontier still
guards ordering: live rows ahead of a hole are not forwarded until durable
backfill fills the prefix. Duplicate and same-seq overwrite behavior remains
unchanged.

## Validation

- New regression must pass and assert target deltas are forwarded before the
  terminal record is sent.
- Seeded replay should still accept same-thread runless controls and reject
  runless controls from other threads.
- Existing committed replay tests must remain green, especially middle-gap,
  initial-lag, terminal, same-seq overwrite, and replay subscription tests.
- Run `cargo test -p garyx-channels --all-targets`.
- Run `cargo test -p garyx-gateway --all-targets`.
