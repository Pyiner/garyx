import assert from "node:assert/strict";
import test from "node:test";

import {
  CLOSED_LAYOUT_OCCUPANCY,
  boundsForAcknowledgedSession,
  createHorizontalLayoutState,
  reduceHorizontalLayout,
} from "./responsive-layout-model.ts";
import {
  createLayoutProtocolExecutorState,
  executeLayoutProtocolCommand,
  takeoverLayoutProtocolExecutor,
} from "./window-layout-protocol.ts";

const sideToolsOpen = Object.freeze({
  globalSidebar: false,
  conversationRail: false,
  sideTools: true,
  threadLogs: false,
});

function snapshot({
  width = 1480,
  revision = 1,
  mode = "normal",
  workAreaWidth = 4000,
} = {}) {
  const bounds = { x: 100, y: 80, width, height: 800 };
  return {
    windowRevision: revision,
    bounds,
    contentBounds: { x: 0, y: 0, width, height: 800 },
    normalBounds: bounds,
    workArea: { x: 0, y: 0, width: workAreaWidth, height: 1400 },
    mode,
    displayId: "protocol-display",
    scaleFactor: 2,
    origin: "hydrate",
  };
}

function funding(panel, widthDelta, fundingId = `funding-${panel}`) {
  return {
    fundingId,
    panel,
    widthDelta,
    xCompensation: 0,
    repayAuthority: { fundingId },
  };
}

function session({
  baseWidth = 1480,
  desiredOccupancy = CLOSED_LAYOUT_OCCUPANCY,
  fundingByPanel = {},
  windowRevision = 1,
  sessionRevision = 1,
} = {}) {
  return {
    normalBaseBounds: { x: 100, y: 80, width: baseWidth, height: 800 },
    fundingByPanel,
    desiredOccupancy,
    windowRevision,
    sessionRevision,
  };
}

function startOpen(epoch = "protocol-epoch") {
  const acknowledgedSession = session();
  const initialSnapshot = snapshot();
  const machine = createHorizontalLayoutState({
    policy: "expand-v1",
    rendererEpoch: epoch,
    snapshot: initialSnapshot,
    desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    acknowledgedSession,
  });
  const started = reduceHorizontalLayout(machine, {
    type: "LAYOUT_INTENT_CHANGED",
    previousOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    nextOccupancy: sideToolsOpen,
    cause: "user-panel",
    transactionId: "open-side-tools",
  });
  const checkpoint = started.effects[0].command;
  const executor = createLayoutProtocolExecutorState({
    activeRendererEpoch: epoch,
    snapshot: initialSnapshot,
    acknowledgedSession,
    freshSession: false,
  });
  const checkpointed = executeLayoutProtocolCommand(executor, checkpoint);
  assert.equal(checkpointed.result.accepted, true);
  const awaiting = reduceHorizontalLayout(started.state, {
    type: "WINDOW_LAYOUT_SESSION_APPLIED",
    rendererEpoch: epoch,
    transactionId: checkpoint.transactionId,
    acknowledgedSession: checkpointed.result.acknowledgedSession,
  });
  const bounds = awaiting.effects.find(
    (effect) => effect.type === "window-bounds",
  ).command;
  return { executor, checkpoint, checkpointed, awaiting, bounds };
}

