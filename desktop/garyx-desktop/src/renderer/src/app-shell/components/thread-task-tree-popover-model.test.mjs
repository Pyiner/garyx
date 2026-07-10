import test from "node:test";
import assert from "node:assert/strict";

import {
  buildTaskRows,
  isCurrentTaskTreeNode,
  resolveTaskTreeActiveCount,
  shouldShowThreadTaskTreePopover,
  taskStatusLabel,
  taskStatusTone,
  taskTreeBadgeCount,
  visibleTaskTreeTasks,
} from "./thread-task-tree-popover-model.ts";

function task(overrides) {
  const number = overrides.number;
  return {
    kind: "task",
    nodeId: `task:thread::${number}`,
    parentNodeId: null,
    threadId: `thread::${number}`,
    taskId: `#TASK-${number}`,
    number,
    title: `Task ${number}`,
    status: "todo",
    creator: { kind: "agent", agentId: "test-agent" },
    assignee: null,
    source: null,
    executor: null,
    updatedAt: "2026-01-01T00:00:00.000Z",
    updatedBy: { kind: "agent", agentId: "test-agent" },
    runtimeAgentId: "test-agent",
    replyCount: 0,
    parentTaskNumber: null,
    parentThreadId: null,
    activeRunId: null,
    runState: "idle",
    lastActiveAt: null,
    depth: null,
    ...overrides,
  };
}

function threadRoot(overrides = {}) {
  return {
    kind: "thread",
    nodeId: "thread-root:thread::root",
    threadId: "thread::root",
    title: "Source conversation",
    threadType: "chat",
    providerType: "codex",
    agentId: "codex",
    messageCount: 1,
    lastMessagePreview: "",
    activeRunId: null,
    runState: "idle",
    updatedAt: null,
    lastActiveAt: null,
    depth: null,
    ...overrides,
  };
}

function rowShape(row) {
  return row.kind === "thread"
    ? ["thread", row.thread.threadId, row.depth]
    : ["task", row.task.number, row.depth];
}

test("keeps task nodes regardless of status for server-retained tree", () => {
  const root = threadRoot();
  const doneAncestor = task({ number: 1, status: "done" });
  const activeChild = task({
    number: 2,
    status: "in_review",
    parentNodeId: doneAncestor.nodeId,
    parentTaskNumber: 1,
    parentThreadId: doneAncestor.threadId,
  });

  assert.deepEqual(
    visibleTaskTreeTasks([root, doneAncestor, activeChild]),
    [doneAncestor, activeChild],
  );
});

test("badge counts only active tasks", () => {
  assert.equal(
    taskTreeBadgeCount([
      threadRoot(),
      task({ number: 1, status: "done" }),
      task({ number: 2, status: "in_progress" }),
      task({ number: 3, status: "in_review" }),
      task({ number: 4, status: "todo" }),
    ]),
    2,
  );
});

test("badge prefers server active_count and falls back to local recount", () => {
  const nodes = [
    threadRoot(),
    task({ number: 1, status: "in_progress" }),
    task({ number: 2, status: "done" }),
  ];
  assert.equal(
    resolveTaskTreeActiveCount({ tasks: nodes, activeCount: 7 }),
    7,
  );
  assert.equal(
    resolveTaskTreeActiveCount({ tasks: nodes, activeCount: null }),
    1,
  );
});

test("server layout renders wire order with visible thread root row", () => {
  const root = threadRoot({ depth: 0 });
  const derivedRoot = task({
    number: 1,
    status: "in_progress",
    parentNodeId: root.nodeId,
    parentThreadId: root.threadId,
    depth: 0,
  });
  const doneLeaf = task({
    number: 2,
    status: "done",
    parentNodeId: derivedRoot.nodeId,
    parentTaskNumber: 1,
    parentThreadId: derivedRoot.threadId,
    depth: 1,
  });

  const rows = buildTaskRows([root, derivedRoot, doneLeaf]);
  assert.deepEqual(rows.map(rowShape), [
    ["thread", "thread::root", 0],
    ["task", 1, 0],
    ["task", 2, 1],
  ]);
});

test("server layout clamps indent depth at 4", () => {
  const deep = task({ number: 9, depth: 7 });
  assert.deepEqual(buildTaskRows([deep]).map(rowShape), [["task", 9, 4]]);
});

