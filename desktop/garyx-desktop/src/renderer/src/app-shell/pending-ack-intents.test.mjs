import assert from "node:assert/strict";
import test from "node:test";

import {
  pendingAckIntentsNotRepresented,
  representedUserIntentIds,
} from "./pending-ack-intents.ts";

test("pending ack lookup indexes optimistic and remote-origin ids once", () => {
  const represented = representedUserIntentIds([
    {
      id: "optimistic-user",
      role: "user",
      text: "local",
      intentId: "intent-local",
    },
    {
      id: "origin:intent-remote",
      role: "user",
      text: "remote",
    },
    {
      id: "assistant",
      role: "assistant",
      text: "intent-local",
      metadata: { origin_id: "ignored-non-user" },
    },
  ]);
  assert.deepEqual(
    [...represented].sort(),
    ["intent-local", "intent-remote"],
  );

  const visible = pendingAckIntentsNotRepresented(
    [
      { intentId: "intent-local" },
      { intentId: "intent-remote" },
      { intentId: "intent-visible" },
    ],
    represented,
  );
  assert.deepEqual(
    visible.map((intent) => intent.intentId),
    ["intent-visible"],
  );
});