test("pure executor closes checkpoint -> dual-CAS bounds -> full ack loop", () => {
  const { checkpointed, awaiting, bounds } = startOpen();
  assert.equal(checkpointed.result.setBoundsApplied, false);
  assert.deepEqual(
    checkpointed.result.acknowledgedSession.desiredOccupancy,
    sideToolsOpen,
  );
  assert.equal(checkpointed.result.acknowledgedSession.sessionRevision, 2);

  const applied = executeLayoutProtocolCommand(checkpointed.state, bounds);
  assert.equal(applied.result.accepted, true);
  assert.equal(applied.result.setBoundsApplied, true);
  assert.equal(applied.state.setBoundsCount, 1);
  assert.equal(applied.result.snapshot.bounds.width, 1810);
  assert.equal(
    applied.result.acknowledgedSession.fundingByPanel.sideTools.widthDelta,
    330,
  );
  assert.equal(applied.result.acknowledgedSession.windowRevision, 2);
  assert.equal(applied.result.acknowledgedSession.sessionRevision, 3);

  const machineApplied = reduceHorizontalLayout(awaiting.state, {
    type: "WINDOW_BOUNDS_APPLIED",
    rendererEpoch: awaiting.state.rendererEpoch,
    transactionId: bounds.transactionId,
    sequence: bounds.sequence,
    acknowledgedSession: applied.result.acknowledgedSession,
    snapshot: applied.result.snapshot,
  });
  assert.equal(
    machineApplied.state.transactions[bounds.transactionId].phase,
    "settled",
  );

  const duplicate = executeLayoutProtocolCommand(applied.state, bounds);
  assert.equal(duplicate.result.accepted, false);
  assert.equal(duplicate.result.reason, "stale");
  assert.equal(duplicate.state.setBoundsCount, 1);
});

test("accepted result folds actual bounds readback into the normal base", () => {
  const { checkpointed, bounds } = startOpen();
  const actualBounds = { ...bounds.targetBounds, width: 1808 };
  const applied = executeLayoutProtocolCommand(
    checkpointed.state,
    bounds,
    { actualBounds },
  );
  assert.equal(applied.result.accepted, true);
  assert.equal(applied.result.snapshot.bounds.width, 1808);
  assert.equal(
    applied.result.acknowledgedSession.normalBaseBounds.width,
    1478,
  );
  assert.equal(
    boundsForAcknowledgedSession(
      applied.result.acknowledgedSession,
    ).width,
    1808,
  );
});

test("bounds require an acknowledged checkpoint even with a valid user token", () => {
  const { executor, bounds } = startOpen();
  const ungated = structuredClone(bounds);
  ungated.expectedWindowRevision = executor.snapshot.windowRevision;
  ungated.expectedSessionRevision = executor.acknowledgedSession.sessionRevision;
  const result = executeLayoutProtocolCommand(executor, ungated);
  assert.equal(result.result.accepted, false);
  assert.equal(result.result.reason, "invalid");
  assert.equal(result.state.setBoundsCount, 0);
});

test("newer checkpoint supersedes a queued old bounds command", () => {
  const { checkpointed, bounds } = startOpen();
  const closeCheckpoint = {
    type: "CHECKPOINT_DESIRED_OCCUPANCY",
    expectedSessionRevision:
      checkpointed.state.acknowledgedSession.sessionRevision,
    desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    transactionId: "close-side-tools",
    rendererEpoch: checkpointed.state.activeRendererEpoch,
    sequence: 2,
  };
  const newer = executeLayoutProtocolCommand(
    checkpointed.state,
    closeCheckpoint,
  );
  assert.equal(newer.result.accepted, true);
  const old = executeLayoutProtocolCommand(newer.state, bounds);
  assert.equal(old.result.accepted, false);
  assert.equal(old.result.reason, "superseded");
  assert.equal(old.state.setBoundsCount, 0);
});

test("bounds execution distinguishes fixed mode and current work-area TOCTOU", () => {
  const { checkpointed, bounds } = startOpen();
  const fixedState = {
    ...checkpointed.state,
    snapshot: { ...checkpointed.state.snapshot, mode: "maximized" },
  };
  const fixed = executeLayoutProtocolCommand(fixedState, bounds);
  assert.equal(fixed.result.accepted, false);
  assert.equal(fixed.result.reason, "fixed-mode");
  assert.equal(fixed.state.setBoundsCount, 0);

  const constrainedState = {
    ...checkpointed.state,
    snapshot: {
      ...checkpointed.state.snapshot,
      workArea: { ...checkpointed.state.snapshot.workArea, width: 1800 },
    },
  };
  const outside = executeLayoutProtocolCommand(constrainedState, bounds);
  assert.equal(outside.result.accepted, false);
  assert.equal(outside.result.reason, "outside-work-area");
  assert.equal(outside.state.setBoundsCount, 0);
});

