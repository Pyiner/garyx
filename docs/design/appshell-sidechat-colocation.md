# Side-Chat Colocation (Endgame Batch 5b-7)

Parent design: `appshell-endgame-architecture.md`, destination-map row
"useSideChatController (1058) — SideChatPanel". Prerequisites landed:
6b-2 dissolved the transcript controller into the mirror (accept*,
notifyStreamEvent, loadSelectedThreadTranscript, ensureThreadOpenable,
per-thread stream start/stop), 2d flipped the dispatch orchestrator to a
MirrorPort, and 5b-6 established the lifetime-split colocation pattern
(intent state in the shell, DOM effects in the feature component).

## Current inventory

`useSideChatController` (1058 lines, 43 args, 45 returned members) owns
four layers:

1. **Session state (per source thread):** `sideChatThreadBySource`
   (source → side-thread id; ALSO persisted to `window.sessionStorage`
   via `persistSideChatThreadId` / restored by a per-source effect —
   sessionStorage on purpose: bindings survive dock toggles and view
   switches within one app session but reset on relaunch; the store must
   keep EXACTLY this scope),
   `sideComposerBySource` (drafts; NOT persisted),
   `sideComposerAttachmentUploadCount` (a single in-flight upload
   counter — it locks the composer and blocks submit while uploads run),
   `sideChatCreatingBySource`, `sideChatErrorBySource`,
   `sideChatCreationBySourceRef` (in-flight create de-dupe),
   `sideChatHistoryLoading`.
2. **Wiring shadows consumed OUTSIDE the panel:** `sideChatThreadIdRef`
   (transcript-lifecycle deps: refetch re-arms the side stream),
   `sideChatThreadIdsRef` (dispatch-orchestrator deps: queue routing
   ignores side threads), `sideChatStreamConsumerId` (pure,
   `side-chat:${threadId}`; lifecycle deps).
3. **Render derivations:** ~20 useMemo/consts over mirror maps
   (messages, renderState, pagination, pending inputs, live stream,
   queue, provider type, agent label, team view, composer gates).
4. **Behavior:** `ensureSideChatThread` (create + bind + persist +
   stream start), `handleSideComposerSubmit`, attachment pipeline,
   `openTaskThreadInSidePanel`, the transcript-load effect (fetch +
   `startCommittedThreadStream` per side thread), the composer-draft
   CRUD.

The panel UI itself is the second `<ThreadPage surfaceVariant="side-chat">`
instance rendered by AppShell inside the inspector dock, fed ~40 props
from the controller's return surface.

## Constraints (why this is not a straight move)

- **C1 — sessions outlive the dock.** Closing the inspector (or leaving
  the chat tool tab) unmounts the panel — confirmed in review
  #TASK-1658: the side ThreadPage unmounts with the dock/tab while the
  controller stays mounted in AppShell. The side-thread MAPPING survives
  via sessionStorage, but composer drafts, the attachment-upload
  counter (composer lock), creating/error transients, and the in-flight
  creation promise are React state: moving them into a dock-mounted
  component loses a half-typed draft — or a mid-upload composer lock and
  its eventual result — on dock toggle. (The 5b-6 lesson: split by
  lifetime, not by file.)
- **C2 — two orchestration seams read side-chat identity.** The
  dispatch-orchestrator deps (`sideChatThreadIdsRef`) and the
  transcript-lifecycle deps (`sideChatThreadIdRef`,
  `sideChatStreamConsumerId`) are fed from AppShell every commit. These
  must keep working while the dock is closed (a refetch on a side thread
  re-arms its stream even when the panel is hidden).
- **C3 — no parallel snapshot slices.** Mixing AppShell-passed map
  snapshots with panel-local uSES reads of the same domain can tear
  within one frame. A colocated panel must read each domain from ONE
  source: either all through its own uSES subscriptions or all through
  props — per domain, never both.
