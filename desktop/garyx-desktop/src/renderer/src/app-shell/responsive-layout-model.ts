export const SINGLE_RAIL_COMPACT_WIDTH = 720;
export const DUAL_RAIL_COMPACT_WIDTH = 980;
export const TASK_TREE_DOCK_MIN_WIDTH = 1088;

export function responsiveSidebarBreakpoint(
  secondaryRailOpen: boolean,
): number {
  return secondaryRailOpen
    ? DUAL_RAIL_COMPACT_WIDTH
    : SINGLE_RAIL_COMPACT_WIDTH;
}

export function isCompactSidebarViewport({
  secondaryRailOpen,
  viewportWidth,
}: {
  secondaryRailOpen: boolean;
  viewportWidth: number;
}): boolean {
  return viewportWidth <= responsiveSidebarBreakpoint(secondaryRailOpen);
}

export function resolveSidebarCollapsed({
  compactOpen,
  compactViewport,
  userCollapsed,
}: {
  compactOpen: boolean;
  compactViewport: boolean;
  userCollapsed: boolean;
}): boolean {
  return compactViewport ? !compactOpen : userCollapsed;
}

export function isDockedTaskTree(threadWidth: number): boolean {
  return threadWidth >= TASK_TREE_DOCK_MIN_WIDTH;
}
