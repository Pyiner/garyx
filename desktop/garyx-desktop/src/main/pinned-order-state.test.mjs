import assert from "node:assert/strict";
import { test } from "node:test";

import { PinnedOrderState } from "./pinned-order-state.ts";

const GATEWAY = "gateway-a";

function makeState(order, revision = 10) {
  return new PinnedOrderState({
    gatewayIdentity: GATEWAY,
    initialOrder: order,
    revision,
  });
}

function page(threadIds, revision) {
  return { threadIds, revision };
}

function outbox(desiredOrder, revision) {
  return {
    gatewayIdentity: GATEWAY,
    desiredOrder,
    lastKnownRevision: revision,
  };
}

function sends(update) {
  return update.effects
    .filter((effect) => effect.kind === "sendReorder")
    .map((effect) => effect.request);
}

function publications(update) {
  return update.effects
    .filter((effect) => effect.kind === "publish")
    .map((effect) => effect.order);
}

function beginDrop(state, order) {
  state.beginDrag();
  state.previewDrag(order);
  const request = sends(state.acceptDrop())[0];
  assert.ok(request, "accepted drop should dispatch one request");
  return request;
}

test("drag preview is mutation-free and cancellation restores the baseline", () => {
  const state = makeState(["a", "b", "c"], 4);
  const epoch = state.epoch;
  state.beginDrag();
  state.previewDrag(["b", "a", "c"]);
  state.previewDrag(["c", "b", "a"]);

  const update = state.cancelDrag();

  assert.deepEqual(state.presentedOrder, ["a", "b", "c"]);
  assert.equal(state.epoch, epoch);
  assert.equal(state.outbox, null);
  assert.deepEqual(sends(update), []);
  assert.equal(
    update.effects.some((effect) => effect.kind === "noteLocalMutation"),
    false,
  );
});

test("an accepted drop commits once and starts one revision-CAS flight", () => {
  const state = makeState(["a", "b"], 4);
  const request = beginDrop(state, ["b", "a"]);

  assert.deepEqual(state.desiredOrder, ["b", "a"]);
  assert.equal(state.epoch, 1);
  assert.deepEqual(request.threadIds, ["b", "a"]);
  assert.equal(request.expectedRevision, 4);
});

test("stale GET issued in the unsettled epoch cannot revert a settled reorder", () => {
  const state = makeState(["a", "b"]);
  const reorder = beginDrop(state, ["b", "a"]);
  const staleGet = state.requestStamp();

  state.completeReorder(reorder, page(["b", "a"], 12));
  const stale = state.receivePage(page(["a", "b"], 11), staleGet);

  assert.equal(stale.acceptance, "discardedBelowFloor");
  assert.deepEqual(state.presentedOrder, ["b", "a"]);
  assert.equal(state.outbox, null);
});

test("revision-descending pin write-backs discard the older page completely", () => {
  const state = makeState(["a"]);
  const pinB = state.beginMembershipChange("b", true).membershipRequest;
  const pinC = state.beginMembershipChange("c", true).membershipRequest;
  assert.ok(pinB && pinC);

  state.completeMembership(pinC, page(["c", "b", "a"], 12));
  const older = state.completeMembership(pinB, page(["b", "a"], 11));

  assert.equal(older.acceptance, "discardedBelowFloor");
  assert.equal(state.highestObservedRevision, 12);
  assert.deepEqual(state.presentedOrder, ["c", "b", "a"]);
});

test("settle advances the epoch so an unsettled-window page only merges", () => {
  const state = makeState(["a", "b"]);
  const reorder = beginDrop(state, ["b", "a"]);
  const unsettledStamp = state.requestStamp();
  state.completeReorder(reorder, page(["b", "a"], 12));

  const oldWindow = state.receivePage(page(["a", "b"], 13), unsettledStamp);
  assert.equal(oldWindow.acceptance, "merged");
  assert.deepEqual(state.presentedOrder, ["b", "a"]);
  assert.ok(state.epoch > unsettledStamp.epoch);

  const current = state.receivePage(page(["a", "b"], 13), state.requestStamp());
  assert.equal(current.acceptance, "authoritative");
  assert.deepEqual(state.presentedOrder, ["a", "b"]);
});

