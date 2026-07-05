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
every React commit) owns all five layers. The mirror facade grows:

```
mirror.setTranscriptLifecycleDeps(deps)
mirror.openThread(threadId, opts)        // the selected-thread loader
mirror.applyAuthoritative / applyRemote  // absorb the ride-alongs
mirror.notifyStreamEvent(event)          // ingest + side-effect pass
mirror.interrupt-related bookkeeping stays in dispatch-orchestrator
```

`GatewayMirrorServices` grows the transport IPC it still lacks:
`startThreadStream`, `stopThreadStream`, `loadThreadTranscriptCache`,
`saveThreadTranscriptCache`, `clearThreadTranscriptCache`,
`getThreadHistoryFull(threadId)` — all injected, keeping the mirror pure
TS and node-testable. The chat-stream subscription itself stays a React
effect (window IPC listener) but its body becomes one call:
`mirror.notifyStreamEvent(event)`.

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
refetchAuthoritativeTranscriptAfterRewrite, scheduleHistoryRefresh
                           — connection/refresh orchestration hooks
selectedThreadIdRef, selectedThreadGenerationRef,
selectThreadRequestSequenceRef, lastRenderedMessageThreadRef,
messagesRef, pendingMessagesPrependAnchorRef
                           — selection/scroll shadows the lifecycle reads
```

Existing dispatch-orchestrator deps that today point at hook functions
(applyCanonicalTranscript, updateMessagesByThread, getLiveStreamState,
updateLiveStreamState, clearLiveStreamState, hasPendingHistoryIntents,
scheduleHistoryRefresh, intentForId, setThreadRuntimeState) flip to
internal mirror references — dissolving most of the deps plumbing.

The hook shrinks to: the two effects (stream listener body = one mirror
call; selected-thread effect = `mirror.openThread` + stream stop on
cleanup), the messagesByThread reader, and the return surface AppShell
still consumes — then AppShell's remaining consumers move to mirror
calls and the hook deletes (a follow-up cut inside this batch).

## Slices (each lands + reviews separately)

1. **6b-2a — machine + run-state orchestration.** Move the machine
   bookkeeping and run-state chain into `transcript-lifecycle.ts`
   (deps: machine access is internal; React seams for title/desktopState).
   The hook delegates; `transcriptRunStateByThreadRef` moves inside the
   module (plain Map, not a React ref). Contract tests: run-start/ack/
   terminal sequences drive machine + live-stream transitions identically
   (dual-run against the current hook bindings, the 3c-2 recipe).
2. **6b-2b — apply chain.** applyCanonical/applyRemote/applyCommitted
   ride-alongs (persist, session cache, title, team, intent marking) move
   into the module; the mirror's applyAuthoritative/applyRemote facades
   run them so every caller (dispatch orchestrator included) gets one
   entry point. updateMessagesByThread moves as the local-write entry.
3. **6b-2c — fetch/stream lifecycle.** openThread absorbs
   loadSelectedThreadTranscriptFromSingleSource + incremental fetch +
   startCommittedThreadStream + the missing-thread gate; services grow
   the stream/cache IPC; handleChatStreamEvent's error/recovery pass
   moves as the notifyStreamEvent side-effect step. The selected-thread
   React effect becomes `mirror.openThread(id)` with the same
   cancellation/stop-stream cleanup.
4. **6b-2d — dissolve.** Live-stream proxies delete (consumers call the
   mirror); dispatch-orchestrator deps flip to internal references;
   AppShell consumers (applyRemoteTranscript in refetch/schedule paths,
   ensureThreadOpenable, side-chat args) move to mirror calls; the hook
   file deletes. ensureThreadOpenable becomes `mirror.ensureThreadOpenable`
   (the parent design's mirror.openThread contract).

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
