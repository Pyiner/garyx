// GatewayMirror: the renderer-side pure-TypeScript mirror of gateway state.
//
// Design: docs/design/appshell-endgame-architecture.md. The mirror is a
// facade over three pinned internal modules (transcript-cache,
// dispatch-machine, frontier). Batch 0 ships the thread-domain contract
// core: frame ingestion as one synchronous atomic commit, per-thread
// subscription isolation, monotonic render-state acceptance, and
// committed/render frontier separation. Root/catalog domains land in
// batch 1; dispatch-machine lands in batch 3; nothing here is wired into
// AppShell yet.
//
// Hard rule (useSyncExternalStore compatibility): every snapshot getter
// returns a cached object reference that is reused until that slice's
// version changes. Allocating per call would make subscribers loop.

import type {
  CommittedMessageEvent,
  ConnectionStatus,
  DesktopChatStreamEvent,
  DesktopCustomAgent,
  DesktopState,
  DesktopTeam,
  DesktopWorkflowDefinition,
  PendingThreadInput,
  RenderState,
  ThreadRuntimeInfo,
  ThreadTranscript,
} from "@shared/contracts";

import type {
  MessageIntent,
  MessageMachineAction,
  MessageMachineState,
} from "../message-machine.ts";
import type { LiveStreamState, UiTranscriptMessage } from "../app-shell/types";
import { DispatchMachine } from "./dispatch-machine.ts";
import { DispatchOrchestrator } from "./dispatch-orchestrator.ts";
import type {
  DispatchOrchestratorDeps,
  SeededTurn,
} from "./dispatch-orchestrator.ts";
import { ThreadFrontier } from "./frontier.ts";
import type { ThreadFrontierSnapshot } from "./frontier.ts";
import { ThreadTranscriptCache } from "./transcript-cache.ts";
import {
  THREAD_HISTORY_PAGE_SIZE,
  THREAD_HISTORY_USER_QUERY_LIMIT,
} from "./transcript-materialize.ts";
import type { ThreadHistoryPaginationState } from "./transcript-materialize.ts";

export type Unsubscribe = () => void;

function connectionEquals(
  a: ConnectionStatus | null,
  b: ConnectionStatus | null,
): boolean {
  if (a === b) {
    return true;
  }
  if (!a || !b) {
    return false;
  }
  return (
    a.ok === b.ok &&
    a.bridgeReady === b.bridgeReady &&
    a.gatewayUrl === b.gatewayUrl &&
    (a.error ?? null) === (b.error ?? null)
  );
}

/**
 * The IPC surface the mirror needs, injected so the class stays pure
 * TypeScript and testable without a preload bridge.
 */
export interface GatewayMirrorServices {
  getState(): Promise<DesktopState>;
  listCustomAgents(): Promise<DesktopCustomAgent[]>;
  listTeams(): Promise<DesktopTeam[]>;
  listWorkflowDefinitions(): Promise<DesktopWorkflowDefinition[]>;
  getThreadHistory(input: {
    threadId: string;
    beforeIndex: number;
    limit: number;
    userQueryLimit: number;
  }): Promise<ThreadTranscript>;
  /**
   * Message-machine intent lookup. Temporary seam: the machine stays with
   * its legacy owner until batch 3, so the wiring layer passes a lookup
   * into the machine's live state. Omitted (tests, machine-less mirrors)
   * means the transcript merge runs remote-only.
   */
  intentForId?(intentId: string): MessageIntent | null;
  /**
   * Called when a committed rewrite/reset control demands an authoritative
   * transcript refetch. The refetch itself (cache clear + full history
   * fetch + stream restart) stays with the legacy owner until batch 2b.
   */
  requestAuthoritativeRefetch?(threadId: string): void;
}

export interface GatewayRootSnapshot {
  readonly version: number;
  readonly connection: ConnectionStatus | null;
  readonly desktopState: DesktopState | null;
}

export interface CatalogSnapshot {
  readonly version: number;
  readonly agents: readonly DesktopCustomAgent[];
  readonly teams: readonly DesktopTeam[];
  readonly workflows: readonly DesktopWorkflowDefinition[];
}

