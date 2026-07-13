import assert from "node:assert/strict";
import test from "node:test";

import { WindowLayoutExecutor } from "./window-layout-executor.ts";

const closed = Object.freeze({
  globalSidebar: false,
  conversationRail: false,
  sideTools: false,
});
const sideToolsOpen = Object.freeze({ ...closed, sideTools: true });

function createHost({ clampWidth = 0 } = {}) {
  let environment = {
    bounds: { x: 100, y: 80, width: 1480, height: 800 },
    contentBounds: { x: 100, y: 80, width: 1480, height: 800 },
    normalBounds: { x: 100, y: 80, width: 1480, height: 800 },
    workArea: { x: 0, y: 0, width: 4000, height: 1400 },
    mode: "normal",
    displayId: "fake-display",
    scaleFactor: 2,
  };
  let setBoundsCount = 0;
  return {
    host: {
      windowId: 7,
      readEnvironment() {
        return structuredClone(environment);
      },
      setBounds(bounds) {
        setBoundsCount += 1;
        const actual = { ...bounds, width: bounds.width + clampWidth };
        environment = {
          ...environment,
          bounds: actual,
          contentBounds: actual,
          normalBounds: actual,
        };
      },
    },
    get environment() {
      return environment;
    },
    set environment(next) {
      environment = next;
    },
    get setBoundsCount() {
      return setBoundsCount;
    },
  };
}

const sender = Object.freeze({ senderWindowId: 7 });

function claimClosed(bootstrap, epoch) {
  return {
    type: "CLAIM_INITIAL_LAYOUT",
    expectedWindowRevision: bootstrap.snapshot.windowRevision,
    expectedSessionRevision: bootstrap.acknowledgedSession.sessionRevision,
    targetNormalBaseBounds: bootstrap.snapshot.normalBounds,
    targetFundingByPanel: {},
    targetDesiredOccupancy: closed,
    transactionId: "claim-initial-layout",
    rendererEpoch: epoch,
    sequence: 0,
  };
}

function sideToolsCheckpoint(session, epoch, sequence = 1) {
  return {
    type: "CHECKPOINT_DESIRED_OCCUPANCY",
    expectedSessionRevision: session.sessionRevision,
    desiredOccupancy: sideToolsOpen,
    transactionId: "open-side-tools",
    rendererEpoch: epoch,
    sequence,
  };
}

function sideToolsBounds(checkpointResult, epoch, sequence = 1) {
  const session = checkpointResult.acknowledgedSession;
  const fundingId = `${epoch}:${sequence}:sideTools`;
  const funding = {
    fundingId,
    panel: "sideTools",
    widthDelta: 330,
    xCompensation: 0,
    repayAuthority: { fundingId },
  };
  return {
    type: "APPLY_WINDOW_BOUNDS",
    authority: {
      kind: "user-cause",
      tokenId: `${epoch}:${sequence}:open-side-tools`,
      transactionId: "open-side-tools",
      cause: "user-panel",
      rendererEpoch: epoch,
      sequence,
    },
    expectedWindowRevision: session.windowRevision,
    expectedSessionRevision: session.sessionRevision,
    targetBounds: {
      ...session.normalBaseBounds,
      width: session.normalBaseBounds.width + 330,
    },
    targetNormalBaseBounds: session.normalBaseBounds,
    targetFundingByPanel: { sideTools: funding },
    targetDesiredOccupancy: sideToolsOpen,
    transactionId: "open-side-tools",
    rendererEpoch: epoch,
    sequence,
  };
}

async function claimedExecutor(options = {}) {
  const fake = createHost(options.host);
  const updates = [];
  const executor = new WindowLayoutExecutor({
    host: fake.host,
    policy: "expand-v1",
    ackDelayMs: options.ackDelayMs,
    onSnapshot: (update) => updates.push(update),
  });
  const epoch = options.epoch ?? "executor-epoch";
  const bootstrap = executor.bootstrap(epoch, sender);
  const claim = await executor.execute(claimClosed(bootstrap, epoch), sender);
  assert.equal(claim.accepted, true);
  return { executor, fake, epoch, claim, updates };
}