- **C4 — the mirror stays UI-free.** Side-chat sessions are a UI concept
  (which side thread a source thread's inspector shows); they do not
  move into the GatewayMirror.

## End state

Two pieces, following the 5b-6 lifetime split:

### 1. `SideChatSessions` — shell-owned session store (plain class)

A pure-TS store created once by AppShell (`useState(() => new ...)`),
holding layer 1 + the layer-2 shadows:

```
class SideChatSessions {
  // state
  threadBySource / composerBySource / creatingBySource / errorBySource
  creationPromiseBySource
  attachmentUploadCount                   // composer lock while uploads run
  // derived shadows (kept in sync on every write)
  sideChatThreadIdRef: { current }        // follows the ACTIVE source
  sideChatThreadIdsRef: { current: Set }  // all bound side threads
  // api
  subscribe(listener) / getSnapshot()     // uSES-compatible, cached ref
  threadFor(source) / rememberThread(source, id)   // + sessionStorage write
  restorePersisted(source)                // sessionStorage read-through
  draftFor(source) / updateDraft(source, updater) / clearDraft(source)
  beginAttachmentUpload() / endAttachmentUpload()  // counter +/-
  setCreating(source, bool) / setError(source, msg)
  setActiveSource(source)                 // feeds sideChatThreadIdRef
  streamConsumerId(threadId)              // pure
}
```

The store is uSES-subscribable (single snapshot object, version-bumped
on writes — the mirror's snapshot rules). AppShell feeds
`sideChatThreadIdRef` / `sideChatThreadIdsRef` / `sideChatStreamConsumerId`
into the orchestrator/lifecycle deps FROM THE STORE — the deps shapes do
not change (C2). `setActiveSource` is driven by a tiny AppShell effect on
`activeThread?.id` (replacing today's `sideChatThreadIdRef` sync effect).

Composer drafts being store state (not React useState) keeps them alive
across dock toggles (C1); the panel re-renders through the uSES
subscription.

### 2. `SideChatPanel` — the colocated feature component

Mounted where the side ThreadPage instance is today (inspector dock).
Owns layers 3 + 4:

- Reads the session store via uSES (its slice of truth for
  bindings/drafts/transients).
- Reads mirror domains via its own subscriptions:
  `useThreadMirror(sideThreadId)` for messages/renderState/pagination/
  pendingInputs/liveStream, machine state via the mirror's machine
  subscription for the queue, root/catalog for desktopState/agents
  (C3: each domain single-sourced inside the panel; the few
  shell-truth inputs that remain — `activeThread`, bot bindings,
  `pendingAgentId`, i18n `t` — stay props).
- Behavior moves verbatim: `ensureSideChatThread`,
  `handleSideComposerSubmit`, attachments. Mirror-backed operations call
  the mirror directly (`acceptRemoteTranscript`, `ensureThreadOpenable`,
  `startCommittedThreadStream`, `runQueuedBatch`, `steerQueuedIntent`,
  `dispatchMachineAction` — all facades exist post-6b-2/2d).
  `prepareAttachmentUploads` and `setError`/`setPendingAutomationRun`
  stay injected props (shell/IPC-owned). `openTaskThreadInSidePanel`
  and the side transcript-load effect do NOT move — both stay in the
  shell (the command and the always-on effect below).
- Renders `<ThreadPage surfaceVariant="side-chat">` itself, collapsing
  the ~40 prop pass-through in AppShell to one `<SideChatPanel ...>`
  with a dozen shell-truth props.

`openTaskThreadInSidePanel` is called from OUTSIDE the panel (task rows
in the Tasks tool tab — where the chat panel is NOT mounted, review
#TASK-1658). A panel handle is therefore unsound: the ref is null at
the real call point, and opening the chat tab first is racy because
`openTool("chat")` fires `onOpenSideChat()` which can create/bind the
DEFAULT side chat before the target task thread is applied. Instead the
operation becomes a **shell/store command**: it writes the binding into
`SideChatSessions` (`rememberThread(source, taskThreadId)` + clears
creating/error), then opens the dock and the chat tool tab. The panel,
on mount, renders whatever the store says — no handle, no defer, and
the `openTool("chat")` auto-open sees the binding already present so it
does not create a default side chat. The command lives next to the
store in AppShell (it also needs `ensureThreadOpenable` + the stream
start, both mirror facades).

Tab control path: `openTools`/`activeTabKey` are ThreadSideToolsPanel-
PRIVATE state today, so the shell command cannot flip the tab directly.
The panel grows one narrow prop, `pendingOpenToolRequest:
{ tool: ThreadSideToolId; requestId: number } | null`, consumed by a
panel effect that runs the EXISTING `openTool(tool)` path and acks by
requestId (the pendingWorkflowTaskHint mailbox pattern; requestId makes
repeat opens of the same tool re-fire). The command therefore does:
store write → set dock open → publish `{ tool: "chat", requestId: n }`.
Because the store binding is written first, the auto-open inside
`openTool("chat")` finds it and does not create a default side chat.
No new tab-state owner: the panel keeps its single source of truth
(#TASK-1470).

If the transcript-load effect must also run while the dock is CLOSED
(today it does — the controller is always mounted, so a bound side
thread keeps its committed stream alive even with the dock hidden),
that effect CANNOT live only in the panel. Decision: it stays a small
AppShell effect driven from the session store (active side thread id →
`loadSideThreadTranscript`-style mirror calls with the side consumer
id), OR we accept the behavior change (stream only while visible) with
an explicit sign-off. Default: keep the always-on effect in the shell —
byte-equal behavior first, revisit later.

## Slices

1. **5b-7a — SideChatSessions store.** Introduce the store; the
   existing controller becomes a consumer (state moves out of useState
   into the store; controller reads via uSES). Deps feeding flips to
   store-backed refs. No UI moves. Dual-oracle: dock toggle keeps a
   half-typed draft; queue routing still skips side threads.
2. **5b-7b — SideChatPanel.** Move derivations + behavior + the side
   ThreadPage instance into the panel; controller deletes;
   `openTaskThreadInSidePanel` becomes the shell/store command with the
   `pendingOpenToolRequest` tab mailbox (NOT a panel handle); the
   always-on transcript effect stays in the shell (explicitly scoped).

## Invariants

1. Dock toggle preserves: bound side thread, half-typed draft,
   creating/error transients, an in-flight creation, and an in-flight
   attachment upload (composer stays locked and the uploaded result
   lands in the surviving draft).
2. Orchestrator/lifecycle deps shapes unchanged; side-thread refetch
   re-arm works with the dock closed.
3. No domain is read through both props and panel-local uSES (C3).
4. Behavior byte-equal; any lifecycle narrowing (stream-while-visible)
   is out of scope unless separately signed off.

## Review-confirmed premises (#TASK-1658)

- The side ThreadPage instance unmounts with the dock/tab; the
  controller stays mounted in AppShell (C1 is real).
- The side transcript-load effect is keyed to `sideChatThreadId`, not
  `inspectorOpen` — the always-on premise holds, so keeping that effect
  in the shell preserves behavior.
- The C2 dep shapes are store-swappable as described.

## Validation

Per slice: tsc, unit (store contract tests: persistence read-through,
draft CRUD, shadow-ref sync), build:ui, electron smoke, live CDP
walkthrough (open side chat from a task row with dock closed → dock
opens and binds; type a draft → toggle dock → draft intact; send in
side chat → queue drain unaffected on main thread; refetch on side
thread with dock closed re-arms stream — verified via stream IPC
logging or gateway consumer list).
