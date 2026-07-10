import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  ThreadStreamGapError,
  createTask,
  updateCustomAgent,
  fetchThreadHistory,
  getWorkflowRun,
  getTask,
  listTaskForest,
  listProviderModels,
  requestJson,
  setGatewayFetch,
  setGatewayStreamFetch,
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

test("listProviderModels maps provider default reasoning effort", async () => {
  const originalFetch = globalThis.fetch;
  let capturedUrl = "";
  globalThis.fetch = async (url) => {
    capturedUrl = String(url);
    return new Response(
      JSON.stringify({
        provider_type: "claude_code",
        supports_model_selection: true,
        models: [],
        supports_reasoning_effort_selection: true,
        reasoning_efforts: [],
        supports_service_tier_selection: false,
        service_tiers: [],
        default_model: "claude-opus-4-8",
        default_reasoning_effort: "max",
        source: "claude_code_builtin",
      }),
      { status: 200, statusText: "OK" },
    );
  };

  try {
    const providerModels = await listProviderModels(
      {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: "",
      },
      "claude_code",
    );

    assert.equal(
      capturedUrl,
      "http://127.0.0.1:31337/api/provider-models/claude_code",
    );
    assert.equal(providerModels.defaultModel, "claude-opus-4-8");
    assert.equal(providerModels.defaultReasoningEffort, "max");
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

test("streamThreadEvents rides the stream transport, control requests the default transport (#TASK-1840)", async () => {
  // Live SSE connections hold their socket for as long as they run; on the
  // shared Chromium pool six of them starve every control request into its
  // AbortSignal timeout. The stream seam is what keeps them on a separate
  // pool, so a regression that lands streams back on the control transport
  // must fail here.
  const controlUrls = [];
  const streamUrls = [];
  setGatewayFetch(async (url) => {
    controlUrls.push(String(url));
    return new Response(JSON.stringify({ ok: true, messages: [] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });
  setGatewayStreamFetch(async (url) => {
    streamUrls.push(String(url));
    const payload = renderFramePayload(
      "thread::transport",
      [committedEvent("thread::transport", 1, "hi")],
      1,
    );
    return sseResponse(`id: 1\ndata: ${payload}\n\n`);
  });

  try {
    await streamThreadEvents(
      { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
      "thread::transport",
      () => {},
    );
    await requestJson(
      { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
      "/api/threads/history?thread_id=thread%3A%3Atransport",
    );

    assert.equal(streamUrls.length, 1);
    assert.match(streamUrls[0], /\/api\/threads\/thread%3A%3Atransport\/stream/);
    assert.equal(controlUrls.length, 1);
    assert.match(controlUrls[0], /\/api\/threads\/history/);
  } finally {
    setGatewayFetch(null);
    setGatewayStreamFetch(null);
  }
});

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
      "http://127.0.0.1:31337/api/threads/thread%3A%3Aper-thread/stream?after_seq=4&render_mode=delta",
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

test("streamThreadEvents accepts a windowed replay frame and keeps the marker", async () => {
  // Server-degraded stale resume: the frame is marked replay:"windowed"
  // and its first event (the window floor) is deliberately NOT contiguous
  // with our cursor. The marker authorizes the discontinuity.
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    const payload = JSON.parse(
      renderFramePayload(
        "thread::windowed",
        [
          committedEvent("thread::windowed", 4801, "window head"),
          committedEvent("thread::windowed", 4802, "window tail"),
        ],
        4802,
      ),
    );
    payload.replay = "windowed";
    payload.render_state.window = { floor_seq: 4801, has_more_above: true };
    return sseResponse(`id: 4802\ndata: ${JSON.stringify(payload)}\n\n`);
  };

  try {
    const events = [];
    await streamThreadEvents(
      { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
      "thread::windowed",
      (event) => events.push(event),
      undefined,
      { afterSeq: 12 },
    );
    assert.equal(events.length, 1);
    assert.equal(events[0].type, "thread_render_frame");
    assert.equal(events[0].replay, "windowed");
    assert.equal(events[0].events.length, 2);
    assert.equal(events[0].events[0].seq, 4801);
    assert.equal(events[0].renderState.window.floor_seq, 4801);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamThreadEvents pins render_floor and reports window floors (#TASK-1715)", async () => {
  const urls = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url) => {
    urls.push(String(url));
    const payload = JSON.parse(
      renderFramePayload(
        "thread::floor",
        [committedEvent("thread::floor", 4802, "tail")],
        4802,
      ),
    );
    payload.render_state.window = { floor_seq: 4711, has_more_above: true };
    return sseResponse(`id: 4802\ndata: ${JSON.stringify(payload)}\n\n`);
  };

  try {
    const floors = [];
    await streamThreadEvents(
      { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
      "thread::floor",
      () => {},
      undefined,
      {
        afterSeq: 4801,
        renderFloor: 4700,
        onWindowFloor: (floorSeq) => floors.push(floorSeq),
      },
    );
    assert.equal(
      urls[0],
      "http://127.0.0.1:31337/api/threads/thread%3A%3Afloor/stream?after_seq=4801&render_mode=delta&render_floor=4700",
    );
    assert.deepEqual(
      floors,
      [4711],
      "a frame carrying a render window must report its floor",
    );

    // Without a pinned floor the request stays byte-identical to today.
    await streamThreadEvents(
      { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
      "thread::floor",
      () => {},
      undefined,
      { afterSeq: 4801 },
    );
    assert.equal(
      urls[1],
      "http://127.0.0.1:31337/api/threads/thread%3A%3Afloor/stream?after_seq=4801&render_mode=delta",
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamThreadEvents still gap-reconnects on unmarked non-contiguous frames", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    const payload = renderFramePayload(
      "thread::gap",
      [committedEvent("thread::gap", 4801, "far ahead")],
      4801,
    );
    return sseResponse(`id: 4801\ndata: ${payload}\n\n`);
  };

  try {
    await assert.rejects(
      streamThreadEvents(
        { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
        "thread::gap",
        () => {},
        undefined,
        { afterSeq: 12 },
      ),
      (error) => error instanceof ThreadStreamGapError,
    );
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

test("streamThreadEvents aborts a fetch whose headers never arrive (#TASK-1840)", async () => {
  // A gateway that accepts the TCP connection but never answers (saturation,
  // crash mid-accept) must not pin a Chromium connection slot forever.
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (_url, init = {}) =>
    new Promise((_resolve, reject) => {
      init.signal?.addEventListener(
        "abort",
        () => reject(new DOMException("aborted", "AbortError")),
        { once: true },
      );
    });

  try {
    await assert.rejects(
      streamThreadEvents(
        { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
        "thread::stalled-headers",
        () => {},
        undefined,
        { headerTimeoutMs: 40 },
      ),
      (error) => {
        assert.match(String(error), /stalled: no response headers within 40ms/);
        return true;
      },
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamThreadEvents recycles a silent stream after the idle timeout (#TASK-1840)", async () => {
  // Zombie SSE: the response connected and delivered one frame, then the
  // connection went half-open (no bytes, no keep-alive pings). The idle
  // watchdog must abort it so the caller's reconnect loop can resume from
  // the committed cursor instead of holding the slot forever.
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (_url, init = {}) => {
    const payload = renderFramePayload(
      "thread::stalled-idle",
      [committedEvent("thread::stalled-idle", 5, "before the silence")],
      5,
    );
    const body = new ReadableStream({
      start(controller) {
        controller.enqueue(
          new TextEncoder().encode(`id: 5\ndata: ${payload}\n\n`),
        );
        // Never close; the manual stream mirrors fetch-abort semantics so
        // the pending read rejects when the watchdog fires.
        init.signal?.addEventListener(
          "abort",
          () => controller.error(new DOMException("aborted", "AbortError")),
          { once: true },
        );
      },
    });
    return Promise.resolve(new Response(body, { status: 200, statusText: "OK" }));
  };

  try {
    const events = [];
    const committedSeqs = [];
    await assert.rejects(
      streamThreadEvents(
        { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
        "thread::stalled-idle",
        (event) => events.push(event),
        undefined,
        {
          afterSeq: 4,
          idleTimeoutMs: 40,
          onCommittedSeq: (seq) => committedSeqs.push(seq),
        },
      ),
      (error) => {
        assert.match(String(error), /stalled: no bytes within 40ms/);
        return true;
      },
    );
    // The frame that arrived before the silence was delivered and advanced
    // the cursor, so the reconnect resumes after seq 5 with no replay gap.
    assert.equal(events.length, 1);
    assert.deepEqual(committedSeqs, [5]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamThreadEvents keeps external aborts distinct from watchdog stalls", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (_url, init = {}) =>
    new Promise((_resolve, reject) => {
      // Real fetch rejects immediately on an already-aborted signal.
      if (init.signal?.aborted) {
        reject(new DOMException("aborted", "AbortError"));
        return;
      }
      init.signal?.addEventListener(
        "abort",
        () => reject(new DOMException("aborted", "AbortError")),
        { once: true },
      );
    });

  try {
    const controller = new AbortController();
    controller.abort();
    await assert.rejects(
      streamThreadEvents(
        { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
        "thread::external-abort",
        () => {},
        controller.signal,
        { headerTimeoutMs: 5_000 },
      ),
      (error) => {
        // An external abort surfaces as the fetch's AbortError, never as a
        // watchdog stall message.
        assert.equal(/stalled/.test(String(error)), false);
        return true;
      },
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("streamThreadEvents leaves fast healthy streams untouched by watchdog timeouts", async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    const payload = renderFramePayload(
      "thread::healthy",
      [committedEvent("thread::healthy", 2, "quick")],
      2,
    );
    return sseResponse(`id: 2\ndata: ${payload}\n\n`);
  };

  try {
    const events = [];
    await streamThreadEvents(
      { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
      "thread::healthy",
      (event) => events.push(event),
      undefined,
      { afterSeq: 1, headerTimeoutMs: 5_000, idleTimeoutMs: 5_000 },
    );
    assert.equal(events.length, 1);
    assert.equal(events[0].renderState.based_on_seq, 2);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("updateCustomAgent carries the optimistic concurrency token", async () => {
  const bodies = [];
  setGatewayFetch(async (url, init) => {
    bodies.push({ url: String(url), body: JSON.parse(init.body) });
    return new Response(
      JSON.stringify({
        agent_id: "occ-agent",
        display_name: "OCC",
        provider_type: "claude_code",
        built_in: false,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-02T00:00:00Z",
      }),
      { status: 200, statusText: "OK" },
    );
  });
  try {
    const settings = { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" };
    await updateCustomAgent(settings, {
      currentAgentId: "occ-agent",
      agentId: "occ-agent",
      displayName: "OCC",
      providerType: "claude_code",
      model: "",
      modelReasoningEffort: "",
      modelServiceTier: "",
      defaultWorkspaceDir: "",
      systemPrompt: "",
      expectedUpdatedAt: "2026-01-01T00:00:00Z",
    });
    assert.equal(bodies.length, 1);
    assert.equal(bodies[0].body.expected_updated_at, "2026-01-01T00:00:00Z");
  } finally {
    setGatewayFetch(null);
  }
});

test("setGatewayFetch routes gateway requests through the injected transport", async () => {
  // The Electron main entry injects net.fetch so gateway requests honor the
  // system proxy. When a transport is injected, gatewayFetch must use it and
  // NOT fall through to globalThis.fetch.
  const injectedUrls = [];
  const originalFetch = globalThis.fetch;
  let globalFetchCalled = false;
  globalThis.fetch = async () => {
    globalFetchCalled = true;
    throw new Error("globalThis.fetch must not be used when a transport is injected");
  };
  setGatewayFetch(async (url) => {
    injectedUrls.push(String(url));
    return new Response(JSON.stringify({ ok: true, via: "injected" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  });
  try {
    const result = await requestJson(
      { gatewayUrl: "https://garyx.example.test", gatewayAuthToken: "" },
      "/api/thing",
    );
    assert.deepEqual(result, { ok: true, via: "injected" });
    assert.equal(injectedUrls.length, 1);
    assert.equal(injectedUrls[0], "https://garyx.example.test/api/thing");
    assert.equal(globalFetchCalled, false);
  } finally {
    setGatewayFetch(null);
    globalThis.fetch = originalFetch;
  }
});

test("gatewayFetch falls back to globalThis.fetch when no transport is injected", async () => {
  // Outside Electron (unit tests / tooling) no transport is injected, so
  // gatewayFetch must read the live globalThis.fetch each call so stubs work.
  const originalFetch = globalThis.fetch;
  setGatewayFetch(null);
  const seen = [];
  globalThis.fetch = async (url) => {
    seen.push(String(url));
    return new Response(JSON.stringify({ ok: true, via: "global" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  try {
    const result = await requestJson(
      { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" },
      "/api/thing",
    );
    assert.deepEqual(result, { ok: true, via: "global" });
    assert.equal(seen.length, 1);
    assert.equal(seen[0], "http://127.0.0.1:31337/api/thing");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("fetchThreads keeps the full limit by default and honors a fast page limit", async () => {
  const originalFetch = globalThis.fetch;
  setGatewayFetch(null);
  const urls = [];
  globalThis.fetch = async (url) => {
    urls.push(String(url));
    return new Response(
      JSON.stringify({ threads: [{ thread_id: "thread::a", thread_label: "A" }] }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  };
  try {
    const settings = { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" };
    const { fetchThreads } = await import("./gary-client.ts");
    const full = await fetchThreads(settings);
    const fast = await fetchThreads(settings, { limit: 200 });
    assert.equal(urls[0], "http://127.0.0.1:31337/api/threads?limit=1000");
    assert.equal(urls[1], "http://127.0.0.1:31337/api/threads?limit=200");
    assert.equal(full.length, 1);
    assert.equal(fast[0].id, "thread::a");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("fetchThreadSummary maps a metadata payload and resolves null on miss", async () => {
  const originalFetch = globalThis.fetch;
  setGatewayFetch(null);
  globalThis.fetch = async (url) => {
    if (String(url).includes("thread%3A%3Agone")) {
      return new Response(JSON.stringify({ error: "thread not found" }), {
        status: 404,
        headers: { "content-type": "application/json" },
      });
    }
    return new Response(
      JSON.stringify({
        thread_id: "thread::pinned-old",
        label: "Pinned old thread",
        workspace_dir: "/Users/test/project",
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  };
  try {
    const settings = { gatewayUrl: "http://127.0.0.1:31337", gatewayAuthToken: "" };
    const { fetchThreadSummary } = await import("./gary-client.ts");
    const found = await fetchThreadSummary(settings, "thread::pinned-old");
    assert.equal(found.id, "thread::pinned-old");
    assert.equal(found.title, "Pinned old thread");
    const missing = await fetchThreadSummary(settings, "thread::gone");
    assert.equal(missing, null);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