export interface ThreadMirrorSnapshot {
  readonly version: number;
  readonly threadId: string;
  /**
   * Committed events verbatim, seq-ordered. Contract tests depend only on
   * version/reference/monotonicity semantics.
   */
  readonly records: readonly CommittedMessageEvent[];
  /**
   * Mapped UI transcript messages (authoritative apply, remote apply, and
   * committed stream mapping — batch 2a-2).
   */
  readonly messages: readonly UiTranscriptMessage[];
  readonly threadInfo: ThreadRuntimeInfo | null;
  readonly pendingRemoteInputs: readonly PendingThreadInput[];
  readonly renderState: RenderState | null;
  readonly historyPagination: ThreadHistoryPaginationState | null;
  /**
   * Per-thread live-stream transport state (batch 3c-1). Dispatch and
   * stream-lifecycle orchestration read and write it through the mirror.
   */
  readonly liveStream: LiveStreamState | null;
  /**
   * True once at least one authoritative/remote transcript apply has
   * landed for this thread (batch 3d). Read-side consumers use it to
   * reproduce the legacy "threadInfo key exists" loaded gate — a thread
   * can have live-stream or committed-event entries before its first
   * transcript fetch resolves.
   */
  readonly transcriptLoaded: boolean;
  readonly frontier: ThreadFrontierSnapshot;
}

/**
 * Aggregate per-thread transcript maps in the exact legacy AppShell state
 * shapes (batch 3d read-side cutover). Key-existence semantics reproduce
 * the legacy write paths:
 * - messagesByThread: key present when the thread has any UI messages
 *   (legacy read sites use `map[id] || []`; the 3b delete-key bridge
 *   syncs an empty array, which drops the key here).
 * - renderStateByThread / historyPaginationByThread: key present when
 *   non-null (legacy reads are `map[id] || null`).
 * - threadInfoByThread: key present once a transcript apply landed —
 *   the value may be null; key existence itself is the legacy
 *   "threadInfo loaded" gate (hasOwnProperty consumer in AppShell).
 * - pendingRemoteInputsByThread: key present when non-empty (the legacy
 *   setRemotePendingInputs deleted the key for empty arrays).
 */
export interface TranscriptMapsSnapshot {
  readonly messagesByThread: Record<string, readonly UiTranscriptMessage[]>;
  readonly renderStateByThread: Record<string, RenderState>;
  readonly threadInfoByThread: Record<string, ThreadRuntimeInfo | null>;
  readonly historyPaginationByThread: Record<
    string,
    ThreadHistoryPaginationState
  >;
  readonly pendingRemoteInputsByThread: Record<
    string,
    readonly PendingThreadInput[]
  >;
}

interface ThreadEntry {
  cache: ThreadTranscriptCache;
  frontier: ThreadFrontier;
  liveStream: LiveStreamState | null;
  listeners: Set<() => void>;
  version: number;
  snapshot: ThreadMirrorSnapshot | null;
}

export class GatewayMirror {
  private threads = new Map<string, ThreadEntry>();
  private services: GatewayMirrorServices | null;

  // Root domain: connection + desktop state.
  private connection: ConnectionStatus | null = null;
  private desktopState: DesktopState | null = null;
  private rootVersion = 0;
  private rootSnapshot: GatewayRootSnapshot | null = null;
  private rootListeners = new Set<() => void>();

  // Catalog domain: agents / teams / workflow definitions.
  private agents: readonly DesktopCustomAgent[] = [];
  private teams: readonly DesktopTeam[] = [];
  private workflows: readonly DesktopWorkflowDefinition[] = [];
  private catalogVersion = 0;
  private catalogSnapshot: CatalogSnapshot | null = null;
  private catalogListeners = new Set<() => void>();

  // Dispatch-machine domain (batch 3a): message-machine state storage.
  private machine = new DispatchMachine();