test("below-floor 200 closes its flight and resends once with the fresh floor", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);
  state.receivePage(page(["a", "b"], 12), state.requestStamp());

  const completion = state.completeReorder(first, page(["b", "a"], 11));
  const second = sends(completion)[0];

  assert.equal(completion.acceptance, "discardedBelowFloor");
  assert.ok(second);
  assert.equal(second.expectedRevision, 12);
  assert.deepEqual(second.threadIds, ["b", "a"]);
  state.completeReorder(second, page(["b", "a"], 13));
  assert.equal(state.outbox, null);
});

test("below-floor 409 page is never reused as a CAS token", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);
  state.receivePage(page(["a", "b"], 14), state.requestStamp());

  const conflict = state.completeReorder(first, page(["a", "b"], 11));

  assert.equal(conflict.acceptance, "discardedBelowFloor");
  assert.deepEqual(sends(conflict).map((request) => request.expectedRevision), [14]);
});

test("higher-revision opposite pin wins when the local ack is below floor", () => {
  const state = makeState(["a"]);
  const pin = state.beginMembershipChange("b", true).membershipRequest;
  assert.ok(pin);
  state.receivePage(page(["a"], 12), state.requestStamp());
  assert.deepEqual(state.presentedOrder, ["b", "a"]);

  const lowAck = state.completeMembership(pin, page(["b", "a"], 11));

  assert.equal(lowAck.acceptance, "discardedBelowFloor");
  assert.deepEqual(state.presentedOrder, ["a"]);
  assert.equal(state.liveMembershipIntentCount, 0);
});

test("gateway switch resets the revision domain and drops a late old response", () => {
  const state = makeState(["a"], 100);
  const oldStamp = state.requestStamp();
  state.switchGateway("gateway-b");

  const fresh = state.receivePage(
    page(["new"], 0),
    { gatewayIdentity: "gateway-b", epoch: 0 },
  );
  const late = state.receivePage(page(["old"], 101), oldStamp);

  assert.equal(state.highestObservedRevision, 0);
  assert.equal(fresh.acceptance, "authoritative");
  assert.equal(late.identityAccepted, false);
  assert.deepEqual(state.presentedOrder, ["new"]);
});

test("ack settle publishes no row-order delta", () => {
  const state = makeState(["a", "b"]);
  const request = beginDrop(state, ["b", "a"]);

  const ack = state.completeReorder(request, page(["b", "a"], 11));

  assert.equal(state.outbox, null);
  assert.deepEqual(publications(ack), []);
  assert.deepEqual(sends(ack), []);
});

test("conflict merges a remote pin, resends the full order, and settles", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);

  const conflict = state.completeReorder(first, page(["c", "a", "b"], 11));
  const second = sends(conflict)[0];

  assert.ok(second);
  assert.deepEqual(second.threadIds, ["c", "b", "a"]);
  assert.equal(second.expectedRevision, 11);
  state.completeReorder(second, page(["c", "b", "a"], 12));
  assert.equal(state.outbox, null);
});

test("conflict page already equal to desired settles without another PUT", () => {
  const state = makeState(["a", "b"]);
  const request = beginDrop(state, ["b", "a"]);

  const conflict = state.completeReorder(request, page(["b", "a"], 11));

  assert.equal(state.outbox, null);
  assert.deepEqual(sends(conflict), []);
});

test("poll during a flight merges membership and coalesces behind the flight", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);

  const poll = state.receivePage(page(["c", "a", "b"], 11), state.requestStamp());

  assert.equal(poll.acceptance, "merged");
  assert.deepEqual(state.presentedOrder, ["c", "b", "a"]);
  assert.deepEqual(sends(poll), []);
  assert.equal(state.pendingSync.kind, "coalescedBehindFlight");

  const completion = state.completeReorder(first, page(["a", "b"], 10));
  assert.deepEqual(sends(completion)[0]?.threadIds, ["c", "b", "a"]);
  assert.equal(sends(completion)[0]?.expectedRevision, 11);
});

