import assert from "node:assert/strict";
import { test } from "node:test";

import { PinnedOrderController } from "./pinned-order-controller.ts";
import { PinnedOrderState } from "./pinned-order-state.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
}

async function waitFor(predicate, message) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (predicate()) {
      return;
    }
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.fail(message);
}

function harness(initialOrder = ["a", "b"], revision = 10) {
  const requests = [];
  const flights = [];
  const persisted = [];
  const controller = new PinnedOrderController(
    new PinnedOrderState({
      gatewayIdentity: "gateway-a",
      initialOrder,
      revision,
    }),
    {
      now: () => 100,
      persist: async (outbox) => {
        persisted.push(outbox ? structuredClone(outbox) : null);
      },
      sendReorder: (request) => {
        const flight = deferred();
        requests.push(structuredClone(request));
        flights.push(flight);
        return flight.promise;
      },
      classifyFailure: (error) => {
        if (error?.retryable) {
          return { kind: "retryable", delay: 5 };
        }
        return { kind: "permanent", statusCode: error?.status ?? null };
      },
    },
  );
  return { controller, flights, persisted, requests };
}

async function settleFlight(h, index, threadIds, revision) {
  h.flights[index].resolve({ threadIds, revision });
  await h.controller.waitForTransportIdle();
}

test("controller persists before dispatch and keeps two quick drops single-flight", async () => {
  const h = harness(["a", "b", "c"]);

  await h.controller.commitOrder(["c", "b", "a"]);
  await h.controller.commitOrder(["b", "c", "a"]);

  assert.equal(h.requests.length, 1);
  assert.deepEqual(h.persisted[0]?.desiredOrder, ["c", "b", "a"]);
  assert.deepEqual(h.persisted.at(-1)?.desiredOrder, ["b", "c", "a"]);

  h.flights[0].resolve({ threadIds: ["c", "b", "a"], revision: 11 });
  await waitFor(() => h.requests.length === 2, "follow-up PUT should start");
  assert.equal(h.requests.length, 2);
  assert.deepEqual(h.requests[1].threadIds, ["b", "c", "a"]);
  h.flights[1].resolve({ threadIds: ["b", "c", "a"], revision: 12 });
  await h.controller.waitForTransportIdle();
  assert.equal(h.controller.state.outbox, null);
});

test("round-5 delayed unpin bounds PUTs and drains once with reduced order", async () => {
  const h = harness(["a", "b", "c"]);
  await h.controller.commitOrder(["c", "b", "a"]);
  const unpin = await h.controller.beginMembershipChange("a", false);
  assert.ok(unpin);

  await settleFlight(h, 0, ["a", "b", "c"], 11);
  assert.equal(h.requests.length, 1);
  assert.equal(h.controller.state.pendingSync.kind, "waitingForMembership");

  await h.controller.completeMembership(unpin, { threadIds: ["b", "c"], revision: 12 });
  assert.equal(h.requests.length, 2);
  assert.deepEqual(h.requests[1].threadIds, ["c", "b"]);
  assert.equal(h.requests[1].expectedRevision, 12);
  await settleFlight(h, 1, ["c", "b"], 13);
  assert.equal(h.controller.state.outbox, null);
  assert.equal(h.requests.some((request) => request.threadIds.length === 0), false);
});

test("round-5 delayed pin does not dispatch its unknown id before membership ack", async () => {
  const h = harness();
  await h.controller.commitOrder(["b", "a"]);
  const pin = await h.controller.beginMembershipChange("c", true);
  assert.ok(pin);

  await settleFlight(h, 0, ["a", "b"], 11);
  assert.equal(h.requests.length, 1);

  await h.controller.completeMembership(pin, {
    threadIds: ["c", "a", "b"],
    revision: 12,
  });
  assert.equal(h.requests.length, 2);
  assert.deepEqual(h.requests[1].threadIds, ["c", "b", "a"]);
  await settleFlight(h, 1, ["c", "b", "a"], 13);
  assert.equal(h.controller.state.outbox, null);
});

test("round-5 full unpin clears the outbox without an empty request", async () => {
  const h = harness();
  await h.controller.commitOrder(["b", "a"]);
  const unpinA = await h.controller.beginMembershipChange("a", false);
  const unpinB = await h.controller.beginMembershipChange("b", false);
  assert.ok(unpinA && unpinB);

  await settleFlight(h, 0, ["a", "b"], 11);
  await h.controller.completeMembership(unpinA, { threadIds: ["b"], revision: 12 });
  await h.controller.completeMembership(unpinB, { threadIds: [], revision: 13 });

  assert.equal(h.requests.length, 1);
  assert.equal(h.requests.some((request) => request.threadIds.length === 0), false);
  assert.equal(h.controller.state.outbox, null);
});

