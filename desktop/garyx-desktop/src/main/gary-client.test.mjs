import test from "node:test";
import assert from "node:assert/strict";

import {
  ThreadStreamGapError,
  mapThreadStreamPassthroughPayload,
  streamGatewayEvents,
  streamThreadEvents,
} from "./gary-client.ts";

test("maps per-thread passthrough tool events into desktop chat stream events", () => {
  const [toolUse] = mapThreadStreamPassthroughPayload(
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

  const [toolResult] = mapThreadStreamPassthroughPayload(
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

test("maps per-thread passthrough history payloads into live renderer events", () => {
  const events = mapThreadStreamPassthroughPayload(
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

test("maps camelCase websocket-shaped per-thread passthrough payloads", () => {
  const events = mapThreadStreamPassthroughPayload(
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

test("maps gateway run_error into terminal desktop error event", () => {
  const [event] = mapThreadStreamPassthroughPayload(
    JSON.stringify({
      type: "run_error",
      thread_id: "thread::failed",
      run_id: "run-failed",
      error: "request timed out",
    }),
  );

  assert.equal(event.type, "error");
  assert.equal(event.threadId, "thread::failed");
  assert.equal(event.runId, "run-failed");
  assert.equal(event.sessionId, "thread::failed");
  assert.equal(event.error, "request timed out");
  assert.equal(event.terminal, true);
});

test("maps gateway run_start into desktop accepted event", () => {
  const [event] = mapThreadStreamPassthroughPayload(
    JSON.stringify({
      type: "run_start",
      thread_id: "thread::started",
      run_id: "run-started",
    }),
  );

  assert.equal(event.type, "accepted");
  assert.equal(event.threadId, "thread::started");
  assert.equal(event.runId, "run-started");
  assert.equal(event.sessionId, "thread::started");
});

test("maps committed_message payloads into desktop transcript stream events", () => {
  const [event] = mapThreadStreamPassthroughPayload(
    JSON.stringify({
      type: "committed_message",
      thread_id: "thread::committed",
      run_id: "run-committed",
      seq: 7,
      message: {
        role: "system",
        kind: "control",
        internal: true,
        internal_kind: "control",
        control: {
          kind: "run_start",
          run_id: "run-committed",
        },
      },
    }),
  );

  assert.equal(event.type, "committed_message");
  assert.equal(event.threadId, "thread::committed");
  assert.equal(event.runId, "run-committed");
  assert.equal(event.seq, 7);
  assert.equal(event.message.id, "thread::committed:6");
  assert.equal(event.message.kind, "control");
  assert.equal(event.message.text, "");
  assert.equal(event.message.content.control.kind, "run_start");
});

test("streamThreadEvents connects to per-thread stream with resume cursor", async () => {
  const urls = [];
  const lastEventIds = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init = {}) => {
    urls.push(String(url));
    lastEventIds.push(init.headers.get("Last-Event-ID"));
    const payload = JSON.stringify({
      type: "committed_message",
      thread_id: "thread::per-thread",
      run_id: "run-per-thread",
      seq: 5,
      message: {
        role: "assistant",
        content: "hello",
        text: "hello",
        timestamp: "2026-06-18T12:00:00Z",
      },
    });
    return new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode(`id: 5\ndata: ${payload}\n\n`));
          controller.close();
        },
      }),
      { status: 200, statusText: "OK" },
    );
  };

  try {
    const events = [];
    await streamThreadEvents(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      "thread::per-thread",
      (event) => events.push(event),
      undefined,
      { afterSeq: 4 },
    );

    assert.equal(
      urls[0],
      "http://127.0.0.1:31337/api/threads/thread%3A%3Aper-thread/stream?after_seq=4",
    );
    assert.equal(lastEventIds[0], "4");
    assert.equal(events.length, 1);
    assert.equal(events[0].type, "committed_message");
    assert.equal(events[0].seq, 5);
    assert.equal(events[0].message.id, "thread::per-thread:4");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamThreadEvents rejects first replay gap relative to requested cursor", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    const payload = JSON.stringify({
      type: "committed_message",
      thread_id: "thread::per-thread-gap",
      run_id: "run-per-thread-gap",
      seq: 7,
      message: {
        role: "assistant",
        content: "gap",
        text: "gap",
        timestamp: "2026-06-18T12:00:00Z",
      },
    });
    return new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode(`id: 7\ndata: ${payload}\n\n`));
          controller.close();
        },
      }),
      { status: 200, statusText: "OK" },
    );
  };

  try {
    const events = [];
    await assert.rejects(
      () =>
        streamThreadEvents(
          {
            gatewayUrl: "http://127.0.0.1:31337",
            gatewayAuthToken: "",
          },
          "thread::per-thread-gap",
          (event) => events.push(event),
          undefined,
          { afterSeq: 4 },
        ),
      (error) => {
        assert.equal(error instanceof ThreadStreamGapError, true);
        assert.equal(error.resumeAfterSeq, 4);
        return true;
      },
    );
    assert.equal(events.length, 0);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamThreadEvents forwards terminal run_error passthrough events", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    const payload = JSON.stringify({
      type: "run_error",
      thread_id: "thread::per-thread-failed",
      run_id: "run-per-thread-failed",
      error: "timeout",
    });
    return new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode(`data: ${payload}\n\n`));
          controller.close();
        },
      }),
      { status: 200, statusText: "OK" },
    );
  };

  try {
    const events = [];
    await streamThreadEvents(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      "thread::per-thread-failed",
      (event) => events.push(event),
    );

    assert.equal(events.length, 1);
    assert.equal(events[0].type, "error");
    assert.equal(events[0].threadId, "thread::per-thread-failed");
    assert.equal(events[0].runId, "run-per-thread-failed");
    assert.equal(events[0].error, "timeout");
    assert.equal(events[0].terminal, true);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamGatewayEvents replays unseen control history without duplicating seen live events", async () => {
  const liveUserAck = JSON.stringify({
    type: "user_ack",
    thread_id: "thread::stream-replay",
    run_id: "run-replay",
    pending_input_id: "pending-replay",
  });
  const unseenDone = JSON.stringify({
    type: "done",
    thread_id: "thread::stream-replay",
    run_id: "run-replay",
  });
  const historyEnvelope = JSON.stringify({
    type: "history",
    events: [liveUserAck, unseenDone],
  });
  const urls = [];
  const originalFetch = globalThis.fetch;
  let callCount = 0;
  globalThis.fetch = async (url) => {
    urls.push(String(url));
    const payload =
      callCount === 0
        ? `data: ${liveUserAck}\n\n`
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
      ["user_ack"],
    );
    assert.deepEqual(
      replayedEvents.map((event) => event.type),
      ["done"],
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamGatewayEvents ignores global content frames", async () => {
  const ignoredAssistantDelta = JSON.stringify({
    type: "assistant_delta",
    thread_id: "thread::global-content",
    run_id: "run-global-content",
    delta: "old global content path",
  });
  const ignoredToolUse = JSON.stringify({
    type: "tool_use",
    thread_id: "thread::global-content",
    run_id: "run-global-content",
    message: {
      role: "tool_use",
      content: { type: "shell" },
      tool_use_id: "toolu-global-content",
      tool_name: "shell",
    },
  });
  const ignoredCommitted = JSON.stringify({
    type: "committed_message",
    thread_id: "thread::global-content",
    run_id: "run-global-content",
    seq: 3,
    message: {
      role: "assistant",
      content: "committed globally",
      text: "committed globally",
    },
  });
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () =>
    new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(
            new TextEncoder().encode(
              [
                `data: ${ignoredAssistantDelta}`,
                "",
                `data: ${ignoredToolUse}`,
                "",
                `data: ${ignoredCommitted}`,
                "",
                "",
              ].join("\n"),
            ),
          );
          controller.close();
        },
      }),
      { status: 200, statusText: "OK" },
    );

  try {
    const events = [];
    await streamGatewayEvents(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      (event) => events.push(event),
      undefined,
      { historyLimit: 0 },
    );

    assert.deepEqual(events, []);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
