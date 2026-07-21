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
  CachedThreadTranscript,
  CommittedMessageEvent,
  ConnectionStatus,
  DesktopAgentCatalog,
  DesktopChatStreamEvent,
  DesktopCustomAgent,
  DesktopState,
  GetThreadHistoryInput,
  PendingThreadInput,
  RenderState,
  StartThreadStreamInput,
  StopThreadStreamInput,
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
import { TranscriptLifecycle } from "./transcript-lifecycle.ts";
import type {
  TranscriptLifecycleDeps,
} from "./transcript-lifecycle.ts";
import {
  jsonValuesEqual,
  THREAD_HISTORY_PAGE_SIZE,
  THREAD_HISTORY_USER_QUERY_LIMIT,
} from "./transcript-materialize.ts";
import type { ThreadHistoryPaginationState } from "./transcript-materialize.ts";
import { NEW_THREAD_DRAFT_THREAD_ID } from "./thread-ids.ts";

export type Unsubscribe = () => void;

const DESKTOP_STATE_REFRESH_TRAILING_MS = 350;
export const GATEWAY_MIRROR_INACTIVE_THREAD_LIMIT = 32;

const EMPTY_COMMITTED_RECORDS: readonly CommittedMessageEvent[] = [];
const EMPTY_UI_MESSAGES: readonly UiTranscriptMessage[] = [];
const EMPTY_PENDING_REMOTE_INPUTS: readonly PendingThreadInput[] = [];
const EMPTY_THREAD_FRONTIER: ThreadFrontierSnapshot = {
  committedSeq: 0,
  renderBasedOnSeq: 0,
};

function renderStatesEqual(
  left: RenderState | null,
  right: RenderState,
): boolean {
  if (!left) {
    return false;
  }
  const { rows_hash: _leftRowsHash, ...leftValue } = left;
  const { rows_hash: _rightRowsHash, ...rightValue } = right;
  return jsonValuesEqual(leftValue, rightValue);
}

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
  listCustomAgents(): Promise<DesktopAgentCatalog>;
  /** Paged history fetch (older pages and the forward incremental fetch). */
  getThreadHistory(input: GetThreadHistoryInput): Promise<ThreadTranscript>;
  /**
   * Message-machine intent lookup. Temporary seam: the machine stays with
   * its legacy owner until batch 3, so the wiring layer passes a lookup
   * into the machine's live state. Omitted (tests, machine-less mirrors)
   * means the transcript merge runs remote-only.
   */
  intentForId?(intentId: string): MessageIntent | null;
  /**
   * Persist a thread transcript (plus its last render snapshot) into the
   * on-disk cache (slice 6b-2b: the apply chain's persist ride-along).
   * Optional: omitted in node tests.
   */
  saveThreadTranscriptCache?(
    scope: string,
    transcript: ThreadTranscript,
    renderState?: RenderState | null,
  ): Promise<void>;
  // Slice 6b-2c: the fetch/stream lifecycle's IPC. Optional as a group so
  // node tests can construct history-only mirrors; the port accessors
  // throw when a lifecycle path runs without them.
  getThreadHistoryFull?(threadId: string): Promise<ThreadTranscript>;
  startThreadStream?(input: StartThreadStreamInput): Promise<void>;
  stopThreadStream?(input: StopThreadStreamInput): Promise<void>;
  loadThreadTranscriptCache?(
    scope: string,
    threadId: string,
  ): Promise<CachedThreadTranscript | null>;
  clearThreadTranscriptCache?(scope: string, threadId: string): Promise<void>;
}

export interface GatewayRootSnapshot {
  readonly version: number;
  readonly connection: ConnectionStatus | null;
  readonly desktopState: DesktopState | null;
}

export type CatalogPhase = "loading" | "ready" | "error";

export interface CatalogSnapshot {
  readonly version: number;
  /** Full request state, not just content: consumers must be able to
   *  distinguish an EMPTY catalog (ready) from one that is still loading
   *  or whose latest request failed (agents then hold the last-known
   *  values, never a silently blank list). */
  readonly phase: CatalogPhase;
  readonly agents: readonly DesktopCustomAgent[];
  readonly defaultAgentId: string | null;
  readonly effectiveDefaultAgentId: string | null;
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

export interface ThreadStreamIngestOptions {
  /** False for a superseded logical request: committed events still apply,
   * while render/window/error ownership stays with the current request. */
  readonly applyConnectionScoped?: boolean;
}

export interface ThreadStreamIngestResult {
  readonly appliedEvents: readonly CommittedMessageEvent[];
  readonly renderAccepted: boolean;
  readonly renderChanged: boolean;
  readonly connectionScopedApplied: boolean;
}

interface ThreadEntry {
  threadId: string;
  cache: ThreadTranscriptCache;
  frontier: ThreadFrontier;
  liveStream: LiveStreamState | null;
  listeners: Set<() => void>;
  lastAccess: number;
  retainCount: number;
  version: number;
  snapshot: ThreadMirrorSnapshot | null;
}

export class GatewayMirror {
  private threads = new Map<string, ThreadEntry>();
  private emptyThreadSnapshots = new Map<string, ThreadMirrorSnapshot>();
  private threadAccessOrdinal = 0;
  private services: GatewayMirrorServices | null;

