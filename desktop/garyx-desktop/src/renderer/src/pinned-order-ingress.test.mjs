import assert from "node:assert/strict";
import { test } from "node:test";

import {
  deriveStampedDesktopState,
  PinnedOrderIngress,
} from "./pinned-order-ingress.ts";

function state(order, revision, gateway = "https://gateway.example.test", label = "state") {
  return {
    settings: {
      gatewayUrl: gateway,
      gatewayAuthToken: "",
      gatewayHeaders: "",
      accountId: "test-account",
      fromId: "test-user",
      timeoutSeconds: 60,
      languagePreference: "en",
      followUpBehavior: "queue",
    },
    gatewayProfiles: [],
    entitiesGatewayUrl: gateway,
    workspaces: [],
    selectedWorkspacePath: null,
    pinnedThreadIds: order,
    pinsRevision: revision,
    threads: [],
    sessions: [],
    endpoints: [],
    configuredBots: [],
    botConsoles: [],
    automations: [],
    selectedAutomationId: null,
    lastSeenRunAtByAutomation: {},
    botMainThreads: {},
    remoteErrors: [{ source: "threads", label, message: label }],
  };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((nextResolve, nextReject) => {
    resolve = nextResolve;
    reject = nextReject;
  });
  return { promise, resolve, reject };
}

test("request envelope captures epoch before await, never when the response lands", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  const initial = state(["a", "b"], 7);
  ingress.initializeFromState(initial);
  const response = deferred();

  const pending = ingress.requestState(() => response.promise);
  ingress.commitLocalOrder(["b", "a"]);
  response.resolve(state(["a", "b"], 7, undefined, "late"));
  const resolved = await pending;

  const envelope = ingress.deliveryEnvelope(resolved);
  assert.ok(envelope);
  assert.equal(envelope.capturedEpoch, 0);
  assert.equal(ingress.currentEpoch, 1);
});

test("nested DesktopState results use the same pre-await envelope stamp", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  ingress.initializeFromState(state(["a", "b"], 7));
  const response = deferred();

  const pending = ingress.requestStateResult(
    () => response.promise,
    (result) => result.state,
  );
  ingress.commitLocalOrder(["b", "a"]);
  response.resolve({ state: state(["a", "b"], 7), value: "kept" });
  const result = await pending;

  assert.equal(result.value, "kept");
  assert.equal(ingress.deliveryEnvelope(result.state)?.capturedEpoch, 0);
});

test("V2-3 resolved snapshot queued before drop rebases at React commit", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  const current = state(["a", "b"], 7);
  ingress.initializeFromState(current);
  const queued = await ingress.requestState(
    async () => state(["a", "b"], 7, undefined, "queued-before-drop"),
  );

  ingress.commitLocalOrder(["b", "a"]);
  const optimistic = ingress.commitState(current, {
    ...current,
    pinnedThreadIds: ["b", "a"],
  });
  const committed = ingress.commitState(optimistic, queued);

  assert.deepEqual(committed.pinnedThreadIds, ["b", "a"]);
  assert.equal(committed.remoteErrors[0].label, "queued-before-drop");
});

test("round-3 F2 stale transition stays rejected after ack retires overlay", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  const current = state(["a", "b"], 10);
  ingress.initializeFromState(current);
  const stale = await ingress.requestState(
    async () => state(["a", "b"], 10, undefined, "stale-transition"),
  );

  ingress.commitLocalOrder(["b", "a"]);
  let committed = ingress.commitState(current, {
    ...current,
    pinnedThreadIds: ["b", "a"],
  });
  const ack = await ingress.requestState(
    async () => state(["b", "a"], 11, undefined, "ack"),
  );
  committed = ingress.commitState(committed, ack);
  assert.equal(ingress.desiredOrder, null);
  assert.equal(ingress.currentEpoch, 2, "drop and settle each advance epoch");

  committed = ingress.commitState(committed, stale);
  assert.deepEqual(committed.pinnedThreadIds, ["b", "a"]);
  assert.equal(committed.remoteErrors[0].label, "stale-transition");
});

