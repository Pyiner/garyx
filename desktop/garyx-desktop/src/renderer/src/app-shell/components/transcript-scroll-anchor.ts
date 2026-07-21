export const TRANSCRIPT_SCROLL_ANCHOR_EPSILON_PX = 0.5;

export interface TranscriptScrollAnchorSnapshot {
  readonly element: HTMLElement;
  readonly viewportTop: number;
}

/**
 * Restore one stable transcript row to its pre-commit viewport coordinate.
 * The caller runs this in the same rendering turn as a tail lifecycle change,
 * after the scroller primitive has applied any bottom-follow correction.
 */
export function restoreTranscriptScrollAnchor(
  viewport: Pick<HTMLElement, "contains" | "scrollTop">,
  snapshot: TranscriptScrollAnchorSnapshot | null,
): number {
  if (
    !snapshot ||
    !snapshot.element.isConnected ||
    !viewport.contains(snapshot.element)
  ) {
    return 0;
  }

  const correction =
    snapshot.element.getBoundingClientRect().top - snapshot.viewportTop;
  if (Math.abs(correction) <= TRANSCRIPT_SCROLL_ANCHOR_EPSILON_PX) {
    return 0;
  }

  const previousScrollTop = viewport.scrollTop;
  viewport.scrollTop = previousScrollTop + correction;
  return viewport.scrollTop - previousScrollTop;
}

/** The in-flow tail row consumes one flex gap in addition to its own height. */
export function tailThinkingScrollReserve(
  rowHeight: number,
  rowGap: number,
  hasPreviousRenderableSibling: boolean,
): number {
  const height = Number.isFinite(rowHeight) ? Math.max(0, rowHeight) : 0;
  const gap =
    hasPreviousRenderableSibling && Number.isFinite(rowGap)
      ? Math.max(0, rowGap)
      : 0;
  return height + gap;
}
