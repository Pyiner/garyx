// Per-thread committed-record and render-state cache for the gateway mirror.
//
// Batch 0 scope (docs/design/appshell-endgame-architecture.md, migration
// batch 0): the cache stores committed events verbatim, keyed by seq, plus
// the latest accepted server RenderState. Mapping committed bodies into the
// UI transcript-message shape is batch-2 work and does not belong here yet.
// The cache never derives transcript structure: rows/grouping/tail-thinking
// stay server-owned inside `renderState`.

import type { CommittedMessageEvent, RenderState } from "@shared/contracts";

export class ThreadTranscriptCache {
  private recordsBySeq = new Map<number, CommittedMessageEvent>();
  private sortedCache: readonly CommittedMessageEvent[] | null = null;
  private renderState: RenderState | null = null;

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
