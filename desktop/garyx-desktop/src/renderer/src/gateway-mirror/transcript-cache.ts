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
import { transcriptWithResolvedActiveRun } from "../../../shared/transcript-sync.ts";

import type { UiTranscriptMessage } from "../app-shell/types";
import {
  materializeRemoteTranscript,
  visibleTranscriptMessages,
} from "./transcript-materialize.ts";

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

  /**
   * Apply committed events idempotently. An event whose seq is already
   * cached is ignored (the committed ledger is append-only; a re-delivered
   * seq carries the same record). Returns the highest newly applied seq, or
   * null when nothing changed.
   */
  applyCommittedEvents(events: readonly CommittedMessageEvent[]): number | null {
    let highestApplied: number | null = null;
    for (const event of events) {
      if (!Number.isFinite(event.seq) || event.seq <= 0) {
        continue;
      }
      if (this.recordsBySeq.has(event.seq)) {
        continue;
      }
      this.recordsBySeq.set(event.seq, event);
      if (highestApplied === null || event.seq > highestApplied) {
        highestApplied = event.seq;
      }
    }
    if (highestApplied !== null) {
      this.sortedCache = null;
    }
    return highestApplied;
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
