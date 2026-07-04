// Per-thread stream frontiers for the gateway mirror.
//
// Two cursors are deliberately separate (design:
// docs/design/appshell-endgame-architecture.md, "Stream and frontier
// semantics"):
//
// - `committedSeq` advances only when committed events are actually applied.
//   It is the safe reconnect `afterSeq` value.
// - `renderBasedOnSeq` advances when a `render_state` snapshot is accepted.
//   Snapshot-only frames (empty `events`) may advance it while `committedSeq`
//   stays put; a render-only frame must never pollute the committed cursor.

export interface ThreadFrontierSnapshot {
  readonly committedSeq: number;
  readonly renderBasedOnSeq: number;
  readonly renderFloor: number;
}

export class ThreadFrontier {
  private committedSeq = 0;
  private renderBasedOnSeq = 0;
  private renderFloor = 0;
  // A caught-up frame for an empty ledger legitimately carries
  // based_on_seq=0 (the server clamps to the committed tail), so "have we
  // ever accepted a render snapshot" needs its own flag instead of using
  // renderBasedOnSeq=0 as a sentinel.
  private hasRenderSnapshot = false;
  private cached: ThreadFrontierSnapshot | null = null;

  /** Advance the committed cursor. Returns true when it moved forward. */
  advanceCommitted(seq: number): boolean {
    if (!Number.isFinite(seq) || seq <= this.committedSeq) {
      return false;
    }
    this.committedSeq = seq;
    this.cached = null;
    return true;
  }

  /**
   * Accept a render snapshot cursor monotonically. Returns whether the
   * snapshot may be applied at all; `changed` is true only when the cursor
   * moved. Re-delivery at the same `based_on_seq` is accepted (the server
   * derives render_state deterministically from the committed ledger, so an
   * equal cursor means an equal snapshot) but does not count as a change.
   */
  acceptRender(basedOnSeq: number): { accepted: boolean; changed: boolean } {
    if (!Number.isFinite(basedOnSeq) || basedOnSeq < this.renderBasedOnSeq) {
      return { accepted: false, changed: false };
    }
    if (basedOnSeq === this.renderBasedOnSeq && this.hasRenderSnapshot) {
      return { accepted: true, changed: false };
    }
    this.renderBasedOnSeq = basedOnSeq;
    this.hasRenderSnapshot = true;
    this.cached = null;
    return { accepted: true, changed: true };
  }

  setRenderFloor(floor: number): boolean {
    const next = Number.isFinite(floor) && floor > 0 ? floor : 0;
    if (next === this.renderFloor) {
      return false;
    }
    this.renderFloor = next;
    this.cached = null;
    return true;
  }

  /** Cached snapshot: the same reference is returned until a cursor moves. */
  snapshot(): ThreadFrontierSnapshot {
    if (!this.cached) {
      this.cached = {
        committedSeq: this.committedSeq,
        renderBasedOnSeq: this.renderBasedOnSeq,
        renderFloor: this.renderFloor,
      };
    }
    return this.cached;
  }
}
