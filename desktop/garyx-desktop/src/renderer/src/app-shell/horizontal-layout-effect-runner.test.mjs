import assert from "node:assert/strict";
import test from "node:test";

import { createHorizontalLayoutEffectRunner } from "./horizontal-layout-effect-runner.ts";
import { createHorizontalLayoutFrameStore } from "./horizontal-layout-frame-store.ts";
import {
  createLayoutProtocolExecutorState,
  executeLayoutProtocolCommand,
} from "./window-layout-protocol.ts";

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

function session() {
  return {
    normalBaseBounds: snapshot().normalBounds,
    fundingByPanel: {},
    desiredOccupancy: closed,
    windowRevision: 1,
    sessionRevision: 1,
  };
}

async function flushEffects() {
  for (let index = 0; index < 8; index += 1) {
    await Promise.resolve();
  }
}

test("live effect runner closes open/checkpoint/bounds and close/frame/repay loops", async () => {
  const epoch = "effect-runner-epoch";
  let protocol = createLayoutProtocolExecutorState({
    activeRendererEpoch: epoch,
    snapshot: snapshot(),
    acknowledgedSession: session(),
    freshSession: false,
  });
  const commands = [];
  const listeners = new Set();
  const api = {
    executeWindowLayoutCommand(command) {
      commands.push(command);
      const execution = executeLayoutProtocolCommand(protocol, command);
      protocol = execution.state;
      return Promise.resolve(execution.result);
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
    snapshot: snapshot(),
    desiredOccupancy: closed,
    acknowledgedSession: session(),
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
  assert.equal(listeners.size, 1);

  runner.stop();
  assert.equal(listeners.size, 0);
});
