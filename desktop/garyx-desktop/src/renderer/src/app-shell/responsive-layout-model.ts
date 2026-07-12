export const SINGLE_RAIL_COMPACT_WIDTH = 720;
export const DUAL_RAIL_COMPACT_WIDTH = 980;
export const TASK_TREE_DOCK_MIN_WIDTH = 1088;
export const SIDE_PANEL_MIN_MAIN_WIDTH = 540;
export const SIDE_PANEL_RESIZER_WIDTH = 10;
export const SIDEBAR_DEFAULT_WIDTH = 245;
export const SIDEBAR_MIN_WIDTH = 245;
export const SIDEBAR_MAX_WIDTH = 520;
export const CONVERSATION_RAIL_DEFAULT_WIDTH = 258;
export const CONVERSATION_RAIL_MIN_WIDTH = 220;
export const CONVERSATION_RAIL_MAX_WIDTH = 420;

export function clampSidebarWidth(width: number): number {
  return Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, width));
}

export function clampConversationRailWidth(width: number): number {
  return Math.max(
    CONVERSATION_RAIL_MIN_WIDTH,
    Math.min(CONVERSATION_RAIL_MAX_WIDTH, width),
  );
}

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

export function isDockedSidePanel({
  canvasWidth,
  panelWidth,
  minMainWidth = SIDE_PANEL_MIN_MAIN_WIDTH,
  resizerWidth = SIDE_PANEL_RESIZER_WIDTH,
}: {
  canvasWidth: number;
  panelWidth: number;
  minMainWidth?: number;
  resizerWidth?: number;
}): boolean {
  if (
    !Number.isFinite(canvasWidth) ||
    !Number.isFinite(panelWidth) ||
    canvasWidth <= 0 ||
    panelWidth <= 0
  ) {
    return false;
  }
  return canvasWidth >= panelWidth + minMainWidth + resizerWidth;
}
