import {
  CLOSED_LAYOUT_OCCUPANCY,
  HORIZONTAL_LAYOUT_PANEL_ORDER,
  boundsForAcknowledgedSession,
  layoutOccupanciesEqual,
  validateBoundsCommandAuthority,
  type AcknowledgedLayoutSession,
  type ClaimInitialLayoutCommand,
  type ConfirmedFunding,
  type FundingByPanel,
  type LayoutPanelId,
  type LayoutPanelOccupancy,
  type LayoutRectangle,
  type WindowBoundsCommand,
  type WindowLayoutSessionCommand,
  type WindowLayoutSnapshot,
} from "./responsive-layout-model.ts";

export type LayoutProtocolCommand =
  | WindowLayoutSessionCommand
  | WindowBoundsCommand
  | ClaimInitialLayoutCommand;

export type LayoutProtocolRejectionReason =
  | "stale"
  | "fixed-mode"
  | "outside-work-area"
  | "superseded"
  | "invalid";

export type LayoutProtocolCheckpoint = Readonly<{
  sequence: number;
  sessionRevision: number;
  desiredOccupancy: LayoutPanelOccupancy;
}>;

export type LayoutProtocolExecutorState = Readonly<{
  activeRendererEpoch: string;
  freshSession: boolean;
  latestSequence: number;
  acknowledgedSession: AcknowledgedLayoutSession;
  snapshot: WindowLayoutSnapshot;
  checkpoints: Readonly<Record<string, LayoutProtocolCheckpoint>>;
  setBoundsCount: number;
}>;

export type LayoutProtocolAcceptedResult = Readonly<{
  accepted: true;
  commandType: LayoutProtocolCommand["type"];
  setBoundsApplied: boolean;
  acknowledgedSession: AcknowledgedLayoutSession;
  snapshot: WindowLayoutSnapshot;
}>;

export type LayoutProtocolRejectedResult = Readonly<{
  accepted: false;
  commandType: LayoutProtocolCommand["type"];
  reason: LayoutProtocolRejectionReason;
  acknowledgedSession: AcknowledgedLayoutSession;
  snapshot: WindowLayoutSnapshot;
}>;

export type LayoutProtocolResult =
  | LayoutProtocolAcceptedResult
  | LayoutProtocolRejectedResult;