test("round-5 delayed unpin gates resend until the membership response", () => {
  const state = makeState(["a", "b", "c"]);
  const first = beginDrop(state, ["c", "b", "a"]);
  const unpin = state.beginMembershipChange("a", false).membershipRequest;
  assert.ok(unpin);

  const conflict = state.completeReorder(first, page(["a", "b", "c"], 11));
  assert.deepEqual(sends(conflict), []);
  assert.equal(state.pendingSync.kind, "waitingForMembership");

  const membership = state.completeMembership(unpin, page(["b", "c"], 12));
  assert.equal(sends(membership).length, 1);
  assert.deepEqual(sends(membership)[0].threadIds, ["c", "b"]);
  assert.equal(sends(membership)[0].expectedRevision, 12);
});

test("round-5 delayed pin never dispatches an unknown id before pin completes", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);
  const pin = state.beginMembershipChange("c", true).membershipRequest;
  assert.ok(pin);

  const oldFlight = state.completeReorder(first, page(["a", "b"], 11));
  assert.deepEqual(sends(oldFlight), []);
  assert.equal(state.pendingSync.kind, "waitingForMembership");

  const membership = state.completeMembership(pin, page(["c", "a", "b"], 12));
  assert.deepEqual(sends(membership)[0]?.threadIds, ["c", "b", "a"]);
});

test("round-5 membership failure rollback wakes the gate exactly once", () => {
  const state = makeState(["a", "b", "c"]);
  const first = beginDrop(state, ["c", "b", "a"]);
  const unpin = state.beginMembershipChange("a", false).membershipRequest;
  assert.ok(unpin);
  state.completeReorder(first, page(["a", "b", "c"], 11));

  const rollback = state.failMembership(unpin);

  assert.deepEqual(state.presentedOrder, ["c", "b", "a"]);
  assert.equal(sends(rollback).length, 1);
  assert.deepEqual(sends(rollback)[0].threadIds, ["c", "b", "a"]);
});

test("round-5 full unpin clears the outbox and never sends an empty PUT", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);
  const unpinA = state.beginMembershipChange("a", false).membershipRequest;
  const unpinB = state.beginMembershipChange("b", false).membershipRequest;
  assert.ok(unpinA && unpinB);
  state.completeReorder(first, page(["a", "b"], 11));

  const firstMembership = state.completeMembership(unpinA, page(["b"], 12));
  const finalMembership = state.completeMembership(unpinB, page([], 13));

  assert.deepEqual(sends(firstMembership), []);
  assert.deepEqual(sends(finalMembership), []);
  assert.equal(state.outbox, null);
  assert.deepEqual(state.presentedOrder, []);
});

test("round-5 restart settles if the server already equals desired or is empty", () => {
  const equal = new PinnedOrderState({
    gatewayIdentity: GATEWAY,
    restoredOutbox: outbox(["b", "a"], 7),
  });
  const equalUpdate = equal.receivePage(page(["b", "a"], 8), equal.requestStamp());
  assert.equal(equal.outbox, null);
  assert.deepEqual(sends(equalUpdate), []);

  const empty = new PinnedOrderState({
    gatewayIdentity: GATEWAY,
    restoredOutbox: outbox([], 7),
  });
  const emptyUpdate = empty.receivePage(page(["a"], 8), empty.requestStamp());
  assert.equal(empty.outbox, null);
  assert.deepEqual(sends(emptyUpdate), []);
  assert.deepEqual(empty.presentedOrder, ["a"]);
});

test("round-6 projected-empty conflict survives live unpins and both rollbacks", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);
  const unpinA = state.beginMembershipChange("a", false).membershipRequest;
  const unpinB = state.beginMembershipChange("b", false).membershipRequest;
  assert.ok(unpinA && unpinB);

  const conflict = state.completeReorder(first, page(["a", "b"], 11));
  assert.ok(state.outbox);
  assert.deepEqual(state.desiredOrder, []);
  assert.equal(state.pendingSync.kind, "waitingForMembership");
  assert.deepEqual(sends(conflict), []);

  const firstRollback = state.failMembership(unpinA);
  const finalRollback = state.failMembership(unpinB);
  assert.deepEqual(sends(firstRollback), []);
  assert.equal(sends(finalRollback).length, 1);
  const recovery = sends(finalRollback)[0];
  assert.deepEqual(recovery.threadIds, ["b", "a"]);

  state.completeReorder(recovery, page(["b", "a"], 12));
  state.receivePage(page(["b", "a"], 12), state.requestStamp());
  assert.equal(state.outbox, null);
  assert.deepEqual(state.presentedOrder, ["b", "a"]);
});

