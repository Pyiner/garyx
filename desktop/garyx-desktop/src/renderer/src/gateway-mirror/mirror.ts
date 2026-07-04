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
  RenderState,
} from "@shared/contracts";
import { ThreadFrontier } from "./frontier.ts";
import type { ThreadFrontierSnapshot } from "./frontier.ts";
import { ThreadTranscriptCache } from "./transcript-cache.ts";

export type Unsubscribe = () => void;

/**
 * The IPC surface the mirror needs, injected so the class stays pure
 * TypeScript and testable without a preload bridge.
 */
export interface GatewayMirrorServices {
  getState(): Promise<DesktopState>;
  listCustomAgents(): Promise<DesktopCustomAgent[]>;
  listTeams(): Promise<DesktopTeam[]>;
  listWorkflowDefinitions(): Promise<DesktopWorkflowDefinition[]>;
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
   * Committed events verbatim, seq-ordered. Batch 2 replaces this surface
   * with mapped UI transcript messages; contract tests depend only on
   * version/reference/monotonicity semantics, which stay stable.
   */
  readonly records: readonly CommittedMessageEvent[];
  readonly renderState: RenderState | null;
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
    if (status === this.connection) {
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
        renderState: entry.cache.getRenderState(),
        frontier: entry.frontier.snapshot(),
      };
    }
    return entry.snapshot;
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

    const highestApplied = entry.cache.applyCommittedEvents(events);
    if (highestApplied !== null) {
      entry.frontier.advanceCommitted(highestApplied);
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
    entry.version += 1;
    entry.snapshot = null;
    for (const listener of [...entry.listeners]) {
      listener();
    }
  }

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