test("round-4 V4-1 request issued pre-drop is rejected after drop and settle", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  const current = state(["a", "b"], 10);
  ingress.initializeFromState(current);
  const lateResponse = deferred();
  const lateRequest = ingress.requestState(() => lateResponse.promise);

  ingress.commitLocalOrder(["b", "a"]);
  let committed = ingress.commitState(current, {
    ...current,
    pinnedThreadIds: ["b", "a"],
  });
  const ack = await ingress.requestState(
    async () => state(["b", "a"], 11, undefined, "ack"),
  );
  committed = ingress.commitState(committed, ack);
  assert.equal(ingress.currentEpoch, 2);

  lateResponse.resolve(state(["a", "b"], 12, undefined, "landed-last"));
  const late = await lateRequest;
  assert.equal(ingress.deliveryEnvelope(late)?.capturedEpoch, 0);
  committed = ingress.commitState(committed, late);

  assert.deepEqual(committed.pinnedThreadIds, ["b", "a"]);
});

test("bookkeeping advances at delivery, never inside the commit decision", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  const initial = state(["a"], 10);
  ingress.initializeFromState(initial);

  const delivered = await ingress.requestState(
    async () => state(["b", "a"], 11, undefined, "next"),
  );
  // Delivery already observed the revision — even if React never commits
  // the updater (abandoned render), the floor holds.
  assert.equal(ingress.highestObservedRevision, 11);

  // The commit decision is PURE: replaying it (StrictMode double-invoke,
  // interrupted renders) changes nothing and returns the same value.
  const once = ingress.commitState(initial, delivered);
  const floorAfterOnce = ingress.highestObservedRevision;
  const epochAfterOnce = ingress.currentEpoch;
  const twice = ingress.commitState(initial, delivered);
  assert.equal(twice, once, "replayed commit returns the same state");
  assert.equal(ingress.highestObservedRevision, floorAfterOnce);
  assert.equal(ingress.currentEpoch, epochAfterOnce);
  assert.deepEqual(once.pinnedThreadIds, ["b", "a"]);
});

test("an unstamped direct object commit is rejected (spread strips identity)", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  const initial = state(["a"], 10);
  ingress.initializeFromState(initial);
  const delivered = await ingress.requestState(
    async () => state(["b", "a"], 11, undefined, "delivered"),
  );
  const committed = ingress.commitState(initial, delivered);
  assert.deepEqual(committed.pinnedThreadIds, ["b", "a"]);

  // A rebuilt copy of a delivery has no envelope: it is indistinguishable
  // from a stale generation's answer, so a DIRECT commit keeps current...
  const rebuilt = { ...delivered, threads: [] };
  assert.equal(ingress.commitState(committed, rebuilt), committed);

  // ...while an explicitly derived state keeps the delivery's identity.
  const derived = deriveStampedDesktopState(delivered, {
    ...delivered,
    threads: [],
  });
  const acceptedDerived = ingress.commitState(committed, derived);
  assert.notEqual(acceptedDerived, committed);
  assert.deepEqual(acceptedDerived.threads, []);
});

test("a delivery cannot resurrect across A->B->A generations", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  const initial = state(["a"], 10, "https://gateway-a.test");
  ingress.initializeFromState(initial);

  // The stale A request is in flight when the user switches away and back:
  // the URL matches again, but the connection it answered for is gone.
  const stalePending = ingress.requestState(
    async () => state(["stale"], 99, "https://gateway-a.test", "a-stale-generation"),
  );
  ingress.beginGatewaySwitch("https://gateway-b.test");
  ingress.beginGatewaySwitch("https://gateway-a.test");
  const fresh = await ingress.requestState(
    async () => state(["fresh"], 3, "https://gateway-a.test", "a-new-generation"),
  );
  let committed = ingress.commitState(null, fresh);
  assert.deepEqual(committed.pinnedThreadIds, ["fresh"]);

  const stale = await stalePending;
  const afterStale = ingress.commitState(committed, stale);
  assert.equal(
    afterStale,
    committed,
    "the first A generation's delivery is rejected wholesale",
  );
});

