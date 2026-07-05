# Transcript Controller Dissolution (Endgame Batch 6b-2)

Parent design: `appshell-endgame-architecture.md`, destination-map row
"useTranscriptController (1917) — Dissolved into mirror." Batches 6a/6b-1
made the mirror the single store (five render maps + transport snapshot);
what remains in the hook (1251 lines) is transport **orchestration**:
run-state publication, machine intent bookkeeping, remote-apply ride-along
duties, and the fetch/stream lifecycle. This design moves that
orchestration into the mirror behind one deps contract and dissolves the
hook to a thin binding.

## Current inventory (26 functions, by layer)

- **Mirror proxies (3c-1 leftovers):** updateLiveStreamState /
  replaceLiveStreamThreadId / clearLiveStreamState / getLiveStreamState —
  one-line wrappers feeding `liveStreamStateRef`.
- **Machine bookkeeping:** intentForId, setThreadRuntimeState,
  hasPendingHistoryIntents, markIntentsFromHistory, applyUserAck,
  forceReleaseThreadRuntime — pure orchestration over mirror-owned
  machine + live-stream state.
- **Run-state chain:** publishTranscriptRunState, syncTranscriptRunState,
  applyCommittedTranscriptRunState + `transcriptRunStateByThreadRef` —
  reduce/apply TranscriptRunState, publish machine/live-stream side
  effects, thread-title propagation.
- **Apply chain:** applyCanonicalTranscript, applyRemoteTranscript,
  applyCommittedThreadMessage, rememberTranscriptSnapshot (disk persist),
  cacheOpenableTranscriptThread + threadSummaryFromTranscript
  (desktopState session cache), applyThreadTitleUpdate (desktopState +
  title-draft sync), updateMessagesByThread (local-write bridge).
- **Fetch/stream lifecycle:** loadSelectedThreadTranscriptFromSingleSource,
  fetchSelectedThreadIncrementalTranscript, startCommittedThreadStream,
  loadOlderThreadHistoryPage (already delegating), handleChatStreamEvent
  (error/recovery orchestration) + the two effects (chat-stream listener,
  selected-thread loader).

## End state

A new mirror module `gateway-mirror/transcript-lifecycle.ts` (same
pattern as `dispatch-orchestrator.ts`: a class with `setDeps`, refreshed
every React commit) owns all five layers.

