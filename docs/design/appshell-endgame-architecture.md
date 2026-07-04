# AppShell Endgame Architecture

Status: design, not yet implemented. Synthesized from design B (#TASK-1596,
`appshell-endgame-architecture-b.md` in its task worktree) plus first-hand
findings from the T13 controller-extraction batches. Supersedes both drafts.

## Goal

Evolve the desktop renderer from the current AppShell-centered controller mesh
into:

```text
Electron main gateway client / preload IPC
        |
        v
Pure TypeScript model layer (no React imports)
  - GatewayMirror: gateway-backed state, per-thread transcript caches,
    stream frontiers, message dispatch, queue state, remote refresh actions
  - DesktopRouteStore: URL hash as the single route source of truth
        |
        v
Thin React bindings
  - useSyncExternalStore domain/thread subscriptions
  - stable service/action contexts (identity never changes)
  - colocated local UI state in feature components
        |
        v
Feature components
  AppChrome, ThreadRoute, ThreadTranscriptViewport, ComposerSurface,
  SideChatPanel, SideToolsPanel, SettingsPanel, AutomationPanel,
  WorkspaceFilesPanel, MemoryDialogRoot, ThreadLogPanel
```

This removes the three structural problems T13 documented without changing any
gateway/server contract:

- **args drilling**: hooks receive 23-46 item arg objects because they cannot
  share state directly (`useMessageDispatchController.ts` args: 46);
- **TDZ cycles**: `runQueuedBatch`/`steerQueuedIntent`/`ensureThreadOpenable`/
  `connection` are pinned in the component body because hook call order is the
  only dependency mechanism (each T13 batch documented one such stay-behind);
- **whole-shell re-renders**: 62 remaining useState in one component mean any
  state change re-renders the entire shell and rebuilds every hook's args
  object.

An additional payoff surfaced by the T13 reviews: three of the six batches
needed careful "effect order shift is safe" proofs (batch 4 ref-sync hoist,
batch 5 queue-drain key separation, batch 6 handler-ref assignment vs IPC
macrotask). Those proofs exist only because domain logic lives in effects whose
registration order matters. Once ingestion and dispatch move into a mirror that
commits synchronously and atomically, that entire class of ordering hazards
disappears.

## Non-goals

- No gateway or server contract change. `thread_render_frame { events,
  render_state }`, `based_on_seq`, `render_floor`, and cursor semantics stay
  exactly as specified in `docs/agents/repository-contracts.md` (Transcript
  Rendering).
- No second transcript reducer. Row grouping, tool grouping, tail thinking, and
  final-answer placement remain server-owned; the renderer keeps dumb-rendering
  `render_state.rows` through the existing `render-view-model.ts` mapper.
- No new npm dependency (see Dependency Decision).

## Ownership model

State classification measured on the post-T13 tree (AppShell 5681 lines,
62 useState):

| Class | ~Share | Examples | End-state owner |
| --- | --- | --- | --- |
| Server/gateway mirror | 40% | messagesByThread, renderStateByThread, liveStreamStateByThread, threadInfoByThread, historyPaginationByThread, pendingRemoteInputs, desktopState, agents/teams/workflows, providerModels, connection | `GatewayMirror` |
| Derived | 20% | activeThread, activeMessages, queue/activity projections, route-derived view | selectors over mirror/route snapshots |
| Local UI | 25% | composer draft/attachments, drag targets, dialogs, layout widths, scroll anchors, thread logs | colocated feature components |
| True global | 15% | route (today split across contentView + selectedThreadId + hash), mirror/store instances, stable services | `DesktopRouteStore`, contexts |

### GatewayMirror

A class (factory-constructed, not a module singleton — tests need isolated
instances and teardown of timers/stream consumers). Pure TypeScript, zero React
imports.

