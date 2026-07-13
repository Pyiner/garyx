import assert from "node:assert/strict";
import test from "node:test";

import {
  deferConversationRailUnmount,
  settleDeferredConversationRailUnmount,
} from "./conversation-rail-lifecycle.ts";

test("closing keeps the mounted rail until its in-flow track is gone", () => {
  const recent = { kind: "recent" };
  const closed = { kind: "closed" };

  const deferred = deferConversationRailUnmount(recent, closed);
  assert.equal(deferred, recent);
  assert.equal(
    settleDeferredConversationRailUnmount(deferred, closed, true),
    recent,
  );
  assert.deepEqual(
    settleDeferredConversationRailUnmount(deferred, closed, false),
    closed,
  );
});

test("opening and switching rails apply immediately", () => {
  const closed = { kind: "closed" };
  const recent = { kind: "recent" };
  const bot = { kind: "bot", groupId: "synthetic-group" };

  assert.equal(deferConversationRailUnmount(closed, recent), recent);
  assert.equal(deferConversationRailUnmount(recent, bot), bot);
  assert.equal(
    settleDeferredConversationRailUnmount(recent, bot, false),
    recent,
  );
});
