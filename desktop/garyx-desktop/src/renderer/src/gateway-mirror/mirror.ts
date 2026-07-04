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
  DesktopChatStreamEvent,
  RenderState,
} from "@shared/contracts";
import { ThreadFrontier } from "./frontier.ts";
import type { ThreadFrontierSnapshot } from "./frontier.ts";
import { ThreadTranscriptCache } from "./transcript-cache.ts";

export type Unsubscribe = () => void;

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
