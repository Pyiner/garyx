import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { streamThreadEvents } from "../../../main/garyx-client/stream.ts";
import { buildThreadViewRows } from "../render-view-model.ts";
import { GatewayMirror } from "./mirror.ts";

const fixture = JSON.parse(
  readFileSync(
    new URL(
      "../../../../../../test-fixtures/stream-sync/queued-input-user-ack.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

function messagesBySeq(messages) {
  return new Map(
    messages
      .filter((message) => typeof message.seq === "number")
      .map((message) => [message.seq, message]),
  );
}

function queuedUserRow(mirror) {
  const snapshot = mirror.getThreadSnapshot(fixture.threadId);
  return buildThreadViewRows(
    snapshot.renderState,
    messagesBySeq(snapshot.messages),
  ).find(
    (row) =>
      row.kind === "user_turn" && row.userBlock.entry.message.seq === 133,
  );
}

function sseResponse(frame) {
  const body = `id: ${frame.render_state.based_on_seq}\ndata: ${JSON.stringify(frame)}\n\n`;
  return new Response(body, { status: 200, statusText: "OK" });
}

async function parseCapturedLiveFrame() {
  const originalFetch = globalThis.fetch;
  const events = [];
  globalThis.fetch = async () => sseResponse(fixture.rawLiveFrame);
  try {
    await streamThreadEvents(
      { gatewayUrl: "http://gateway.test", gatewayAuthToken: "" },
      fixture.threadId,
      (event) => events.push(event),
      undefined,
      { afterSeq: 132 },
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
  assert.equal(events.length, 1);
  return events[0];
}

test("live queued-input user_ack reconciliation survives a stale history response", async () => {
  const mirror = new GatewayMirror();
  mirror.applyRemoteTranscript(fixture.threadId, fixture.seedTranscript);
  mirror.syncThreadUiMessages(fixture.threadId, [
    ...mirror.getThreadSnapshot(fixture.threadId).messages,
    fixture.optimisticUser,
  ]);

  const frame = await parseCapturedLiveFrame();
  mirror.ingest(frame);

  // Model a history request that started at seq 132 before the SSE frame and
  // completed after its queued user + user_ack records had already landed.
  // The subsequent re-delivery is an intentional records-layer no-op, so the
  // history apply itself must keep the body cache coherent before seq de-dupe.
  mirror.applyRemoteTranscript(fixture.threadId, fixture.seedTranscript);
  mirror.ingest(frame);

  assert.deepEqual(
    mirror
      .getThreadSnapshot(fixture.threadId)
      .records.map((event) => event.seq),
    [133, 134, 135],
    "the live records remain cached across the stale history response",
  );

  const user = mirror
    .getThreadSnapshot(fixture.threadId)
    .messages.find((message) => message.id === `origin:${fixture.intentId}`);
  assert.equal(user?.seq, 133, "live reconciliation must carry committed seq");
  assert.ok(
    queuedUserRow(mirror),
    "buildThreadViewRows must keep the queued user bubble between turn rows",
  );
});

test("history replay keeps the queued-input user body addressable by render seq", () => {
  const mirror = new GatewayMirror();
  mirror.applyRemoteTranscript(fixture.threadId, fixture.historyTranscript);
  mirror.ingest({
    type: "thread_render_frame",
    threadId: fixture.threadId,
    events: [],
    renderState: fixture.rawLiveFrame.render_state,
  });

  const user = mirror
    .getThreadSnapshot(fixture.threadId)
    .messages.find((message) => message.id === `origin:${fixture.intentId}`);
  assert.equal(user?.seq, 133, "history mapping must retain committed seq");
  assert.ok(
    queuedUserRow(mirror),
    "buildThreadViewRows must render the history-loaded queued user bubble",
  );
});
