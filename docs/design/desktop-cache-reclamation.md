# Desktop cache reclamation

Status: proposed for #TASK-2210

Scope: `desktop/garyx-desktop` only. This change bounds renderer intent and
Capsule memory, bounds the main-process transcript disk cache, and makes the
new-thread Git-status cache expire. It does not change server `render_state`
derivation or transcript row structure.

## Reproduce-first baseline

Baseline source revision: `e734a243c`.

All reproductions run through the repository unit-test entry point. The tests
are checked in with the design so the same assertions become post-fix guards.

| Surface | Deterministic setup | Baseline result |
| --- | --- | --- |
| Message intents | Create 64 intents, each with a 16 KiB base64 image payload; move each to `completed`; dispatch `thread/clear` | FAIL: `intentsById` remains 64, expected 0 |
| Main transcript disk cache | Save 241 distinct minimal thread transcripts into an isolated `userData` directory | FAIL: 241 JSON records remain, expected at most 240 |
| Renderer Capsule HTML | Resolve 257 distinct Capsule HTML requests | FAIL: the oldest entry is still `ready`, expected `idle` after eviction |
| Renderer Capsule thumbnail | Resolve 257 distinct thumbnail data URLs | FAIL: the oldest entry is still `ready`, expected `idle` after eviction |

Focused commands and observed assertions:

```text
npm run test:unit -- src/renderer/src/message-machine.test.mjs \
  --test-name-pattern "thread clear releases completed attachment intents"
64 !== 0

npm run test:unit -- src/main/transcript-cache.test.mjs
expected at most 240 cache records, found 241

npm run test:unit -- src/renderer/src/app-shell/capsule-html-store.test.mjs \
  --test-name-pattern "evicts old ready HTML"
'ready' !== 'idle'

npm run test:unit -- src/renderer/src/app-shell/capsule-thumbnail-store.test.mjs \
  --test-name-pattern "evicts old ready data URLs"
'ready' !== 'idle'
```

The Git-status stale-negative path is also direct in the current code:
`NewThreadEmptyState.tsx` stores `DesktopWorkspaceGitStatus` in a module-level
`Map` and returns every hit without an age check or focus invalidation. The
implementation will add a headless `git init` regression test around the same
cache/load helper instead of relying on a UI observation.

## 1. Terminal intent collection without breaking reconciliation

### Why collection cannot happen at `intent/completed`

An intent remains a temporary reconciliation record until the committed
history apply has consumed it:

1. `seededUserBubble` creates the optimistic user message with both
   `id = origin:<intentId>` and `intentId`.
2. A committed user message carries `metadata.origin_id`. Transcript
   materialization normalizes that body to the same `origin:<intentId>` id.
3. `mergeRemoteTranscriptWithLocal` calls `intentForId` and
   `resolveIntentHistoryMatch` before deciding whether the optimistic user,
   assistant, or tool rows can be removed.
4. `buildThreadViewRowsWithLocalUsers` appends only still-unrepresented local
   user rows beside the server-owned `render_state.rows` projection.
5. For queued inputs, `pendingInputOriginRefsForThread` maps
   `pendingInputId -> intentId`; `visibleRemotePendingInputsForThread` compares
   that origin with the committed user message to avoid displaying a duplicate
   remote pending input.

Deleting at the terminal state transition would therefore race steps 2-5.
Collection happens only on a thread release, after the current transcript
apply/match pass.

### Retention set

The production `GatewayMirror` will enrich a `thread/clear` dispatch with a
thread-local retention set computed from its existing snapshot:

- retain an intent referenced by a non-`remote_final` local UI message (the
  optimistic/error/interrupted row still needs it);
- retain an intent whose `pendingInputId` is still present in the thread's
  remote `awaiting_ack` inputs, preserving the origin-deduplication chain;
- non-terminal intents are never collection candidates, irrespective of the
  retention set.

The reducer then deletes only this thread's unretained terminal states:
`completed`, `cancelled`, `failed`, and `interrupted`. It also removes the
thread's queue property when the queue is empty, but preserves a non-empty
queue and every other thread.

`markIntentsFromHistory` will request a release whenever the thread has no
pending-history intent, even if its runtime was cleared earlier. That gives a
later authoritative apply a chance to collect an intent retained during an
earlier failure/interruption race.

### Shared conversation-state compatibility

`thread/clear` is shared with the iOS state-machine fixture and currently means
"remove runtime". To preserve that contract, the desktop action gains an
optional `retainedIntentIds` field:

- omitted: current runtime-only behavior, so existing shared fixtures and iOS
  semantics are unchanged;