  // Root domain: connection + desktop state.
  private connection: ConnectionStatus | null = null;
  private desktopState: DesktopState | null = null;
  private rootVersion = 0;
  private rootSnapshot: GatewayRootSnapshot | null = null;
  private rootListeners = new Set<() => void>();
  private desktopStateRefreshInFlight: Promise<DesktopState> | null = null;
  private desktopStateRefreshTailRequested = false;
  private desktopStateRefreshTrailing: Promise<DesktopState> | null = null;
  /** Settles (rejects) the pending trailing refresh when the connection
   *  scope changes: dropping the reference alone would leave awaiting
   *  callers hanging forever. */
  private cancelDesktopStateRefreshTrailing:
    | ((reason: Error) => void)
    | null = null;

  // Catalog domain: agents.
  private agents: readonly DesktopCustomAgent[] = [];
  private catalogPhase: CatalogPhase = "loading";
  private defaultAgentId: string | null = null;
  private effectiveDefaultAgentId: string | null = null;
  private catalogVersion = 0;
  private catalogSnapshot: CatalogSnapshot | null = null;
  private catalogListeners = new Set<() => void>();

  // Dispatch-machine domain (batch 3a): message-machine state storage.
  private machine = new DispatchMachine();
  /** Gateway connection scope: the mirror's ENTIRE per-thread data universe
   *  (transcripts, dispatch machine, live streams, in-flight loads) belongs
   *  to one gateway connection. The key tracks the adopted universe; the
   *  epoch invalidates this module's in-flight continuations (lifecycle and
   *  orchestrator own their own epochs, advanced from the same transition). */
  private connectionScopeKey: string | null = null;
  private connectionEpoch = 0;

  // Dispatch-orchestration domain (batch 3c-2): send/steer/interrupt and
  // the queued-batch drain. Deps are attached by the UI layer on every
  // React commit (setDispatchDeps) until the remaining seams dissolve in
  // batches 5/6.
  // The orchestrator reaches machine/live-stream/message/accept entries
  // directly (6b-2d); the mirror is its structural MirrorPort.
  private dispatchOrchestrator = new DispatchOrchestrator(this);

  // Transcript-lifecycle domain (batch 6b-2): the transport orchestration
  // that used to live in useTranscriptController — machine bookkeeping and
  // the run-state chain in slice 2a; apply chain and fetch/stream follow.
  // The mirror itself is the module's MirrorPort (structural match).
  private transcriptLifecycle = new TranscriptLifecycle(this);

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
        phase: this.catalogPhase,
        agents: this.agents,
        defaultAgentId: this.defaultAgentId,
        effectiveDefaultAgentId: this.effectiveDefaultAgentId,
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
   * Fetch the desktop root state plus the agent catalog in one round.
   * The catalog fetch is best-effort. Updates root/catalog snapshots atomically
   * per domain and returns the fresh DesktopState for callers that need it.
   */
  refreshDesktopState(): Promise<DesktopState> {
    if (!this.services) {
      return Promise.reject(
        new Error("GatewayMirror constructed without services"),
      );
    }
    if (this.desktopStateRefreshInFlight) {
      this.desktopStateRefreshTailRequested = true;
      return this.desktopStateRefreshInFlight;
    }
    if (this.desktopStateRefreshTrailing) {
      return this.desktopStateRefreshTrailing;
    }
    return this.startDesktopStateRefresh();
  }

  private executeDesktopStateRefresh(): Promise<DesktopState> {
    const services = this.services;
    if (!services) {
      return Promise.reject(
        new Error("GatewayMirror constructed without services"),
      );
    }
    const epoch = this.connectionEpoch;
    const catalogOrdinal = ++this.catalogRequestOrdinal;
    return Promise.all([
      Promise.resolve().then(() => services.getState()),
      Promise.resolve()
        .then(() => services.listCustomAgents())
        .catch(() => null),
    ]).then(([nextState, nextCatalog]) => {
      const responseKey = nextState?.entitiesGatewayUrl || "";
      const scopeKey = this.connectionScopeKey ?? "";
      if (this.connectionEpoch !== epoch || responseKey !== scopeKey) {
        // Epoch AND identity must both hold: an answer from the previous
        // gateway universe — or a response whose own gateway identity does
        // not match the adopted scope — goes to the awaiting caller (whose
        // writes are epoch-fenced) without ever publishing as root/catalog.
        return nextState;
      }
      this.desktopState = nextState;
      this.bumpRoot();
      if (this.catalogRequestOrdinal === catalogOrdinal) {
        if (nextCatalog) {
          this.publishAgentCatalog(nextCatalog);
        } else {
          // The LATEST request failed: publish the failure (content keeps
          // the last-known values) instead of a silent stale snapshot.
          this.publishAgentCatalogFailure();
        }
      }
      return nextState;
    });
  }