Owns: root gateway state (DesktopState, connection, agents, teams, workflows,
provider model catalogs, refresh status); per-thread committed message body
cache; per-thread server `RenderState` accepted only monotonically by
`based_on_seq`; per-thread runtime info, pending remote inputs, pending
automation run, history pagination/window; per-thread live-stream status,
consumer registry, committed frontier, render frontier, render floor;
message-machine state and intent queues coupled to dispatch/ack/steer/interrupt
and history convergence.

Does not own: URL/route state (sibling `DesktopRouteStore`); text drafts,
attachment pickers, drag targets, scroll anchors, panel widths, dialog state;
any transcript structure derivation.

Internally the mirror is one store with three pinned module boundaries —
`transcript-cache` (committed bodies + render snapshots + pagination),
`dispatch-machine` (message machine, queue, send/steer/interrupt), and
`frontier` (per-thread/per-consumer cursors + stream lifecycle) — so the class
stays a facade over separable, individually-testable modules rather than
drifting into a god object.

### DesktopRouteStore

Sibling pure-TS store. Owns the current `DesktopRoute`, parses
`window.location.hash`, subscribes to `hashchange`/`popstate`, and writes the
hash only through explicit `navigate(route, { replace })` calls. `contentView`
becomes a selector over the route via the already-pure
`contentViewForDesktopRoute` (`app-shell/desktop-route.ts`); the separate
`contentView` state and its sessionStorage persistence are deleted.

## GatewayMirror API draft (signature level)

```ts
export type Unsubscribe = () => void;

export interface GatewayMirrorServices {
  api: DesktopApi;                       // preload IPC surface
  now?: () => number;                    // injectable for tests
  schedule?: (task: () => void, delayMs: number) => number;
  cancelScheduled?: (id: number) => void;
}

export interface GatewayRootSnapshot {
  readonly version: number;
  readonly connection: ConnectionStatus | null;
  readonly desktopState: DesktopState | null;
  readonly loading: boolean;
  readonly error: string | null;
}

// One named snapshot type per domain instead of a stringly-typed
// getDomainSnapshot(domain): unknown. Typed accessors keep call sites
// honest and keep each slice's cache/version independent.
export interface CatalogSnapshot {
  readonly version: number;
  readonly agents: readonly DesktopCustomAgent[];
  readonly teams: readonly DesktopTeam[];
  readonly workflows: readonly DesktopWorkflowDefinition[];
  readonly providerModelsByType: Readonly<
    Record<string, DesktopProviderModels | null>
  >;
}

export interface ThreadFrontierSnapshot {
  readonly committedSeq: number;      // advanced only by applied committed events
  readonly renderBasedOnSeq: number;  // advanced by accepted render_state
  readonly renderFloor: number;
  readonly streamStatus: LiveStreamStatus | null;
  readonly consumers: Readonly<Record<string, ConsumerStreamStatus>>;
}

export interface ThreadMirrorSnapshot {
  readonly version: number;
  readonly threadId: string;
  readonly messages: readonly UiTranscriptMessage[];
  readonly messagesBySeq: ReadonlyMap<number, UiTranscriptMessage>;
  readonly renderState: RenderState | null;
  readonly threadInfo: ThreadRuntimeInfo | null;
  readonly historyPagination: ThreadHistoryPaginationState | null;
  readonly liveStream: LiveStreamState | null;
  readonly pendingRemoteInputs: readonly PendingThreadInput[];
  readonly pendingAutomationRun: PendingAutomationRun | null;
  readonly queue: readonly MessageIntent[];
  readonly frontier: ThreadFrontierSnapshot;
}

export interface GatewayMirror {
  subscribeRoot(listener: () => void): Unsubscribe;
  subscribeCatalog(listener: () => void): Unsubscribe;
  subscribeThread(threadId: string, listener: () => void): Unsubscribe;

  getRootSnapshot(): GatewayRootSnapshot;
  getCatalogSnapshot(): CatalogSnapshot;
  getThreadSnapshot(threadId: string): ThreadMirrorSnapshot;

  refreshDesktopState(options?: { silent?: boolean }): Promise<DesktopState>;
  observeConnection(status: ConnectionStatus | null, reason?: string | null): void;
  loadThreadHistory(threadId: string, options?: HistoryLoadOptions): Promise<void>;
  startThreadStream(threadId: string, options: {
    consumerId: string;
    afterSeq?: number | null;
    renderFloor?: number | null;
  }): Promise<void>;
  stopThreadStream(threadId: string, consumerId: string): Promise<void>;
  ingestChatStreamEvent(event: DesktopChatStreamEvent): void;

  openThread(threadId: string): Promise<OpenThreadResult>;
  sendIntent(input: SendIntentInput): Promise<SendIntentResult>;
  runQueuedBatch(threadId: string, initialIntentId?: string): Promise<void>;
  steerQueuedIntent(intentId: string, options?: { canSteer?: boolean }): Promise<void>;
  interruptThread(threadId: string): Promise<void>;
}
```