**Facade layering (review #TASK-1630): the existing pure-commit facades
keep their contract.** `applyAuthoritativeTranscript` /
`applyRemoteTranscript` stay cache-only commits (mirror-contract tests
pin that transcript commits do not touch the machine). The lifecycle
adds HIGH-LEVEL entries that run the ride-alongs and call the pure
commits internally:

```
mirror.setTranscriptLifecycleDeps(deps)
mirror.acceptAuthoritativeTranscript(threadId, transcript, opts)
mirror.acceptRemoteTranscript(threadId, transcript, opts)
mirror.notifyStreamEvent(event)     // ingest first, then side effects
mirror.ensureThreadOpenable(threadId)
mirror.loadSelectedThreadTranscript(threadId)  // + cancel, below
```

**Route/mirror boundary (review ruling):** the mirror never depends on
`DesktopRouteStore`. Route origin/version, `syncRoute`, the
request-sequence guard, and the entrySource mailbox stay in
AppShell/RouteEffectBridge; `selectExistingThreadInPlace` remains the
AppShell shell that wraps guard + route sync around the mirror's
transport call. "mirror.openThread" from the parent design is fulfilled
as `ensureThreadOpenable` + `loadSelectedThreadTranscript` — data and
transport only.

`GatewayMirrorServices` grows the transport IPC it still lacks —
`startThreadStream`, `stopThreadStream`, `loadThreadTranscriptCache`,
`saveThreadTranscriptCache`, `clearThreadTranscriptCache`,
`getThreadHistoryFull(threadId)`, and a paged
`getThreadHistoryPage({ threadId, afterIndex?, beforeIndex?, limit,
userQueryLimit })` covering both the forward incremental fetch and the
existing older-page fetch. All injected; the mirror stays pure TS and
node-testable. The chat-stream subscription itself stays a React effect
(window IPC listener) whose body becomes `mirror.notifyStreamEvent`.

**Single refetch owner (review #TASK-1630):** a committed
rewrite/reset control today reaches two seams — the ingest-side
`requestAuthoritativeRefetch` (deliberately no-op) and the hook's
side-effect pass. After 2c the lifecycle is the ONLY owner: ingest's
seam routes to the lifecycle's refetch entry, which de-dupes in-flight
refetches per thread (concurrent triggers coalesce into one
fetch+stream restart).

**Async operation tokens (review #TASK-1630):** every
`loadSelectedThreadTranscript` run takes a per-thread operation token
(generation counter inside the module). The React selected-thread
effect's cleanup calls `mirror.cancelSelectedThreadLoad(threadId)`
(token invalidation + stream stop, exactly the current effect-local
`cancelled` + cleanup semantics). Deps are destructured once at each
operation's entry (the legacy closure capture, the 3c-2 recipe); every
post-await state landing re-checks the token first — a superseded
operation never writes loading flags, streams, or errors.

**TranscriptLifecycleDeps (the React seams that remain):**

```
setDesktopState            — title/team propagation, session cache
setError                   — load/stream failures
setHistoryLoading          — selected-thread loader chrome
setPendingAutomationRun    — automation-response clearing
syncThreadTitleDraft       — colocated title root handle
requestSelectedThreadMessagesBottomSnap / onOlderPageFetched
                           — scroll-anchor seams (UI-owned by design)
recordGatewayStatusObservation, scheduleDesktopStateRefresh,
scheduleHistoryRefresh     — connection/refresh orchestration hooks
connection, settingsDraft  — per-commit snapshots (stream recovery's
                             gatewayUrl fallback)
selectedThreadIdRef, selectedThreadGenerationRef,
selectThreadRequestSequenceRef, lastRenderedMessageThreadRef,
messagesRef, pendingMessagesPrependAnchorRef
                           — selection/scroll shadows the lifecycle reads
```

(`refetchAuthoritativeTranscriptAfterRewrite` is NOT a seam — it moves
inside as the single-owner refetch above.) `threadTitleOverridesRef`
becomes module-internal state with a read facade for the
dispatch-orchestrator (which uses it to avoid clobbering remote titles);
its orchestrator dep flips to the internal reference in 2d.

Existing dispatch-orchestrator deps that today point at hook functions
(applyCanonicalTranscript, updateMessagesByThread, getLiveStreamState,
updateLiveStreamState, clearLiveStreamState, hasPendingHistoryIntents,
scheduleHistoryRefresh, intentForId, setThreadRuntimeState) flip to
internal mirror references — dissolving most of the deps plumbing.

The hook shrinks to: the stream-listener effect (body =
`mirror.notifyStreamEvent`), the selected-thread effect
(`mirror.loadSelectedThreadTranscript` via the AppShell shell, cleanup =
cancel), the older-page auto-load effect and the messagesByThread
reader — then AppShell's remaining consumers (scroll handlers calling
loadOlderThreadHistoryPage, refetch/schedule paths, side-chat args) move
to mirror calls and the hook deletes.

## Slices (each lands + reviews separately)

1. **6b-2a — machine + run-state orchestration.** Move the machine
   bookkeeping and run-state chain into `transcript-lifecycle.ts`
   (deps: machine access is internal; React seams for title/desktopState).
   The hook delegates; `transcriptRunStateByThreadRef` moves inside the
   module (plain Map, not a React ref). Dual-run oracle construction
   (review #TASK-1630): the test builds TWO independent GatewayMirror
   instances — one driven through legacy-shaped hook bindings extracted
   as pure functions, one through the lifecycle module — replays the
   same recorded event sequences into both, and asserts the machine
   action trace, live-stream transition trace, and terminal states are
   deep-equal (the 3c-2 recorded-ack recipe; side effects land in stub
   deps that append to traces).
2. **6b-2b — apply chain.** applyCanonical/applyRemote/applyCommitted
   ride-alongs (persist, session cache, title, team, intent marking) move
   into the module; the mirror's applyAuthoritative/applyRemote facades
   run them so every caller (dispatch orchestrator included) gets one
   entry point. updateMessagesByThread moves as the local-write entry.
3. **6b-2c — fetch/stream lifecycle.**
   `loadSelectedThreadTranscript` absorbs the single-source loader +
   incremental fetch + startCommittedThreadStream + the missing-thread
   gate behind the operation token; services grow the stream/cache IPC;
   handleChatStreamEvent's error/recovery pass moves as the
   notifyStreamEvent side-effect step with the single refetch owner.
   The selected-thread React effect calls the mirror with cleanup =
   `cancelSelectedThreadLoad`; the older-page auto-load effect and the
   AppShell main/side scroll handlers keep calling the (now fully
   mirror-owned) older-page entry.
4. **6b-2d — dissolve.** Live-stream proxies delete (consumers call the
   mirror); dispatch-orchestrator deps flip to internal references;
   AppShell consumers (acceptRemoteTranscript in refetch/schedule paths,
   ensureThreadOpenable, side-chat args, the scroll handlers) move to
   mirror calls; threadTitleOverrides flips to the internal read facade;
   the hook file deletes. `ensureThreadOpenable` +
   `loadSelectedThreadTranscript` together fulfill the parent design's
   mirror.openThread contract (data/transport only — route semantics
   stay in the shell per the boundary ruling).

## Invariants

1. Byte-equal orchestration: for identical event/transcript sequences,
   machine actions, live-stream transitions, persisted cache writes, and
   render maps match the current hook (dual-run tests per slice, plus the
   existing 385-test suite and electron smoke).
2. The scroll-anchor and selection-sequence seams keep their exact
   call timing (onPageFetched between fetch and apply; request-sequence
   guard semantics; same-tick promotion sync from 6c-2).
3. No new React state: everything the module keeps is internal maps;
   React reads stay uSES snapshots.
4. Side-chat behavior unchanged (it consumes the same facades through
   AppShell args until its own colocation cut).

## Validation

Per slice: tsc, unit (dual-run additions), build:ui, electron smoke,
live CDP dispatch walkthrough (send → optimistic → settle → replay),
pagination + cold-open + missing-thread gates re-run on the final slice;
packaged proof after 6b-2d.
