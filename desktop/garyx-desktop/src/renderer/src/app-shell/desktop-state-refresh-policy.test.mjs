import assert from "node:assert/strict";
import test from "node:test";

import { desktopStateRefreshDecision } from "./desktop-state-refresh-policy.ts";

test("only a false-to-true connection transition refreshes immediately", () => {
  assert.equal(
    desktopStateRefreshDecision({
      kind: "connection",
      previousOk: false,
      nextOk: true,
    }).desktopRefresh,
    "immediate",
  );

  for (const [previousOk, nextOk] of [
    [null, true],
    [true, true],
    [true, false],
    [false, false],
  ]) {
    assert.equal(
      desktopStateRefreshDecision({
        kind: "connection",
        previousOk,
        nextOk,
      }).desktopRefresh,
      "none",
    );
  }
});

test("periodic refreshes require a visible document", () => {
  assert.deepEqual(
    desktopStateRefreshDecision({ kind: "periodic", hidden: false }),
    {
      desktopRefresh: "debounced",
      refreshSelectedThreadHistory: false,
      requiresVisible: true,
    },
  );
  assert.equal(
    desktopStateRefreshDecision({ kind: "periodic", hidden: true })
      .desktopRefresh,
    "none",
  );
});

test("visibility recovery refreshes root and selected-thread history once", () => {
  assert.deepEqual(
    desktopStateRefreshDecision({
      kind: "visibility",
      hidden: false,
      hasSelectedThread: true,
    }),
    {
      desktopRefresh: "debounced",
      refreshSelectedThreadHistory: true,
      requiresVisible: true,
    },
  );
  assert.equal(
    desktopStateRefreshDecision({
      kind: "visibility",
      hidden: true,
      hasSelectedThread: true,
    }).desktopRefresh,
    "none",
  );
});

test("mutation invalidations stay enabled while hidden", () => {
  assert.deepEqual(desktopStateRefreshDecision({ kind: "mutation" }), {
    desktopRefresh: "debounced",
    refreshSelectedThreadHistory: false,
    requiresVisible: false,
  });
});