  /** Monotonic catalog request order: within one epoch, only the LATEST
   *  issued fetch may publish, so an older response that returns after a
   *  newer one cannot roll the catalog back. */
  private catalogRequestOrdinal = 0;

  private publishAgentCatalog(catalog: DesktopAgentCatalog): void {
    this.agents = catalog.agents;
    this.defaultAgentId = catalog.defaultAgentId;
    this.effectiveDefaultAgentId = catalog.effectiveDefaultAgentId;
    this.catalogPhase = "ready";
    this.bumpCatalog();
  }

  /** Take-latest with an honest failure state: dropping an older response
   *  is only sound when the LATEST request's failure is published (with
   *  the last-known content retained) instead of leaving a silent blank. */
  private publishAgentCatalogFailure(): void {
    this.catalogPhase = "error";
    this.bumpCatalog();
  }

  /**
   * The mirror is the ONLY catalog owner and the only request source.
   * Refetch the agent catalog from the current gateway; the publish is
   * epoch-fenced and a failure keeps the last-known catalog.
   */
  refreshAgentCatalog(): Promise<boolean> {
    const services = this.services;
    if (!services) {
      return Promise.resolve(false);
    }
    const epoch = this.connectionEpoch;
    const ordinal = ++this.catalogRequestOrdinal;
    if (this.catalogPhase !== "loading") {
      this.catalogPhase = "loading";
      this.bumpCatalog();
    }
    return Promise.resolve()
      .then(() => services.listCustomAgents())
      .then((catalog) => {
        if (
          this.connectionEpoch !== epoch ||
          this.catalogRequestOrdinal !== ordinal
        ) {
          return true;
        }
        this.publishAgentCatalog(catalog);
        return true;
      })
      .catch(() => {
        if (
          this.connectionEpoch === epoch &&
          this.catalogRequestOrdinal === ordinal
        ) {
          this.publishAgentCatalogFailure();
        }
        return false;
      });
  }

  /** Adopt a catalog the caller already fetched (boot hydration): the
   *  caller owns the staleness fence; consumers still read one owner. */
  adoptAgentCatalog(catalog: DesktopAgentCatalog): void {
    // Adoption counts as the latest issued request: an older in-flight
    // fetch must not overwrite it.
    this.catalogRequestOrdinal += 1;
    this.publishAgentCatalog(catalog);
  }

  private startDesktopStateRefresh(): Promise<DesktopState> {
    const refresh = this.executeDesktopStateRefresh();
    this.desktopStateRefreshInFlight = refresh;
    this.observeDesktopStateRefreshSettlement(refresh);
    return refresh;
  }

  private observeDesktopStateRefreshSettlement(
    refresh: Promise<DesktopState>,
  ): void {
    void refresh.then(
      () => this.settleDesktopStateRefresh(refresh),
      () => this.settleDesktopStateRefresh(refresh),
    );
  }

  private settleDesktopStateRefresh(refresh: Promise<DesktopState>): void {
    if (this.desktopStateRefreshInFlight !== refresh) {
      return;
    }
    this.desktopStateRefreshInFlight = null;
    if (!this.desktopStateRefreshTailRequested) {
      return;
    }
    this.desktopStateRefreshTailRequested = false;
    this.scheduleTrailingDesktopStateRefresh();
  }