  // Dispatch-orchestration domain (batch 3c-2): send/steer/interrupt and
  // the queued-batch drain. Deps are attached by the UI layer on every
  // React commit (setDispatchDeps) until the remaining seams dissolve in
  // batches 5/6.
  private dispatchOrchestrator = new DispatchOrchestrator();

  // Live-stream domain (batch 3c-1): per-thread transport state storage.
  // The aggregate map mirrors the legacy `liveStreamStateByThread` React
  // state: one Record identity, rebuilt as a whole on every update (the
  // legacy updater always allocated a fresh Record and re-rendered), so
  // subscribers see exactly the legacy notification cadence.
  private liveStreamMap: Record<string, LiveStreamState> = {};
  private liveStreamListeners = new Set<() => void>();

  // Transcript-maps domain (batch 3d): the aggregate read-side view over
  // every thread entry, in the legacy AppShell state shapes. Rebuilt
  // lazily on first read after any thread commit; reference-stable
  // otherwise (uSES hard rule).
  private transcriptMapsSnapshot: TranscriptMapsSnapshot | null = null;
  private transcriptMapsListeners = new Set<() => void>();

  constructor(services?: GatewayMirrorServices) {
    this.services = services ?? null;
  }

  subscribeRoot(listener: () => void): Unsubscribe {
    this.rootListeners.add(listener);
    return () => {
      this.rootListeners.delete(listener);
    };
  }

  subscribeCatalog(listener: () => void): Unsubscribe {
    this.catalogListeners.add(listener);
    return () => {
      this.catalogListeners.delete(listener);
    };
  }

  getRootSnapshot(): GatewayRootSnapshot {
    if (!this.rootSnapshot) {
      this.rootSnapshot = {
        version: this.rootVersion,
        connection: this.connection,
        desktopState: this.desktopState,
      };
    }
    return this.rootSnapshot;
  }

  getCatalogSnapshot(): CatalogSnapshot {
    if (!this.catalogSnapshot) {
      this.catalogSnapshot = {
        version: this.catalogVersion,
        agents: this.agents,
        teams: this.teams,
        workflows: this.workflows,
      };
    }
    return this.catalogSnapshot;
  }

  /** Record a connection status observation (poll/setup/error paths). */
  observeConnection(status: ConnectionStatus | null): void {
    // Shallow value comparison: the healthy poll delivers a fresh object
    // with identical content every cycle; bumping root for those would
    // re-render every root subscriber for nothing.
    if (connectionEquals(status, this.connection)) {
      return;
    }
    this.connection = status;
    this.bumpRoot();
  }

  /**
   * Fetch the desktop root state plus the agent/team/workflow catalogs in
   * one round (the legacy refreshDesktopState behavior: catalog fetches are
   * individually best-effort). Updates root/catalog snapshots atomically
   * per domain and returns the fresh DesktopState for callers that need it.
   */
  async refreshDesktopState(): Promise<DesktopState> {
    if (!this.services) {
      throw new Error("GatewayMirror constructed without services");
    }
    const [nextState, nextAgents, nextTeams, nextWorkflows] = await Promise.all([
      this.services.getState(),
      this.services.listCustomAgents().catch(() => [] as DesktopCustomAgent[]),
      this.services.listTeams().catch(() => [] as DesktopTeam[]),
      this.services
        .listWorkflowDefinitions()
        .catch(() => [] as DesktopWorkflowDefinition[]),
    ]);
    this.desktopState = nextState;
    this.bumpRoot();
    this.agents = nextAgents;
    this.teams = nextTeams;
    this.workflows = nextWorkflows;
    this.bumpCatalog();
    return nextState;
  }

  private bumpRoot(): void {
    this.rootVersion += 1;
    this.rootSnapshot = null;
    for (const listener of [...this.rootListeners]) {
      listener();
    }
  }

  private bumpCatalog(): void {
    this.catalogVersion += 1;
    this.catalogSnapshot = null;
    for (const listener of [...this.catalogListeners]) {
      listener();
    }
  }

  subscribeMachine(listener: () => void): Unsubscribe {
    return this.machine.subscribe(listener);
  }

