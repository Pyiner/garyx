import assert from "node:assert/strict";
import test from "node:test";

import { WindowLayoutExecutor } from "../../../main/window-layout-executor.ts";

import { createHorizontalLayoutEffectRunner } from "./horizontal-layout-effect-runner.ts";
import { createHorizontalLayoutFrameStore } from "./horizontal-layout-frame-store.ts";

const closed = Object.freeze({
  globalSidebar: false,
  conversationRail: false,
  sideTools: false,
});
const sideToolsOpen = Object.freeze({ ...closed, sideTools: true });

function snapshot(width = 1480, revision = 1) {
  const bounds = { x: 100, y: 80, width, height: 800 };
  return {
    windowRevision: revision,
    bounds,
    contentBounds: bounds,
    normalBounds: bounds,
    workArea: { x: 0, y: 0, width: 4000, height: 1400 },
    mode: "normal",
    displayId: "effect-runner-display",
    scaleFactor: 2,
    origin: "hydrate",
  };
}

async function flushEffects() {
  for (let index = 0; index < 8; index += 1) {
    await Promise.resolve();
  }
}

test("live effect runner closes open/checkpoint/bounds and close/frame/repay loops", async () => {
  const epoch = "effect-runner-epoch";
  const sender = { senderWindowId: 17 };
  const initialSnapshot = snapshot();
  let environment = {
    bounds: structuredClone(initialSnapshot.bounds),
    contentBounds: structuredClone(initialSnapshot.contentBounds),
    normalBounds: structuredClone(initialSnapshot.normalBounds),
    workArea: structuredClone(initialSnapshot.workArea),
    mode: initialSnapshot.mode,
    displayId: initialSnapshot.displayId,
    scaleFactor: initialSnapshot.scaleFactor,
  };
  let setBoundsCount = 0;
  const listeners = new Set();
  const executor = new WindowLayoutExecutor({
    policy: "expand-v1",
    host: {
      windowId: sender.senderWindowId,
      readEnvironment() {
        return structuredClone(environment);
      },
      setBounds(bounds) {
        setBoundsCount += 1;
        environment = {
          ...environment,
          bounds: structuredClone(bounds),
          contentBounds: structuredClone(bounds),
          normalBounds: structuredClone(bounds),
        };
      },
    },
    onSnapshot(update) {
      for (const listener of listeners) {
        listener(update);
      }
    },
  });
  const bootstrap = executor.bootstrap(epoch, sender);
  const claim = await executor.execute(
    {
      type: "CLAIM_INITIAL_LAYOUT",
      expectedWindowRevision: bootstrap.snapshot.windowRevision,
      expectedSessionRevision:
        bootstrap.acknowledgedSession.sessionRevision,
      targetNormalBaseBounds: bootstrap.snapshot.normalBounds,
      targetFundingByPanel: {},
      targetDesiredOccupancy: closed,
      transactionId: "claim-initial-layout",
      rendererEpoch: epoch,
      sequence: 0,
    },
    sender,
  );
  assert.equal(claim.accepted, true);
  const commands = [];
  const api = {
    executeWindowLayoutCommand(command) {
      commands.push(command);
      return executor.execute(command, sender);
    },
    subscribeWindowLayoutSnapshots(listener) {
      listeners.add(listener);
    },
    unsubscribeWindowLayoutSnapshots(listener) {
      listeners.delete(listener);
    },
  };
  const timers = [];
  const frames = [];
  let nextHandle = 1;
  const store = createHorizontalLayoutFrameStore({
    policy: "expand-v1",
    rendererEpoch: epoch,
    snapshot: claim.snapshot,
    desiredOccupancy: closed,
    acknowledgedSession: claim.acknowledgedSession,
  });
  const runner = createHorizontalLayoutEffectRunner({
    api,
    store,
    scheduleTimeout(callback, delay) {
      const handle = nextHandle++;
      timers.push({ callback, delay, handle });
      return handle;
    },
    cancelTimeout() {},
    scheduleFrame(callback) {
      const handle = nextHandle++;
      frames.push({ callback, handle, cancelled: false });
      return handle;
    },
    cancelFrame(handle) {
      const frame = frames.find((candidate) => candidate.handle === handle);
      if (frame) {
        frame.cancelled = true;
      }
    },
  });

  runner.dispatch({
    type: "LAYOUT_INTENT_CHANGED",
    previousOccupancy: closed,
    nextOccupancy: sideToolsOpen,
    cause: "user-panel",
    transactionId: "open-side-tools",
  });
  await flushEffects();

  assert.deepEqual(commands.map((command) => command.type), [
    "CHECKPOINT_DESIRED_OCCUPANCY",
    "APPLY_WINDOW_BOUNDS",
  ]);
  assert.equal(store.getState().transactions["open-side-tools"].phase, "settled");
  assert.equal(store.getSnapshot().presentation.sideTools, "docked");
  assert.equal(store.getSnapshot().contentViewportWidth, 2130);
  assert.equal(timers.find((timer) => timer.delay === 100) !== undefined, true);

  runner.dispatch({
    type: "LAYOUT_INTENT_CHANGED",
    previousOccupancy: sideToolsOpen,
    nextOccupancy: closed,
    cause: "user-panel",
    transactionId: "close-side-tools",
  });
  await flushEffects();
  assert.equal(
    store.getState().transactions["close-side-tools"].phase,
    "frame-commit-pending",
  );
  assert.equal(store.getSnapshot().presentation.sideTools, "closed");
  const beforeDeadlineRevision = store.getSnapshot().revision;

  const closeFrame = frames.find((frame) => !frame.cancelled);
  assert.ok(closeFrame);
  closeFrame.callback();
  await flushEffects();
  assert.ok(store.getSnapshot().revision > beforeDeadlineRevision);

  assert.deepEqual(commands.map((command) => command.type), [
    "CHECKPOINT_DESIRED_OCCUPANCY",
    "APPLY_WINDOW_BOUNDS",
    "CHECKPOINT_DESIRED_OCCUPANCY",
    "APPLY_WINDOW_BOUNDS",
  ]);
  assert.equal(store.getState().transactions["close-side-tools"].phase, "settled");
  assert.equal(store.getSnapshot().contentViewportWidth, 1480);
  assert.equal(store.getSnapshot().presentation.sideTools, "closed");
  assert.equal(setBoundsCount, 2);
  assert.equal(listeners.size, 1);

  runner.stop();
  assert.equal(listeners.size, 0);
});

