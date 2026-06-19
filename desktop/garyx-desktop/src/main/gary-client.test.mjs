import test from "node:test";
import assert from "node:assert/strict";

import {
  ThreadStreamGapError,
  fetchThreadHistory,
  streamThreadEvents,
} from "./gary-client.ts";

test("fetchThreadHistory preserves kind parity fields for committed reducers", async () => {
  const originalFetch = globalThis.fetch;
  const urls = [];
  globalThis.fetch = async (url) => {
    urls.push(String(url));
    return new Response(
      JSON.stringify({
        ok: true,
        messages: [
          {
            index: 0,
            role: "tool",
            kind: "tool_trace",
            timestamp: "2026-06-18T12:00:00Z",
            message: {
              role: "tool",
              input: {
                tool_calls: [{ id: "call-history-tool" }],
              },
              result: {
                tool_use_id: "call-history-tool",
              },
            },
          },
          {
            index: 1,
            role: "assistant",
            kind: "assistant_reply",
            timestamp: "2026-06-18T12:00:01Z",
            message: {
              role: "assistant",
              input: {
                tool_calls: [{ id: "call-history-input" }],
              },
            },
          },
        ],
        pending_user_inputs: [],
      }),
      { status: 200, statusText: "OK" },
    );
  };

  try {
    const transcript = await fetchThreadHistory(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      {
        threadId: "thread::history-parity",
        afterIndex: 0,
      },
    );

    assert.equal(urls.length, 1);
    assert.match(urls[0], /\/api\/threads\/history\?/);
    assert.equal(transcript.messages.length, 2);
    assert.equal(transcript.messages[0].role, "tool");
    assert.deepEqual(transcript.messages[0].input, {
      tool_calls: [{ id: "call-history-tool" }],
    });
    assert.deepEqual(transcript.messages[0].result, {
      tool_use_id: "call-history-tool",
    });
    assert.equal(transcript.messages[1].role, "assistant");
    assert.deepEqual(transcript.messages[1].input, {
      tool_calls: [{ id: "call-history-input" }],
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
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

test("streamThreadEvents ignores non-committed per-thread frames", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    const legacyPayload = JSON.stringify({
      type: "run_error",
      thread_id: "thread::per-thread-failed",
      run_id: "run-per-thread-failed",
      error: "timeout",
    });
    const committedPayload = JSON.stringify({
      type: "committed_message",
      thread_id: "thread::per-thread-failed",
      run_id: "run-per-thread-failed",
      seq: 1,
      message: {
        role: "assistant",
        content: "committed",
        text: "committed",
      },
    });
    return new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(
            new TextEncoder().encode(
              `data: ${legacyPayload}\n\ndata: ${committedPayload}\n\n`,
            ),
          );
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
    assert.equal(events[0].type, "committed_message");
    assert.equal(events[0].threadId, "thread::per-thread-failed");
    assert.equal(events[0].seq, 1);
    assert.equal(events[0].message.text, "committed");
  } finally {
    globalThis.fetch = originalFetch;
  }
});