  getMachineState(): MessageMachineState {
    return this.machine.getState();
  }

  /**
   * Apply one message-machine action (batch 3a: the mirror owns machine
   * state storage; the reducer stays in message-machine.ts). Returns the
   * committed post-dispatch state for callers that need it synchronously.
   */
  dispatchMachineAction(action: MessageMachineAction): MessageMachineState {
    return this.machine.dispatch(action);
  }

  /** Attach (or refresh) the dispatch-orchestration deps. */
  setDispatchDeps(deps: DispatchOrchestratorDeps): void {
    this.dispatchOrchestrator.setDeps(deps);
  }

  appendSeededTurn(
    threadId: string,
    intent: MessageIntent,
    options?: { seedUserBubble?: boolean },
  ): SeededTurn {
    return this.dispatchOrchestrator.appendSeededTurn(threadId, intent, options);
  }

  sendIntentOnce(
    threadId: string,
    intentId: string,
    options?: { seedUserBubble?: boolean; seededTurn?: SeededTurn },
  ): Promise<boolean> {
    return this.dispatchOrchestrator.sendIntentOnce(threadId, intentId, options);
  }

  runQueuedBatch(threadId: string, initialIntentId?: string): Promise<void> {
    return this.dispatchOrchestrator.runQueuedBatch(threadId, initialIntentId);
  }

  steerQueuedIntent(
    latestIntent: MessageIntent,
    options?: { canSteer?: boolean },
  ): Promise<void> {
    return this.dispatchOrchestrator.steerQueuedIntent(latestIntent, options);
  }

  interruptThread(threadId: string | null | undefined): Promise<void> {
    return this.dispatchOrchestrator.interruptThread(threadId);
  }

  subscribeLiveStreams(listener: () => void): Unsubscribe {
    this.liveStreamListeners.add(listener);
    return () => {
      this.liveStreamListeners.delete(listener);
    };
  }

  subscribeTranscriptMaps(listener: () => void): Unsubscribe {
    this.transcriptMapsListeners.add(listener);
    return () => {
      this.transcriptMapsListeners.delete(listener);
    };
  }

  /**
   * Aggregate transcript maps in the legacy AppShell state shapes (batch
   * 3d). Cached by reference; rebuilt lazily after any thread commit.
   * Key-existence semantics per TranscriptMapsSnapshot's contract.
   */
  getTranscriptMapsSnapshot(): TranscriptMapsSnapshot {
    if (!this.transcriptMapsSnapshot) {
      const messagesByThread: Record<string, readonly UiTranscriptMessage[]> =
        {};
      const renderStateByThread: Record<string, RenderState> = {};
      const threadInfoByThread: Record<string, ThreadRuntimeInfo | null> = {};
      const historyPaginationByThread: Record<
        string,
        ThreadHistoryPaginationState
      > = {};
      const pendingRemoteInputsByThread: Record<
        string,
        readonly PendingThreadInput[]
      > = {};
      for (const [threadId, entry] of this.threads) {
        const messages = entry.cache.getUiMessages();
        if (messages.length > 0) {
          messagesByThread[threadId] = messages;
        }
        const renderState = entry.cache.getRenderState();
        if (renderState) {
          renderStateByThread[threadId] = renderState;
        }
        if (entry.cache.isTranscriptLoaded()) {
          threadInfoByThread[threadId] = entry.cache.getThreadInfo();
        }
        const pagination = entry.cache.getHistoryPagination();
        if (pagination) {
          historyPaginationByThread[threadId] = pagination;
        }
        const pendingInputs = entry.cache.getPendingRemoteInputs();
        if (pendingInputs.length > 0) {
          pendingRemoteInputsByThread[threadId] = pendingInputs;
        }
      }
      this.transcriptMapsSnapshot = {
        messagesByThread,
        renderStateByThread,
        threadInfoByThread,
        historyPaginationByThread,
        pendingRemoteInputsByThread,
      };
    }
    return this.transcriptMapsSnapshot;
  }

