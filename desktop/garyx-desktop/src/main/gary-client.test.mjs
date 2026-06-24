import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  ThreadStreamGapError,
  createTask,
  fetchThreadHistory,
  getWorkflowRun,
  getTask,
  listTaskForest,
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

test("fetchThreadHistory sends configured gateway headers", async () => {
  const originalFetch = globalThis.fetch;
  let capturedHeaders = null;
  globalThis.fetch = async (_url, options) => {
    capturedHeaders = new Headers(options?.headers);
    return new Response(
      JSON.stringify({
        ok: true,
        messages: [],
        pending_user_inputs: [],
      }),
      { status: 200, statusText: "OK" },
    );
  };

  try {
    await fetchThreadHistory(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "test-token",
        gatewayHeaders: [
          "X-Garyx-Proxy: proxy-token",
          "X-Trace-Id=trace-123",
        ].join("\n"),
      },
      {
        threadId: "thread::header-test",
        afterIndex: 0,
      },
    );

    assert.equal(capturedHeaders.get("Authorization"), "Bearer test-token");
    assert.equal(capturedHeaders.get("X-Garyx-Proxy"), "proxy-token");
    assert.equal(capturedHeaders.get("X-Trace-Id"), "trace-123");
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

test("listTaskForest maps parent and run-state fields", async () => {
  const originalFetch = globalThis.fetch;
  const urls = [];
  globalThis.fetch = async (url) => {
    urls.push(String(url));
    return new Response(
      JSON.stringify({
        tasks: [
          {
            kind: "thread",
            node_id: "thread-root:thread::forest-parent",
            thread_id: "thread::forest-parent",
            title: "Pinned conversation",
            thread_type: "chat",
            provider_type: "codex",
            agent_id: "codex",
            message_count: 9,
            last_message_preview: "Launch from here",
            active_run_id: null,
            run_state: "idle",
            updated_at: "2026-06-22T00:00:00Z",
            last_active_at: "2026-06-22T00:00:00Z",
          },
          {
            kind: "task",
            node_id: "task:thread::forest-child",
            parent_node_id: "thread-root:thread::forest-parent",
            task_id: "#TASK-7",
            number: 7,
            title: "Synthetic forest child",
            status: "in_progress",
            thread_id: "thread::forest-child",
            creator: { kind: "agent", agent_id: "claude" },
            updated_by: { kind: "agent", agent_id: "claude" },
            updated_at: "2026-06-22T00:00:00Z",
            runtime_agent_id: "claude",
            reply_count: 5,
            parent_task_number: 3,
            parent_thread_id: "thread::forest-parent",
            active_run_id: "run::forest-active",
            run_state: "running",
            last_active_at: "2026-06-22T00:01:00Z",
          },
        ],
        total: 2,
        projection_current: true,
        root_thread_ids: ["thread::forest-parent"],
        skipped_pinned_thread_ids: ["thread::plain-chat"],
      }),
      { status: 200, statusText: "OK" },
    );
  };

  try {
    const page = await listTaskForest(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      {
        status: "in_progress",
        sourceBot: "test-bot",
        includeDone: true,
        anchorThreadId: "thread::forest-child",
      },
    );

    assert.equal(urls.length, 1);
    assert.equal(
      urls[0],
      "http://127.0.0.1:31337/api/tasks/forest?status=in_progress&source_bot_id=test-bot&include_done=true&anchor_thread_id=thread%3A%3Aforest-child",
    );
    assert.equal(page.total, 2);
    assert.equal(page.projectionCurrent, true);
    assert.deepEqual(page.rootThreadIds, ["thread::forest-parent"]);
    assert.deepEqual(page.skippedPinnedThreadIds, ["thread::plain-chat"]);
    assert.equal(page.tasks[0].kind, "thread");
    assert.equal(page.tasks[0].nodeId, "thread-root:thread::forest-parent");
    assert.equal(page.tasks[0].title, "Pinned conversation");
    assert.equal(page.tasks[0].messageCount, 9);
    assert.equal(page.tasks[1].kind, "task");
    assert.equal(page.tasks[1].nodeId, "task:thread::forest-child");
    assert.equal(
      page.tasks[1].parentNodeId,
      "thread-root:thread::forest-parent",
    );
    assert.equal(page.tasks[1].taskId, "#TASK-7");
    assert.equal(page.tasks[1].parentTaskNumber, 3);
    assert.equal(page.tasks[1].parentThreadId, "thread::forest-parent");
    assert.equal(page.tasks[1].activeRunId, "run::forest-active");
    assert.equal(page.tasks[1].runState, "running");
    assert.equal(page.tasks[1].lastActiveAt, "2026-06-22T00:01:00Z");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("createTask serializes child task source fields", async () => {
  const originalFetch = globalThis.fetch;
  let requestBody = null;
  globalThis.fetch = async (_url, options) => {
    requestBody = JSON.parse(options.body);
    return new Response(
      JSON.stringify({
        task_id: "#TASK-8",
        number: 8,
        title: "Synthetic child",
        status: "in_progress",
        thread_id: "thread::forest-created-child",
        creator: { kind: "agent", agent_id: "claude" },
        updated_by: { kind: "agent", agent_id: "claude" },
        updated_at: "2026-06-22T00:02:00Z",
        runtime_agent_id: "claude",
        reply_count: 0,
      }),
      { status: 201, statusText: "Created" },
    );
  };

  try {
    const task = await createTask(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      {
        title: "Synthetic child",
        body: null,
        source: {
          threadId: "thread::forest-parent",
          taskId: "#TASK-7",
          taskThreadId: "thread::forest-parent",
          botId: "test-bot",
          channel: "test-channel",
          accountId: "test-account",
        },
        executor: { type: "agent", agentId: "claude" },
        start: true,
        workspaceDir: "/Users/test/project",
        workspaceMode: "local",
        notificationTarget: { kind: "none" },
      },
    );

    assert.equal(task.taskId, "#TASK-8");
    assert.deepEqual(requestBody.source, {
      thread_id: "thread::forest-parent",
      task_id: "#TASK-7",
      task_thread_id: "thread::forest-parent",
      bot_id: "test-bot",
      channel: "test-channel",
      account_id: "test-account",
    });
    assert.deepEqual(requestBody.executor, {
      type: "agent",
      agent_id: "claude",
    });
    assert.equal(requestBody.runtime.workspace_dir, "/Users/test/project");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("getWorkflowRun maps shared server presentation fixture", async () => {
  const originalFetch = globalThis.fetch;
  const urls = [];
  const fixture = JSON.parse(
    readFileSync(
      new URL(
        "../../../../test-fixtures/workflow-presentation/mobile-desktop-parity.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  globalThis.fetch = async (url) => {
    urls.push(String(url));
    return new Response(JSON.stringify(fixture), {
      status: 200,
      statusText: "OK",
    });
  };

  try {
    const run = await getWorkflowRun(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      { workflowRunId: "thread::workflow-1001" },
    );

    assert.equal(
      urls[0],
      "http://127.0.0.1:31337/api/workflows/thread%3A%3Aworkflow-1001",
    );
    assert.equal(run.presentation?.workflowRunId, "thread::workflow-1001");
    assert.equal(run.presentation?.terminalComplete, false);
    assert.equal(run.presentation?.stale, false);
    assert.deepEqual(
      run.presentation?.phases.map((phase) => phase.phaseId),
      ["plan", "review", "finalize"],
    );
    assert.deepEqual(
      run.presentation?.phases[1].children.map(
        (child) => child.workflowChildRunId,
      ),
      ["child::risk", "child::lint"],
    );
    assert.deepEqual(
      run.presentation?.childCards.map((child) => child.workflowChildRunId),
      ["child::risk", "child::lint", "child::summary"],
    );
    assert.equal(run.presentation?.snapshotVersion, 1782028950000);
    assert.equal(run.presentation?.latestEventSeq, 2);
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
