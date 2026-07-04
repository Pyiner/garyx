// Contract tests for DesktopRouteStore (endgame architecture batch 4a).
// A fake RouteHost simulates the browser hash surface, including the
// hashchange echo that a real location.hash assignment produces.

import assert from "node:assert/strict";
import { test } from "node:test";

import { DesktopRouteStore } from "./desktop-route-store.ts";
import { desktopRoutesEqual } from "./desktop-route.ts";

function fakeHost(initialHash = "") {
  const listeners = new Set();
  const host = {
    hash: initialHash,
    log: [],
    getHref() {
      return `file:///index.html${this.hash}`;
    },
    replaceHash(hash) {
      this.hash = hash;
      this.log.push({ op: "replace", hash });
      // history.replaceState does NOT fire hashchange.
    },
    pushHash(hash) {
      this.hash = hash;
      this.log.push({ op: "push", hash });
      // location.hash assignment fires hashchange asynchronously in real
      // browsers; fire synchronously here (stricter for echo dedupe).
      for (const listener of [...listeners]) {
        listener();
      }
    },
    subscribe(onExternalChange) {
      listeners.add(onExternalChange);
      return () => {
        listeners.delete(onExternalChange);
      };
    },
    /** Simulate a manual/external hash edit. */
    externalEdit(hash) {
      this.hash = hash;
      for (const listener of [...listeners]) {
        listener();
      }
    },
  };
  return host;
}

test("seeds from the initial hash and keeps a stable snapshot reference", () => {
  const host = fakeHost("#/thread/thread%3A%3Aabc");
  const store = new DesktopRouteStore(host);
  const snap1 = store.getSnapshot();
  const snap2 = store.getSnapshot();
  assert.equal(snap1, snap2, "unchanged snapshot must be the same ref");
  assert.deepEqual(snap1.route, { kind: "thread", threadId: "thread::abc" });
  store.dispose();
});

test("navigate pushes the hash, commits once, and dedupes its own echo", () => {
  const host = fakeHost("#/thread");
  const store = new DesktopRouteStore(host);
  let notified = 0;
  store.subscribe(() => {
    notified += 1;
  });

  store.navigate({ kind: "view", view: "tasks" });
  assert.equal(notified, 1, "one commit despite the synchronous echo");
  assert.deepEqual(store.getSnapshot().route, { kind: "view", view: "tasks" });
  assert.deepEqual(host.log, [{ op: "push", hash: "#/tasks" }]);

  // Navigating to the same route again is a full no-op.
  store.navigate({ kind: "view", view: "tasks" });
  assert.equal(notified, 1);
  assert.equal(host.log.length, 1);
  store.dispose();
});

test("navigate with replace uses replaceState and does not push", () => {
  const host = fakeHost("#/thread");
  const store = new DesktopRouteStore(host);
  store.navigate({ kind: "settings", tabId: "gateway" }, { replace: true });
  assert.deepEqual(host.log, [{ op: "replace", hash: "#/settings/gateway" }]);
  assert.deepEqual(store.getSnapshot().route, {
    kind: "settings",
    tabId: "gateway",
  });
  store.dispose();
});

test("an equal route with a non-canonical hash canonicalizes via replace without a commit", () => {
  // #/threads/<id> is a legacy alias for #/thread/<id>.
  const host = fakeHost("#/threads/thread%3A%3Aabc");
  const store = new DesktopRouteStore(host);
  let notified = 0;
  store.subscribe(() => {
    notified += 1;
  });
  const before = store.getSnapshot();

  store.navigate({ kind: "thread", threadId: "thread::abc" });
  assert.equal(notified, 0, "route did not change: no commit");
  assert.equal(store.getSnapshot(), before, "snapshot ref unchanged");
  assert.deepEqual(host.log, [
    { op: "replace", hash: "#/thread/thread%3A%3Aabc" },
  ]);
  store.dispose();
});

test("external hash edits commit the parsed route (no counter-write)", () => {
  const host = fakeHost("#/thread/thread%3A%3Aabc");
  const store = new DesktopRouteStore(host);
  let notified = 0;
  store.subscribe(() => {
    notified += 1;
  });

  host.externalEdit("#/agents");
  assert.equal(notified, 1);
  assert.deepEqual(store.getSnapshot().route, { kind: "view", view: "agents" });
  // The store must not have written the hash back.
  assert.deepEqual(host.log, []);

  // An unknown-thread hash stays addressable as a thread route.
  host.externalEdit("#/thread/thread%3A%3Amissing");
  assert.equal(notified, 2);
  assert.deepEqual(store.getSnapshot().route, {
    kind: "thread",
    threadId: "thread::missing",
  });
  assert.deepEqual(host.log, [], "still no counter-write");
  store.dispose();
});

test("an external edit that parses to the current route is ignored", () => {
  const host = fakeHost("#/thread/thread%3A%3Aabc");
  const store = new DesktopRouteStore(host);
  let notified = 0;
  store.subscribe(() => {
    notified += 1;
  });
  // Same route, alias spelling.
  host.externalEdit("#/threads/thread%3A%3Aabc");
  assert.equal(notified, 0);
  store.dispose();
});

test("dispose unsubscribes from the host", () => {
  const host = fakeHost("#/thread");
  const store = new DesktopRouteStore(host);
  let notified = 0;
  store.subscribe(() => {
    notified += 1;
  });
  store.dispose();
  host.externalEdit("#/agents");
  assert.equal(notified, 0);
});

test("desktopRoutesEqual normalizes optional null/undefined fields", () => {
  assert.ok(
    desktopRoutesEqual(
      { kind: "new-thread", workspacePath: null, agentId: undefined },
      { kind: "new-thread" },
    ),
  );
  assert.ok(
    !desktopRoutesEqual(
      { kind: "new-thread", workspacePath: "/Users/test/repo" },
      { kind: "new-thread" },
    ),
  );
  assert.ok(
    desktopRoutesEqual({ kind: "automation" }, { kind: "automation", automationId: null }),
  );
  assert.ok(
    !desktopRoutesEqual(
      { kind: "thread", threadId: "thread::a" },
      { kind: "thread", threadId: "thread::b" },
    ),
  );
  assert.ok(
    !desktopRoutesEqual({ kind: "thread-home" }, { kind: "view", view: "tasks" }),
  );
});