test("fake BrowserWindow executor performs checkpoint -> dual CAS -> one setBounds -> readback", async () => {
  const { executor, fake, epoch, claim, updates } = await claimedExecutor({
    host: { clampWidth: -2 },
  });
  const checkpoint = await executor.execute(
    sideToolsCheckpoint(claim.acknowledgedSession, epoch),
    sender,
  );
  assert.equal(checkpoint.accepted, true);
  const command = sideToolsBounds(checkpoint, epoch);
  const applied = await executor.execute(command, sender);

  assert.equal(applied.accepted, true);
  assert.equal(applied.setBoundsApplied, true);
  assert.equal(fake.setBoundsCount, 1);
  assert.equal(applied.snapshot.bounds.width, 1808);
  assert.equal(applied.acknowledgedSession.normalBaseBounds.width, 1478);
  assert.equal(applied.acknowledgedSession.fundingByPanel.sideTools.widthDelta, 330);
  assert.equal(applied.acknowledgedSession.windowRevision, 2);
  assert.equal(applied.acknowledgedSession.sessionRevision, 3);
  assert.equal(updates.length, 1);

  const duplicate = await executor.execute(command, sender);
  assert.equal(duplicate.accepted, false);
  assert.equal(duplicate.reason, "stale");
  assert.equal(fake.setBoundsCount, 1);
});

test("fresh initial claim funds the visible sidebar without moving the window", async () => {
  const fake = createHost();
  const executor = new WindowLayoutExecutor({
    host: fake.host,
    policy: "expand-v1",
  });
  const epoch = "sidebar-claim";
  const bootstrap = executor.bootstrap(epoch, sender);
  const fundingId = `claim:${epoch}:globalSidebar`;
  const claim = await executor.execute(
    {
      ...claimClosed(bootstrap, epoch),
      targetNormalBaseBounds: {
        ...bootstrap.snapshot.normalBounds,
        width: 1235,
      },
      targetFundingByPanel: {
        globalSidebar: {
          fundingId,
          panel: "globalSidebar",
          widthDelta: 245,
          xCompensation: 0,
          repayAuthority: { fundingId },
        },
      },
      targetDesiredOccupancy: { ...closed, globalSidebar: true },
    },
    sender,
  );
  assert.equal(claim.accepted, true);
  assert.equal(claim.setBoundsApplied, false);
  assert.equal(fake.setBoundsCount, 0);
  assert.equal(claim.snapshot.bounds.width, 1480);
  assert.equal(claim.acknowledgedSession.normalBaseBounds.width, 1235);
  assert.equal(
    claim.acknowledgedSession.fundingByPanel.globalSidebar.widthDelta,
    245,
  );
});

test("sender binding, work-area TOCTOU, and fixed mode reject without setBounds", async () => {
  const { executor, fake, epoch, claim } = await claimedExecutor();
  const unauthorized = await executor.execute(
    sideToolsCheckpoint(claim.acknowledgedSession, epoch),
    { senderWindowId: 99 },
  );
  assert.equal(unauthorized.accepted, false);
  assert.equal(unauthorized.reason, "invalid");

  const checkpoint = await executor.execute(
    sideToolsCheckpoint(claim.acknowledgedSession, epoch),
    sender,
  );
  const command = sideToolsBounds(checkpoint, epoch);
  fake.environment = {
    ...fake.environment,
    workArea: { ...fake.environment.workArea, width: 1800 },
  };
  const outside = await executor.execute(command, sender);
  assert.equal(outside.accepted, false);
  assert.equal(outside.reason, "outside-work-area");
  assert.equal(fake.setBoundsCount, 0);

  const fixed = await claimedExecutor({ epoch: "fixed-epoch" });
  const fixedCheckpoint = await fixed.executor.execute(
    sideToolsCheckpoint(fixed.claim.acknowledgedSession, fixed.epoch),
    sender,
  );
  const fixedCommand = sideToolsBounds(fixedCheckpoint, fixed.epoch);
  fixed.fake.environment = { ...fixed.fake.environment, mode: "maximized" };
  const fixedResult = await fixed.executor.execute(fixedCommand, sender);
  assert.equal(fixedResult.accepted, false);
  assert.equal(fixedResult.reason, "fixed-mode");
  assert.equal(fixed.fake.setBoundsCount, 0);
});

test("malformed bounds are invalid rather than a work-area constraint", async () => {
  const { executor, fake, epoch, claim } = await claimedExecutor({
    epoch: "invalid-bounds",
  });
  const checkpoint = await executor.execute(
    sideToolsCheckpoint(claim.acknowledgedSession, epoch),
    sender,
  );
  const command = sideToolsBounds(checkpoint, epoch);
  const result = await executor.execute(
    {
      ...command,
      targetBounds: { ...command.targetBounds, x: Number.NaN },
    },
    sender,
  );
  assert.equal(result.accepted, false);
  assert.equal(result.reason, "invalid");
  assert.equal(fake.setBoundsCount, 0);
});

