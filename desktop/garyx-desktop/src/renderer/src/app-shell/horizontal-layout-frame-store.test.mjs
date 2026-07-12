import assert from "node:assert/strict";
import test from "node:test";

import {
  HORIZONTAL_LAYOUT_FRAME_ATTRIBUTES,
  HORIZONTAL_LAYOUT_FRAME_VARIABLES,
  clearFrame,
  createLegacyHorizontalLayoutFrameStore,
} from "./horizontal-layout-frame-store.ts";

function snapshot(width = 1480, revision = 1) {
  const bounds = { x: 0, y: 0, width, height: 940 };
  return {
    windowRevision: revision,
    bounds,
    contentBounds: bounds,
    normalBounds: bounds,
    workArea: { x: 0, y: 0, width: 3200, height: 1400 },
    mode: "normal",
    displayId: "synthetic-display",
    scaleFactor: 2,
    origin: "hydrate",
  };
}

function mockRoot() {
  const variables = new Map();
  const attributes = new Map();
  const operations = [];
  return {
    attributes,
    operations,
    variables,
    root: {
      style: {
        setProperty(name, value) {
          variables.set(name, value);
          operations.push(["variable", name, value]);
        },
        removeProperty(name) {
          variables.delete(name);
          operations.push(["remove-variable", name]);
        },
      },
      setAttribute(name, value) {
        attributes.set(name, value);
        operations.push(["attribute", name, value]);
      },
      removeAttribute(name) {
        attributes.delete(name);
        operations.push(["remove-attribute", name]);
      },
    },
  };
}

function baselineStore() {
  return createLegacyHorizontalLayoutFrameStore({
    rendererEpoch: "phase-2-test",
    snapshot: snapshot(),
    desiredOccupancy: {
      globalSidebar: true,
      conversationRail: false,
      sideTools: false,
      threadLogs: false,
    },
  });
}

test("applyFrame publishes every px variable and presentation attribute under one revision", () => {
  const store = baselineStore();
  const target = mockRoot();
  store.attachRoot(target.root);

  assert.deepEqual(
    [...target.variables.keys()].sort(),
    [...HORIZONTAL_LAYOUT_FRAME_VARIABLES].sort(),
  );
  assert.deepEqual(
    [...target.attributes.keys()].sort(),
    [...HORIZONTAL_LAYOUT_FRAME_ATTRIBUTES].sort(),
  );
  assert.equal(target.variables.get("--gx-sidebar-width"), "245px");
  assert.equal(target.variables.get("--app-sidebar-width"), "245px");
  assert.equal(target.variables.get("--spacing-token-sidebar"), "245px");
  assert.equal(target.variables.get("--spacing-token-rail"), "0px");
  assert.equal(target.attributes.get("data-layout-policy"), "legacy");
  assert.equal(target.attributes.get("data-layout-revision"), "0");
  assert.deepEqual(target.operations.at(-1), [
    "attribute",
    "data-layout-revision",
    "0",
  ]);

  const firstAttribute = target.operations.findIndex(
    ([kind]) => kind === "attribute",
  );
  assert.equal(
    target.operations
      .slice(0, firstAttribute)
      .every(([kind]) => kind === "variable"),
    true,
  );
});

test("legacy store reduces normalized occupancy and width events without wiring bounds effects", () => {
  const store = baselineStore();
  const target = mockRoot();
  store.attachRoot(target.root);
  target.operations.length = 0;
  let notifications = 0;
  const unsubscribe = store.subscribe(() => {
    notifications += 1;
  });

  const effects = store.dispatch({
    type: "LAYOUT_INTENT_CHANGED",
    previousOccupancy: {
      globalSidebar: true,
      conversationRail: false,
      sideTools: false,
      threadLogs: false,
    },
    nextOccupancy: {
      globalSidebar: true,
      conversationRail: true,
      sideTools: false,
      threadLogs: false,
    },
    cause: "user-route",
    transactionId: "phase-2-open-rail",
  });

  assert.deepEqual(effects.map((effect) => effect.type), [
    "window-layout-session",
  ]);
  assert.equal(
    effects.some((effect) => effect.type === "window-bounds"),
    false,
  );
  assert.equal(store.getSnapshot().policy, "legacy");
  assert.equal(store.getSnapshot().presentation.conversationRail, "open");
  assert.deepEqual(store.getState().transactions, {});
  assert.equal(store.getState().headTransactionId, null);
  assert.equal(
    store.getState().acknowledgedSession.desiredOccupancy.conversationRail,
    true,
  );
  assert.equal(target.variables.get("--spacing-token-rail"), "258px");
  assert.equal(
    target.attributes.get("data-conversation-rail-state"),
    "open",
  );
  assert.equal(
    target.attributes.get("data-layout-revision"),
    String(store.getSnapshot().revision),
  );
  assert.deepEqual(target.operations.at(-1), [
    "attribute",
    "data-layout-revision",
    String(store.getSnapshot().revision),
  ]);

  store.dispatch({
    type: "PANEL_WIDTH_CHANGED",
    panel: "conversationRail",
    width: 333,
    commit: true,
  });
  assert.equal(target.variables.get("--spacing-token-rail"), "333px");
  assert.equal(notifications, 2);
  unsubscribe();
});

test("detaching or clearing a frame removes only the owned variables and attributes", () => {
  const store = baselineStore();
  const target = mockRoot();
  store.attachRoot(target.root);
  target.attributes.set("data-feature-owned", "keep");
  target.variables.set("--feature-owned", "keep");

  clearFrame(target.root);
  assert.deepEqual([...target.variables.entries()], [["--feature-owned", "keep"]]);
  assert.deepEqual([...target.attributes.entries()], [
    ["data-feature-owned", "keep"],
  ]);
});