**`SendIntentInput` boundary**: attachment *preparation* (file pickers, drag
data, `File` objects, base64 reads via `prepareAttachmentUploads`) stays in the
UI layer — it is DOM-coupled. The mirror receives already-serialized attachment
payloads plus the prompt text. This keeps the mirror runnable in plain node
tests with no DOM shims.

**`getSnapshot` stability rule (hard requirement)**: every getter returns a
cached object reference reused until that slice's version changes.
`getThreadSnapshot` keeps a per-thread cached snapshot and rebuilds only when
one of that thread's input versions bumps. A test in the contract harness calls
`getSnapshot` repeatedly with no mutations and asserts reference equality —
violating this makes `useSyncExternalStore` loop.

React bindings stay thin:

```ts
export function useGatewayRoot(): GatewayRootSnapshot;
export function useCatalog(): CatalogSnapshot;
export function useThreadMirror(threadId: string | null): ThreadMirrorSnapshot | null;
```

No generic selector hook in the first iteration; purpose-built per-slice hooks
avoid the `useSyncExternalStoreWithSelector` dependency and its equality
pitfalls. Revisit only if selector sprawl becomes real.

## Stream and frontier semantics

The mirror consumes `DesktopChatStreamEvent` objects; Electron main keeps
owning HTTP, auth, `Last-Event-ID`, raw SSE parsing, and gap errors
(`main/gary-client/stream.ts` performs gap detection per committed inner event
and treats the SSE `id:` line as informational).

Per thread the mirror tracks two frontiers, mirroring that split:

- `committedSeq` — advanced only by applied committed `events`; this is the
  safe reconnect `afterSeq`.
- `renderBasedOnSeq` — advanced by accepted `render_state.based_on_seq`;
  guards stale snapshots. Snapshot-only frames may advance it without touching
  `committedSeq`.

A `thread_render_frame` is applied as **one synchronous mirror commit**:
committed events, then render snapshot, then a single notify. Subscribers can
never observe half a frame — this also retires the effect-ordering proofs that
batches 4-6 needed.

`renderFloor` is a per-consumer client request; fewer rows under a non-zero
floor mean a windowed view, not deletion. During implementation the desktop
`RenderState` type gains the server's optional `window` field (Rust already
serializes it; this is a desktop type alignment, not a contract change), and
the renderer IPC input type gains an optional `renderFloor` (local client type
extension, default 0).

## Controller destination map

All 11 current controllers (8 from T13 plus 3 earlier ones):