  private scheduleTrailingDesktopStateRefresh(): Promise<DesktopState> {
    if (this.desktopStateRefreshTrailing) {
      return this.desktopStateRefreshTrailing;
    }

    let resolveTrailing: (state: DesktopState) => void = () => undefined;
    let rejectTrailing: (reason?: unknown) => void = () => undefined;
    const trailing = new Promise<DesktopState>((resolve, reject) => {
      resolveTrailing = resolve;
      rejectTrailing = reject;
    });
    this.desktopStateRefreshTrailing = trailing;
    this.cancelDesktopStateRefreshTrailing = rejectTrailing;
    // A trailing refresh can be scheduled only because callers joined the
    // prior flight, so nobody necessarily awaits this second round.
    void trailing.catch(() => undefined);
    globalThis.setTimeout(() => {
      if (this.desktopStateRefreshTrailing !== trailing) {
        return;
      }
      this.desktopStateRefreshTrailing = null;
      this.cancelDesktopStateRefreshTrailing = null;
      this.desktopStateRefreshInFlight = trailing;
      this.executeDesktopStateRefresh().then(resolveTrailing, rejectTrailing);
    }, DESKTOP_STATE_REFRESH_TRAILING_MS);
    this.observeDesktopStateRefreshSettlement(trailing);
    return trailing;
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
    if (action.type === "thread/clear") {
      return this.machine.releaseThread(
        action.threadId,
        this.retainedIntentIdsForThread(action.threadId),
      );
    }
    return this.machine.dispatch(action);
  }

  /**
   * Adopt a gateway connection scope (normalized gateway key). The first
   * adoption (cold start) and same-key calls are no-ops. A KEY CHANGE is a
   * universe switch: every in-flight continuation is invalidated by epoch,
   * committed streams stop, pending transcript persists are cancelled (they
   * belong to the old universe), and every machine resets — while observed
   * thread entries reset IN PLACE so live subscriptions re-read the empty
   * new-universe snapshot instead of the previous gateway's data (thread
   * ids are only unique per gateway).
   */
  get currentConnectionEpoch(): number {
    return this.connectionEpoch;
  }

  isCurrentConnectionEpoch(epoch: number): boolean {
    return this.connectionEpoch === epoch;
  }

  beginConnectionScope(
    key: string,
    options?: { desktopState?: DesktopState | null },
  ): void {
    const previous = this.connectionScopeKey;
    this.connectionScopeKey = key;
    if (previous === null || previous === key || previous === "") {
      if (previous !== key && options?.desktopState) {
        // First landing on a real key: adopt the committed root so root
        // consumers have the same universe as React from frame one. No
        // destructive reset — nothing meaningful loaded under the cold
        // scope.
        this.desktopState = options.desktopState;
        this.bumpRoot();
      }
      return;
    }
    this.connectionEpoch += 1;
    // The mirror ROOT belongs to the universe too: adopt the committed
    // new-gateway state (when the caller has it) instead of keeping the
    // previous gateway's root, drop any in-flight root refresh (its answer
    // is epoch-fenced in executeDesktopStateRefresh), and clear the agent
    // catalog — a late A catalog must never republish under B.
    this.desktopState = options?.desktopState ?? null;
    this.desktopStateRefreshInFlight = null;
    this.cancelDesktopStateRefreshTrailing?.(
      new Error("gateway connection scope changed"),
    );
    this.cancelDesktopStateRefreshTrailing = null;
    this.desktopStateRefreshTrailing = null;
    this.desktopStateRefreshTailRequested = false;
    this.bumpRoot();
    this.agents = [];
    this.defaultAgentId = null;
    this.effectiveDefaultAgentId = null;
    this.catalogPhase = "loading";
    this.bumpCatalog();
    if (this.services) {
      // Repopulate root + catalog from the NEW gateway.
      void this.refreshDesktopState().catch(() => {});
    }
    this.transcriptLifecycle.beginConnectionEpoch();
    this.dispatchOrchestrator.beginConnectionEpoch();
    for (const entry of this.threads.values()) {
      entry.cache = new ThreadTranscriptCache();
      entry.frontier = new ThreadFrontier();
      entry.liveStream = null;
      entry.retainCount = 0;
      this.commitThread(entry, false);
    }
    this.liveStreamMap = {};
    for (const listener of [...this.liveStreamListeners]) {
      listener();
    }
    this.transcriptMapsSnapshot = null;
    for (const listener of [...this.transcriptMapsListeners]) {
      listener();
    }
    this.machine.resetAll();
  }

