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

export const LEGACY_WINDOW_MIN_WIDTH = 1180;
export const EXPAND_V1_WINDOW_MIN_WIDTH = 480;
export const MIN_PRIMARY_THREAD_WIDTH = 350;
export const THREAD_LOG_DOCK_COMFORT_WIDTH = SIDE_PANEL_MIN_MAIN_WIDTH;
export const RIGHT_DOCK_AUTO_HIDE_WIDTH = 960;
export const SIDE_TOOLS_DEFAULT_WIDTH = 320;
export const SIDE_TOOLS_MIN_WIDTH = 320;
export const SIDE_TOOLS_MAX_WIDTH = 1180;
export const THREAD_LOGS_DEFAULT_WIDTH = 360;
export const THREAD_LOGS_MIN_WIDTH = 280;
export const THREAD_LOGS_MAX_WIDTH = 760;
export const LAYOUT_EDGE_TOLERANCE = 2;

export type LayoutPolicyName = "legacy" | "expand-v1";
export type LayoutPanelId =
  | "globalSidebar"
  | "conversationRail"
  | "sideTools"
  | "threadLogs";
export type LayoutIntentCause =
  | "user-panel"
  | "user-route"
  | "system-cleanup"
  | "hydrate";

export type LayoutPanelOccupancy = Readonly<{
  globalSidebar: boolean;
  conversationRail: boolean;
  sideTools: boolean;
  threadLogs: boolean;
}>;

export type LayoutWidths = Readonly<{
  globalSidebar: number;
  conversationRail: number;
  sideTools: number;
  threadLogs: number;
  sideToolsCustomized: boolean;
}>;

export type LayoutRectangle = Readonly<{
  x: number;
  y: number;
  width: number;
  height: number;
}>;

export type WindowMode = "normal" | "maximized" | "fullscreen";
export type WindowSnapshotOrigin =
  | "user"
  | "display"
  | "panel-machine"
  | "mode"
  | "hydrate";

export type WindowLayoutSnapshot = Readonly<{
  windowRevision: number;
  bounds: LayoutRectangle;
  contentBounds: LayoutRectangle;
  normalBounds: LayoutRectangle;
  workArea: LayoutRectangle;
  mode: WindowMode;
  displayId: string;
  scaleFactor: number;
  origin: WindowSnapshotOrigin;
}>;

export type UserCauseToken = Readonly<{
  kind: "user-cause";
  tokenId: string;
  transactionId: string;
  cause: "user-panel" | "user-route";
  rendererEpoch: string;
  sequence: number;
}>;

export type RepayProof = Readonly<{
  kind: "repay-proof";
  fundingIds: readonly string[];
  expectedSessionRevision: number;
}>;

export type BoundsAuthority = UserCauseToken | RepayProof;

export type ConfirmedFunding = Readonly<{
  fundingId: string;
  panel: LayoutPanelId;
  widthDelta: number;
  xCompensation: number;
  repayAuthority: Readonly<{ fundingId: string }>;
}>;

export type FundingByPanel = Readonly<
  Partial<Record<LayoutPanelId, ConfirmedFunding>>
>;

export type AcknowledgedLayoutSession = Readonly<{
  normalBaseBounds: LayoutRectangle;
  fundingByPanel: FundingByPanel;
  desiredOccupancy: LayoutPanelOccupancy;
  windowRevision: number;
  sessionRevision: number;
}>;

export type WindowLayoutSessionCommand = Readonly<{
  type: "CHECKPOINT_DESIRED_OCCUPANCY";
  expectedSessionRevision: number;
  desiredOccupancy: LayoutPanelOccupancy;
  transactionId: string;
  rendererEpoch: string;
  sequence: number;
}>;

export type WindowBoundsCommand = Readonly<{
  type: "APPLY_WINDOW_BOUNDS";
  authority: BoundsAuthority;
  expectedWindowRevision: number;
  expectedSessionRevision: number;
  targetBounds: LayoutRectangle;
  targetNormalBaseBounds: LayoutRectangle;
  targetFundingByPanel: FundingByPanel;
  targetDesiredOccupancy: LayoutPanelOccupancy;
  transactionId: string;
  rendererEpoch: string;
  sequence: number;
}>;

export type ClaimInitialLayoutCommand = Readonly<{
  type: "CLAIM_INITIAL_LAYOUT";
  expectedWindowRevision: number;
  expectedSessionRevision: number;
  targetNormalBaseBounds: LayoutRectangle;
  targetFundingByPanel: FundingByPanel;
  targetDesiredOccupancy: LayoutPanelOccupancy;
  transactionId: "claim-initial-layout";
  rendererEpoch: string;
  sequence: 0;
}>;

export type LayoutMachineEffect =
  | Readonly<{
      type: "window-layout-session";
      command: WindowLayoutSessionCommand;
    }>
  | Readonly<{ type: "window-bounds"; command: WindowBoundsCommand }>
  | Readonly<{
      type: "claim-initial-layout";
      command: ClaimInitialLayoutCommand;
    }>
  | Readonly<{
      type: "schedule-deadline";
      deadline: "open" | "close";
      transactionId: string;
    }>
  | Readonly<{ type: "request-frame-commit"; transactionId: string }>
  | Readonly<{
      type: "persist-preference";
      preference: "panel-width";
      panel: LayoutPanelId;
      value: number;
    }>
  | Readonly<{
      type: "diagnostic";
      code: string;
      transactionId?: string;
    }>;

export type LayoutPolicy = Readonly<{
  name: LayoutPolicyName;
  windowMinWidth: number;
  windowExpansionEnabled: boolean;
  sideToolsAutoHide: boolean;
}>;

const LAYOUT_POLICIES: Readonly<Record<LayoutPolicyName, LayoutPolicy>> = {
  legacy: {
    name: "legacy",
    windowMinWidth: LEGACY_WINDOW_MIN_WIDTH,
    windowExpansionEnabled: false,
    sideToolsAutoHide: false,
  },
  "expand-v1": {
    name: "expand-v1",
    windowMinWidth: EXPAND_V1_WINDOW_MIN_WIDTH,
    windowExpansionEnabled: true,
    sideToolsAutoHide: true,
  },
};

export function horizontalLayoutPolicy(name: LayoutPolicyName): LayoutPolicy {
  return LAYOUT_POLICIES[name];
}

export const CLOSED_LAYOUT_OCCUPANCY: LayoutPanelOccupancy = Object.freeze({
  globalSidebar: false,
  conversationRail: false,
  sideTools: false,
  threadLogs: false,
});

export const DEFAULT_LAYOUT_WIDTHS: LayoutWidths = Object.freeze({
  globalSidebar: SIDEBAR_DEFAULT_WIDTH,
  conversationRail: CONVERSATION_RAIL_DEFAULT_WIDTH,
  sideTools: SIDE_TOOLS_DEFAULT_WIDTH,
  threadLogs: THREAD_LOGS_DEFAULT_WIDTH,
  sideToolsCustomized: false,
});

export const HORIZONTAL_LAYOUT_PANEL_ORDER: readonly LayoutPanelId[] =
  Object.freeze([
    "globalSidebar",
    "conversationRail",
    "sideTools",
    "threadLogs",
  ]);

export type LayoutPresentationReason =
  | "requested"
  | "compact"
  | "auto-hidden"
  | "capacity"
  | "fixed-mode"
  | "closed";

export type HorizontalLayoutPresentation = Readonly<{
  globalSidebar: "expanded" | "collapsed" | "compact-overlay";
  conversationRail: "open" | "hidden" | "closed";
  sideTools: "docked" | "hidden" | "closed";
  threadLogs: "docked" | "overlay" | "closed";
  taskTree: "docked" | "overlay-closed" | "absent";
  taskTreeDocked: boolean;
  compactViewport: boolean;
  headerDensity: "compact" | "regular";
  reasons: Readonly<Record<LayoutPanelId, LayoutPresentationReason>>;
}>;

/**
 * One flattened list of every in-flow horizontal track. Nested grids are
 * deliberately flattened so the sum is a single, testable invariant.
 */
export type HorizontalLayoutColumns = Readonly<{
  globalSidebar: number;
  conversationRail: number;
  conversationDivider: number;
  primaryThread: number;
  sideToolsResizer: number;
  sideTools: number;
  threadLogsResizer: number;
  threadLogs: number;
}>;

export type HorizontalLayoutNestedColumns = Readonly<{
  shell: Readonly<{
    globalSidebar: number;
    conversationRail: number;
    main: number;
  }>;
  conversation: Readonly<{
    threadLayout: number;
    sideToolsResizer: number;
    sideTools: number;
  }>;
  thread: Readonly<{
    main: number;
    threadLogsResizer: number;
    threadLogs: number;
  }>;
}>;

export type HorizontalLayoutCssVariables = Readonly<{
  "--gx-sidebar-preferred-width": string;
  "--gx-conversation-rail-preferred-width": string;
  "--gx-side-tools-preferred-width": string;
  "--gx-thread-logs-preferred-width": string;
  "--gx-sidebar-width": string;
  "--gx-conversation-rail-width": string;
  "--gx-shell-main-width": string;
  "--gx-conversation-width": string;
  "--gx-right-resizer-width": string;
  "--gx-right-panel-width": string;
  "--gx-thread-main-width": string;
  "--gx-thread-log-resizer-width": string;
  "--gx-thread-log-panel-width": string;
}>;

export type HorizontalLayoutDataAttributes = Readonly<{
  "data-layout-policy": LayoutPolicyName;
  "data-layout-revision": string;
  "data-sidebar-state": HorizontalLayoutPresentation["globalSidebar"];
  "data-conversation-rail-state":
    HorizontalLayoutPresentation["conversationRail"];
  "data-side-tools-state": HorizontalLayoutPresentation["sideTools"];
  "data-thread-logs-presentation":
    HorizontalLayoutPresentation["threadLogs"];
  "data-task-tree-presentation": HorizontalLayoutPresentation["taskTree"];
  "data-header-density": HorizontalLayoutPresentation["headerDensity"];
}>;

