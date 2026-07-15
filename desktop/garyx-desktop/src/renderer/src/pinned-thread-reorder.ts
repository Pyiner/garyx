export function reorderPinnedThreadIds(
  threadIds: readonly string[],
  activeThreadId: string,
  overThreadId: string | null,
): string[] | null {
  if (!overThreadId || activeThreadId === overThreadId) {
    return null;
  }
  const activeIndex = threadIds.indexOf(activeThreadId);
  const overIndex = threadIds.indexOf(overThreadId);
  if (activeIndex < 0 || overIndex < 0) {
    return null;
  }
  const reordered = [...threadIds];
  const [active] = reordered.splice(activeIndex, 1);
  reordered.splice(overIndex, 0, active);
  return reordered;
}

/**
 * Whether a drag session has become dangling and must be cancelled: the
 * pinned rows projection emptied (removing the DndContext subtree, which
 * fires no onDragCancel of its own) while a drag is still active. The
 * sidebar component stays mounted when rows empty — it merely renders null —
 * so this must be evaluated on rows change, not on unmount.
 */
export function shouldCancelDanglingDrag(
  rowCount: number,
  dragActive: boolean,
): boolean {
  return rowCount === 0 && dragActive;
}