test("queue coalesces an unstarted lower sequence and commits only the head", async () => {
  const { executor, epoch, claim } = await claimedExecutor();
  const first = sideToolsCheckpoint(claim.acknowledgedSession, epoch, 1);
  const second = {
    ...sideToolsCheckpoint(claim.acknowledgedSession, epoch, 2),
    transactionId: "replace-with-rail",
    desiredOccupancy: { ...closed, conversationRail: true },
  };
  const firstPromise = executor.execute(first, sender);
  const secondPromise = executor.execute(second, sender);
  const [firstResult, secondResult] = await Promise.all([
    firstPromise,
    secondPromise,
  ]);
  assert.equal(firstResult.accepted, false);
  assert.equal(firstResult.reason, "superseded");
  assert.equal(secondResult.accepted, true);
  assert.deepEqual(
    secondResult.acknowledgedSession.desiredOccupancy,
    second.desiredOccupancy,
  );
});

test("150ms-style delayed ack commits physical facts before return and survives epoch takeover", async () => {
  const { executor, fake, epoch, claim } = await claimedExecutor({
    ackDelayMs: 20,
    epoch: "delayed-old",
  });
  const checkpoint = await executor.execute(
    sideToolsCheckpoint(claim.acknowledgedSession, epoch),
    sender,
  );
  const pending = executor.execute(sideToolsBounds(checkpoint, epoch), sender);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(fake.environment.bounds.width, 1810);
  assert.equal(executor.debugState().acknowledgedSession.windowRevision, 2);

  const takeover = executor.bootstrap("delayed-new", sender);
  assert.equal(takeover.freshSession, false);
  assert.equal(takeover.acknowledgedSession.windowRevision, 2);
  const late = await pending;
  assert.equal(late.accepted, true);
  assert.equal(late.snapshot.bounds.width, 1810);
});

test("external user/display snapshots rebase the acknowledged normal session monotonically", async () => {
  const { executor, fake } = await claimedExecutor();
  fake.environment = {
    ...fake.environment,
    bounds: { ...fake.environment.bounds, width: 1600 },
    contentBounds: { ...fake.environment.contentBounds, width: 1600 },
    normalBounds: { ...fake.environment.normalBounds, width: 1600 },
  };
  const update = executor.syncExternalEnvironment("user");
  assert.equal(update.snapshot.origin, "user");
  assert.equal(update.snapshot.windowRevision, 2);
  assert.equal(update.acknowledgedSession.windowRevision, 2);
  assert.equal(update.acknowledgedSession.sessionRevision, 2);
  assert.equal(update.acknowledgedSession.normalBaseBounds.width, 1600);
  assert.equal(executor.syncExternalEnvironment("display"), null);
});

test("a manual width resize adopts the user's bounds and clears prior panel funding", async () => {
  const epoch = "manual-resize-rebase";
  const { executor, fake, claim } = await claimedExecutor({ epoch });
  const checkpoint = await executor.execute(
    sideToolsCheckpoint(claim.acknowledgedSession, epoch),
    sender,
  );
  const expanded = await executor.execute(sideToolsBounds(checkpoint, epoch), sender);
  assert.equal(expanded.accepted, true);
  assert.ok(expanded.acknowledgedSession.fundingByPanel.sideTools);

  fake.environment = {
    ...fake.environment,
    bounds: { ...fake.environment.bounds, width: 1200 },
    contentBounds: { ...fake.environment.contentBounds, width: 1200 },
    normalBounds: { ...fake.environment.normalBounds, width: 1200 },
  };
  const resized = executor.syncExternalEnvironment("user");

  assert.deepEqual(resized.acknowledgedSession.fundingByPanel, {});
  assert.equal(resized.acknowledgedSession.normalBaseBounds.width, 1200);
  assert.equal(resized.snapshot.bounds.width, 1200);
});

test("a height-only resize preserves horizontal panel funding", async () => {
  const epoch = "height-resize-keeps-funding";
  const { executor, fake, claim } = await claimedExecutor({ epoch });
  const checkpoint = await executor.execute(
    sideToolsCheckpoint(claim.acknowledgedSession, epoch),
    sender,
  );
  const expanded = await executor.execute(sideToolsBounds(checkpoint, epoch), sender);
  assert.equal(expanded.accepted, true);
  assert.ok(expanded.acknowledgedSession.fundingByPanel.sideTools);

  fake.environment = {
    ...fake.environment,
    bounds: { ...fake.environment.bounds, height: 720 },
    contentBounds: { ...fake.environment.contentBounds, height: 720 },
    normalBounds: { ...fake.environment.normalBounds, height: 720 },
  };
  const resized = executor.syncExternalEnvironment("user");

  assert.deepEqual(
    resized.acknowledgedSession.fundingByPanel,
    expanded.acknowledgedSession.fundingByPanel,
  );
  assert.equal(
    resized.acknowledgedSession.normalBaseBounds.width,
    expanded.acknowledgedSession.normalBaseBounds.width,
  );
  assert.equal(resized.snapshot.bounds.height, 720);
});
