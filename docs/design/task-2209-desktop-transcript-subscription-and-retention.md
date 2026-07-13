# TASK-2209 Desktop Transcript Subscription And Retention

Status: v2 proposed for re-review

## Goal

Bound desktop transcript work to the thread that changed:

- a committed frame for an unrelated background thread must not render
  `AppShell` or its active `ThreadPage`;
- an active stream frame must render only transcript rows whose presentation
  changed;
- shell derivations must not allocate again when their inputs are unchanged;
- recoverable authoritative inactive mirror transcript entries must be bounded
  and recover correctly when reopened, while unrecoverable local-only state is
  retained until finalized or cleared.

The deterministic pre-change evidence and oracle protocol are recorded in
`docs/design/task-2209-transcript-render-baseline.md`.

## Constraints And Non-goals

- Transcript structure remains server-owned. The desktop continues to map
  `render_state.rows`, `tailActivity`, and `activeToolGroupId`; it does not add
  grouping, tool-pairing, final-answer, or tail-thinking heuristics.
- `useSyncExternalStore` snapshots must remain referentially stable until the
  subscribed thread changes.
- No windowing/virtualization is introduced in this task.
- No gateway protocol or persisted-cache format changes are required.
- This task does not edit `gateway-mirror/transcript-materialize.ts`,
  renderer `gateway-mirror/transcript-cache.ts`, or `message-machine.ts`, which
  are owned by parallel work.

## Baseline

The dev oracle drives the real `GatewayMirror`, `AppShell`, and `ThreadPage`
under React StrictMode. Two consecutive 12-frame / 40-row runs were identical:

- background frames: 24 `AppShell` and 24 `ThreadPage` function renders;
- active frames changing only the tail row: 960 row renders;
- 936 of those row renders belonged to the 39 unchanged history rows.

StrictMode accounts for the stable 2x multiplier. Normalized, every background
frame rendered the whole shell once and every active frame rendered all 40
rows once.

## Design

### 1. Per-thread React subscriptions

`GatewayMirror` already has the correct atomic domain:
`subscribeThread(threadId)` plus cached `getThreadSnapshot(threadId)`. The
current regression comes from bypassing it in the shell and side-chat panel in
favor of the legacy five-map aggregate.

Add a thin binding in `gateway-mirror/react.ts` which accepts an explicit
mirror instance and nullable thread id. `AppShell` owns the context provider,
so it cannot consume its own context; both that binding and the existing
context-backed `useThreadMirror` will share the same implementation.

For a non-null id:

```ts
subscribe = callback => mirror.subscribeThread(threadId, callback)
getSnapshot = () => mirror.getThreadSnapshot(threadId)
```

For a null id both functions return stable no-op/null values. The mirror's
existing per-entry `snapshot` cache preserves the `useSyncExternalStore`
reference contract. Contract tests will assert that a background commit keeps
the active snapshot reference unchanged and sends no active notification.

`AppShell` will subscribe to exactly two transcript entries:

1. `activeThreadMessageKey` (selected thread or the new-thread draft); and
2. the bound side-chat thread, whose always-on queue/stream orchestration is
   intentionally shell-owned even while its dock is hidden.

All active transcript values (`messages`, `renderState`, `threadInfo`, loaded
gate, pagination, pending remote inputs, and live stream) come from the first
snapshot. The always-on side-chat derivations come from the second. The
aggregate transcript-map and aggregate live-stream React subscriptions are
removed from `AppShell`.

`SideChatPanel` independently consumes `useThreadMirror(sideChatThreadId)`.
Its panel-local message/render/info/pagination/pending/live values therefore
change only for that side thread. Two subscribers to the same side thread are
valid and are also the reference count that protects the entry from eviction.

Single-thread imperative reads in `AppShell`, `DispatchOrchestrator`, and
`TranscriptLifecycle` will use `getThreadSnapshot(threadId)` or
`getThreadLiveStream(threadId)` instead of rebuilding an aggregate map merely
to select one key. The aggregate map API remains temporarily for its
legacy-shaped multi-key updater and compatibility contract tests; it is no
longer a render subscription or a hot single-thread selector.

