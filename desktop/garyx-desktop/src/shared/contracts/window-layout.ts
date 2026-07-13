export type HorizontalLayoutPolicyName = "legacy" | "expand-v1";

export type WindowLayoutPanelId =
  | "globalSidebar"
  | "conversationRail"
  | "sideTools"
  | "threadLogs";

export const WINDOW_LAYOUT_PANEL_ORDER: readonly WindowLayoutPanelId[] =
  Object.freeze([
    "globalSidebar",
    "conversationRail",
    "sideTools",
    "threadLogs",
  ]);

export type WindowLayoutPanelOccupancy = Readonly<{
  globalSidebar: boolean;
  conversationRail: boolean;
  sideTools: boolean;
  threadLogs: boolean;
}>;

export const CLOSED_WINDOW_LAYOUT_OCCUPANCY: WindowLayoutPanelOccupancy =
  Object.freeze({
    globalSidebar: false,
    conversationRail: false,
    sideTools: false,
    threadLogs: false,
  });

export type WindowLayoutRectangle = Readonly<{
  x: number;
  y: number;
  width: number;
  height: number;
}>;

export type WindowLayoutMode = "normal" | "maximized" | "fullscreen";

export type WindowLayoutSnapshotOrigin =
  | "user"
  | "display"
  | "panel-machine"
  | "mode"
  | "hydrate";

export type WindowLayoutSnapshot = Readonly<{
  windowRevision: number;
  bounds: WindowLayoutRectangle;
  contentBounds: WindowLayoutRectangle;
  normalBounds: WindowLayoutRectangle;
  workArea: WindowLayoutRectangle;
  mode: WindowLayoutMode;
  displayId: string;
  scaleFactor: number;
  origin: WindowLayoutSnapshotOrigin;
}>;

export type WindowLayoutUserCauseToken = Readonly<{
  kind: "user-cause";
  tokenId: string;
  transactionId: string;
  cause: "user-panel" | "user-route";
  rendererEpoch: string;
  sequence: number;
}>;

export type WindowLayoutRepayProof = Readonly<{
  kind: "repay-proof";
  fundingIds: readonly string[];
  expectedSessionRevision: number;
}>;

export type WindowLayoutBoundsAuthority =
  | WindowLayoutUserCauseToken
  | WindowLayoutRepayProof;

export type WindowLayoutConfirmedFunding = Readonly<{
  fundingId: string;
  panel: WindowLayoutPanelId;
  widthDelta: number;
  xCompensation: number;
  repayAuthority: Readonly<{ fundingId: string }>;
}>;

export type WindowLayoutFundingByPanel = Readonly<
  Partial<Record<WindowLayoutPanelId, WindowLayoutConfirmedFunding>>
>;

export type AcknowledgedWindowLayoutSession = Readonly<{
  normalBaseBounds: WindowLayoutRectangle;
  fundingByPanel: WindowLayoutFundingByPanel;
  desiredOccupancy: WindowLayoutPanelOccupancy;
  windowRevision: number;
  sessionRevision: number;
}>;

export type WindowLayoutSessionCommand = Readonly<{
  type: "CHECKPOINT_DESIRED_OCCUPANCY";
  expectedSessionRevision: number;
  desiredOccupancy: WindowLayoutPanelOccupancy;
  transactionId: string;
  rendererEpoch: string;
  sequence: number;
}>;

export type WindowBoundsCommand = Readonly<{
  type: "APPLY_WINDOW_BOUNDS";
  authority: WindowLayoutBoundsAuthority;
  expectedWindowRevision: number;
  expectedSessionRevision: number;
  targetBounds: WindowLayoutRectangle;
  targetNormalBaseBounds: WindowLayoutRectangle;
  targetFundingByPanel: WindowLayoutFundingByPanel;
  targetDesiredOccupancy: WindowLayoutPanelOccupancy;
  transactionId: string;
  rendererEpoch: string;
  sequence: number;
}>;

export type ClaimInitialWindowLayoutCommand = Readonly<{
  type: "CLAIM_INITIAL_LAYOUT";
  expectedWindowRevision: number;
  expectedSessionRevision: number;
  targetNormalBaseBounds: WindowLayoutRectangle;
  targetFundingByPanel: WindowLayoutFundingByPanel;
  targetDesiredOccupancy: WindowLayoutPanelOccupancy;
  transactionId: "claim-initial-layout";
  rendererEpoch: string;
  sequence: 0;
}>;

export type WindowLayoutCommand =
  | WindowLayoutSessionCommand
  | WindowBoundsCommand
  | ClaimInitialWindowLayoutCommand;

export type WindowLayoutCommandRejectionReason =
  | "stale"
  | "fixed-mode"
  | "outside-work-area"
  | "superseded"
  | "invalid";

export type WindowLayoutCommandResult =
  | Readonly<{
      accepted: true;
      commandType: WindowLayoutCommand["type"];
      setBoundsApplied: boolean;
      acknowledgedSession: AcknowledgedWindowLayoutSession;
      snapshot: WindowLayoutSnapshot;
    }>
  | Readonly<{
      accepted: false;
      commandType: WindowLayoutCommand["type"];
      reason: WindowLayoutCommandRejectionReason;
      acknowledgedSession: AcknowledgedWindowLayoutSession;
      snapshot: WindowLayoutSnapshot;
    }>;

export type WindowLayoutBootstrap = Readonly<{
  policy: HorizontalLayoutPolicyName;
  freshSession: boolean;
  snapshot: WindowLayoutSnapshot;
  acknowledgedSession: AcknowledgedWindowLayoutSession;
}>;

export type WindowLayoutSnapshotUpdate = Readonly<{
  snapshot: WindowLayoutSnapshot;
  acknowledgedSession: AcknowledgedWindowLayoutSession;
}>;

export type WindowLayoutSnapshotListener = (
  update: WindowLayoutSnapshotUpdate,
) => void;

export function resolveHorizontalLayoutPolicy(
  featureValue: string | undefined,
): HorizontalLayoutPolicyName {
  const normalized = featureValue?.trim().toLowerCase();
  return normalized === "0" ||
    normalized === "false" ||
    normalized === "off" ||
    normalized === "legacy"
    ? "legacy"
    : "expand-v1";
}
