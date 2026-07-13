# TASK-2190 — Preserve transcript ownership across `render_floor`

Status: design review passed (`#TASK-2191`)
Owner: `garyx-models` transcript render reducer + `garyx-router` transcript cache
Clients: server-owned behavior; desktop and iOS remain dumb renderers

## Problem and deterministic reproduction

The reported iOS thread contains 1,761 committed records. The visible card in
the captured screenshot is a task-notification user record at seq 1589. Its
actual ledger boundary is:

```text
seq 1588  tool_use(tool id X)
seq 1589  user(<garyx_task_notification ...>)  <- cold-open render floor
seq 1590  control(user_ack)
seq 1591  tool_result(tool id X)
seq 1592+ later tool / assistant records
```

The full reducer correctly flushes the pending tool group before the user
record and, when seq 1591 arrives, repairs seq 1588's already-emitted entry in
place. The windowed path does something different: both the file-tail cache
and full-read fallback discard every record below `render_floor`, then invoke
`reduce_transcript_render_state_with_run_state` on only the truncated ledger.
The reducer can no longer see seq 1588, so it treats seq 1591 as an orphan
result and appends it to the task-notification turn. Consecutive tool records
can then join that incorrect group.

The minimized router test
`render_window_does_not_reparent_cross_floor_tool_result_to_task_notification`
reproduces the same five-record shape. Baseline output is:

```text
full refs:   [seq:1, seq:2, seq:4, seq:3, seq:5]
window refs: [seq:3, seq:4, seq:5]   # wrong: seq:4 changed owner
expected:    [seq:3, seq:5]
```

The test fails before implementation. This is a render-state defect, not a
SwiftUI sorting defect: the same committed result changes turn ownership when
the client declares a floor. Because row ids remain stable while their nested
structure and height change, the iOS eager `VStack` can retain/reconcile an old
card at the wrong geometry while later rows keep changing, producing the
reported fixed/overlapping appearance. The screenshot's translucent mirrored
text is the task card passing under the glass top chrome; it is a consequence
of the incorrect oversized/reparented turn, not a separate pinned overlay.

## Root cause

`ThreadTranscriptStore::render_snapshot_in_window` correctly derives
`TranscriptRunState` from the full committed prefix, but it supplies no render
prefix state to the render reducer. The reducer therefore starts these pieces
of state from empty/default at the floor:

- the currently open tool group;
- identified tool calls flushed before the floor but still awaiting a late
  result;
- the latest `run_start` seq used to decide whether a tail tool group belongs
  to the current run.