test("renderer reload drops a previous-session delivery envelope", async () => {
  const initial = state(["a", "b"], 10);
  const previous = new PinnedOrderIngress("renderer-session-old");
  previous.initializeFromState(initial);
  const oldEnvelopeState = await previous.requestState(
    async () => state(["b", "a"], 11, undefined, "old-session"),
  );

  const reloaded = new PinnedOrderIngress("renderer-session-new");
  reloaded.initializeFromState(initial);
  const committed = reloaded.commitState(initial, oldEnvelopeState);

  assert.equal(committed, initial);
  assert.deepEqual(committed.pinnedThreadIds, ["a", "b"]);
});

test("revision floor rejects a whole lower-revision page at ingress", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  let committed = state(["a"], 10);
  ingress.initializeFromState(committed);
  const lowPending = ingress.requestState(
    async () => state(["stale"], 11, undefined, "low"),
  );
  const high = await ingress.requestState(
    async () => state(["new"], 12, undefined, "high"),
  );
  committed = ingress.commitState(committed, high);
  const low = await lowPending;
  committed = ingress.commitState(committed, low);

  assert.equal(ingress.highestObservedRevision, 12);
  assert.deepEqual(committed.pinnedThreadIds, ["new"]);
});

test("gateway identity check runs before revision acceptance", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  const old = state(["old"], 100, "https://old.example.test");
  ingress.initializeFromState(old);
  const oldPending = ingress.requestState(
    async () => state(["late-old"], 101, "https://old.example.test"),
  );

  ingress.beginGatewaySwitch("https://new.example.test");
  const fresh = await ingress.requestState(
    async () => state(["new"], 0, "https://new.example.test"),
    "https://new.example.test",
  );
  let committed = ingress.commitState(old, fresh);
  const lateOld = await oldPending;
  committed = ingress.commitState(committed, lateOld);

  assert.deepEqual(committed.pinnedThreadIds, ["new"]);
  assert.equal(ingress.highestObservedRevision, 0);
});

test("failed gateway switch restores its prior domain and invalidates old requests", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  let committed = state(["old-a", "old-b"], 100, "https://old.example.test");
  ingress.initializeFromState(committed);
  const oldPending = ingress.requestState(
    async () => state(["late-old"], 101, "https://old.example.test"),
  );

  const rollback = ingress.beginGatewaySwitch("https://new.example.test");
  ingress.restoreGatewayDomain(rollback);
  committed = ingress.commitState(committed, await oldPending);

  assert.deepEqual(committed.pinnedThreadIds, ["old-a", "old-b"]);
  assert.equal(ingress.highestObservedRevision, 100);
  assert.ok(ingress.currentEpoch > rollback.epoch);
});

test("legacy state without pinsRevision normalizes its renderer floor to zero", () => {
  const legacy = state(["a", "b"], 0);
  delete legacy.pinsRevision;
  legacy.capturedEpoch = 999;
  legacy.rendererSessionId = "persisted-by-mistake";
  const ingress = new PinnedOrderIngress("renderer-session-a");

  ingress.initializeFromState(legacy);

  assert.equal(ingress.highestObservedRevision, 0);
  assert.deepEqual(ingress.presentedOrder, ["a", "b"]);
});