### 2. Memoized transcript row boundary

Move the inline row body renderer out of `turnRows.map` into a named
`memo(...)` component keyed by `row.key`. It receives one server-mapped
`TurnRenderRow`, the active tool-group id, and a stable action bridge. The
action bridge reads the latest callbacks through a ref so changing shell
closure identities do not invalidate every historical row. Translation
identity remains an explicit prop so a locale change still rerenders text.

The memo comparator is presentation-conservative:

- row kind and `row.key` must match;
- message blocks compare their cached message object references;
- turn running/timestamp/final-block values and activity block sequences are
  compared;
- tool entries compare their resolved message references and compare every
  lightweight projection selector by value, including selector path elements;
  an equal projection received as a fresh wire object does not rerender, while
  any changed projection field does;
- Capsule cards compare their server fields by value, so equal fresh wire
  objects are stable and a changed card rerenders;
- an active-tool-group-id change rerenders only a row containing either the
  previous or next active group id. Rows with no affected group remain equal,
  so active shimmer cannot go stale without invalidating the whole list.

This does not infer any transcript semantics. It only decides whether the
already mapped presentation inputs are equal. The parent still performs the
small keyed list reconciliation; expensive row React subtrees, Markdown, tool
groups, and cards are not rebuilt for unchanged keys.

`messagesBySeq` and `turnRows` remain `useMemo` derivations keyed by their real
inputs. Per-thread subscription isolation means a background commit no longer
enters `ThreadPage`, so those derivations do not run. During an active frame
they may still map the server snapshot in O(N); true windowing and deeper
incremental view-model materialization remain a later measurement-driven
phase.

### 3. Shell derivation memoization

Memoize the currently unstable active-thread projections:

- queue intents;
- pending-ack intents;
- pending-history intent presence;
- active thread endpoints and bound bots;
- mapped bot id and final active bot.

For pending-ack visibility, build one memoized `Set<string>` from active user
messages containing both explicit `message.intentId` and the normalized
server origin id. Filtering pending acknowledgements then becomes O(messages +
pendingAck), replacing `pendingAck × activeMessages` scans. The same derivation
shape will be used in `SideChatPanel` where practical, without changing its
message/intent matching contract.

### 4. Bounded mirror entry retention

Keep all referenced or live entries, plus an LRU of at most 32 inactive
entries. The limit applies to inactive entries rather than total entries so
concurrent selected/side/live threads are never evicted merely to hit a hard
global number.

Each `ThreadEntry` gains:

- a monotonically increasing `lastAccess` ordinal;
- its `threadId` for deletion/diagnostics;
- an operation retain count for code that holds the mutable entry across an
  `await`.

An entry is evictable only when all are true:

- `listeners.size === 0` (not selected and no mounted subscriber);
- retain count is zero (no in-flight entry-scoped operation);
- `liveStream === null` (all non-null transport/recovery states are protected);
- no UI message has a local state other than `remote_final`, because queued,
  failed, and other local-only rows are not recoverable from authoritative
  history; and
- the id is not `NEW_THREAD_DRAFT_THREAD_ID`, which has no gateway recovery
  route. The sentinel moves to a small shared thread-id module so the mirror
  and dispatch controller use one definition without a UI-layer import.

Reads, subscriptions, and commits touch recency. Snapshot getters are
side-effect-free with respect to the entry table, snapshot invalidation, and
notifications: an absent id returns a stable empty snapshot cached by thread
id (so its required `threadId` field stays correct), and a read may only update
an existing entry's unobservable recency ordinal. Pruning never runs from a
getter or render-time snapshot read. It runs only after mutation
paths: subscription creation, commit, unsubscribe, operation release, and
live-stream clearing. When more than 32 entries are eligible, it deletes
oldest eligible entries until 32 remain. A newly accessed entry is protected
as the most recent during the pruning pass.

Eviction deletes the whole heavy entry: committed records, UI messages,
transport snapshot, frontier, and cached snapshot. This is safe only because
there are no per-thread subscribers or live stream. It also invalidates the
legacy aggregate snapshot; any remaining aggregate compatibility subscriber is
notified once after the pruning batch.

