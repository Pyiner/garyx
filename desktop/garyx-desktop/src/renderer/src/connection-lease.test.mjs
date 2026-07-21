// The dual-identity connection lease: dies when EITHER the ingress domain
// generation (advances at switch REQUEST/rollback) or the mirror
// connection epoch (advances at switch COMMIT) moves. This is the fence
// long-running poll chains (automation reconcile, history refresh) hold —
// weakening either half reopens a cross-gateway persistence window.

import assert from "node:assert/strict";
import { test } from "node:test";

const { PinnedOrderIngress, installPinnedOrderIngress } = await import(
  "./pinned-order-ingress.ts"
);
const { openConnectionLease } = await import("./connection-lease.ts");

function fakeMirror() {
  let epoch = 0;
  return {
    get currentConnectionEpoch() {
      return epoch;
    },
    isCurrentConnectionEpoch: (value) => value === epoch,
    advance: () => {
      epoch += 1;
    },
  };
}

test("the lease dies at the switch REQUEST, before any commit", () => {
  const ingress = new PinnedOrderIngress("renderer-lease-1");
  installPinnedOrderIngress(ingress);
  ingress.beginGatewaySwitch("https://gateway-a.test");
  const mirror = fakeMirror();

  const lease = openConnectionLease(mirror);
  assert.equal(lease.isCurrent(), true);

  // The reviewer's exact window: settings saved, transport now answers
  // for B, but B's state has NOT committed (mirror epoch unchanged). The
  // generation half of the lease must already be dead.
  ingress.beginGatewaySwitch("https://gateway-b.test");
  assert.equal(mirror.isCurrentConnectionEpoch(0), true, "no commit yet");
  assert.equal(
    lease.isCurrent(),
    false,
    "the lease dies with the switch request, not the commit",
  );
});

test("the lease dies at the commit even without a generation change", () => {
  const ingress = new PinnedOrderIngress("renderer-lease-2");
  installPinnedOrderIngress(ingress);
  ingress.beginGatewaySwitch("https://gateway-a.test");
  const mirror = fakeMirror();

  const lease = openConnectionLease(mirror);
  mirror.advance();
  assert.equal(lease.isCurrent(), false, "epoch half stands on its own");
});

test("a rollback also kills leases opened before the aborted switch", () => {
  const ingress = new PinnedOrderIngress("renderer-lease-3");
  installPinnedOrderIngress(ingress);
  ingress.beginGatewaySwitch("https://gateway-a.test");
  const mirror = fakeMirror();

  const lease = openConnectionLease(mirror);
  const snapshot = ingress.beginGatewaySwitch("https://gateway-b.test");
  ingress.restoreGatewayDomain(snapshot);
  assert.equal(
    lease.isCurrent(),
    false,
    "rollback is a new generation; pre-switch leases stay dead",
  );
});
