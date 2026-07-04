// Per-thread committed-record and render-state cache for the gateway mirror.
//
// Batch 0 scope (docs/design/appshell-endgame-architecture.md, migration
// batch 0): the cache stores committed events verbatim, keyed by seq, plus
// the latest accepted server RenderState. Mapping committed bodies into the
// UI transcript-message shape is batch-2 work and does not belong here yet.
// The cache never derives transcript structure: rows/grouping/tail-thinking
// stay server-owned inside `renderState`.

import type {
  CommittedMessageEvent,
  PendingThreadInput,
  RenderState,
  ThreadRuntimeInfo,
  ThreadTranscript,
} from "@shared/contracts";
import {
  transcriptRewriteAction,
  transcriptWithResolvedActiveRun,
} from "../../../shared/transcript-sync.ts";

import type { MessageIntent } from "../message-machine.ts";
import type { UiTranscriptMessage } from "../app-shell/types";
import {
  committedMessageForwardPage,
  materializeRemoteTranscript,
  mergeRemotePaginationState,
  mergeRemoteTranscriptWithLocal,
  paginationStateFromTranscript,
  visibleTranscriptMessages,
  type ThreadHistoryPaginationState,
} from "./transcript-materialize.ts";

/**
 * Message-machine intent lookup, injected because the machine stays with
 * its legacy owner until batch 3. A mirror without live local intents
 * passes a null lookup and the merge behaves like the remote-only path.
 */
export interface RemoteApplyOptions {
  intentForId: (intentId: string) => MessageIntent | null;
}

export class ThreadTranscriptCache {
  private recordsBySeq = new Map<number, CommittedMessageEvent>();
  private sortedCache: readonly CommittedMessageEvent[] | null = null;
  private renderState: RenderState | null = null;

  // Authoritative-transcript domain (batch 2a-2): the mirror-side
  // equivalents of the hook's messagesByThread / threadInfoByThread /
  // pendingRemoteInputsByThread slices plus the remembered snapshot.
  // Run-state sync (message machine) and cache persistence (IPC) stay
  // with their owners per the design: batch 3 and batch 2b respectively.
  private uiMessages: readonly UiTranscriptMessage[] = [];
  private threadInfo: ThreadRuntimeInfo | null = null;
  private pendingRemoteInputs: readonly PendingThreadInput[] = [];
  private snapshotTranscript: ThreadTranscript | null = null;
  private historyPagination: ThreadHistoryPaginationState | null = null;

  /**
   * Apply an authoritative (canonical) transcript: the pure core of the
   * hook's applyCanonicalTranscript. Resolves the active run, remembers
   * the snapshot, replaces thread info and pending inputs, and merges the
   * visible messages into the UI message cache through
   * materializeRemoteTranscript — identical inputs therefore produce
   * identical message arrays to the legacy path (dual-run tested).
   */
  applyAuthoritative(transcript: ThreadTranscript): void {
    const resolved = transcriptWithResolvedActiveRun(transcript);
    this.snapshotTranscript = resolved;
    this.threadInfo = resolved.threadInfo ?? null;
    this.pendingRemoteInputs = resolved.pendingInputs ?? [];
    const visible = visibleTranscriptMessages(resolved.messages);
    this.uiMessages = materializeRemoteTranscript(visible, [
      ...this.uiMessages,
    ]);
  }

  getUiMessages(): readonly UiTranscriptMessage[] {
    return this.uiMessages;
  }

  getThreadInfo(): ThreadRuntimeInfo | null {
    return this.threadInfo;
  }

  getPendingRemoteInputs(): readonly PendingThreadInput[] {
    return this.pendingRemoteInputs;
  }

  getSnapshotTranscript(): ThreadTranscript | null {
    return this.snapshotTranscript;
  }

  getHistoryPagination(): ThreadHistoryPaginationState | null {
    return this.historyPagination;
  }

  setHistoryPagination(state: ThreadHistoryPaginationState | null): void {
    this.historyPagination = state;
  }

