import test from "node:test";
import assert from "node:assert/strict";

import {
  ThreadStreamGapError,
  fetchThreadHistory,
  getTask,
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

test("getTask fetches task detail and preserves backing workflow thread id", async () => {
  const originalFetch = globalThis.fetch;
  const urls = [];
  globalThis.fetch = async (url) => {
    urls.push(String(url));
    return new Response(
      JSON.stringify({
        task_id: "#TASK-42",
        number: 42,
        title: "Synthetic workflow task",
        status: "in_progress",
        thread_id: "thread::workflow-task-42",
        executor: {
          type: "workflow",
          workflow_id: "development-loop",
          workflow_version: 1,
        },
      }),
      { status: 200, statusText: "OK" },
    );
  };

  try {
    const task = await getTask(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      { taskId: "#TASK-42" },
    );

    assert.equal(urls.length, 1);
    assert.equal(urls[0], "http://127.0.0.1:31337/api/tasks/%23TASK-42");
    assert.equal(task.taskId, "#TASK-42");
    assert.equal(task.threadId, "thread::workflow-task-42");
    assert.deepEqual(task.executor, {
      type: "workflow",
      workflowId: "development-loop",
      workflowVersion: 1,
    });
  } finally {
    globalThis.fetch = originalFetch;
  }
});

function committedEvent(threadId, seq, text) {
  return {
    type: "committed_message",
    thread_id: threadId,
    run_id: `run-${threadId}`,
    seq,
    message: {
      role: "assistant",
      content: text,
      text,
      timestamp: "2026-06-18T12:00:00Z",
    },
  };
}

function renderFramePayload(threadId, events, basedOnSeq) {
  return JSON.stringify({
    type: "thread_render_frame",
    thread_id: threadId,
    events,
    render_state: {
      based_on_seq: basedOnSeq,
      rows: [],
      tailActivity: "none",
      activeToolGroupId: null,
      progress_locus: "none",
      visibleMessageIds: [],
      filtered_placeholders: [],
    },
  });
}

function sseResponse(...frames) {
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const frame of frames) {
          controller.enqueue(new TextEncoder().encode(frame));
        }
        controller.close();
      },
    }),
    { status: 200, statusText: "OK" },
  );
}

test("streamThreadEvents connects to per-thread stream with resume cursor", async () => {
  const urls = [];
  const lastEventIds = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init = {}) => {
    urls.push(String(url));
    lastEventIds.push(init.headers.get("Last-Event-ID"));
    const payload = renderFramePayload(
      "thread::per-thread",
      [committedEvent("thread::per-thread", 5, "hello")],
      5,
    );
    return sseResponse(`id: 5\ndata: ${payload}\n\n`);
  };

  try {
    const events = [];
    const committedSeqs = [];
    await streamThreadEvents(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      "thread::per-thread",
      (event) => events.push(event),
      undefined,
      { afterSeq: 4, onCommittedSeq: (seq) => committedSeqs.push(seq) },
    );

    assert.equal(
      urls[0],
      "http://127.0.0.1:31337/api/threads/thread%3A%3Aper-thread/stream?after_seq=4",
    );
    assert.equal(lastEventIds[0], "4");
    assert.equal(events.length, 1);
    assert.equal(events[0].type, "thread_render_frame");
    assert.equal(events[0].events.length, 1);
    assert.equal(events[0].events[0].seq, 5);
    assert.equal(events[0].events[0].message.id, "thread::per-thread:4");
    assert.equal(events[0].renderState.based_on_seq, 5);
    assert.deepEqual(committedSeqs, [5]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamThreadEvents accepts a batched catch-up frame without reconnecting", async () => {
  // Regression for the SSR frame protocol: a reconnect/catch-up frame carries
  // events[M+1..N] in one frame with based_on_seq=N. Gap detection runs per
  // inner event (M+1, M+2, …), so it must NOT treat based_on_seq=N as a gap.
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    const payload = renderFramePayload(
      "thread::per-thread-batch",
      [
        committedEvent("thread::per-thread-batch", 5, "five"),
        committedEvent("thread::per-thread-batch", 6, "six"),
        committedEvent("thread::per-thread-batch", 7, "seven"),
      ],
      7,
    );
    return sseResponse(`id: 7\ndata: ${payload}\n\n`);
  };

  try {
    const events = [];
    const committedSeqs = [];
    await streamThreadEvents(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      "thread::per-thread-batch",
      (event) => events.push(event),
      undefined,
      { afterSeq: 4, onCommittedSeq: (seq) => committedSeqs.push(seq) },
    );

    assert.equal(events.length, 1);
    assert.equal(events[0].type, "thread_render_frame");
    assert.equal(events[0].events.length, 3);
    assert.deepEqual(
      events[0].events.map((event) => event.seq),
      [5, 6, 7],
    );
    assert.equal(events[0].renderState.based_on_seq, 7);
    assert.deepEqual(committedSeqs, [7]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamThreadEvents rejects first replay gap relative to requested cursor", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    const payload = renderFramePayload(
      "thread::per-thread-gap",
      [committedEvent("thread::per-thread-gap", 7, "gap")],
      7,
    );
    return sseResponse(`id: 7\ndata: ${payload}\n\n`);
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

test("streamThreadEvents ignores non-render per-thread frames", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    const legacyPayload = JSON.stringify({
      type: "run_error",
      thread_id: "thread::per-thread-failed",
      run_id: "run-per-thread-failed",
      error: "timeout",
    });
    const renderPayload = renderFramePayload(
      "thread::per-thread-failed",
      [committedEvent("thread::per-thread-failed", 1, "committed")],
      1,
    );
    return sseResponse(`data: ${legacyPayload}\n\ndata: ${renderPayload}\n\n`);
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
    assert.equal(events[0].type, "thread_render_frame");
    assert.equal(events[0].threadId, "thread::per-thread-failed");
    assert.equal(events[0].events[0].seq, 1);
    assert.equal(events[0].events[0].message.text, "committed");
  } finally {
    globalThis.fetch = originalFetch;
  }
});