The full reducer's `ToolGroupBuilder::flushed_pending_by_id` is deliberately
long-lived across user/assistant boundaries (#TASK-1603/#TASK-1680), so a
windowed reducer cannot reconstruct correct ownership from records at or above
the floor alone. Supplying only the full `TranscriptRunState` cannot fix tool
ownership or group identity.

## Proposed change

### 1. Add a compact, foldable render-prefix state in `garyx-models`

Introduce an opaque `TranscriptRenderPrefixState` next to the transcript
render reducer. It is `Clone + Default` and can be advanced one committed
record at a time. It stores only information that can affect records after a
future floor; it does not retain rendered rows or command output:

- `latest_run_start_seq`;
- an optional hidden/open tool-group seed (first seq, first tool-use id,
  started/finished timestamps);
- identified and anonymous pending calls in that open group;
- identified pending calls that were already flushed at a narrative boundary.

The reducer and prefix derivation must share one per-record transition
function; the prefix fold is not a second implementation of grouping rules.
The transition accepts an emission mode: full/window reduction retains
visible blocks, while prefix derivation discards blocks that are wholly below
the floor but retains the exact residual `ToolGroupBuilder` state. Its fold
therefore follows the existing boundary rules by construction:

- tool uses/results update the open group;
- user/assistant/unknown-message boundaries flush the open group, moving only
  identified unresolved calls into the cross-boundary pending set;
- ordinary controls remain non-boundaries; `capsule_attached` remains a
  boundary, matching the reducer;
- an empty streaming assistant flushes the open group before that assistant is
  filtered, matching the full reducer's ordering;
- a late identified result consumes the matching flushed pending call;
- `run_start` advances `latest_run_start_seq`.

No task-tag special case is added. A task notification is an ordinary visible
user boundary; the same invariant must hold for human steering and any future
synthetic user input.

### 2. Seed the window reducer from that prefix state

Add a window-aware reducer entry point that accepts:

```text
records at floor..=based_on_seq
full-prefix TranscriptRunState at based_on_seq
TranscriptRenderPrefixState for records with seq < floor
```

`ToolGroupBuilder` receives the hidden prefix state:

- a result matching a pre-floor flushed call is consumed without emitting or
  reparenting a visible result entry (the owning group is above the window);
- a result matching a hidden call in the open cross-floor group is consumed
  without inventing a new visible entry;
- visible entries that continue a cross-floor open group reuse its original
  group id/timestamps, so floor changes do not manufacture a new identity;
- a pending hidden entry keeps a mixed visible continuation group active,
  without emitting a body-less placeholder entry;
- unmatched result-only records still render as genuine orphans, preserving
  current forward-compatibility behavior;
- hidden-only groups emit no row.

Hidden and visible owners share the same pending-id bookkeeping semantics.
For an identified result, lookup order is:

1. an in-window pending call in the open group;
2. an in-window pending call already flushed to a visible group;
3. a pre-floor pending owner, whose result is consumed invisibly;
4. the existing anonymous/single-pending fallback rules;
5. a genuine visible orphan.

An in-window use that reuses an id held by the pre-floor state shadows and
removes that hidden owner. If the visible call is then flushed, its visible
owner replaces the hidden owner in the flushed map. This preserves the full
reducer's last-writer-wins behavior for duplicate ids and prevents a visible
result from being swallowed by stale prefix state. The anonymous pending FIFO
for a cross-floor open group is also preserved: a result consumes the oldest
hidden anonymous call invisibly, while anonymous calls that were flushed at a
pre-floor boundary do not survive that boundary, exactly as today.

The reducer initializes `latest_run_start_seq` from the prefix state, then
continues folding window controls normally. Full-snapshot reduction uses the
same code with a default/empty prefix, avoiding two grouping implementations.

### 3. Checkpoint render-prefix state beside run state in the router cache

Extend `ThreadCache` with a `render_prefix_checkpoint` whose boundary is the
existing `base_seq`. Whenever `roll_tail` folds a record into the run-state
checkpoint, it folds the same record into the render-prefix checkpoint. For a
window request, clone the checkpoint and fold cached tail records with
`seq < floor_seq`; pass the result to the window reducer.

The full-read/memory fallback derives the same prefix state from records below
the floor through that shared transition. This preserves the cache's current
performance contract:

- no full transcript read on each live frame;
- no copied tool output in the checkpoint;
- bounded work over the already-cached tail;
- file-cache and uncached derivations remain structural oracles for each
  other.

`based_on_seq`, event delivery, and `RenderWindow { floor_seq,
has_more_above }` are unchanged.

## Why this layer

- The server owns row order, grouping, tool pairing, active tool state, and
  final-answer placement. Repairing order or pinning in Swift would violate
  the dumb-renderer contract and leave desktop wrong.
- Dropping every unmatched result in a window is too broad: genuine
  result-only provider records are supported today. Prefix state lets the
  reducer distinguish a hidden matching use from a true orphan.
- Re-reading/reducing the entire transcript for every live frame is correct
  but regresses the bounded-tail cache that exists specifically for long
  active threads.
- Moving the task notification or delaying it until a run finishes treats one
  producer, while ordinary user steering can create the identical boundary.

## Impact and compatibility

- Wire schema: unchanged.
- iOS/desktop mapping code: unchanged.
- Full snapshots: unchanged.
- Windowed snapshots: only cross-floor context-dependent ownership/group
  identity and current-run tail classification change.
- Cache memory: one compact prefix state per resident transcript; it retains
  unresolved tool-call ids and group identity/timestamps, never transcript
  bodies or command output, and is charged to the transcript-cache budget.
- Persistence: no new on-disk format. Checkpoints rebuild from jsonl as the
  existing cache does.

## Validation plan

1. Keep the red router reproduction and turn it green. Assert the task turn
   references only its own user and assistant messages; the late result is not
   reparented.
2. `garyx-models` focused tests:
   - cross-floor flushed identified tool call;
   - floor inside an open tool group preserves group id/timestamps for visible
     continuation entries;
   - a hidden pending call keeps a mixed visible continuation group active
     while its visible completed entries remain completed;
   - genuine unmatched result-only record remains visible;
   - a hidden anonymous call in an open cross-floor group consumes the first
     late anonymous result invisibly and in FIFO order;
   - an empty streaming assistant immediately below the floor flushes before
     being filtered;
   - `capsule_attached` below the floor applies its flush side effect without
     emitting the capsule card;
   - a duplicate id above the floor shadows a hidden owner, both while open and
     after the visible group has flushed;
   - `run_start` below the floor makes tail activity use the correct run. The
     assertion uses the first *visible* entry for `tool_group_first_seq`; it
     does not claim the hidden seed changes that helper.
3. Add a model-level full-reduction ground-truth oracle, not merely two users
   of the new prefix fold. Sweep floors through table-driven transcripts and
   compare each visible tool entry's owning group/turn and group id with the
   full snapshot after clipping references below the floor. Include floors
   inside open groups and immediately after narrative, empty-streaming, and
   capsule boundaries. Structures whose anchor is visible must retain the full
   snapshot's owner and identity; visible continuation entries from a
   cross-floor group must retain that full group's id.
4. Router cache oracle tests must force a rolled checkpoint with tiny
   `tail_max_records`/`tail_max_bytes` and assert `base_seq > 0` before testing.
   Compare the production cached path with the full-reduction ground truth,
   not only with an uncached call that uses the same prefix fold. Separately
   retain cached-vs-uncached equality to cover checkpoint associativity, and
   use instrumentation to prove the cache-eligible hot path adds no full-file
   read.
5. Gateway stream test with a sanitized captured shape: initial frame floor on
   the notification; `render_state.rows` keeps chronological ownership and
   `based_on_seq` remains the committed tail.
6. iOS SwiftPM mapper regression: decode the sanitized frame and assert rows
   stay in server order with the notification as an ordinary in-flow row and
   no client-side sorting/pinning.
7. Focused commands: exact red/green test, `cargo test -p garyx-models`,
   `cargo test -p garyx-router --lib`, relevant gateway route test,
   `swift test`, and an iOS simulator target build (`xcodebuild ... -target
   GaryxMobile -sdk iphonesimulator ...`).

## Rollback

The change is internal derivation state with no migration. Reverting the model
prefix state and router checkpoint restores the old reducer behavior; no
client or persisted-data rollback is needed.