| Controller | End state |
| --- | --- |
| useTranscriptController (1917) | **Dissolved into mirror.** Stream ingestion, history convergence, render snapshot apply, pending inputs, thread info, pagination → `GatewayMirror`. Pure cache helpers → `gateway-mirror/transcript-cache.ts`. React keeps `useThreadMirror` + render-view mapping. |
| useMessageDispatchController (1714) | **Split.** Message machine, send/steer/interrupt, queue drain, runtime convergence → mirror. Composer draft/attachments/drag state → `ComposerSurface`/`ComposerQueue` (colocated). |
| useGatewayConnectionController (526) | **Dissolved into mirror** (polling, status observation, refresh, recovery scheduling). Gateway-setup form state stays local to its panel. |
| useDeepLinkRouteController (379) | **Replaced** by `DesktopRouteStore` + a small `RouteEffectBridge`. The bridge owns two inputs: (a) route-store changes (hash/popstate), and (b) the `garyx://` deep-link IPC channel (`subscribeDeepLinks`) — an external command stream that is neither hash state nor pure mirror state. Bridge translations: `open-thread`/`resume-session` → `mirror.openThread` (+ the existing gateway-readiness retry ladder from `waitForGatewayReadyForDeepLink`, which moves into the bridge) then `routeStore.navigate`; `new-thread`/`open-capsule` → `navigate` with route params. No writable `contentView`. |
| useSideChatController (1058) | Side-thread mapping + side composer state colocate into `SideChatPanel`; it subscribes `useThreadMirror(sideThreadId)` and calls mirror actions. The two reverse couplings T13 documented (sendIntentOnce reads sideChatThreadIdsRef; rewrite refetch reads the consumer id) become plain mirror state, dissolving both. |
| useLayoutResizeController (477) | Keep hook shape, colocate under `AppLayoutFrame`/`ThreadLayout`. Not a mirror concern. |
| useMessagesScrollController (277) | Keep hook shape, colocate under `ThreadTranscriptViewport`. |
| useMemoryDialogController (243) | Keep hook shape, colocate under `MemoryDialogRoot`. |
| useAutomationController | Keep as `AutomationPanel` controller; desktop-state reads become catalog/root subscriptions; navigation goes through route store. |
| useSettingsController (819) | Keep as `SettingsPanel` controller; drafts stay local; persist/refresh call mirror services. |
| useWorkspaceController (337) | Keep as `WorkspaceFilesPanel` controller; directory/preview state stays UI-local. |

T13's TDZ stay-behinds all dissolve: `runQueuedBatch`/`steerQueuedIntent`
become mirror methods (no render-phase consumption), `ensureThreadOpenable`
becomes `mirror.openThread`, `connection` lives in the root snapshot.

## Route as URL single source of truth

Today the route is triple-written: AppShell seeds `selectedThreadId` and
`contentView` separately, `applyDesktopRoute` writes React state from the hash,
a second effect rewrites the hash from state, and a fallback effect silently
converges unknown/non-selected threads back to `desktopState.threads[0]`. That
chain is the root cause of the known quirk where an externally-changed hash
gets rewritten to the previously selected thread (verified identical on the
pre-T13 baseline during batch-6 acceptance).

End state:

- `DesktopRouteStore.getSnapshot().route` is the only route state; contentView,
  selected thread/automation/workflow-task/capsule ids, and new-thread params
  are selectors.
- `navigate(route, { replace })` is the only hash writer. External `hashchange`
  updates the store first; route effects may load data or surface an error but
  never rewrite the hash unless they explicitly navigate.
- Unknown `#/thread/<id>` stays addressable while `mirror.openThread` retries
  through `refreshDesktopState`/`getThreadHistory` (today's
  `ensureThreadOpenable` behavior); on failure it renders an error state
  instead of silently selecting the first thread.

### Intentional behavior changes (user-visible, sign-off required)

1. External/manual hash edits take real effect instead of being converged back
   to the selected thread.
2. Unknown `#/thread/<id>`: today `selectExistingThreadInPlace` sets
   `Thread not found: <id>` (`AppShell.tsx:2459`) **and the state-to-hash
   effect then rewrites the URL back to the currently selected thread**. New
   behavior keeps the error state but leaves the entered hash in place as an
   addressable error route (no rewrite). Note: the `threads[0]` default
   selection lives in the `thread-home` (`#/`) branch and is intentional
   default behavior — it is **not** changed by this design.
