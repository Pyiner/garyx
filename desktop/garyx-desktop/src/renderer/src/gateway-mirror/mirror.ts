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

import type { MessageIntent } from "../message-machine.ts";
import type { UiTranscriptMessage } from "../app-shell/types";
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
  readonly frontier: ThreadFrontierSnapshot;
}

interface ThreadEntry {
  cache: ThreadTranscriptCache;
  frontier: ThreadFrontier;
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

  /** One atomic thread commit: version bump, snapshot invalidation, notify. */
  private commitThread(entry: ThreadEntry): void {
    entry.version += 1;
    entry.snapshot = null;
    for (const listener of [...entry.listeners]) {
      listener();
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
        listeners: new Set(),
        version: 0,
        snapshot: null,
      };
      this.threads.set(threadId, entry);
    }
    return entry;
  }
}
