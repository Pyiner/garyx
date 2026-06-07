import test from "node:test";
import assert from "node:assert/strict";

import {
  mapGatewayEventPayload,
  streamGatewayEvents,
} from "./gary-client.ts";

test("maps gateway SSE tool events into desktop chat stream events", () => {
  const [toolUse] = mapGatewayEventPayload(
    JSON.stringify({
      type: "tool_use",
      thread_id: "thread::stream-1",
      run_id: "run-1",
      message: {
        role: "tool_use",
        content: {
          type: "shell",
          command: "git status --short",
        },
        tool_use_id: "toolu-1",
        tool_name: "shell",
        timestamp: "2026-06-07T00:00:00Z",
        metadata: {
          source: "claude_sdk",
        },
      },
    }),
  );

  assert.equal(toolUse.type, "tool_use");
  assert.equal(toolUse.threadId, "thread::stream-1");
  assert.equal(toolUse.runId, "run-1");
  assert.equal(toolUse.sessionId, "thread::stream-1");
  assert.equal(toolUse.message.role, "tool_use");
  assert.equal(toolUse.message.toolUseId, "toolu-1");
  assert.equal(toolUse.message.toolName, "shell");
  assert.deepEqual(toolUse.message.content, {
    type: "shell",
    command: "git status --short",
  });
  assert.deepEqual(toolUse.message.metadata, {
    source: "claude_sdk",
  });

  const [toolResult] = mapGatewayEventPayload(
    JSON.stringify({
      type: "tool_result",
      thread_id: "thread::stream-1",
      run_id: "run-1",
      message: {
        role: "tool_result",
        content: "clean",
        tool_use_id: "toolu-1",
        is_error: false,
      },
    }),
  );

  assert.equal(toolResult.type, "tool_result");
  assert.equal(toolResult.message.role, "tool_result");
  assert.equal(toolResult.message.toolUseId, "toolu-1");
  assert.equal(toolResult.message.content, "clean");
  assert.equal(toolResult.message.isError, false);
});

test("maps gateway SSE history payloads into live renderer events", () => {
  const events = mapGatewayEventPayload(
    JSON.stringify({
      type: "history",
      events: [
        JSON.stringify({
          type: "assistant_delta",
          thread_id: "thread::stream-2",
          run_id: "run-2",
          delta: "working",
        }),
        JSON.stringify({
          type: "user_ack",
          thread_id: "thread::stream-2",
          run_id: "run-2",
          pending_input_id: "pending-1",
        }),
        JSON.stringify({
          type: "done",
          thread_id: "thread::stream-2",
          run_id: "run-2",
        }),
      ],
    }),
  );

  assert.deepEqual(
    events.map((event) => event.type),
    ["assistant_delta", "user_ack", "done"],
  );
  assert.equal(events[0].threadId, "thread::stream-2");
  assert.equal(events[0].runId, "run-2");
  assert.equal(events[0].sessionId, "thread::stream-2");
  assert.equal(events[0].delta, "working");
  assert.equal(events[1].pendingInputId, "pending-1");
});

test("maps camelCase websocket-shaped gateway stream payloads", () => {
  const events = mapGatewayEventPayload(
    JSON.stringify({
      type: "tool_use",
      threadId: "thread::stream-3",
      runId: "run-3",
      message: {
        role: "tool_use",
        content: {
          type: "search",
        },
        toolUseId: "toolu-3",
        toolName: "search",
      },
    }),
  );

  assert.equal(events.length, 1);
  assert.equal(events[0].type, "tool_use");
  assert.equal(events[0].threadId, "thread::stream-3");
  assert.equal(events[0].runId, "run-3");
  assert.equal(events[0].message.toolUseId, "toolu-3");
  assert.equal(events[0].message.toolName, "search");
});

test("streamGatewayEvents replays unseen history without duplicating seen live events", async () => {
  const liveToolUse = JSON.stringify({
    type: "tool_use",
    thread_id: "thread::stream-replay",
    run_id: "run-replay",
    message: {
      role: "tool_use",
      content: { type: "shell" },
      tool_use_id: "toolu-replay",
      tool_name: "shell",
    },
  });
  const unseenToolResult = JSON.stringify({
    type: "tool_result",
    thread_id: "thread::stream-replay",
    run_id: "run-replay",
    message: {
      role: "tool_result",
      content: "ok",
      tool_use_id: "toolu-replay",
      is_error: false,
    },
  });
  const historyEnvelope = JSON.stringify({
    type: "history",
    events: [liveToolUse, unseenToolResult],
  });
  const urls = [];
  const originalFetch = globalThis.fetch;
  let callCount = 0;
  globalThis.fetch = async (url) => {
    urls.push(String(url));
    const payload =
      callCount === 0
        ? `data: ${liveToolUse}\n\n`
        : `event: history\ndata: ${historyEnvelope}\n\n`;
    callCount += 1;
    return new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode(payload));
          controller.close();
        },
      }),
      { status: 200, statusText: "OK" },
    );
  };

  try {
    const settings = {
      gatewayUrl: "http://127.0.0.1:31337",
      gatewayAuthToken: "",
    };
    const firstEvents = [];
    await streamGatewayEvents(
      settings,
      (event) => firstEvents.push(event),
      undefined,
      { historyLimit: 0 },
    );
    const replayedEvents = [];
    await streamGatewayEvents(
      settings,
      (event) => replayedEvents.push(event),
      undefined,
      { historyLimit: 50 },
    );

    assert.equal(urls[0], "http://127.0.0.1:31337/api/stream?history_limit=0");
    assert.equal(urls[1], "http://127.0.0.1:31337/api/stream?history_limit=50");
    assert.deepEqual(
      firstEvents.map((event) => event.type),
      ["tool_use"],
    );
    assert.deepEqual(
      replayedEvents.map((event) => event.type),
      ["tool_result"],
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});