test("an external snapshot is reduced before a covered checkpoint result", async () => {
  const epoch = "snapshot-before-checkpoint-epoch";
  const store = createHorizontalLayoutFrameStore({
    policy: "expand-v1",
    rendererEpoch: epoch,
    snapshot: snapshot(),
    desiredOccupancy: closed,
  });
  let snapshotListener = null;
  let resolveFirstCommand = null;
  const commands = [];
  const frames = [];
  let nextHandle = 1;
  const runner = createHorizontalLayoutEffectRunner({
    api: {
      executeWindowLayoutCommand(command) {
        commands.push(command);
        if (commands.length === 1) {
          return new Promise((resolve) => {
            resolveFirstCommand = resolve;
          });
        }
        return new Promise(() => {});
      },
      subscribeWindowLayoutSnapshots(listener) {
        snapshotListener = listener;
      },
      unsubscribeWindowLayoutSnapshots(listener) {
        if (snapshotListener === listener) {
          snapshotListener = null;
        }
      },
    },
    store,
    scheduleFrame(callback) {
      const frame = {
        callback,
        handle: nextHandle++,
        cancelled: false,
      };
      frames.push(frame);
      return frame.handle;
    },
    cancelFrame(handle) {
      const frame = frames.find((candidate) => candidate.handle === handle);
      if (frame) {
        frame.cancelled = true;
      }
    },
  });

  runner.dispatch({
    type: "LAYOUT_INTENT_CHANGED",
    previousOccupancy: closed,
    nextOccupancy: sideToolsOpen,
    cause: "user-panel",
    transactionId: "open-during-user-resize",
  });
  assert.equal(commands[0].type, "CHECKPOINT_DESIRED_OCCUPANCY");

  const userSnapshot = { ...snapshot(900, 2), origin: "user" };
  snapshotListener({ snapshot: userSnapshot });
  const initialSession = store.getState().acknowledgedSession;
  resolveFirstCommand({
    accepted: false,
    commandType: "CHECKPOINT_DESIRED_OCCUPANCY",
    reason: "stale",
    acknowledgedSession: {
      ...initialSession,
      normalBaseBounds: { ...userSnapshot.normalBounds },
      windowRevision: 2,
      sessionRevision: initialSession.sessionRevision + 1,
    },
    snapshot: userSnapshot,
  });
  await flushEffects();

  assert.equal(store.getState().snapshot.windowRevision, 2);
  assert.equal(store.getState().responsiveBasisWidth, 900);
  assert.equal(commands.length, 2, "the checkpoint retries from the user revision");
  assert.equal(commands[1].expectedSessionRevision, 1);
  assert.equal(frames.every((frame) => frame.cancelled), true);
  runner.stop();
});