  /**
   * Aggregate live-stream map, cached by reference: the getter never
   * allocates; the map identity changes exactly once per applied update
   * (legacy setState cadence).
   */
  getLiveStreamMap(): Record<string, LiveStreamState> {
    return this.liveStreamMap;
  }

  getThreadLiveStream(threadId: string): LiveStreamState | null {
    return this.threads.get(threadId)?.liveStream ?? null;
  }

  /**
   * Update one thread's live-stream state (batch 3c-1: the mirror owns
   * live-stream storage; orchestration migrates in 3c-2). Verbatim legacy
   * semantics: the updater receives the current entry (or null), a null
   * result deletes the entry, and the aggregate map is rebuilt and
   * re-notified on every call — even a no-change one — matching the
   * legacy React setState behavior. Returns the next entry.
   */
  updateThreadLiveStream(
    threadId: string,
    updater: (current: LiveStreamState | null) => LiveStreamState | null,
  ): LiveStreamState | null {
    const entry = this.threadEntry(threadId);
    const next = updater(entry.liveStream);
    entry.liveStream = next;
    const updated = { ...this.liveStreamMap };
    if (next) {
      updated[threadId] = next;
    } else {
      delete updated[threadId];
    }
    this.liveStreamMap = updated;
    this.commitThread(entry, false);
    this.notifyLiveStreams();
    return next;
  }

  clearThreadLiveStream(threadId: string): void {
    this.updateThreadLiveStream(threadId, () => null);
  }

  /**
   * Move a live-stream entry between thread ids in one commit — the
   * new-thread draft promotion path. Verbatim legacy semantics: no-op when
   * the source has no entry; otherwise the entry lands under the target id
   * with its `threadId` field rewritten, and the aggregate map changes
   * identity once.
   */
  replaceLiveStreamThreadId(fromThreadId: string, toThreadId: string): void {
    const fromEntry = this.threadEntry(fromThreadId);
    const draft = fromEntry.liveStream;
    if (!draft) {
      return;
    }
    const toEntry = this.threadEntry(toThreadId);
    fromEntry.liveStream = null;
    toEntry.liveStream = {
      ...draft,
      threadId: toThreadId,
    };
    const updated = { ...this.liveStreamMap };
    delete updated[fromThreadId];
    updated[toThreadId] = toEntry.liveStream;
    this.liveStreamMap = updated;
    this.commitThread(fromEntry, false);
    this.commitThread(toEntry, false);
    this.notifyLiveStreams();
  }

  private notifyLiveStreams(): void {
    for (const listener of [...this.liveStreamListeners]) {
      listener();
    }
  }

  subscribeThread(threadId: string, listener: () => void): Unsubscribe {
    const entry = this.threadEntry(threadId);
    entry.listeners.add(listener);
    return () => {
      entry.listeners.delete(listener);
    };
  }

  getThreadSnapshot(threadId: string): ThreadMirrorSnapshot {
    const entry = this.threadEntry(threadId);
    if (!entry.snapshot) {
      entry.snapshot = {
        version: entry.version,
        threadId,
        records: entry.cache.sortedRecords(),
        messages: entry.cache.getUiMessages(),
        threadInfo: entry.cache.getThreadInfo(),
        pendingRemoteInputs: entry.cache.getPendingRemoteInputs(),
        renderState: entry.cache.getRenderState(),
        historyPagination: entry.cache.getHistoryPagination(),
        liveStream: entry.liveStream,
        transcriptLoaded: entry.cache.isTranscriptLoaded(),
        frontier: entry.frontier.snapshot(),
      };
    }
    return entry.snapshot;
  }

  /**
   * Apply an authoritative (canonical) transcript for a thread as one
   * synchronous commit: the mirror-side counterpart of the hook's
   * applyCanonicalTranscript cache path. Message-machine sync and cache
   * persistence remain with their legacy owners until batches 3/2b.
   */
  applyAuthoritativeTranscript(
    threadId: string,
    transcript: ThreadTranscript,
  ): void {
    const entry = this.threadEntry(threadId);
    entry.cache.applyAuthoritative(transcript);
    this.commitThread(entry);
  }