test("round-6 409 plus projected-empty unpins survives rollback and sends once", async () => {
  const h = harness();
  await h.controller.commitOrder(["b", "a"]);
  const unpinA = await h.controller.beginMembershipChange("a", false);
  const unpinB = await h.controller.beginMembershipChange("b", false);
  assert.ok(unpinA && unpinB);

  await settleFlight(h, 0, ["a", "b"], 11);
  assert.equal(h.requests.length, 1);
  assert.ok(h.controller.state.outbox);
  assert.equal(h.controller.state.pendingSync.kind, "waitingForMembership");

  await h.controller.failMembership(unpinA);
  assert.equal(h.requests.length, 1);
  await h.controller.failMembership(unpinB);
  assert.equal(h.requests.length, 2);
  assert.deepEqual(h.requests[1].threadIds, ["b", "a"]);
  await settleFlight(h, 1, ["b", "a"], 12);
  await h.controller.receivePage(
    { threadIds: ["b", "a"], revision: 12 },
    h.controller.requestStamp(),
  );
  assert.equal(h.controller.state.outbox, null);
  assert.deepEqual(h.controller.state.presentedOrder, ["b", "a"]);
});

test("round-6 membership response raises floor before exactly one drain", async () => {
  const h = harness();
  await h.controller.commitOrder(["b", "a"]);
  const pin = await h.controller.beginMembershipChange("c", true);
  assert.ok(pin);
  await settleFlight(h, 0, ["a", "b"], 11);

  await h.controller.completeMembership(pin, {
    threadIds: ["c", "a", "b"],
    revision: 12,
  });

  assert.equal(h.requests.length, 2);
  assert.equal(h.requests[1].expectedRevision, 12);
  await settleFlight(h, 1, ["c", "b", "a"], 13);
});

test("round-7 membership completion before old flight never starts a concurrent PUT", async () => {
  const h = harness();
  await h.controller.commitOrder(["b", "a"]);
  const pin = await h.controller.beginMembershipChange("c", true);
  assert.ok(pin);

  await h.controller.completeMembership(pin, {
    threadIds: ["c", "a", "b"],
    revision: 12,
  });
  assert.equal(h.requests.length, 1);
  assert.equal(h.controller.state.pendingSync.kind, "coalescedBehindFlight");

  h.flights[0].resolve({ threadIds: ["b", "a"], revision: 11 });
  await waitFor(() => h.requests.length === 2, "fresh-floor follow-up should start");
  assert.equal(h.requests.length, 2);
  assert.equal(h.requests[1].expectedRevision, 12);
  assert.deepEqual(h.requests[1].threadIds, ["c", "b", "a"]);
  h.flights[1].resolve({ threadIds: ["c", "b", "a"], revision: 13 });
  await h.controller.waitForTransportIdle();
});

test("round-7 a late old flight cannot revive an outbox settled by membership", async () => {
  const h = harness();
  await h.controller.commitOrder(["b", "a"]);
  const pin = await h.controller.beginMembershipChange("c", true);
  assert.ok(pin);

  await h.controller.completeMembership(pin, {
    threadIds: ["c", "b", "a"],
    revision: 12,
  });
  assert.equal(h.controller.state.outbox, null);
  assert.equal(h.requests.length, 1);

  await settleFlight(h, 0, ["b", "a"], 11);
  assert.equal(h.requests.length, 1);
  assert.equal(h.controller.state.outbox, null);
});

test("controller exposes non-blocking permanent pause and resumes explicitly", async () => {
  const h = harness();
  await h.controller.commitOrder(["b", "a"]);
  h.flights[0].reject({ status: 405 });
  await h.controller.waitForTransportIdle();

  assert.equal(h.controller.snapshot().syncState, "paused_permanent");
  assert.equal(h.controller.snapshot().unsettled, true);
  await h.controller.resumePausedSync();
  assert.equal(h.requests.length, 2);
  await settleFlight(h, 1, ["b", "a"], 11);
  assert.equal(h.controller.snapshot().unsettled, false);
});