- present: runtime clear plus desktop terminal-intent GC and empty-queue
  cleanup.

Every production desktop clear goes through `GatewayMirror`, which supplies
the field. Focused reducer tests can pass an explicit empty or non-empty set.
`intent/cancelled` on an unknown record keeps its existing unconditional queue
removal semantics, and `thread/delete` remains the unconditional full purge.

This design does not derive, group, pair, or reorder transcript rows. Server
`render_state` remains the only semantic render source.

## 2. Bounded main-process transcript disk cache

Refactor `src/main/transcript-cache.ts` into a small directory-backed store
while retaining the three current IPC functions. The default limits mirror
the established Capsule disk-store policy:

- maximum 240 JSON records;
- maximum 48 MiB total bytes.

The payload's existing `savedAt` is its initial recency. A successful load
touches the file modification time, making `mtimeMs` the access timestamp
without rewriting a potentially large transcript body. Pruning sorts by
`mtimeMs` (then filename for deterministic ties) and removes oldest files
until both limits hold.

All directory operations share one serialized chain. Per-thread generations
still coalesce superseded saves/clears, while the directory chain prevents
concurrent writes from each observing an under-limit partial directory and
leaving 241 records behind. Reads/touches are serialized with pruning so a
load cannot race eviction of the same file.

Pruning runs:

- once, best-effort, from the main-process `app.whenReady()` bootstrap;
- after every successful atomic save.

Only final `*.json` records participate; temporary files are ignored. A cache
failure remains non-fatal and loads continue returning `null` on invalid or
missing records.

Tests instantiate the same store against temporary directories with small
limits to prove record eviction, byte eviction, load-touch LRU order, startup
pruning, and save/clear ordering. The existing 241-record baseline remains an
integration guard over the public functions.

## 3. Expiring workspace Git status

Move the module singleton into a React-free helper with:

- 30-second TTL for every result;
- 64-entry LRU count bound;
- explicit negative-entry invalidation.

`NewThreadEmptyState` uses the helper for the existing IPC load. When the app
window regains focus, it invalidates a cached `isGitRepo: false` for the
selected workspace and reruns the check. This makes the common flow—leave the
app, run `git init`, return—refresh immediately, while positive repositories do
not incur a focus-time IPC on every switch.

A headless test creates a temporary non-repository directory, caches the
negative result through the helper, runs `git init`, simulates focus
invalidation, and verifies the next helper load returns `isGitRepo: true`. A
separate fake-clock assertion covers TTL expiry and the LRU count bound.

## 4. Bounded renderer Capsule stores

Both singleton stores become access-ordered maps:

- HTML store: at most 32 terminal entries;
- thumbnail store: at most 64 terminal entries.

`getState` and cache-hit `request` touch recency. `setEntry` inserts as newest
and prunes the oldest terminal entries. Loading/in-flight/queued keys are not
evicted; they become eligible when they settle, so concurrency and generation
guards remain intact. The per-id generation entry is dropped once that id has
no queued or in-flight job, preventing invalidation metadata from becoming a
second unbounded map.

Eviction returns the stable `idle` snapshot. The two Capsule hooks request
again when an active key becomes `idle`; the renderer then normally hits the
already-bounded main-process thumbnail disk LRU. Deleted/404 cross-store
invalidation and late-result generation guards retain their existing
semantics.

The 257-entry baseline tests prove a finite bound. Additional assertions touch
an older key before inserting another entry (true LRU rather than FIFO) and
re-request an evicted key (recoverability).

## Change surface and conflict containment

Expected files:

- intent GC: `message-machine.ts`, `gateway-mirror/mirror.ts`,
  `gateway-mirror/transcript-lifecycle.ts`, and focused tests;
- disk LRU: `main/transcript-cache.ts`, `main/index.ts`, and focused tests;
- Git status: `NewThreadEmptyState.tsx`, one React-free cache helper, and its
  test;
- Capsule memory: the two renderer stores, their tests, and the existing
  Capsule hook file.

No `AppShell.tsx` change is required. No renderer
`gateway-mirror/transcript-cache.ts` change is planned, avoiding the parallel
work called out in the task.

## Validation and gates

1. Re-run each baseline guard and record FAIL -> PASS.
2. Run the focused intent, mirror-contract, transcript-cache, workspace-cache,
   and Capsule-store tests.
3. Run `npm run test:unit` from `desktop/garyx-desktop`.
4. Run `npx tsc --noEmit` from `desktop/garyx-desktop`.
5. Rebase onto the latest `origin/main`, reconcile overlapping upstream
   semantics, and repeat the full desktop checks before merging.
