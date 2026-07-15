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
}

export class ThreadFrontier {
  private committedSeq = 0;
  private renderBasedOnSeq = 0;
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
   * Accept a render snapshot cursor monotonically. Cursor equality is an
   * ordering success, not snapshot identity: the server may legitimately
   * send same-seq overwrite or wider-window snapshots. Full-value change
   * detection belongs to the transcript mirror.
   */
  acceptRender(basedOnSeq: number): boolean {
    if (!Number.isFinite(basedOnSeq) || basedOnSeq < this.renderBasedOnSeq) {
      return false;
    }
    if (basedOnSeq > this.renderBasedOnSeq || !this.hasRenderSnapshot) {
      this.renderBasedOnSeq = basedOnSeq;
      this.hasRenderSnapshot = true;
      this.cached = null;
    }
    return true;
  }

  /** Cached snapshot: the same reference is returned until a cursor moves. */
  snapshot(): ThreadFrontierSnapshot {
    if (!this.cached) {
      this.cached = {
        committedSeq: this.committedSeq,
        renderBasedOnSeq: this.renderBasedOnSeq,
      };
    }
    return this.cached;
  }
}
