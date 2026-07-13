import assert from "node:assert/strict";
import test from "node:test";

import { WindowLayoutExecutor } from "../../../main/window-layout-executor.ts";

import { createHorizontalLayoutEffectRunner } from "./horizontal-layout-effect-runner.ts";
import { createHorizontalLayoutFrameStore } from "./horizontal-layout-frame-store.ts";

const closed = Object.freeze({
  globalSidebar: false,
  conversationRail: false,
  sideTools: false,
  threadLogs: false,
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
      frames.push({ callback, handle });
      return handle;
    },
    cancelFrame() {},
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
  assert.equal(store.getSnapshot().contentViewportWidth, 1810);
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
    "closing-animation",
  );
  assert.equal(store.getSnapshot().presentation.sideTools, "docked");
  const beforeDeadlineRevision = store.getSnapshot().revision;

  timers.find((timer) => timer.delay === 420).callback();
  assert.equal(frames.length, 1);
  assert.equal(store.getSnapshot().presentation.sideTools, "closed");
  assert.ok(store.getSnapshot().revision > beforeDeadlineRevision);
  frames[0].callback();
  await flushEffects();

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