test("fresh CLAIM_INITIAL_LAYOUT changes only the opaque session", () => {
  const epoch = "claim-protocol";
  const initialSnapshot = snapshot();
  const machine = createHorizontalLayoutState({
    policy: "expand-v1",
    rendererEpoch: epoch,
    snapshot: initialSnapshot,
    hydrated: false,
  });
  const hydrated = reduceHorizontalLayout(machine, {
    type: "HYDRATE",
    freshSession: true,
    snapshot: initialSnapshot,
    desiredOccupancy: {
      ...CLOSED_LAYOUT_OCCUPANCY,
      globalSidebar: true,
    },
  });
  const claim = hydrated.effects[0].command;
  const executor = createLayoutProtocolExecutorState({
    activeRendererEpoch: epoch,
    snapshot: initialSnapshot,
  });
  const claimed = executeLayoutProtocolCommand(executor, claim);
  assert.equal(claimed.result.accepted, true);
  assert.equal(claimed.result.setBoundsApplied, false);
  assert.equal(claimed.state.setBoundsCount, 0);
  assert.equal(claimed.state.freshSession, false);
  assert.deepEqual(claimed.result.snapshot.bounds, initialSnapshot.bounds);
  assert.equal(
    claimed.result.acknowledgedSession.normalBaseBounds.width,
    1235,
  );
  assert.equal(
    boundsForAcknowledgedSession(
      claimed.result.acknowledgedSession,
    ).width,
    1480,
  );

  const duplicate = {
    ...claim,
    expectedSessionRevision:
      claimed.state.acknowledgedSession.sessionRevision,
  };
  const rejected = executeLayoutProtocolCommand(claimed.state, duplicate);
  assert.equal(rejected.result.accepted, false);
  assert.equal(rejected.result.reason, "invalid");
});

test("hydrate orphan RepayProof is the only bounds path without a new checkpoint", () => {
  const epoch = "orphan-protocol";
  const sideFunding = funding("sideTools", 330, "orphan-funding");
  const orphanSession = session({
    desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    fundingByPanel: { sideTools: sideFunding },
    windowRevision: 4,
    sessionRevision: 8,
  });
  const physicalWidth = boundsForAcknowledgedSession(orphanSession).width;
  const currentSnapshot = snapshot({ width: physicalWidth, revision: 4 });
  const machine = createHorizontalLayoutState({
    policy: "expand-v1",
    rendererEpoch: epoch,
    snapshot: currentSnapshot,
    hydrated: false,
  });
  const hydrated = reduceHorizontalLayout(machine, {
    type: "HYDRATE",
    freshSession: false,
    snapshot: currentSnapshot,
    acknowledgedSession: orphanSession,
  });
  const command = hydrated.effects[0].command;
  const executor = createLayoutProtocolExecutorState({
    activeRendererEpoch: epoch,
    snapshot: currentSnapshot,
    acknowledgedSession: orphanSession,
    freshSession: false,
  });
  const repaid = executeLayoutProtocolCommand(executor, command);
  assert.equal(repaid.result.accepted, true);
  assert.equal(repaid.state.setBoundsCount, 1);
  assert.equal(repaid.result.snapshot.bounds.width, 1480);

  const impostor = {
    ...command,
    transactionId: "uncheckpointed-cleanup",
  };
  const rejected = executeLayoutProtocolCommand(executor, impostor);
  assert.equal(rejected.result.accepted, false);
  assert.equal(rejected.result.reason, "invalid");
});

test("renderer epoch takeover preserves session and rejects old queued commands", () => {
  const { checkpointed, bounds } = startOpen("old-epoch");
  const takeover = takeoverLayoutProtocolExecutor(
    checkpointed.state,
    "new-epoch",
  );
  assert.deepEqual(
    takeover.acknowledgedSession,
    checkpointed.state.acknowledgedSession,
  );
  assert.deepEqual(takeover.checkpoints, {});
  const old = executeLayoutProtocolCommand(takeover, bounds);
  assert.equal(old.result.accepted, false);
  assert.equal(old.result.reason, "superseded");
  assert.equal(old.state.setBoundsCount, 0);
});