test("accepted pages are frozen during drag and cancel publishes the newest page", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  let committed = state(["a", "b"], 10);
  ingress.initializeFromState(committed);
  assert.deepEqual(ingress.beginDrag(), ["a", "b"]);

  const revision12 = await ingress.requestState(
    async () => state(["c", "b", "a"], 12, undefined, "revision-12"),
  );
  committed = ingress.commitState(committed, revision12);
  assert.deepEqual(committed.pinnedThreadIds, ["a", "b"]);
  assert.equal(ingress.highestObservedRevision, 12);

  const revision11 = await ingress.requestState(
    async () => state(["b", "a"], 11, undefined, "revision-11"),
  );
  committed = ingress.commitState(committed, revision11);
  assert.deepEqual(committed.pinnedThreadIds, ["a", "b"]);
  assert.equal(ingress.highestObservedRevision, 12);

  const epochBeforeCancel = ingress.currentEpoch;
  const afterCancel = ingress.cancelDrag();
  committed = ingress.commitState(committed, (liveCurrent) => ({
    ...liveCurrent,
    pinnedThreadIds: afterCancel,
  }));
  assert.equal(ingress.currentEpoch, epochBeforeCancel);
  assert.deepEqual(committed.pinnedThreadIds, ["c", "b", "a"]);
});

test("drop reduces its preview against membership accepted during the freeze", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  let committed = state(["a", "b"], 10);
  ingress.initializeFromState(committed);
  ingress.beginDrag();

  const membershipPage = await ingress.requestState(
    async () => state(["c", "a", "b"], 11, undefined, "remote-pin"),
  );
  committed = ingress.commitState(committed, membershipPage);
  assert.deepEqual(committed.pinnedThreadIds, ["a", "b"]);

  const dropped = ingress.commitDragOrder(["b", "a"]);
  committed = ingress.commitState(committed, (liveCurrent) => ({
    ...liveCurrent,
    pinnedThreadIds: dropped,
  }));

  assert.deepEqual(dropped, ["c", "b", "a"]);
  assert.deepEqual(committed.pinnedThreadIds, ["c", "b", "a"]);
  assert.equal(ingress.currentEpoch, 1);
});

test("same-revision current-epoch page can settle an already-equal drop", async () => {
  const ingress = new PinnedOrderIngress("renderer-session-a");
  let committed = state(["a", "b"], 10);
  ingress.initializeFromState(committed);
  ingress.beginDrag();
  const dropped = ingress.commitDragOrder(["b", "a"]);
  committed = ingress.commitState(committed, (liveCurrent) => ({
    ...liveCurrent,
    pinnedThreadIds: dropped,
  }));

  const alreadyEqual = await ingress.requestState(
    async () => state(["b", "a"], 10, undefined, "already-equal"),
  );
  committed = ingress.commitState(committed, alreadyEqual);

  assert.equal(ingress.desiredOrder, null);
  assert.equal(ingress.currentEpoch, 2);
  assert.deepEqual(committed.pinnedThreadIds, ["b", "a"]);
});

test("renderer source funnels state requests and commits through the authority", async () => {
  const [{ readFile }, { default: path }] = await Promise.all([
    import("node:fs/promises"),
    import("node:path"),
  ]);
  const root = path.dirname(new URL(import.meta.url).pathname);
  const [appShell, platform, ingressSource] = await Promise.all([
    readFile(path.join(root, "app-shell/AppShell.tsx"), "utf8"),
    readFile(path.join(root, "platform/desktop-api.ts"), "utf8"),
    readFile(path.join(root, "pinned-order-ingress.ts"), "utf8"),
  ]);

  assert.equal((appShell.match(/setDesktopStateRaw/g) || []).length, 2);
  assert.match(appShell, /pinnedOrderIngress\.commitState\(current, action\)/);
  assert.match(platform, /requestDesktopStateResult/);
  assert.match(platform, /stateMethods/);
  assert.match(platform, /stateResultMethods/);
  const stamp = ingressSource.indexOf("const capturedEpoch = this.epoch");
  const requestAwait = ingressSource.indexOf("const result = await request()", stamp);
  assert.ok(stamp >= 0 && requestAwait > stamp);
});