  /**
   * Apply a remote transcript (full fetch, forward aggregate, or committed
   * forward-merge): the pure core of the hook's applyRemoteTranscript.
   * Covers snapshot memory, pagination merge, thread info, pending inputs,
   * and the local/remote message merge. Not covered here (stays with legacy
   * owners): message-machine run-state sync, IPC cache persistence,
   * desktopState session/team propagation, and intent history marking.
   */
  applyRemote(transcript: ThreadTranscript, options: RemoteApplyOptions): void {
    const resolved = transcriptWithResolvedActiveRun(transcript);
    this.snapshotTranscript = resolved;
    // Pagination merges against the message cache BEFORE this apply's merge,
    // matching the legacy read order of messagesByThreadRef.
    const existing = [...this.uiMessages];
    this.historyPagination = mergeRemotePaginationState(
      this.historyPagination,
      paginationStateFromTranscript(resolved),
      existing,
    );
    this.threadInfo = resolved.threadInfo ?? null;
    this.pendingRemoteInputs = resolved.pendingInputs ?? [];
    const visibleMessages = visibleTranscriptMessages(resolved.messages);
    const merged = mergeRemoteTranscriptWithLocal(visibleMessages, existing, {
      activeRunLiveRows: Boolean(resolved.threadInfo?.activeRun),
      preserveRemoteBeforeIndex: resolved.pageInfo?.startIndex ?? null,
      threadRunActive: Boolean(resolved.threadInfo?.activeRun),
      intentForId: options.intentForId,
    });
    // Identity-preserving check from the legacy updater: an equivalent merge
    // keeps the previous array reference so snapshots stay stable.
    if (
      merged.length === existing.length &&
      merged.every((entry, index) => entry === existing[index])
    ) {
      return;
    }
    this.uiMessages = merged;
  }

  /**
   * Fold one committed stream record into the transcript state: the pure
   * core of the hook's applyCommittedThreadMessage. Returns
   * "refetch_authoritative" when the record is a rewrite/reset control the
   * caller must resolve with an authoritative refetch (no state is touched),
   * "applied" otherwise.
   */
  applyCommittedMessage(
    event: CommittedMessageEvent,
    options: RemoteApplyOptions,
  ): "refetch_authoritative" | "applied" {
    if (transcriptRewriteAction(event.message) === "refetch_authoritative") {
      return "refetch_authoritative";
    }
    this.applyRemote(
      committedMessageForwardPage(this.snapshotTranscript, event),
      options,
    );
    return "applied";
  }

  /**
   * Prepend an older history page: the pure core of the hook's
   * applyOlderRemoteTranscriptPage. Replaces pagination from the page and
   * prepends materialized entries not already present.
   */
  applyOlderPage(transcript: ThreadTranscript): void {
    this.historyPagination = paginationStateFromTranscript(transcript);
    const visibleMessages = visibleTranscriptMessages(transcript.messages);
    if (visibleMessages.length === 0) {
      return;
    }
    const existing = this.uiMessages;
    const existingIds = new Set(existing.map((entry) => entry.id));
    const olderEntries = materializeRemoteTranscript(visibleMessages, []).filter(
      (entry) => !existingIds.has(entry.id),
    );
    if (olderEntries.length === 0) {
      return;
    }
    this.uiMessages = [...olderEntries, ...existing];
  }

  /**
   * Apply committed events idempotently. An event whose seq is already
   * cached is ignored (the committed ledger is append-only; a re-delivered
   * seq carries the same record). Returns the newly applied events in input
   * order (empty when nothing changed).
   */
  applyCommittedEvents(
    events: readonly CommittedMessageEvent[],
  ): CommittedMessageEvent[] {
    const applied: CommittedMessageEvent[] = [];
    for (const event of events) {
      if (!Number.isFinite(event.seq) || event.seq <= 0) {
        continue;
      }
      if (this.recordsBySeq.has(event.seq)) {
        continue;
      }
      this.recordsBySeq.set(event.seq, event);
      applied.push(event);
    }
    if (applied.length > 0) {
      this.sortedCache = null;
    }
    return applied;
  }

  /** Store an accepted render snapshot (monotonicity is the caller's job). */
  setRenderState(renderState: RenderState): void {
    this.renderState = renderState;
  }

  getRenderState(): RenderState | null {
    return this.renderState;
  }

  /** Cached seq-ordered records; same reference until new events apply. */
  sortedRecords(): readonly CommittedMessageEvent[] {
    if (!this.sortedCache) {
      this.sortedCache = [...this.recordsBySeq.values()].sort(
        (a, b) => a.seq - b.seq,
      );
    }
    return this.sortedCache;
  }
}