  private retainedIntentIdsForThread(threadId: string): Set<string> {
    const snapshot = this.getThreadSnapshot(threadId);
    const retained = new Set<string>();

    for (const message of snapshot.messages) {
      if (message.intentId && message.localState !== "remote_final") {
        retained.add(message.intentId);
      }
    }

    const awaitingAckInputIds = new Set(
      snapshot.pendingRemoteInputs
        .filter((input) => input.status === "awaiting_ack")
        .map((input) => input.id),
    );
    if (awaitingAckInputIds.size > 0) {
      for (const intent of Object.values(this.machine.getState().intentsById)) {
        if (
          intent.threadId === threadId &&
          intent.pendingInputId &&
          awaitingAckInputIds.has(intent.pendingInputId)
        ) {
          retained.add(intent.intentId);
        }
      }
    }

    return retained;
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

  /** Attach (or refresh) the transcript-lifecycle React seams (6b-2a). */
  setTranscriptLifecycleDeps(deps: TranscriptLifecycleDeps): void {
    this.transcriptLifecycle.setDeps(deps);
  }

  syncTranscriptRunState(threadId: string, transcript: ThreadTranscript) {
    return this.transcriptLifecycle.syncTranscriptRunState(
      threadId,
      transcript,
    );
  }

  applyCommittedTranscriptRunState(event: {
    threadId: string;
    seq: number;
    message: CommittedMessageEvent["message"];
  }) {
    return this.transcriptLifecycle.applyCommittedTranscriptRunState(event);
  }

  markIntentsFromHistory(
    threadId: string,
    transcript: Parameters<TranscriptLifecycle["markIntentsFromHistory"]>[1],
  ): void {
    this.transcriptLifecycle.markIntentsFromHistory(threadId, transcript);
  }

  applyUserAck(threadId: string, runId: string, pendingInputId?: string): void {
    this.transcriptLifecycle.applyUserAck(threadId, runId, pendingInputId);
  }

  forceReleaseThreadRuntime(threadId: string): void {
    this.transcriptLifecycle.forceReleaseThreadRuntime(threadId);
  }

  hasPendingHistoryIntents(threadId: string): boolean {
    return this.transcriptLifecycle.hasPendingHistoryIntents(threadId);
  }

  setThreadRuntimeState(
    ...args: Parameters<TranscriptLifecycle["setThreadRuntimeState"]>
  ): void {
    this.transcriptLifecycle.setThreadRuntimeState(...args);
  }

  getThreadTitleOverrides(): Record<string, string> {
    return this.transcriptLifecycle.getThreadTitleOverrides();
  }

  /**
   * Slice 2b high-level entries: the accept* facades run the apply-chain
   * ride-alongs and call the pure cache-only commits internally. The pure
   * applyAuthoritativeTranscript / applyRemoteTranscript keep their
   * cache-only contract.
   */
  acceptAuthoritativeTranscript(
    threadId: string,
    transcript: ThreadTranscript,
    options?: { syncRunState?: boolean },
  ): void {
    this.transcriptLifecycle.acceptAuthoritativeTranscript(
      threadId,
      transcript,
      options,
    );
  }

  acceptRemoteTranscript(
    ...args: Parameters<TranscriptLifecycle["acceptRemoteTranscript"]>
  ): void {
    this.transcriptLifecycle.acceptRemoteTranscript(...args);
  }

  applyCommittedThreadMessage(
    event: Parameters<TranscriptLifecycle["applyCommittedThreadMessage"]>[0],
  ): void {
    this.transcriptLifecycle.applyCommittedThreadMessage(event);
  }

  updateMessagesByThread(
    updater: Parameters<TranscriptLifecycle["updateMessagesByThread"]>[0],
  ): ReturnType<TranscriptLifecycle["updateMessagesByThread"]> {
    return this.transcriptLifecycle.updateMessagesByThread(updater);
  }

  /**
   * Transcript-cache persistence IPC (the lifecycle's persist ride-along
   * reaches it through the MirrorPort). Optional service: omitted in
   * node tests, where persists are recorded by the stub port instead.
   */
  persistTranscriptCache(
    transcript: ThreadTranscript,
    renderState: RenderState | null,
  ): void {
    // The disk cache is partitioned by gateway scope: a persist carries the
    // universe it belongs to, so even a leaked late save can only ever
    // write into its OWN gateway's partition.
    void this.services
      ?.saveThreadTranscriptCache?.(
        this.connectionScopeKey ?? "",
        transcript,
        renderState,
      )
      .catch(() => {});
  }

  // Slice 6b-2c MirrorPort accessors: the lifecycle's transport IPC,
  // resolved from the injected services (throw-if-missing, matching
  // fetchOlderThreadHistoryPage).
  private requireLifecycleService<K extends keyof GatewayMirrorServices>(
    name: K,
  ): NonNullable<GatewayMirrorServices[K]> {
    const service = this.services?.[name];
    if (!service) {
      throw new Error(`GatewayMirror constructed without services (${name})`);
    }
    return service as NonNullable<GatewayMirrorServices[K]>;
  }

  startThreadStream(input: StartThreadStreamInput): Promise<void> {
    return this.requireLifecycleService("startThreadStream")(input);
  }

  stopThreadStream(input: StopThreadStreamInput): Promise<void> {
    return this.requireLifecycleService("stopThreadStream")(input);
  }

  loadThreadTranscriptCache(
    threadId: string,
  ): Promise<CachedThreadTranscript | null> {
    return this.requireLifecycleService("loadThreadTranscriptCache")(
      this.connectionScopeKey ?? "",
      threadId,
    );
  }

  clearThreadTranscriptCache(threadId: string): Promise<void> {
    return this.requireLifecycleService("clearThreadTranscriptCache")(
      this.connectionScopeKey ?? "",
      threadId,
    );
  }

  getThreadHistoryFull(threadId: string): Promise<ThreadTranscript> {
    return this.requireLifecycleService("getThreadHistoryFull")(threadId);
  }

  getThreadHistoryPage(input: GetThreadHistoryInput): Promise<ThreadTranscript> {
    return this.requireLifecycleService("getThreadHistory")(input);
  }

  /**
   * Slice 2c high-level entries: the fetch/stream lifecycle.
   */
  notifyStreamEvent(event: DesktopChatStreamEvent): void {
    this.transcriptLifecycle.notifyStreamEvent(event);
  }

  loadSelectedThreadTranscript(threadId: string): Promise<void> {
    return this.transcriptLifecycle.loadSelectedThreadTranscript(threadId);
  }

  cancelSelectedThreadLoad(threadId: string): void {
    this.transcriptLifecycle.cancelSelectedThreadLoad(threadId);
  }

  startCommittedThreadStream(
    threadId: string,
    transcript: ThreadTranscript,
    consumerId: string,
  ): Promise<void> {
    return this.transcriptLifecycle.startCommittedThreadStream(
      threadId,
      transcript,
      consumerId,
    );
  }

  stopCommittedThreadStream(input: StopThreadStreamInput): Promise<void> {
    return this.transcriptLifecycle.stopCommittedThreadStream(input);
  }

  flushAllTranscriptPersistence(): number {
    return this.transcriptLifecycle.flushAllTranscriptPersistence();
  }

  refetchAuthoritativeTranscriptAfterRewrite(threadId: string): Promise<void> {
    return this.transcriptLifecycle.refetchAuthoritativeTranscriptAfterRewrite(
      threadId,
    );
  }

  /**
   * Older-page load with the UI scroll-anchor + error chrome (lifecycle);
   * the pure guard/fetch/apply stays fetchOlderThreadHistoryPage above.
   */
  loadOlderThreadHistoryPage(threadId: string): Promise<void> {
    return this.transcriptLifecycle.loadOlderThreadHistoryPage(threadId);
  }

  ensureThreadOpenable(threadId: string): Promise<boolean> {
    return this.transcriptLifecycle.ensureThreadOpenable(threadId);
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
    const entry = this.threads.get(threadId);
    if (!entry) {
      return null;
    }
    this.touchThread(entry);
    return entry.liveStream;
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
    const fromEntry = this.threads.get(fromThreadId);
    if (!fromEntry) {
      return;
    }
    this.touchThread(fromEntry);
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
    this.touchThread(entry);
    this.pruneAndNotifyTranscriptMaps();
    return () => {
      if (!entry.listeners.delete(listener)) {
        return;
      }
      this.touchThread(entry);
      this.pruneAndNotifyTranscriptMaps();
    };
  }

  getThreadSnapshot(threadId: string): ThreadMirrorSnapshot {
    const entry = this.threads.get(threadId);
    if (!entry) {
      return this.emptyThreadSnapshot(threadId);
    }
    this.touchThread(entry);
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
   * The committed transport snapshot for a thread (batch 6b-1): the
   * resolved ThreadTranscript remembered by the latest authoritative/
   * remote/committed apply. The transcript controller's fetch/stream
   * lifecycle reads it for resume cursors and forward merges; the
   * controller no longer keeps its own copy.
   */
  getThreadSnapshotTranscript(threadId: string): ThreadTranscript | null {
    const entry = this.threads.get(threadId);
    if (!entry) {
      return null;
    }
    this.touchThread(entry);
    return entry.cache.getSnapshotTranscript();
  }

  earliestLoadedCommittedBodySeq(threadId: string): number | null {
    const entry = this.threads.get(threadId);
    if (!entry) {
      return null;
    }
    this.touchThread(entry);
    return entry.cache.earliestLoadedCommittedBodySeq();
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
   * Drop a thread's transcript state (batch 4b missing-thread cleanup):
   * the selected thread turned out not to exist, so applied stale-cache
   * values — including any committed records/frontier progress from the
   * briefly-live stream — roll back to the never-loaded shape in one
   * commit. Live-stream transport state stays (owned by dispatch).
   */
  clearThreadTranscript(threadId: string): void {
    const entry = this.threadEntry(threadId);
    entry.cache = new ThreadTranscriptCache();
    entry.frontier = new ThreadFrontier();
    this.transcriptLifecycle.clearThreadWindowState(threadId);
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
    const entry = this.threads.get(threadId);
    if (!entry) {
      return;
    }
    this.touchThread(entry);
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
  async fetchOlderThreadHistoryPage(
    threadId: string,
    options?: { onPageFetched?: (transcript: ThreadTranscript) => void },
  ): Promise<boolean> {
    if (!this.services) {
      throw new Error("GatewayMirror constructed without services");
    }
    const entry = this.threadEntry(threadId);
    const release = this.retainThreadEntry(entry);
    try {
      const pagination = entry.cache.getHistoryPagination();
      if (
        !pagination?.hasMoreBefore ||
        pagination.loadingBefore ||
        pagination.nextBeforeIndex === null
      ) {
        return false;
      }

      entry.cache.setHistoryPagination({
        hasMoreBefore: pagination.hasMoreBefore,
        nextBeforeIndex: pagination.nextBeforeIndex,
        loadingBefore: true,
      });
      this.commitThread(entry);

      const epoch = this.connectionEpoch;
      try {
        const transcript = await this.services.getThreadHistory({
          threadId,
          beforeIndex: pagination.nextBeforeIndex,
          limit: THREAD_HISTORY_PAGE_SIZE,
          userQueryLimit: THREAD_HISTORY_USER_QUERY_LIMIT,
        });
        if (this.connectionEpoch !== epoch) {
          // The page belongs to the previous gateway universe; the entry
          // now holds the NEW universe's transcript for this thread id.
          return false;
        }
        options?.onPageFetched?.(transcript);
        entry.cache.applyOlderPage(transcript);
        return true;
      } finally {
        // Only the owning epoch settles the loading flag: a stale fetch's
        // finally must not clear an in-flight NEW-universe page load.
        if (this.connectionEpoch === epoch) {
          const current = entry.cache.getHistoryPagination();
          entry.cache.setHistoryPagination(
            current ? { ...current, loadingBefore: false } : current,
          );
          this.commitThread(entry);
        }
      }
    } finally {
      release();
    }
  }

  /**
   * Ingest one desktop chat-stream event. A `thread_render_frame` is applied
   * as ONE synchronous commit: committed events first, then the render
   * snapshot, then a single notification. Subscribers can never observe half
   * a frame. A bare `committed_message` is treated as a frame without a
   * render snapshot.
   */
  ingest(
    event: DesktopChatStreamEvent,
    options?: ThreadStreamIngestOptions,
  ): ThreadStreamIngestResult {
    const applyConnectionScoped = options?.applyConnectionScoped !== false;
    if (event.type === "thread_render_frame") {
      return this.applyFrame(
        event.threadId,
        event.events,
        applyConnectionScoped ? event.renderState : null,
        applyConnectionScoped && event.replay === "windowed",
        applyConnectionScoped,
      );
    }
    if (event.type === "committed_message") {
      return this.applyFrame(event.threadId, [event], null);
    }
    // "error" events carry no mirrored state in batch 0.
    return {
      appliedEvents: [],
      renderAccepted: false,
      renderChanged: false,
      connectionScopedApplied: applyConnectionScoped,
    };
  }

  private applyFrame(
    threadId: string,
    events: readonly CommittedMessageEvent[],
    renderState: RenderState | null,
    windowedReplay = false,
    connectionScopedApplied = true,
  ): ThreadStreamIngestResult {
    const entry = this.threadEntry(threadId);
    let changed = false;

    if (windowedReplay) {
      // Server-degraded stale resume: cached committed records below the
      // window floor are no longer contiguous with this connection.
      const floorSeq = renderState?.window?.floor_seq ?? 0;
      if (floorSeq > 0 && entry.cache.dropCommittedBelow(floorSeq)) {
        changed = true;
      }
    }

    const acceptedEvents = entry.cache.applyCommittedEvents(events);
    const appliedEvents = acceptedEvents.map(({ event }) => event);
    if (acceptedEvents.length > 0) {
      let highestApplied = 0;
      for (const { event } of acceptedEvents) {
        if (event.seq > highestApplied) {
          highestApplied = event.seq;
        }
      }
      entry.frontier.advanceCommitted(highestApplied);
      // Committed → UI-message mapping, mirroring the hook's per-event
      // applyCommittedThreadMessage loop. Only newly applied events map
      // (seq-idempotent redelivery stays a no-op). Rewrite/reset controls
      // skip the mapping and request an authoritative refetch instead.
      for (const { event, disposition } of acceptedEvents) {
        const outcome = entry.cache.applyCommittedMessage(
          event,
          { intentForId: this.intentLookup },
          disposition,
        );
        if (outcome === "refetch_authoritative") {
          // Slice 6b-2c: the lifecycle is the single de-duplicated refetch
          // owner. The committed side-effect step triggers it too for the
          // same event; the per-thread in-flight de-dupe coalesces both
          // into one fetch + stream restart.
          void this.transcriptLifecycle.refetchAuthoritativeTranscriptAfterRewrite(
            threadId,
          );
        }
      }
      changed = true;
    }

    let renderAccepted = false;
    let renderChanged = false;
    if (renderState) {
      renderAccepted = entry.frontier.acceptRender(renderState.based_on_seq);
      if (
        renderAccepted &&
        !renderStatesEqual(entry.cache.getRenderState(), renderState)
      ) {
        entry.cache.setRenderState(renderState);
        changed = true;
        renderChanged = true;
      }
      // Ordering-rejected or value-identical snapshots preserve the held
      // reference. Cursor equality alone is never a change decision.
    }

    if (!changed) {
      this.pruneAndNotifyTranscriptMaps();
      return {
        appliedEvents,
        renderAccepted,
        renderChanged,
        connectionScopedApplied,
      };
    }
    this.commitThread(entry);
    return {
      appliedEvents,
      renderAccepted,
      renderChanged,
      connectionScopedApplied,
    };
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
    this.touchThread(entry);
    entry.version += 1;
    entry.snapshot = null;
    const pruned = this.pruneInactiveThreads();
    if (touchesTranscript || pruned) {
      this.transcriptMapsSnapshot = null;
    }
    for (const listener of [...entry.listeners]) {
      listener();
    }
    if (touchesTranscript || pruned) {
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
        threadId,
        cache: new ThreadTranscriptCache(),
        frontier: new ThreadFrontier(),
        liveStream: null,
        listeners: new Set(),
        lastAccess: ++this.threadAccessOrdinal,
        retainCount: 0,
        version: 0,
        snapshot: this.emptyThreadSnapshot(threadId),
      };
      this.threads.set(threadId, entry);
    } else {
      this.touchThread(entry);
    }
    return entry;
  }

  private emptyThreadSnapshot(threadId: string): ThreadMirrorSnapshot {
    let snapshot = this.emptyThreadSnapshots.get(threadId);
    if (!snapshot) {
      snapshot = {
        version: 0,
        threadId,
        records: EMPTY_COMMITTED_RECORDS,
        messages: EMPTY_UI_MESSAGES,
        threadInfo: null,
        pendingRemoteInputs: EMPTY_PENDING_REMOTE_INPUTS,
        renderState: null,
        historyPagination: null,
        liveStream: null,
        transcriptLoaded: false,
        frontier: EMPTY_THREAD_FRONTIER,
      };
      this.emptyThreadSnapshots.set(threadId, snapshot);
    }
    return snapshot;
  }

  private touchThread(entry: ThreadEntry): void {
    entry.lastAccess = ++this.threadAccessOrdinal;
  }

  private entryHasUnrecoverableLocalMessages(entry: ThreadEntry): boolean {
    return entry.cache.getUiMessages().some((message) => {
      return Boolean(message.localState) && message.localState !== "remote_final";
    });
  }

  private threadEntryIsEvictable(entry: ThreadEntry): boolean {
    return (
      entry.threadId !== NEW_THREAD_DRAFT_THREAD_ID &&
      entry.listeners.size === 0 &&
      entry.retainCount === 0 &&
      entry.liveStream === null &&
      !this.entryHasUnrecoverableLocalMessages(entry)
    );
  }

  private pruneInactiveThreads(): boolean {
    const evictable = [...this.threads.values()].filter((entry) =>
      this.threadEntryIsEvictable(entry),
    );
    const removeCount =
      evictable.length - GATEWAY_MIRROR_INACTIVE_THREAD_LIMIT;
    if (removeCount <= 0) {
      return false;
    }
    evictable.sort((left, right) => left.lastAccess - right.lastAccess);
    let removed = false;
    for (const entry of evictable.slice(0, removeCount)) {
      if (this.threads.get(entry.threadId) === entry) {
        this.threads.delete(entry.threadId);
        this.transcriptLifecycle.clearThreadWindowState(entry.threadId);
        removed = true;
      }
    }
    return removed;
  }

  private pruneAndNotifyTranscriptMaps(): void {
    if (!this.pruneInactiveThreads()) {
      return;
    }
    this.transcriptMapsSnapshot = null;
    for (const listener of [...this.transcriptMapsListeners]) {
      listener();
    }
  }

  private retainThreadEntry(entry: ThreadEntry): () => void {
    entry.retainCount += 1;
    this.touchThread(entry);
    let released = false;
    return () => {
      if (released) {
        return;
      }
      released = true;
      entry.retainCount = Math.max(0, entry.retainCount - 1);
      this.touchThread(entry);
      this.pruneAndNotifyTranscriptMaps();
    };
  }
}