The selected-thread recovery path remains authoritative:

1. subscribing recreates a stable empty entry;
2. `loadSelectedThreadTranscript` restores the disk cache when present;
3. the authoritative/incremental fetch reconciles it;
4. the committed stream restarts from the recovered cursor/render floor.

A contract test will fill beyond the inactive LRU, prove the oldest heavy
entry was released, reopen it through the real cache/fetch/stream lifecycle,
and assert restored messages/render state plus the expected stream start.
Separate cases prove that the draft, a failed local message, a listener, a
live stream, and an async retain are never evicted.

## Notification And Concurrency Rules

- A `thread_render_frame` remains one synchronous per-thread commit.
- Entry snapshot invalidation happens before that thread's listeners run.
- No listener-bearing entry can be removed, so an active
  `useSyncExternalStore` subscription never observes a version reset.
- Pruning batches aggregate invalidation/notification; it does not send a
  notification for every deleted key.
- The one mirror method that retains a raw entry across an async older-page
  fetch holds an operation retain until its final commit completes.
- Dispatch error recovery currently clears live state immediately before
  replacing the optimistic message. The 32-entry recency floor makes eviction
  unreachable in that synchronous short window; any future long async window
  must acquire an operation retain, while a full authoritative apply already
  self-heals an absent entry. A focused regression test exercises the current
  clear-then-update sequence.
- The bound applies to authoritative heavy `GatewayMirror` `ThreadEntry`
  caches. The small lifecycle registries outside the mirror are not claimed to
  be covered by this LRU and can be assessed independently if they become
  material.

## Files And Impact

Expected implementation surface:

- `app-shell/AppShell.tsx`: per-thread reads and requested memos;
- `app-shell/components/ThreadPage.tsx`: memo row boundary;
- `app-shell/components/SideChatPanel.tsx`: side-thread snapshot;
- `gateway-mirror/react.ts`: explicit-instance per-thread binding;
- `gateway-mirror/mirror.ts`: LRU/reference retention;
- `gateway-mirror/dispatch-orchestrator.ts` and
  `gateway-mirror/transcript-lifecycle.ts`: direct single-thread reads;
- focused contract/performance tests, the dev probe/oracle, and baseline data.

No server, main-process cache, transcript reducer, or persisted schema changes
are expected.

## Validation

Focused gates:

1. `npm run transcript-render:oracle -- --expect optimized --frames 12 --rows 40`
   must report zero background `AppShell` renders and zero unchanged-row
   renders. The fixture mixes ordinary and tool-bearing history rows, rebuilds
   each wire render snapshot with fresh projection objects, and records a row
   only from inside the memoized row component body.
2. Mirror contract tests cover snapshot stability, notification isolation,
   eviction protection, LRU order, and reopen recovery.
3. Row comparator tests cover unchanged message rows, changed tail content,
   running/tool state, locale identity, equal-value/different-reference tool
   projections and Capsule cards, changed selector fields, and active-group
   changes that do and do not intersect a row.
4. Source/owner contract verifies `AppShell` and `SideChatPanel` no longer
   subscribe to aggregate transcript maps.
5. A view-model/cache contract asserts that appending a tail frame preserves
   unchanged message object references; this pins the memo comparator's main
   input invariant without editing the parallel-owned materializer/cache.
6. Full required gates: `npm run test:unit` and `npx tsc --noEmit`.

The post-change raw oracle result will be appended to the baseline document
before code review.

## Tradeoffs

- Parent row-list reconciliation and server view-model mapping remain O(N) on
  active frames. This task removes whole-shell fan-out and heavy historical
  React subtree work; it deliberately does not claim windowing.
- The 32-entry inactive LRU places a deterministic bound on recoverable,
  authoritative heavy transcript entries. Draft and local-only state is kept
  outside that bound because dropping it would be data loss; those protected
  entries become evictable once finalized or explicitly cleared. Older
  authoritative revisits may show the existing cache/history loading state
  before restoration completes.
- Projection and Capsule comparison is field-wise over small server-owned
  selector/card records. This bounded comparison avoids historical tool-row
  subtree work when wire deserialization creates fresh but equal objects.
