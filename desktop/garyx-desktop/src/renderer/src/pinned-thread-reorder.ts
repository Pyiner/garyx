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