  /**
   * Apply a remote transcript (full fetch or forward aggregate) as one
   * synchronous commit: the mirror-side counterpart of the hook's
   * applyRemoteTranscript cache path. Run-state sync, IPC persistence,
   * desktopState propagation, and intent marking stay with their legacy
   * owners (batches 3/2b).
   */
  applyRemoteTranscript(threadId: string, transcript: ThreadTranscript): void {
    const entry = this.threadEntry(threadId);
    entry.cache.applyRemote(transcript, { intentForId: this.intentLookup });
    this.commitThread(entry);
  }

  /**
   * Sync a locally-mutated UI message array into the mirror as one
   * commit. Batch-3b bridge (deleted with the legacy path in batch 6):
   * optimistic and recovery writes still run through the legacy
   * updateMessagesByThread updater; the legacy result is mirrored here so
   * mirror messages stay converged including non-remote rows. Remote
   * applies must not use this — they keep the mirror computing its own
   * result through applyRemoteTranscript/applyAuthoritativeTranscript/
   * applyOlderHistoryPage.
   */
  syncThreadUiMessages(
    threadId: string,
    messages: readonly UiTranscriptMessage[],
  ): void {
    const entry = this.threadEntry(threadId);
    entry.cache.setUiMessages(messages);
    this.commitThread(entry);
  }

  /**
   * Apply an already-fetched older history page as one commit. Batch-2b
   * dual-write entry: while the legacy hook still owns the older-page
   * fetch, it feeds the fetched page here so the mirror stays converged.
   * Once the mirror owns the fetch (loadOlderThreadHistoryPage below),
   * this remains the shared apply step.
   */
  applyOlderHistoryPage(threadId: string, transcript: ThreadTranscript): void {
    const entry = this.threadEntry(threadId);
    entry.cache.applyOlderPage(transcript);
    this.commitThread(entry);
  }

  /**
   * Bridge the legacy older-page fetch's loadingBefore lifecycle into the
   * mirror (batch 3d). The legacy hook still owns the fetch until batch 6;
   * loadingBefore is the one pagination field it mutates that the mirror
   * cannot derive from applied transcripts. No-op when the thread has no
   * pagination yet or the flag already matches.
   */
  setThreadHistoryLoadingBefore(threadId: string, loadingBefore: boolean): void {
    const entry = this.threadEntry(threadId);
    const current = entry.cache.getHistoryPagination();
    if (!current || current.loadingBefore === loadingBefore) {
      return;
    }
    entry.cache.setHistoryPagination({ ...current, loadingBefore });
    this.commitThread(entry);
  }

  /**
   * Load one older history page for a thread: the mirror-side counterpart
   * of the hook's loadOlderThreadHistoryPage. Guards on the thread's
   * pagination state, marks loadingBefore for the duration of the fetch,
   * and prepends the fetched page. `onPageFetched` runs between the fetch
   * and the apply so the UI layer can capture its scroll-anchor state
   * (scroll anchoring is UI-owned by design). Fetch errors clear the
   * loading flag and propagate to the caller.
   */
  async loadOlderThreadHistoryPage(
    threadId: string,
    options?: { onPageFetched?: (transcript: ThreadTranscript) => void },
  ): Promise<void> {
    if (!this.services) {
      throw new Error("GatewayMirror constructed without services");
    }
    const entry = this.threadEntry(threadId);
    const pagination = entry.cache.getHistoryPagination();
    if (
      !pagination?.hasMoreBefore ||
      pagination.loadingBefore ||
      pagination.nextBeforeIndex === null
    ) {
      return;
    }

    entry.cache.setHistoryPagination({
      hasMoreBefore: pagination.hasMoreBefore,
      nextBeforeIndex: pagination.nextBeforeIndex,
      loadingBefore: true,
    });
    this.commitThread(entry);

    try {
      const transcript = await this.services.getThreadHistory({
        threadId,
        beforeIndex: pagination.nextBeforeIndex,
        limit: THREAD_HISTORY_PAGE_SIZE,
        userQueryLimit: THREAD_HISTORY_USER_QUERY_LIMIT,
      });
      options?.onPageFetched?.(transcript);
      entry.cache.applyOlderPage(transcript);
    } finally {
      const current = entry.cache.getHistoryPagination();
      entry.cache.setHistoryPagination(
        current ? { ...current, loadingBefore: false } : current,
      );
      this.commitThread(entry);
    }
  }