3. `contentView` sessionStorage persistence is removed; the hash (which
   Electron already restores per window) is the only persisted route.

These are quirk removals, not regressions, but they change observable behavior
and are called out for review sign-off.

## Local state colocation list

- ComposerSurface: composer draft/resetKey/textPresent/images/files/
  browserAnnotations/attachmentUploadCount + textarea/input refs + submit locks.
- ComposerQueue: draggedQueueIntentId, queueDropTarget.
- SideChatPanel: sideComposerBySource, side attachment counts,
  sideChatThreadBySource/CreatingBySource/ErrorBySource, historyLoading, side refs.
- AppLayoutFrame/ThreadLayout: sidebar/rail/side-tools/thread-logs widths and
  resize flags.
- ThreadTranscriptViewport: scroll anchors/stickiness refs.
- MemoryDialogRoot: dialog target/draft/dirty/loading/saving/error + overlay
  pause effect.
- SettingsPanel: drafts, command/MCP loading flags, active tab.
- WorkspaceFilesPanel: directory expansion, preview, upload pending.
- ConversationHeaderTitle: title edit state.
- ThreadLogPanel: log text/path/cursor/loading/unread.
- Bot management: add-bot dialog state.
- ToastProvider: toasts, with a separate stable ToastActions context.

## Context injection

Contexts carry only stable identities (constructed once, never snapshots):
`GatewayMirrorContext`, `DesktopRouteStoreContext`, `ToastActionsContext`,
optional `DesktopServicesContext` (IPC api, clock/scheduler for tests). Zero
re-render cost by construction; volatile data enters exclusively through
`useSyncExternalStore`.

## React concurrency and tearing

- `useSyncExternalStore` for all mirror/route reads (built into the pinned
  React version; tearing-safe by design).
- Frame application is a synchronous atomic commit (see above); components
  needing consistent thread data subscribe to one `ThreadMirrorSnapshot`, never
  to parallel slices.
- `useTransition` wraps navigation-triggered UI transitions (thread open,
  panel switch, load-older initiation). Never used to reorder SSE ingestion.
- `useDeferredValue` only on immutable composite snapshots (e.g. the transcript
  row-mapping input `{ version, messagesBySeq, renderState, optimisticUsers }`),
  never on parallel independent values. Composer controls and interrupt/steer
  affordances read the non-deferred snapshot.

## Migration plan

Each batch merges independently; no long-lived fork of AppShell (it remains
high-churn). Additive modules + temporary adapters, legacy deletion last.

