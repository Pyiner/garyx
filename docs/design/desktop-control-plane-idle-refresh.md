# Desktop control-plane idle refresh convergence

Status: approved (#TASK-2220)

Task: #TASK-2207

## Goal and boundary

Reduce Mac app control-plane traffic while an already-connected, visible
window is idle, without changing gateway behavior. The desktop client keeps a
slow fallback refresh because control-plane SSE invalidation is a separate
follow-up. Reconnect recovery, visibility recovery, and refreshes caused by
mutations remain event-driven and must continue to converge state.

This change does not alter transcript rendering. `render_state` remains the
only source of transcript row structure, tool grouping, active tool state,
final-answer placement, and tail thinking.

## Deterministic baseline

Baseline source revision: `e734a243c`.

The measurement runs the current desktop source in dev mode with an isolated
Electron `userData` directory. Its saved gateway URL points at an HTTP
forwarding meter on `127.0.0.1`; the meter forwards every request unchanged to
the already-running local gateway. This isolates the measured app from other
Garyx clients without adding gateway instrumentation.

Before opening the measurement window:

1. Wait for the app to connect and finish bootstrap.
2. Select a non-running thread whose live `/api/tasks/forest` response is
   empty, representing an ordinary conversation with no task tree.
3. Bring the renderer to the front and verify through CDP that
   `document.hidden === false`.
4. Reset the meter's epoch, make no UI input for 60 seconds, then snapshot it.
   Requests that began before the reset, including pre-existing streams, do
   not count in the new epoch.

The meter counts requests forwarded upstream. Byte totals are wire-level
request headers + request bodies + response headers + response bodies observed
at the forwarding boundary. Query strings are omitted only from the grouped
path labels, not from byte accounting.

### Before

The fixed window lasted 60.153 seconds.

| Traffic | Requests | Bytes |
| --- | ---: | ---: |
| All forwarded gateway traffic | 70 | 14,111,714 |
| Response bodies (subset of total) | — | 14,070,359 |
| `/api/threads` | 6 | 10,151,928 |
| `/api/custom-agents` | 6 | 3,243,996 |
| `/api/tasks/forest` for the empty tree | 11 | 8,261 |
| Health trio (`/api/chat/health`, `/api/status`, `/runtime`) | 15 | 9,915 |

The remaining requests are the other full desktop-state slices. Automations,
channel endpoints, configured bots, thread pins, and workspaces were each
fetched six times; the independently cached bot-console slice was fetched
twice. The observation reproduces both reported causes: the 12-second healthy
poll drives repeated full state rounds, and an empty task forest keeps polling
every five seconds.

### After

The same meter, isolated profile, selected-thread condition, visibility check,
and 60-second procedure were rerun against the changed renderer. The fixed
window lasted 60.191 seconds.

| Traffic | Requests | Bytes |
| --- | ---: | ---: |
| All forwarded gateway traffic | 23 | 2,417,566 |
| Response bodies (subset of total) | — | 2,404,360 |
| `/api/threads` | 1 | 1,692,308 |
| `/api/custom-agents` | 1 | 540,666 |
| `/api/tasks/forest` for the empty tree | 0 | 0 |
| Health trio (`/api/chat/health`, `/api/status`, `/runtime`) | 15 | 9,915 |

Requests fell by 47 (67.1%), and total observed bytes fell by 11,694,148
(82.9%). The health trio is byte-for-byte unchanged, showing that liveness
polling was retained. Every uncached desktop-state slice ran exactly once from
the 60-second fallback, while the confirmed-empty task forest made no request
inside the measurement window. The independently cached bot-console slice was
also due once during this final run, so every full-state slice is represented
in the after evidence.

The merge gate repeated the same cold-start procedure after fetching and
rebasing onto the latest `origin/main` (the rebase was a no-op). That 60.172
second window again recorded 23 requests: zero task-forest requests, 15 health
requests totaling 9,915 bytes, and exactly one request for every full-state
slice. Total traffic was 2,419,634 bytes; the 2,068-byte difference from the
table above came from live thread, agent, endpoint, and pin payload growth, not
from an extra request or a scheduling change.

## Current causes

### Healthy connection polls trigger data refreshes

`useGatewayConnectionController.ts` runs a lightweight health poll every 12
seconds. Every successful result calls the ready-state refresh path, whose
throttle is also 12 seconds, so steady health is treated like a readiness
transition. Each trigger reaches the full desktop-state fetch plus the custom
agent catalog.

The same hook has an independent 60-second silent refresh interval. It is
therefore redundant with the accidental 12-second full refresh, and it calls
the fetch directly rather than sharing the existing debounce timer.

### Full refreshes can overlap

`GatewayMirror.refreshDesktopState()` starts `getState()` and
`listCustomAgents()` for every invocation. Reconnect recovery, visibility
recovery, stream/mutation invalidations, and periodic paths can overlap. A slow
gateway can consequently have multiple complete rounds in flight.

### Empty task forests never become quiescent

`ThreadTaskTreePopover` starts a five-second interval whenever the component is
mounted. Mounting depends on the surrounding desktop layout, not on whether the
selected thread belongs to a task tree. An empty successful response therefore
polls forever. The loader also has no page-visibility guard. Separately, tree
snapshot eviction removes only `treeSnapshotByKey`; reverse anchor entries in
`treeKeyByAnchor` accumulate and can point at evicted trees.

## Design

### 1. One periodic desktop refresh channel

The 12-second health poll will update connection state and retry backoff only.
It will not request desktop state after an unchanged successful poll.

The existing connection-transition effect remains the readiness owner. It
requests exactly one refresh only for `false -> true`. `null -> true` is initial
bootstrap and does not need a duplicate fetch; `true -> true` is steady health
and does nothing. Recovery history scheduling for active/disconnected threads
stays after that refresh exactly as today.

The 60-second fallback remains, but its timer will submit to the same
`scheduleDesktopStateRefresh` debounce channel used by stream/mutation
invalidations. It will not call the fetch directly or maintain a second local
`refreshing` flag. A periodic tick while `document.hidden` is ignored.

Visibility recovery remains immediate in intent: when the page becomes visible
while connected and bootstrapped, it submits one desktop refresh through the
same debounce channel and schedules the selected thread's canonical history
refresh. Hiding the page does not cancel event-driven mutation/reconnect work;
only periodic idle fetches pause.

A small pure refresh-policy helper will encode and test the transition and
visibility decisions used by the hook. This keeps timer/React wiring thin and
makes these regressions deterministic without a browser test harness.

### 2. Mirror-owned single-flight and trailing convergence

`GatewayMirror.refreshDesktopState()` will cease being an `async` wrapper so it
can return the exact shared Promise.

The mirror owns these states:

- the currently active refresh Promise;
- whether another refresh intent arrived while that Promise was active;
- at most one pending trailing timer/Promise.

The first call starts `getState()` and best-effort `listCustomAgents()` in
parallel, preserving the existing atomic per-domain landing behavior. Calls
while it is active return that exact Promise and set the trailing-intent bit;
they never start another network round. Once the active request settles, a
single short trailing debounce starts one more round if an intent arrived.
Calls while that tail is pending reuse its Promise. This prevents overlap while
still preserving a mutation or recovery signal that arrived during a slow
refresh. A failed active round also releases the single-flight slot and may run
the one queued tail; future calls are never wedged behind a rejected Promise.

The trailing delay uses the existing desktop refresh debounce scale (350 ms).
It is deliberately much shorter than the 60-second fallback: it combines a
burst of event invalidations but does not turn event-driven convergence into a
periodic wait.

The mirror itself lands every round into its root and catalog snapshots. Most
AppShell consumers still use migration-era React copies of those domains, so
the connection controller subscribes those two bridge states to mirror
notifications. This is necessary for an internally started trailing round to
reach legacy consumers even though no second hook caller owns that round.

### 3. Task-forest polling becomes evidence-based

Each mount/thread selection starts with one live probe, even when a cached
snapshot exists. Only the first successful live snapshot for that selected
thread controls quiescence:

- empty first snapshot: stop the interval for that component/thread;
- non-empty first snapshot: retain five-second revalidation;
- transient error or a request skipped while hidden: no successful snapshot,
  so polling remains eligible.

Unmounting/remounting the surface or selecting another thread resets the probe
state and tries once again. Every load checks `document.hidden` before IPC.
When a polling-eligible tree becomes visible, a visibility listener triggers a
prompt revalidation instead of waiting for the next five-second boundary. A
tree stopped because its first snapshot was empty stays stopped until remount
or thread change, as required. Run activity does not re-arm an empty stopped
tree: whether a task is created externally or by that same conversation after
the empty snapshot, discovery waits for the explicitly requested remount/thread
change probe. Adding a run-driven re-arm would be a different product contract
and would couple task-forest polling to transcript/run events that this task
does not ask to introduce.

When snapshot LRU eviction removes a tree key, the cache will also delete every
anchor entry whose value is that key. The eviction operation moves to a pure
model helper so the reverse-index invariant has a unit test.

## Event behavior retained

| Trigger | Result after this change |
| --- | --- |
| Healthy 12-second poll, still connected | Connection observation only; no desktop-state refresh |
| Connection `false -> true` | One desktop refresh, then existing thread-history recovery |
| 60-second fallback while visible | One debounced desktop refresh |
| 60-second fallback while hidden | No desktop refresh |
| Hidden -> visible | One debounced desktop refresh plus canonical history refresh for the open thread |
| Stream lifecycle / mutation invalidation | Existing debounced refresh entry remains active, including while another refresh is in flight |
| Direct mutation flow that awaits fresh state | Existing direct call remains; mirror single-flight/tail semantics prevent overlap |

## Validation plan

Focused unit coverage will assert:

1. Refresh policy: only `false -> true` is reconnect readiness; steady healthy
   polls do not refresh; visible recovery requests desktop plus selected-thread
   history; hidden periodic ticks do nothing; mutation invalidation remains a
   schedulable event.
2. Mirror contract: concurrent calls return the same Promise and one fetch
   pair; a burst during that request produces at most one trailing fetch;
   pending-tail callers share one Promise; failure releases the slot.
3. Task forest: an empty first successful snapshot stops polling, an error does
   not, visibility gates loads, thread/remount reset is represented by a fresh
   policy state, run/visibility activity alone does not re-arm an empty stopped
   tree, and LRU eviction removes reverse anchor mappings.
4. Existing transcript lifecycle tests still prove run-lifecycle events call
   the mutation refresh seam. The full mirror contract suite must remain green.

Required final checks:

```text
cd desktop/garyx-desktop
npm run test:unit
npx tsc --noEmit
```

The post-change 60-second traffic measurement is a release gate. Code review
must compare the before/after request and byte totals in this document.

## Trade-offs and follow-up

- The client retains one 60-second full refresh while visible, so it still
  converges if an event source is missed. Removing that fallback requires the
  separate gateway control-plane SSE work.
- Health polling remains at 12 seconds. Its three small endpoints are visible
  in the baseline, but connection liveness is not the expensive payload and is
  outside this task's intended semantic change.
- Stopping empty task-tree polling means a task tree created later—externally or
  by the same already-open conversation—is discovered on remount/thread
  reselection rather than within five seconds. This is the explicit
  bandwidth/latency trade-off requested for ordinary conversations ("first
  empty snapshot stops periodic polling; reopening the panel or switching
  threads probes again").
