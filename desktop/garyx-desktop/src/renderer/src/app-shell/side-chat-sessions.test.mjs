// Contract tests for the shell-owned SideChatSessions store (endgame
// batch 5b-7a, docs/design/appshell-sidechat-colocation.md): snapshot
// caching (uSES rules), sessionStorage read-through/write, draft CRUD,
// the attachment-upload composer lock, and the orchestration shadow
// refs the dispatch/lifecycle deps consume.

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

const storageBacking = new Map();
globalThis.window = {
  sessionStorage: {
    getItem: (key) => (storageBacking.has(key) ? storageBacking.get(key) : null),
    setItem: (key, value) => {
      storageBacking.set(key, String(value));
    },
    removeItem: (key) => {
      storageBacking.delete(key);
    },
  },
};

const { SideChatSessions } = await import("./side-chat-sessions.ts");

const SOURCE = "thread::source-1";
const SIDE = "thread::side-1";

beforeEach(() => {
  storageBacking.clear();
});

test("snapshot is cached by reference and rebuilt only after writes", () => {
  const store = new SideChatSessions();
  const first = store.getSnapshot();
  assert.equal(store.getSnapshot(), first, "same reference until a write");

  let notified = 0;
  store.subscribe(() => {
    notified += 1;
  });
  store.rememberThread(SOURCE, SIDE);
  const second = store.getSnapshot();
  assert.notEqual(second, first, "write rebuilds the snapshot");
  assert.equal(second.threadBySource[SOURCE], SIDE);
  assert.equal(notified, 1);

  // Idempotent write: no bump, no notify.
  store.rememberThread(SOURCE, SIDE);
  assert.equal(store.getSnapshot(), second);
  assert.equal(notified, 1);
});

test("rememberThread persists to sessionStorage; restorePersisted reads through", () => {
  const store = new SideChatSessions();
  store.gatewayScope = "http://gateway-a";
  store.rememberThread(SOURCE, SIDE);
  // Bindings are gateway-partitioned: thread ids are only unique per
  // gateway, so the storage key carries the scope.
  assert.equal(
    storageBacking.get(`garyx.side-tools.side-chat-thread.http://gateway-a.${SOURCE}`),
    SIDE,
  );

  // A fresh store on the SAME gateway adopts the persisted binding.
  const fresh = new SideChatSessions();
  fresh.gatewayScope = "http://gateway-a";
  assert.equal(fresh.threadFor(SOURCE), null);
  assert.equal(fresh.restorePersisted(SOURCE), SIDE);
  assert.equal(fresh.threadFor(SOURCE), SIDE);
  // In-memory binding wins over storage on subsequent calls.
  assert.equal(fresh.restorePersisted(SOURCE), SIDE);

  // A different gateway never adopts gateway A's binding.
  const otherGateway = new SideChatSessions();
  otherGateway.gatewayScope = "http://gateway-b";
  assert.equal(otherGateway.restorePersisted(SOURCE), null);
});

test("forgetThread drops only the expected binding and keeps storage", () => {
  const store = new SideChatSessions();
  store.gatewayScope = "http://gateway-a";
  store.rememberThread(SOURCE, SIDE);
  store.forgetThread(SOURCE, "thread::other");
  assert.equal(store.threadFor(SOURCE), SIDE, "mismatched id is a no-op");
  store.forgetThread(SOURCE, SIDE);
  assert.equal(store.threadFor(SOURCE), null);
  assert.equal(
    storageBacking.get(`garyx.side-tools.side-chat-thread.http://gateway-a.${SOURCE}`),
    SIDE,
    "storage entry stays (legacy catch only cleared memory)",
  );
});

test("shadow refs track bindings and the active source", () => {
  const store = new SideChatSessions();
  store.setActiveSource(SOURCE);
  assert.equal(store.sideChatThreadIdRef.current, null);

  store.rememberThread(SOURCE, SIDE);
  assert.equal(store.sideChatThreadIdRef.current, SIDE);
  assert.ok(store.sideChatThreadIdsRef.current.has(SIDE));

  store.setActiveSource("thread::source-2");
  assert.equal(store.sideChatThreadIdRef.current, null);
  assert.ok(
    store.sideChatThreadIdsRef.current.has(SIDE),
    "the ids shadow keeps all bound side threads",
  );

  store.rememberSideThreadId("thread::side-oob");
  assert.ok(store.sideChatThreadIdsRef.current.has("thread::side-oob"));
});

test("draft CRUD survives across snapshots (the dock-toggle oracle)", () => {
  const store = new SideChatSessions();
  store.updateDraft(SOURCE, (current) => ({
    ...current,
    text: "half-typed",
    textPresent: true,
  }));
  assert.equal(store.draftFor(SOURCE).text, "half-typed");

  // A no-op updater (same reference) neither bumps nor notifies.
  const before = store.getSnapshot();
  store.updateDraft(SOURCE, (current) => current);
  assert.equal(store.getSnapshot(), before);

  store.clearDraft(SOURCE);
  const cleared = store.draftFor(SOURCE);
  assert.equal(cleared.text, "");
  assert.equal(cleared.resetKey, 1, "clear bumps the composer reset key");
});

test("attachment upload counter locks and unlocks", () => {
  const store = new SideChatSessions();
  store.beginAttachmentUpload();
  store.beginAttachmentUpload();
  assert.equal(store.getSnapshot().attachmentUploadCount, 2);
  store.endAttachmentUpload();
  store.endAttachmentUpload();
  store.endAttachmentUpload();
  assert.equal(
    store.getSnapshot().attachmentUploadCount,
    0,
    "never below zero",
  );
});

test("creating/error transients and the creation promise de-dupe", async () => {
  const store = new SideChatSessions();
  store.setCreating(SOURCE, true);
  store.setError(SOURCE, "boom");
  assert.equal(store.getSnapshot().creatingBySource[SOURCE], true);
  assert.equal(store.getSnapshot().errorBySource[SOURCE], "boom");
  store.setError(SOURCE, null);
  assert.ok(!(SOURCE in store.getSnapshot().errorBySource));

  const promise = Promise.resolve(SIDE);
  store.setCreationPromise(SOURCE, promise);
  assert.equal(store.creationPromiseFor(SOURCE), promise);
  store.setCreationPromise(SOURCE, null);
  assert.equal(store.creationPromiseFor(SOURCE), undefined);
  await promise;
});