  /**
   * Ingest one desktop chat-stream event. A `thread_render_frame` is applied
   * as ONE synchronous commit: committed events first, then the render
   * snapshot, then a single notification. Subscribers can never observe half
   * a frame. A bare `committed_message` is treated as a frame without a
   * render snapshot.
   */
  ingest(event: DesktopChatStreamEvent): void {
    if (event.type === "thread_render_frame") {
      this.applyFrame(event.threadId, event.events, event.renderState);
      return;
    }
    if (event.type === "committed_message") {
      this.applyFrame(event.threadId, [event], null);
    }
    // "error" events carry no mirrored state in batch 0.
  }

  private applyFrame(
    threadId: string,
    events: readonly CommittedMessageEvent[],
    renderState: RenderState | null,
  ): void {
    const entry = this.threadEntry(threadId);
    let changed = false;

    const appliedEvents = entry.cache.applyCommittedEvents(events);
    if (appliedEvents.length > 0) {
      let highestApplied = 0;
      for (const event of appliedEvents) {
        if (event.seq > highestApplied) {
          highestApplied = event.seq;
        }
      }
      entry.frontier.advanceCommitted(highestApplied);
      // Committed → UI-message mapping, mirroring the hook's per-event
      // applyCommittedThreadMessage loop. Only newly applied events map
      // (seq-idempotent redelivery stays a no-op). Rewrite/reset controls
      // skip the mapping and request an authoritative refetch instead.
      for (const event of appliedEvents) {
        const outcome = entry.cache.applyCommittedMessage(event, {
          intentForId: this.intentLookup,
        });
        if (outcome === "refetch_authoritative") {
          this.services?.requestAuthoritativeRefetch?.(threadId);
        }
      }
      changed = true;
    }

    if (renderState) {
      const verdict = entry.frontier.acceptRender(renderState.based_on_seq);
      if (verdict.accepted && verdict.changed) {
        entry.cache.setRenderState(renderState);
        changed = true;
      }
      // Rejected (stale) or unchanged (same based_on_seq) snapshots neither
      // bump the version nor notify: the server derives render_state
      // deterministically from the committed ledger.
    }

    if (!changed) {
      return;
    }
    this.commitThread(entry);
  }

  /**
   * One atomic thread commit: version bump, snapshot invalidation, notify.
   * `touchesTranscript: false` (the live-stream-only paths) skips the
   * aggregate transcript-maps invalidation so those map identities stay
   * stable across pure transport-state updates — matching the legacy
   * separation between liveStreamStateByThread and the 5 transcript
   * states.
   */
  private commitThread(entry: ThreadEntry, touchesTranscript = true): void {
    entry.version += 1;
    entry.snapshot = null;
    if (touchesTranscript) {
      this.transcriptMapsSnapshot = null;
    }
    for (const listener of [...entry.listeners]) {
      listener();
    }
    if (touchesTranscript) {
      for (const listener of [...this.transcriptMapsListeners]) {
        listener();
      }
    }
  }

  private intentLookup = (intentId: string): MessageIntent | null =>
    this.services?.intentForId?.(intentId) ?? null;

  private threadEntry(threadId: string): ThreadEntry {
    let entry = this.threads.get(threadId);
    if (!entry) {
      entry = {
        cache: new ThreadTranscriptCache(),
        frontier: new ThreadFrontier(),
        liveStream: null,
        listeners: new Set(),
        version: 0,
        snapshot: null,
      };
      this.threads.set(threadId, entry);
    }
    return entry;
  }
}
