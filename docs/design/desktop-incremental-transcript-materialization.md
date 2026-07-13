# Desktop Incremental Transcript Materialization (#TASK-2208)

Status: design approved (`PASS / SHIP-DESIGN`, #TASK-2221); implemented and
validated in the task worktree.

Scope: `desktop/garyx-desktop` only.

## Problem and locked baseline

Each accepted committed event currently rebuilds the whole UI transcript:

1. `ThreadTranscriptCache.applyCommittedMessage` forward-merges one event into
   a full `ThreadTranscript`.
2. `applyRemote` calls `mergeRemoteTranscriptWithLocal` for the whole visible
   history.
3. `materializeRemoteTranscript` uses `existing.findIndex` for every remote
   message, making a full apply quadratic.
4. Stable matches stringify both `content` and `metadata`, even though the
   forward fold normally preserves those field references.
5. The lifecycle computes `transcriptForCommittedCache` and sends the full
   snapshot to the main process after every event. The main process then
   stringifies and atomically rewrites the full cache file.

The deterministic benchmark is
`desktop/garyx-desktop/scripts/benchmark-transcript-materialization.mjs`. It
uses the production wire identity rule (`message.seq === event.seq`, message id
suffix `seq - 1`), 1,200 seeded messages, 120 sequential committed events, and
a 2,048-byte content payload. The timer covers only the committed-event fold;
the seed apply is warm-up state.

Locked pre-change evidence at `e734a243c96c3c5362494200a78826a9412fd380`
on Node v26.3.0, macOS 26.3.1, arm64 (Apple M4 Max):

```text
command: node --experimental-strip-types \
  scripts/benchmark-transcript-materialization.mjs --samples=9
median: 673.24 ms
samples: 673.24, 660.76, 657.38, 601.85, 628.31,
         924.70, 854.43, 701.81, 735.30 ms
JSON.stringify calls/sample: 604,560
JSON.stringify bytes/sample: 640,302,240
final UI messages: 1,320
```

The exact stringify counts/bytes are deterministic. Timing is reported as a
median because concurrent development work can perturb wall time.

## Contracts that must not change

- `render_state` remains the only source of rendered rows, user-turn grouping,
  tool pairing/grouping, final-answer placement, filtered placeholders, and
  tail activity. This work only maintains message bodies addressed by server
  `seq` references and local optimistic overlays.
- For all inputs, materialized transcript structure must equal the old
  implementation. Stable, unaffected remote messages must also retain their
  object references so downstream memoization continues to work.
- The first-unused existing entry wins when duplicate ids exist.
- Generated-image synthetic rows retain their current matching priority:
  synthetic id, then generated-image tool-use id, then content equivalence.
- Rewrites/resets still trigger an authoritative refetch. The optimization
  does not interpret rewrite ranges locally.
- Pagination, optimistic/error/interrupted rows, and the
  `threadRunActive` rule remain unchanged. The iOS `GaryxTranscriptMerge`
  contract therefore does not change.
- The disk snapshot remains a reconstructable cache. Gateway transcript state
  is authoritative.

## Design overview

The change has three independent layers. Every optimized layer has a
reference-equivalent fallback.

### 1. Make full materialization linear and allocation-light

`materializeRemoteTranscript` will build indexes once per full apply:

- `id -> ordered existing indexes`;
- generated-image tool-use id -> ordered existing indexes;
- the existing generated-image content fallback candidate list.

Each index value is an array plus a cursor, not `shift()`, so consuming many
duplicate ids remains linear. Taking an entry skips indexes already consumed
through another generated-image fallback. This is exactly equivalent to the
current `findIndex(... !used && id === candidate)` rule: both choose the first
unused match in existing-array order.

`remoteTranscriptMessageCanReuseExisting` keeps its current field set. It does
not opportunistically add comparisons for `input`, `result`, or other fields,
because that would change legacy reuse behavior.

The two `JSON.stringify` comparisons are replaced by a JSON-safe structural
comparator with these properties:

- reference and primitive equality return immediately;
- arrays compare length and entries in order;
- objects compare the same enumerable JSON keys in the same serialization
  order, then recurse;
- JSON array nullification and object omission rules for `undefined`,
  functions, symbols, holes, and non-finite numbers are covered by focused
  tests;
- unsupported non-wire values (cycles, `BigInt`, exotic `toJSON`) return
  "not reusable" rather than risking reuse of stale content.

Returning "not reusable" is always structurally safe: the remote entry is
materialized afresh. Production transcript values are parsed JSON, so the
comparator covers the reachable wire domain. It performs no full-string
allocation. On the committed path, stable `content`/`metadata` references take
the constant-time branch.

The generated-image content fallback uses the same comparator, removing its
last content `JSON.stringify` as well.

This layer is unconditional and improves authoritative, forward-fetch,
pagination, and committed fallback applies from quadratic matching to linear
matching.

### 2. Incrementally fold ordinary committed tails

`ThreadTranscriptCache` gains a conservative committed fast path. Its minimal
sidecar contains only:

- whether the current raw snapshot is strictly ordered by physical history
  index and its maximum index;
- the length of the `remote_final` UI prefix, or invalid if remote/local rows
  are interleaved;
- ids in that remote prefix, for collision detection.

The sidecar is derived indexing, never a second source of truth. Full applies
rebuild it. `setUiMessages` and other non-fast writes invalidate it unless the
remote prefix is demonstrably unchanged. An invalid sidecar causes one full
fallback, which rebuilds it.

#### Fast-path qualification

An event is handled incrementally only when all of the following are proven:

- it is not a rewrite/reset;
- the cached snapshot belongs to the same thread;
- every cached raw message has a numeric, strictly increasing physical history
  index;
- the new message carries the accepted event seq, its history index is after
  the cached tail, and the forward relation is append-only;
- the UI array still has a `remote_final` prefix followed by a local suffix;
- the normalized new id does not collide with the existing remote prefix;
- it is not a legacy loading placeholder;
- it is not a generated-image tool result requiring a synthetic row;
- it is not a control message.

Controls, loading placeholders, generated images, duplicate logical ids,
overlaps, out-of-order events, malformed ids, and any uncertain state use the
full linear fallback. Controls are deliberately conservative because terminal
controls can change `activeRun` and remove unmatched local tool rows. Generated
images retain their three-stage matching in one place. These events are sparse
relative to normal assistant/tool text bodies.

Seq redelivery remains intercepted by `recordsBySeq` before this method, so it
is still an exact no-op.

#### Raw snapshot append

For a qualified event, the snapshot uses an append helper rather than
`normalizeCommittedTranscriptMessages` plus sort. The helper:

- copies the existing message array once and appends the new message;
- derives page information through the existing forward-page merge rules,
  rather than reimplementing page arithmetic;
- preserves pending inputs, thread data, and thread info exactly as the
  existing committed forward page does;
- resolves active-run state against the already-resolved base plus only the
  new event. A non-control qualified event cannot change that fixed point.

If any append precondition fails, the existing
`committedMessageForwardPage` remains the fallback.

#### UI append and local reconciliation

The previous UI result is partitioned as:

```text
[ stable remote_final prefix | local overlay suffix ]
```

Only the new message is materialized, using the local suffix as the match pool.
This covers the important optimistic-user transition: a committed user body
normalizes to the optimistic `origin:<intent>` id, carries forward
`intentId`/`remoteRunId`, changes to `remote_final`, and moves to the remote
boundary.

Local preservation is extracted from the current
`mergeRemoteTranscriptWithLocal` into one shared helper, so both full and
incremental paths execute the same rules for:

- duplicate local ids;
- local user ids and origin ids;
- assistant intent/history matches;
- tool equivalence while a run is active;
- error/interrupted rows;
- the inactive-run rule that drops unmatched local tool rows.

The incremental path passes the stable remote prefix plus the new remote entry
to that helper. It may scan the small local suffix and, while an intent is
pending, inspect visible history. It does not rematerialize or compare stable
remote bodies. This intentionally avoids a larger intent/tool sidecar whose
invalidation surface would be harder to prove.

The result is:

```text
[ same remote object references..., new/settled remote body,
  preserved local object references... ]
```

If no UI body or local-row change occurs, the existing array reference is
retained. Mirror commit/notification cadence is unchanged. The current
per-event loop in `mirror.ts` is retained to avoid colliding with concurrent
snapshot/subscription work; frame-level batch folding is a possible follow-up,
not required to eliminate full body rematerialization.

#### History and rewrite behavior

- Older history continues through `applyOlderPage`; it prepends stable remote
  entries and invalidates/rebuilds the sidecar.
- A forward HTTP aggregate or authoritative transcript always uses the full
  linear path, then establishes a new fast-path baseline.
- Rewrites, resets, shrink refetches, gaps, and ambiguous overlaps never use
  local patch logic.
- Windowed replay can resume the fast path after a full baseline is accepted;
  a numeric gap itself is allowed only if the existing forward semantics prove
  it is a tail append.

### 3. Coalesce persistence before IPC and stringify

The write-admission scheduler belongs in the renderer transcript lifecycle,
not only in the main process. Main-only debouncing would still pay the
per-event `transcriptForCommittedCache` scan and multi-megabyte structured
clone over IPC.

A small, clock-injectable `TranscriptPersistScheduler` keeps one dirty marker
per thread:

- trailing delay: 1 second;
- maximum wait under continuous traffic: 5 seconds;
- `schedule(threadId)` stores no serialized payload;
- `flush(threadId)` clears timers and asks the lifecycle to read the latest
  mirror transcript/render state;
- `cancel(threadId)` discards a pending stale write;
- `flushAll()` is available for deterministic teardown.

At flush time, and only then, the lifecycle runs the unchanged
`transcriptForCommittedCache` and existing persistence gate, then invokes
`saveThreadTranscriptCache`. The main process keeps its current per-thread
generation queue, temporary-file write, and rename as a second ordering and
atomicity layer.

Persistence modes are explicit:

- committed event: schedule/coalesce;
- terminal control (`run_complete`, `run_interrupted`,
  `interrupt_confirmed`): stage the final snapshot, then force flush;
- terminal stream error: force flush the latest accepted snapshot;
- authoritative/full/forward HTTP apply: immediate persistence;
- selected-thread cancellation/switch and thread-specific stream stop: force
  flush that thread;
- rewrite/refetch, missing-thread rollback, cache clear/delete: cancel the
  pending stale write before the clear; the successful authoritative result is
  persisted immediately;
- test/teardown: flush all.

The scheduler does not reset the maximum-wait deadline when more events
arrive, so a long run persists at least every five seconds. Within the window,
only the latest dirty snapshot exists. A crash can therefore lose at most that
one coalesced, not-yet-flushed cache snapshot (bounded to five seconds of
traffic), never authoritative transcript data. The gateway remains the source
of truth, and existing cache-cursor forward fetch plus stream replay repairs
any staleness on reopen. Atomic rename means a crash leaves either the prior
complete cache or the new complete cache.

No cache delta protocol is introduced in this task.

## Equivalence oracle

The implementation is not accepted on ordinary unit coverage alone.

### Frozen legacy reference

A test-only module freezes the pre-change materializer and committed full-fold
composition. It must not call the new matching, comparator, or incremental
helpers. Tests drive legacy and new folds with identical inputs and assert
`deepStrictEqual` after every prefix.

Data sources:

- every populated record sequence in
  `test-fixtures/render-layer/render-state-cases.json`, mapped into the real
  desktop wire shape (1-based seq, 0-based id suffix, control envelope);
- all captured `test-fixtures/stream-sync/*.jsonl` sources, including fixture
  references used by cases whose inline `records` are empty;
- deterministic constructed cases for duplicate ids, optimistic echo,
  assistant intent settlement, active/inactive local tools, control-only
  frames, loading placeholders, generated-image saved-path/base64 forms,
  older-page prepend, windowed/gapped tails, and rewrite/refetch.

For each prefix, the oracle compares:

- snapshot transcript;
- UI messages;
- pagination;
- thread info;
- pending inputs;
- apply outcome (`applied` or `refetch_authoritative`).

The mirror contract suite additionally proves that server render state is
accepted unchanged and that frontier/notification semantics did not move.

### Reference identity assertions

Separate assertions cover behavior that deep equality cannot prove:

- after an ordinary append, every one of 1,200 seeded remote entries is the
  exact same object;
- an optimistic echo replaces only its affected local entry;
- preserved local/error/interrupted rows keep their references;
- idempotent redelivery keeps the UI array reference;
- control/loading events with no effective UI change keep the array reference;
- full, structurally equal rematerialization reuses stable entries in the
  reachable JSON-safe domain.

### Comparator and index assertions

- A table-driven test compares the new JSON-safe comparator with the old
  stringify predicate over supported values, including omission/nullification
  cases and object-key order.
- Duplicate-id and generated-image candidates prove FIFO first-unused matching.
- Instrumented steady-state committed tests assert zero materialization
  `JSON.stringify` calls and bytes.

### Persistence assertions

Fake-clock tests prove:

- a burst causes zero immediate writes and one latest-state write at the
  trailing deadline;
- continuous traffic is forced by max-wait;
- terminal and thread-switch boundaries flush immediately;
- two threads schedule independently;
- cancel prevents a stale timer from reviving a cleared cache;
- a successful authoritative apply supersedes pending committed state;
- `flushAll` drains each dirty thread once.

Lifecycle tests assert cancel-before-clear ordering. Main-process generation
tests continue to cover latest-write ordering and atomic write behavior.

## Benchmark acceptance

The post-change benchmark must use the same script, config, runtime, and warmup
method as the locked baseline. The report records all samples and median.

Acceptance requires:

- final UI message count remains 1,320;
- legacy/new output is deep-equal;
- 1,200 stable seed references remain identical;
- steady-state materialization stringify calls and bytes are zero;
- median committed-fold time improves by at least one order of magnitude.

Post-change evidence on the same Node v26.3.0 / darwin-arm64 runtime and the
same nine-sample command:

```text
median: 1.71 ms (393.7x faster than the 673.24 ms baseline)
samples: 2.28, 1.71, 1.82, 1.63, 2.00, 2.34, 1.71, 1.62, 1.46 ms
JSON.stringify calls/sample: 0
JSON.stringify bytes/sample: 0
final UI messages: 1,320
stable seed references: 1,200 / 1,200
committed applies: 120 incremental / 0 full fallback
```

The frozen pre-change fold and optimized cache are also deep-equal after every
one of the 120 prefixes. The same oracle drives every captured
`stream-sync/*.jsonl` stream and every inline or referenced
`render-state-cases.json` sequence.

## Expected files

Primary implementation surface:

- `src/renderer/src/gateway-mirror/transcript-materialize.ts`
- `src/renderer/src/gateway-mirror/transcript-cache.ts`
- `src/renderer/src/gateway-mirror/transcript-lifecycle.ts`
- one focused pure incremental helper module if needed
- one focused persistence scheduler module

Minimal wiring may touch `mirror.ts` for persistence flush/cancel facades, but
must not alter snapshot derivation, subscription granularity, or render-state
acceptance. App-shell wiring is avoided unless teardown proves it is required.
The main-process transcript writer remains the atomic sink; write cadence is
reduced before it is invoked.

Tests/evidence:

- frozen legacy/oracle tests under `gateway-mirror/`;
- persistence scheduler/lifecycle tests;
- the benchmark script and this document's post-change result section;
- unchanged `mirror-contract.test.mjs` as a mandatory regression suite.

## Tradeoffs and rejected alternatives

- A conservative fallback means sparse control/image events still do one full
  linear pass. This is preferable to duplicating subtle synthetic/local rules.
- Local intent matching may remain O(history length) while an intent overlay is
  present. The overlay is small and short-lived; persistent intent/tool indexes
  would add more invalidation risk than this task justifies.
- The UI array still copies references when a visible append occurs. A
  rope/chunked collection would complicate consumers for little benefit.
- Main-only debounce was rejected because it leaves renderer scan and IPC clone
  amplification intact.
- Client-side render-state delta interpretation was rejected because it would
  violate the server-owned rendering contract.
- Frame-level committed batching is deferred to avoid changing the concurrent
  `mirror.ts` snapshot/subscription area. The per-event incremental fold already
  removes whole-transcript body work.

## iOS assessment

The intended output is structurally identical to the frozen desktop legacy
implementation. `mergeRemoteTranscriptWithLocal`, including its documented
alignment with `GaryxTranscriptMerge.threadRunActive`, keeps the same semantics.
No iOS change is required.

If implementation or oracle findings require changing duplicate-id handling,
generated-image placement, local-tool retention, or control timing, that is a
semantic change outside this task. The design must return to review and the iOS
side must be evaluated explicitly before proceeding.

## Validation and landing

Before handoff:

1. focused legacy/new oracle and persistence tests;
2. `npm run test:unit -- src/renderer/src/gateway-mirror/mirror-contract.test.mjs`;
3. `npm run test:unit`;
4. `npx tsc --noEmit`;
5. locked before/after benchmark with results added to this document;
6. fetch latest `origin/main`, rebase, reconcile overlapping upstream semantics,
   rerun focused/full validation, commit, merge to `main`, and push
   `origin/main`.

No desktop UI, gateway runtime, iOS, capsule, artifact, screenshot, or release
work is part of this task.