test("fallback layout builds the tree locally when depth is absent", () => {
  const root = threadRoot();
  const derivedRoot = task({
    number: 1,
    status: "in_progress",
    parentNodeId: root.nodeId,
    parentThreadId: root.threadId,
  });
  const grandchild = task({
    number: 2,
    status: "in_review",
    parentNodeId: derivedRoot.nodeId,
    parentTaskNumber: 1,
    parentThreadId: derivedRoot.threadId,
  });

  assert.deepEqual(
    buildTaskRows([root, derivedRoot, grandchild]).map(rowShape),
    [
      ["thread", "thread::root", 0],
      ["task", 1, 0],
      ["task", 2, 1],
    ],
  );
});

test("fallback layout matches server layout for the same forest", () => {
  const root = threadRoot({ depth: 0 });
  const a = task({
    number: 1,
    parentNodeId: root.nodeId,
    parentThreadId: root.threadId,
    depth: 0,
  });
  const b = task({
    number: 2,
    parentNodeId: a.nodeId,
    parentTaskNumber: 1,
    parentThreadId: a.threadId,
    depth: 1,
  });
  const c = task({
    number: 3,
    parentNodeId: root.nodeId,
    parentThreadId: root.threadId,
    depth: 0,
  });

  const serverRows = buildTaskRows([root, a, b, c]).map(rowShape);
  const stripped = [root, a, b, c].map((node) => ({ ...node, depth: null }));
  const fallbackRows = buildTaskRows(stripped).map(rowShape);
  assert.deepEqual(fallbackRows, serverRows);
});

test("fallback layout treats orphan parents as roots", () => {
  const orphan = task({
    number: 5,
    parentNodeId: "task:thread::missing",
    parentTaskNumber: 4,
    parentThreadId: "thread::missing",
  });

  assert.deepEqual(buildTaskRows([orphan]).map(rowShape), [["task", 5, 0]]);
});

test("current node matching applies to thread and task rows by thread id", () => {
  const current = task({ number: 2, threadId: "thread::current" });
  assert.equal(isCurrentTaskTreeNode(current, "thread::current"), true);
  assert.equal(isCurrentTaskTreeNode(current, "thread::other"), false);
  assert.equal(
    isCurrentTaskTreeNode(threadRoot({ threadId: "thread::current" }), "thread::current"),
    true,
  );
});

test("task tree popover yields to inspector panel", () => {
  assert.equal(
    shouldShowThreadTaskTreePopover({
      inspectorOpen: false,
      isSideChatSurface: false,
      selectedThreadId: "thread::current",
      threadLogsOpen: false,
    }),
    true,
  );
  assert.equal(
    shouldShowThreadTaskTreePopover({
      inspectorOpen: true,
      isSideChatSurface: false,
      selectedThreadId: "thread::current",
      threadLogsOpen: false,
    }),
    false,
  );
  assert.equal(
    shouldShowThreadTaskTreePopover({
      inspectorOpen: false,
      isSideChatSurface: false,
      selectedThreadId: "thread::current",
      threadLogsOpen: true,
    }),
    false,
  );
  assert.equal(
    shouldShowThreadTaskTreePopover({
      inspectorOpen: false,
      isSideChatSurface: false,
      selectedThreadId: null,
      threadLogsOpen: false,
    }),
    false,
  );
});

test("task tree popover stays on the primary thread surface", () => {
  assert.equal(
    shouldShowThreadTaskTreePopover({
      inspectorOpen: false,
      isSideChatSurface: true,
      selectedThreadId: "thread::side-chat",
      threadLogsOpen: false,
    }),
    false,
  );
});

test("fallback rows preserve original parent edges for done ancestors", () => {
  const doneAncestor = task({ number: 1, status: "done" });
  const activeChild = task({
    number: 2,
    status: "in_progress",
    parentNodeId: doneAncestor.nodeId,
    parentTaskNumber: 1,
    parentThreadId: doneAncestor.threadId,
  });

  assert.deepEqual(
    buildTaskRows([doneAncestor, activeChild]).map(rowShape),
    [
      ["task", 1, 0],
      ["task", 2, 1],
    ],
  );
});

test("status labels and tones include inactive ancestors", () => {
  assert.equal(taskStatusLabel("todo"), "Todo");
  assert.equal(taskStatusLabel("done"), "Done");
  assert.equal(taskStatusTone("todo"), "todo");
  assert.equal(taskStatusTone("done"), "done");
});