test("round-6 wake drains once after membership acceptance raises the floor", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);
  const pin = state.beginMembershipChange("c", true).membershipRequest;
  assert.ok(pin);
  state.completeReorder(first, page(["a", "b"], 11));

  const completion = state.completeMembership(pin, page(["c", "a", "b"], 12));

  assert.equal(sends(completion).length, 1);
  assert.equal(sends(completion)[0].expectedRevision, 12);
  assert.deepEqual(sends(completion)[0].threadIds, ["c", "b", "a"]);
});

test("round-7 membership response before old flight coalesces then follows up once", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);
  const pin = state.beginMembershipChange("c", true).membershipRequest;
  assert.ok(pin);

  const membership = state.completeMembership(pin, page(["c", "a", "b"], 12));
  assert.deepEqual(sends(membership), []);
  assert.equal(state.pendingSync.kind, "coalescedBehindFlight");

  const oldFlight = state.completeReorder(first, page(["b", "a"], 11));
  assert.equal(sends(oldFlight).length, 1);
  assert.equal(sends(oldFlight)[0].expectedRevision, 12);
  assert.deepEqual(sends(oldFlight)[0].threadIds, ["c", "b", "a"]);
});

test("round-7 late flight completion cannot revive an outbox settled while airborne", () => {
  const state = makeState(["a", "b"]);
  const first = beginDrop(state, ["b", "a"]);
  const pin = state.beginMembershipChange("c", true).membershipRequest;
  assert.ok(pin);

  const membership = state.completeMembership(pin, page(["c", "b", "a"], 12));
  assert.equal(state.outbox, null);
  assert.deepEqual(sends(membership), []);

  const late = state.completeReorder(first, page(["b", "a"], 11));
  assert.equal(state.outbox, null);
  assert.deepEqual(sends(late), []);
});

test("durable restart recovery survives and a newer drop supersedes the outbox", () => {
  const state = new PinnedOrderState({
    gatewayIdentity: GATEWAY,
    restoredOutbox: outbox(["b", "a"], 10),
  });
  const recovery = state.receivePage(page(["a", "b"], 11), state.requestStamp());
  const oldFlight = sends(recovery)[0];
  assert.ok(oldFlight);

  state.beginDrag();
  state.previewDrag(["a", "b"]);
  state.acceptDrop();
  assert.deepEqual(state.outbox?.desiredOrder, ["a", "b"]);

  const late = state.completeReorder(oldFlight, page(["b", "a"], 12));
  assert.deepEqual(sends(late)[0]?.threadIds, ["a", "b"]);
});

test("retryable failure backs off while permanent failure pauses non-blockingly", () => {
  const retryable = makeState(["a", "b"]);
  const first = beginDrop(retryable, ["b", "a"]);
  retryable.failReorder(first, { kind: "retryable", delay: 5 }, 10);
  assert.deepEqual(retryable.pendingSync, {
    kind: "retryScheduled",
    attempt: 1,
    notBefore: 15,
  });
  assert.deepEqual(sends(retryable.retryTick(14.9)), []);
  assert.equal(sends(retryable.retryTick(15)).length, 1);

  const permanent = makeState(["a", "b"]);
  const unsupported = beginDrop(permanent, ["b", "a"]);
  permanent.failReorder(unsupported, { kind: "permanent", statusCode: 405 });
  assert.deepEqual(permanent.pendingSync, {
    kind: "pausedPermanent",
    statusCode: 405,
  });
  assert.equal(permanent.hasPendingSync, true);
  assert.deepEqual(sends(permanent.retryTick(100)), []);
  assert.equal(sends(permanent.resumePausedSync()).length, 1);
});

test("same-gateway reload restores the durable outbox without stale transport tokens", () => {
  const state = makeState(["a", "b"]);
  beginDrop(state, ["b", "a"]);
  const persisted = state.outbox;
  assert.ok(persisted);

  state.reloadCurrentGateway({ restoredOutbox: persisted });

  assert.equal(state.gatewayIdentity, GATEWAY);
  assert.deepEqual(state.desiredOrder, ["b", "a"]);
  assert.equal(state.pendingSync.kind, "ready");
  assert.equal(state.activeReorderFlight, null);
});
