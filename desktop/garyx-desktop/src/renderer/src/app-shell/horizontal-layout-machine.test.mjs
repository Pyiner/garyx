import assert from "node:assert/strict";
import test from "node:test";

import {
  CLOSED_LAYOUT_OCCUPANCY,
  boundsForAcknowledgedSession,
  canExpandWindowForLayout,
  createHorizontalLayoutState,
  projectHorizontalLayout,
  reduceHorizontalLayout,
  stableFrameFromProjection,
  totalConfirmedFunding,
  validateBoundsCommandAuthority,
} from "./responsive-layout-model.ts";

const sidebarOpen = Object.freeze({
  globalSidebar: true,
  conversationRail: false,
  sideTools: false,
  threadLogs: false,
});
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
  origin = "hydrate",
  x = 100,
  workAreaWidth = 4000,
} = {}) {
  const bounds = { x, y: 80, width, height: 800 };
  return {
    windowRevision: revision,
    bounds,
    contentBounds: { x: 0, y: 0, width, height: 800 },
    normalBounds: bounds,
    workArea: { x: 0, y: 0, width: workAreaWidth, height: 1400 },
    mode,
    displayId: "synthetic-display",
    scaleFactor: 2,
    origin,
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

function acknowledgedSession({
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

function stateWithSession({
  policy = "expand-v1",
  epoch = "renderer-test",
  desiredOccupancy = CLOSED_LAYOUT_OCCUPANCY,
  session = acknowledgedSession({ desiredOccupancy }),
  mode = "normal",
  origin = "hydrate",
  x = 100,
  workAreaWidth = 4000,
} = {}) {
  const physicalBounds = boundsForAcknowledgedSession(session);
  return createHorizontalLayoutState({
    policy,
    rendererEpoch: epoch,
    snapshot: snapshot({
      width: physicalBounds.width,
      revision: session.windowRevision,
      mode,
      origin,
      x,
      workAreaWidth,
    }),
    desiredOccupancy,
    acknowledgedSession: session,
  });
}

function beginIntent(
  state,
  nextOccupancy,
  {
    cause = "user-panel",
    transactionId = "transaction-1",
  } = {},
) {
  return reduceHorizontalLayout(state, {
    type: "LAYOUT_INTENT_CHANGED",
    previousOccupancy: state.desiredOccupancy,
    nextOccupancy,
    cause,
    transactionId,
  });
}

function acknowledgeCheckpoint(reduction, sessionRevision) {
  const checkpoint = reduction.effects.find(
    (effect) => effect.type === "window-layout-session",
  );
  assert.ok(checkpoint, "transaction must begin with a session checkpoint");
  const state = reduction.state;
  const session = {
    ...state.acknowledgedSession,
    desiredOccupancy: checkpoint.command.desiredOccupancy,
    sessionRevision:
      sessionRevision ?? state.acknowledgedSession.sessionRevision + 1,
  };
  return reduceHorizontalLayout(state, {
    type: "WINDOW_LAYOUT_SESSION_APPLIED",
    rendererEpoch: state.rendererEpoch,
    transactionId: checkpoint.command.transactionId,
    acknowledgedSession: session,
  });
}

function boundsEffect(reduction) {
  const effect = reduction.effects.find(
    (candidate) => candidate.type === "window-bounds",
  );
  assert.ok(effect, "expected one bounds effect");
  return effect;
}

function acceptBounds(reduction, { desiredOccupancy } = {}) {
  const effect = boundsEffect(reduction);
  const command = effect.command;
  const state = reduction.state;
  const nextWindowRevision = state.snapshot.windowRevision + 1;
  const nextSessionRevision =
    state.acknowledgedSession.sessionRevision + 1;
  const session = {
    normalBaseBounds: command.targetNormalBaseBounds,
    fundingByPanel: command.targetFundingByPanel,
    desiredOccupancy:
      desiredOccupancy ?? command.targetDesiredOccupancy,
    windowRevision: nextWindowRevision,
    sessionRevision: nextSessionRevision,
  };
  return reduceHorizontalLayout(state, {
    type: "WINDOW_BOUNDS_APPLIED",
    rendererEpoch: state.rendererEpoch,
    transactionId: command.transactionId,
    sequence: command.sequence,
    acknowledgedSession: session,
    snapshot: snapshot({
      width: command.targetBounds.width,
      revision: nextWindowRevision,
      origin: "panel-machine",
      x: command.targetBounds.x,
    }),
  });
}

test("every intent checkpoints desired occupancy before policy-specific work", () => {
  for (const policy of ["legacy", "expand-v1"]) {
    const state = stateWithSession({ policy });
    const started = beginIntent(state, sidebarOpen);
    assert.deepEqual(
      started.effects.map((effect) => effect.type),
      ["window-layout-session"],
      policy,
    );
    const checkpoint = started.effects[0].command;
    assert.equal(checkpoint.type, "CHECKPOINT_DESIRED_OCCUPANCY");
    assert.deepEqual(checkpoint.desiredOccupancy, sidebarOpen);
    assert.equal(checkpoint.expectedSessionRevision, 1);
    assert.equal(
      started.state.transactions["transaction-1"].authority?.kind,
      "user-cause",
    );

    const acknowledged = acknowledgeCheckpoint(started);
    if (policy === "legacy") {
      assert.deepEqual(acknowledged.effects, []);
      assert.equal(
        acknowledged.state.transactions["transaction-1"].phase,
        "settled",
      );
    } else {
      assert.deepEqual(
        acknowledged.effects.map((effect) => effect.type),
        ["window-bounds", "schedule-deadline"],
      );
    }
  }
});

test("ordinary sidebar intents never mint a compact overlay outside expand-v1 compact mode", () => {
  const legacySession = acknowledgedSession({
    baseWidth: 700,
    desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
  });
  const legacyStarted = beginIntent(
    stateWithSession({
      policy: "legacy",
      desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
      session: legacySession,
    }),
    sidebarOpen,
  );
  assert.equal(legacyStarted.state.compactSidebarOpen, false);
  const legacyFrame = stableFrameFromProjection(
    projectHorizontalLayout(legacyStarted.state),
  );
  assert.equal(
    legacyFrame.presentation.globalSidebar,
    "collapsed",
  );

  const wideStarted = beginIntent(stateWithSession(), sidebarOpen);
  assert.equal(wideStarted.state.compactSidebarOpen, false);
});

test("expandability uses the exact 2-DIP edge and work-area predicate", () => {
  const targetBounds = { x: 3, y: 80, width: 200, height: 800 };
  const cases = [
    {
      name: "left gap equals tolerance",
      snapshot: {
        ...snapshot({ width: 100, x: 2, workAreaWidth: 205 }),
        bounds: { x: 2, y: 80, width: 100, height: 800 },
      },
      targetBounds: { ...targetBounds, x: 2 },
      deltaWidth: 100,
      expected: false,
    },
    {
      name: "both gaps and containment pass",
      snapshot: {
        ...snapshot({ width: 100, x: 3, workAreaWidth: 205 }),
        bounds: { x: 3, y: 80, width: 100, height: 800 },
      },
      targetBounds,
      deltaWidth: 100,
      expected: true,
    },
    {
      name: "right gap is below delta plus tolerance",
      snapshot: {
        ...snapshot({ width: 100, x: 3, workAreaWidth: 204 }),
        bounds: { x: 3, y: 80, width: 100, height: 800 },
      },
      targetBounds,
      deltaWidth: 100,
      expected: false,
    },
    {
      name: "vertical target escapes work area",
      snapshot: {
        ...snapshot({ width: 100, x: 3, workAreaWidth: 205 }),
        bounds: { x: 3, y: 80, width: 100, height: 800 },
      },
      targetBounds: { ...targetBounds, y: 700 },
      deltaWidth: 100,
      expected: false,
    },
    {
      name: "fixed mode never expands",
      snapshot: {
        ...snapshot({ width: 100, x: 3, workAreaWidth: 205, mode: "fullscreen" }),
        bounds: { x: 3, y: 80, width: 100, height: 800 },
      },
      targetBounds,
      deltaWidth: 100,
      expected: false,
    },
  ];
  for (const { name, expected, ...input } of cases) {
    assert.equal(canExpandWindowForLayout(input), expected, name);
  }
});

test("reduce and project are deterministic and do not mutate their inputs", () => {
  const state = stateWithSession({ desiredOccupancy: sidebarOpen });
  const before = structuredClone(state);
  const firstFrame = projectHorizontalLayout(state);
  const secondFrame = projectHorizontalLayout(state);
  assert.deepEqual(firstFrame, secondFrame);
  assert.deepEqual(state, before);

  const event = {
    type: "LAYOUT_INTENT_CHANGED",
    previousOccupancy: sidebarOpen,
    nextOccupancy: { ...sidebarOpen, conversationRail: true },
    cause: "user-route",
    transactionId: "pure-reducer",
  };
  const eventBefore = structuredClone(event);
  reduceHorizontalLayout(state, event);
  assert.deepEqual(state, before);
  assert.deepEqual(event, eventBefore);
});

test("expandable open uses one user token and late ack survives the 100ms fallback", () => {
  const initial = stateWithSession();
  const started = beginIntent(initial, sideToolsOpen);
  const awaiting = acknowledgeCheckpoint(started);
  const command = boundsEffect(awaiting).command;
  assert.equal(command.authority.kind, "user-cause");
  assert.equal(
    validateBoundsCommandAuthority(
      command,
      awaiting.state.acknowledgedSession,
    ).valid,
    true,
  );
  assert.equal(command.targetBounds.width, 1810);
  assert.equal(totalConfirmedFunding(command.targetFundingByPanel), 330);

  const timedOut = reduceHorizontalLayout(awaiting.state, {
    type: "OPEN_DEADLINE_EXPIRED",
    transactionId: command.transactionId,
  });
  assert.deepEqual(timedOut.effects, []);
  assert.equal(
    timedOut.state.transactions[command.transactionId].phase,
    "opening-fallback",
  );
  const fallback = projectHorizontalLayout(timedOut.state);
  assert.equal(fallback.kind, "pending");
  assert.equal(fallback.fallbackVisible, true);
  assert.equal(fallback.frame.presentation.sideTools, "docked");

  const accepted = acceptBounds(
    { state: timedOut.state, effects: awaiting.effects },
  );
  assert.equal(
    accepted.state.transactions[command.transactionId].phase,
    "settled",
  );
  const frame = projectHorizontalLayout(accepted.state);
  assert.equal(frame.kind, "stable");
  assert.equal(frame.presentation.sideTools, "docked");
  assert.equal(frame.primaryThreadWidth, 1480);
});

test("a late rejected open follows the same reason route after fallback", () => {
  const awaiting = acknowledgeCheckpoint(
    beginIntent(stateWithSession(), sideToolsOpen),
  );
  const command = boundsEffect(awaiting).command;
  const timedOut = reduceHorizontalLayout(awaiting.state, {
    type: "OPEN_DEADLINE_EXPIRED",
    transactionId: command.transactionId,
  });
  const rejected = reduceHorizontalLayout(timedOut.state, {
    type: "WINDOW_BOUNDS_REJECTED",
    rendererEpoch: timedOut.state.rendererEpoch,
    transactionId: command.transactionId,
    sequence: command.sequence,
    reason: "outside-work-area",
    acknowledgedSession: {
      ...timedOut.state.acknowledgedSession,
      windowRevision: 2,
      sessionRevision: 3,
    },
    snapshot: snapshot({ revision: 2, origin: "display" }),
  });
  assert.equal(
    rejected.state.transactions[command.transactionId].phase,
    "constrained",
  );
  assert.deepEqual(rejected.effects, []);
});

test("funded close removes the frame before a RepayProof shrink", () => {
  const sideFunding = funding("sideTools", 330);
  const initialSession = acknowledgedSession({
    desiredOccupancy: sideToolsOpen,
    fundingByPanel: { sideTools: sideFunding },
    windowRevision: 5,
    sessionRevision: 8,
  });
  const initial = stateWithSession({
    desiredOccupancy: sideToolsOpen,
    session: initialSession,
  });
  const started = beginIntent(initial, CLOSED_LAYOUT_OCCUPANCY, {
    cause: "system-cleanup",
    transactionId: "cleanup-close",
  });
  assert.equal(
    projectHorizontalLayout(started.state).frame.presentation.sideTools,
    "closed",
    "a close frame must not wait for the main-process checkpoint ack",
  );
  const closing = acknowledgeCheckpoint(started, 9);
  assert.deepEqual(
    closing.effects.map((effect) => effect.type),
    ["request-frame-commit"],
  );
  assert.equal(
    closing.state.transactions["cleanup-close"].phase,
    "frame-commit-pending",
  );
  assert.equal(
    closing.state.transactions["cleanup-close"].authority.kind,
    "repay-proof",
  );
  assert.equal(
    projectHorizontalLayout(closing.state).frame.presentation.sideTools,
    "closed",
  );

  const frameCommitted = reduceHorizontalLayout(closing.state, {
    type: "FRAME_COMMITTED",
    transactionId: "cleanup-close",
  });
  const command = boundsEffect(frameCommitted).command;
  assert.equal(command.authority.kind, "repay-proof");
  assert.deepEqual(command.authority.fundingIds, [sideFunding.fundingId]);
  assert.equal(command.targetBounds.width, 1480);
  assert.equal(
    validateBoundsCommandAuthority(
      command,
      frameCommitted.state.acknowledgedSession,
    ).valid,
    true,
  );
});

test("funding-zero close settles without shrink", () => {
  const constrainedOpenSession = acknowledgedSession({
    desiredOccupancy: sideToolsOpen,
    fundingByPanel: {},
    sessionRevision: 3,
  });
  const initial = stateWithSession({
    desiredOccupancy: sideToolsOpen,
    session: constrainedOpenSession,
  });
  const closed = acknowledgeCheckpoint(
    beginIntent(initial, CLOSED_LAYOUT_OCCUPANCY, {
      transactionId: "unfunded-close",
    }),
  );
  assert.deepEqual(closed.effects, []);
  assert.equal(
    closed.state.transactions["unfunded-close"].phase,
    "constrained",
  );
  assert.equal(projectHorizontalLayout(closed.state).presentation.sideTools, "closed");
});

test("reopen supersedes a pending close frame and makes its callback a no-op", () => {
  const sideFunding = funding("sideTools", 330, "reopen-side-tools");
  const initialSession = acknowledgedSession({
    desiredOccupancy: sideToolsOpen,
    fundingByPanel: { sideTools: sideFunding },
    windowRevision: 2,
    sessionRevision: 4,
  });
  const initial = stateWithSession({
    desiredOccupancy: sideToolsOpen,
    session: initialSession,
  });
  const closing = acknowledgeCheckpoint(
    beginIntent(initial, CLOSED_LAYOUT_OCCUPANCY, {
      transactionId: "closing-1",
    }),
  );
  const reopening = beginIntent(closing.state, sideToolsOpen, {
    transactionId: "reopen-2",
  });
  assert.equal(reopening.state.transactions["closing-1"].phase, "superseded");
  const staleFrame = reduceHorizontalLayout(reopening.state, {
    type: "FRAME_COMMITTED",
    transactionId: "closing-1",
  });
  assert.equal(staleFrame.state, reopening.state);
  assert.deepEqual(staleFrame.effects, []);
});

test("funded open then close restores the exact base bounds", () => {
  const initial = stateWithSession();
  const opening = acknowledgeCheckpoint(beginIntent(initial, sideToolsOpen));
  const opened = acceptBounds(opening);
  assert.equal(opened.state.snapshot.bounds.width, 1810);

  const closingStarted = beginIntent(opened.state, CLOSED_LAYOUT_OCCUPANCY, {
    transactionId: "close-after-open",
  });
  const closing = acknowledgeCheckpoint(closingStarted);
  const committed = reduceHorizontalLayout(closing.state, {
    type: "FRAME_COMMITTED",
    transactionId: "close-after-open",
  });
  const closed = acceptBounds(committed);
  assert.deepEqual(closed.state.snapshot.bounds, initial.snapshot.bounds);
  assert.equal(
    totalConfirmedFunding(closed.state.acknowledgedSession.fundingByPanel),
    0,
  );
});

test("side-tools to logs replacement emits one net +40 funding command", () => {
  const sideFunding = funding("sideTools", 330, "replace-side-tools");
  const initialSession = acknowledgedSession({
    desiredOccupancy: sideToolsOpen,
    fundingByPanel: { sideTools: sideFunding },
    windowRevision: 3,
    sessionRevision: 5,
  });
  const initial = stateWithSession({
    desiredOccupancy: sideToolsOpen,
    session: initialSession,
  });
  const logsOpen = {
    globalSidebar: false,
    conversationRail: false,
    sideTools: false,
    threadLogs: true,
  };
  const started = beginIntent(initial, logsOpen, {
    cause: "user-panel",
    transactionId: "replace-right-panel",
  });
  const pendingFrame = projectHorizontalLayout(started.state).frame;
  assert.equal(pendingFrame.presentation.sideTools, "closed");
  assert.equal(
    pendingFrame.presentation.threadLogs,
    "closed",
    "the replacement must not open before its funding checkpoint",
  );
  const closing = acknowledgeCheckpoint(started);
  assert.deepEqual(
    closing.effects.map((effect) => effect.type),
    ["request-frame-commit"],
  );
  const closeFrame = projectHorizontalLayout(closing.state).frame;
  assert.equal(closeFrame.presentation.sideTools, "closed");
  assert.equal(
    closeFrame.presentation.threadLogs,
    "closed",
    "the replacement must stay closed while the close frame commits",
  );
  const committed = reduceHorizontalLayout(closing.state, {
    type: "FRAME_COMMITTED",
    transactionId: "replace-right-panel",
  });
  assert.deepEqual(
    committed.effects.map((effect) => effect.type),
    ["window-bounds"],
  );
  const awaitingBoundsFrame = projectHorizontalLayout(committed.state).frame;
  assert.equal(awaitingBoundsFrame.presentation.sideTools, "closed");
  assert.equal(
    awaitingBoundsFrame.presentation.threadLogs,
    "closed",
    "the replacement must not reopen until its funding bounds are applied",
  );
  const command = boundsEffect(committed).command;
  assert.equal(command.targetBounds.width - initial.snapshot.bounds.width, 40);
  assert.equal(command.targetFundingByPanel.sideTools, undefined);
  assert.equal(command.targetFundingByPanel.threadLogs.widthDelta, 370);
  assert.equal(command.authority.kind, "user-cause");
  const settled = acceptBounds(committed);
  const settledFrame = stableFrameFromProjection(
    projectHorizontalLayout(settled.state),
  );
  assert.equal(settledFrame.presentation.sideTools, "closed");
  assert.equal(settledFrame.presentation.threadLogs, "docked");
});

test("checkpoint stale retries with the same transaction sequence and token", () => {
  const started = beginIntent(stateWithSession(), sideToolsOpen);
  const first = started.effects[0].command;
  const staleSession = {
    ...started.state.acknowledgedSession,
    sessionRevision: 4,
  };
  const stale = reduceHorizontalLayout(started.state, {
    type: "WINDOW_LAYOUT_SESSION_REJECTED",
    rendererEpoch: started.state.rendererEpoch,
    transactionId: first.transactionId,
    reason: "stale",
    acknowledgedSession: staleSession,
  });
  const retry = stale.effects[0].command;
  assert.equal(retry.expectedSessionRevision, 4);
  assert.equal(retry.sequence, first.sequence);
  assert.equal(retry.transactionId, first.transactionId);
  assert.equal(
    stale.state.transactions[first.transactionId].checkpointAttempts,
    2,
  );

  const checkpointed = acknowledgeCheckpoint(stale, 5);
  const command = boundsEffect(checkpointed).command;
  assert.equal(command.authority.tokenId, `${stale.state.rendererEpoch}:1:transaction-1`);
});

test("bounds rejection reasons have distinct convergence paths", () => {
  for (const reason of [
    "stale",
    "fixed-mode",
    "outside-work-area",
    "superseded",
    "invalid",
  ]) {
    const awaiting = acknowledgeCheckpoint(
      beginIntent(stateWithSession(), sideToolsOpen),
    );
    const command = boundsEffect(awaiting).command;
    const authoritativeSnapshot = snapshot({
      width: 1480,
      revision: 2,
      mode: reason === "fixed-mode" ? "maximized" : "normal",
      origin: "mode",
    });
    const rejected = reduceHorizontalLayout(awaiting.state, {
      type: "WINDOW_BOUNDS_REJECTED",
      rendererEpoch: awaiting.state.rendererEpoch,
      transactionId: command.transactionId,
      sequence: command.sequence,
      reason,
      acknowledgedSession: {
        ...awaiting.state.acknowledgedSession,
        windowRevision: 2,
        sessionRevision: 3,
      },
      snapshot: authoritativeSnapshot,
    });
    const phase = rejected.state.transactions[command.transactionId].phase;
    if (reason === "stale") {
      assert.equal(phase, "awaiting-bounds");
      assert.equal(boundsEffect(rejected).command.authority.tokenId, command.authority.tokenId);
    } else if (reason === "fixed-mode") {
      assert.equal(phase, "deferred-funding");
      assert.deepEqual(rejected.effects, []);
    } else if (reason === "outside-work-area") {
      assert.equal(phase, "constrained");
      assert.deepEqual(rejected.effects, []);
    } else if (reason === "superseded") {
      assert.equal(phase, "superseded");
    } else {
      assert.equal(phase, "rejected");
      assert.equal(rejected.effects[0].type, "diagnostic");
    }
  }
});

test("stale opening ack that reveals fixed mode becomes deferred funding", () => {
  const awaiting = acknowledgeCheckpoint(
    beginIntent(stateWithSession(), sideToolsOpen),
  );
  const command = boundsEffect(awaiting).command;
  const stale = reduceHorizontalLayout(awaiting.state, {
    type: "WINDOW_BOUNDS_REJECTED",
    rendererEpoch: awaiting.state.rendererEpoch,
    transactionId: command.transactionId,
    sequence: command.sequence,
    reason: "stale",
    acknowledgedSession: {
      ...awaiting.state.acknowledgedSession,
      windowRevision: 2,
      sessionRevision: 3,
    },
    snapshot: snapshot({
      revision: 2,
      mode: "maximized",
      origin: "mode",
    }),
  });
  assert.equal(
    stale.state.transactions[command.transactionId].phase,
    "deferred-funding",
  );
  assert.deepEqual(stale.effects, []);
});

test("fixed-mode open defers funding and resumes with the original token", () => {
  const fixed = stateWithSession({ mode: "maximized" });
  const checkpointed = acknowledgeCheckpoint(
    beginIntent(fixed, sideToolsOpen),
  );
  const transaction = checkpointed.state.transactions["transaction-1"];
  assert.equal(transaction.phase, "deferred-funding");
  assert.deepEqual(checkpointed.effects, []);

  const resumed = reduceHorizontalLayout(checkpointed.state, {
    type: "WINDOW_SNAPSHOT_CHANGED",
    snapshot: snapshot({ revision: 2, mode: "normal", origin: "mode" }),
  });
  const command = boundsEffect(resumed).command;
  assert.equal(command.authority.tokenId, transaction.authority.tokenId);
  assert.equal(command.expectedWindowRevision, 2);
  assert.equal(resumed.state.responsiveBasisWidth, 1480);
});

test("fixed-mode funded close waits for frame commit and repays on exit", () => {
  const sideFunding = funding("sideTools", 330, "fixed-side-tools");
  const initialSession = acknowledgedSession({
    desiredOccupancy: sideToolsOpen,
    fundingByPanel: { sideTools: sideFunding },
    windowRevision: 5,
    sessionRevision: 7,
  });
  const fixed = stateWithSession({
    desiredOccupancy: sideToolsOpen,
    session: initialSession,
    mode: "maximized",
  });
  const closing = acknowledgeCheckpoint(
    beginIntent(fixed, CLOSED_LAYOUT_OCCUPANCY, {
      transactionId: "fixed-close",
    }),
  );
  assert.equal(
    closing.state.transactions["fixed-close"].phase,
    "frame-commit-pending",
  );
  const committed = reduceHorizontalLayout(closing.state, {
    type: "FRAME_COMMITTED",
    transactionId: "fixed-close",
  });
  assert.deepEqual(committed.effects, []);
  assert.equal(
    committed.state.transactions["fixed-close"].phase,
    "deferred-reconcile",
  );
  assert.equal(
    committed.state.transactions["fixed-close"].authority.kind,
    "user-cause",
  );

  const resumed = reduceHorizontalLayout(committed.state, {
    type: "WINDOW_SNAPSHOT_CHANGED",
    snapshot: snapshot({
      width: 1810,
      revision: 6,
      mode: "normal",
      origin: "mode",
    }),
  });
  const command = boundsEffect(resumed).command;
  assert.equal(command.targetBounds.width, 1480);
  assert.equal(command.authority.kind, "user-cause");
});

test("edge-constrained open checkpoints intent but never emits bounds", () => {
  const constrained = stateWithSession({ x: 0 });
  const checkpointed = acknowledgeCheckpoint(
    beginIntent(constrained, sideToolsOpen),
  );
  assert.deepEqual(checkpointed.effects, []);
  assert.equal(
    checkpointed.state.transactions["transaction-1"].phase,
    "constrained",
  );
  assert.deepEqual(
    checkpointed.state.acknowledgedSession.desiredOccupancy,
    sideToolsOpen,
  );
  assert.equal(
    totalConfirmedFunding(
      checkpointed.state.acknowledgedSession.fundingByPanel,
    ),
    0,
  );
});

test("hydrate intent can checkpoint visibility but can never expand", () => {
  const started = beginIntent(stateWithSession(), sideToolsOpen, {
    cause: "hydrate",
    transactionId: "hydrate-intent",
  });
  assert.equal(started.state.transactions["hydrate-intent"].authority, null);
  const checkpointed = acknowledgeCheckpoint(started);
  assert.deepEqual(checkpointed.effects, []);
  assert.equal(
    checkpointed.state.transactions["hydrate-intent"].phase,
    "constrained",
  );
});

test("responsive resize clears the completed transaction trigger", () => {
  const narrow = stateWithSession({
    x: 0,
    session: acknowledgedSession({
      baseWidth: 480,
      desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    }),
  });
  const constrained = acknowledgeCheckpoint(
    beginIntent(narrow, sideToolsOpen),
  );
  const triggered = projectHorizontalLayout(constrained.state);
  assert.equal(triggered.kind, "rejected");
  assert.equal(triggered.reason, "trigger-capacity");

  const resized = reduceHorizontalLayout(constrained.state, {
    type: "WINDOW_SNAPSHOT_CHANGED",
    snapshot: snapshot({
      width: 500,
      revision: 2,
      origin: "user",
      x: 0,
    }),
  });
  assert.equal(
    resized.state.transactions["transaction-1"].triggerPanel,
    null,
  );
  assert.equal(projectHorizontalLayout(resized.state).kind, "stable");
  assert.deepEqual(resized.state.desiredOccupancy, sideToolsOpen);
});

test("rapid reverse folds the superseded physical ack before closing", () => {
  const openAwaiting = acknowledgeCheckpoint(
    beginIntent(stateWithSession(), sideToolsOpen, {
      transactionId: "open-1",
    }),
  );
  const openCommand = boundsEffect(openAwaiting).command;
  const closeStarted = beginIntent(
    openAwaiting.state,
    CLOSED_LAYOUT_OCCUPANCY,
    { transactionId: "close-2" },
  );
  assert.equal(closeStarted.state.transactions["open-1"].phase, "superseded");

  const openAckSession = {
    normalBaseBounds: openCommand.targetNormalBaseBounds,
    fundingByPanel: openCommand.targetFundingByPanel,
    desiredOccupancy: sideToolsOpen,
    windowRevision: 2,
    sessionRevision: 3,
  };
  const oldApplied = reduceHorizontalLayout(closeStarted.state, {
    type: "WINDOW_BOUNDS_APPLIED",
    rendererEpoch: closeStarted.state.rendererEpoch,
    transactionId: "open-1",
    sequence: openCommand.sequence,
    acknowledgedSession: openAckSession,
    snapshot: snapshot({
      width: openCommand.targetBounds.width,
      revision: 2,
      origin: "panel-machine",
    }),
  });
  assert.deepEqual(oldApplied.state.desiredOccupancy, CLOSED_LAYOUT_OCCUPANCY);
  assert.equal(
    totalConfirmedFunding(oldApplied.state.acknowledgedSession.fundingByPanel),
    330,
  );
  assert.equal(
    oldApplied.state.transactions["close-2"].phase,
    "checkpoint-pending",
  );

  const closeCheckpoint = acknowledgeCheckpoint(
    { state: oldApplied.state, effects: closeStarted.effects },
    4,
  );
  assert.equal(
    closeCheckpoint.state.transactions["close-2"].phase,
    "frame-commit-pending",
  );
});

test("fresh claim is ordered, reload restores intent, and orphan funding repays", () => {
  const unhydrated = createHorizontalLayoutState({
    policy: "expand-v1",
    rendererEpoch: "hydrate-test",
    snapshot: snapshot(),
    hydrated: false,
  });
  const initialIntent = {
    globalSidebar: true,
    conversationRail: true,
    sideTools: false,
    threadLogs: false,
  };
  const fresh = reduceHorizontalLayout(unhydrated, {
    type: "HYDRATE",
    freshSession: true,
    snapshot: snapshot(),
    desiredOccupancy: initialIntent,
  });
  assert.deepEqual(
    fresh.effects.map((effect) => effect.type),
    ["claim-initial-layout"],
  );
  const claim = fresh.effects[0].command;
  assert.deepEqual(Object.keys(claim.targetFundingByPanel), [
    "globalSidebar",
    "conversationRail",
  ]);
  assert.equal(claim.targetNormalBaseBounds.width, 977);
  assert.deepEqual(claim.targetDesiredOccupancy, initialIntent);

  const constrainedSession = acknowledgedSession({
    desiredOccupancy: sideToolsOpen,
    fundingByPanel: {},
    sessionRevision: 10,
  });
  const reload = reduceHorizontalLayout(unhydrated, {
    type: "HYDRATE",
    freshSession: false,
    snapshot: snapshot(),
    acknowledgedSession: constrainedSession,
  });
  assert.deepEqual(reload.state.desiredOccupancy, sideToolsOpen);
  assert.deepEqual(reload.effects, []);

  const sideFunding = funding("sideTools", 330, "orphaned-side-tools");
  const orphanSession = acknowledgedSession({
    desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    fundingByPanel: { sideTools: sideFunding },
    windowRevision: 4,
    sessionRevision: 11,
  });
  const orphan = reduceHorizontalLayout(unhydrated, {
    type: "HYDRATE",
    freshSession: false,
    snapshot: snapshot({ width: 1810, revision: 4 }),
    acknowledgedSession: orphanSession,
  });
  const repay = boundsEffect(orphan).command;
  assert.equal(repay.authority.kind, "repay-proof");
  assert.deepEqual(repay.authority.fundingIds, ["orphaned-side-tools"]);
  assert.equal(repay.targetBounds.width, 1480);
});

test("initial claim accepts, retries only a still-fresh CAS, and adopts takeover", () => {
  const unhydrated = createHorizontalLayoutState({
    policy: "expand-v1",
    rendererEpoch: "claim-lifecycle",
    snapshot: snapshot(),
    hydrated: false,
  });
  const fresh = reduceHorizontalLayout(unhydrated, {
    type: "HYDRATE",
    freshSession: true,
    snapshot: snapshot(),
    desiredOccupancy: sidebarOpen,
  });
  const claim = fresh.effects[0].command;
  const claimedSession = {
    normalBaseBounds: claim.targetNormalBaseBounds,
    fundingByPanel: claim.targetFundingByPanel,
    desiredOccupancy: claim.targetDesiredOccupancy,
    windowRevision: 1,
    sessionRevision: 1,
  };
  const applied = reduceHorizontalLayout(fresh.state, {
    type: "CLAIM_INITIAL_LAYOUT_APPLIED",
    rendererEpoch: fresh.state.rendererEpoch,
    acknowledgedSession: claimedSession,
    snapshot: snapshot(),
  });
  assert.equal(applied.state.pendingInitialClaim, false);
  assert.equal(
    applied.state.acknowledgedSession.fundingByPanel.globalSidebar.widthDelta,
    245,
  );
  const initialClose = acknowledgeCheckpoint(
    beginIntent(applied.state, CLOSED_LAYOUT_OCCUPANCY, {
      transactionId: "close-initial-sidebar",
    }),
  );
  const initialCloseCommitted = reduceHorizontalLayout(
    initialClose.state,
    {
      type: "FRAME_COMMITTED",
      transactionId: "close-initial-sidebar",
    },
  );
  assert.equal(boundsEffect(initialCloseCommitted).command.targetBounds.width, 1235);

  const stillFreshSession = acknowledgedSession({
    desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    windowRevision: 2,
    sessionRevision: 0,
  });
  const retried = reduceHorizontalLayout(fresh.state, {
    type: "CLAIM_INITIAL_LAYOUT_REJECTED",
    rendererEpoch: fresh.state.rendererEpoch,
    reason: "stale",
    acknowledgedSession: stillFreshSession,
    snapshot: snapshot({ revision: 2 }),
  });
  assert.deepEqual(
    retried.effects.map((effect) => effect.type),
    ["claim-initial-layout"],
  );
  assert.equal(retried.effects[0].command.expectedWindowRevision, 2);

  const takeoverSession = acknowledgedSession({
    desiredOccupancy: sideToolsOpen,
    fundingByPanel: {},
    windowRevision: 3,
    sessionRevision: 2,
  });
  const takeover = reduceHorizontalLayout(fresh.state, {
    type: "CLAIM_INITIAL_LAYOUT_REJECTED",
    rendererEpoch: fresh.state.rendererEpoch,
    reason: "stale",
    acknowledgedSession: takeoverSession,
    snapshot: snapshot({ revision: 3 }),
  });
  assert.deepEqual(takeover.effects, []);
  assert.equal(takeover.state.pendingInitialClaim, false);
  assert.deepEqual(takeover.state.desiredOccupancy, sideToolsOpen);
});

test("fixed-mode orphaned funding carries RepayProof until normal mode", () => {
  const unhydrated = createHorizontalLayoutState({
    policy: "expand-v1",
    rendererEpoch: "fixed-hydrate",
    snapshot: snapshot({ mode: "maximized" }),
    hydrated: false,
  });
  const orphanSession = acknowledgedSession({
    desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    fundingByPanel: {
      sideTools: funding("sideTools", 330, "fixed-orphan"),
    },
    windowRevision: 4,
    sessionRevision: 8,
  });
  const hydrated = reduceHorizontalLayout(unhydrated, {
    type: "HYDRATE",
    freshSession: false,
    snapshot: snapshot({ width: 1810, revision: 4, mode: "maximized" }),
    acknowledgedSession: orphanSession,
  });
  assert.deepEqual(hydrated.effects, []);
  const transaction = hydrated.state.transactions["hydrate-orphaned-funding"];
  assert.equal(transaction.phase, "deferred-reconcile");
  assert.equal(transaction.authority.kind, "repay-proof");

  const resumed = reduceHorizontalLayout(hydrated.state, {
    type: "WINDOW_SNAPSHOT_CHANGED",
    snapshot: snapshot({
      width: 1810,
      revision: 5,
      mode: "normal",
      origin: "mode",
    }),
  });
  const command = boundsEffect(resumed).command;
  assert.equal(command.authority.kind, "repay-proof");
  assert.deepEqual(command.authority.fundingIds, ["fixed-orphan"]);
});

test("responsive snapshots preserve intent and only user/display update basis", () => {
  let state = stateWithSession({ desiredOccupancy: sideToolsOpen });
  state = { ...state, sideToolsManualOverride: true };
  for (const [revision, width, origin, expectedBasis] of [
    [2, 1600, "panel-machine", 1480],
    [3, 1550, "mode", 1480],
    [4, 900, "user", 900],
    [4, 800, "display", 900],
    [5, 800, "display", 800],
  ]) {
    const reduced = reduceHorizontalLayout(state, {
      type: "WINDOW_SNAPSHOT_CHANGED",
      snapshot: snapshot({ width, revision, origin }),
    });
    assert.deepEqual(reduced.effects, []);
    state = reduced.state;
    assert.equal(state.responsiveBasisWidth, expectedBasis);
    assert.deepEqual(state.desiredOccupancy, sideToolsOpen);
  }
  assert.equal(state.sideToolsManualOverride, false);

  const native = reduceHorizontalLayout(state, {
    type: "VIEWPORT_RESIZED_DURING_NATIVE_SESSION",
    snapshot: snapshot({ width: 1200, revision: 6, origin: "user" }),
  });
  assert.equal(native.state.responsiveBasisWidth, 800);
  assert.deepEqual(native.effects, []);
});

test("responsive narrowing hides the conversation rail before the global sidebar", () => {
  const dualRailOpen = {
    globalSidebar: true,
    conversationRail: true,
    sideTools: false,
    threadLogs: false,
  };
  const session = acknowledgedSession({
    baseWidth: 981,
    desiredOccupancy: dualRailOpen,
  });
  let state = stateWithSession({
    desiredOccupancy: dualRailOpen,
    session,
  });
  assert.equal(
    stableFrameFromProjection(projectHorizontalLayout(state)).presentation
      .conversationRail,
    "open",
  );

  state = reduceHorizontalLayout(state, {
    type: "WINDOW_SNAPSHOT_CHANGED",
    snapshot: snapshot({ width: 980, revision: 2, origin: "user" }),
  }).state;
  let frame = stableFrameFromProjection(projectHorizontalLayout(state));
  assert.equal(frame.presentation.conversationRail, "hidden");
  assert.equal(frame.presentation.globalSidebar, "expanded");
  assert.deepEqual(state.desiredOccupancy, dualRailOpen);

  state = reduceHorizontalLayout(state, {
    type: "WINDOW_SNAPSHOT_CHANGED",
    snapshot: snapshot({ width: 981, revision: 3, origin: "user" }),
  }).state;
  frame = stableFrameFromProjection(projectHorizontalLayout(state));
  assert.equal(frame.presentation.conversationRail, "open");
  assert.equal(frame.presentation.globalSidebar, "expanded");
});

test("explicit compact sidebar expansion grows native bounds before presenting in flow", () => {
  const session = acknowledgedSession({
    baseWidth: 720,
    desiredOccupancy: sidebarOpen,
  });
  const initial = stateWithSession({
    desiredOccupancy: sidebarOpen,
    session,
  });
  assert.equal(
    stableFrameFromProjection(projectHorizontalLayout(initial)).presentation
      .globalSidebar,
    "collapsed",
  );

  const started = reduceHorizontalLayout(initial, {
    type: "COMPACT_SIDEBAR_TOGGLED",
  });
  const transaction = started.state.transactions[
    started.state.headTransactionId
  ];
  assert.deepEqual(transaction.openingPanels, ["globalSidebar"]);
  assert.deepEqual(
    started.effects.map((effect) => effect.type),
    ["window-layout-session"],
  );
  assert.equal(
    stableFrameFromProjection(projectHorizontalLayout(started.state))
      .presentation.globalSidebar,
    "collapsed",
  );

  const awaitingBounds = acknowledgeCheckpoint(started);
  const command = boundsEffect(awaitingBounds).command;
  assert.equal(command.targetBounds.width, 965);
  assert.equal(command.targetFundingByPanel.globalSidebar.widthDelta, 245);

  const accepted = acceptBounds(awaitingBounds);
  const frame = stableFrameFromProjection(
    projectHorizontalLayout(accepted.state),
  );
  assert.equal(accepted.state.responsiveBasisWidth, 720);
  assert.equal(frame.contentViewportWidth, 965);
  assert.equal(frame.presentation.globalSidebar, "expanded");
  assert.equal(frame.columns.globalSidebar, 245);
  assert.notEqual(frame.presentation.globalSidebar, "compact-overlay");
});

test("authoritative user resize folds the rebased acknowledged session", () => {
  const sideFunding = funding("sideTools", 330, "resize-side-tools");
  const initialSession = acknowledgedSession({
    desiredOccupancy: sideToolsOpen,
    fundingByPanel: { sideTools: sideFunding },
    windowRevision: 1,
    sessionRevision: 1,
  });
  const initial = stateWithSession({
    desiredOccupancy: sideToolsOpen,
    session: initialSession,
  });
  const rebasedSession = {
    ...initialSession,
    normalBaseBounds: {
      ...initialSession.normalBaseBounds,
      width: 1370,
    },
    windowRevision: 2,
    sessionRevision: 2,
  };
  const resized = reduceHorizontalLayout(initial, {
    type: "WINDOW_SNAPSHOT_CHANGED",
    snapshot: snapshot({ width: 1700, revision: 2, origin: "user" }),
    acknowledgedSession: rebasedSession,
  });
  assert.deepEqual(resized.effects, []);
  assert.equal(
    resized.state.acknowledgedSession.normalBaseBounds.width,
    1370,
  );
  assert.equal(
    boundsForAcknowledgedSession(resized.state.acknowledgedSession).width,
    1700,
  );
  assert.equal(resized.state.responsiveBasisWidth, 1700);
  assert.deepEqual(resized.state.desiredOccupancy, sideToolsOpen);
});

test("accepted physical facts fold by window revision, never by sequence", () => {
  const initial = stateWithSession({ desiredOccupancy: sideToolsOpen });
  const newerFunding = funding("sideTools", 330, "physical-newer");
  const newerSession = acknowledgedSession({
    desiredOccupancy: sideToolsOpen,
    fundingByPanel: { sideTools: newerFunding },
    windowRevision: 7,
    sessionRevision: 7,
  });
  const newer = reduceHorizontalLayout(initial, {
    type: "WINDOW_BOUNDS_APPLIED",
    rendererEpoch: initial.rendererEpoch,
    transactionId: "old-sequence",
    sequence: 1,
    acknowledgedSession: newerSession,
    snapshot: snapshot({ width: 1810, revision: 7, origin: "panel-machine" }),
  });
  assert.equal(newer.state.snapshot.windowRevision, 7);
  assert.equal(
    newer.state.acknowledgedSession.fundingByPanel.sideTools.fundingId,
    "physical-newer",
  );

  const stale = reduceHorizontalLayout(newer.state, {
    type: "WINDOW_BOUNDS_APPLIED",
    rendererEpoch: newer.state.rendererEpoch,
    transactionId: "newer-sequence",
    sequence: 99,
    acknowledgedSession: acknowledgedSession({
      desiredOccupancy: sideToolsOpen,
      windowRevision: 6,
      sessionRevision: 6,
    }),
    snapshot: snapshot({ width: 1480, revision: 6, origin: "panel-machine" }),
  });
  assert.equal(stale.state.snapshot.windowRevision, 7);
  assert.equal(
    stale.state.acknowledgedSession.fundingByPanel.sideTools.fundingId,
    "physical-newer",
  );
});

test("panel width events never mint bounds authority and persist only logs", () => {
  const initial = stateWithSession({ desiredOccupancy: sidebarOpen });
  const sidePreview = reduceHorizontalLayout(initial, {
    type: "PANEL_WIDTH_CHANGED",
    panel: "sideTools",
    width: 480,
    commit: false,
  });
  assert.deepEqual(sidePreview.effects, []);
  assert.equal(sidePreview.state.widths.sideTools, 480);
  assert.deepEqual(sidePreview.state.desiredOccupancy, sidebarOpen);

  const sideCommit = reduceHorizontalLayout(sidePreview.state, {
    type: "PANEL_WIDTH_CHANGED",
    panel: "sideTools",
    width: 500,
    commit: true,
  });
  assert.deepEqual(sideCommit.effects, []);

  const logsCommit = reduceHorizontalLayout(sideCommit.state, {
    type: "PANEL_WIDTH_CHANGED",
    panel: "threadLogs",
    width: 500,
    commit: true,
  });
  assert.deepEqual(logsCommit.effects, [
    {
      type: "persist-preference",
      preference: "panel-width",
      panel: "threadLogs",
      value: 500,
    },
  ]);
});

test("RepayProof cannot add, expand, cite unknown funding, or cross revisions", () => {
  const sideFunding = funding("sideTools", 330, "proof-side");
  const session = acknowledgedSession({
    desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    fundingByPanel: { sideTools: sideFunding },
    sessionRevision: 9,
  });
  const state = stateWithSession({
    desiredOccupancy: CLOSED_LAYOUT_OCCUPANCY,
    session,
  });
  const hydrated = reduceHorizontalLayout(
    createHorizontalLayoutState({
      policy: "expand-v1",
      rendererEpoch: state.rendererEpoch,
      snapshot: state.snapshot,
      hydrated: false,
    }),
    {
      type: "HYDRATE",
      freshSession: false,
      snapshot: state.snapshot,
      acknowledgedSession: session,
    },
  );
  const valid = boundsEffect(hydrated).command;
  assert.equal(validateBoundsCommandAuthority(valid, session).valid, true);

  const wrongRevision = structuredClone(valid);
  wrongRevision.authority.expectedSessionRevision = 8;
  assert.equal(
    validateBoundsCommandAuthority(wrongRevision, session).valid,
    false,
  );
  const unknown = structuredClone(valid);
  unknown.authority.fundingIds = ["unknown"];
  assert.equal(validateBoundsCommandAuthority(unknown, session).valid, false);
  const expands = structuredClone(valid);
  expands.targetFundingByPanel.sideTools = sideFunding;
  expands.targetBounds.width = 1810;
  assert.equal(validateBoundsCommandAuthority(expands, session).valid, false);
  const rewrites = structuredClone(valid);
  rewrites.targetFundingByPanel.sideTools = {
    ...sideFunding,
    widthDelta: 100,
  };
  rewrites.targetBounds.width = 1580;
  assert.equal(validateBoundsCommandAuthority(rewrites, session).valid, false);

  const userCommand = {
    ...valid,
    authority: {
      kind: "user-cause",
      tokenId: "token",
      transactionId: "another-transaction",
      cause: "user-panel",
      rendererEpoch: valid.rendererEpoch,
      sequence: valid.sequence,
    },
  };
  assert.equal(
    validateBoundsCommandAuthority(userCommand, session).valid,
    false,
  );
});

test("old renderer epoch results cannot mutate the takeover state", () => {
  const state = stateWithSession({ epoch: "new-epoch" });
  const result = reduceHorizontalLayout(state, {
    type: "WINDOW_BOUNDS_APPLIED",
    rendererEpoch: "old-epoch",
    transactionId: "old-command",
    sequence: 99,
    acknowledgedSession: acknowledgedSession({
      windowRevision: 9,
      sessionRevision: 9,
    }),
    snapshot: snapshot({ width: 1900, revision: 9 }),
  });
  assert.equal(result.state, state);
  assert.deepEqual(result.effects, []);
});