test("snapshot bursts coalesce per frame without merging user and machine origins", () => {
  const epoch = "snapshot-coalescing-epoch";
  const store = createHorizontalLayoutFrameStore({
    policy: "expand-v1",
    rendererEpoch: epoch,
    snapshot: snapshot(),
    desiredOccupancy: closed,
  });
  let snapshotListener = null;
  const frames = [];
  let nextHandle = 1;
  const runner = createHorizontalLayoutEffectRunner({
    api: {
      executeWindowLayoutCommand() {
        throw new Error("snapshot test does not execute commands");
      },
      subscribeWindowLayoutSnapshots(listener) {
        snapshotListener = listener;
      },
      unsubscribeWindowLayoutSnapshots(listener) {
        if (snapshotListener === listener) {
          snapshotListener = null;
        }
      },
    },
    store,
    scheduleFrame(callback) {
      const frame = {
        callback,
        handle: nextHandle++,
        cancelled: false,
        ran: false,
      };
      frames.push(frame);
      return frame.handle;
    },
    cancelFrame(handle) {
      const frame = frames.find((candidate) => candidate.handle === handle);
      if (frame) {
        frame.cancelled = true;
      }
    },
  });
  const activeFrames = () =>
    frames.filter((frame) => !frame.cancelled && !frame.ran);
  const runFrame = (frame) => {
    frame.ran = true;
    frame.callback();
  };

  assert.ok(snapshotListener);
  for (let revision = 2; revision <= 121; revision += 1) {
    snapshotListener({
      snapshot: {
        ...snapshot(1400 + revision, revision),
        origin: "user",
      },
    });
  }
  assert.equal(store.getState().snapshot.windowRevision, 1);
  assert.equal(activeFrames().length, 1);
  runFrame(activeFrames()[0]);
  assert.equal(store.getState().snapshot.windowRevision, 121);
  assert.equal(store.getState().responsiveBasisWidth, 1521);

  snapshotListener({
    snapshot: { ...snapshot(900, 122), origin: "user" },
  });
  snapshotListener({
    snapshot: { ...snapshot(1200, 123), origin: "panel-machine" },
  });
  assert.equal(
    store.getState().snapshot.windowRevision,
    122,
    "origin class change flushes the user resize before queuing machine work",
  );
  assert.equal(store.getState().responsiveBasisWidth, 900);
  assert.equal(activeFrames().length, 1);
  runFrame(activeFrames()[0]);
  assert.equal(store.getState().snapshot.windowRevision, 123);
  assert.equal(
    store.getState().responsiveBasisWidth,
    900,
    "panel-machine resize must not overwrite the user's responsive basis",
  );

  snapshotListener({
    snapshot: { ...snapshot(850, 124), origin: "user" },
  });
  snapshotListener({
    snapshot: {
      ...snapshot(1600, 125),
      mode: "maximized",
      origin: "mode",
    },
  });
  assert.equal(
    store.getState().snapshot.windowRevision,
    124,
    "mode changes flush a pending user resize instead of swallowing its basis",
  );
  assert.equal(store.getState().responsiveBasisWidth, 850);
  assert.equal(activeFrames().length, 1);
  runFrame(activeFrames()[0]);
  assert.equal(store.getState().snapshot.windowRevision, 125);
  assert.equal(store.getState().responsiveBasisWidth, 850);

  runner.stop();
  assert.equal(snapshotListener, null);
});
