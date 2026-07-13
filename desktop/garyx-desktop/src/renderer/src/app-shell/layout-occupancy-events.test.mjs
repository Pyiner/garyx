import assert from "node:assert/strict";
import test from "node:test";

import {
  appendLayoutOccupancyIntent,
  createLayoutOccupancyEventLog,
  projectLayoutOccupancy,
} from "./layout-occupancy-events.ts";

const closed = Object.freeze({
  globalSidebar: true,
  conversationRailKey: null,
  inspectorOpen: false,
  openCapsuleCount: 0,
});

function append(log, patch, cause = "user-panel") {
  return appendLayoutOccupancyIntent(
    log,
    { ...log.currentSources, ...patch },
    cause,
  );
}

test("projects all three desired occupancies from legacy source state", () => {
  for (const globalSidebar of [false, true]) {
    for (const conversationRailKey of [null, "recent"]) {
      for (const inspectorOpen of [false, true]) {
        for (const openCapsuleCount of [0, 2]) {
          assert.deepEqual(
            projectLayoutOccupancy({
              globalSidebar,
              conversationRailKey,
              inspectorOpen,
              openCapsuleCount,
            }),
            {
              globalSidebar,
              conversationRail: conversationRailKey !== null,
              sideTools: inspectorOpen || openCapsuleCount > 0,
            },
          );
        }
      }
    }
  }
});

test("capsule and inspector writers emit only on side-tools union edges", () => {
  let log = createLayoutOccupancyEventLog(closed);

  let result = append(log, { openCapsuleCount: 1 }, "user-route");
  assert.equal(result.event?.transactionId, "layout-intent-1");
  assert.deepEqual(result.event?.nextOccupancy, {
    globalSidebar: true,
    conversationRail: false,
    sideTools: true,
  });
  log = result.log;

  result = append(log, { openCapsuleCount: 2 }, "user-route");
  assert.equal(
    result.event,
    null,
    "a second capsule keeps the same union occupancy",
  );
  log = result.log;

  result = append(log, { inspectorOpen: true }, "user-panel");
  assert.equal(
    result.event,
    null,
    "inspector opening behind capsules is not a panel edge",
  );
  log = result.log;

  result = append(log, { openCapsuleCount: 0 }, "user-route");
  assert.equal(result.event, null, "inspector keeps the union occupied");
  log = result.log;

  result = append(log, { inspectorOpen: false }, "user-panel");
  assert.equal(result.event?.transactionId, "layout-intent-2");
  assert.equal(result.event?.nextOccupancy.sideTools, false);
});

test("rail open, identity switch, and cleanup are full-vector events", () => {
  let log = createLayoutOccupancyEventLog(closed);

  let result = append(log, { conversationRailKey: "recent" }, "user-route");
  assert.equal(result.event?.cause, "user-route");
  assert.equal(result.event?.nextOccupancy.conversationRail, true);
  log = result.log;

  result = append(log, { conversationRailKey: "bot:alpha" }, "user-route");
  assert.ok(result.event, "rail-to-rail switches remain one replace event");
  assert.deepEqual(
    result.event.previousOccupancy,
    result.event.nextOccupancy,
    "the full boolean vector may be equal for an identity replacement",
  );
  log = result.log;

  result = append(log, { conversationRailKey: "bot:alpha" }, "user-route");
  assert.equal(result.event, null, "repeating the same route is a no-op");
  log = result.log;

  result = append(log, { conversationRailKey: null }, "system-cleanup");
  assert.equal(result.event?.cause, "system-cleanup");
  assert.equal(result.event?.nextOccupancy.conversationRail, false);
});

test("transaction ids advance only for emitted events and history stays bounded", () => {
  let log = createLayoutOccupancyEventLog(closed);
  let result = append(log, { openCapsuleCount: 0 }, "hydrate");
  assert.equal(result.event, null);
  assert.equal(result.log.nextTransactionSequence, 1);
  log = result.log;

  for (let index = 0; index < 300; index += 1) {
    result = append(
      log,
      { globalSidebar: !log.currentSources.globalSidebar },
      "user-panel",
    );
    log = result.log;
  }

  assert.equal(log.events.length, 256);
  assert.equal(log.events.at(0)?.transactionId, "layout-intent-45");
  assert.equal(log.events.at(-1)?.transactionId, "layout-intent-300");
  assert.equal(log.nextTransactionSequence, 301);
});

test("rejects invalid source state instead of manufacturing occupancy", () => {
  assert.throws(
    () => projectLayoutOccupancy({ ...closed, openCapsuleCount: -1 }),
    /non-negative integer/,
  );
  assert.throws(
    () => createLayoutOccupancyEventLog({ ...closed, conversationRailKey: "" }),
    /null or non-empty/,
  );
});