export type StableHorizontalLayoutFrame = Readonly<{
  kind: "stable";
  policy: LayoutPolicyName;
  revision: number;
  contentViewportWidth: number;
  responsiveBasisWidth: number;
  requestedOccupancy: LayoutPanelOccupancy;
  effectiveOccupancy: LayoutPanelOccupancy;
  columns: HorizontalLayoutColumns;
  nestedColumns: HorizontalLayoutNestedColumns;
  presentation: HorizontalLayoutPresentation;
  primaryThreadWidth: number;
  threadMainWidth: number;
  cssVariables: HorizontalLayoutCssVariables;
  dataAttributes: HorizontalLayoutDataAttributes;
}>;

export type PendingHorizontalLayoutFrame = Readonly<{
  kind: "pending";
  policy: LayoutPolicyName;
  revision: number;
  transactionId: string;
  phase: LayoutTransactionPhase;
  fallbackVisible: boolean;
  frame: StableHorizontalLayoutFrame;
}>;

export type RejectedHorizontalLayoutFrame = Readonly<{
  kind: "rejected";
  policy: LayoutPolicyName;
  revision: number;
  reason: LayoutProjectionRejectionReason;
  triggerPanel: LayoutPanelId | null;
  frame: StableHorizontalLayoutFrame | null;
}>;

export type HorizontalLayoutFrame =
  | StableHorizontalLayoutFrame
  | PendingHorizontalLayoutFrame
  | RejectedHorizontalLayoutFrame;

export type LayoutProjectionRejectionReason =
  | "invalid-viewport"
  | "invalid-intent"
  | "trigger-capacity"
  | "protocol";

export type LayoutTransactionPhase =
  | "checkpoint-pending"
  | "preparing-open"
  | "awaiting-bounds"
  | "opening-fallback"
  | "closing-animation"
  | "frame-commit-pending"
  | "deferred-funding"
  | "deferred-reconcile"
  | "constrained"
  | "settled"
  | "rejected"
  | "superseded";

export type LayoutTransaction = Readonly<{
  transactionId: string;
  rendererEpoch: string;
  sequence: number;
  cause: LayoutIntentCause;
  previousOccupancy: LayoutPanelOccupancy;
  nextOccupancy: LayoutPanelOccupancy;
  openingPanels: readonly LayoutPanelId[];
  closingPanels: readonly LayoutPanelId[];
  triggerPanel: LayoutPanelId | null;
  authority: BoundsAuthority | null;
  phase: LayoutTransactionPhase;
  fallbackVisible: boolean;
  checkpointAttempts: number;
  supersededBy: string | null;
}>;

export type LayoutMachineDiagnostic = Readonly<{
  code: string;
  transactionId?: string;
}>;

export type HorizontalLayoutState = Readonly<{
  policy: LayoutPolicyName;
  rendererEpoch: string;
  revision: number;
  nextSequence: number;
  desiredOccupancy: LayoutPanelOccupancy;
  widths: LayoutWidths;
  compactSidebarOpen: boolean;
  sideToolsManualOverride: boolean;
  snapshot: WindowLayoutSnapshot;
  responsiveBasisWidth: number;
  acknowledgedSession: AcknowledgedLayoutSession;
  transactions: Readonly<Record<string, LayoutTransaction>>;
  headTransactionId: string | null;
  pendingInitialClaim: boolean;
  hydrated: boolean;
  diagnostics: readonly LayoutMachineDiagnostic[];
}>;

export type CreateHorizontalLayoutStateInput = Readonly<{
  policy: LayoutPolicyName;
  rendererEpoch: string;
  snapshot: WindowLayoutSnapshot;
  desiredOccupancy?: LayoutPanelOccupancy;
  widths?: Partial<LayoutWidths>;
  acknowledgedSession?: AcknowledgedLayoutSession;
  hydrated?: boolean;
}>;

function cloneOccupancy(
  occupancy: LayoutPanelOccupancy,
): LayoutPanelOccupancy {
  return {
    globalSidebar: occupancy.globalSidebar,
    conversationRail: occupancy.conversationRail,
    sideTools: occupancy.sideTools,
    threadLogs: occupancy.threadLogs,
  };
}

export function layoutOccupanciesEqual(
  left: LayoutPanelOccupancy,
  right: LayoutPanelOccupancy,
): boolean {
  return HORIZONTAL_LAYOUT_PANEL_ORDER.every(
    (panel) => left[panel] === right[panel],
  );
}

function defaultAcknowledgedSession(
  snapshot: WindowLayoutSnapshot,
  desiredOccupancy: LayoutPanelOccupancy,
): AcknowledgedLayoutSession {
  return {
    normalBaseBounds: snapshot.normalBounds,
    fundingByPanel: {},
    desiredOccupancy: cloneOccupancy(desiredOccupancy),
    windowRevision: snapshot.windowRevision,
    sessionRevision: 0,
  };
}

export function createHorizontalLayoutState({
  policy,
  rendererEpoch,
  snapshot,
  desiredOccupancy = CLOSED_LAYOUT_OCCUPANCY,
  widths,
  acknowledgedSession,
  hydrated = true,
}: CreateHorizontalLayoutStateInput): HorizontalLayoutState {
  const normalizedWidths: LayoutWidths = {
    ...DEFAULT_LAYOUT_WIDTHS,
    ...widths,
  };
  return {
    policy,
    rendererEpoch,
    revision: 0,
    nextSequence: 1,
    desiredOccupancy: cloneOccupancy(desiredOccupancy),
    widths: normalizedWidths,
    compactSidebarOpen: false,
    sideToolsManualOverride: false,
    snapshot,
    responsiveBasisWidth: snapshot.contentBounds.width,
    acknowledgedSession:
      acknowledgedSession ??
      defaultAcknowledgedSession(snapshot, desiredOccupancy),
    transactions: {},
    headTransactionId: null,
    pendingInitialClaim: false,
    hydrated,
    diagnostics: [],
  };
}