| Batch | Scope | Validation | Rollback |
| --- | --- | --- | --- |
| 0. Contract harness | No-React mirror test fixtures. Frame sources: (a) electron-smoke mock-gateway frames are real `thread_render_frame` wire envelopes and can be replayed as-is; (b) `test-fixtures/render-layer/render-state-cases.json` is a **reducer case set** (`records` ledger → expected `RenderState`), not wire frames — reusing it for mirror ingestion requires wrapping each case into a synthesized frame envelope (events + render_state); that wrapping cost belongs to this batch. Lock getSnapshot reference stability, per-thread notify isolation, monotonic render apply, frontier separation. | New mirror tests + existing render-view-model/thread-activity/desktop-route suites. | Delete additive tests. |
| 1. Mirror shell + root/catalog domains | `GatewayMirror` class, root+catalog snapshots, contexts, AppShell adapter feeding legacy props. desktopState/connection/agents/teams/workflows/providerModels refresh moves in. | `npm run test:unit`, `npm run build:ui`. | Revert adapter wiring. |
| 2. Thread transcript domain | Transcript caches, stream ingestion, render snapshots, pending inputs, thread info, pagination → mirror. `useThreadMirror` feeds the same view-model inputs. | **Dual-run comparison**: same recorded event sequences into legacy helpers and mirror, assert identical messagesByThread/renderState/pending/pagination. Plus unit suite + build:ui. | Adapter flag returns ThreadPage to legacy controller. |
| 3. Dispatch + queue domain | Message machine, send/steer/interrupt, queue drain, runtime convergence → mirror. Composer draft stays put for now. | Message-machine + pending-inputs + thread-activity suites; dual-run comparison strictly on **recorded ack/event sequences** — replay the same recorded gateway responses into legacy and mirror state machines and compare transitions; never live-double-send (dispatch has real gateway side effects). CDP script: send, queued follow-up, steer, interrupt. | Adapter routes composer actions back to legacy hook. |
| 4. Route store | `DesktopRouteStore`; contentView becomes selector; `applyDesktopRoute` → route effects; fallback hash rewrite removed (intentional changes above). | desktop-route + main deep-link suites; CDP route script incl. unknown-thread hash. | Route store read-only; re-enable legacy state-to-hash effect. |
| 5. Local colocation | Composer/queue-drag/side-chat/scroll/layout/dialogs/toasts/logs into feature roots; AppShell becomes providers + route outlet + layout (~≤800 lines). | Unit suite, build:ui; renderer render-count probe confirming thread frames no longer re-render unrelated panels. | Revert one feature root at a time. |
| 6. Legacy deletion | Delete dissolved T13 hooks and adapter shims. | Full: test:unit, build:ui, test:smoke. | Restore adapter from prior commit. |
| 7. Packaged proof | After renderer/preload-affecting batches: `npm run dist:dir`, restart installed app, CDP transcript/route/composer smoke. | Packaged checklist per `docs/agents/validation.md`. | Revert last batch. |

**Behavior-equivalence confidence (replacing T13's byte-identical oracle)**:
(1) contract tests over recorded frames; (2) dual-run comparison while legacy
helpers still exist — same inputs, compare externally observable snapshots;
(3) CDP scenario scripts against the dev app (the T13 acceptance scripts —
side-chat send, queue drain, thread switch, pagination, resize — become the
regression playbook); (4) the existing unit + smoke matrix stays green per
batch.

## Dependency decision

Zero new npm dependencies. `useSyncExternalStore` ships with the pinned React.
Zustand/Jotai/Redux are rejected: they would not solve the actual hard parts
(render_state compliance, frontier separation, route ownership,
dispatch migration), add a public dependency, and still require the same
contract tests. Reconsider only if the hand-rolled selector/cache layer
demonstrably becomes a maintenance burden.

## Risks and tradeoffs

- Snapshot getters allocating per call → render loops. Mitigated by the
  stability rule + harness test.
- Mirror drifting into a second reducer. Mitigated by ownership fences above;
  row production stays in `render-view-model.ts`.
- Dispatch before transcript would be unprovable; order fixed (2 before 3).
- Route-truth change exposes previously-hidden unknown-thread errors —
  intentional, listed for sign-off.
- High-churn AppShell during migration. Mitigated by additive batches and
  adapters; each batch merges within a day-scale window like T13 batches did.
  Operationally: rebase onto latest main before starting each batch, and
  re-scan for remaining adapter references before batch 6 deletes the legacy
  paths.
- Dual-run harness is temporary scaffolding; deleted in batch 6 to avoid
  bit-rot.

## Contract compliance

Gateway keeps emitting `thread_render_frame { events, render_state }`; main
keeps parsing/gap-checking; the renderer mirror stores and distributes. The
mirror caches committed bodies and server RenderState, keeps optimistic local
user rows and pending-ack chrome (explicitly permitted by
`docs/agents/repository-contracts.md`), and never: groups user turns, groups
tool calls, places final answers, fabricates tail thinking/active tool groups,
or treats `LiveStreamStatus` as a render source.