export type LayoutProtocolExecution = Readonly<{
  state: LayoutProtocolExecutorState;
  result: LayoutProtocolResult;
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

function cloneFunding(funding: FundingByPanel): FundingByPanel {
  const clone: Partial<Record<LayoutPanelId, ConfirmedFunding>> = {};
  for (const panel of HORIZONTAL_LAYOUT_PANEL_ORDER) {
    const entry = funding[panel];
    if (entry) {
      clone[panel] = {
        ...entry,
        repayAuthority: { ...entry.repayAuthority },
      };
    }
  }
  return clone;
}

function cloneSession(
  session: AcknowledgedLayoutSession,
): AcknowledgedLayoutSession {
  return {
    ...session,
    normalBaseBounds: { ...session.normalBaseBounds },
    fundingByPanel: cloneFunding(session.fundingByPanel),
    desiredOccupancy: cloneOccupancy(session.desiredOccupancy),
  };
}

function defaultSession(
  snapshot: WindowLayoutSnapshot,
): AcknowledgedLayoutSession {
  return {
    normalBaseBounds: { ...snapshot.normalBounds },
    fundingByPanel: {},
    desiredOccupancy: cloneOccupancy(CLOSED_LAYOUT_OCCUPANCY),
    windowRevision: snapshot.windowRevision,
    sessionRevision: 0,
  };
}

export function createLayoutProtocolExecutorState({
  activeRendererEpoch,
  snapshot,
  acknowledgedSession,
  freshSession = acknowledgedSession === undefined,
}: {
  activeRendererEpoch: string;
  snapshot: WindowLayoutSnapshot;
  acknowledgedSession?: AcknowledgedLayoutSession;
  freshSession?: boolean;
}): LayoutProtocolExecutorState {
  return {
    activeRendererEpoch,
    freshSession,
    latestSequence: 0,
    acknowledgedSession: cloneSession(
      acknowledgedSession ?? defaultSession(snapshot),
    ),
    snapshot: { ...snapshot },
    checkpoints: {},
    setBoundsCount: 0,
  };
}

export function takeoverLayoutProtocolExecutor(
  state: LayoutProtocolExecutorState,
  rendererEpoch: string,
): LayoutProtocolExecutorState {
  return {
    ...state,
    activeRendererEpoch: rendererEpoch,
    latestSequence: 0,
    checkpoints: {},
  };
}

function reject(
  state: LayoutProtocolExecutorState,
  command: LayoutProtocolCommand,
  reason: LayoutProtocolRejectionReason,
): LayoutProtocolExecution {
  return {
    state,
    result: {
      accepted: false,
      commandType: command.type,
      reason,
      acknowledgedSession: cloneSession(state.acknowledgedSession),
      snapshot: { ...state.snapshot },
    },
  };
}

function accepted(
  state: LayoutProtocolExecutorState,
  command: LayoutProtocolCommand,
  setBoundsApplied: boolean,
): LayoutProtocolExecution {
  return {
    state,
    result: {
      accepted: true,
      commandType: command.type,
      setBoundsApplied,
      acknowledgedSession: cloneSession(state.acknowledgedSession),
      snapshot: { ...state.snapshot },
    },
  };
}

function isValidOccupancy(occupancy: LayoutPanelOccupancy): boolean {
  return !(occupancy.sideTools && occupancy.threadLogs);
}

function isFiniteRectangle(rectangle: LayoutRectangle): boolean {
  return (
    Number.isFinite(rectangle.x) &&
    Number.isFinite(rectangle.y) &&
    Number.isFinite(rectangle.width) &&
    Number.isFinite(rectangle.height) &&
    rectangle.width > 0 &&
    rectangle.height > 0
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

function fundingIsValid(funding: FundingByPanel): boolean {
  const ids = new Set<string>();
  for (const panel of HORIZONTAL_LAYOUT_PANEL_ORDER) {
    const entry = funding[panel];
    if (!entry) {
      continue;
    }
    if (
      entry.panel !== panel ||
      !entry.fundingId ||
      ids.has(entry.fundingId) ||
      !Number.isFinite(entry.widthDelta) ||
      entry.widthDelta <= 0 ||
      !Number.isFinite(entry.xCompensation) ||
      entry.repayAuthority.fundingId !== entry.fundingId
    ) {
      return false;
    }
    ids.add(entry.fundingId);
  }
  return true;
}

function commandEpoch(command: LayoutProtocolCommand): string {
  return command.rendererEpoch;
}

function commandSequence(command: LayoutProtocolCommand): number {
  return command.sequence;
}

function checkpointDesiredOccupancy(
  state: LayoutProtocolExecutorState,
  command: WindowLayoutSessionCommand,
): LayoutProtocolExecution {
  if (state.freshSession) {
    return reject(state, command, "invalid");
  }
  if (command.expectedSessionRevision !== state.acknowledgedSession.sessionRevision) {
    return reject(state, command, "stale");
  }
  if (!isValidOccupancy(command.desiredOccupancy)) {
    return reject(state, command, "invalid");
  }
  const sessionRevision = state.acknowledgedSession.sessionRevision + 1;
  const session: AcknowledgedLayoutSession = {
    ...state.acknowledgedSession,
    desiredOccupancy: cloneOccupancy(command.desiredOccupancy),
    windowRevision: state.snapshot.windowRevision,
    sessionRevision,
  };
  const next: LayoutProtocolExecutorState = {
    ...state,
    latestSequence: Math.max(state.latestSequence, command.sequence),
    acknowledgedSession: session,
    checkpoints: {
      [command.transactionId]: {
        sequence: command.sequence,
        sessionRevision,
        desiredOccupancy: cloneOccupancy(command.desiredOccupancy),
      },
    },
  };
  return accepted(next, command, false);
}

function claimInitialLayout(
  state: LayoutProtocolExecutorState,
  command: ClaimInitialLayoutCommand,
): LayoutProtocolExecution {
  if (
    command.expectedWindowRevision !== state.snapshot.windowRevision ||
    command.expectedSessionRevision !== state.acknowledgedSession.sessionRevision
  ) {
    return reject(state, command, "stale");
  }
  if (!state.freshSession) {
    return reject(state, command, "invalid");
  }
  if (
    !isValidOccupancy(command.targetDesiredOccupancy) ||
    !isFiniteRectangle(command.targetNormalBaseBounds) ||
    !fundingIsValid(command.targetFundingByPanel)
  ) {
    return reject(state, command, "invalid");
  }
  const reconstructedBounds = boundsForAcknowledgedSession({
    normalBaseBounds: command.targetNormalBaseBounds,
    fundingByPanel: command.targetFundingByPanel,
  });
  if (!rectanglesEqual(reconstructedBounds, state.snapshot.normalBounds)) {
    return reject(state, command, "invalid");
  }
  const session: AcknowledgedLayoutSession = {
    normalBaseBounds: { ...command.targetNormalBaseBounds },
    fundingByPanel: cloneFunding(command.targetFundingByPanel),
    desiredOccupancy: cloneOccupancy(command.targetDesiredOccupancy),
    windowRevision: state.snapshot.windowRevision,
    sessionRevision: state.acknowledgedSession.sessionRevision + 1,
  };
  const next: LayoutProtocolExecutorState = {
    ...state,
    freshSession: false,
    acknowledgedSession: session,
  };
  return accepted(next, command, false);
}

function hasCheckpointAuthority(
  state: LayoutProtocolExecutorState,
  command: WindowBoundsCommand,
): boolean {
  if (
    command.authority.kind === "repay-proof" &&
    command.transactionId === "hydrate-orphaned-funding"
  ) {
    return layoutOccupanciesEqual(
      command.targetDesiredOccupancy,
      state.acknowledgedSession.desiredOccupancy,
    );
  }
  const checkpoint = state.checkpoints[command.transactionId];
  return Boolean(
    checkpoint &&
      checkpoint.sequence === command.sequence &&
      checkpoint.sessionRevision <= state.acknowledgedSession.sessionRevision &&
      layoutOccupanciesEqual(
        checkpoint.desiredOccupancy,
        command.targetDesiredOccupancy,
      ) &&
      layoutOccupanciesEqual(
        state.acknowledgedSession.desiredOccupancy,
        command.targetDesiredOccupancy,
      ),
  );
}

function applyWindowBounds(
  state: LayoutProtocolExecutorState,
  command: WindowBoundsCommand,
  actualBounds: LayoutRectangle,
): LayoutProtocolExecution {
  if (
    command.expectedWindowRevision !== state.snapshot.windowRevision ||
    command.expectedSessionRevision !== state.acknowledgedSession.sessionRevision
  ) {
    return reject(state, command, "stale");
  }
  if (!hasCheckpointAuthority(state, command)) {
    return reject(state, command, "invalid");
  }
  if (state.snapshot.mode !== "normal") {
    return reject(state, command, "fixed-mode");
  }
  if (
    !isValidOccupancy(command.targetDesiredOccupancy) ||
    !isFiniteRectangle(command.targetBounds) ||
    !isFiniteRectangle(command.targetNormalBaseBounds) ||
    !fundingIsValid(command.targetFundingByPanel) ||
    !validateBoundsCommandAuthority(command, state.acknowledgedSession).valid
  ) {
    return reject(state, command, "invalid");
  }
  if (!rectangleContainedBy(command.targetBounds, state.snapshot.workArea)) {
    return reject(state, command, "outside-work-area");
  }
  if (!isFiniteRectangle(actualBounds)) {
    return reject(state, command, "invalid");
  }

  const windowRevision = state.snapshot.windowRevision + 1;
  const sessionRevision = state.acknowledgedSession.sessionRevision + 1;
  const contentWidthDelta =
    actualBounds.width - state.snapshot.bounds.width;
  const acknowledgedTarget = boundsForAcknowledgedSession({
    normalBaseBounds: command.targetNormalBaseBounds,
    fundingByPanel: command.targetFundingByPanel,
  });
  const actualNormalBaseBounds: LayoutRectangle = {
    x:
      command.targetNormalBaseBounds.x +
      (actualBounds.x - acknowledgedTarget.x),
    y:
      command.targetNormalBaseBounds.y +
      (actualBounds.y - acknowledgedTarget.y),
    width:
      command.targetNormalBaseBounds.width +
      (actualBounds.width - acknowledgedTarget.width),
    height:
      command.targetNormalBaseBounds.height +
      (actualBounds.height - acknowledgedTarget.height),
  };
  const snapshot: WindowLayoutSnapshot = {
    ...state.snapshot,
    windowRevision,
    bounds: { ...actualBounds },
    contentBounds: {
      ...state.snapshot.contentBounds,
      width: state.snapshot.contentBounds.width + contentWidthDelta,
    },
    normalBounds: { ...actualBounds },
    origin: "panel-machine",
  };
  const session: AcknowledgedLayoutSession = {
    normalBaseBounds: actualNormalBaseBounds,
    fundingByPanel: cloneFunding(command.targetFundingByPanel),
    desiredOccupancy: cloneOccupancy(command.targetDesiredOccupancy),
    windowRevision,
    sessionRevision,
  };
  const checkpoints = { ...state.checkpoints };
  delete checkpoints[command.transactionId];
  const next: LayoutProtocolExecutorState = {
    ...state,
    latestSequence: Math.max(state.latestSequence, command.sequence),
    acknowledgedSession: session,
    snapshot,
    checkpoints,
    setBoundsCount: state.setBoundsCount + 1,
  };
  return accepted(next, command, true);
}

export function executeLayoutProtocolCommand(
  state: LayoutProtocolExecutorState,
  command: LayoutProtocolCommand,
  options: Readonly<{ actualBounds?: LayoutRectangle }> = {},
): LayoutProtocolExecution {
  if (commandEpoch(command) !== state.activeRendererEpoch) {
    return reject(state, command, "superseded");
  }
  if (commandSequence(command) < state.latestSequence) {
    return reject(state, command, "superseded");
  }
  switch (command.type) {
    case "CHECKPOINT_DESIRED_OCCUPANCY":
      return checkpointDesiredOccupancy(state, command);
    case "CLAIM_INITIAL_LAYOUT":
      return claimInitialLayout(state, command);
    case "APPLY_WINDOW_BOUNDS":
      return applyWindowBounds(
        state,
        command,
        options.actualBounds ?? command.targetBounds,
      );
  }
}