function finiteWidthOr(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

export function normalizeLayoutWidths(widths: LayoutWidths): LayoutWidths {
  return {
    globalSidebar: clampSidebarWidth(
      finiteWidthOr(widths.globalSidebar, SIDEBAR_DEFAULT_WIDTH),
    ),
    conversationRail: clampConversationRailWidth(
      finiteWidthOr(
        widths.conversationRail,
        CONVERSATION_RAIL_DEFAULT_WIDTH,
      ),
    ),
    sideTools: Math.max(
      SIDE_TOOLS_MIN_WIDTH,
      Math.min(
        SIDE_TOOLS_MAX_WIDTH,
        Math.round(finiteWidthOr(widths.sideTools, SIDE_TOOLS_DEFAULT_WIDTH)),
      ),
    ),
    threadLogs: Math.max(
      THREAD_LOGS_MIN_WIDTH,
      Math.min(
        THREAD_LOGS_MAX_WIDTH,
        Math.round(
          finiteWidthOr(widths.threadLogs, THREAD_LOGS_DEFAULT_WIDTH),
        ),
      ),
    ),
    sideToolsCustomized: widths.sideToolsCustomized,
  };
}

type SolvedLayout = Readonly<{
  frame: StableHorizontalLayoutFrame | null;
  rejection: LayoutProjectionRejectionReason | null;
  triggerPanel: LayoutPanelId | null;
}>;

function asPixels(value: number): string {
  return `${value}px`;
}

function sumColumns(columns: HorizontalLayoutColumns): number {
  const allocated =
    columns.globalSidebar +
    columns.conversationRail +
    columns.conversationDivider +
    columns.sideToolsResizer +
    columns.sideTools +
    columns.threadLogsResizer +
    columns.threadLogs;
  return allocated + columns.primaryThread;
}

export function horizontalLayoutColumnSum(
  frame: StableHorizontalLayoutFrame,
): number {
  return sumColumns(frame.columns);
}

function solveStableHorizontalLayout(
  state: HorizontalLayoutState,
  requestedOccupancy: LayoutPanelOccupancy,
  triggerPanel: LayoutPanelId | null,
): SolvedLayout {
  const viewportWidth = state.snapshot.contentBounds.width;
  if (
    !Number.isFinite(viewportWidth) ||
    viewportWidth < MIN_PRIMARY_THREAD_WIDTH
  ) {
    return {
      frame: null,
      rejection: "invalid-viewport",
      triggerPanel,
    };
  }
  if (requestedOccupancy.sideTools && requestedOccupancy.threadLogs) {
    return {
      frame: null,
      rejection: "invalid-intent",
      triggerPanel,
    };
  }

  const policy = horizontalLayoutPolicy(state.policy);
  const widths = normalizeLayoutWidths(state.widths);
  const compactViewport = isCompactSidebarViewport({
    secondaryRailOpen: requestedOccupancy.conversationRail,
    viewportWidth: state.responsiveBasisWidth,
  });
  let sidebar =
    requestedOccupancy.globalSidebar && !compactViewport
      ? widths.globalSidebar
      : 0;
  let rail = requestedOccupancy.conversationRail
    ? widths.conversationRail
    : 0;
  let divider = rail > 0 ? 1 : 0;
  const autoHideSideTools =
    policy.sideToolsAutoHide &&
    state.responsiveBasisWidth <= RIGHT_DOCK_AUTO_HIDE_WIDTH &&
    !state.sideToolsManualOverride;
  let sideTools =
    requestedOccupancy.sideTools && !autoHideSideTools
      ? widths.sideTools
      : 0;
  let sideToolsResizer = sideTools > 0 ? SIDE_PANEL_RESIZER_WIDTH : 0;

  const logsRequested = requestedOccupancy.threadLogs;
  const canvasBeforeLogs = viewportWidth - sidebar - rail - divider;
  let logsDocked =
    logsRequested &&
    canvasBeforeLogs >=
      widths.threadLogs +
        SIDE_PANEL_RESIZER_WIDTH +
        THREAD_LOG_DOCK_COMFORT_WIDTH;
  let threadLogs = logsDocked ? widths.threadLogs : 0;
  let threadLogsResizer = logsDocked ? SIDE_PANEL_RESIZER_WIDTH : 0;

  let primaryThread =
    viewportWidth -
    sidebar -
    rail -
    divider -
    sideToolsResizer -
    sideTools -
    threadLogsResizer -
    threadLogs;
  let capacityRejected = false;

  // The degradation order is contractual. It protects the most recent
  // explicit trigger but never mutates requested occupancy.
  if (primaryThread < MIN_PRIMARY_THREAD_WIDTH) {
    if (sideTools > 0 && triggerPanel !== "sideTools") {
      sideTools = 0;
      sideToolsResizer = 0;
    }
    primaryThread =
      viewportWidth -
      sidebar -
      rail -
      divider -
      sideToolsResizer -
      sideTools -
      threadLogsResizer -
      threadLogs;
  }
  if (primaryThread < MIN_PRIMARY_THREAD_WIDTH && sidebar > 0) {
    sidebar = 0;
    primaryThread =
      viewportWidth -
      rail -
      divider -
      sideToolsResizer -
      sideTools -
      threadLogsResizer -
      threadLogs;
  }
  if (primaryThread < MIN_PRIMARY_THREAD_WIDTH && logsDocked) {
    logsDocked = false;
    threadLogs = 0;
    threadLogsResizer = 0;
    primaryThread =
      viewportWidth - rail - divider - sideToolsResizer - sideTools;
  }
  if (
    primaryThread < MIN_PRIMARY_THREAD_WIDTH &&
    rail > 0 &&
    triggerPanel !== "conversationRail"
  ) {
    rail = 0;
    divider = 0;
    primaryThread = viewportWidth - sideToolsResizer - sideTools;
  }
  if (primaryThread < MIN_PRIMARY_THREAD_WIDTH) {
    capacityRejected = true;
    if (triggerPanel === "sideTools") {
      sideTools = 0;
      sideToolsResizer = 0;
    } else if (triggerPanel === "conversationRail") {
      rail = 0;
      divider = 0;
    }
    primaryThread =
      viewportWidth - rail - divider - sideToolsResizer - sideTools;
  }

  if (primaryThread < MIN_PRIMARY_THREAD_WIDTH) {
    return {
      frame: null,
      rejection: "invalid-viewport",
      triggerPanel,
    };
  }

  const allocatedNonPrimaryTracks =
    sidebar +
    rail +
    divider +
    sideToolsResizer +
    sideTools +
    threadLogsResizer +
    threadLogs;
  primaryThread = viewportWidth - allocatedNonPrimaryTracks;

  const sidebarPresentation: HorizontalLayoutPresentation["globalSidebar"] =
    compactViewport && state.compactSidebarOpen
      ? "compact-overlay"
      : sidebar > 0
        ? "expanded"
        : "collapsed";
  const conversationRailPresentation: HorizontalLayoutPresentation["conversationRail"] =
    !requestedOccupancy.conversationRail
      ? "closed"
      : rail > 0
        ? "open"
        : "hidden";
  const sideToolsPresentation: HorizontalLayoutPresentation["sideTools"] =
    !requestedOccupancy.sideTools
      ? "closed"
      : sideTools > 0
        ? "docked"
        : "hidden";
  const threadLogsPresentation: HorizontalLayoutPresentation["threadLogs"] =
    !logsRequested ? "closed" : logsDocked ? "docked" : "overlay";
  const rightPanelVisible =
    sideToolsPresentation === "docked" || logsRequested;
  const taskTreePresentation: HorizontalLayoutPresentation["taskTree"] =
    rightPanelVisible
      ? "absent"
      : isDockedTaskTree(primaryThread)
        ? "docked"
        : "overlay-closed";

  const columns: HorizontalLayoutColumns = {
    globalSidebar: sidebar,
    conversationRail: rail,
    conversationDivider: divider,
    sideToolsResizer,
    sideTools,
    threadLogsResizer,
    threadLogs,
    primaryThread,
  };
  const shellMain = viewportWidth - sidebar - rail;
  const conversationWidth = shellMain - divider;
  const threadLayout =
    conversationWidth - sideToolsResizer - sideTools;

  const presentation: HorizontalLayoutPresentation = {
    globalSidebar: sidebarPresentation,
    conversationRail: conversationRailPresentation,
    sideTools: sideToolsPresentation,
    threadLogs: threadLogsPresentation,
    taskTree: taskTreePresentation,
    taskTreeDocked: taskTreePresentation === "docked",
    compactViewport,
    headerDensity: compactViewport ? "compact" : "regular",
    reasons: {
      globalSidebar: !requestedOccupancy.globalSidebar
        ? "closed"
        : compactViewport
          ? "compact"
          : sidebar > 0
            ? "requested"
            : "capacity",
      conversationRail: !requestedOccupancy.conversationRail
        ? "closed"
        : rail > 0
          ? "requested"
          : "capacity",
      sideTools: !requestedOccupancy.sideTools
        ? "closed"
        : autoHideSideTools
          ? "auto-hidden"
          : sideTools > 0
            ? "requested"
            : "capacity",
      threadLogs: !logsRequested
        ? "closed"
        : logsDocked
          ? "requested"
          : "capacity",
    },
  };
  const effectiveOccupancy: LayoutPanelOccupancy = {
    globalSidebar:
      sidebarPresentation === "expanded" ||
      sidebarPresentation === "compact-overlay",
    conversationRail: conversationRailPresentation === "open",
    sideTools: sideToolsPresentation === "docked",
    threadLogs: threadLogsPresentation !== "closed",
  };
  const cssVariables: HorizontalLayoutCssVariables = {
    "--gx-sidebar-preferred-width": asPixels(widths.globalSidebar),
    "--gx-conversation-rail-preferred-width": asPixels(
      widths.conversationRail,
    ),
    "--gx-side-tools-preferred-width": asPixels(widths.sideTools),
    "--gx-thread-logs-preferred-width": asPixels(widths.threadLogs),
    "--gx-sidebar-width": asPixels(sidebar),
    "--gx-conversation-rail-width": asPixels(rail),
    "--gx-shell-main-width": asPixels(shellMain),
    "--gx-conversation-width": asPixels(conversationWidth),
    "--gx-right-resizer-width": asPixels(sideToolsResizer),
    "--gx-right-panel-width": asPixels(sideTools),
    "--gx-thread-main-width": asPixels(primaryThread),
    "--gx-thread-log-resizer-width": asPixels(threadLogsResizer),
    "--gx-thread-log-panel-width": asPixels(
      logsRequested ? widths.threadLogs : 0,
    ),
  };
  const dataAttributes: HorizontalLayoutDataAttributes = {
    "data-layout-policy": state.policy,
    "data-layout-revision": String(state.revision),
    "data-sidebar-state": sidebarPresentation,
    "data-conversation-rail-state": conversationRailPresentation,
    "data-side-tools-state": sideToolsPresentation,
    "data-thread-logs-presentation": threadLogsPresentation,
    "data-task-tree-presentation": taskTreePresentation,
    "data-header-density": presentation.headerDensity,
  };
  const frame: StableHorizontalLayoutFrame = {
    kind: "stable",
    policy: state.policy,
    revision: state.revision,
    contentViewportWidth: viewportWidth,
    responsiveBasisWidth: state.responsiveBasisWidth,
    requestedOccupancy: cloneOccupancy(requestedOccupancy),
    effectiveOccupancy,
    columns,
    nestedColumns: {
      shell: {
        globalSidebar: sidebar,
        conversationRail: rail,
        main: shellMain,
      },
      conversation: {
        threadLayout,
        sideToolsResizer,
        sideTools,
      },
      thread: {
        main: primaryThread,
        threadLogsResizer,
        threadLogs,
      },
    },
    presentation,
    primaryThreadWidth: primaryThread,
    threadMainWidth: primaryThread,
    cssVariables,
    dataAttributes,
  };
  if (Math.abs(sumColumns(columns) - viewportWidth) > 1e-7) {
    throw new Error("horizontal layout columns do not fill the viewport");
  }
  return {
    frame,
    rejection: capacityRejected ? "trigger-capacity" : null,
    triggerPanel,
  };
}

function activeHeadTransaction(
  state: HorizontalLayoutState,
): LayoutTransaction | null {
  if (!state.headTransactionId) {
    return null;
  }
  return state.transactions[state.headTransactionId] ?? null;
}

function projectionOccupancy(
  state: HorizontalLayoutState,
  transaction: LayoutTransaction | null,
): LayoutPanelOccupancy {
  if (!transaction || state.policy === "legacy") {
    return state.desiredOccupancy;
  }
  switch (transaction.phase) {
    case "checkpoint-pending":
    case "preparing-open":
    case "awaiting-bounds":
      return transaction.fallbackVisible
        ? transaction.nextOccupancy
        : transaction.previousOccupancy;
    case "closing-animation":
      return transaction.previousOccupancy;
    default:
      return transaction.nextOccupancy;
  }
}

export function projectHorizontalLayout(
  state: HorizontalLayoutState,
): HorizontalLayoutFrame {
  const transaction = activeHeadTransaction(state);
  const occupancy = projectionOccupancy(state, transaction);
  const triggerPanel =
    transaction &&
    !["settled", "rejected", "superseded"].includes(transaction.phase)
      ? transaction.triggerPanel
      : null;
  const solved = solveStableHorizontalLayout(
    state,
    occupancy,
    triggerPanel,
  );
  if (!solved.frame) {
    return {
      kind: "rejected",
      policy: state.policy,
      revision: state.revision,
      reason: solved.rejection ?? "invalid-viewport",
      triggerPanel: solved.triggerPanel,
      frame: null,
    };
  }
  if (solved.rejection) {
    return {
      kind: "rejected",
      policy: state.policy,
      revision: state.revision,
      reason: solved.rejection,
      triggerPanel: solved.triggerPanel,
      frame: solved.frame,
    };
  }
  if (transaction?.phase === "rejected") {
    return {
      kind: "rejected",
      policy: state.policy,
      revision: state.revision,
      reason: "protocol",
      triggerPanel: transaction.triggerPanel,
      frame: solved.frame,
    };
  }
  if (state.policy === "legacy") {
    return solved.frame;
  }
  if (
    transaction &&
    !["settled", "rejected", "superseded", "constrained"].includes(
      transaction.phase,
    )
  ) {
    return {
      kind: "pending",
      policy: state.policy,
      revision: state.revision,
      transactionId: transaction.transactionId,
      phase: transaction.phase,
      fallbackVisible: transaction.fallbackVisible,
      frame: solved.frame,
    };
  }
  return solved.frame;
}

export function stableFrameFromProjection(
  frame: HorizontalLayoutFrame,
): StableHorizontalLayoutFrame | null {
  return frame.kind === "stable" ? frame : frame.frame;
}

export type BoundsRejectionReason =
  | "stale"
  | "fixed-mode"
  | "outside-work-area"
  | "superseded"
  | "invalid";

export type SessionCheckpointRejectionReason =
  | "stale"
  | "superseded"
  | "invalid";

export type HorizontalLayoutEvent =
  | Readonly<{
      type: "HYDRATE";
      freshSession: boolean;
      snapshot: WindowLayoutSnapshot;
      desiredOccupancy?: LayoutPanelOccupancy;
      acknowledgedSession?: AcknowledgedLayoutSession;
    }>
  | Readonly<{
      type: "CLAIM_INITIAL_LAYOUT_APPLIED";
      rendererEpoch: string;
      acknowledgedSession: AcknowledgedLayoutSession;
      snapshot: WindowLayoutSnapshot;
    }>
  | Readonly<{
      type: "CLAIM_INITIAL_LAYOUT_REJECTED";
      rendererEpoch: string;
      reason: SessionCheckpointRejectionReason;
      acknowledgedSession: AcknowledgedLayoutSession;
      snapshot: WindowLayoutSnapshot;
    }>
  | Readonly<{
      type: "LAYOUT_INTENT_CHANGED";
      previousOccupancy: LayoutPanelOccupancy;
      nextOccupancy: LayoutPanelOccupancy;
      cause: LayoutIntentCause;
      transactionId: string;
    }>
  | Readonly<{
      type: "WINDOW_LAYOUT_SESSION_APPLIED";
      rendererEpoch: string;
      transactionId: string;
      acknowledgedSession: AcknowledgedLayoutSession;
    }>
  | Readonly<{
      type: "WINDOW_LAYOUT_SESSION_REJECTED";
      rendererEpoch: string;
      transactionId: string;
      reason: SessionCheckpointRejectionReason;
      acknowledgedSession: AcknowledgedLayoutSession;
    }>
  | Readonly<{
      type: "WINDOW_BOUNDS_APPLIED";
      rendererEpoch: string;
      transactionId: string;
      sequence: number;
      acknowledgedSession: AcknowledgedLayoutSession;
      snapshot: WindowLayoutSnapshot;
    }>
  | Readonly<{
      type: "WINDOW_BOUNDS_REJECTED";
      rendererEpoch: string;
      transactionId: string;
      sequence: number;
      reason: BoundsRejectionReason;
      acknowledgedSession: AcknowledgedLayoutSession;
      snapshot: WindowLayoutSnapshot;
    }>
  | Readonly<{
      type: "OPEN_DEADLINE_EXPIRED";
      transactionId: string;
    }>
  | Readonly<{
      type: "CLOSE_DEADLINE_EXPIRED";
      transactionId: string;
    }>
  | Readonly<{
      type: "PRESENTATION_ANIMATION_FINISHED";
      transactionId: string;
    }>
  | Readonly<{
      type: "FRAME_COMMITTED";
      transactionId: string;
    }>
  | Readonly<{
      type: "WINDOW_SNAPSHOT_CHANGED";
      snapshot: WindowLayoutSnapshot;
      acknowledgedSession?: AcknowledgedLayoutSession;
    }>
  | Readonly<{
      type: "VIEWPORT_RESIZED_DURING_NATIVE_SESSION";
      snapshot: WindowLayoutSnapshot;
      acknowledgedSession?: AcknowledgedLayoutSession;
    }>
  | Readonly<{
      type: "COMPACT_SIDEBAR_TOGGLED";
    }>
  | Readonly<{
      type: "PANEL_WIDTH_CHANGED";
      panel: LayoutPanelId;
      width: number;
      commit: boolean;
    }>;

export type HorizontalLayoutReduction = Readonly<{
  state: HorizontalLayoutState;
  effects: readonly LayoutMachineEffect[];
}>;

function cloneFundingByPanel(funding: FundingByPanel): FundingByPanel {
  const clone: Partial<Record<LayoutPanelId, ConfirmedFunding>> = {};
  for (const panel of HORIZONTAL_LAYOUT_PANEL_ORDER) {
    const entry = funding[panel];
    if (entry) {
      clone[panel] = { ...entry, repayAuthority: { ...entry.repayAuthority } };
    }
  }
  return clone;
}

export function totalConfirmedFunding(funding: FundingByPanel): number {
  return HORIZONTAL_LAYOUT_PANEL_ORDER.reduce(
    (total, panel) => total + (funding[panel]?.widthDelta ?? 0),
    0,
  );
}

function totalFundingXCompensation(funding: FundingByPanel): number {
  return HORIZONTAL_LAYOUT_PANEL_ORDER.reduce(
    (total, panel) => total + (funding[panel]?.xCompensation ?? 0),
    0,
  );
}

export function boundsForAcknowledgedSession(
  session: Pick<
    AcknowledgedLayoutSession,
    "normalBaseBounds" | "fundingByPanel"
  >,
): LayoutRectangle {
  const base = session.normalBaseBounds;
  return {
    x: base.x + totalFundingXCompensation(session.fundingByPanel),
    y: base.y,
    width: base.width + totalConfirmedFunding(session.fundingByPanel),
    height: base.height,
  };
}

function fundingEntriesEqual(
  left: ConfirmedFunding | undefined,
  right: ConfirmedFunding | undefined,
): boolean {
  if (!left || !right) {
    return left === right;
  }
  return (
    left.fundingId === right.fundingId &&
    left.panel === right.panel &&
    left.widthDelta === right.widthDelta &&
    left.xCompensation === right.xCompensation &&
    left.repayAuthority.fundingId === right.repayAuthority.fundingId
  );
}

export function fundingMapsEqual(
  left: FundingByPanel,
  right: FundingByPanel,
): boolean {
  return HORIZONTAL_LAYOUT_PANEL_ORDER.every((panel) =>
    fundingEntriesEqual(left[panel], right[panel]),
  );
}

function rectanglesEqual(
  left: LayoutRectangle,
  right: LayoutRectangle,
): boolean {
  return (
    left.x === right.x &&
    left.y === right.y &&
    left.width === right.width &&
    left.height === right.height
  );
}

export type BoundsAuthorityValidation = Readonly<{
  valid: boolean;
  reason?: string;
}>;

/**
 * Renderer-side mirror of the executor's authority gate. The main process is
 * still the enforcement boundary in Phase 4; keeping this pure validator in
 * the model makes illegal commands unrepresentable in reducer tests today.
 */
export function validateBoundsCommandAuthority(
  command: WindowBoundsCommand,
  acknowledgedSession: AcknowledgedLayoutSession,
): BoundsAuthorityValidation {
  const authority = command.authority;
  if (
    command.expectedWindowRevision !== acknowledgedSession.windowRevision ||
    command.expectedSessionRevision !== acknowledgedSession.sessionRevision
  ) {
    return { valid: false, reason: "command revisions are stale" };
  }
  if (
    !rectanglesEqual(
      command.targetNormalBaseBounds,
      acknowledgedSession.normalBaseBounds,
    )
  ) {
    return { valid: false, reason: "command rewrites the normal base" };
  }
  const derivedTargetBounds = boundsForAcknowledgedSession({
    normalBaseBounds: command.targetNormalBaseBounds,
    fundingByPanel: command.targetFundingByPanel,
  });
  if (!rectanglesEqual(command.targetBounds, derivedTargetBounds)) {
    return { valid: false, reason: "target bounds and funding disagree" };
  }
  if (authority.kind === "user-cause") {
    if (
      authority.cause !== "user-panel" &&
      authority.cause !== "user-route"
    ) {
      return { valid: false, reason: "user token has a non-user cause" };
    }
    if (
      authority.transactionId !== command.transactionId ||
      authority.rendererEpoch !== command.rendererEpoch ||
      authority.sequence !== command.sequence
    ) {
      return { valid: false, reason: "user token does not own the command" };
    }
    return { valid: true };
  }

  if (
    authority.expectedSessionRevision !== command.expectedSessionRevision ||
    authority.expectedSessionRevision !==
      acknowledgedSession.sessionRevision
  ) {
    return { valid: false, reason: "repay proof revision is stale" };
  }
  if (authority.fundingIds.length === 0) {
    return { valid: false, reason: "repay proof is empty" };
  }
  const proofIds = new Set(authority.fundingIds);
  if (proofIds.size !== authority.fundingIds.length) {
    return { valid: false, reason: "repay proof contains duplicates" };
  }
  for (const panel of HORIZONTAL_LAYOUT_PANEL_ORDER) {
    const current = acknowledgedSession.fundingByPanel[panel];
    const target = command.targetFundingByPanel[panel];
    if (!current && target) {
      return { valid: false, reason: "repay proof cannot add funding" };
    }
    if (!current) {
      continue;
    }
    const changed = !fundingEntriesEqual(current, target);
    if (changed && target) {
      return { valid: false, reason: "repay proof cannot rewrite funding" };
    }
    if (changed && !proofIds.has(current.fundingId)) {
      return { valid: false, reason: "repay proof does not cover a change" };
    }
    if (!changed && proofIds.has(current.fundingId)) {
      return { valid: false, reason: "repay proof does not repay its funding" };
    }
  }
  for (const fundingId of proofIds) {
    const current = HORIZONTAL_LAYOUT_PANEL_ORDER.find(
      (panel) =>
        acknowledgedSession.fundingByPanel[panel]?.fundingId === fundingId,
    );
    if (!current) {
      return { valid: false, reason: "repay proof cites unknown funding" };
    }
  }
  if (
    totalConfirmedFunding(command.targetFundingByPanel) >
    totalConfirmedFunding(acknowledgedSession.fundingByPanel)
  ) {
    return { valid: false, reason: "repay proof cannot expand bounds" };
  }
  const currentBounds = boundsForAcknowledgedSession(acknowledgedSession);
  if (command.targetBounds.width > currentBounds.width) {
    return { valid: false, reason: "repay proof target expands bounds" };
  }
  return { valid: true };
}

function rectangleContainedBy(
  inner: LayoutRectangle,
  outer: LayoutRectangle,
): boolean {
  return (
    inner.x >= outer.x &&
    inner.y >= outer.y &&
    inner.x + inner.width <= outer.x + outer.width &&
    inner.y + inner.height <= outer.y + outer.height
  );
}

export function canExpandWindowForLayout({
  snapshot,
  targetBounds,
  deltaWidth,
}: {
  snapshot: WindowLayoutSnapshot;
  targetBounds: LayoutRectangle;
  deltaWidth: number;
}): boolean {
  if (snapshot.mode !== "normal" || deltaWidth < 0) {
    return false;
  }
  const leftGap = snapshot.bounds.x - snapshot.workArea.x;
  const rightGap =
    snapshot.workArea.x +
    snapshot.workArea.width -
    (snapshot.bounds.x + snapshot.bounds.width);
  return (
    leftGap > LAYOUT_EDGE_TOLERANCE &&
    rightGap >= deltaWidth + LAYOUT_EDGE_TOLERANCE &&
    rectangleContainedBy(targetBounds, snapshot.workArea)
  );
}

function checkpointEffect(
  state: HorizontalLayoutState,
  transaction: LayoutTransaction,
): LayoutMachineEffect {
  return {
    type: "window-layout-session",
    command: {
      type: "CHECKPOINT_DESIRED_OCCUPANCY",
      expectedSessionRevision:
        state.acknowledgedSession.sessionRevision,
      desiredOccupancy: cloneOccupancy(transaction.nextOccupancy),
      transactionId: transaction.transactionId,
      rendererEpoch: state.rendererEpoch,
      sequence: transaction.sequence,
    },
  };
}

function updateTransaction(
  state: HorizontalLayoutState,
  transactionId: string,
  patch: Partial<LayoutTransaction>,
): HorizontalLayoutState {
  const current = state.transactions[transactionId];
  if (!current) {
    return state;
  }
  return withRevision({
    ...state,
    transactions: {
      ...state.transactions,
      [transactionId]: { ...current, ...patch },
    },
  });
}

function addDiagnostic(
  state: HorizontalLayoutState,
  code: string,
  transactionId?: string,
): HorizontalLayoutState {
  return {
    ...state,
    diagnostics: [
      ...state.diagnostics,
      transactionId ? { code, transactionId } : { code },
    ].slice(-64),
  };
}

function withRevision(state: HorizontalLayoutState): HorizontalLayoutState {
  return { ...state, revision: state.revision + 1 };
}

function eventEpochMatches(
  state: HorizontalLayoutState,
  rendererEpoch: string,
): boolean {
  return state.rendererEpoch === rendererEpoch;
}

function foldAcknowledgedFacts(
  state: HorizontalLayoutState,
  acknowledgedSession: AcknowledgedLayoutSession,
  snapshot?: WindowLayoutSnapshot,
): HorizontalLayoutState {
  let next = state;
  if (
    acknowledgedSession.sessionRevision >
      state.acknowledgedSession.sessionRevision ||
    acknowledgedSession.windowRevision >
      state.acknowledgedSession.windowRevision
  ) {
    next = {
      ...next,
      acknowledgedSession: {
        ...acknowledgedSession,
        normalBaseBounds: { ...acknowledgedSession.normalBaseBounds },
        fundingByPanel: cloneFundingByPanel(
          acknowledgedSession.fundingByPanel,
        ),
        desiredOccupancy: cloneOccupancy(
          acknowledgedSession.desiredOccupancy,
        ),
      },
    };
  }
  if (snapshot && snapshot.windowRevision > next.snapshot.windowRevision) {
    next = { ...next, snapshot: { ...snapshot } };
  }
  return next === state ? state : withRevision(next);
}

function changedPanels(
  previous: LayoutPanelOccupancy,
  next: LayoutPanelOccupancy,
  open: boolean,
): readonly LayoutPanelId[] {
  return HORIZONTAL_LAYOUT_PANEL_ORDER.filter(
    (panel) => previous[panel] !== next[panel] && next[panel] === open,
  );
}

function requestedPanelContribution(
  state: HorizontalLayoutState,
  occupancy: LayoutPanelOccupancy,
  panel: LayoutPanelId,
): number {
  const widths = normalizeLayoutWidths(state.widths);
  if (!occupancy[panel]) {
    return 0;
  }
  if (panel === "globalSidebar") {
    return isCompactSidebarViewport({
      secondaryRailOpen: occupancy.conversationRail,
      viewportWidth: state.responsiveBasisWidth,
    })
      ? 0
      : widths.globalSidebar;
  }
  if (panel === "conversationRail") {
    return widths.conversationRail;
  }
  if (panel === "sideTools") {
    const autoHidden =
      horizontalLayoutPolicy(state.policy).sideToolsAutoHide &&
      state.responsiveBasisWidth <= RIGHT_DOCK_AUTO_HIDE_WIDTH &&
      !state.sideToolsManualOverride;
    return autoHidden ? 0 : widths.sideTools + SIDE_PANEL_RESIZER_WIDTH;
  }

  const sidebar = requestedPanelContribution(
    state,
    occupancy,
    "globalSidebar",
  );
  const rail = requestedPanelContribution(
    state,
    occupancy,
    "conversationRail",
  );
  const divider = rail > 0 ? 1 : 0;
  const mainBeforeLogs =
    state.snapshot.contentBounds.width - sidebar - rail - divider;
  return mainBeforeLogs >= THREAD_LOG_DOCK_COMFORT_WIDTH
    ? widths.threadLogs + SIDE_PANEL_RESIZER_WIDTH
    : 0;
}

function makeFunding(
  transaction: LayoutTransaction,
  panel: LayoutPanelId,
  widthDelta: number,
): ConfirmedFunding {
  const fundingId = `${transaction.rendererEpoch}:${transaction.sequence}:${panel}`;
  return {
    fundingId,
    panel,
    widthDelta,
    xCompensation: 0,
    repayAuthority: { fundingId },
  };
}

function targetFundingForTransaction(
  state: HorizontalLayoutState,
  transaction: LayoutTransaction,
  includeOpeningFunding: boolean,
): FundingByPanel {
  const target = cloneFundingByPanel(
    state.acknowledgedSession.fundingByPanel,
  ) as Partial<Record<LayoutPanelId, ConfirmedFunding>>;
  for (const panel of HORIZONTAL_LAYOUT_PANEL_ORDER) {
    if (!transaction.nextOccupancy[panel]) {
      delete target[panel];
    }
  }
  if (includeOpeningFunding) {
    for (const panel of transaction.openingPanels) {
      if (target[panel]) {
        continue;
      }
      const widthDelta = requestedPanelContribution(
        state,
        transaction.nextOccupancy,
        panel,
      );
      if (widthDelta > 0) {
        target[panel] = makeFunding(transaction, panel, widthDelta);
      }
    }
  }
  return target;
}

function removedFundingIds(
  current: FundingByPanel,
  target: FundingByPanel,
): readonly string[] {
  const ids: string[] = [];
  for (const panel of HORIZONTAL_LAYOUT_PANEL_ORDER) {
    const entry = current[panel];
    if (entry && !fundingEntriesEqual(entry, target[panel])) {
      ids.push(entry.fundingId);
    }
  }
  return ids;
}

function authorityForFundingTarget(
  state: HorizontalLayoutState,
  transaction: LayoutTransaction,
  targetFunding: FundingByPanel,
): BoundsAuthority | null {
  if (transaction.authority?.kind === "user-cause") {
    return transaction.authority;
  }
  const fundingIds = removedFundingIds(
    state.acknowledgedSession.fundingByPanel,
    targetFunding,
  );
  return fundingIds.length > 0
    ? {
        kind: "repay-proof",
        fundingIds,
        expectedSessionRevision:
          state.acknowledgedSession.sessionRevision,
      }
    : null;
}

function boundsCommandForTarget(
  state: HorizontalLayoutState,
  transaction: LayoutTransaction,
  targetFundingByPanel: FundingByPanel,
): WindowBoundsCommand | null {
  const authority = authorityForFundingTarget(
    state,
    transaction,
    targetFundingByPanel,
  );
  if (!authority) {
    return null;
  }
  const targetBounds = boundsForAcknowledgedSession({
    normalBaseBounds: state.acknowledgedSession.normalBaseBounds,
    fundingByPanel: targetFundingByPanel,
  });
  const command: WindowBoundsCommand = {
    type: "APPLY_WINDOW_BOUNDS",
    authority,
    expectedWindowRevision:
      state.acknowledgedSession.windowRevision,
    expectedSessionRevision:
      state.acknowledgedSession.sessionRevision,
    targetBounds,
    targetNormalBaseBounds: {
      ...state.acknowledgedSession.normalBaseBounds,
    },
    targetFundingByPanel: cloneFundingByPanel(targetFundingByPanel),
    targetDesiredOccupancy: cloneOccupancy(state.desiredOccupancy),
    transactionId: transaction.transactionId,
    rendererEpoch: state.rendererEpoch,
    sequence: transaction.sequence,
  };
  const validation = validateBoundsCommandAuthority(
    command,
    state.acknowledgedSession,
  );
  return validation.valid ? command : null;
}

function shouldDeferForFixedMode(state: HorizontalLayoutState): boolean {
  return state.snapshot.mode !== "normal";
}

function transitionNeedsOpeningFunding(
  state: HorizontalLayoutState,
  transaction: LayoutTransaction,
): boolean {
  return transaction.openingPanels.some(
    (panel) =>
      !state.acknowledgedSession.fundingByPanel[panel] &&
      requestedPanelContribution(
        state,
        transaction.nextOccupancy,
        panel,
      ) > 0,
  );
}

function transitionHasFundedClose(
  state: HorizontalLayoutState,
  transaction: LayoutTransaction,
): boolean {
  return transaction.closingPanels.some(
    (panel) => Boolean(state.acknowledgedSession.fundingByPanel[panel]),
  );
}

function planBoundsAfterFrameCommit(
  state: HorizontalLayoutState,
  transaction: LayoutTransaction,
): HorizontalLayoutReduction {
  if (!horizontalLayoutPolicy(state.policy).windowExpansionEnabled) {
    return {
      state: updateTransaction(state, transaction.transactionId, {
        phase: "settled",
      }),
      effects: [],
    };
  }
  if (shouldDeferForFixedMode(state)) {
    return {
      state: updateTransaction(state, transaction.transactionId, {
        phase: transitionHasFundedClose(state, transaction)
          ? "deferred-reconcile"
          : "deferred-funding",
      }),
      effects: [],
    };
  }

  let targetFunding = targetFundingForTransaction(state, transaction, true);
  const currentFunding = state.acknowledgedSession.fundingByPanel;
  const currentTotal = totalConfirmedFunding(currentFunding);
  let targetTotal = totalConfirmedFunding(targetFunding);
  const targetBounds = boundsForAcknowledgedSession({
    normalBaseBounds: state.acknowledgedSession.normalBaseBounds,
    fundingByPanel: targetFunding,
  });
  const addedFunding = targetTotal > currentTotal;
  if (
    addedFunding &&
    !canExpandWindowForLayout({
      snapshot: state.snapshot,
      targetBounds,
      deltaWidth: targetTotal - currentTotal,
    })
  ) {
    targetFunding = targetFundingForTransaction(state, transaction, false);
  }
  if (fundingMapsEqual(currentFunding, targetFunding)) {
    return {
      state: updateTransaction(state, transaction.transactionId, {
        phase: "constrained",
      }),
      effects: [],
    };
  }
  const command = boundsCommandForTarget(state, transaction, targetFunding);
  if (!command) {
    const diagnosed = addDiagnostic(
      updateTransaction(state, transaction.transactionId, {
        phase: "rejected",
      }),
      "invalid-bounds-authority",
      transaction.transactionId,
    );
    return {
      state: diagnosed,
      effects: [
        {
          type: "diagnostic",
          code: "invalid-bounds-authority",
          transactionId: transaction.transactionId,
        },
      ],
    };
  }
  return {
    state: updateTransaction(state, transaction.transactionId, {
      phase: "awaiting-bounds",
      authority: command.authority,
    }),
    effects: [{ type: "window-bounds", command }],
  };
}

function planAfterCheckpoint(
  state: HorizontalLayoutState,
  transaction: LayoutTransaction,
): HorizontalLayoutReduction {
  if (!horizontalLayoutPolicy(state.policy).windowExpansionEnabled) {
    return {
      state: updateTransaction(state, transaction.transactionId, {
        phase: "settled",
      }),
      effects: [],
    };
  }

  if (transitionHasFundedClose(state, transaction)) {
    const targetWithoutOpening = targetFundingForTransaction(
      state,
      transaction,
      false,
    );
    const continuationAuthority = authorityForFundingTarget(
      state,
      transaction,
      targetWithoutOpening,
    );
    return {
      state: updateTransaction(state, transaction.transactionId, {
        phase: "closing-animation",
        authority: continuationAuthority ?? transaction.authority,
      }),
      effects: [
        {
          type: "schedule-deadline",
          deadline: "close",
          transactionId: transaction.transactionId,
        },
      ],
    };
  }
  if (transaction.cause === "hydrate") {
    return {
      state: updateTransaction(state, transaction.transactionId, {
        phase: "constrained",
      }),
      effects: [],
    };
  }
  if (shouldDeferForFixedMode(state)) {
    return {
      state: updateTransaction(state, transaction.transactionId, {
        phase: transitionNeedsOpeningFunding(state, transaction)
          ? "deferred-funding"
          : "constrained",
      }),
      effects: [],
    };
  }
  if (!transitionNeedsOpeningFunding(state, transaction)) {
    return {
      state: updateTransaction(state, transaction.transactionId, {
        phase: "constrained",
      }),
      effects: [],
    };
  }

  const targetFunding = targetFundingForTransaction(state, transaction, true);
  const currentTotal = totalConfirmedFunding(
    state.acknowledgedSession.fundingByPanel,
  );
  const targetTotal = totalConfirmedFunding(targetFunding);
  const targetBounds = boundsForAcknowledgedSession({
    normalBaseBounds: state.acknowledgedSession.normalBaseBounds,
    fundingByPanel: targetFunding,
  });
  if (
    !canExpandWindowForLayout({
      snapshot: state.snapshot,
      targetBounds,
      deltaWidth: targetTotal - currentTotal,
    })
  ) {
    return {
      state: updateTransaction(state, transaction.transactionId, {
        phase: "constrained",
      }),
      effects: [],
    };
  }
  const command = boundsCommandForTarget(state, transaction, targetFunding);
  if (!command) {
    const diagnosed = addDiagnostic(
      updateTransaction(state, transaction.transactionId, {
        phase: "rejected",
      }),
      "invalid-bounds-authority",
      transaction.transactionId,
    );
    return {
      state: diagnosed,
      effects: [
        {
          type: "diagnostic",
          code: "invalid-bounds-authority",
          transactionId: transaction.transactionId,
        },
      ],
    };
  }
  return {
    state: updateTransaction(state, transaction.transactionId, {
      phase: "awaiting-bounds",
    }),
    effects: [
      { type: "window-bounds", command },
      {
        type: "schedule-deadline",
        deadline: "open",
        transactionId: transaction.transactionId,
      },
    ],
  };
}

function initialClaimFunding(
  state: HorizontalLayoutState,
  desiredOccupancy: LayoutPanelOccupancy,
): FundingByPanel {
  const solved = solveStableHorizontalLayout(state, desiredOccupancy, null);
  if (!solved.frame) {
    return {};
  }
  const contributions: Readonly<Record<LayoutPanelId, number>> = {
    globalSidebar: solved.frame.columns.globalSidebar,
    conversationRail: solved.frame.columns.conversationRail,
    sideTools:
      solved.frame.columns.sideToolsResizer +
      solved.frame.columns.sideTools,
    threadLogs:
      solved.frame.columns.threadLogsResizer +
      solved.frame.columns.threadLogs,
  };
  const normalWidth = state.snapshot.normalBounds.width;
  let available = Math.max(
    0,
    normalWidth - horizontalLayoutPolicy(state.policy).windowMinWidth,
  );
  const funding: Partial<Record<LayoutPanelId, ConfirmedFunding>> = {};
  for (const panel of HORIZONTAL_LAYOUT_PANEL_ORDER) {
    const widthDelta = Math.min(contributions[panel], available);
    if (widthDelta <= 0) {
      continue;
    }
    const fundingId = `claim:${state.rendererEpoch}:${panel}`;
    funding[panel] = {
      fundingId,
      panel,
      widthDelta,
      xCompensation: 0,
      repayAuthority: { fundingId },
    };
    available -= widthDelta;
  }
  return funding;
}

function initialClaimCommand(
  state: HorizontalLayoutState,
): ClaimInitialLayoutCommand {
  const targetFundingByPanel = initialClaimFunding(
    state,
    state.desiredOccupancy,
  );
  const claimedWidth = totalConfirmedFunding(targetFundingByPanel);
  const normalBounds = state.snapshot.normalBounds;
  return {
    type: "CLAIM_INITIAL_LAYOUT",
    expectedWindowRevision: state.snapshot.windowRevision,
    expectedSessionRevision:
      state.acknowledgedSession.sessionRevision,
    targetNormalBaseBounds: {
      ...normalBounds,
      width: normalBounds.width - claimedWidth,
    },
    targetFundingByPanel,
    targetDesiredOccupancy: cloneOccupancy(state.desiredOccupancy),
    transactionId: "claim-initial-layout",
    rendererEpoch: state.rendererEpoch,
    sequence: 0,
  };
}

function hydrateOrphanedFunding(
  state: HorizontalLayoutState,
): HorizontalLayoutReduction {
  const orphanedPanels = HORIZONTAL_LAYOUT_PANEL_ORDER.filter(
    (panel) =>
      Boolean(state.acknowledgedSession.fundingByPanel[panel]) &&
      !state.desiredOccupancy[panel],
  );
  if (orphanedPanels.length === 0) {
    return { state, effects: [] };
  }
  const transactionId = "hydrate-orphaned-funding";
  const transaction: LayoutTransaction = {
    transactionId,
    rendererEpoch: state.rendererEpoch,
    sequence: 0,
    cause: "hydrate",
    previousOccupancy: cloneOccupancy(state.desiredOccupancy),
    nextOccupancy: cloneOccupancy(state.desiredOccupancy),
    openingPanels: [],
    closingPanels: orphanedPanels,
    triggerPanel: null,
    authority: {
      kind: "repay-proof",
      fundingIds: orphanedPanels.map(
        (panel) =>
          state.acknowledgedSession.fundingByPanel[panel]!.fundingId,
      ),
      expectedSessionRevision:
        state.acknowledgedSession.sessionRevision,
    },
    phase:
      state.snapshot.mode === "normal"
        ? "awaiting-bounds"
        : "deferred-reconcile",
    fallbackVisible: true,
    checkpointAttempts: 0,
    supersededBy: null,
  };
  let next: HorizontalLayoutState = {
    ...state,
    transactions: { [transactionId]: transaction },
    headTransactionId: transactionId,
  };
  if (state.snapshot.mode !== "normal") {
    return { state: next, effects: [] };
  }
  const targetFunding = cloneFundingByPanel(
    state.acknowledgedSession.fundingByPanel,
  ) as Partial<Record<LayoutPanelId, ConfirmedFunding>>;
  for (const panel of orphanedPanels) {
    delete targetFunding[panel];
  }
  const command = boundsCommandForTarget(next, transaction, targetFunding);
  if (!command) {
    next = addDiagnostic(
      updateTransaction(next, transactionId, { phase: "rejected" }),
      "invalid-orphaned-repay",
      transactionId,
    );
    return {
      state: next,
      effects: [
        {
          type: "diagnostic",
          code: "invalid-orphaned-repay",
          transactionId,
        },
      ],
    };
  }
  next = updateTransaction(next, transactionId, {
    authority: command.authority,
  });
  return {
    state: next,
    effects: [{ type: "window-bounds", command }],
  };
}

function hydrateState(
  state: HorizontalLayoutState,
  event: Extract<HorizontalLayoutEvent, { type: "HYDRATE" }>,
): HorizontalLayoutReduction {
  if (event.freshSession) {
    const desired = cloneOccupancy(
      event.desiredOccupancy ?? CLOSED_LAYOUT_OCCUPANCY,
    );
    const fresh: HorizontalLayoutState = {
      ...state,
      revision: state.revision + 1,
      nextSequence: 1,
      desiredOccupancy: desired,
      compactSidebarOpen: false,
      sideToolsManualOverride: false,
      snapshot: { ...event.snapshot },
      responsiveBasisWidth: event.snapshot.contentBounds.width,
      acknowledgedSession: defaultAcknowledgedSession(
        event.snapshot,
        desired,
      ),
      transactions: {},
      headTransactionId: null,
      pendingInitialClaim: true,
      hydrated: true,
      diagnostics: [],
    };
    return {
      state: fresh,
      effects: [
        {
          type: "claim-initial-layout",
          command: initialClaimCommand(fresh),
        },
      ],
    };
  }
  if (!event.acknowledgedSession) {
    const diagnosed = addDiagnostic(
      state,
      "hydrate-missing-acknowledged-session",
    );
    return {
      state: diagnosed,
      effects: [
        {
          type: "diagnostic",
          code: "hydrate-missing-acknowledged-session",
        },
      ],
    };
  }
  const session = event.acknowledgedSession;
  const reloaded: HorizontalLayoutState = {
    ...state,
    revision: state.revision + 1,
    nextSequence: 1,
    desiredOccupancy: cloneOccupancy(session.desiredOccupancy),
    compactSidebarOpen: false,
    sideToolsManualOverride: false,
    snapshot: { ...event.snapshot },
    responsiveBasisWidth: event.snapshot.contentBounds.width,
    acknowledgedSession: {
      ...session,
      normalBaseBounds: { ...session.normalBaseBounds },
      fundingByPanel: cloneFundingByPanel(session.fundingByPanel),
      desiredOccupancy: cloneOccupancy(session.desiredOccupancy),
    },
    transactions: {},
    headTransactionId: null,
    pendingInitialClaim: false,
    hydrated: true,
    diagnostics: [],
  };
  return hydrateOrphanedFunding(reloaded);
}

function startIntentTransaction(
  state: HorizontalLayoutState,
  event: Extract<
    HorizontalLayoutEvent,
    { type: "LAYOUT_INTENT_CHANGED" }
  >,
): HorizontalLayoutReduction {
  if (state.transactions[event.transactionId]) {
    const diagnosed = addDiagnostic(
      state,
      "duplicate-layout-transaction",
      event.transactionId,
    );
    return {
      state: diagnosed,
      effects: [
        {
          type: "diagnostic",
          code: "duplicate-layout-transaction",
          transactionId: event.transactionId,
        },
      ],
    };
  }
  if (!layoutOccupanciesEqual(state.desiredOccupancy, event.previousOccupancy)) {
    const diagnosed = addDiagnostic(
      state,
      "layout-intent-previous-mismatch",
      event.transactionId,
    );
    return {
      state: diagnosed,
      effects: [
        {
          type: "diagnostic",
          code: "layout-intent-previous-mismatch",
          transactionId: event.transactionId,
        },
      ],
    };
  }
  if (event.nextOccupancy.sideTools && event.nextOccupancy.threadLogs) {
    const diagnosed = addDiagnostic(
      state,
      "mutually-exclusive-right-panels",
      event.transactionId,
    );
    return {
      state: diagnosed,
      effects: [
        {
          type: "diagnostic",
          code: "mutually-exclusive-right-panels",
          transactionId: event.transactionId,
        },
      ],
    };
  }
  const openingPanels = changedPanels(
    event.previousOccupancy,
    event.nextOccupancy,
    true,
  );
  const closingPanels = changedPanels(
    event.previousOccupancy,
    event.nextOccupancy,
    false,
  );
  if (event.cause === "system-cleanup" && openingPanels.length > 0) {
    const diagnosed = addDiagnostic(
      state,
      "cleanup-cannot-open-panel",
      event.transactionId,
    );
    return {
      state: diagnosed,
      effects: [
        {
          type: "diagnostic",
          code: "cleanup-cannot-open-panel",
          transactionId: event.transactionId,
        },
      ],
    };
  }
  const sequence = state.nextSequence;
  const authority: UserCauseToken | null =
    event.cause === "user-panel" || event.cause === "user-route"
      ? {
          kind: "user-cause",
          tokenId: `${state.rendererEpoch}:${sequence}:${event.transactionId}`,
          transactionId: event.transactionId,
          cause: event.cause,
          rendererEpoch: state.rendererEpoch,
          sequence,
        }
      : null;
  const booleanNoop = layoutOccupanciesEqual(
    event.previousOccupancy,
    event.nextOccupancy,
  );
  const triggerPanel =
    openingPanels.at(-1) ??
    (booleanNoop &&
    event.cause === "user-route" &&
    event.nextOccupancy.conversationRail
      ? "conversationRail"
      : null);
  const transaction: LayoutTransaction = {
    transactionId: event.transactionId,
    rendererEpoch: state.rendererEpoch,
    sequence,
    cause: event.cause,
    previousOccupancy: cloneOccupancy(event.previousOccupancy),
    nextOccupancy: cloneOccupancy(event.nextOccupancy),
    openingPanels,
    closingPanels,
    triggerPanel,
    authority,
    phase: "checkpoint-pending",
    fallbackVisible: state.policy === "legacy",
    checkpointAttempts: 1,
    supersededBy: null,
  };
  let transactions = { ...state.transactions };
  if (state.headTransactionId) {
    const previousHead = transactions[state.headTransactionId];
    if (
      previousHead &&
      !["settled", "rejected", "superseded"].includes(previousHead.phase)
    ) {
      transactions[state.headTransactionId] = {
        ...previousHead,
        phase: "superseded",
        supersededBy: event.transactionId,
      };
    }
  }
  transactions[event.transactionId] = transaction;
  const userOpenedSideTools =
    openingPanels.includes("sideTools") &&
    (event.cause === "user-panel" || event.cause === "user-route");
  const next: HorizontalLayoutState = {
    ...state,
    revision: state.revision + 1,
    nextSequence: sequence + 1,
    desiredOccupancy: cloneOccupancy(event.nextOccupancy),
    sideToolsManualOverride: userOpenedSideTools
      ? true
      : event.nextOccupancy.sideTools
        ? state.sideToolsManualOverride
        : false,
    transactions,
    headTransactionId: event.transactionId,
  };
  return { state: next, effects: [checkpointEffect(next, transaction)] };
}

function settleAppliedBounds(
  state: HorizontalLayoutState,
  transactionId: string,
): HorizontalLayoutReduction {
  const transaction = state.transactions[transactionId];
  let next = state;
  if (transaction) {
    next = updateTransaction(next, transactionId, {
      phase:
        state.headTransactionId === transactionId
          ? "settled"
          : "superseded",
    });
  }
  if (
    state.headTransactionId &&
    state.headTransactionId !== transactionId
  ) {
    const head = next.transactions[state.headTransactionId];
    if (
      head &&
      ["settled", "constrained", "deferred-reconcile"].includes(head.phase)
    ) {
      return planBoundsAfterFrameCommit(next, head);
    }
  }
  return { state: next, effects: [] };
}

function handleSessionApplied(
  state: HorizontalLayoutState,
  event: Extract<
    HorizontalLayoutEvent,
    { type: "WINDOW_LAYOUT_SESSION_APPLIED" }
  >,
): HorizontalLayoutReduction {
  if (!eventEpochMatches(state, event.rendererEpoch)) {
    return { state, effects: [] };
  }
  let next = foldAcknowledgedFacts(state, event.acknowledgedSession);
  const transaction = next.transactions[event.transactionId];
  if (!transaction) {
    return { state: next, effects: [] };
  }
  if (
    next.headTransactionId !== event.transactionId ||
    transaction.phase === "superseded"
  ) {
    return {
      state: updateTransaction(next, event.transactionId, {
        phase: "superseded",
      }),
      effects: [],
    };
  }
  if (transaction.phase !== "checkpoint-pending") {
    return { state: next, effects: [] };
  }
  if (
    !layoutOccupanciesEqual(
      event.acknowledgedSession.desiredOccupancy,
      transaction.nextOccupancy,
    )
  ) {
    next = addDiagnostic(
      updateTransaction(next, event.transactionId, { phase: "rejected" }),
      "checkpoint-ack-intent-mismatch",
      event.transactionId,
    );
    return {
      state: next,
      effects: [
        {
          type: "diagnostic",
          code: "checkpoint-ack-intent-mismatch",
          transactionId: event.transactionId,
        },
      ],
    };
  }
  return planAfterCheckpoint(next, transaction);
}

function handleSessionRejected(
  state: HorizontalLayoutState,
  event: Extract<
    HorizontalLayoutEvent,
    { type: "WINDOW_LAYOUT_SESSION_REJECTED" }
  >,
): HorizontalLayoutReduction {
  if (!eventEpochMatches(state, event.rendererEpoch)) {
    return { state, effects: [] };
  }
  let next = foldAcknowledgedFacts(state, event.acknowledgedSession);
  const transaction = next.transactions[event.transactionId];
  if (!transaction) {
    return { state: next, effects: [] };
  }
  if (
    event.reason === "stale" &&
    next.headTransactionId === event.transactionId
  ) {
    next = updateTransaction(next, event.transactionId, {
      phase: "checkpoint-pending",
      checkpointAttempts: transaction.checkpointAttempts + 1,
    });
    const retried = next.transactions[event.transactionId];
    return { state: next, effects: [checkpointEffect(next, retried)] };
  }
  if (event.reason === "superseded") {
    return {
      state: updateTransaction(next, event.transactionId, {
        phase: "superseded",
      }),
      effects: [],
    };
  }
  next = addDiagnostic(
    updateTransaction(next, event.transactionId, { phase: "rejected" }),
    "invalid-session-checkpoint",
    event.transactionId,
  );
  return {
    state: next,
    effects: [
      {
        type: "diagnostic",
        code: "invalid-session-checkpoint",
        transactionId: event.transactionId,
      },
    ],
  };
}

function handleBoundsRejected(
  state: HorizontalLayoutState,
  event: Extract<
    HorizontalLayoutEvent,
    { type: "WINDOW_BOUNDS_REJECTED" }
  >,
): HorizontalLayoutReduction {
  if (!eventEpochMatches(state, event.rendererEpoch)) {
    return { state, effects: [] };
  }
  let next = foldAcknowledgedFacts(
    state,
    event.acknowledgedSession,
    event.snapshot,
  );
  const transaction = next.transactions[event.transactionId];
  if (!transaction) {
    return { state: next, effects: [] };
  }
  if (event.reason === "stale") {
    if (next.headTransactionId !== event.transactionId) {
      return {
        state: updateTransaction(next, event.transactionId, {
          phase: "superseded",
        }),
        effects: [],
      };
    }
    return planBoundsAfterFrameCommit(next, transaction);
  }
  if (event.reason === "fixed-mode") {
    const phase = transitionHasFundedClose(next, transaction)
      ? "deferred-reconcile"
      : "deferred-funding";
    return {
      state: updateTransaction(next, event.transactionId, { phase }),
      effects: [],
    };
  }
  if (event.reason === "outside-work-area") {
    const targetWithoutOpening = targetFundingForTransaction(
      next,
      transaction,
      false,
    );
    const owesRepay = !fundingMapsEqual(
      next.acknowledgedSession.fundingByPanel,
      targetWithoutOpening,
    );
    return {
      state: updateTransaction(next, event.transactionId, {
        phase: owesRepay ? "deferred-reconcile" : "constrained",
      }),
      effects: [],
    };
  }
  if (event.reason === "superseded") {
    next = updateTransaction(next, event.transactionId, {
      phase: "superseded",
    });
    if (
      next.headTransactionId &&
      next.headTransactionId !== event.transactionId
    ) {
      const head = next.transactions[next.headTransactionId];
      if (
        head &&
        ["settled", "constrained", "deferred-reconcile"].includes(head.phase)
      ) {
        return planBoundsAfterFrameCommit(next, head);
      }
    }
    return { state: next, effects: [] };
  }
  next = addDiagnostic(
    updateTransaction(next, event.transactionId, { phase: "rejected" }),
    "invalid-window-bounds-command",
    event.transactionId,
  );
  return {
    state: next,
    effects: [
      {
        type: "diagnostic",
        code: "invalid-window-bounds-command",
        transactionId: event.transactionId,
      },
    ],
  };
}

function handleWindowSnapshot(
  state: HorizontalLayoutState,
  snapshot: WindowLayoutSnapshot,
  forcePanelMachineOrigin = false,
  acknowledgedSession?: AcknowledgedLayoutSession,
): HorizontalLayoutReduction {
  const folded = acknowledgedSession
    ? foldAcknowledgedFacts(state, acknowledgedSession)
    : state;
  if (snapshot.windowRevision <= folded.snapshot.windowRevision) {
    return { state: folded, effects: [] };
  }
  const origin: WindowSnapshotOrigin = forcePanelMachineOrigin
    ? "panel-machine"
    : snapshot.origin;
  const updatesResponsiveBasis = origin === "user" || origin === "display";
  const sessionAtSnapshotRevision =
    folded.acknowledgedSession.windowRevision < snapshot.windowRevision
      ? {
          ...folded.acknowledgedSession,
          windowRevision: snapshot.windowRevision,
        }
      : folded.acknowledgedSession;
  let next: HorizontalLayoutState = {
    ...folded,
    revision: folded.revision + 1,
    snapshot: { ...snapshot, origin },
    acknowledgedSession: sessionAtSnapshotRevision,
    responsiveBasisWidth: updatesResponsiveBasis
      ? snapshot.contentBounds.width
      : folded.responsiveBasisWidth,
    sideToolsManualOverride: updatesResponsiveBasis
      ? false
      : folded.sideToolsManualOverride,
  };
  const head = activeHeadTransaction(next);
  if (!head) {
    return { state: next, effects: [] };
  }
  if (["settled", "constrained"].includes(head.phase)) {
    return {
      state: updateTransaction(next, head.transactionId, {
        triggerPanel: null,
      }),
      effects: [],
    };
  }
  if (snapshot.mode !== "normal") {
    if (
      ["awaiting-bounds", "opening-fallback", "deferred-funding"].includes(
        head.phase,
      )
    ) {
      next = updateTransaction(next, head.transactionId, {
        phase: transitionHasFundedClose(next, head)
          ? "deferred-reconcile"
          : "deferred-funding",
      });
    }
    return { state: next, effects: [] };
  }
  if (head.phase === "deferred-funding") {
    return planAfterCheckpoint(next, head);
  }
  if (head.phase === "deferred-reconcile") {
    return planBoundsAfterFrameCommit(next, head);
  }
  return { state: next, effects: [] };
}

function updatePanelWidth(
  state: HorizontalLayoutState,
  event: Extract<
    HorizontalLayoutEvent,
    { type: "PANEL_WIDTH_CHANGED" }
  >,
): HorizontalLayoutReduction {
  const candidate: LayoutWidths = {
    ...state.widths,
    [event.panel]: event.width,
    sideToolsCustomized:
      event.panel === "sideTools"
        ? true
        : state.widths.sideToolsCustomized,
  };
  const widths = normalizeLayoutWidths(candidate);
  const next = withRevision({ ...state, widths });
  return {
    state: next,
    effects: event.commit && event.panel === "threadLogs"
      ? [
          {
            type: "persist-preference",
            preference: "panel-width",
            panel: event.panel,
            value: widths[event.panel],
          },
        ]
      : [],
  };
}

export function reduceHorizontalLayout(
  state: HorizontalLayoutState,
  event: HorizontalLayoutEvent,
): HorizontalLayoutReduction {
  switch (event.type) {
    case "HYDRATE":
      return hydrateState(state, event);
    case "CLAIM_INITIAL_LAYOUT_APPLIED": {
      if (!eventEpochMatches(state, event.rendererEpoch)) {
        return { state, effects: [] };
      }
      const folded = foldAcknowledgedFacts(
        state,
        event.acknowledgedSession,
        event.snapshot,
      );
      return {
        state: { ...folded, pendingInitialClaim: false },
        effects: [],
      };
    }
    case "CLAIM_INITIAL_LAYOUT_REJECTED": {
      if (!eventEpochMatches(state, event.rendererEpoch)) {
        return { state, effects: [] };
      }
      let next = foldAcknowledgedFacts(
        state,
        event.acknowledgedSession,
        event.snapshot,
      );
      if (
        event.reason === "stale" &&
        event.acknowledgedSession.sessionRevision === 0
      ) {
        next = { ...next, pendingInitialClaim: true };
        return {
          state: next,
          effects: [
            {
              type: "claim-initial-layout",
              command: initialClaimCommand(next),
            },
          ],
        };
      }
      if (event.reason === "stale" || event.reason === "superseded") {
        return hydrateOrphanedFunding({
          ...next,
          pendingInitialClaim: false,
          desiredOccupancy: cloneOccupancy(
            event.acknowledgedSession.desiredOccupancy,
          ),
        });
      }
      next = addDiagnostic(
        { ...next, pendingInitialClaim: false },
        "invalid-initial-layout-claim",
      );
      return {
        state: next,
        effects: [
          { type: "diagnostic", code: "invalid-initial-layout-claim" },
        ],
      };
    }
    case "LAYOUT_INTENT_CHANGED":
      return startIntentTransaction(state, event);
    case "WINDOW_LAYOUT_SESSION_APPLIED":
      return handleSessionApplied(state, event);
    case "WINDOW_LAYOUT_SESSION_REJECTED":
      return handleSessionRejected(state, event);
    case "WINDOW_BOUNDS_APPLIED": {
      if (!eventEpochMatches(state, event.rendererEpoch)) {
        return { state, effects: [] };
      }
      const folded = foldAcknowledgedFacts(
        state,
        event.acknowledgedSession,
        event.snapshot,
      );
      return settleAppliedBounds(folded, event.transactionId);
    }
    case "WINDOW_BOUNDS_REJECTED":
      return handleBoundsRejected(state, event);
    case "OPEN_DEADLINE_EXPIRED": {
      const transaction = state.transactions[event.transactionId];
      if (
        !transaction ||
        state.headTransactionId !== event.transactionId ||
        transaction.phase !== "awaiting-bounds"
      ) {
        return { state, effects: [] };
      }
      return {
        state: updateTransaction(state, event.transactionId, {
          phase: "opening-fallback",
          fallbackVisible: true,
        }),
        effects: [],
      };
    }
    case "CLOSE_DEADLINE_EXPIRED":
    case "PRESENTATION_ANIMATION_FINISHED": {
      const transaction = state.transactions[event.transactionId];
      if (
        !transaction ||
        state.headTransactionId !== event.transactionId ||
        transaction.phase !== "closing-animation"
      ) {
        return { state, effects: [] };
      }
      return {
        state: updateTransaction(state, event.transactionId, {
          phase: "frame-commit-pending",
        }),
        effects: [
          {
            type: "request-frame-commit",
            transactionId: event.transactionId,
          },
        ],
      };
    }
    case "FRAME_COMMITTED": {
      const transaction = state.transactions[event.transactionId];
      if (
        !transaction ||
        state.headTransactionId !== event.transactionId ||
        transaction.phase !== "frame-commit-pending"
      ) {
        return { state, effects: [] };
      }
      return planBoundsAfterFrameCommit(state, transaction);
    }
    case "WINDOW_SNAPSHOT_CHANGED":
      return handleWindowSnapshot(
        state,
        event.snapshot,
        false,
        event.acknowledgedSession,
      );
    case "VIEWPORT_RESIZED_DURING_NATIVE_SESSION":
      return handleWindowSnapshot(
        state,
        event.snapshot,
        true,
        event.acknowledgedSession,
      );
    case "COMPACT_SIDEBAR_TOGGLED":
      return {
        state: withRevision({
          ...state,
          compactSidebarOpen: !state.compactSidebarOpen,
        }),
        effects: [],
      };
    case "PANEL_WIDTH_CHANGED":
      return updatePanelWidth(state, event);
  }
}
